//! 水生生物（Cod/Salmon/TropicalFish + Axolotl）：变体打包、离水窒息
//! （26.1 dry-out）、游动 AI（`RandomSwimmingGoal` 等价随机游 +
//! `PanicGoal`/`AvoidEntityGoal` 逃离）与离水扑腾（`AbstractFish.aiStep`）。
//!
//! 与 game.rs `mob_ai_system` 的关系（同 passive.rs 的双轨约定）：
//! - 本模块的系统只处理持有 [`AquaticMode`] 的实体（由
//!   [`spawn_aquatic_mob`] 装配）；game.rs Brain 路线在实体门处跳过
//!   （ aquatic 怪无 Brain 索敌面），两套运行时互不触碰；
//! - 物理固定步复用 `mcv_game::step_entity`：水中喂 `in_water: true`
//!   （引擎游泳律 LivingEntity.travelInWater 等价面），离水走地面律
//!   + 扑腾跳；
//! - 离水窒息：[`dry_out_step`] 照抄 WaterAnimal.handleAirSupply
//!   （air−1/t → air≤−20 结算 2.0 伤害并清零）+ Axolotl 覆写上限
//!   6000（:86 AXOLOTL_TOTAL_AIR_SUPPLY）。
//!
//! 常数出处（mc-ref/src-26.1）：
//! - `AbstractFish.java`：travelInWater（moveRelative 0.01 + 阻尼 0.9）、
//!   aiStep 扑腾（:117-127 ±0.05 水平 / +0.4 竖直 / 播 flop 音）、
//!   registerGoals（:92-96 Panic 1.25 / Avoid 8.0,1.6,1.4 / FishSwimGoal
//!   interval 40）、FishMoveControl（eye-in-water +0.005 浮力、rotlerp
//!   90°/tick）、MAX_HEALTH 3.0、getMaxSpawnClusterSize 8；
//! - `WaterAnimal.java`：handleAirSupply（:46-57）、
//!   checkSurfaceWaterAnimalSpawnRules（:66-79：y∈[sea−13, sea] + 上下
//!   邻水）、getBaseExperienceReward（1+rand(3)）；
//! - `TropicalFish.java`：packVariant（:59-63）、COMMON_VARIANTS（:24-47，
//!   22 组常见变体）、finalizeSpawn 90% 常见 / 10% 全随机（:317-338）；
//! - `Salmon.java`：Variant 30/50/15 权重 + 0.5/1.0/1.5 缩放（:129-138,
//!   :190-200）；
//! - `Axolotl.java`：Variant 5 型（:626-631，BLUE 稀有）、useRareVariant
//!   1/1200（:308-310）、handleAirSupply 上限 6000（:86）、
//!   checkAxolotlSpawnRules（下方块 ∈ AXOLOTLS_SPAWNABLE_ON = clay）。
//!
//! KNOWN-DIVERGENCE（引擎边界，报告详注）：
//! - 鱼的巡航速度：26.1 由 moveRelative(0.01)+阻尼 0.9 自洽收敛（≈1.2
//!   m/s），引擎游泳律按玩家 SWIM_SPEED(1.6 m/s) 一阶收敛——用 wish
//!   幅值 0.75/1.0 近似巡航/逃速；
//! - 鱼变体只到 [`DropEvent::variant`]（物品栈组件持久化待 mcv_item
//!   inventory schema 扩容，拾起后重放出变体属二期）。

use crate::defs::MobId;
use glam::Vec3;
use mcv_ecs::{Entity, World};
use std::collections::HashMap;
use std::sync::Arc;

use mcv_core::{BlockId, BlockPos, ChunkHandle, ChunkPos, Stage};
use mcv_game::{VoxelAccess, step_entity};

// ---- 原版常数 ---------------------------------------------------------------

