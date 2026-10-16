//! vanilla 后端 pass 2.2 —— 陆地植被（草原系三族：short_grass 草丛
//! [patch_grass_* 5 行] + tall_grass [2 行] + 花族 [flower_plains /
//! flower_default / flower_warm 3 行]执行器）。
//!
//! 把 26.1 陆生植被特征接进 [`crate::vanilla::terrain::generate`]：树 pass
//! 后、水草（pass 2.3）前——26.1 各陆生群系 vegetal 槽内行的相对序
//! （plains.json features[10] 行序 patch_tall_grass_2 < flower_plains <
//! patch_grass_plain 等）由本模块 ROWS 全局序逐行保序；RIVER 槽
//! （river.json features[9] 行序 flower_default/patch_grass_badlands <
//! seagrass_river）要求陆地植被在 [`crate::vanilla::water_flora`] 之前
//! ——terrain generate 调度序（lake→ore→tree→**陆地植被**→水草→freeze）
//! 与原版同槽串行序对齐。执行器与重放模型照
//! [`crate::vanilla::water_flora`] 先例（ocean-flora2 同款）：3×3 origin
//! 区块重放 + 裁剪写自块；判据读 pass-1 纯函数重放态
//! （[`crate::vanilla::ore_pass::Replay`]，不读缓冲、不读先前特征写）。
//!
//! ## 26.1 实物锚点（相对 `data/minecraft/worldgen/`，Java 相对
//! `net/minecraft/world/.../`）
//!
//! - **数据行（placed → configured）**：
//!   - plains.json features[10] → `patch_tall_grass_2`（noise_threshold_
//!     count above 7/below 0, level −0.8 → rarity 1/32 → in_square →
//!     heightmap MOTION_BLOCKING → biome → count 96 → random_offset
//!     ±7/±3 → air）→ configured tall_grass.json（simple_block，
//!     tall_grass[hanf=lower]）；
//!   - plains.json features[10] → `flower_plains`（noise_threshold_count
//!     above 4/below 15 → rarity 1/32 → in_square → MOTION_BLOCKING →
//!     biome → count 64 → random_offset ±6/±2 → air）→ configured
//!     flower_plain.json（**noise_threshold_provider**：threshold −0.8、
//!     high_chance 0.33333334、noise seed 2345 / firstOctave 0 /
//!     amplitudes [1.0] / scale 0.005；default dandelion，high=poppy/
//!     azure_bluet/oxeye_daisy/cornflower，low=orange/red/pink/white
//!     tulip）；
//!   - plains.json features[10] → `patch_grass_plain`（noise_threshold_
//!     count above 10/below 5 → in_square → heightmap **WORLD_SURFACE_
//!     WG** → biome → count 32 → random_offset ±7/±3 → air）→
//!     configured grass.json（simple_block，short_grass）；
//!   - forest.json / birch_forest.json features[9] → `flower_default`
//!     （rarity 1/32 → in_square → MOTION_BLOCKING → biome，**无
//!     count/offset/air**）→ configured flower_default.json（weighted：
//!     poppy 2 / dandelion 1）；`patch_grass_forest`（count 2 → …链尾同
//!     grass）→ grass.json。birch 另有 wildflowers_birch_forest 行——
//!     wildflower 状态面（facing/flower_amount 16 变体）单 id 无表达，
//!     未登记（本波未登记行段）；
//!   - taiga.json features[9] → `flower_default` → flower_default.json；
//!     `patch_grass_taiga_2`（无链头 count）→ configured taiga_grass.json
//!     （weighted：short_grass 1 / fern 4）；
//!   - snowy_plains.json / desert.json / beach.json / river.json 的
//!     `patch_grass_badlands`（**无链头 count**）→ grass.json 与
//!     `flower_default` → flower_default.json（4+beach=5 群系共用）；
//!   - savanna.json features[9] → `patch_tall_grass`（rarity 1/5 → …同
//!     tall 链尾）→ tall_grass.json；`flower_warm`（rarity 1/16）→
//!     placed flower_warm.json **指向 configured flower_default.json**
//!     （同 flower_default configured）；`patch_grass_savanna`
//!     （count 20）→ grass.json。
//!   - ocean.json 无草/花行（陆族对 Ocean 不登记）。
//!
//! - **SimpleBlockFeature.place（SimpleBlockFeature.java:16-47）**：
//!   stateToPlace 先取（:21，**canSurvive 失败也照消耗 provider 抽**）→
//!   canSurvive（:26，VegetationBlock.java:44-46 mayPlaceOn =
//!   BlockTags.SUPPORTS_VEGETATION——本仓静态表
//!   `mcv_core::tags::TAG_BLOCK_SUPPORTS_VEGETATION`，gen-tags 波产物，
//!   26.1 supports_vegetation.json = #dirt/#mud/#moss/#grass_blocks +
//!   farmland）→ DoublePlantBlock 分支（:27-32）：上格非空（:28
//!   isEmptyBlock）整投放弃，否则 `placeAt` 下半+上半
//!   （DoublePlantBlock.java:86-90，本仓 HALF 状态面不存 → 双写同 id，
//!   tall_seagrass 先例）→ 其余单格 setBlock（:36）。
//!
//! - **放置链（placed JSON 修饰序，PlacedFeature.java:47 lazy flatMap
//!   逐修饰串行；origin = 区块角 (ocx·16, 0, ocz·16)，
//!   ChunkGenerator.java:314-318 SectionPos.origin）**：
//!   - 链头 RepeatingPlacement（RepeatingPlacement.java:11-16：count 次
//!     同 pos 复制，零随机）：minecraft:count（CountPlacement.java:27-29
//!     → ConstantInt 零随机）或 noise_threshold_count
//!     （NoiseThresholdCountPlacement.java:34-37：BIOME_INFO_NOISE.
//!     getValue(x/200, z/200, false)，n < noise_level → below 否则
//!     above；origin 一次评估）；无链头 = 单复制流。
//!   - rarity_filter（RarityFilter.java:20-23：`nextFloat < 1/chance`，
//!     每复制 1 抽——在 in_square 之前）。
//!   - in_square（InSquarePlacement.java:17-18，2×nextInt(16)，每复制）。
//!   - heightmap（HeightmapPlacement.java:25-34，Y 置换零随机）——
//!     WORLD_SURFACE_WG = NOT_AIR 谓词（Heightmap.java:147，水计顶）、
//!     MOTION_BLOCKING = blocksMotion‖非空流体（:151，水计顶）：pass-1
//!     重放下两者同值 = [`ore_pass::Replay::top_floor`]（max(fs, SEA)+1）。
//!   - biome（BiomeFilter.java:21-26 零随机，判当前 Placement 位群系；
//!     本仓 2D 点 [`ore_pass::Replay::biome_at`] 先例）。
//!   - 链中 count 32/96/64（RepeatingPlacement 复制，零随机）。
//!   - random_offset（RandomOffsetPlacement.java:46-51：抽序 x→y→z；
//!     TrapezoidInt.sample 的 plateau 0 + 对称界走 `nextInt(b+1) −
//!     nextInt(b+1)` 快路 TrapezoidInt.java:40-43——±7=nextInt(8) 对、
//!     ±6=nextInt(7) 对、±3=nextInt(4) 对、±2=nextInt(3) 对；每向 2 抽）。
//!   - block_predicate_filter air（BlockPredicateFilter.java:24-26，
//!     #minecraft:air，零随机；pass-1 重放位读）。
//!   - feature（SimpleBlockFeature 如上）+ configured 状态面抽：
//!     short_grass 0 抽；taiga_grass weighted 1 抽（WeightedList.java:
//!     66-73 `nextInt(total)`，累计界按 JSON 序 = [0,1) short / [1,5)
//!     fern）；flower_default weighted 1 抽（= [0,2) poppy /
//!     [2,3) dandelion）；flower_plain noise_threshold_provider
//!     （NoiseThresholdProvider.java:59-66：value < threshold → low 四
//!     tulip `Util.getRandom` 一抽 nextInt(4)；否则 `nextFloat <
//!     highChance` 一抽 + 命中再 nextInt(4) 选 high 四花；default
//!     dandelion）。
//!
//! - **花域噪声（flower_plain configured，seed 2345 常量、与世界种子
//!   无关）**：NormalNoise.create(WorldgenRandom(LegacyRandomSource(2345))
//!   ）（NoiseBasedStateProvider.java:31，**legacy 链**非 xoroshiro
//!   fork 链）：PerlinNoise.create(random, 0, [1.0])（PerlinNoise.java:57
//!   useNewInitialization=true；NormalNoise.java:43-44 first/second 两次）
//!   → 每次 forkPositional = LegacyRandomSource.java:27-29 取 nextLong
//!   作工厂种子（WorldgenRandom.java:25-27 透传；BitRandomSource.java:
//!   37-41 双 next(32) 合成 nextLong）→ octave_0 =
//!   fromHashOf("octave_0") = 名字 hashCode() ^ factorySeed
//!   （LegacyRandomSource.java:71-73）→ ImprovedNoise（nextDouble×3 +
//!   Fisher–Yates nextInt(256−i)×256，ImprovedNoise.java:14-30）→
//!   值 = (first + second·1.0181268882175227) × 0.16666…/0.2
//!   （NormalNoise.java:63/:75-79 + expectedDeviation span 0 =
//!   0.1·(1+1/1) :97-103）→ provider 乘 scale 0.005 含 y
//!   （NoiseBasedStateProvider.java:34-36，3D）。
//!
//! - **BIOME_INFO_NOISE 复用**：NoiseThresholdCountPlacement（:35）与
//!   water_flora kelp 同一 Biome.java:66 常量单例（种子 2345、
//!   PerlinSimplexNoise）——直接复用
//!   [`crate::vanilla::water_flora::biome_info_noise`]。
//!
//! - **随机域**：每 (origin 区块, 行) 一条独立流（WATER_FLORA_DOMAIN 同
//!   型域盐 + 行序混入；非原版 setDecorationSeed 特征种子链——本仓契约
//!   「同 seed 同字节」，非逐位对拍，tree/ore/water_flora 同款登记）。
//!   抽取序 = 链序 + 修饰内抽序逐字照抄（feature 状态抽在 canSurvive
//!   判前——SimpleBlockFeature.java:21 先于 :26，门败照消耗）。
//!
//! ## 本仓建模登记（KNOWN-DIVERGENCE 同族）
//! - **判据读 pass-1 重放态**：树写（叶/干）不入重放判据、本 pass 写后
//!   的格子也不入同块后续行判据——原版 features 全局串行世界写互见不同
//!   频（湖/矿/树/水草 pass 同族登记）。效应：树冠下仍可落草（vanilla
//!   的 air 门会拦）、跨行同格叠写以「后写者赢」落 buffer。
//! - **WORLD_SURFACE_WG / MOTION_BLOCKING pass-1 口径**：两者在重放态
//!   同值 = max(fs, SEA)+1（水计顶；树叶差异不入重放，见上条）。水面行
//!   y=63 恒水（aquifer.rs open_column / geometry_locks「海下柱 63 层非
//!   水」契约）→ 水面/水下位 air 门与 canSurvive 双杀，陆地族写永不落
//!   水体（水草族 y ≤ SEA−1 写帽所防的「海面行占格」路径本族不可达，
//!   不设另帽）。
//! - **状态面**：tall_grass 上/下半单 id 双写；flower 单 id 单写
//!   （facing/flower_amount 状态面无表达——wildflowers 行未登记承担）。
//! - **本波未登记行**（vegetal 槽内同家族、非本轮 grass/tall_grass/花
//!   三族范围）：wildflowers_birch_forest（状态面缺）、patch_bush
//!   （forest/birch/river）、forest_flowers、patch_dry_grass_desert
//!   （short/tall_dry_grass）、brown/red_mushroom（含 taiga 变体）、
//!   patch_pumpkin、patch_sugar_cane（含 desert 变体）、
//!   patch_firefly_bush_near_water、patch_large_fern、patch_berry_common
//!   （taiga）、patch_cactus_decored/p_*（desert）。10 群系点表无
//!   Meadow/SunflowerPlains/Swamp 等变体 → 其行路径不可达（biomes.rs
//!   v1，water_flora 头注同款）。

