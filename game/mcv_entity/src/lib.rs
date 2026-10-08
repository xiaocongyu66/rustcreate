//! Mobs: definitions, spawn rules (NaturalSpawner, MC 26.1), combat math
//! (Player.attack / LivingEntity.hurt / CombatRules), hostile AI state machine
//! (Goal-system equivalent, simplified) and death drops.
//! All constants from /root/mc-ref/NOTES-mobs.md & NOTES-2.md (source line refs).

pub mod ai;
pub mod combat;
pub mod defs;
pub mod drops;
pub mod spawner;

pub use ai::{ai_table, AiAction, Brain, MobAiTable, MobState, Percept};
pub use defs::{MobDef, MobId, MOBS};
pub use drops::{death_drops, DropEvent};
pub use spawner::{SpawnCategory, SpawnConfig, SpawnRule, SpawnWorld, Spawned, SPAWN_CAPS};

use glam::Vec3;

/// Runtime mob instance.
pub struct Mob {
    pub def: MobId,
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub health: f32,
    pub on_ground: bool,
    /// Ticks since last attacked (invulnerability frames, 20 = 1s).
    pub invulnerable: u32,
    /// Damage of the last hit taken (i-frame差值结算基准).
    pub last_hurt: f32,
    /// Ticks idle (despawn accounting).
    pub idle_ticks: u64,
    pub burning: bool,
    /// AI 状态机（每只一份）。
    pub brain: Brain,
}

impl Mob {
    pub fn new(def: MobId, pos: Vec3) -> Self {
        Self {
            def,
            pos,
            vel: Vec3::ZERO,
            yaw: 0.0,
            health: def.def().health,
            on_ground: false,
            invulnerable: 0,
            last_hurt: 0.0,
            idle_ticks: 0,
            burning: false,
            brain: Brain::new(),
        }
    }

    pub fn def(&self) -> &'static MobDef {
        self.def.def()
    }
}
