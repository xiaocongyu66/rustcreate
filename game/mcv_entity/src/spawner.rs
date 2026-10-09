//! Natural spawn rules (NOTES-mobs.md §5, NaturalSpawner.java / Monster.java line refs).
//!
//! Loop shape (26.1): per eligible chunk, 3 spawn "groups"; each group walks
//! up to 4 candidate positions with ±(next(6)-next(6)) jitter; category
//! filters: global cap, local density, light rules, distance rules.
//! 逻辑层简化：不遍历 chunk，按玩家环形带 (24,128] 随机采样，常数照抄原版。

use glam::Vec3;
use mcv_core::BlockPos;

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
///
/// 26.1 的 sampler 具体值在 DimensionType JSON（未提取）——按经典语义实现：
/// block light > 0 拒绝（近似），sky 用 rand(32)，最终 brightness <= rand(8)。
/// TODO(research)：DimensionType monsterSettings JSON。
#[derive(Clone, Copy, Debug)]
pub struct SpawnRule {
    pub category: SpawnCategory,
    pub hostile: bool,
}

/// MobCategory.java:7 — MONSTER despawnDistance=128（`>128²` 立即移除，
/// Mob.java:662-666）。
pub const DESPAWN_DIST: i32 = 128;
/// MobCategory.java:21/57 — noDespawnDistance=32（`<32²` 清 idle 账本；
/// idle>600 且 1/800 移除只在 `>32²` 生效，Mob.java:668-674）。
/// 注意：派单/审计报告写 24..32，源码实况为 32（`getNoDespawnDistance()`
/// 恒返 32，无 24 一说）。
pub const NO_DESPAWN_DIST: i32 = 32;
/// Animal.java:118-120 — 被动 `getRawBrightness(pos, 0) > 8` 才可生成。
pub const ANIMAL_MIN_RAW_BRIGHTNESS: u8 = 8;

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
    idle_ticks > 600 && rng().is_multiple_of(800)
}

/// noActionTime 每 tick 增量：基础 +1（Mob.java:683 serverAiStep），敌对
/// 在亮处（magic light br>0.5）额外 +2（Monster.java:50-54
/// updateNoActionTime，每 tick aiStep 调用）。
pub fn no_action_inc(hostile: bool, br: f32) -> u64 {
    if hostile && br > 0.5 { 3 } else { 1 }
}

/// `getRawBrightness(pos, skyDarken)`（LevelReader.java:167-175）：
/// `max(sky − skyDarken, block)`。skyDarken 来自 `Level.java:736`
/// `15 − SKY_LIGHT_LEVEL`（Timelines.java:81-84 白天 15/夜 4 → darken
/// 0..11；由接线侧按时间提供）。
pub fn raw_brightness(sky: u8, block: u8, sky_darken: u8) -> u8 {
    sky.saturating_sub(sky_darken).max(block)
}

/// 组内候选位置游走：每步 x/z 各 next(6)-next(6)（:164-190）。
pub fn group_walk(start: Vec3, rng: &mut impl FnMut() -> u32) -> Vec3 {
    let jx = (rng() % 6) as f32 - (rng() % 6) as f32;
    let jz = (rng() % 6) as f32 - (rng() % 6) as f32;
    Vec3::new(start.x + jx, start.y, start.z + jz)
}

/// 白天燃烧（Mob.java:480-513，26.1 环境属性曲线版）：
/// 白天(MONSTERS_BURN) && 眼睛可见天 && !sheltered(入水/细雪/头盔) &&
/// `br > 0.5 && rand·30 < (br−0.4)·2` → 点燃 8s。
/// br = magic_light(maxLocalRawBrightness)，见 [`magic_light`]。
pub fn burn_in_daylight(
    sky_exposed: bool,
    sheltered: bool,
    br: f32,
    day: bool,
    rng: &mut impl FnMut() -> u32,
) -> bool {
    if !(day && sky_exposed) || sheltered {
        return false;
    }
    br > 0.5 && rng() as f32 / u32::MAX as f32 * 30.0 < (br - 0.4) * 2.0
}

/// `LevelReader.getLightLevelDependentMagicValue`（LevelReader.java:113-117）：
/// `v = raw/15; br = v/(4−3v)`（overworld ambient=0 时的 lerp 退化形式）。
/// raw 为 maxLocalRawBrightness（0..15）。
pub fn magic_light(raw: u8) -> f32 {
    let v = raw.min(15) as f32 / 15.0;
    v / (4.0 - 3.0 * v)
}