use crate::vanilla::aquifer::AquiferField;
use crate::vanilla::biomes::Biome;
use crate::vanilla::channel_seed;
use crate::vanilla::ore_pass::Replay;
use crate::vanilla::perlin::ImprovedNoise;
use crate::vanilla::simplex::LegacyRandom;
use crate::vanilla::splitmix64;
use crate::vanilla::terrain::Orchestrator;
use crate::vanilla::water_flora::{FloraRng, biome_info_noise};
use std::sync::OnceLock;

/// 陆地植被通道域盐（WATER_FLORA_DOMAIN = 0x005E_A5EE_0000_0001 的姊妹
/// 域；每 (origin 区块, 行) 流再域内派生，见模块头注随机域段）。
pub const LAND_FLORA_DOMAIN: u64 = 0x005E_A5EE_0000_0002;

/// 下行 id 引 `mcv_core::BLOCKS`（blocks_gen.inc.rs，注释 = 表行号）：
/// 草丛（26.1 改名 short_grass）。
const SHORT_GRASS: u16 = 907; // blocks_gen.inc.rs:907 ("short_grass")
/// 高草丛（DoublePlantBlock；HALF 状态面本仓不存，双写同 id）。
const TALL_GRASS: u16 = 1001; // blocks_gen.inc.rs:1001 ("tall_grass")
/// 蕨（taiga_grass weighted 权重 4 支）。
const FERN: u16 = 404; // blocks_gen.inc.rs:404 ("fern")
/// 蒲公英（ legacy 行 "flower_yellow" id 13，贴图 230 = 26.1 dandelion）。
const DANDELION: u16 = 13; // blocks_gen.inc.rs:13 ("flower_yellow")
/// 虞美人（legacy 行 "flower_red" id 12，贴图 574 = 26.1 poppy）。
const POPPY: u16 = 12; // blocks_gen.inc.rs:12 ("flower_red")
/// 花域 high_states（flower_plain.json high_states 数组序）。
const AZURE_BLUET: u16 = 46; // blocks_gen.inc.rs:79 ("azure_bluet")
const CORNFLOWER: u16 = 245; // blocks_gen.inc.rs:245 ("cornflower")
const OXEYE_DAISY: u16 = 675; // blocks_gen.inc.rs:708 ("oxeye_daisy")
/// low_states（flower_plain.json low_states 数组序）。
const ORANGE_TULIP: u16 = 672; // blocks_gen.inc.rs:705 ("orange_tulip")
const RED_TULIP: u16 = 872; // blocks_gen.inc.rs:905 ("red_tulip")
const PINK_TULIP: u16 = 732; // blocks_gen.inc.rs:765 ("pink_tulip")
const WHITE_TULIP: u16 = 1148; // blocks_gen.inc.rs:1181 ("white_tulip")