/// AbstractFish.registerGoals — FishSwimGoal = RandomSwimmingGoal(mob, 1.0, 40)
/// （interval 40：每 tick `nextInt(40)==0` 才起游，RandomStrollGoal.java:47）。
pub const SWIM_INTERVAL: u32 = 40;
/// RandomSwimmingGoal.getPosition — BehaviorUtils.getRandomSwimmablePos(10, 7)
/// 水平半径（DefaultRandomPos.getPos 的水平域）。
pub const SWIM_RADIUS: f32 = 10.0;
/// 垂直域（同上，±7 格；引擎水下找点取 ±3 保证贴地可见性，原版垂直
/// 掷域见 DefaultRandomPos.getPos(mob, 10, 7)）。
pub const SWIM_RADIUS_V: f32 = 3.0;
/// AbstractFish.registerGoals:92 — PanicGoal(this, 1.25) 速度倍率。
pub const PANIC_SPEED: f32 = 1.25;
/// AvoidEntityGoal(this, Player, 8.0, 1.6, 1.4) 的警戒半径（:93）。
pub const AVOID_RADIUS_SQR: f32 = 64.0;
/// PanicGoal/Avoid 起跑后的兜底时长（原版跑到导航完成；直线移动给 5 s 上界，
/// 与 passive.rs PANIC_MAX_TICKS 同约）。
pub const FLEE_MAX_TICKS: u32 = 100;
/// 离水扑腾水平冲量（AbstractFish.aiStep:120 `(rand*2−1)*0.05` 块/tick；
/// 每秒重掷 20 次 + 地表阻力 → 净水平 ≈1.1 m/s，wish 幅值 0.25×WALK 4.317
/// ≈ 1.1 m/s 对齐，KNOWN-DIVERGENCE：引擎地表律收敛阶不同）。
pub const FLOP_WISH: f32 = 0.25;
/// 离水扑腾竖直冲量（:121 +0.4 块/tick ≈ 8 m/s ≈ 引擎 JUMP_SPEED 8.4）。
pub const FLOP_JUMP_SPEED: f32 = 0.4;
/// WaterAnimal.handleAirSupply 窒息伤害（hurtServer drown/dryOut, 2.0F）。
pub const DRYOUT_DMG: f32 = 2.0;
/// LivingEntity.shouldTakeDrowningDamage（:487-489）— air ≤ −20 才结算。
pub const DROWN_GRACE: i32 = 20;
/// Axolotl.java:308-310 — useRareVariant：`nextInt(1200) == 0`（繁殖路径）。
pub const AXOLOTL_RARE_ONE_IN: u32 = 1200;
/// TropicalFish.finalizeSpawn — 90% 走 COMMON_VARIANTS，10% 全随机（:326）。
pub const TROPICAL_COMMON_PCT: u32 = 90;
/// AbstractFish.getMaxSpawnClusterSize — 8（AbstractSchoolingFish 群上限；
/// Salmon 覆写 5，getMaxSchoolSize:44）。
pub const FISH_CLUSTER: u32 = 8;
pub const SALMON_SCHOOL: u32 = 5;
/// 引擎游泳律 wish 幅值（SWIM_SPEED 1.6 m/s 的比例近似：巡航 ≈1.2 m/s、
/// 逃跑 ≈1.6 m/s——26.1 moveRelative(0.01)+阻尼 0.9 稳态近似，见模块头）。
pub const SWIM_WISH_CRUISE: f32 = 0.75;
pub const SWIM_WISH_FLEE: f32 = 1.0;
/// 美西螈地面行走速度钳制（Axolotl.createAttributes MOVEMENT_SPEED 1.0 在
/// 引擎 43.17 换算律下 ≈43 m/s 无意义；取玩家步速——KNOWN-DIVERGENCE，
/// 引擎移动律换算差异）。
pub const AXOLOTL_LAND_SPEED: f32 = 4.317;
/// 美西螈地面行走 wish 幅值（WALK_SPEED 4.317 × 0.4 ≈ 1.73 m/s，见上注）。
pub const AXOLOTL_LAND_WISH: f32 = 0.4;

// ---- 热带鱼变体表 -----------------------------------------------------------

/// TropicalFish 变体打包/解包（TropicalFish.java:59-72 逐式照抄）：
/// `packed = pattern.packedId & 0xFFFF | baseColor<<16 | patternColor<<24`，
/// 其中 `pattern.packedId = base.id | index<<8`（Base: SMALL 0 / LARGE 1，
/// index 为该 base 内 0..5）。DyeColor id 按 DyeColor.java 枚举序
/// （white 0 .. black 15）。
pub mod tropical {
    /// Pattern 名（26.1 枚举序）：small（KOB..SPOTTY）在前、large
    /// （FLOPPER..CLAYFISH）在后；`base`/`index` 见源码构造器。
    pub const SMALL_PATTERNS: [&str; 6] =
        ["kob", "sunstreak", "snooper", "dasher", "brinely", "spotty"];
    pub const LARGE_PATTERNS: [&str; 6] = [
        "flopper",
        "stripey",
        "glitter",
        "blockfish",
        "betty",
        "clayfish",
    ];
    /// DyeColor.id（white..black 0..15，与 DyeColor.java 枚举序一致）。
    pub const DYE_WHITE: u8 = 0;
    pub const DYE_ORANGE: u8 = 1;
    pub const DYE_LIGHT_BLUE: u8 = 3;
    pub const DYE_YELLOW: u8 = 4;
    pub const DYE_LIME: u8 = 5;
    pub const DYE_PINK: u8 = 6;
    pub const DYE_GRAY: u8 = 7;
    pub const DYE_CYAN: u8 = 9;
    pub const DYE_PURPLE: u8 = 10;
    pub const DYE_BLUE: u8 = 11;
    pub const DYE_RED: u8 = 14;

    /// packVariant（TropicalFish.java:59-63）：pattern index(0..5) + 大小旗
    /// + 底色 + 花纹色。
    pub const fn pack(pattern_index: u8, large: bool, base_color: u8, pattern_color: u8) -> u32 {
        let pattern = ((large as u16) << 8) | (pattern_index as u16);
        (pattern as u32) | ((base_color as u32) << 16) | ((pattern_color as u32) << 24)
    }
    /// getPattern — `byId(packed & 0xFFFF)` → (is_large, index)。
    pub fn unpack_pattern(packed: u32) -> (bool, u8) {
        let p = (packed & 0xFFFF) as u16;
        ((p >> 8) != 0, (p & 0xFF) as u8)
    }
    /// getBaseColor — `byId(packed >> 16 & 0xFF)`。
    pub fn base_color(packed: u32) -> u8 {
        ((packed >> 16) & 0xFF) as u8
    }
    /// getPatternColor — `byId(packed >> 24 & 0xFF)`。
    pub fn pattern_color(packed: u32) -> u8 {
        ((packed >> 24) & 0xFF) as u8
    }
    /// DEFAULT_VARIANT（TropicalFish.java:22）— KOB + WHITE + WHITE。
    pub const DEFAULT: u32 = pack(0, false, DYE_WHITE, DYE_WHITE);

