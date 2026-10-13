//! vanilla 数据层 —— 矿石生成参数表（#178 刀3a）。
//!
//! 数据行由 `ci/gen-ores.py` 从 26.1 worldgen JSON 生成
//! （`ores_gen.inc.rs`，寻径/防漂移政策同 gen-tags.py，原版 JSON 原文
//! 不入仓）。本文件只声明行的形状；形状逐项对位原版实物：
//!
//! - [`OreRow::targets`]：`OreConfiguration.java:45-57` TargetBlockState
//!   列表（json 序 = `OreFeature.java:145-151` 首个 canPlaceOre 胜出的试放
//!   序）。目标岩走 `mcv_core::tags::TAG_BLOCK_*`（#178 框架口径：tag
//!   单一真源，不抄成员名单）。
//! - [`OreRow::count`]/[`OreRow::rarity`]：placed_feature 的
//!   CountPlacement（IntProvider，`CountPlacement.java:23-24`）与
//!   RarityFilter（`RarityFilter.java:22-24`，`nextFloat() < 1/chance`）。
//!   `rarity = 0` = 无该 modifier（实物两链型见 gen-ores.py 断言）。
//! - [`HeightDist`]：`UniformHeight.java:37-48`（一次均匀）与
//!   `TrapezoidHeight.java:37-55`（两次均匀叠加；plateau≥range 退化
//!   uniform、`plateauStart=(range-plateau)/2`）。锚已按
//!   `VerticalAnchor.java:55/72/89` 换算世界 y；越界值原样保留
//!   （如 diamond above_bottom −80 → −144，`OreFeature.java:132`
//!   isOutsideBuildHeight 兜界，分布形状是本体）。
//! - [`OreRow::biomes`]：biome modifier（`BiomeFilter.java:24-28`）真源 =
//!   群系 json features[UNDERGROUND_ORES=6]（`GenerationStep.java:8-18`
//!   枚举序）；位序 = `biomes.rs` Biome 枚举判别值。原版只收 10 群系表内
//!   出现的特征（quartz/下界、emerald 的山地群系特征自动出局——
//!   emerald 与 Jungle 树同型：数据行留真源，本仓群系子集不可达，报告）。

use mcv_registry::TagEntry;

/// `OreConfiguration.TargetBlockState`：目标岩 tag → 命中后放的矿 id
/// （GEN_BLOCKS 行序）。
#[derive(Debug)]
pub struct OreTarget {
    /// 矿石方块 id（stone 变体位放 `*_ore`、深板岩位放 `deepslate_*_ore`）。
    pub state: u16,
    /// 目标岩集合（构建期已解引用的 tags_gen 表项；`set.contains(id)`）。
    pub tag: &'static TagEntry,
}

/// placed_feature 的 `count` IntProvider（`CountProviders.java`）：
/// 常数零随机；uniform = `min + nextInt(hi-lo+1)`（`UniformInt.java`）。
#[derive(Debug, Clone, Copy)]
pub enum OreCount {
    /// ConstantInt（实物多数行）。
    Fixed(u32),
    /// UniformInt（实物：ore_gold_lower 0..1）。
    Uniform { lo: u32, hi: u32 },
}

/// HeightProviderType 实物两型 + constant 壳（`HeightProvider.java:14-19`
/// 常数分支；本波数据行无 constant，留位防数据侧漂移）。
#[derive(Debug, Clone, Copy)]
pub enum HeightDist {
    /// `UniformHeight.java:37`：`min + nextInt(hi-lo+1)`（lo>hi 返 lo）。
    Uniform { lo: i32, hi: i32 },
    /// `TrapezoidHeight.java:37-55`：`lo + nextInt(pe+1) + nextInt(ps+1)`。
    Trapezoid { lo: i32, hi: i32, plateau: i32 },
    /// 常数（零随机）。
    Const(i32),
}

/// 一行 = 一个 placed ore feature（configured 的 targets/size/discard
/// 展平进来；数据链与行序依据见模块头注与 ci/gen-ores.py）。
#[derive(Debug)]
pub struct OreRow {
    /// placed feature 名（诊断/测试锚）。
    pub name: &'static str,
    /// 目标对（json 序 = 试放序，首个 canPlaceOre 胜出即 break，
    /// `OreFeature.java:145-151`）。
    pub targets: &'static [OreTarget],
    /// count placement modifier。
    pub count: OreCount,
    /// rarity_filter chance（0 = 无）。
    pub rarity: u32,
    /// OreConfiguration.size（blob 节点数，`OreFeature.java:74-88`）。
    pub size: u8,
    /// discard_chance_on_air_exposure（`OreFeature.java:183-189`）。
    pub discard_on_air: f32,
    /// height_range placement modifier。
    pub height: HeightDist,
    /// biome 位图（位 = Biome 枚举判别值）。
    pub biomes: u32,
}

include!("ores_gen.inc.rs");

/// biome modifier 门（`BiomeFilter.java:24-28` 的本仓数据对位：群系 json
/// features[6] 表 → 位图）。
#[must_use]
pub fn biome_allowed(row: &OreRow, b: crate::vanilla::Biome) -> bool {
    row.biomes & (1u32 << (b as u32)) != 0
}
