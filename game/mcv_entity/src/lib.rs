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

pub use ai::{AiAction, Brain, MobAiTable, MobState, Percept, ai_table};
pub use components::{
    Health, LastHurt, MobKind, MobTicks, PhysBody, Yaw, register_mob_components, spawn_mob,
};
pub use defs::{MOBS, MobDef, MobId};
pub use drops::{DropEvent, death_drops};
pub use spawner::{SPAWN_CAPS, SpawnCategory, SpawnConfig, SpawnRule, SpawnWorld, Spawned};
