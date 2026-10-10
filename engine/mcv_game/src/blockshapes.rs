//! 非立方形状方块的状态相关 AABB（物理碰撞箱与射线拾取箱）。
//!
//! 数值全部取自 26.1 反编译源码（`Block.java:163-202` 的 px→格换算：
//! `box(a..b) = [a/16, b/16]`，`column(w, y0, y1)` = x/z 居中 `w` 宽）：
//! - 半砖：`SlabBlock.java:35-36` `column(16,0,8)` / `column(16,8,16)`，
//!   碰撞=形状（`getShape` 未分离碰撞版）。
//! - 楼梯（直段）：`StairBlock.java:37-38,74-89`——整格宽半高底座
//!   `column(16,0,8)` + 踏步半盒 `box(0,8,0,8,16,8)`；facing=NORTH 时
//!   上半占 -Z 半格，即**踏步（整高侧）位于 FACING 朝向侧**，碰撞=形状。
//! - 栅栏：`CrossCollisionBlock.java:36-47,52-68` + `FenceBlock.java:37,41`
//!   `super(4,16,4,16,24)`——柱 `column(4,0,24)`（x/z 6..10px，碰撞高 1.5
//!   防跳）；拾取形状（outline）柱/臂高 16px（1.0）；连接臂
//!   `boxZ(4, 0, h, 0, 8)`：沿臂向从格边到中心（0..0.5）、横断面与柱同宽、
//!   碰撞同样计入（防从两柱间穿过）。
//! - 火把：`BaseTorchBlock.java:16` 拾取 outline `column(4,0,10)`
//!   （x/z 6..10px、y 0..10px）；无碰撞（Blocks.java torch 为 noCollision，
//!   `getShape` 仅是拾取/遮挡形状）。
//! - 十字花草：无碰撞（Blocks.java:587/750/765 noCollision → 碰撞空，
//!   `BlockBehaviour.java:333-334`）。拾取轮廓原版**非**整格：花
//!   `column(6,0,10)`（`FlowerBlock.java:24`）、短草 `column(12,0,13)`
//!   （`TallGrassBlock.java:17`；整格 `Shapes.block()` 仅是未覆写方的
//!   默认 `BlockBehaviour.java:329-331`）。本引擎按整格出盒
//!   KNOWN-DIVERGENCE：网格器把十字渲染为整格对角 quad
//!   （`cpp/src/mesher.cpp` `emit_cross`），拾取盒与引擎自身视觉一致
//!   优先于贴原版细轮廓（单一 Cross 形状位也容纳不下花/草两种尺寸）。
//!
//! - 地毯：`CarpetBlock.java:17` `SHAPE = column(16, 0, 1)` → 全宽
//!   1/16 薄盒（y 0..0.0625），碰撞=拾取；模型
//!   `models/block/carpet.json` 元素 [0,0,0]→[16,1,16] 同值。
//! - 活板门（关态）：`TrapDoorBlock.java:48`
//!   `boxZ(16, 13, 16)`=box(0,13,0,16,16,16) 上态；下态 y 0..3px
//!   （模型 `template_trapdoor_bottom/_top` 3px 薄板）。状态 bit2=1 上态
//!   （与楼梯 top 位同位）；关态平板几何与 facing 无关。open 开态
//!   （绕铰链竖转）无状态位承载——登记遗留。
//! - 玻璃板：`StainedGlassPaneBlock extends IronBarsBlock`，
//!   `IronBarsBlock.java:29-31` `super(2, 16, 2, 16, 16)`
//!   （CrossCollisionBlock）→ 柱 `column(2,0,16)`（x/z 7..9px）、臂
//!   `boxZ(2,0,16,0,8)`（格边→中心、断面同柱宽）；碰撞=拾取（高均 1.0，
//!   无栅栏式 1.5 抬高）。模型 `template_glass_pane_post/_side`
//!   （柱/臂 x 7..9、臂 z 0..7）。
//! - 墙：`WallBlock.java:66-74`——拾取 `makeShapes(16, 14)`：柱
//!   `column(8,0,16)`（x/z 4..12px）+ 低臂 `boxZ(6,0,14,0,11)`（x/z
//!   5..11px、臂自格边至 11px、高 14px）；碰撞 `makeShapes(24, 24)`：
//!   柱/臂全抬到 24px（1.5，防跳，与栅栏同语义）。模型
//!   `template_wall_post` [4,0,4]→[12,16,12]、`template_wall_side`
//!   [5,0,0]→[11,14,8]。本引擎无 height_state/side 三态（none/low/
//!   tall）状态位——恒出柱+低臂，tall 叠墙登记遗留。
//!
//! 楼梯 facing 编码（0=+Z 1=-Z 2=+X 3=-X）与 C++ 网格器
//! `emit_stairs`、`mcv_core::BlockId::state` 注释同一约定；几何含义
//! （踏步在朝向侧）按上表 StairBlock 源码核对。
//!
//! KNOWN-DIVERGENCE（渲染侧）：carpet/trapdoor/pane/wall（Shape 6..9）
//! 的**渲染**暂为全盒占位（C++ `emit_shapes` default 分支 / Rust
//! mcv_mesher 全盒路径），薄盒/柱臂模板待 Rust 网格器（#77）按本文件
//! 数值落地；碰撞/拾取先行按原版几何，保证物理与交互正确。

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
/// “实体不透明整立方”近似原版 sturdy。`fence_connects` 的
/// KNOWN-DIVERGENCE 清单（无 tag、无例外名单）同样适用。
fn connects_family(nb: BlockId, family: Shape) -> bool {
    if nb.id() == 0 {
        return false;
    }
    let sh = shape(nb.id());
    if sh == family {
        return true;
    }
    let d = nb.def();
    d.solid && d.opaque && !d.liquid && sh == Shape::Cube
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

/// 按 `mode` 发射 `id`（位于 `p`）的盒，返回数量。栅栏臂按四邻即时判定
/// （栅栏连接位不存 nibble，与网格器同样的邻格重算法，规则见
/// [`fence_connects`]）。
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
    match sh {
        Shape::Cube => {
            // 固体整格碰撞；拾取同样整格。非固体立方体（空气/水/未注册）
            // 既无碰撞也不可命中（Pick 亦空）。
            if d.solid {
                add!(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
            }
        }
        Shape::Cross => {
            // 原版：outline=整格（可挖可选），碰撞=无（noCollision）。
            if mode == RayTarget::Pick {
                add!(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
            }
        }
        Shape::Torch => {
            // BaseTorchBlock.java:16 outline column(4,0,10)；无碰撞。
            if mode == RayTarget::Pick {
                add!(0.375, 0.0, 0.375, 0.625, 0.625, 0.625);
            }
        }
        Shape::Slab => {
            // SlabBlock.java:35-36：bit0=1 上半（y 0.5..1）。
            let y0 = if (st & 1) != 0 { 0.5 } else { 0.0 };
            add!(0.0, y0, 0.0, 1.0, y0 + 0.5, 1.0);
        }
        Shape::Stairs => {
            // StairBlock.java:37-38：底座全宽半高 + 踏步（朝向侧半格、
            // 另半高）。bit2=1（上半）上下翻转。facing 0=+Z 1=-Z 2=+X 3=-X。
            let flipped = (st & 4) != 0;
            let (by0, sy0) = if flipped { (0.5, 0.0) } else { (0.0, 0.5) };
            add!(0.0, by0, 0.0, 1.0, by0 + 0.5, 1.0);
            match st & 3 {
                0 => add!(0.0, sy0, 0.5, 1.0, sy0 + 0.5, 1.0),
                1 => add!(0.0, sy0, 0.0, 1.0, sy0 + 0.5, 0.5),
                2 => add!(0.5, sy0, 0.0, 1.0, sy0 + 0.5, 1.0),
                _ => add!(0.0, sy0, 0.0, 0.5, sy0 + 0.5, 1.0),
            }
        }
        Shape::Fence => {
            // CrossCollisionBlock.java:45-46：碰撞柱高 24px（1.5，防跳），
            // 拾取 outline 柱/臂高 16px（1.0）；臂按连接判定延至格边。
            let h = if mode == RayTarget::Collide { 1.5 } else { 1.0 };
            add!(0.375, 0.0, 0.375, 0.625, h, 0.625);
            // (dx, dz, 臂盒)——臂沿 (dx,dz) 方向从格边到中心、断面对齐柱。
            const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
            const ARMS: [(f32, f32, f32, f32); 4] = [
                (0.5, 0.375, 1.0, 0.625), // +X：x 0.5..1，z 对齐柱
                (0.0, 0.375, 0.5, 0.625), // -X
                (0.375, 0.5, 0.625, 1.0), // +Z：z 0.5..1，x 对齐柱
                (0.375, 0.0, 0.625, 0.5), // -Z
            ];
            for (&(dx, dz), &(x0, z0, x1, z1)) in DIRS.iter().zip(&ARMS) {
                if !fence_connects(world.block(BlockPos::new(p.x + dx, p.y, p.z + dz))) {
                    continue;
                }
                add!(x0, 0.0, z0, x1, h, z1);
            }
        }
        Shape::Carpet => {
            // CarpetBlock.java:17 column(16,0,1)：全宽 1/16 薄盒，碰撞=拾取。
            // moss_carpet（MossyCarpetBlock makeShapes：BASE 态同为
            // boxZ(16,0,1)）取 BASE 几何；悬边 draped 态登记遗留。
            add!(0.0, 0.0, 0.0, 1.0, 0.0625, 1.0);
        }
        Shape::Trapdoor => {
            // 关态平板（TrapDoorBlock.java:48 + template_trapdoor_*）：
            // bit2=1 上态 y 13..16px，否则下态 y 0..3px。facing/open 位
            // 未启用（开态登记遗留），关态几何与 facing 无关。
            let (y0, y1) = if (st & 4) != 0 {
                (0.8125, 1.0)
            } else {
                (0.0, 0.1875)
            };
            add!(0.0, y0, 0.0, 1.0, y1, 1.0);
        }
        Shape::Pane => {
            // IronBarsBlock（CrossCollisionBlock 子类，super(2,16,2,16,16)）：
            // 柱 column(2,0,16)、臂 boxZ(2,0,16,0,8) 格边→中心；碰撞=拾取。
            add!(0.4375, 0.0, 0.4375, 0.5625, 1.0, 0.5625);
            const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
            const ARMS: [(f32, f32, f32, f32); 4] = [
                (0.5, 0.4375, 1.0, 0.5625), // +X：x 0.5..1，z 断面 7..9px
                (0.0, 0.4375, 0.5, 0.5625), // -X
                (0.4375, 0.5, 0.5625, 1.0), // +Z
                (0.4375, 0.0, 0.5625, 0.5), // -Z
            ];
            for (&(dx, dz), &(x0, z0, x1, z1)) in DIRS.iter().zip(&ARMS) {
                if !pane_connects(world.block(BlockPos::new(p.x + dx, p.y, p.z + dz))) {
                    continue;
                }
                add!(x0, 0.0, z0, x1, 1.0, z1);
            }
        }
        Shape::Wall => {
            // WallBlock.java:66-74：拾取 post column(8,0,16) + low 臂
            // boxZ(6,0,14,0,11)；碰撞 post/臂均抬高到 24px（1.5，防跳，
            // 与栅栏同语义）。臂断面 6px（0.3125..0.6875）、自格边伸入
            // 11px（越过中心与对侧臂/柱搭接）。
            let (hp, ha) = if mode == RayTarget::Collide {
                (1.5f32, 1.5f32)
            } else {
                (1.0, 0.875)
            };
            add!(0.25, 0.0, 0.25, 0.75, hp, 0.75);
            const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
            const ARMS: [(f32, f32, f32, f32); 4] = [
                (0.3125, 0.3125, 1.0, 0.6875), // +X：x 0.3125..1
                (0.0, 0.3125, 0.6875, 0.6875), // -X
                (0.3125, 0.3125, 0.6875, 1.0), // +Z：z 0.3125..1
                (0.3125, 0.0, 0.6875, 0.6875), // -Z
            ];
            for (&(dx, dz), &(x0, z0, x1, z1)) in DIRS.iter().zip(&ARMS) {
                if !wall_connects(world.block(BlockPos::new(p.x + dx, p.y, p.z + dz))) {
                    continue;
                }
                add!(x0, 0.0, z0, x1, ha, z1);
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