// ---------------------------------------------------------------------------
// 26.1 精确刷怪（Monster.isDarkEnoughToSpawn + NaturalSpawner 常数）
// ---------------------------------------------------------------------------

/// NaturalSpawner.java:61 — MAGIC_NUMBER = 17²，全局配额分母。
pub const MAGIC_NUMBER: u32 = 289;
/// NaturalSpawner.java:239 — 世界出生点 24 格内不刷。
pub const RESPAWN_REJECT_DIST: f32 = 24.0;
/// NaturalSpawner.java:59 — 生成外半径 128（内半径见 MIN_PLAYER_DIST_SQR）。
pub const MAX_SPAWN_DIST: f32 = 128.0;

/// 全局配额：`cap × spawnableChunks / 289`（NaturalSpawner.java:537）。
pub fn global_cap_total(spawnable_chunks: u32, cap: u32) -> u32 {
    cap * spawnable_chunks / MAGIC_NUMBER
}

/// 世界查询接口（由主控接线：光照引擎 + 生成位置扫描）。
pub trait SpawnWorld {
    /// LightLayer.SKY（0..15）。
    fn sky_light(&self, p: BlockPos) -> u8;
    /// LightLayer.BLOCK（0..15）。
    fn block_light(&self, p: BlockPos) -> u8;
    /// maxLocalRawBrightness（雷暴修正 `getMaxLocalRawBrightness(pos,10)`，
    /// Monster.java:97，由接线侧统一处理）。
    fn max_local_raw_brightness(&self, p: BlockPos) -> u8 {
        self.sky_light(p).max(self.block_light(p))
    }
    fn can_see_sky(&self, p: BlockPos) -> bool;
    /// 在 (x,z) 柱内找可生成格（下方实心 + 自身及头顶空气，SpawnPlacements
    /// ON_GROUND 语义）；找不到返回 None。
    fn find_spawn_pos(&self, x: i32, z: i32) -> Option<BlockPos>;
}

/// 刷怪配置（原版常数默认，和平/创造豁免暴露给调用方）。
#[derive(Clone, Copy, Debug)]
pub struct SpawnConfig {
    /// DimensionType.monsterSpawnBlockLightLimit（overworld 经典 = 0；
    /// 维度 json 未提取，TODO(research)）。
    pub block_light_limit: u8,
    /// DimensionType.monsterSpawnLightTest = uniform(0..=7) → 亮度 ≤ rand(8)。
    pub light_test_max: u8,
    /// 距最近玩家 ≤ 24 格拒刷（distSqr ≤ 576，NaturalSpawner.java:234）。
    pub min_dist_sqr: f32,
    pub max_dist_sqr: f32,
    /// 世界出生点（Some 时其 24 格内拒刷）。
    pub respawn_center: Option<Vec3>,
    /// MobCategory.MONSTER 上限 70（MobCategory.java:7）。
    pub monster_cap: u32,
    /// NaturalSpawner.java:170 — 每起点 3 组。
    pub groups_per_round: usize,
    /// NaturalSpawner.java:176 — 初始 ceil(rand×4) 上限 = 4 步。
    pub walks_per_group: usize,
    /// 组内数量 min..=max（原版来自生物群系 spawndata，默认 1..4）。
    pub group_min: usize,
    pub group_max: usize,
    /// 和平难度不刷怪（Monster.java:104）。
    pub peaceful: bool,
    /// 创造/游戏规则豁免（ServerChunkCache.spawnEnemies，调用方控制）。
    pub spawn_enemies: bool,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        Self {
            block_light_limit: 0,
            light_test_max: 7,
            min_dist_sqr: MIN_PLAYER_DIST_SQR,
            max_dist_sqr: MAX_SPAWN_DIST * MAX_SPAWN_DIST,
            respawn_center: None,
            monster_cap: 70,
            groups_per_round: GROUPS_PER_CHUNK,
            walks_per_group: ATTEMPTS_PER_GROUP,
            group_min: 1,
            group_max: 4,
            peaceful: false,
            spawn_enemies: true,
        }
    }
}

/// Monster.isDarkEnoughToSpawn（Monster.java:86-99）逐步照抄：
/// 1. `sky > rand(32)` 拒；2. `limit < 15 && block > limit` 拒；
/// 3. `brightness <= rand(light_test_max+1)`。
pub fn is_dark_enough(
    sky: u8,
    block: u8,
    brightness: u8,
    cfg: &SpawnConfig,
    rng: &mut impl FnMut() -> u32,
) -> bool {
    if u32::from(sky) > rng() % 32 {
        return false;
    }
    if cfg.block_light_limit < 15 && block > cfg.block_light_limit {
        return false;
    }
    u32::from(brightness) <= rng() % (u32::from(cfg.light_test_max) + 1)
}

