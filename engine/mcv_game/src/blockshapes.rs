//! 非立方形状方块的状态相关 AABB（物理碰撞箱与射线拾取箱）。
//!
//! **几何常数唯一数据源 = `mcv_core::shape::template_boxes`**（M 波 #138 起；
//! 渲染=拾取=碰撞同源，26.1 引注逐形状在该表 match 臂内）。数值全部取自
//! 26.1 反编译源码（`Block.java:163-202` 的 px→格换算：
//! `box(a..b) = [a/16, b/16]`，`column(w, y0, y1)` = x/z 居中 `w` 宽；
//! `Shapes.rotateHorizontal:399-420` 参数名 `north` = 基准朝向 → 贴边形
//! 统一「slab 位于 FACING 对侧边」）。本文件保留形状语义与 divergences：
//!
//! - 半砖：`SlabBlock.java:35-36`，碰撞=形状。
//! - 楼梯：`StairBlock.java:37-38,74-89`——踏步（整高侧）位于 FACING
//!   朝向侧，碰撞=形状。
//! - 栅栏：`CrossCollisionBlock.java:36-47,52-68` + `FenceBlock.java:37,41`
//!   ——碰撞柱/臂 24px（1.5 防跳）、拾取 outline 16px（1.0）；臂按连接
//!   判定延至格边，碰撞同样计入。
//! - 火把：`BaseTorchBlock.java:16` outline `column(4,0,10)`；无碰撞。
//! - 十字花草：无碰撞；拾取轮廓按**整格**出盒（原版花 column(6,0,10)/
//!   短草 column(12,0,13) 的细轮廓与单一 Cross 形状位不兼容——与引擎
//!   自身整格对角 quad 视觉一致优先，KNOWN-DIVERGENCE，#62 既定）。
//!
//! - 地毯：`CarpetBlock.java:17` `column(16,0,1)`，碰撞=拾取；moss_carpet
//!   悬边 draped 态登记遗留。
//! - 活板门（M 波 4 态）：关下/关上/开（竖板贴 FACING 对侧边），
//!   `TrapDoorBlock.java:48-60`；POWERED/WATERLOGGED 无状态位，登记遗留。
//! - 玻璃板：`IronBarsBlock.java:29-31`（CrossCollisionBlock）柱/臂，
//!   碰撞=拾取（无栅栏式 1.5 抬高）。
//! - 墙：`WallBlock.java:66-74`——拾取柱 16px + low 臂 14px；碰撞柱/臂
//!   24px。tall 叠墙三态无状态位，登记遗留。
//! - 柵栏门（M 波）：outline `FenceGateBlock.java:39-44` cube(16,16,4)、
//!   碰撞 `column(16,4,0,24)`（open 空，:56-62）；in_wall outline −3px。
//! - 门（M 波）：`DoorBlock.java:47-55` 3px 板贴 FACING 对侧边；open 绕
//!   hinge（LEFT→CW / RIGHT→CCW）；outline=collision。HALF（上/下半）
//!   不影响形状（getShape 无 half 分支）——half 只影响贴图/逻辑，登记债。
//! - 按钮（M 波）：`ButtonBlock.java:70-79` 6×4×2px 贴附小盒（地面/
//!   天花板/墙面）；无碰撞。POWERED（按下 1px 厚）无状态位余量，登记债。
//! - 压力板（M 波）：`BasePressurePlateBlock.java:26-27` column(14,0,1)、
//!   pressed 0.5px；无碰撞。
//! - 梯子（M 波）：`LadderBlock.java:29,43` 3px 板贴 FACING 对侧边。
//!   原版无碰撞（noCollission）；gen 表 solid=true + shapes_table
//!   「solid ⇒ 非空碰撞盒」不变量 → 碰撞=拾取薄板，KNOWN-DIVERGENCE
//!   （原版实体可嵌入梯格）。
//! - 雪层（M 波）：`SnowLayerBlock.java:28,47-55` 高度=层数×2px。原版
//!   碰撞=SHAPES[layer-1]（低一层、1 层时空形）；gen 表 solid=true +
//!   shapes_table 不变量 → 碰撞=outline 高，KNOWN-DIVERGENCE（原版 1 层
//!   雪可踩穿）。
//!
//! 楼梯 facing 编码（0=+Z 1=-Z 2=+X 3=-X）与 C++ 网格器
//! `emit_stairs`、`mcv_core::BlockId::state` 注释同一约定；几何含义
//! （踏步在朝向侧）按上表 StairBlock 源码核对。

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, Shape, shape::shape};

