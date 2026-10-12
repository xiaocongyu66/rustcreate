//! Mobs: definitions, spawn rules (NaturalSpawner, MC 26.1), combat math
//! (Player.attack / LivingEntity.hurt / CombatRules), hostile AI state machine
//! (Goal-system equivalent, simplified) and death drops.
//! All constants from mc-ref/NOTES-mobs.md & NOTES-2.md (source line refs).

pub mod ai;
pub mod arrow;
pub mod combat;
pub mod components;
pub mod defs;
pub mod drops;
pub mod effect;
pub mod items;
pub mod passive;
pub mod pathfinding;
pub mod spawner;
pub mod xp;

pub use ai::{AiAction, Brain, MobAiTable, MobState, Percept, ai_table};
pub use arrow::{BOW_MIN_POWER, crit_bonus, hit_damage, mob_base_damage, ranged_power, triangle};
pub use components::{
    Effects, Health, LastHurt, MobArrow, MobBrain, MobIntent, MobKind, MobPath, MobTicks, PhysBody,
    Yaw, register_mob_components, spawn_mob,
};
pub use defs::{MOBS, MobDef, MobId};
pub use drops::{DropEvent, death_drops};
pub use effect::{
    ALL_KINDS, Bundle, DeathBurst, EffectBook, Harm, Kind, MAX_AMPLIFIER, apply_instant,
    apply_tick, attribute_modifier, blend_durations, blend_step, category, color, death_burst,
    fires_this_tick, hurt_burst, is_instant, slimes_to_spawn, wind_burst_power,
};
pub use items::{
    DEATH_PICKUP_DELAY, DESPAWN_AGE, DropWorld, ITEM_HALF, ItemDrop, ItemDropSnap, PICKUP_DELAY,
    PICKUP_INFLATE_XZ, PICKUP_INFLATE_Y, PickupReq, STEPS_PER_TICK, collect_item_drop_snaps,
    item_merge_system, item_physics_system, item_pickup_system, register_drop_components,
    settle_pickups, spawn_item_drop,
};
pub use passive::{
    AnimState, EAT_GRASS_INTERVAL, EAT_GRASS_TICKS, PANIC_MAX_TICKS, PANIC_RADIUS,
    PLAYER_WALK_SPEED, PassiveMode, PassiveServices, STROLL_INTERVAL, WanderState,
    panic_speed_mult, passive_ai_system, register_passive_components, spawn_passive_mob,
    update_wing,
};
pub use pathfinding::{BASE_FALL_DISTANCE, Path, PathParams, find_path, max_fall_distance};
pub use spawner::{SPAWN_CAPS, SpawnCategory, SpawnConfig, SpawnRule, SpawnWorld, Spawned};
pub use xp::{
    TAKE_XP_DELAY_TICKS, XP_ORB_DESPAWN_AGE, XP_ORB_GRAVITY_SCALE, XP_ORB_HALF, XpOrb, XpOrbPickup,
    award, get_experience_value, register_xp_components, spawn_xp_orb, xp_orb_physics_system,
    xp_orb_pickup_system,
};