/// 重放空格哨兵（Replay::block_at 越界/空段返回 0；本模块判读字面制，
/// 同 ore_pass::AIR 语义）。
const AIR: u16 = 0;

/// "octave_0" 的 Java String.hashCode（31 进制滚动 i32 包绕；python
/// 独立复算 = 1261148513，tests 白名单锁死）。
const JAVA_HASH_OCTAVE_0: i32 = 1_261_148_513;

/// Java `String.hashCode()`（UTF-16 单元序、31 进制滚动）——ASCII 字面
/// 量与字节序一致；编译期求值供常量哨兵对拍（tests）。
pub const fn java_string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        h = h.wrapping_mul(31).wrapping_add(bytes[i] as i32);
        i += 1;
    }
    h
}

/// ImprovedNoise 的巨坐标封包（PerlinNoise.java:188-190 wrap；noise.rs
/// ·one_perlin 同式）。
#[must_use]
fn wrap(x: f64) -> f64 {
    const D: f64 = 3.355_443_2e7;
    x - (x / D + 0.5).floor() * D
}

/// 花域噪声：flower_plain.json noise_threshold_provider 的 NormalNoise
///（seed 2345、firstOctave 0、amplitudes [1.0]，**legacy 种子链**，见模
/// 块头注花域噪声段）。双层独立实例（NormalNoise.java:43-44 first/second
/// 各自 PerlinNoise.create，各自 forkPositional）。
struct FlowerNoise {
    first: ImprovedNoise,
    second: ImprovedNoise,
}