use crate::VoxelAccess;
use crate::physics::Aabb;

/// 单格最多盒数（栅栏：立柱 1 + 连接臂 4）。
pub const MAX_SHAPE_BOXES: usize = 5;

/// 形状盒缓冲类型。
pub type ShapeBoxes = [Aabb; MAX_SHAPE_BOXES];

/// 空的盒缓冲初始化值。
pub const EMPTY_AABB: Aabb = Aabb {
    min: Vec3::ZERO,
    max: Vec3::ZERO,
};

/// 射线命中判据。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RayTarget {
    /// 拾取形状（outline / `getShape`）：挖掘、准星选中、放置基准。
    /// 26.1 准星拾取显式走 OUTLINE（=getShape）而非碰撞：
    /// `Entity.java:1926-1931` `pick` → `ClipContext.Block.OUTLINE`
    /// （`ClipContext.java:54`），火把 outline 可命中但无碰撞。
    Pick,
    /// 碰撞形状：第三人称相机遮挡。原版相机（`Camera.java:297`）用
    /// VISUAL，而 `getVisualShape` 默认即碰撞形状（`BlockBehaviour.java:
    /// 345-347`），火把（无碰撞）不挡相机、半砖/楼梯按状态盒——与本作
    /// 一致；栅栏覆写 visual=outline 高 1.0（`FenceBlock.java:52-55`），
    /// 本作按碰撞高 1.5，相机在栅栏顶附近略多让位（KNOWN-DIVERGENCE，
    /// 物理碰撞仍须 1.5 防跳，`CrossCollisionBlock.java:45`）。
    Collide,
}

#[inline]
fn local_box(p: BlockPos, x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32) -> Aabb {
    let b = Vec3::new(p.x as f32, p.y as f32, p.z as f32);
    Aabb {
        min: b + Vec3::new(x0, y0, z0),
        max: b + Vec3::new(x1, y1, z1),
    }
}

/// 栅栏臂连接判据，26.1 `FenceBlock.java:59-63` `connectsTo` 的近似移植：
/// `sturdy 邻块 ∥ 同栅栏类别`（栅栏门分支缺：本引擎未注册栅栏门，
/// `FenceGateBlock.java` 的 `connectsToDirection` 无从对应）。
///
/// KNOWN-DIVERGENCE（无 tag / blockstate 体系，与 `cpp/src/mesher.cpp`
/// `fence_arm_connects` 同一近似，两侧必须同步）：
/// - 26.1 sturdy = 邻格支撑形（=碰撞形，`BlockBehaviour.java:297-299`）在
///   该面整面（`SupportType.java:11-16` FULL=`Block.isFaceFull`）；此处以
///   "实体不透明整立方"近似——玻璃等 opaque=false 整方块原版会连，这里
///   不连（保守方向）；
/// - `BlockTags.FENCES` / `WOODEN_FENCES`（`FenceBlock.java:65-68`）以
///   "shape==Fence" 近似——木质/非木质跨类（橡木↔下界砖栅栏）会误连；
/// - `isExceptionForConnection` 例外名单（`Block.java:255-262` 叶/屏障/雕纹
///   南瓜/南瓜灯/西瓜/南瓜/潜影盒）未移植——叶因 opaque=false 行为碰巧一致，
///   南瓜系 sturdy 会被误连。
pub fn fence_connects(nb: BlockId) -> bool {
    connects_family(nb, Shape::Fence)
}

/// 玻璃板臂连接：26.1 `IronBarsBlock.getStateForPlacement` 四邻
/// `attachsTo ∥ isFaceSturdy`——同板类（`_pane` 名族）或 sturdy 邻块；
/// sturdy 近似同 [`fence_connects`]（实体不透明整立方，宁缺勿错连）。
/// KNOWN-DIVERGENCE：`iron_bars`/`iron_trapdoor` 在原版属板/门类可互连，
/// 本引擎无 tag 不识别（保守方向）。
pub fn pane_connects(nb: BlockId) -> bool {
    connects_family(nb, Shape::Pane)
}