    /// COMMON_VARIANTS 22 组（TropicalFish.java:24-47 源序），每项 =
    /// (pattern 名, is_large, index, baseColor, patternColor)。
    pub const COMMON: [(&str, bool, u8, u8, u8); 22] = [
        ("stripey", true, 1, DYE_ORANGE, DYE_GRAY),
        ("flopper", true, 0, DYE_GRAY, DYE_GRAY),
        ("flopper", true, 0, DYE_GRAY, DYE_BLUE),
        ("clayfish", true, 5, DYE_WHITE, DYE_GRAY),
        ("sunstreak", false, 1, DYE_BLUE, DYE_GRAY),
        ("kob", false, 0, DYE_ORANGE, DYE_WHITE),
        ("spotty", false, 5, DYE_PINK, DYE_LIGHT_BLUE),
        ("blockfish", true, 3, DYE_PURPLE, DYE_YELLOW),
        ("clayfish", true, 5, DYE_WHITE, DYE_RED),
        ("spotty", false, 5, DYE_WHITE, DYE_YELLOW),
        ("glitter", true, 2, DYE_WHITE, DYE_GRAY),
        ("clayfish", true, 5, DYE_WHITE, DYE_ORANGE),
        ("dasher", false, 3, DYE_CYAN, DYE_PINK),
        ("brinely", false, 4, DYE_LIME, DYE_LIGHT_BLUE),
        ("betty", true, 4, DYE_RED, DYE_WHITE),
        ("snooper", false, 2, DYE_GRAY, DYE_RED),
        ("blockfish", true, 3, DYE_RED, DYE_WHITE),
        ("flopper", true, 0, DYE_WHITE, DYE_YELLOW),
        ("kob", false, 0, DYE_RED, DYE_WHITE),
        ("sunstreak", false, 1, DYE_GRAY, DYE_WHITE),
        ("dasher", false, 3, DYE_CYAN, DYE_YELLOW),
        ("flopper", true, 0, DYE_YELLOW, DYE_YELLOW),
    ];

    /// COMMON_VARIANTS 的 packed 值（渲染/掉落直取）。
    pub const COMMON_PACKED: [u32; 22] = {
        let mut out = [0u32; 22];
        let mut i = 0;
        while i < 22 {
            out[i] = pack(COMMON[i].2, COMMON[i].1, COMMON[i].3, COMMON[i].4);
            i += 1;
        }
        out
    };

    /// finalizeSpawn（:325-338）：90% 从 COMMON_VARIANTS 均匀取（组内同变
    /// 体由 group data 复制——引擎刷怪组直接复用同一 packed），10% 全随机
    /// （pattern 12 × baseColor 16 × patternColor 16）。
    pub fn finalize_variant(rng: &mut impl FnMut() -> u32) -> u32 {
        if rng() % 100 < super::TROPICAL_COMMON_PCT {
            COMMON_PACKED[(rng() as usize) % COMMON_PACKED.len()]
        } else {
            let index = (rng() % 6) as u8;
            let large = rng().is_multiple_of(2);
            pack(index, large, (rng() % 16) as u8, (rng() % 16) as u8)
        }
    }

    /// 渲染贴图选择（TropicalFishRenderer/TropicalFishPatternLayer）：小
    /// 型贴 tropical_a(.png / _pattern_N)，大型贴 tropical_b。index 0 =
    /// 素底（KOB→a / FLOPPER→b），index 1..5 → pattern_1..6。返回
    /// (is_large, pattern_layer 0..6，0=素底)。
    pub fn texture_slot(packed: u32) -> (bool, u8) {
        let (large, index) = unpack_pattern(packed);
        (large, index.min(5))
    }
}

/// Salmon.Variant（Salmon.java:190-200）：id 0/1/2 = small/medium/large，
/// boundingBoxScale 0.5/1.0/1.5；finalizeSpawn 权重 30/50/15（:129-136）。
pub mod salmon {
    pub const SMALL: u32 = 0;
    pub const MEDIUM: u32 = 1;
    pub const LARGE: u32 = 2;
    pub const DEFAULT: u32 = MEDIUM;

    /// getSalmonScale（:188-190）。
    pub fn scale(variant: u32) -> f32 {
        match variant {
            SMALL => 0.5,
            LARGE => 1.5,
            _ => 1.0,
        }
    }

    /// finalizeSpawn（:129-136）WeightedList：SMALL 30 / MEDIUM 50 /
    /// LARGE 15（累计 95 域内均匀采样）。
    pub fn finalize_variant(rng: &mut impl FnMut() -> u32) -> u32 {
        let r = rng() % 95;
        if r < 30 {
            SMALL
        } else if r < 80 {
            MEDIUM
        } else {
            LARGE
        }
    }
}

/// Axolotl.Variant（Axolotl.java:626-631）：LUCY/WILD/GOLD/CYAN common、
/// BLUE 稀有（spawn 只出 common，finalizeSpawn:156-178
/// getCommonSpawnVariant → Util.getRandom uniform）。
pub mod axolotl {
    pub const LUCY: u32 = 0;
    pub const WILD: u32 = 1;
    pub const GOLD: u32 = 2;
    pub const CYAN: u32 = 3;
    pub const BLUE: u32 = 4;
    pub const DEFAULT: u32 = LUCY;

    /// getCommonSpawnVariant：4 common 变体均匀（Util.getRandom）。
    pub fn spawn_variant(rng: &mut impl FnMut() -> u32) -> u32 {
        [LUCY, WILD, GOLD, CYAN][(rng() as usize) % 4]
    }

    /// useRareVariant（:308-310）——繁殖路径（本引擎未接繁殖，函数供
    /// 对拍与未来接线）。
    pub fn use_rare(rng: &mut impl FnMut() -> u32) -> bool {
        rng().is_multiple_of(super::AXOLOTL_RARE_ONE_IN)
    }
}

// ---- 组件 -------------------------------------------------------------------