/// 单层 PerlinNoise.create(rng, 0, [1.0])（PerlinNoise.java:57 ·
/// useNewInitialization=true 支路 :87-95）：forkPositional → 工厂种子 =
/// nextLong（BitRandomSource.java:37-41 双 next(32) 合成）→
/// fromHashOf("octave_0") = 名字哈希 ^ 工厂种子（LegacyRandomSource.java:
/// 71-73）。
fn octave_leaf(rng: &mut LegacyRandom) -> ImprovedNoise {
    let factory = rng.next_long();
    let mut src = LegacyRandom::new(i64::from(JAVA_HASH_OCTAVE_0) ^ factory);
    ImprovedNoise::from_legacy(&mut src)
}

impl FlowerNoise {
    fn new() -> Self {
        let mut rng = LegacyRandom::new(2345);
        let first = octave_leaf(&mut rng);
        let second = octave_leaf(&mut rng);
        Self { first, second }
    }

    /// NormalNoise.getValue（NormalNoise.java:75-79）：单倍频
    /// lowestFreqInputFactor = 2⁰、lowestFreqValueFactor = 2⁰/(2¹−1)
    /// （PerlinNoise.java:129-131，amplitudes.len()=1）；坐标巨值 wrap。
    fn one_perlin(&self, inst: &ImprovedNoise, x: f64, y: f64, z: f64) -> f64 {
        inst.noise(wrap(x), wrap(y), wrap(z))
    }

    /// NormalNoise.getValue：第二层坐标 1.0181268882175227（NormalNoise.
    /// java:76-78）；valueFactor = 0.16666666666666666 /
    /// expectedDeviation（ :59-70 扫非零振幅 → span 0 → :97-103 = 0.1·
    /// (1+1/1) = 0.2）。
    fn get_value(&self, x: f64, y: f64, z: f64) -> f64 {
        const D: f64 = 1.018_126_888_217_522_7;
        const VALUE_FACTOR: f64 = 0.166_666_666_666_666_66 / 0.2;
        (self.one_perlin(&self.first, x, y, z) + self.one_perlin(&self.second, x * D, y * D, z * D))
            * VALUE_FACTOR
    }
}

/// 花域噪声单例（常量种子 2345，世界种子无关——NoiseBasedStateProvider
/// .java:31 构造即用字面 seed）。
fn flower_noise() -> &'static FlowerNoise {
    static NOISE: OnceLock<FlowerNoise> = OnceLock::new();
    NOISE.get_or_init(FlowerNoise::new)
}

/// 花域 NormalNoise 值域暴露（测试/工具面：`ci/flora-legacy-noise-mirror.
/// py` python 独立复算对拍 + K-FLORA 位型打印；生产判据零消费——
/// noise_threshold_provider 的分支门在本值面外不导出）。
#[doc(hidden)]
#[must_use]
pub fn flower_noise_probe(x: f64, y: f64, z: f64) -> f64 {
    flower_noise().get_value(x, y, z)
}