/// 墙臂连接：26.1 `WallBlock.canConnectTo` = 墙类同族或 sturdy 整方块；
/// 近似规则同 [`fence_connects`]。
pub fn wall_connects(nb: BlockId) -> bool {
    connects_family(nb, Shape::Wall)
}

/// 同类臂连接判据（栅栏/板/墙共用）：同族形状 → 连；否则
/// “实体不透明整立方”近似原版 sturdy。M 波 #138 起谓词唯一出处是
/// `mcv_core::shape::connects`（mesher 渲染臂同一函数，渲染臂与碰撞臂
/// 一致性由构造保证）；本包装保留 BlockId 入参与既有公开 API。
/// `fence_connects` 的 KNOWN-DIVERGENCE 清单（无 tag、无例外名单，
/// 另注：柵栏门原版可与栅栏互连 `FenceBlock.connectsTo` gate 分支，
/// 同族近似不识别）同样适用。
fn connects_family(nb: BlockId, family: Shape) -> bool {
    mcv_core::shape::connects(nb.id(), family)
}

/// `world` 在 `p` 处方块的**碰撞**盒（世界坐标），返回写入 `out` 的盒数。
/// 全立方固体=整格；火把/花草=无碰撞；形状方块按上表数值。
#[inline]
pub fn collision_boxes(world: &dyn VoxelAccess, p: BlockPos, out: &mut ShapeBoxes) -> usize {
    push_boxes(world.block(p), world, p, RayTarget::Collide, out)
}

/// `world` 在 `p` 处方块的**拾取**盒（世界坐标）：与碰撞的区别——
/// 火把/花草可命中（outline），solid=false 的空气/水仍不可命中。
#[inline]
pub fn pick_boxes(world: &dyn VoxelAccess, p: BlockPos, out: &mut ShapeBoxes) -> usize {
    push_boxes(world.block(p), world, p, RayTarget::Pick, out)
}

