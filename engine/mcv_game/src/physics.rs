//! 玩家物理：AABB、单轴扫掠碰撞与固定步长模拟。
//!
//! 出处：仓库外反编译参照 `src-26.1/`（Minecraft 26.1）；
//! 常数换算与峰高对照见 `mc-ref/NOTES-physics.md`。
//!
//! 积分顺序（显式 Euler）：每步先用当前速度做碰撞位移（X→Z→Y），
//! 再积分外力更新速度供下一步使用。跳跃首步以完整
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
                    // 正向移动被盒子 min 面挡住，负向被 max 面挡住。
                    let v = match (axis, forward) {
                        (Axis::X, true) => b.min.x,
                        (Axis::X, false) => b.max.x,
                        (Axis::Y, true) => b.min.y,
                        (Axis::Y, false) => b.max.y,
                        (Axis::Z, true) => b.min.z,
                        (Axis::Z, false) => b.max.z,
                    };
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
/// 顺序：碰撞位移（X→Z→Y，-Y 命中且此前下落 → `on_ground`），
/// 再按 flying / 水 / 空气三种模式积分速度。
/// 固定步长推进一个通用实体（怪物/掉落物/投射物）。
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
    move_box(world, &mut e.pos, &mut e.vel, &mut b, Axis::X, dx);
    move_box(world, &mut e.pos, &mut e.vel, &mut b, Axis::Z, dz);
    let was_falling = e.vel.y < 0.0;
    let hit_y = move_box(world, &mut e.pos, &mut e.vel, &mut b, Axis::Y, dy);
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

    // --- 碰撞位移 ---
    let hit_x = move_axis(world, player, &mut aabb, Axis::X, player.vel.x * dt);
    let hit_z = move_axis(world, player, &mut aabb, Axis::Z, player.vel.z * dt);
    let horizontal_collision = hit_x || hit_z;
    let falling = player.vel.y < 0.0;
    let hit_y = move_axis(world, player, &mut aabb, Axis::Y, player.vel.y * dt);
    // -Y 命中且此前在下落 → 站在地面；否则离地。
    player.on_ground = falling && hit_y;
    player.pos = Vec3::new(
        (aabb.min.x + aabb.max.x) * 0.5,
        aabb.min.y,
        (aabb.min.z + aabb.max.z) * 0.5,
    );

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

    if input.in_water {
        // ---- 水中（travelInWater，LivingEntity.java:2459-2486）----
        // 水平：moveRelative 0.02 块/tick（:2461）+ 阻力 0.8（:2466-2368，
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
    } else if player.on_ground && input.jump {
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
    } else {
        // 空气：g=32 + 竖直空气阻力（仅下落时，见 consts::AIR_DRAG_K）。
        player.vel.y -= consts::GRAVITY * dt;
        if player.vel.y < 0.0 {
            player.vel.y *= (-consts::AIR_DRAG_K * dt).exp();
        }
    }
}