/// feat 子类（configured_feature 面见模块头注数据行段）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Feat {
    /// grass.json：simple_block short_grass（5 条 patch_grass_* 行共用）。
    ShortGrass,
    /// taiga_grass.json：weighted short_grass 1 / fern 4。
    TaigaGrass,
    /// tall_grass.json：simple_block DoublePlant（下半+上半）。
    TallGrass,
    /// flower_default.json：weighted poppy 2 / dandelion 1（flower_default
    /// 与指向同 configured 的 flower_warm 两 placed 行共用）。
    FlowerWeighted,
    /// flower_plain.json：noise_threshold_provider（花域噪声门的高/低/
    /// 默认三支花选）。
    FlowerPlainNoise,
}

/// 链头 RepeatingPlacement（模块头注放置链段）。
#[derive(Clone, Copy, Debug)]
enum Pre {
    /// 无链头 count 修饰（单复制流：patch_grass_badlands / flower 行）。
    None,
    /// minecraft:count（ConstantInt，CountPlacement.java:27-29 零随机）。
    Const(i32),
    /// noise_threshold_count（NoiseThresholdCountPlacement.java:34-37，
    /// BIOME_INFO_NOISE x/200、n < level → below 否则 above，origin 一评
    /// 、零随机）。
    NoiseThresh { level: f64, below: i32, above: i32 },
}

/// 一行陆地植被 placed 行（群系 + 26.1 features 槽内行序 + 链参数）。
struct Row {
    biome: Biome,
    feat: Feat,
    pre: Pre,
    /// 链中 RarityFilter（chance；None = 无此修饰）——在 in_square 之前。
    rarity: Option<u32>,
    /// in_square + heightmap + biome 门后的 RepeatingPlacement count。
    count: i32,
    /// random_offset (xz_spread, y_spread)（TrapezoidInt 梯形 ±b；None =
    /// 无 offset 修饰）。
    offset: Option<(i32, i32)>,
    /// block_predicate_filter #minecraft:air（链尾）。
    air: bool,
}