/// 按 `mode` 发射 `id`（位于 `p`）的盒，返回数量。几何唯一数据源 =
/// `mcv_core::shape::template_boxes`（M 波 #138；本函数按 mode 选用途，
/// 柱臂族再按邻格连接判定裁剪臂——栅栏连接位不存 nibble，与网格器同样
/// 的邻格重算法，规则见 [`fence_connects`]）。
pub fn push_boxes(
    id: BlockId,
    world: &dyn VoxelAccess,
    p: BlockPos,
    mode: RayTarget,
    out: &mut ShapeBoxes,
) -> usize {
    let d = id.def();
    let sh = shape(id.id());
    let st = id.state();
    let mut n = 0usize;
    macro_rules! add {
        ($x0:expr, $y0:expr, $z0:expr, $x1:expr, $y1:expr, $z1:expr) => {
            if n < MAX_SHAPE_BOXES {
                out[n] = local_box(p, $x0, $y0, $z0, $x1, $y1, $z1);
                n += 1;
            }
        };
    }
    let use_ = if mode == RayTarget::Collide {
        mcv_core::shape::BoxUse::Collision
    } else {
        mcv_core::shape::BoxUse::Outline
    };
    match sh {
        Shape::Cube => {
            // 固体整格碰撞；拾取同样整格。非固体立方体（空气/水/未注册）
            // 既无碰撞也不可命中（Pick 亦空）。
            if d.solid {
                add!(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
            }
        }
        Shape::Fence | Shape::Pane | Shape::Wall => {
            // 柱臂族：boxes[0]=柱（恒发射）、boxes[1..5]=四向臂
            //（表臂序 +X -X +Z -Z，与 mcv_core ARM_DIRS 同一顺序）。
            // 栅栏：碰撞柱/臂 1.5 防跳、outline 1.0（CrossCollisionBlock
            // .java:45-46）；板无抬高；墙 outline low 臂 0.875、碰撞 1.5。
            let (boxes, count) = mcv_core::shape::template_boxes(sh, 0, use_);
            if count > 0 {
                let b = &boxes[0];
                add!(b.x0, b.y0, b.z0, b.x1, b.y1, b.z1);
            }
            for (i, b) in boxes
                .iter()
                .enumerate()
                .skip(1)
                .take(count.saturating_sub(1))
            {
                let (dx, dz) = match i {
                    1 => (1, 0),
                    2 => (-1, 0),
                    3 => (0, 1),
                    _ => (0, -1),
                };
                let nb = world.block(BlockPos::new(p.x + dx, p.y, p.z + dz));
                let connected = match sh {
                    Shape::Fence => fence_connects(nb),
                    Shape::Pane => pane_connects(nb),
                    _ => wall_connects(nb),
                };
                if connected {
                    add!(b.x0, b.y0, b.z0, b.x1, b.y1, b.z1);
                }
            }
        }
        // 其余形状（cross/torch/slab/stairs/carpet/trapdoor/柵栏门/门/
        // 按钮/压力板/梯子/雪层）：模板表直出。碰撞恒空的形状（火把/
        // 按钮/压力板，原版 noCollision）在 Collision 用途下表给 0 盒。
        _ => {
            let (boxes, count) = mcv_core::shape::template_boxes(sh, st, use_);
            for b in boxes.iter().take(count) {
                add!(b.x0, b.y0, b.z0, b.x1, b.y1, b.z1);
            }
        }
    }
    n
}

/// 射线 `origin + dir*t` 与盒 `b` 在参数区间 `[t0, t1]` 内的首次相交。
/// 返回 `(t, 面法线)`；若射线从 `t0` 处已在盒内（或恰在盒面上进入），
/// 返回 `(t0, entry_normal)`——`entry_normal` 取网格步进的进入面法线，
/// 使全立方行为与旧 DDA（按格边界给法线）逐位一致。
/// `dir` 允许零分量（平行轴按"该轴恒在盒内/外"处理）。
pub fn clip_ray(
    origin: Vec3,
    dir: Vec3,
    t0: f32,
    t1: f32,
    b: &Aabb,
    entry_normal: [i32; 3],
) -> Option<(f32, [i32; 3])> {
    let mut tmin = t0;
    let mut tmax = t1;
    let mut normal = entry_normal;
    let o = [origin.x, origin.y, origin.z];
    let d = [dir.x, dir.y, dir.z];
    let lo = [b.min.x, b.min.y, b.min.z];
    let hi = [b.max.x, b.max.y, b.max.z];
    for a in 0..3 {
        if d[a] == 0.0 {
            // 平行轴：射线必须已在盒带内。
            if o[a] < lo[a] || o[a] > hi[a] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d[a];
        let ta = (lo[a] - o[a]) * inv;
        let tb = (hi[a] - o[a]) * inv;
        // 进入面：ta<tb 从 min 面进（法线 -轴），否则从 max 面进（+轴）。
        let (enter, leave, face) = if ta <= tb {
            (ta, tb, -1i32)
        } else {
            (tb, ta, 1i32)
        };
        if enter > tmin {
            tmin = enter;
            let mut n = [0i32; 3];
            n[a] = face;
            normal = n;
        }
        if leave < tmax {
            tmax = leave;
        }
        if tmin > tmax {
            return None;
        }
    }
    Some((tmin, normal))
}

/// 格 `p` 内、参数区间 `[t0, t1]` 上按 `mode` 的射线命中（首盒序无关，
/// 盒互不重叠；取最小 t）。
// 参数即一格 DDA 命中的全部输入（世界/格/射线/参数区间/判据/进入面），
// 其中射线五项原样透传给 clip_ray，打包结构体反而模糊调用点，显式放行上限。
#[allow(clippy::too_many_arguments)]
pub fn hit_in_cell(
    world: &dyn VoxelAccess,
    p: BlockPos,
    origin: Vec3,
    dir: Vec3,
    t0: f32,
    t1: f32,
    mode: RayTarget,
    entry_normal: [i32; 3],
) -> Option<(f32, [i32; 3])> {
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    let n = match mode {
        RayTarget::Pick => pick_boxes(world, p, &mut boxes),
        RayTarget::Collide => collision_boxes(world, p, &mut boxes),
    };
    let mut best: Option<(f32, [i32; 3])> = None;
    for b in &boxes[..n] {
        if let Some((t, nrm)) = clip_ray(origin, dir, t0, t1, b, entry_normal)
            && best.is_none_or(|(bt, _)| t < bt)
        {
            best = Some((t, nrm));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Mem {
        m: HashMap<(i32, i32, i32), u16>,
    }
    impl VoxelAccess for Mem {
        fn block(&self, p: BlockPos) -> BlockId {
            BlockId(*self.m.get(&(p.x, p.y, p.z)).unwrap_or(&0))
        }
        fn light(&self, _p: BlockPos) -> u8 {
            15
        }
        fn chunk_loaded(&self, _c: mcv_core::ChunkPos) -> bool {
            true
        }
    }

    fn id_of(name: &str) -> u16 {
        (0..mcv_core::BLOCKS.len() as u16)
            .find(|i| mcv_core::BLOCKS[*i as usize].name == name)
            .unwrap()
    }

    #[test]
    fn slab_stairs_torch_box_counts() {
        let w = Mem { m: HashMap::new() };
        let p = BlockPos::new(0, 0, 0);
        let mut out = [EMPTY_AABB; MAX_SHAPE_BOXES];
        let mut id = BlockId(id_of("oak_slab"));
        id = id.with_state(0);
        assert_eq!(push_boxes(id, &w, p, RayTarget::Pick, &mut out), 1);
        assert_eq!(out[0].min.y, 0.0);
        id = id.with_state(1);
        assert_eq!(push_boxes(id, &w, p, RayTarget::Collide, &mut out), 1);
        assert_eq!(out[0].min.y, 0.5);

        let mut id = BlockId(id_of("oak_stairs"));
        id = id.with_state(2); // +X 朝向
        assert_eq!(push_boxes(id, &w, p, RayTarget::Collide, &mut out), 2);
        // 踏步半盒在 +X 侧（StairBlock.java:37-38 几何推论）。
        assert!(out[1].min.x >= 0.5);

        let id = BlockId(id_of("torch"));
        assert_eq!(push_boxes(id, &w, p, RayTarget::Collide, &mut out), 0);
        assert_eq!(push_boxes(id, &w, p, RayTarget::Pick, &mut out), 1);
        assert_eq!(out[0].max.y, 0.625);

        // 空气：拾取/碰撞都无盒。
        assert_eq!(push_boxes(BlockId(0), &w, p, RayTarget::Pick, &mut out), 0);
    }

    #[test]
    fn fence_collision_height_and_arms() {
        let fence = id_of("oak_fence");
        let mut m = HashMap::new();
        m.insert((0, 0, 0), fence);
        m.insert((1, 0, 0), fence); // 同系（木质 tag 一致，原版也连）
        m.insert((-1, 0, 0), 1); // 石头 sturdy → 原版连臂
        let w = Mem { m };
        let p = BlockPos::new(0, 0, 0);
        let mut out = [EMPTY_AABB; MAX_SHAPE_BOXES];
        let n = collision_boxes(&w, p, &mut out);
        // 柱(1) +X 臂 + -X 臂 = 3；柱高 1.5（CrossCollisionBlock.java:45）。
        assert_eq!(n, 3);
        assert_eq!(out[0].max.y, 1.5);
        // outline 高 1.0。
        let n = pick_boxes(&w, p, &mut out);
        assert_eq!(n, 3);
        assert_eq!(out[0].max.y, 1.0);
    }

    #[test]
    fn clip_ray_normals() {
        // 从 -X 侧水平射入整格盒：进入面法线 [-1,0,0]。
        let b = Aabb {
            min: Vec3::new(2.0, 2.0, 2.0),
            max: Vec3::new(3.0, 3.0, 3.0),
        };
        let (t, n) = clip_ray(
            Vec3::new(0.0, 2.5, 2.5),
            Vec3::new(1.0, 0.0, 0.0),
            0.0,
            10.0,
            &b,
            [0, 0, 0],
        )
        .unwrap();
        assert_eq!(t, 2.0);
        assert_eq!(n, [-1, 0, 0]);
        // 从上方斜下命中盒顶（火把柱顶 0.625 型场景）。
        let torch = Aabb {
            min: Vec3::new(0.375, 0.0, 0.375),
            max: Vec3::new(0.625, 0.625, 0.625),
        };
        let (t, n) = clip_ray(
            Vec3::new(0.5, 3.0, 0.5),
            Vec3::new(0.0, -1.0, 0.0),
            0.0,
            10.0,
            &torch,
            [0, 0, 0],
        )
        .unwrap();
        assert_eq!(t, 3.0 - 0.625);
        assert_eq!(n, [0, 1, 0]);
        // 区间外（盒在 t1 之后）不命中。
        assert!(
            clip_ray(
                Vec3::new(0.0, 2.5, 2.5),
                Vec3::new(1.0, 0.0, 0.0),
                0.0,
                1.0,
                &b,
                [0, 0, 0]
            )
            .is_none()
        );
    }
}
