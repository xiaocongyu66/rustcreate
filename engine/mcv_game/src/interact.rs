//! 方块交互：放置合法性判定。

use mcv_core::{BlockId, BlockPos};

use crate::Player;
use crate::VoxelAccess;
use crate::physics::Aabb;

/// 判断能否在 `pos` 放置方块：
/// 目标格必须是空气或液体（可替换），且候选格的单位立方体
/// 不与玩家 AABB（[`Aabb::from_player`]）相交。
pub fn can_place_block(world: &dyn VoxelAccess, pos: BlockPos, player: &Player) -> bool {
    let id: BlockId = world.block(pos);
    let d = id.def();
    if !(id == mcv_core::AIR || d.liquid) {
        return false;
    }
    let (fx, fy, fz) = (pos.x as f32, pos.y as f32, pos.z as f32);
    let aabb = Aabb::from_player(player.pos);
    // 候选格 [pos, pos+1) 与玩家包围盒严格重叠则不可放置。
    !(fx < aabb.max.x
        && fx + 1.0 > aabb.min.x
        && fy < aabb.max.y
        && fy + 1.0 > aabb.min.y
        && fz < aabb.max.z
        && fz + 1.0 > aabb.min.z)
}