/// 26.1 rows（10 群系点表可及位 · grass/tall_grass/花族三族；全局序 = 各
/// 群系槽内行相对序保序，见模块头注。行引注 = biome json features 槽内
/// vegetal 行 + placed_feature/<名>.json 修链逐位对照）。
const ROWS: [Row; 20] = [
    // plains.json features[10]：槽内行序 patch_tall_grass_2(0) <
    // flower_plains(3) < patch_grass_plain(4)（bush(1)/trees(2) 位不属本
    // 波未登记 + 调度面树 pass 统一在先——头注 KNOWN-DIVERGENCE 第 1 条）。
    Row {
        biome: Biome::Plains,
        // plains features[10]:0 patch_tall_grass_2.
        feat: Feat::TallGrass,
        pre: Pre::NoiseThresh {
            level: -0.8,
            below: 0,
            above: 7,
        },
        rarity: Some(32),
        count: 96,
        offset: Some((7, 3)),
        air: true,
    },
    Row {
        biome: Biome::Plains,
        // plains features[10]:3 flower_plains.
        feat: Feat::FlowerPlainNoise,
        pre: Pre::NoiseThresh {
            level: -0.8,
            below: 15,
            above: 4,
        },
        rarity: Some(32),
        count: 64,
        offset: Some((6, 2)),
        air: true,
    },
    Row {
        biome: Biome::Plains,
        // plains features[10]:4 patch_grass_plain.
        feat: Feat::ShortGrass,
        pre: Pre::NoiseThresh {
            level: -0.8,
            below: 5,
            above: 10,
        },
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
    // forest.json features[9]：flower_default(4) < patch_grass_forest(5)。
    Row {
        biome: Biome::Forest,
        // forest features[9]:4 flower_default.
        feat: Feat::FlowerWeighted,
        pre: Pre::None,
        rarity: Some(32),
        count: 1,
        offset: None,
        air: false,
    },
    Row {
        biome: Biome::Forest,
        // forest features[9]:5 patch_grass_forest.
        feat: Feat::ShortGrass,
        pre: Pre::Const(2),
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
    Row {
        biome: Biome::BirchForest,
        // birch_forest features[9]:5 flower_default.
        feat: Feat::FlowerWeighted,
        pre: Pre::None,
        rarity: Some(32),
        count: 1,
        offset: None,
        air: false,
    },
    Row {
        biome: Biome::BirchForest,
        // birch_forest features[9]:6 patch_grass_forest.
        feat: Feat::ShortGrass,
        pre: Pre::Const(2),
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
    // taiga.json features[9]：flower_default(3) < patch_grass_taiga_2(4)。
    Row {
        biome: Biome::Taiga,
        // taiga features[9]:3 flower_default.
        feat: Feat::FlowerWeighted,
        pre: Pre::None,
        rarity: Some(32),
        count: 1,
        offset: None,
        air: false,
    },
    Row {
        biome: Biome::Taiga,
        // taiga features[9]:4 patch_grass_taiga_2.
        feat: Feat::TaigaGrass,
        pre: Pre::None,
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
    // snowy_plains.json features[9]：flower_default(2) <
    // patch_grass_badlands(3)。
    Row {
        biome: Biome::SnowyPlains,
        // snowy_plains features[9]:2 flower_default.
        feat: Feat::FlowerWeighted,
        pre: Pre::None,
        rarity: Some(32),
        count: 1,
        offset: None,
        air: false,
    },
    Row {
        biome: Biome::SnowyPlains,
        // snowy_plains features[9]:3 patch_grass_badlands.
        feat: Feat::ShortGrass,
        pre: Pre::None,
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
    // savanna.json features[9]：patch_tall_grass(1) < flower_warm(3) <
    // patch_grass_savanna(4)。
    Row {
        biome: Biome::Savanna,
        // savanna features[9]:1 patch_tall_grass.
        feat: Feat::TallGrass,
        pre: Pre::None,
        rarity: Some(5),
        count: 96,
        offset: Some((7, 3)),
        air: true,
    },
    Row {
        biome: Biome::Savanna,
        // savanna features[9]:3 flower_warm.
        feat: Feat::FlowerWeighted,
        pre: Pre::None,
        rarity: Some(16),
        count: 1,
        offset: None,
        air: false,
    },
    Row {
        biome: Biome::Savanna,
        // savanna features[9]:4 patch_grass_savanna.
        feat: Feat::ShortGrass,
        pre: Pre::Const(20),
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
    // desert.json features[9]：flower_default(1) < patch_grass_badlands(2)。
    Row {
        biome: Biome::Desert,
        // desert features[9]:1 flower_default.
        feat: Feat::FlowerWeighted,
        pre: Pre::None,
        rarity: Some(32),
        count: 1,
        offset: None,
        air: false,
    },
    Row {
        biome: Biome::Desert,
        // desert features[9]:2 patch_grass_badlands.
        feat: Feat::ShortGrass,
        pre: Pre::None,
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
    // beach.json features[9]：flower_default(1) < patch_grass_badlands(2)。
    Row {
        biome: Biome::Beach,
        // beach features[9]:1 flower_default.
        feat: Feat::FlowerWeighted,
        pre: Pre::None,
        rarity: Some(32),
        count: 1,
        offset: None,
        air: false,
    },
    Row {
        biome: Biome::Beach,
        // beach features[9]:2 patch_grass_badlands.
        feat: Feat::ShortGrass,
        pre: Pre::None,
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
    // river.json features[9]：flower_default(3) < patch_grass_badlands(4)
    //（seagrass_river 行 10 = water_flora pass 2.3，调度在本 pass 后）。
    Row {
        biome: Biome::River,
        // river features[9]:3 flower_default.
        feat: Feat::FlowerWeighted,
        pre: Pre::None,
        rarity: Some(32),
        count: 1,
        offset: None,
        air: false,
    },
    Row {
        biome: Biome::River,
        // river features[9]:4 patch_grass_badlands.
        feat: Feat::ShortGrass,
        pre: Pre::None,
        rarity: None,
        count: 32,
        offset: Some((7, 3)),
        air: true,
    },
];

/// 一条写记录（世界坐标 + 植被 id；y ∈ 世界体素域由投影端钳）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloraWrite {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: u16,
}

/// 草原植被附入门（VegetationBlock.java:44-46 mayPlaceOn = SUPPORTS_
/// VEGETATION tag；gen-tags 波静态表，26.1 substrate_overworld+farmland
/// 展开集）。
fn supports(below: u16) -> bool {
    mcv_core::tags::TAG_BLOCK_SUPPORTS_VEGETATION
        .set
        .contains(below)
}

/// 梯形 spread 一抽（TrapezoidInt.sample 的 plateau 0 + 对称界快路，
/// TrapezoidInt.java:40-43：`nextInt(b+1) − nextInt(b+1)`，2 抽）。
fn trapezoid(rng: &mut FloraRng, b: i32) -> i32 {
    rng.next_int((b + 1) as u32) as i32 - rng.next_int((b + 1) as u32) as i32
}

/// WeightedStateProvider 一抽（WeightedList.java:66-73 nextInt(total)，
/// 累计界按 JSON 序展开——entry i 占 [sum(前 i), sum(含 i)））。
fn weighted_pick(rng: &mut FloraRng, weights: &[u32]) -> usize {
    let total: u32 = weights.iter().sum();
    let mut roll = rng.next_int(total);
    for (i, &w) in weights.iter().enumerate() {
        if roll < w {
            return i;
        }
        roll -= w;
    }
    weights.len() - 1
}

/// SimpleBlockFeature 终段（:21/:26/:27-32/:36 单次 attempt；状态抽在
/// canSurvive 判前——:21 先于 :26，门败照消耗，见头注随机域段）。写 =
/// 单格 setBlock（:36）或 tall 双半 placeAt（DoublePlantBlock.java:86-90）。
fn place_feature(
    rp: &mut Replay<'_, '_>,
    rng: &mut FloraRng,
    feat: Feat,
    x: i32,
    y: i32,
    z: i32,
    writes: &mut Vec<FloraWrite>,
) {
    // :21 状态面（configured to_place；各 feat 的抽消耗面见头注放置链
    // feature 段——noise_threshold_provider 分支抽在本 match 内完成）。
    let id = match feat {
        Feat::ShortGrass => SHORT_GRASS,
        Feat::TaigaGrass => {
            // taiga_grass.json weighted [short_grass 1, fern 4]。
            if weighted_pick(rng, &[1, 4]) == 0 {
                SHORT_GRASS
            } else {
                FERN
            }
        }
        Feat::TallGrass => TALL_GRASS,
        Feat::FlowerWeighted => {
            // flower_default.json weighted [poppy 2, dandelion 1]。
            if weighted_pick(rng, &[2, 1]) == 0 {
                POPPY
            } else {
                DANDELION
            }
        }
        Feat::FlowerPlainNoise => {
            // NoiseThresholdProvider.java:59-66（噪声 3D、scale 0.005、
            // threshold f32 −0.8、high_chance 0.33333334f）。
            let v = flower_noise().get_value(
                f64::from(x) * 0.005,
                f64::from(y) * 0.005,
                f64::from(z) * 0.005,
            );
            const LOW: [u16; 4] = [ORANGE_TULIP, RED_TULIP, PINK_TULIP, WHITE_TULIP];
            const HIGH: [u16; 4] = [POPPY, AZURE_BLUET, OXEYE_DAISY, CORNFLOWER];
            if v < f64::from(-0.8_f32) {
                LOW[rng.next_int(4) as usize]
            } else if rng.next_float() < 0.333_333_34_f32 {
                HIGH[rng.next_int(4) as usize]
            } else {
                DANDELION
            }
        }
    };
    // :26 canSurvive（VegetationBlock.java:44-46：下方格里 SUPPORTS_
    // VEGETATION）。
    if !supports(rp.block_at(x, y - 1, z)) {
        return;
    }
    if feat == Feat::TallGrass {
        // :28 isEmptyBlock(origin.above()) 门（读重放态；水/实心均非空）。
        if rp.block_at(x, y + 1, z) != AIR {
            return;
        }
        // :32 placeAt（DoublePlantBlock.java:86-90 下半+上半；HALF 状态
        // 面本仓不存——双写同 id，tall_seagrass 先例）。
        writes.push(FloraWrite { x, y, z, id });
        writes.push(FloraWrite { x, y: y + 1, z, id });
    } else {
        // :36 单格 setBlock。
        writes.push(FloraWrite { x, y, z, id });
    }
}

/// 单个 origin 区块的全部陆地植被写（全序；(seed, origin) 纯函数——
/// 两侧块重放恒同输出）。链序/抽取序逐字对位（模块头注放置链段）。
pub fn origin_flora_writes(
    seed: u64,
    ocx: i32,
    ocz: i32,
    rp: &mut Replay<'_, '_>,
) -> Vec<FloraWrite> {
    let ts = splitmix64(channel_seed(seed, LAND_FLORA_DOMAIN) as u64);
    let cs = splitmix64(
        ts ^ (i64::from(ocx) as u64).wrapping_mul(0x9E37_79B1_9E37_79B1)
            ^ (i64::from(ocz) as u64).wrapping_mul(0x85EB_CA77_85EB_CA77),
    );
    let mut writes = Vec::new();
    for (idx, row) in ROWS.iter().enumerate() {
        let mut rng = FloraRng::new(splitmix64(
            cs ^ (idx as u64 + 1).wrapping_mul(0x36B1_C4A2_77B1_23A3),
        ));
        // 链头 RepeatingPlacement（RepeatingPlacement.java:11-16）：count
        // 次复制同 pos（origin = 区块角，y=0 无用位）；noise 型 origin 一
        // 评（NoiseThresholdCountPlacement.java:34-37，零随机）。
        let copies = match row.pre {
            Pre::None => 1,
            Pre::Const(n) => n,
            Pre::NoiseThresh {
                level,
                below,
                above,
            } => {
                let n = biome_info_noise().get_value(
                    f64::from(ocx * 16) / 200.0,
                    f64::from(ocz * 16) / 200.0,
                    false,
                );
                if n < level { below } else { above }
            }
        };
        for _ in 0..copies {
            // rarity（RarityFilter.java:20-23，在 in_square 之前；nextFloat
            // 有界无 NaN——`>=` 与 `!(<)` 同面，clippy neg_cmp_op 直写式）。
            if let Some(chance) = row.rarity
                && rng.next_float() >= 1.0 / chance as f32
            {
                continue;
            }
            // in_square（InSquarePlacement.java:17-18，2 抽）。
            let x = ocx * 16 + rng.next_int(16) as i32;
            let z = ocz * 16 + rng.next_int(16) as i32;
            // heightmap（HeightmapPlacement.java:25-34，零随机；pass-1
            // 口径 = max(fs, SEA)+1，两型同值见头注 KNOWN-DIVERGENCE 2）。
            let y = rp.top_floor(x, z);
            // biome（BiomeFilter.java:21-26，零随机，判 Placement 位）。
            if rp.biome_at(x, z) != row.biome {
                continue;
            }
            // 链中 RepeatingPlacement（count 32/96/64；flower 行 =1）。
            for _ in 0..row.count {
                // random_offset（RandomOffsetPlacement.java:46-51 抽序
                // x→y→z；TrapezoidInt.java:40-43 每向 2 抽）。
                let (fx, fy, fz) = match row.offset {
                    Some((bx, by)) => (
                        x + trapezoid(&mut rng, bx),
                        y + trapezoid(&mut rng, by),
                        z + trapezoid(&mut rng, bx),
                    ),
                    None => (x, y, z),
                };
                // block_predicate_filter #air（BlockPredicateFilter.java:
                // 24-26；pass-1 重放位读；无此修饰的行为直通）。
                if row.air && rp.block_at(fx, fy, fz) != AIR {
                    continue;
                }
                place_feature(rp, &mut rng, row.feat, fx, fy, fz, &mut writes);
            }
        }
    }
    writes
}

/// 本 pass 写集 id 面（投影护栏白名单）。
fn owns(id: u16) -> bool {
    matches!(
        id,
        SHORT_GRASS
            | TALL_GRASS
            | FERN
            | DANDELION
            | POPPY
            | AZURE_BLUET
            | CORNFLOWER
            | OXEYE_DAISY
            | ORANGE_TULIP
            | RED_TULIP
            | PINK_TULIP
            | WHITE_TULIP
    )
}

/// pass 2.2：陆地植被投影。3×3 origin 区块重放 + 裁剪写自块（ore_pass
/// ·project_ores / water_flora·project_water_flora 同型；random_offset
/// ±7 抖动可越界本块 7 格——统一 3×3 域重放两侧块恒同几何）。替换写 =
/// 原版 setBlock(flags 2) 直写语义；y 域出界格丢（offset 上探 +3 兜底，
/// 高草上格 y+1 同域）。返回写数。
///
/// 投影缓冲护栏（tree_pass ChunkClipper 直写门同型先例）：放置链的
/// live-world 前置门（BlockPredicateFilter #air / canSurvive 下方支撑面）
/// 在原版读**含特征写**的世界态（Heightmap.java:147 NOT_AIR 谓词、
/// VegetationBlock.java:44-46），本仓判据面 = pass-1 重放态（树/湖/先后
/// 特征写不入，模块头注 KNOWN-DIVERGENCE 登记）——树 pass（2.1）先于本
/// pass，其干/冠已落缓冲：SURF-2 判据面收口后植被 y 精对实顶+1 位（=
/// 树基位），直写吞原木 = 树基格 907 非原木（trees_cross_chunk.rs:262
/// 树基格 (-1760,66,-1884)=907，CI 38907640949）。护栏 = 缓冲格仅在
/// AIR 或本 pass 植被 id（KNOWN-DIVERGENCE「跨行同格叠写后写者赢」放行）
/// 才落写；原木/叶/石/水格丢弃 = live-world 门败位的投影侧等价。
pub fn project_land_flora(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
    voxels: &mut [u16],
) -> usize {
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    let mut rp = Replay::new(seed, orch, field);
    let mut applied = 0usize;
    for dz in -1..=1 {
        for dx in -1..=1 {
            for w in origin_flora_writes(seed, chunk_x + dx, chunk_z + dz, &mut rp) {
                let lx = w.x - base_x;
                let lz = w.z - base_z;
                if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
                    continue; // 属邻块：由邻块重放同一 (seed, origin) 补齐
                }
                if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&w.y) {
                    continue;
                }
                let cell = &mut voxels[mcv_core::vidx(lx as usize, w.y, lz as usize)];
                if *cell != AIR && !owns(*cell) {
                    continue; // 缓冲护栏：原版 live-world 门败位（干/冠/石/水）
                }
                *cell = w.id;
                applied += 1;
            }
        }
    }
    applied
}

/// 测试/工具入口：自备 Replay 的便捷封装（同 (seed, origin) 恒同输出；
/// water_flora::flora_writes 先例）。
#[must_use]
pub fn flora_writes(
    seed: u64,
    ocx: i32,
    ocz: i32,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
) -> Vec<FloraWrite> {
    let mut rp = Replay::new(seed, orch, field);
    origin_flora_writes(seed, ocx, ocz, &mut rp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "octave_0" Java String.hashCode 哨兵（python 独立复算 = 1261148513；
    /// 防常量手抄漂移，flora 花域种子链的第一输入）。
    #[test]
    fn octave_hash_locks_java_value() {
        assert_eq!(java_string_hash("octave_0"), 1_261_148_513);
        assert_eq!(JAVA_HASH_OCTAVE_0, 1_261_148_513);
    }
}
