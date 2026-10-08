//! Mobs: definitions, spawn rules (NaturalSpawner, MC 26.1), combat math
//! (Player.attack / LivingEntity.hurt / CombatRules), hostile AI state machine
//! (Goal-system equivalent, simplified) and death drops.
//! All constants from /root/mc-ref/NOTES-mobs.md & NOTES-2.md (source line refs).

pub mod ai;
pub mod combat;
pub mod components;
pub mod defs;
pub mod drops;
pub mod spawner;

pub use ai::{ai_table, AiAction, Brain, MobAiTable, MobState, Percept};
pub use components::{
    register_mob_components, spawn_mob, Health, LastHurt, MobKind, MobTicks, PhysBody, Yaw,
};
pub use defs::{MobDef, MobId, MOBS};
pub use drops::{death_drops, DropEvent};
pub use spawner::{SpawnCategory, SpawnConfig, SpawnRule, SpawnWorld, Spawned, SPAWN_CAPS};
