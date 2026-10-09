//! 方块射线检测（Amanatides & Woo 网格步进）。

use glam::Vec3;
use mcv_core::BlockPos;

use crate::VoxelAccess;
use crate::blockshapes::{self, RayTarget};

/// 生存模式交互到达距离（米），挖掘 / 放置射线应传本值。
///
/// 参照 MC 26.1 `Attributes.BLOCK_INTERACTION_RANGE` 基础值 **4.5**
/// （`world/entity/ai/attributes/Attributes.java:23`；旧版
/// `GameType#getReachDistance` 生存同为 4.5，创造 5.0 靠属性修饰）。
/// 替换此前调用方的 5.0 临时值（mcv_app 迁移时改传本常数）。
pub const REACH: f32 = 4.5;

/// 从 `origin` 沿 `dir`（自动归一化）步进 voxel 网格，返回第一个命中方块
/// 及进入面法线（指向射线来向一侧，如从上方进入 -Y 面 → `[0, 1, 0]`）。
/// 超过 `max_dist` 未命中返回 `None`。
pub fn raycast(
    world: &dyn VoxelAccess,
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
) -> Option<(BlockPos, [i32; 3])> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }

    let mut x = origin.x.floor() as i32;
    let mut y = origin.y.floor() as i32;
    let mut z = origin.z.floor() as i32;

    let step_x = if dir.x > 0.0 { 1 } else { -1 };
    let step_y = if dir.y > 0.0 { 1 } else { -1 };
    let step_z = if dir.z > 0.0 { 1 } else { -1 };

    // 穿越一个 voxel 边界所需的 t。
    let t_delta = Vec3::new(
        if dir.x != 0.0 {
            (1.0 / dir.x).abs()
        } else {
            f32::INFINITY
        },
        if dir.y != 0.0 {
            (1.0 / dir.y).abs()
        } else {
            f32::INFINITY
        },
        if dir.z != 0.0 {
            (1.0 / dir.z).abs()
        } else {
            f32::INFINITY
        },
    );
    // 到下一个边界的初始 t。
    let mut t_max = Vec3::new(
        if dir.x > 0.0 {
            (x as f32 + 1.0 - origin.x) / dir.x
        } else if dir.x < 0.0 {
            (x as f32 - origin.x) / dir.x
        } else {
            f32::INFINITY
        },
        if dir.y > 0.0 {
            (y as f32 + 1.0 - origin.y) / dir.y
        } else if dir.y < 0.0 {
            (y as f32 - origin.y) / dir.y
        } else {
            f32::INFINITY
        },
        if dir.z > 0.0 {
            (z as f32 + 1.0 - origin.z) / dir.z
        } else if dir.z < 0.0 {
            (z as f32 - origin.z) / dir.z
        } else {
            f32::INFINITY
        },
    );

    // 命中判据 = 拾取形状（outline，`blockshapes::pick_boxes`；26.1 拾取
    // 与碰撞无关：火把 column(4,0,10) 可命中、花草整格、空气/水无盒）。
    // 每格按射线落在格内的参数区间 [进入 t, 离开 t] 与各盒求交。
    // 起始格即可命中（视线在方块内部，法线取 [0,0,0]）。
    let t1 = t_max.x.min(t_max.y).min(t_max.z).min(max_dist);
    let start = BlockPos::new(x, y, z);
    if let Some((_, normal)) = blockshapes::hit_in_cell(
        world,
        start,
        origin,
        dir,
        0.0,
        t1,
        RayTarget::Pick,
        [0, 0, 0],
    ) {
        return Some((start, normal));
    }

    let mut t = 0.0f32;
    while t <= max_dist {
        // 步进到相邻 voxel（取 t_max 最小的轴）。
        let (axis_id, normal): (u8, [i32; 3]) = if t_max.x < t_max.y && t_max.x < t_max.z {
            (0, [-step_x, 0, 0])
        } else if t_max.y < t_max.z {
            (1, [0, -step_y, 0])
        } else {
            (2, [0, 0, -step_z])
        };
        match axis_id {
            0 => {
                x += step_x;
                t = t_max.x;
                t_max.x += t_delta.x;
            }
            1 => {
                y += step_y;
                t = t_max.y;
                t_max.y += t_delta.y;
            }
            _ => {
                z += step_z;
                t = t_max.z;
                t_max.z += t_delta.z;
            }
        }
        if t > max_dist {
            return None;
        }
        let p = BlockPos::new(x, y, z);
        // 格内区间：进入 t 至下一个网格边界（离开本格）或 max_dist。
        let t1 = t_max.x.min(t_max.y).min(t_max.z);
        if let Some((th, normal)) = blockshapes::hit_in_cell(
            world,
            p,
            origin,
            dir,
            t,
            t1.min(max_dist),
            RayTarget::Pick,
            normal,
        ) {
            if th <= max_dist {
                return Some((p, normal));
            }
            return None;
        }
    }
    None
}