/// 离水窒息 + 水状态账本（每只水生怪一份，20 tick/s 语义）。
#[derive(Clone, Copy, Debug)]
pub struct WaterState {
    /// 眼位/体心水位判定（每固定步刷新，供物理 in_water 与窒息门共用）。
    pub in_water: bool,
    /// 空气账（tick，可为负到 −20 门槛——LivingEntity.shouldTakeDrowningDamage）。
    pub air: i32,
    /// tick 年龄（尾摆 sin(0.6·age) 相位源——Cod/TropicalFish setupAnim）。
    pub age_ticks: u32,
}

impl WaterState {
    fn new(max_air: i32) -> Self {
        Self {
            in_water: true,
            air: max_air,
            age_ticks: 0,
        }
    }
}

/// 变体持久面（渲染贴图选择 + 掉落对拍）：
/// - TropicalFish：完整 packed variant（[`tropical`]）；
/// - Salmon：0/1/2（[`salmon`]）；
/// - Axolotl：0..4（[`axolotl`]）；Cod 恒 0。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AquaticVariant(pub u32);

/// 游动 AI 运行时（每只水生怪一份，20 Hz 语义；同 passive.rs
/// WanderState 的双轨约定）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AquaticBehavior {
    /// 静栖（缓沉/贴底）。
    Idle,
    /// 游向 [`AquaticMode::target`]（水内随机点）。
    Wander,
    /// 受击逃离 / 避玩家（远离威胁点直线冲刺）。
    Flee,
}

#[derive(Clone, Copy, Debug)]
pub struct AquaticMode {
    pub behavior: AquaticBehavior,
    /// 游走/逃离目标点（世界坐标；水内取点）。
    pub target: Vec3,
    /// Wander 到点预算（tick；导航未完成的兜底）。
    pub wander_ticks: u32,
    /// Flee 剩余 tick。
    pub flee_ticks: u32,
}

// ---- 宿主快照资源 -----------------------------------------------------------

/// 水生怪系统的宿主快照（同 [`crate::passive::PassiveServices`] 的耦合面；
/// ticket 表在 mcv_logic 域内，跨 crate 用模拟环格集快照解耦）。
pub struct AquaticServices {
    pub chunks: Arc<HashMap<ChunkPos, Arc<ChunkHandle>>>,
    pub player_pos: Vec3,
    /// 本固定步是否跨过 20 Hz tick 边界（AI 决策门）。
    pub on_tick: bool,
    /// xorshift64* 状态；宿主给初值，系统每步推进。
    pub seed: u64,
    /// 模拟环格集（PLAYER_SIMULATION 级别视图 ≤ ENTITY_TICKING 的 chunk
    /// 集，ServerLevel 实体门 ServerLevel.java:418-419 等价面）——环外
    /// 实体冻结（拍 pos_old 后跳过 AI/物理，渲染插值不回退）。
    pub sim_cells: Arc<std::collections::HashSet<ChunkPos>>,
}

/// 单区块只读视图（与 passive.rs `ChunkWorld` 同语义：未加载 = 石头保安全）。
struct AquaticWorld<'a> {
    chunks: &'a HashMap<ChunkPos, Arc<ChunkHandle>>,
}

/// 原版水方块 id（mcv_core blocks_gen：water = 5）。
const WATER_BLOCK: BlockId = BlockId(5);

impl VoxelAccess for AquaticWorld<'_> {
    fn block(&self, p: BlockPos) -> BlockId {
        let Some(chunk) = self.chunks.get(&p.chunk()) else {
            return BlockId(1);
        };
        if chunk.stage() == Stage::Empty {
            return BlockId(1);
        }
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&p.y) {
            return BlockId(1);
        }
        let [lx, _, lz] = p.local();
        chunk
            .voxels
            .read()
            .unwrap()
            .get_flat(mcv_core::vidx(lx, p.y, lz))
    }

    fn light(&self, _p: BlockPos) -> u8 {
        15
    }

    fn chunk_loaded(&self, c: ChunkPos) -> bool {
        self.chunks.contains_key(&c)
    }
}

/// xorshift64*（Marsaglia）——与 passive.rs 同实现（模块私有，防蒸发不共文件）。
fn xorshift(state: &mut u64) -> u64 {
    let mut x = if *state == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        *state
    };
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn rand01(state: &mut u64) -> f32 {
    (xorshift(state) >> 40) as f32 / (1u64 << 24) as f32
}

// ---- 窒息（dry-out） ---------------------------------------------------------

/// WaterAnimal.handleAirSupply（:46-57）+ LivingEntity.shouldTakeDrowningDamage
/// （:487-489）逐步照抄：
/// - 入水：air 重置上限（Axolotl 覆写 isInWaterOrRain——雨面未接，注记）；
/// - 离水：air −1/t；`air <= −20` → air 清零并结算一次 [`DRYOUT_DMG`]。
///
/// 返回 Some(伤害) 表示本 tick 需扣血（调用方走 Health/受击面）。
pub fn dry_out_step(air: &mut i32, max_air: i32, in_water: bool) -> Option<f32> {
    if in_water {
        *air = max_air;
        return None;
    }
    *air -= 1;
    if *air <= -DROWN_GRACE {
        *air = 0;
        Some(DRYOUT_DMG)
    } else {
        None
    }
}

