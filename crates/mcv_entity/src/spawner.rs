//! Natural spawn rules (NOTES-2 §2, NaturalSpawner.java line refs).
//!
//! Loop shape (26.1): per eligible chunk, 3 spawn "groups"; each group walks
//! up to 4 candidate positions with ±(next(6)-next(6)) jitter; category
//! filters: global cap, local density, light rules, distance rules.

use glam::Vec3;

/// Per-category global caps (MobCategory.java, 26.1 decompiled enum).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnCategory {
    Monster,
    Creature,
    Ambient,
    WaterAmbient,
    Misc,
}

/// (cap, despawn_distance, persistent) from MobCategory.java:40-48.
pub const SPAWN_CAPS: [(SpawnCategory, u32, i32, bool); 5] = [
    (SpawnCategory::Monster, 70, 128, false),
    (SpawnCategory::Creature, 10, 128, true),
    (SpawnCategory::Ambient, 15, 128, true),
    (SpawnCategory::WaterAmbient, 20, 64, true),
    (SpawnCategory::Misc, 0, 128, true), // MISC cap -1 = 不参与自然生成
];

pub const GROUPS_PER_CHUNK: usize = 3;
pub const ATTEMPTS_PER_GROUP: usize = 4;
/// 距玩家 24 格内不刷（distSqr <= 576，NaturalSpawner.java:60）。
pub const MIN_PLAYER_DIST_SQR: f32 = 576.0;
/// 生成范围 8 chunk = 128 格（:61）。
pub const SPAWN_RANGE_CHUNKS: i32 = 8;

/// Light rule for monster spawns (Monster.isDarkEnoughToSpawn, 26.1):
/// - sky light > rand(32) → reject (i.e. sky 0 always passes, 15 passes 1/32)
/// - block light > monster limit → reject
/// - overall brightness <= rand(8) sampler
/// 26.1 的 sampler 具体值在 DimensionType JSON（未提取）——按经典语义实现：
/// block light > 0 拒绝（近似），sky 用 rand(32)，最终 brightness <= rand(8)。
/// 标注 TODO(research)：DimensionType monsterSettings JSON。
#[derive(Clone, Copy, Debug)]
pub struct SpawnRule {
    pub category: SpawnCategory,
    pub hostile: bool,
}

/// 检查一个候选位置能否刷怪（纯函数，便于测试）。
pub fn light_allows_hostile(sky_light: u8, block_light: u8, rng: &mut impl FnMut() -> u32) -> bool {
    // Monster.java:79-81: if (sky > random.nextInt(32)) return false
    if u32::from(sky_light) > rng() % 32 {
        return false;
    }
    // block light limit（26.1 默认 overworld 为 0）
    if block_light > 0 {
        return false;
    }
    // :89-90 overall maxLocalRawBrightness <= sampler(8)
    let overall = u32::from(sky_light.max(block_light));
    overall <= rng() % 8
}

/// Despawn (Mob.java:668-687):
/// - 距最近玩家 > despawn² 立即移除（removeWhenFarAway）
/// - noDespawn 距离内重置 idle；idle > 600 时每 tick 1/800 概率消失
pub fn should_despawn(
    dist_sqr_to_player: f32,
    idle_ticks: u64,
    despawn_dist: i32,
    no_despawn_dist: i32,
    rng: &mut impl FnMut() -> u32,
) -> bool {
    let d2 = despawn_dist as f32 * despawn_dist as f32;
    let nd2 = no_despawn_dist as f32 * no_despawn_dist as f32;
    if dist_sqr_to_player > d2 {
        return true;
    }
    if dist_sqr_to_player < nd2 {
        return false;
    }
    if idle_ticks > 600 {
        return rng() % 800 == 0;
    }
    false
}

/// 组内候选位置游走：每步 x/z 各 next(6)-next(6)（:164-190）。
pub fn group_walk(start: Vec3, rng: &mut impl FnMut() -> u32) -> Vec3 {
    let jx = (rng() % 6) as f32 - (rng() % 6) as f32;
    let jz = (rng() % 6) as f32 - (rng() % 6) as f32;
    Vec3::new(start.x + jx, start.y, start.z + jz)
}

/// 白天燃烧（Mob.java:478-512，26.1 环境属性版）：
/// 夜晚不燃；白天对暴露天空者 `rand*30 < (brightness-0.4)*2` 判定，燃 8s。
/// brightness 为 0..1 环境值；sky=15 时 brightness≈1 → (1-0.4)*2=1.2 →
/// rand<0.04 即 4%/tick。TODO(research)：EnvironmentAttributeSystem 曲线。
pub fn burn_in_daylight(
    sky_exposed: bool,
    brightness: f32,
    day: bool,
    rng: &mut impl FnMut() -> u32,
) -> bool {
    day && sky_exposed && rng() as f32 / u32::MAX as f32 * 30.0 < (brightness - 0.4) * 2.0
}