/// 玩家环形带随机采样：r² 均匀落在 (min², max²]（原版按 chunk 采样 +
/// ≤576 拒绝，此处直接环形采样，分布等价）。返回 (x, z, r²)。
pub fn annulus_pos(
    player: Vec3,
    cfg: &SpawnConfig,
    rng: &mut impl FnMut() -> u32,
) -> (f32, f32, f32) {
    let r1 = rng() as f64 / u32::MAX as f64;
    let r2 = rng() as f64 / u32::MAX as f64;
    let lo = (cfg.min_dist_sqr + 1.0) as f64;
    let hi = cfg.max_dist_sqr as f64;
    let d2 = lo + r1 * (hi - lo);
    let ang = r2 * std::f64::consts::TAU;
    let d = d2.sqrt() as f32;
    (
        player.x + ang.cos() as f32 * d,
        player.z + ang.sin() as f32 * d,
        d2 as f32,
    )
}

/// 一次刷怪尝试的输出。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spawned {
    pub kind: crate::defs::MobId,
    pub pos: BlockPos,
}

/// 一轮刷怪（调用方每 N tick 触发；N≈20 即每秒一轮）。
/// 结构照抄 spawnCategoryForPosition：每组先环形采样起点，随后
/// ≤walks 步 ±(6,6) jitter 游走；组内类型首次成功后固定；
/// 组数量 group_min..=group_max；受全局配额与 24/128 距离、亮度约束。
pub fn spawn_round(
    world: &dyn SpawnWorld,
    players: &[Vec3],
    alive_monsters: u32,
    spawnable_chunks: u32,
    kinds: &[crate::defs::MobId],
    cfg: &SpawnConfig,
    rng: &mut impl FnMut() -> u32,
) -> Vec<Spawned> {
    let mut out = Vec::new();
    // 和平难度 / 关闭敌对生成（创造豁免）→ 直接空（Monster.java:104）。
    if cfg.peaceful || !cfg.spawn_enemies || players.is_empty() || kinds.is_empty() {
        return out;
    }
    let cap_total = global_cap_total(spawnable_chunks, cfg.monster_cap);
    let mut room = cap_total.saturating_sub(alive_monsters);
    if room == 0 {
        return out;
    }
    for _group in 0..cfg.groups_per_round {
        if room == 0 {
            break;
        }
        let player = players[(rng() as usize) % players.len()];
        let (px, pz, _) = annulus_pos(player, cfg, rng);
        let (mut cx, mut cz) = (px.floor() as i32, pz.floor() as i32);
        let mut kind: Option<usize> = None;
        let mut want = 0usize;
        let mut got = 0usize;
        for _walk in 0..cfg.walks_per_group {
            if (want != 0 && got >= want) || room == 0 {
                break;
            }
            // NaturalSpawner.java:180-181 — 步进 jitter ±(next(6)-next(6))。
            cx += (rng() % 6) as i32 - (rng() % 6) as i32;
            cz += (rng() % 6) as i32 - (rng() % 6) as i32;
            let Some(pos) = world.find_spawn_pos(cx, cz) else {
                continue;
            };
            let center = Vec3::new(pos.x as f32 + 0.5, pos.y as f32, pos.z as f32 + 0.5);
            // 距最近玩家检查（getNearestPlayer 语义，NaturalSpawner.java:185-188）。
            let d2 = players
                .iter()
                .map(|q| q.distance_squared(center))
                .fold(f32::INFINITY, f32::min);
            if d2 <= cfg.min_dist_sqr || d2 > cfg.max_dist_sqr {
                continue;
            }
            if let Some(r) = cfg.respawn_center
                && r.distance_squared(center) <= RESPAWN_REJECT_DIST * RESPAWN_REJECT_DIST
            {
                continue;
            }
            if !is_dark_enough(
                world.sky_light(pos),
                world.block_light(pos),
                world.max_local_raw_brightness(pos),
                cfg,
                rng,
            ) {
                continue;
            }
            if kind.is_none() {
                // 组类型固定（原版按生物群系权重；此处均匀采样 kinds）。
                kind = Some((rng() as usize) % kinds.len());
                want = cfg.group_min + (rng() as usize) % (cfg.group_max - cfg.group_min + 1);
            }
            out.push(Spawned {
                kind: kinds[kind.unwrap()],
                pos,
            });
            got += 1;
            room -= 1;
        }
    }
    out
}