/// 离水扑腾冲量（AbstractFish.aiStep:117-127）：`vel += (rand±0.05, +0.4,
/// rand±0.05)` 并清 on_ground。返回 (水平 wish 方向, 需起跳)——引擎把
/// +0.4 映到 jump 通道（JUMP_SPEED 8.4 ≈ 0.4 块/tick），水平 ±0.05 块/tick
/// 映到地表摩擦律 wish 幅值（见 [`FLOP_WISH`] 注）。
pub fn flop_impulse(rng: &mut u64) -> (Vec3, bool) {
    let jx = (rand01(rng) * 2.0 - 1.0) * 0.05;
    let jz = (rand01(rng) * 2.0 - 1.0) * 0.05;
    let d = Vec3::new(jx, 0.0, jz);
    if d.length_squared() < 1e-6 {
        (Vec3::ZERO, true)
    } else {
        (d.normalize() * FLOP_WISH, true)
    }
}

// ---- 装配 -------------------------------------------------------------------

/// 预注册水生组件表（GameRuntime 建 World 后调一次）。
pub fn register_aquatic_components(world: &mut World) {
    world.register::<WaterState>();
    world.register::<AquaticVariant>();
    world.register::<AquaticMode>();
}

/// 变体抽选（spawn 侧 finalizeSpawn 等价面）。`variant` 语义见
/// [`AquaticVariant`]。
pub fn finalize_variant(id: MobId, rng: &mut impl FnMut() -> u32) -> u32 {
    match id {
        MobId::SALMON => salmon::finalize_variant(rng),
        MobId::TROPICAL_FISH => tropical::finalize_variant(rng),
        MobId::AXOLOTL => axolotl::spawn_variant(rng),
        _ => 0,
    }
}

/// 装配一只水生怪：在 [`crate::spawn_mob`] 组件之上追加
/// [`WaterState`] + [`AquaticVariant`] + [`AquaticMode`]。
pub fn spawn_aquatic_mob(world: &mut World, id: MobId, pos: Vec3, variant: u32) -> Entity {
    let e = crate::spawn_mob(world, id, pos);
    world.insert(e, WaterState::new(id.def().air_supply as i32));
    world.insert(e, AquaticVariant(variant));
    world.insert(
        e,
        AquaticMode {
            behavior: AquaticBehavior::Idle,
            target: pos,
            wander_ticks: 0,
            flee_ticks: 0,
        },
    );
    e
}

// ---- 系统 -------------------------------------------------------------------

