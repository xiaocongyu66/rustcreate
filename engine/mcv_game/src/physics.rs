//! 玩家物理：AABB、单轴扫掠碰撞与固定步长模拟。
//!
//! 出处：仓库外反编译参照 `src-26.1/`（Minecraft 26.1）；
//! 常数换算与峰高对照见 `mc-ref/NOTES-physics.md`。
//!
//! 积分顺序（显式 Euler）：每步先用当前速度做碰撞位移——轴序对齐原版
//! collideWithShapes（Entity.java:1174-1184 + Direction.axisStepOrder
//! Direction.java:379-381）：**Y 恒最先**，水平两轴按 |vx| < |vz| 取
//! Z→X 否则 X→Z——先落定再水平，落到台阶/岸檐口时落在顶面而非被水平
//! 钳位弹回。随后积分外力更新速度供下一步使用。跳跃首步以完整
//! [`consts::JUMP_SPEED`] 位移，峰值 ≈ v²/2g + v0·dt/2 = 1.173 m
//! （连续解析 1.1025，MC tick 制离散 1.2522；三者均 > 1.0，
//! 保证可上一格台阶，见 tests/physics_calib.rs::jump_onto_one_block；
//! 半隐式 Euler 会低估约 6%，见 tests/physics.rs::jump_peak）。

use glam::Vec3;
use mcv_core::BlockPos;

use crate::Player;
use crate::VoxelAccess;
use crate::blockshapes;
use crate::consts;

/// 单轴扫掠的子步上限（米），防止高速穿墙。
const MAX_SUBSTEP: f32 = 0.5;
/// 钳位到方块面时保留的间隙（米）。
const SKIN: f32 = 1e-4;
/// 梯子攀附探测外扩（米）：本仓梯子按生成表仍是**满格碰撞**（形状分类
/// 无梯子细板，KNOWN-DIVERGENCE：原版 LadderBlock 是贴面薄板、实体可嵌入
/// 梯子格；本仓实体最多贴面 SKIN≈1e-4），故「AABB 与梯子格相交」放宽为
/// 「AABB 外扩本量后与梯子格相交」= 贴上梯子即视为 onClimbable。
const LADDER_PROBE: f32 = 1e-3;

