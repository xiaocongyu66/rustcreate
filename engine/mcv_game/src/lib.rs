//! Game logic: voxel world access, player, raycasting, input state.

pub mod blockshapes;
pub mod consts;
pub mod interact;
pub mod keymap;
pub mod mining;
pub mod physics;
pub mod raycast;

pub use interact::can_place_block;
pub use mining::{
    DigState, HeldTool, break_seconds, hardness, has_correct_tool_for_drops, progress_for,
    progress_per_tick, requires_correct_tool,
};
pub use physics::{Aabb, Axis, Entity, StepInput, move_axis, step, step_entity};
pub use raycast::{CREATIVE_REACH, REACH, raycast};

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
    /// 饥饿值（满值 20；hunger≥18 慢线回血、=20 且有饱和走快线，
    /// 0 时掉血至 10 为止，和平封顶——见 game::food_data_tick）。
    pub hunger: f32,
    /// 饥饿饱和度（26.1 FoodData.saturationLevel，FoodData.java:15；初始 5.0，
    /// FoodConstants.java:6 START_SATURATION；exhaustion 结算先扣饱和再扣
    /// hunger，FoodData.java:35-40）。
    pub saturation: f32,
    /// 受伤无敌帧（**tick** 单位，20 tick = 1s，26.1 LivingEntity.invulnerableTime，
    /// LivingEntity.java:1206 置 20、ServerPlayer.java:576-577 每 tick −1；由
    /// GameRuntime 在 on_tick 递减，绝不按 60 Hz 固定步计）。
    pub invulnerable: i32,
    /// 上次受伤结算用的原始伤害值（26.1 LivingEntity.lastHurt，LivingEntity.java:232；
    /// 无敌帧 >10 tick 时伤害 ≤ lastHurt 整段忽略、更强只扣差值，:1196-1206）。
    pub last_hurt: f32,
    /// 饥饿消耗累计（26.1 FoodData.exhaustionLevel，上限 40，
    /// FoodData.addExhaustion:100-101；增量表见 FoodConstants.java 与
    /// GameRuntime::fixed_step）。
    pub exhaustion: f32,
    /// 吸收盾（26.1 LivingEntity.absorptionAmount，LivingEntity.java:247；
    /// clamp 0..maxAbsorption :3335，maxAbsorption 由吸收效果 +4×(amp+1)
    /// 给出——MobEffects.java:83-87）。伤害先扣盾后扣血
    ///（actuallyHurt LivingEntity.java:1933-1939）。
    pub absorption: f32,
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
            saturation: 5.0,
            invulnerable: 0,
            last_hurt: 0.0,
            exhaustion: 0.0,
            absorption: 0.0,
        }
    }
}