/// 水生怪固定步系统：AI 决策 20 Hz（`on_tick` 门）→ 写 [`MobIntent`]/[`Yaw`]；
/// 物理 60 Hz 每步照上一 tick 意图步进（水中走引擎游泳律，离水走地面律 +
/// 扑腾跳）；窒息账本每 tick 推进。只迭代持有 [`AquaticMode`] 的实体。
pub fn aquatic_ai_system(ctx: &mut mcv_ecs::SysCtx) {
    let Some(svc) = ctx.resources.get_mut::<AquaticServices>() else {
        return;
    };
    let world: &World = ctx.world;
    let commands = &mut *ctx.commands;
    let on_tick = svc.on_tick;
    let view = AquaticWorld {
        chunks: &svc.chunks,
    };
    let player_pos = svc.player_pos;
    // 确定性随机源：与 passive.rs 同约定（实体序稳定 → 全程可复现）。
    let mut rng_state = svc.seed;
    if rng_state == 0 {
        rng_state = 0x9E37_79B9_7F4A_7C15;
    }

    let (
        mut phys,
        kinds,
        mut waters,
        mut modes,
        variants,
        mut yaws,
        mut intents,
        mut ticks,
        mut healths,
        mut anims,
    ) = (
        world.write::<crate::PhysBody>(),
        world.read::<crate::MobKind>(),
        world.write::<WaterState>(),
        world.write::<AquaticMode>(),
        world.read::<AquaticVariant>(),
        world.write::<crate::Yaw>(),
        world.write::<crate::MobIntent>(),
        world.write::<crate::MobTicks>(),
        world.write::<crate::Health>(),
        world.write::<crate::AnimState>(),
    );

    phys.for_each(|e, body| {
        let Some(mode) = modes.get_mut(e) else {
            return; // 非水生怪（Brain / passive 路径），跳过
        };
        // 实体门 1（模拟环）：环外冻结——只拍 pos_old（渲染插值不回退），
        // AI/物理/消散全跳（消散由 mob_ai_system 环外 checkDespawn 面接管）。
        let own = ChunkPos::new(
            (body.pos.x / 16.0).floor() as i32,
            (body.pos.z / 16.0).floor() as i32,
        );
        if !svc.sim_cells.contains(&own) {
            body.pos_old = body.pos;
            return;
        }
        let Some(&crate::MobKind(id)) = kinds.get(e) else {
            return;
        };
        let def = *id.def();
        let pos = body.pos;

        // 水位判定：眼位（体高×0.85 代理，与 game.rs 同式）或体心任一为水。
        // 26.1 isInWater 的流体 AABB 近似——浅水贴底也不误判离水。
        let eye_y = (pos.y + def.half_size[1] * 1.7).floor() as i32;
        let mid_y = (pos.y + def.half_size[1]).floor() as i32;
        let cell = BlockPos::new(pos.x.floor() as i32, eye_y, pos.z.floor() as i32);
        let mid_cell = BlockPos::new(pos.x.floor() as i32, mid_y, pos.z.floor() as i32);
        let in_water = view.block(cell) == WATER_BLOCK || view.block(mid_cell) == WATER_BLOCK;

        // ---- 20 Hz AI（on_tick 门）----
        if on_tick {
            // 受击逃离（事件输入）：PanicGoal 优先级最高（:92）。
            let hurt = ticks.get_mut(e).is_some_and(|tk| {
                let h = tk.hurt_flag;
                tk.hurt_flag = false;
                h
            });
            if hurt {
                let ang = rand01(&mut rng_state) * std::f32::consts::TAU;
                let d = 2.0 + rand01(&mut rng_state) * 3.0;
                mode.target = pos + Vec3::new(ang.cos() * d, 0.0, ang.sin() * d);
                mode.behavior = AquaticBehavior::Flee;
                mode.flee_ticks = FLEE_MAX_TICKS;
            }

            match mode.behavior {
                AquaticBehavior::Flee => {
                    mode.flee_ticks = mode.flee_ticks.saturating_sub(1);
                    // PanicGoal 跑到导航完成 / 兜底到点。
                    let reached = horizontal_dist(pos, mode.target) < 0.5;
                    if reached || mode.flee_ticks == 0 {
                        mode.behavior = AquaticBehavior::Idle;
                    }
                }
                AquaticBehavior::Wander => {
                    mode.wander_ticks = mode.wander_ticks.saturating_sub(1);
                    let reached = horizontal_dist(pos, mode.target) < 0.5;
                    if reached || mode.wander_ticks == 0 {
                        mode.behavior = AquaticBehavior::Idle;
                    }
                }
                AquaticBehavior::Idle => {
                    // AvoidEntityGoal（:93，p=2）：玩家 8 格内 → 1.6 倍速
                    // 远离（PanicGoal 不在跑时才生效——Flee 已在上面持有）。
                    let dp = player_pos - pos;
                    if dp.length_squared() < AVOID_RADIUS_SQR
                        && mode.behavior == AquaticBehavior::Idle
                    {
                        let dir = dp.normalize_or_zero();
                        mode.target = pos - dir * 4.0;
                        mode.behavior = AquaticBehavior::Flee;
                        mode.flee_ticks = FLEE_MAX_TICKS;
                    }
                    // FishSwimGoal（p=4）：每 tick 1/40 起游，目标点为水内
                    // 可游位（getRandomSwimmablePos(10,7) 的 10 次重掷近似；
                    // 全部落空保持静栖——原版 navigation.moveTo(false) 等价）。
                    if mode.behavior == AquaticBehavior::Idle
                        && xorshift(&mut rng_state).is_multiple_of(u64::from(SWIM_INTERVAL))
                    {
                        for _ in 0..10 {
                            let ang = rand01(&mut rng_state) * std::f32::consts::TAU;
                            let d = 1.0 + rand01(&mut rng_state) * (SWIM_RADIUS - 1.0);
                            let dy = (rand01(&mut rng_state) * 2.0 - 1.0) * SWIM_RADIUS_V;
                            let cand = pos + Vec3::new(ang.cos() * d, dy, ang.sin() * d);
                            let cp = BlockPos::new(
                                cand.x.floor() as i32,
                                cand.y.floor() as i32,
                                cand.z.floor() as i32,
                            );
                            if view.block(cp) == WATER_BLOCK {
                                mode.target = cand;
                                mode.behavior = AquaticBehavior::Wander;
                                mode.wander_ticks = FLEE_MAX_TICKS;
                                break;
                            }
                        }
                    }
                }
            }

            // 移动意图 → MobIntent（60 Hz 物理复用）+ 朝向。wish 幅值语义
            // （physics.rs StepInput）：游泳律 target = wish×SWIM_SPEED，
            // 地面律 target = wish×WALK_SPEED，均只保亚单位幅值。
            let mut wish = match mode.behavior {
                AquaticBehavior::Wander => wish_to(pos, mode.target, SWIM_WISH_CRUISE),
                AquaticBehavior::Flee => wish_to(pos, mode.target, SWIM_WISH_FLEE),
                AquaticBehavior::Idle => Vec3::ZERO,
            };
            let mut jump = false;
            if in_water {
                // 水内竖直导航：目标高于体位 → 走引擎 jumpInLiquid 通道
                //（+0.04/tick，physics.rs 水分支）；缓沉由引擎 SWIM_SINK
                // 兜底（≈ AbstractFish travelInWater −0.005/tick 与
                // FishMoveControl +0.005 浮力的合成量级）。
                if wish.length_squared() > 1e-6 && mode.target.y > pos.y + 0.5 {
                    jump = true;
                }
            } else if def.kind == crate::defs::MobKind::Axolotl {
                // 美西螈陆行（AmphibiousNavigation）：地面律；速度属性 1.0
                // 经引擎 WALK 收敛律饱和——wish 幅值取 0.4（≈1.73 m/s，
                // KNOWN-DIVERGENCE：引擎移动律换算差异）。想走走不动 →
                // 跳跃兜底（与 game.rs/passive.rs 同款）。
                if wish.length_squared() > 1e-6 {
                    wish = wish.normalize() * AXOLOTL_LAND_WISH;
                    let flat = (body.vel.x * body.vel.x + body.vel.z * body.vel.z).sqrt();
                    jump = body.on_ground && flat < AXOLOTL_LAND_SPEED * 0.25;
                }
            } else if body.on_ground {
                // 离水扑腾（AbstractFish.aiStep:117-127）：每 tick 重掷
                // （verticalCollision≈on_ground 近似——引擎无逐碰撞旗）。
                let (flop_wish, flop_jump) = flop_impulse(&mut rng_state);
                wish = flop_wish;
                jump = flop_jump;
            }

            if let Some(intent) = intents.get_mut(e) {
                *intent = crate::MobIntent { wish, jump };
            }
            // 朝向：FishMoveControl.rotlerp(90°/tick)（AbstractFish.java
            // :175-179）——上限 π/2 每 tick 有界逼近航向。
            if wish.length_squared() > 1e-4
                && let Some(nyaw) = crate::ai::walk_body_yaw(
                    yaws.get(e).map(|v| v.0).unwrap_or(0.0),
                    wish,
                    std::f32::consts::FRAC_PI_2,
                )
                && let Some(y) = yaws.get_mut(e)
            {
                y.0 = nyaw;
            }

            // 窒息账本（WaterAnimal.handleAirSupply / Axolotl 6000 覆写；
            // Axolotl 的 isInWaterOrRain 含雨——引擎步内无雨判，恒按
            // !in_water 计，KNOWN-DIVERGENCE）。
            let Some(water) = waters.get_mut(e) else {
                return;
            };
            if let Some(dmg) = dry_out_step(&mut water.air, def.air_supply as i32, in_water)
                && let Some(h) = healths.get_mut(e)
            {
                h.0 -= dmg;
            }
            water.in_water = in_water;
            water.age_ticks = water.age_ticks.wrapping_add(1);

            // 消散（Mob.java:662-687）：WATER_AMBIENT despawn 64²（
            // MobCategory.java:13）AXOLOTLS 128²（:10）；<32² 清 idle 账。
            let despawn_dist = if id == MobId::AXOLOTL { 128 } else { 64 };
            let d2 = (player_pos - pos).length_squared();
            let idle = ticks.get_mut(e).map(|tk| {
                if d2 < 32.0 * 32.0 {
                    tk.idle_ticks = 0;
                } else {
                    tk.idle_ticks += 1;
                }
                tk.idle_ticks
            });
            if let Some(idle) = idle
                && crate::spawner::should_despawn(d2, idle, despawn_dist, 32, &mut || {
                    let v = xorshift(&mut rng_state);
                    (v >> 32) as u32
                })
            {
                // 直接消灭（despawn ≠ 死亡：无掉落无经验，Mob.remove）。
                commands.despawn(e);
                return;
            }
        }

        // ---- 60 Hz 物理：水中喂游泳律，离水喂地面律 ----
        let input = mcv_game::StepInput {
            wish_dir: intents.get(e).map(|i| i.wish).unwrap_or(Vec3::ZERO),
            jump: intents.get(e).is_some_and(|i| i.jump) && (in_water || body.on_ground),
            in_water,
            sneak: false,
            sprint: false,
            look_dir: Vec3::ZERO,
            gravity_scale: 1.0,
        };
        let mut eng = body.body();
        body.pos_old = body.pos;
        step_entity(&view, &mut eng, def.half_size, &input);
        body.set_body(&eng);

        // 行走/游动摆动账本（尾摆相位由 WaterState.age_ticks 供渲染侧；
        // 这里推 amount 收敛，腿摆与尾摆共用）。
        if let Some(anim) = anims.get_mut(e) {
            const DT: f32 = 1.0 / 60.0;
            let horiz = (eng.vel.x * eng.vel.x + eng.vel.z * eng.vel.z).sqrt();
            crate::passive::advance_walk_anim(anim, horiz, DT);
        }

        // 死亡：Health≤0 → despawn + 环境掉落（玩家击杀掉落在 game.rs
        // try_attack 路径结算，此处只管窒息等环境死）。
        if healths.get(e).is_some_and(|h| h.0 <= 0.0) {
            let pos = body.pos;
            let variant = variants.get(e).map(|v| v.0).unwrap_or(0);
            let mut drop_rng = xorshift(&mut rng_state) as u32; // 种子传承
            let drops: Vec<(u16, u8)> =
                crate::drops::death_drops_variant(def.kind, variant, false, pos, &mut || {
                    drop_rng = drop_rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    drop_rng
                })
                .into_iter()
                .filter_map(|ev| {
                    mcv_item::item_by_name(ev.item).map(|i| (i, ev.count.min(u8::MAX as u32) as u8))
                })
                .collect();
            commands.despawn(e);
            if !drops.is_empty() {
                let at = pos + Vec3::Y * 0.5;
                commands.push(move |w| {
                    let mut r = 0x1234_5678_9ABC_DEF0u64;
                    let mut rng32 = move || {
                        r = r.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        (r >> 32) as u32
                    };
                    for (item, count) in drops {
                        crate::spawn_item_drop(w, at, item, count, crate::PICKUP_DELAY, &mut rng32);
                    }
                });
            }
        }
    });

    // 推进后的随机流写回（下一固定步继续）。
    if let Some(svc) = ctx.resources.get_mut::<AquaticServices>() {
        svc.seed = rng_state;
    }
}