/// 轴对齐包围盒（世界坐标，米）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// 玩家 AABB：`pos` 为脚底中心，半尺寸 [`Player::HALF`]，总高 1.8。
    pub fn from_player(pos: Vec3) -> Self {
        let [hx, hy, hz] = crate::Player::HALF;
        Self {
            min: Vec3::new(pos.x - hx, pos.y, pos.z - hz),
            max: Vec3::new(pos.x + hx, pos.y + 2.0 * hy, pos.z + hz),
        }
    }

    /// 是否与 voxel `(x, y, z)` 的单位立方体相交（严格重叠，贴面不算）。
    /// 盒体对应的脚底中心点。
    pub fn feet_center(&self) -> Vec3 {
        Vec3::new(
            (self.min.x + self.max.x) * 0.5,
            self.min.y,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    pub fn intersects_voxel(&self, x: i32, y: i32, z: i32) -> bool {
        let (fx, fy, fz) = (x as f32, y as f32, z as f32);
        self.min.x < fx + 1.0
            && self.max.x > fx
            && self.min.y < fy + 1.0
            && self.max.y > fy
            && self.min.z < fz + 1.0
            && self.max.z > fz
    }

    /// 与另一 AABB 严格重叠（贴面不算）。
    pub fn overlaps(&self, o: &Self) -> bool {
        self.min.x < o.max.x
            && self.max.x > o.min.x
            && self.min.y < o.max.y
            && self.max.y > o.min.y
            && self.min.z < o.max.z
            && self.max.z > o.min.z
    }

    fn shift(&mut self, axis: Axis, d: f32) {
        match axis {
            Axis::X => {
                self.min.x += d;
                self.max.x += d;
            }
            Axis::Y => {
                self.min.y += d;
                self.max.y += d;
            }
            Axis::Z => {
                self.min.z += d;
                self.max.z += d;
            }
        }
    }
}

/// 碰撞检测的坐标轴。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// 单轴扫掠移动：把 `aabb` 沿 `axis` 移动 `dist`，与固体 voxel 碰撞时
/// 钳位到方块面（留 [`SKIN`] 间隙）并把 `player.vel[axis]` 清零。
///
/// 位移按 ≤ [`MAX_SUBSTEP`] 子步推进；每个子步对扫掠盒覆盖的 voxel
/// 区间（`floor(min)-1 ..= floor(max)+1`）一次遍历，取最近的阻挡面。
/// 返回本步是否发生碰撞。
/// 扫描 `aabb` 覆盖的候选格，返回**碰撞形状**（[`blockshapes`]：全立方=
/// 格边界；半砖/楼梯/栅栏=按状态盒——栅栏柱+连接臂高 1.5，
/// CrossCollisionBlock.java:45；火把/花草无碰撞盒）沿移动轴最近的阻挡面
/// 坐标：正向（朝 +轴）先碰到坐标较小的面，取各盒 min 面中最小者；
/// 负向取各盒 max 面中最大者。
///
/// 候选过滤：正向只接受**仍位于自身后边界之前**的 min 面（`b.min ≥
/// aabb.min`），负向只接受仍位于自身前边界之后的 max 面。AABB 跨两格
/// 且两格同轴都有盒时（= 玩家已嵌入固体，如出生埋沙/挤入方块），无过滤
/// 的 `min(全部 min 面)` 会选中**身后**的面，把玩家隔着整块向反方向瞬移
/// （沙坑内每次移动都被甩到沙柱另一侧，即"老是能到沙子里面"的挣扎表现）。
/// 过滤后嵌入态只会就近推出所嵌盒的最近面（正常逼近路径的候选恒满足
/// 该条件，行为逐位不变）。
fn nearest_face(world: &dyn VoxelAccess, aabb: &Aabb, axis: Axis, forward: bool) -> Option<f32> {
    let x0 = aabb.min.x.floor() as i32 - 1;
    let x1 = aabb.max.x.floor() as i32 + 1;
    let y0 = aabb.min.y.floor() as i32 - 1;
    let y1 = aabb.max.y.floor() as i32 + 1;
    let z0 = aabb.min.z.floor() as i32 - 1;
    let z1 = aabb.max.z.floor() as i32 + 1;

    let mut boxes = [blockshapes::EMPTY_AABB; blockshapes::MAX_SHAPE_BOXES];
    let mut face: Option<f32> = None;
    for bx in x0..=x1 {
        for by in y0..=y1 {
            for bz in z0..=z1 {
                let n = blockshapes::collision_boxes(world, BlockPos::new(bx, by, bz), &mut boxes);
                for b in &boxes[..n] {
                    if !aabb.overlaps(b) {
                        continue;
                    }
                    // 正向移动被盒子 min 面挡住，负向被 max 面挡住；
                    // 候选面必须不在自身反向边界之后（见函数注：防嵌入
                    // 态隔块瞬移；逼近路径恒满足，行为不变）。
                    let v = match (axis, forward) {
                        (Axis::X, true) => b.min.x,
                        (Axis::X, false) => b.max.x,
                        (Axis::Y, true) => b.min.y,
                        (Axis::Y, false) => b.max.y,
                        (Axis::Z, true) => b.min.z,
                        (Axis::Z, false) => b.max.z,
                    };
                    let in_front = match (axis, forward) {
                        (Axis::X, true) => b.min.x >= aabb.min.x,
                        (Axis::X, false) => b.max.x <= aabb.max.x,
                        (Axis::Y, true) => b.min.y >= aabb.min.y,
                        (Axis::Y, false) => b.max.y <= aabb.max.y,
                        (Axis::Z, true) => b.min.z >= aabb.min.z,
                        (Axis::Z, false) => b.max.z <= aabb.max.z,
                    };
                    if !in_front {
                        continue;
                    }
                    face = Some(match face {
                        None => v,
                        Some(f) => {
                            if forward {
                                f.min(v)
                            } else {
                                f.max(v)
                            }
                        }
                    });
                }
            }
        }
    }
    face
}

/// 单轴扫掠移动的核心实现（与 `Player` 解耦）。
/// 阻挡时钳位到碰撞面并清零 `vel` 对应分量，返回是否命中。
pub fn move_box(
    world: &dyn VoxelAccess,
    pos: &mut Vec3,
    vel: &mut Vec3,
    aabb: &mut Aabb,
    axis: Axis,
    dist: f32,
) -> bool {
    let mut remaining = dist;
    while remaining.abs() > 1e-9 {
        let step = remaining.clamp(-MAX_SUBSTEP, MAX_SUBSTEP);
        aabb.shift(axis, step);

        if let Some(v) = nearest_face(world, aabb, axis, step > 0.0) {
            // v 已是阻挡面本身（正向=盒 min 面，负向=盒 max 面）。
            let target = if step > 0.0 { v - SKIN } else { v + SKIN };
            let delta = match (axis, step > 0.0) {
                (Axis::X, true) => target - aabb.max.x,
                (Axis::X, false) => target - aabb.min.x,
                (Axis::Y, true) => target - aabb.max.y,
                (Axis::Y, false) => target - aabb.min.y,
                (Axis::Z, true) => target - aabb.max.z,
                (Axis::Z, false) => target - aabb.min.z,
            };
            let delta = if step > 0.0 {
                delta.min(0.0)
            } else {
                delta.max(0.0)
            };
            aabb.shift(axis, delta);
            match axis {
                Axis::X => vel.x = 0.0,
                Axis::Y => vel.y = 0.0,
                Axis::Z => vel.z = 0.0,
            }
            *pos = aabb.feet_center();
            return true;
        }
        remaining -= step;
    }
    false
}

/// 出水余量检查：玩家盒整体抬升 `rise` 后是否与任何碰撞形状重叠
/// （vanilla `jumpOutOfFluid` 的 `isFree(dx, movement.y + 0.6 − Δy, dz)`
/// 近似——只验竖直抬升，水平位移分量以已发生的碰撞判定替代）。
/// 玩家是否处于可攀附状态（原版 `LivingEntity.onClimbable` :1689-1713，
/// 实查：谓词 = 身处格的方块态属 CLIMBABLE 标签，梯子是唯一注册者）。
/// 观测量 = 玩家 AABB 外扩 [`LADDER_PROBE`] 后与任意梯子格严格相交
/// （原版为嵌入梯子格；本仓梯子满格碰撞贴面即达，见 LADDER_PROBE 注）。
pub fn on_climbable(world: &dyn VoxelAccess, aabb: &Aabb) -> bool {
    let p = Aabb {
        min: Vec3::new(
            aabb.min.x - LADDER_PROBE,
            aabb.min.y - LADDER_PROBE,
            aabb.min.z - LADDER_PROBE,
        ),
        max: Vec3::new(
            aabb.max.x + LADDER_PROBE,
            aabb.max.y + LADDER_PROBE,
            aabb.max.z + LADDER_PROBE,
        ),
    };
    let x0 = p.min.x.floor() as i32;
    let x1 = p.max.x.floor() as i32;
    let y0 = p.min.y.floor() as i32;
    let y1 = p.max.y.floor() as i32;
    let z0 = p.min.z.floor() as i32;
    let z1 = p.max.z.floor() as i32;
    for bx in x0..=x1 {
        for by in y0..=y1 {
            for bz in z0..=z1 {
                if world.block(BlockPos::new(bx, by, bz)).def().name == "ladder"
                    && p.intersects_voxel(bx, by, bz)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn headroom_clear(world: &dyn VoxelAccess, pos: Vec3, rise: f32) -> bool {
    let hx = crate::Player::HALF[0];
    let h = 2.0 * crate::Player::HALF[1];
    let x0 = (pos.x - hx).floor() as i32;
    let x1 = (pos.x + hx).floor() as i32;
    let y0 = (pos.y + rise).floor() as i32;
    let y1 = (pos.y + rise + h).floor() as i32;
    let z0 = (pos.z - hx).floor() as i32;
    let z1 = (pos.z + hx).floor() as i32;
    let mut boxes = [blockshapes::EMPTY_AABB; blockshapes::MAX_SHAPE_BOXES];
    for bx in x0..=x1 {
        for by in y0..=y1 {
            for bz in z0..=z1 {
                if blockshapes::collision_boxes(world, BlockPos::new(bx, by, bz), &mut boxes) > 0 {
                    return false;
                }
            }
        }
    }
    true
}

/// 潜行边缘判定：把 `aabb`（含水平位移后的脚底盒）压平为脚底平面、
/// 沿 -Y 扩 `max_down`（原版 canFallAtLeast 的探测盒，Player.java:935-946
/// `AABB(minX+1e-7+dx, minY-maxDown-1e-7, ..., maxX-1e-7+dx, minY, ...)`），
/// 范围内**无任何碰撞形状 = 可以跌落** 返回 true。
fn can_fall_at(world: &dyn VoxelAccess, aabb: &Aabb, dx: f32, dz: f32, max_down: f32) -> bool {
    let eps = 1e-7f32;
    let probe = Aabb {
        min: Vec3::new(
            aabb.min.x + eps + dx,
            aabb.min.y - max_down - eps,
            aabb.min.z + eps + dz,
        ),
        max: Vec3::new(aabb.max.x - eps + dx, aabb.min.y, aabb.max.z - eps + dz),
    };
    let mut boxes = [blockshapes::EMPTY_AABB; blockshapes::MAX_SHAPE_BOXES];
    for bx in probe.min.x.floor() as i32..=probe.max.x.floor() as i32 {
        for by in probe.min.y.floor() as i32..=probe.max.y.floor() as i32 {
            for bz in probe.min.z.floor() as i32..=probe.max.z.floor() as i32 {
                if !probe.intersects_voxel(bx, by, bz) {
                    continue;
                }
                let n = blockshapes::collision_boxes(world, BlockPos::new(bx, by, bz), &mut boxes);
                for b in &boxes[..n] {
                    if probe.overlaps(b) {
                        return false; // 有支撑，掉不下去
                    }
                }
            }
        }
    }
    true
}

/// 潜行防跌落（26.1 `Player.maybeBackOffFromEdge`，Player.java:880-933，
/// 在 `Entity.move` 碰撞前对 delta 生效，Entity.java:737）：X、Z 各自
/// 0.05 步长回缩到「不再悬空」，双轴合走再回缩一轮。原版步长 0.05 块
/// /tick（Player.java:889-891），本函数在 60 Hz 步内对当步子位移
/// （≤0.1 块）同规则处理——循环至多一两轮，语义等价。
fn edge_back_off(
    world: &dyn VoxelAccess,
    aabb: &Aabb,
    mut dx: f32,
    mut dz: f32,
    max_down: f32,
) -> (f32, f32) {
    const STEP: f32 = 0.05; // Player.java:889
    while dx != 0.0 && can_fall_at(world, aabb, dx, 0.0, max_down) {
        if dx.abs() <= STEP {
            dx = 0.0;
            break;
        }
        dx -= dx.signum() * STEP;
    }
    while dz != 0.0 && can_fall_at(world, aabb, 0.0, dz, max_down) {
        if dz.abs() <= STEP {
            dz = 0.0;
            break;
        }
        dz -= dz.signum() * STEP;
    }
    while dx != 0.0 && dz != 0.0 && can_fall_at(world, aabb, dx, dz, max_down) {
        if dx.abs() <= STEP {
            dx = 0.0;
        } else {
            dx -= dx.signum() * STEP;
        }
        if dz.abs() <= STEP {
            dz = 0.0;
        } else {
            dz -= dz.signum() * STEP;
        }
    }
    (dx, dz)
}

/// 自动上台阶重试（26.1 `Entity.collide`，Entity.java:1080-1106 +
/// `collectCandidateStepUpHeights` :1111-1136）：水平被挡且站地时，取
/// 移动方向扩展、上抬 [`consts::STEP_HEIGHT`] 范围内的碰撞盒顶面为候选
/// 高度，升序逐个「抬 h 后重走完整位移（Y 先，水平序同主路径）」，
/// 第一个比本次实际水平位移更远的候选即采纳（原版 :1101 的
/// horizontalDistanceSqr 比较）。返回 (落位盒, 水平速度)。
/// 半砖 0.5 可登上，整块 1.0 > 0.6 无候选、维持撞墙。
fn try_step_up(
    world: &dyn VoxelAccess,
    grounded: &Aabb,
    moved: &Aabb,
    dx: f32,
    dz: f32,
    vx0: f32,
    vz0: f32,
) -> Option<(Aabb, f32, f32)> {
    let max_up = consts::STEP_HEIGHT;
    // 候选收集（Entity.java:1090-1136：stepUpAABB = grounded 沿移动方向
    // 扩 dx/dz、上抬 maxUpStep；候选 = 碰撞盒顶面 − 脚底，(0, maxUp]）。
    let mut boxes = [blockshapes::EMPTY_AABB; blockshapes::MAX_SHAPE_BOXES];
    let mut cand: Vec<f32> = Vec::new();
    let ex0 = grounded.min.x.min(grounded.min.x + dx);
    let ex1 = grounded.max.x.max(grounded.max.x + dx);
    let ez0 = grounded.min.z.min(grounded.min.z + dz);
    let ez1 = grounded.max.z.max(grounded.max.z + dz);
    for bx in ex0.floor() as i32..=ex1.floor() as i32 {
        for by in grounded.min.y.floor() as i32..=(grounded.max.y + max_up).floor() as i32 {
            for bz in ez0.floor() as i32..=ez1.floor() as i32 {
                let n = blockshapes::collision_boxes(world, BlockPos::new(bx, by, bz), &mut boxes);
                for b in &boxes[..n] {
                    let rel = b.max.y - grounded.min.y;
                    if rel > 1e-3 && rel <= max_up && !cand.contains(&rel) {
                        cand.push(rel);
                    }
                }
            }
        }
    }
    if cand.is_empty() {
        return None;
    }
    cand.sort_by(f32::total_cmp); // FloatArrays.unstableSort（:1134 升序）
    // 本次实际水平位移（movementStep.horizontalDistance，比较基准 :1101）。
    let gx = (moved.min.x + moved.max.x - grounded.min.x - grounded.max.x) * 0.5;
    let gz = (moved.min.z + moved.max.z - grounded.min.z - grounded.max.z) * 0.5;
    let base = gx * gx + gz * gz;
    for &h in &cand {
        let mut tb = *grounded;
        let mut tpos = tb.feet_center();
        let mut tvel = Vec3::new(vx0, 0.0, vz0);
        // 重试向量 (dx, h, dz)：Y 恒最先（collideWithShapes，
        // Entity.java:1168-1183），水平序同主路径。抬升加 SKIN 余量：
        // 浮点舍入若把盒底落到台阶面下方 1ulp，会被严格重叠判挡住重试
        // （原版无此问题：Y 碰撞不留间隙，minY 精确等于顶面）。
        move_box(world, &mut tpos, &mut tvel, &mut tb, Axis::Y, h + SKIN);
        if dx.abs() < dz.abs() {
            move_box(world, &mut tpos, &mut tvel, &mut tb, Axis::Z, dz);
            move_box(world, &mut tpos, &mut tvel, &mut tb, Axis::X, dx);
        } else {
            move_box(world, &mut tpos, &mut tvel, &mut tb, Axis::X, dx);
            move_box(world, &mut tpos, &mut tvel, &mut tb, Axis::Z, dz);
        }
        let rx = (tb.min.x + tb.max.x - grounded.min.x - grounded.max.x) * 0.5;
        let rz = (tb.min.z + tb.max.z - grounded.min.z - grounded.max.z) * 0.5;
        if rx * rx + rz * rz > base + 1e-12 {
            return Some((tb, tvel.x, tvel.z));
        }
    }
    None
}

pub fn move_axis(
    world: &dyn VoxelAccess,
    player: &mut Player,
    aabb: &mut Aabb,
    axis: Axis,
    dist: f32,
) -> bool {
    let mut remaining = dist;
    while remaining.abs() > 1e-9 {
        let step = remaining.clamp(-MAX_SUBSTEP, MAX_SUBSTEP);
        aabb.shift(axis, step);

        if let Some(v) = nearest_face(world, aabb, axis, step > 0.0) {
            // v 已是阻挡面坐标（正向=盒 min 面，负向=盒 max 面），
            // 钳位留 SKIN 间隙。
            let target = if step > 0.0 { v - SKIN } else { v + SKIN };
            // 正向移动看 max 边，负向看 min 边。
            let delta = match (axis, step > 0.0) {
                (Axis::X, true) => target - aabb.max.x,
                (Axis::X, false) => target - aabb.min.x,
                (Axis::Y, true) => target - aabb.max.y,
                (Axis::Y, false) => target - aabb.min.y,
                (Axis::Z, true) => target - aabb.max.z,
                (Axis::Z, false) => target - aabb.min.z,
            };
            // 修正量只能把盒子往移动反方向拉。
            let delta = if step > 0.0 {
                delta.min(0.0)
            } else {
                delta.max(0.0)
            };
            aabb.shift(axis, delta);
            match axis {
                Axis::X => player.vel.x = 0.0,
                Axis::Y => player.vel.y = 0.0,
                Axis::Z => player.vel.z = 0.0,
            }
            return true;
        }
        remaining -= step;
    }
    false
}

/// 一步物理的输入意图。
/// Generic AABB physics body (mobs, items, projectiles).
#[derive(Clone, Copy, Debug)]
pub struct Entity {
    pub pos: Vec3, // feet-center
    pub vel: Vec3,
    pub on_ground: bool,
}

pub struct StepInput {
    /// 期望水平移动方向（世界坐标，模长 ≤1：键盘全速为 1，触屏摇杆为
    /// 偏移比例——原版 getInputVector 只在模长 >1 时归一化
    /// （Entity.java:1677），保留亚单位量才能表达模拟量输入）。
    pub wish_dir: Vec3,
    /// 跳跃 / 上升。
    pub jump: bool,
    /// 身体处于水中。
    pub in_water: bool,
    /// 潜行 / 下降。
    pub sneak: bool,
    /// 冲刺：水平目标速度取 [`consts::SPRINT_SPEED`]（26.1 SPRINTING 速度修饰
    /// 为 `+30%` ADD_MULTIPLIED_TOTAL，LivingEntity.java:156-158）。潜行优先于
    /// 冲刺（原版蹲下即退冲刺）。调用方负责饥饿门（food>6，见 GameRuntime）。
    pub sprint: bool,
    /// 归一化视线方向（俯仰含 y 分量）：地面冲刺跳沿其水平分量增补
    /// （LivingEntity.java:2349-2351 用 yaw 朝向）；冲刺游泳竖直按其 y
    /// 分量转向（Player.java:1383-1392）。
    pub look_dir: Vec3,
    /// 重力缩放（生物 1.0；掉落物 0.5 = 原版 ItemEntity.getDefaultGravity
    /// 0.04 块/tick² 相对 Entity 默认 0.08 的比值 → 16 m/s²）。
    pub gravity_scale: f32,
}

impl Default for StepInput {
    fn default() -> Self {
        Self {
            wish_dir: Vec3::ZERO,
            jump: false,
            in_water: false,
            sneak: false,
            sprint: false,
            look_dir: Vec3::ZERO,
            gravity_scale: 1.0,
        }
    }
}

/// 推进一个固定步长（[`consts::FIXED_DT`]）的玩家物理。
///
/// 顺序：碰撞位移（Y 先行 + 水平 |vx|<|vz| 序，见模块注；-Y 命中且此前
/// 下落 → `on_ground`），再按 flying / 水 / 空气三种模式积分速度。
pub fn step_entity(world: &dyn VoxelAccess, e: &mut Entity, half: [f32; 3], input: &StepInput) {
    let dt = consts::FIXED_DT;
    let speed = if e.on_ground { 1.0 } else { 0.2 };
    let target = input.wish_dir * speed;
    let accel = if e.on_ground { 60.0 } else { 8.0 };
    e.vel.x += (target.x - e.vel.x).clamp(-accel * dt, accel * dt);
    e.vel.z += (target.z - e.vel.z).clamp(-accel * dt, accel * dt);
    e.vel.y -= consts::GRAVITY * input.gravity_scale * dt;
    e.vel.y *= (-consts::AIR_DRAG_K * dt).exp();

    let mut b = Aabb {
        min: Vec3::new(e.pos.x - half[0], e.pos.y, e.pos.z - half[2]),
        max: Vec3::new(
            e.pos.x + half[0],
            e.pos.y + half[1] * 2.0,
            e.pos.z + half[2],
        ),
    };
    let dx = e.vel.x * dt;
    let dz = e.vel.z * dt;
    let dy = e.vel.y * dt;
    // 轴序同 `step`：Y 恒最先（Entity.java:1174-1184），水平按
    // |vx| < |vz| 取 (Z,X) 否则 (X,Z)（Direction.java:379-381）。
    let was_falling = e.vel.y < 0.0;
    let hit_y = move_box(world, &mut e.pos, &mut e.vel, &mut b, Axis::Y, dy);
    if dx.abs() < dz.abs() {
        move_box(world, &mut e.pos, &mut e.vel, &mut b, Axis::Z, dz);
        move_box(world, &mut e.pos, &mut e.vel, &mut b, Axis::X, dx);
    } else {
        move_box(world, &mut e.pos, &mut e.vel, &mut b, Axis::X, dx);
        move_box(world, &mut e.pos, &mut e.vel, &mut b, Axis::Z, dz);
    }
    // move_box 仅在钳位（碰撞）路径写回 pos，无碰撞自由移动只推进盒体
    // ——悬空实体（下落中的掉落物/跳跃中的怪物）pos 会冻结在原地。与
    // `step`（玩家路径）同款：步末统一从盒体回填 feet-center。
    e.pos = Vec3::new(
        (b.min.x + b.max.x) * 0.5,
        b.min.y,
        (b.min.z + b.max.z) * 0.5,
    );
    e.on_ground = hit_y && was_falling;
}

pub fn step(world: &dyn VoxelAccess, player: &mut Player, input: &StepInput) {
    let dt = consts::FIXED_DT;
    let mut aabb = Aabb::from_player(player.pos);
    // 攀附谓词（onClimbable）在步首判定；创造飞行优先（原版 onClimbable
    // 对 spectator=false 才成立，飞行中无攀附爬升）。
    let on_ladder = !player.flying && on_climbable(world, &aabb);

    // --- 碰撞位移（轴序 = 原版 collideWithShapes，Entity.java:1174-1184）---
    // Y 恒最先（Direction.axisStepOrder，Direction.java:379-381），水平两轴
    // 按 |vx| < |vz| 取 Z→X 否则 X→Z。先落定再水平：下落到台阶/岸檐口时
    // 落在其顶面，而非先被水平钳位从檐口上弹回（水中贴岸登陆依赖此序）。
    let falling = player.vel.y < 0.0;
    let hit_y = move_axis(world, player, &mut aabb, Axis::Y, player.vel.y * dt);
    // -Y 命中且此前在下落 → 站在地面；否则离地。
    player.on_ground = falling && hit_y;
    // 当步子位移。原版 `Entity.move` 先过 maybeBackOffFromEdge 再拿回缩后
    // 的 delta 去碰撞（Entity.java:737），轴序也按回缩后的分量取——这里
    // 同序：先潜行边缘回缩，再按回缩值定轴序。
    let mut dx = player.vel.x * dt;
    let mut dz = player.vel.z * dt;
    // 潜行防跌落（Player.maybeBackOffFromEdge，Player.java:880-933）：
    // 原版门 = 不飞行 + delta.y ≤ 0 + isStayingOnGroundSurface()
    // (=isShiftKeyDown，Player.java:299-301) + isAboveGround(maxDownStep)
    // （:931-933）。本仓用本步 Y 位移后的 on_ground 近似 isAboveGround
    // （原版在坠落下落的最后 <0.6 m 段内同样生效，差异 <0.03 s，登记）。
    if input.sneak && !player.flying && player.vel.y <= 0.0 && player.on_ground {
        let (sx, sz) = edge_back_off(world, &aabb, dx, dz, consts::STEP_HEIGHT);
        dx = sx;
        dz = sz;
    }
    let grounded = aabb; // Y 落定后的盒（原版 groundedAABB，Entity.java:1089）
    let (vx0, vz0) = (player.vel.x, player.vel.z);
    let hit_x;
    let hit_z;
    if dx.abs() < dz.abs() {
        hit_z = move_axis(world, player, &mut aabb, Axis::Z, dz);
        hit_x = move_axis(world, player, &mut aabb, Axis::X, dx);
    } else {
        hit_x = move_axis(world, player, &mut aabb, Axis::X, dx);
        hit_z = move_axis(world, player, &mut aabb, Axis::Z, dz);
    }
    let horizontal_collision = hit_x || hit_z;
    player.pos = Vec3::new(
        (aabb.min.x + aabb.max.x) * 0.5,
        aabb.min.y,
        (aabb.min.z + aabb.max.z) * 0.5,
    );

    // ---- 自动上台阶（Entity.collide，Entity.java:1080-1106）----
    // 条件：maxUpStep > 0 &&（本步落地或站地）&& 水平碰撞。原版上台阶
    // 只裁位置不动 deltaMovement（水平速度保留）；重试盒若二次撞墙，
    // move_box 已把对应分量清零，直接采纳其结果。
    if horizontal_collision
        && (player.on_ground || (falling && hit_y))
        && let Some((tb, tvx, tvz)) = try_step_up(world, &grounded, &aabb, dx, dz, vx0, vz0)
    {
        player.pos = tb.feet_center();
        player.vel.x = tvx;
        player.vel.z = tvz;
        // 抬上台阶面即视为站地（原版靠下一 tick Y 位移复核，这里同帧
        // 置位让跳跃/步进节奏不断档）。
        player.on_ground = true;
    }

    // --- 速度积分 ---
    // 原版 moveRelative/getInputVector 只在输入模长 >1 时归一化
    // （Entity.java:1677）——保留亚单位模长以支持触屏摇杆模拟量。
    let mut wish = input.wish_dir;
    wish.y = 0.0;
    let wish = if wish.length_squared() > 1.0 {
        wish.normalize_or_zero()
    } else {
        wish
    };
    if player.flying {
        // 创造飞行（Player.travel 飞行分支 Player.java:1394-1397 + super
        // travelInAir）：无重力。
        // 水平：moveRelative 输入 0.05 块/tick（Abilities.java:19）+ 阻力
        // 0.91/tick（LivingEntity.java:2443）→ 一阶滞后收敛到 FLY_SPEED，
        // 冲刺时 getFlyingSpeed ×2（Player.java:1953）→ 20.22 m/s。
        // 竖直：jump/sneak 每 tick ±0.15（LocalPlayer.java:878），y 回写 ×0.6
        // （Player.java:1397）→ 收敛到 ±4.5 m/s，松键 ~0.2s 刹停。
        let speed = if input.sprint {
            consts::FLY_SPEED * consts::SPRINT_FLY_MULTIPLIER
        } else {
            consts::FLY_SPEED
        };
        let damp_h = (-consts::FLY_DRAG_H * dt).exp();
        let damp_v = (-consts::FLY_DRAG_V * dt).exp();
        let target_h = wish * speed;
        let target_y = if input.jump {
            consts::FLY_VERT_SPEED
        } else if input.sneak {
            -consts::FLY_VERT_SPEED
        } else {
            0.0
        };
        player.vel.x += (target_h.x - player.vel.x) * (1.0 - damp_h);
        player.vel.z += (target_h.z - player.vel.z) * (1.0 - damp_h);
        player.vel.y += (target_y - player.vel.y) * (1.0 - damp_v);
        return;
    }

    if input.in_water {
        // ---- 水中（travelInWater，LivingEntity.java:2459-2486）----
        // **替换**通用地面/空气收敛——水中无地面摩擦体系，两套一阶滞后
        // 叠加会打架（实测稳态被顶到 3.5 m/s）。
        // 水平：moveRelative 0.02 块/tick（:2461）+ 阻力 0.8（:2366-2368，
        // sprinting 0.9 :2460）→ 一阶滞后收敛到 1.6（冲刺 3.6）m/s。
        let (h_sp, h_k) = if input.sprint {
            (consts::SWIM_SPRINT_SPEED, consts::SWIM_SPRINT_DRAG_K)
        } else {
            (consts::SWIM_SPEED, consts::SWIM_DRAG_K)
        };
        let conv_h = 1.0 - (-h_k * dt).exp();
        let target_h = wish * h_sp;
        player.vel.x += (target_h.x - player.vel.x) * conv_h;
        player.vel.z += (target_h.z - player.vel.z) * conv_h;
        // 竖直：按住跳 +0.04/tick（jumpInLiquid :2362-2364）、按住潜行
        // −0.04/tick（goDownInWater，LocalPlayer.java:855-857）、中性缓沉；
        // 叠加重力 −g/16 = −0.005/tick（getFluidFallingAdjustedMovement
        // :2627-2639，sprinting 免疫）与 y 阻力 0.8/tick（:2483）。
        // 深水**没有** jumpFromGround（aiStep 流体分支 :3032-3046）。
        let (target_y, v_k) = if input.sprint {
            // 冲刺游泳（isSwimming）：y 朝视线俯仰收敛
            // （Player.travel，Player.java:1383-1392）且免重力。
            (
                input.look_dir.y * consts::SWIM_LOOK_GAIN,
                consts::SWIM_LOOK_STEER_K,
            )
        } else if input.jump {
            (consts::SWIM_UP_SPEED, consts::SWIM_DRAG_K)
        } else if input.sneak {
            (-consts::SWIM_DOWN_SPEED, consts::SWIM_DRAG_K)
        } else {
            (-consts::SWIM_SINK_SPEED, consts::SWIM_DRAG_K)
        };
        let conv_y = 1.0 - (-v_k * dt).exp();
        player.vel.y += (target_y - player.vel.y) * conv_y;
        // 贴水面/按墙出水：水平碰撞且抬升 0.6 后无碰撞 → vy 置 6 m/s
        // （jumpOutOfFluid :2506-2511；每 tick 结算——水下按墙攀爬、
        // 游出水面登陆均源于此）。
        if horizontal_collision && headroom_clear(world, player.pos, 0.6) {
            player.vel.y = consts::SWIM_EXIT_SPEED;
        }
        return;
    }

    // 水平加速：一阶滞后朝目标速度收敛（「每 tick 先加输入再乘阻」的
    // 等价连续式），不越冲、松手同曲线减速。
    let (target_speed, k) = if player.on_ground {
        let s = if input.sneak {
            consts::SNEAK_SPEED
        } else if input.sprint {
            consts::SPRINT_SPEED
        } else {
            consts::WALK_SPEED
        };
        // 地面：阻力 0.546/tick（0.6×0.91，LivingEntity.java:2424-2425）
        // → k = −20·ln 0.546。
        (s, consts::GROUND_CONVERGE_K)
    } else {
        // 空中：moveRelative 输入 0.02（冲刺 0.026）块/tick（Player.java:1955）
        // + 阻力 0.91/tick → 稳态 4.044/5.258 m/s（略低于地面目标——原版
        // 空中控速本就更弱）；潜行对输入 ×0.3（LocalPlayer.java:714）。
        let s = if input.sneak {
            consts::AIR_TERMINAL * 0.3
        } else if input.sprint {
            consts::AIR_TERMINAL_SPRINT
        } else {
            consts::AIR_TERMINAL
        };
        (s, consts::AIR_CONVERGE_K)
    };
    let target_h = wish * target_speed;
    let conv = 1.0 - (-k * dt).exp();
    player.vel.x += (target_h.x - player.vel.x) * conv;
    player.vel.z += (target_h.z - player.vel.z) * conv;

    if player.on_ground && input.jump {
        // 地面跳跃：JUMP_STRENGTH 0.42 块/tick = 8.4 m/s
        // （Attributes.java:48-49 + LivingEntity.java:2344-2348，覆盖本步
        // 重力，下一步离地）。
        player.vel.y = consts::JUMP_SPEED;
        // 冲刺跳水平增补：沿视线水平分量 +0.2 块/tick = 4.0 m/s
        // （LivingEntity.java:2349-2351）。与移动意图无关——原版沿
        // yaw 朝向加。
        let fh = Vec3::new(input.look_dir.x, 0.0, input.look_dir.z);
        if input.sprint && fh.length_squared() > 1e-12 {
            let boost = fh.normalize_or_zero() * consts::SPRINT_JUMP_BOOST;
            player.vel.x += boost.x;
            player.vel.z += boost.z;
        }
    } else if on_ladder {
        // ---- 梯子攀爬（LivingEntity.handleOnClimbable :2642-2654 + travelInAir
        // 碰撞爬升 :2620-2622）----
        // 爬升条件 = (水平碰撞 || 跳) && onClimbable：贴面向上爬
        // consts::LADDER_CLIMB_SPEED（≈2.4 格/s，离散折算见 consts 注）。
        // 松手不按跳：重力保留但下落钳 max(yd, −0.15 块/tick)（:2648）；
        // 潜行（isSuppressingSlidingDownLadder :3585-3587 = isShiftKeyDown，
        // Player 专属）钉停 y=0。水平钳 ±0.15 块/tick（:2646-2647）。
        // 原版 onClimbable 每拍 resetFallDistance（:2644）——摔落账豁免由
        // GameRuntime 侧挂同一 on_climbable 谓词。
        if horizontal_collision || input.jump {
            player.vel.y = consts::LADDER_CLIMB_SPEED;
        } else {
            player.vel.y -= consts::GRAVITY * dt;
            if input.sneak {
                player.vel.y = 0.0;
            }
            player.vel.y = player.vel.y.max(-consts::LADDER_SLIDE_SPEED);
        }
        player.vel.x = player
            .vel
            .x
            .clamp(-consts::LADDER_H_CLAMP, consts::LADDER_H_CLAMP);
        player.vel.z = player
            .vel
            .z
            .clamp(-consts::LADDER_H_CLAMP, consts::LADDER_H_CLAMP);
    } else {
        // 空气：g=32 + 竖直空气阻力（仅下落时，见 consts::AIR_DRAG_K）。
        player.vel.y -= consts::GRAVITY * dt;
        if player.vel.y < 0.0 {
            player.vel.y *= (-consts::AIR_DRAG_K * dt).exp();
        }
    }
}
