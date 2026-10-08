//! 玩家物理：AABB、单轴扫掠碰撞与固定步长模拟。
//!
//! 出处：`/root/mc-ref/NOTES.md`（Minecraft 26.1 反编译换算）。
//!
//! 积分顺序（显式 Euler）：每步先用当前速度做碰撞位移（X→Z→Y），
//! 再积分外力更新速度供下一步使用。这样跳跃首步即以完整
//! [`consts::JUMP_SPEED`] 位移，峰高与 v²/2g 解析值一致
//! （半隐式 Euler 会低估约 6%，见 tests/physics.rs::jump_peak）。

use glam::Vec3;
use mcv_core::BlockPos;

use crate::consts;
use crate::Player;
use crate::VoxelAccess;

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
    pub fn intersects_voxel(&self, x: i32, y: i32, z: i32) -> bool {
        let (fx, fy, fz) = (x as f32, y as f32, z as f32);
        self.min.x < fx + 1.0
            && self.max.x > fx
            && self.min.y < fy + 1.0
            && self.max.y > fy
            && self.min.z < fz + 1.0
            && self.max.z > fz
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

        // 扫掠盒覆盖的 voxel 区间（含 ±1 余量）。
        let x0 = aabb.min.x.floor() as i32 - 1;
        let x1 = aabb.max.x.floor() as i32 + 1;
        let y0 = aabb.min.y.floor() as i32 - 1;
        let y1 = aabb.max.y.floor() as i32 + 1;
        let z0 = aabb.min.z.floor() as i32 - 1;
        let z1 = aabb.max.z.floor() as i32 + 1;

        // 一次遍历找最近的阻挡面坐标。
        let mut face: Option<f32> = None;
        for bx in x0..=x1 {
            for by in y0..=y1 {
                for bz in z0..=z1 {
                    if !world.block(BlockPos::new(bx, by, bz)).def().solid {
                        continue;
                    }
                    if !aabb.intersects_voxel(bx, by, bz) {
                        continue;
                    }
                    let v = match axis {
                        Axis::X => bx as f32,
                        Axis::Y => by as f32,
                        Axis::Z => bz as f32,
                    };
                    face = Some(match face {
                        None => v,
                        // 正向移动取最小面，负向取最大面。
                        Some(f) => {
                            if step > 0.0 {
                                f.min(v)
                            } else {
                                f.max(v)
                            }
                        }
                    });
                }
            }
        }

        if let Some(v) = face {
            // 钳位到方块面，留 SKIN 间隙。
            let target = if step > 0.0 { v - SKIN } else { v + 1.0 + SKIN };
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
pub struct StepInput {
    /// 期望水平移动方向（世界坐标，会被归一化；零向量表示无输入）。
    pub wish_dir: Vec3,
    /// 跳跃 / 上升。
    pub jump: bool,
    /// 身体处于水中。
    pub in_water: bool,
    /// 潜行 / 下降。
    pub sneak: bool,
}

impl Default for StepInput {
    fn default() -> Self {
        Self {
            wish_dir: Vec3::ZERO,
            jump: false,
            in_water: false,
            sneak: false,
        }
    }
}

/// 推进一个固定步长（[`consts::FIXED_DT`]）的玩家物理。
///
/// 顺序：碰撞位移（X→Z→Y，-Y 命中且此前下落 → `on_ground`），
/// 再按 flying / 水 / 空气三种模式积分速度。
pub fn step(world: &dyn VoxelAccess, player: &mut Player, input: &StepInput) {
    let dt = consts::FIXED_DT;
    let mut aabb = Aabb::from_player(player.pos);

    // --- 碰撞位移 ---
    move_axis(world, player, &mut aabb, Axis::X, player.vel.x * dt);
    move_axis(world, player, &mut aabb, Axis::Z, player.vel.z * dt);
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
    if player.flying {
        // 无重力：vel 以 exp(-8·dt) 阻尼朝目标收敛；
        // 水平目标为 wish_dir * FLY_SPEED，竖直由 jump/sneak 给出 ±6 m/s。
        let damp = (-consts::FLY_DAMP_K * dt).exp();
        let mut wish = input.wish_dir;
        wish.y = 0.0;
        let target_h = if wish.length_squared() > 1e-12 {
            wish.normalize_or_zero() * consts::FLY_SPEED
        } else {
            Vec3::ZERO
        };
        let target_y = if input.jump {
            consts::FLY_VERT_SPEED
        } else if input.sneak {
            -consts::FLY_VERT_SPEED
        } else {
            0.0
        };
        let target = Vec3::new(target_h.x, target_y, target_h.z);
        player.vel += (target - player.vel) * (1.0 - damp);
        return;
    }

    // 水平加速：朝 wish 目标速度收敛，不越冲。
    let mut wish = input.wish_dir;
    wish.y = 0.0;
    let speed = if input.sneak {
        consts::SNEAK_SPEED
    } else {
        consts::WALK_SPEED
    };
    let target_h = if wish.length_squared() > 1e-12 {
        wish.normalize_or_zero() * speed
    } else {
        Vec3::ZERO
    };
    let accel = if player.on_ground {
        consts::GROUND_ACCEL
    } else {
        consts::AIR_ACCEL
    };
    let horizontal = Vec3::new(player.vel.x, 0.0, player.vel.z);
    let dv = (target_h - horizontal).clamp_length_max(accel * dt);
    player.vel.x = horizontal.x + dv.x;
    player.vel.z = horizontal.z + dv.z;

    if input.in_water {
        // 水：g=4，下沉终端 2 m/s；jump 上浮 3 m/s。
        player.vel.y -= consts::WATER_GRAVITY * dt;
        if player.vel.y < -consts::WATER_SINK_SPEED {
            player.vel.y = -consts::WATER_SINK_SPEED;
        }
        if input.jump && player.vel.y < consts::WATER_RISE_SPEED {
            player.vel.y = consts::WATER_RISE_SPEED;
        }
    } else if player.on_ground && input.jump {
        // 地面跳跃：8.4 m/s 初速（覆盖本步重力，下一步离地）。
        player.vel.y = consts::JUMP_SPEED;
    } else {
        // 空气：g=32 + 竖直空气阻力（仅下落时，见 consts::AIR_DRAG_K）。
        player.vel.y -= consts::GRAVITY * dt;
        if player.vel.y < 0.0 {
            player.vel.y *= (-consts::AIR_DRAG_K * dt).exp();
        }
    }
}