/// 水平归一 wish（游走/逃离指向 [`AquaticMode::target`]）。
fn wish_to(pos: Vec3, target: Vec3, magnitude: f32) -> Vec3 {
    let mut dir = target - pos;
    dir.y = 0.0;
    let len = dir.length();
    if len < 1e-4 {
        Vec3::ZERO
    } else {
        dir / len * magnitude
    }
}

fn horizontal_dist(a: Vec3, b: Vec3) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::speed_m_s;

    /// 递增 u32 流（确定性 RNG 替身；finalize_variant 只吃流不关心分布）。
    fn seq() -> impl FnMut() -> u32 {
        let mut i = 0u32;
        move || {
            let v = i;
            i = i.wrapping_add(1);
            v
        }
    }

    #[test]
    fn xorshift_is_deterministic() {
        let (mut a, mut b) = (42u64, 42u64);
        for _ in 0..100 {
            assert_eq!(xorshift(&mut a), xorshift(&mut b));
        }
        assert_ne!(a, 42, "状态必须推进");
    }

    #[test]
    fn dry_out_cadence_matches_vanilla() {
        // 鱼：300 tick 空气 + 20 tick 宽限 → 第 320 tick 首次 2.0 伤害，
        // 此后每 20 tick 一跳（air 清零后重新 −1..−20）。
        let mut air = 300i32;
        let mut hits = 0;
        let mut first_hit = 0;
        for t in 1..=650 {
            if dry_out_step(&mut air, 300, false).is_some() {
                hits += 1;
                if first_hit == 0 {
                    first_hit = t;
                }
            }
        }
        assert_eq!(first_hit, 320);
        assert_eq!(hits, (650 - 320) / 20 + 1, "每 20 tick 一跳");
        // 入水即回满。
        let mut air2 = -20;
        assert_eq!(dry_out_step(&mut air2, 300, true), None);
        assert_eq!(air2, 300);
        // 美西螈：6000 + 20 → 6020 tick 首跳。t 记「结算当拍的序数」——
        // 循环先推进 t 再结算，与 air=300 面首跳序数 320 同口径（系统级
        // dry_out_cadence_death_and_drop 的 first_hit=320 同一计数法）。
        let mut air3 = 6000i32;
        let mut t = 0;
        loop {
            t += 1;
            if dry_out_step(&mut air3, 6000, false).is_some() {
                break;
            }
        }
        assert_eq!(t, 6020);
    }

    #[test]
    fn flop_impulse_is_bounded() {
        let mut rng = 7u64;
        for _ in 0..50 {
            let (w, jump) = flop_impulse(&mut rng);
            assert!(jump);
            assert!(w.x.abs() <= FLOP_WISH + 1e-5);
            assert!(w.z.abs() <= FLOP_WISH + 1e-5);
            assert_eq!(w.y, 0.0);
        }
    }

    #[test]
    fn tropical_pack_roundtrip() {
        // packVariant ↔ getBaseColor/getPatternColor/getPattern（26.1:59-72）。
        let packed = tropical::pack(3, true, tropical::DYE_CYAN, tropical::DYE_PINK);
        assert_eq!(tropical::unpack_pattern(packed), (true, 3));
        assert_eq!(tropical::base_color(packed), tropical::DYE_CYAN);
        assert_eq!(tropical::pattern_color(packed), tropical::DYE_PINK);
        // DEFAULT_VARIANT = KOB + WHITE + WHITE。
        assert_eq!(tropical::DEFAULT, 0);
        assert_eq!(tropical::unpack_pattern(tropical::DEFAULT), (false, 0));
        // 22 组常见变体与源序一致（抽查 + 长度）。
        assert_eq!(tropical::COMMON.len(), 22);
        assert_eq!(tropical::COMMON[0].0, "stripey");
        assert_eq!(tropical::COMMON[5].0, "kob");
        assert_eq!(tropical::COMMON[21].0, "flopper");
        assert!(tropical::COMMON_PACKED.iter().all(|&p| p != 0));
        // 贴图槽位：素底 index 0 → layer 0，其余 1..6（渲染侧映射面）。
        assert_eq!(tropical::texture_slot(tropical::DEFAULT), (false, 0));
        assert_eq!(
            tropical::texture_slot(tropical::COMMON_PACKED[1]),
            (true, 0)
        );
    }

    #[test]
    fn salmon_and_axolotl_variant_tables() {
        // 权重 30/50/15（Salmon.java:129-136）：递增流 0..30 → SMALL（<30），
        // 30..80 → MEDIUM（<80），80..95 → LARGE。
        let mut rng = seq();
        assert_eq!(salmon::finalize_variant(&mut rng), salmon::SMALL);
        let mut rng30 = {
            let mut i = 30u32;
            move || {
                let v = i;
                i += 1;
                v
            }
        };
        assert_eq!(salmon::finalize_variant(&mut rng30), salmon::MEDIUM);
        let mut rng80 = {
            let mut i = 80u32;
            move || {
                let v = i;
                i += 1;
                v
            }
        };
        assert_eq!(salmon::finalize_variant(&mut rng80), salmon::LARGE);
        // scale 0.5/1.0/1.5（Salmon.java:190-197）。
        assert_eq!(salmon::scale(salmon::SMALL), 0.5);
        assert_eq!(salmon::scale(salmon::MEDIUM), 1.0);
        assert_eq!(salmon::scale(salmon::LARGE), 1.5);
        // 美西螈 spawn 只出 common 四型（finalizeSpawn:156-178）。
        let mut rng2 = seq();
        for _ in 0..40 {
            let v = axolotl::spawn_variant(&mut rng2);
            assert!(v <= axolotl::CYAN, "BLUE 不进自然刷怪池");
        }
        // 稀有门（1/1200）面存在（繁殖路径）。
        let mut rng3 = seq();
        let _ = axolotl::use_rare(&mut rng3);
    }

    #[test]
    fn def_table_anchored() {
        // 数值锚：health/air/尺寸全按 26.1 EntityType/Axolotl 源码。
        for id in [MobId::COD, MobId::SALMON, MobId::TROPICAL_FISH] {
            let def = id.def();
            assert_eq!(def.health, 3.0, "AbstractFish MAX_HEALTH");
            assert_eq!(def.air_supply, 300);
            assert!(!def.hostile);
        }
        let ax = MobId::AXOLOTL.def();
        assert_eq!(ax.health, 14.0);
        assert_eq!(ax.attack_damage, 2.0);
        assert_eq!(ax.air_supply, 6000);
        assert_eq!(ax.half_size, [0.375, 0.21, 0.375]);
        assert!(speed_m_s(MobId::COD.def().speed_attr) > 0.0);
    }
}
