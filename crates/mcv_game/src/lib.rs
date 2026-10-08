//! Game logic: voxel world access, player, raycasting, input state.

pub mod consts;
pub mod interact;
pub mod keymap;
pub mod mining;
pub mod physics;
pub mod raycast;

pub use interact::can_place_block;
pub use mining::{
    break_seconds, hardness, has_correct_tool_for_drops, progress_for, progress_per_tick,
    requires_correct_tool, DigState, HeldTool,
};
pub use physics::{move_axis, step, step_entity, Aabb, Axis, Entity, StepInput};
pub use raycast::{raycast, REACH};

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkPos};

/// Read-only view of loaded voxels used by physics and interaction.
pub trait VoxelAccess {
    fn block(&self, p: BlockPos) -> BlockId;
    fn light(&self, p: BlockPos) -> u8;
    fn chunk_loaded(&self, c: ChunkPos) -> bool;
}

pub struct Player {
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
    pub on_ground: bool,
    pub sel_slot: usize,
    /// 生命值（MC 满值 20 = 10 心；0 死亡）。
    pub health: f32,
    /// 饥饿值（满值 20；饥饿>17 缓慢回血，0 时掉血至 10 为止，和平难度规则）。
    pub hunger: f32,
    /// 受伤无敌帧（tick，MC hurtTime=10；受击方向击退 + 闪烁）。
    pub invulnerable: i32,
    /// 饥饿消耗累计（步行距离，MC exhaustion 机制的粗化）。
    pub exhaustion: f32,
}

impl Player {
    /// Half extents (x, y, z) of the player AABB.
    pub const HALF: [f32; 3] = [0.3, 0.9, 0.3];
    /// Eye height above feet.
    pub const EYE: f32 = 1.62;
}

impl Default for Player {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            flying: false,
            on_ground: false,
            sel_slot: 0,
            health: 20.0,
            hunger: 20.0,
            invulnerable: 0,
            exhaustion: 0.0,
        }
    }
}
