//! 矿石参数表数据对拍（#178 刀3a 漂移守卫）。
//!
//! 抽查行的 count/size/discard/height/targets 与 26.1 源 JSON 实物数值
//! 逐一手工比对后固化为字面量（比对记录：configured_feature/ore_*.json +
//! placed_feature/ore_*.json，`/root/mc-ref/src-26.1/data/minecraft/`，
//! 原文不入仓——只锁数值结论）。gen-ores.py 重跑若偏离实物即红。

use mcv_worldgen::vanilla::ores_data::{HeightDist, ORE_ROWS, OreCount, OreRow};

fn row(name: &str) -> &'static OreRow {
    *ORE_ROWS
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("缺行 {name}（placed feature 实物存在）"))
}

#[test]
fn row_set_is_overworld_underground_ores_of_repo_biomes() {
    // 25 行 = 本仓 10 群系 json features[UNDERGROUND_ORES] 中
    // type=minecraft:ore 的全部实物；disk/magma 非 ore 型跳过。
    assert_eq!(ORE_ROWS.len(), 25);
    // 10 群系 features[6] 实测同表（gen-ores.py 断言）→ 掩码全开。
    for r in ORE_ROWS.iter() {
        assert_eq!(r.biomes, 0x3FF, "{}: biome 位图漂移", r.name);
    }
    // 范围红线：下界石英（nether 群系）与 emerald（windswept 系山地群系）
    // 不在本仓群系子集——数据行不得混入。
    for r in ORE_ROWS.iter() {
        assert!(!r.name.contains("quartz") && !r.name.contains("emerald"));
    }
    // 行序 = 实物 features[6] 序（首三行锚）。
    assert_eq!(ORE_ROWS[0].name, "ore_dirt");
    assert_eq!(ORE_ROWS[1].name, "ore_gravel");
    assert_eq!(ORE_ROWS[9].name, "ore_coal_upper");
}

#[test]
fn coal_upper_matches_json() {
    // placed ore_coal_upper：count 30 / uniform(absolute 136, below_top 0
    // → 319)；configured ore_coal：size 17 / discard 0.0。
    let r = row("ore_coal_upper");
    assert!(matches!(r.count, OreCount::Fixed(30)));
    assert_eq!(r.rarity, 0);
    assert_eq!(r.size, 17);
    assert_eq!(r.discard_on_air, 0.0);
    assert!(matches!(r.height, HeightDist::Uniform { lo: 136, hi: 319 }));
    assert_eq!(r.targets.len(), 2);
    assert_eq!(
        mcv_core::BLOCKS[r.targets[0].state as usize].name,
        "coal_ore"
    );
    assert_eq!(r.targets[0].tag.key.path, "stone_ore_replaceables");
    assert_eq!(
        mcv_core::BLOCKS[r.targets[1].state as usize].name,
        "deepslate_coal_ore"
    );
    assert_eq!(r.targets[1].tag.key.path, "deepslate_ore_replaceables");
}

#[test]
fn coal_lower_buried_matches_json() {
    // ore_coal_lower → ore_coal_buried：count 20 / trapezoid 0..192 /
    // size 17 / discard 0.5。
    let r = row("ore_coal_lower");
    assert!(matches!(r.count, OreCount::Fixed(20)));
    assert_eq!(r.size, 17);
    assert_eq!(r.discard_on_air, 0.5);
    assert!(matches!(
        r.height,
        HeightDist::Trapezoid {
            lo: 0,
            hi: 192,
            plateau: 0
        }
    ));
}

#[test]
fn iron_middle_matches_json() {
    // 任务书深度分布实物例：count 10 trapezoid(absolute −24..56)、size 9。
    let r = row("ore_iron_middle");
    assert!(matches!(r.count, OreCount::Fixed(10)));
    assert_eq!(r.size, 9);
    assert_eq!(r.discard_on_air, 0.0);
    assert!(matches!(
        r.height,
        HeightDist::Trapezoid {
            lo: -24,
            hi: 56,
            plateau: 0
        }
    ));
}

#[test]
fn iron_small_matches_json() {
    // ore_iron_small：count 10 / uniform(above_bottom 0 → −64, absolute
    // 72) / size 4。
    let r = row("ore_iron_small");
    assert!(matches!(r.count, OreCount::Fixed(10)));
    assert_eq!(r.size, 4);
    assert!(matches!(r.height, HeightDist::Uniform { lo: -64, hi: 72 }));
}

#[test]
fn copper_matches_json() {
    // 实物例：count 16 trapezoid(absolute −16..112)；configured
    // ore_copper_small size 10（placed ore_copper 引 small 变体）。
    let r = row("ore_copper");
    assert!(matches!(r.count, OreCount::Fixed(16)));
    assert_eq!(r.size, 10);
    assert!(matches!(
        r.height,
        HeightDist::Trapezoid {
            lo: -16,
            hi: 112,
            plateau: 0
        }
    ));
    assert_eq!(
        mcv_core::BLOCKS[r.targets[0].state as usize].name,
        "copper_ore"
    );
}

#[test]
fn diamond_family_matches_json() {
    // 实物例：diamond count 7 trapezoid(above_bottom −80..80) → 世界 y
    // −144..16（越界值原样保留，min_y=−64 由执行器 isOutsideBuildHeight
    // 兜界）；placed ore_diamond 引 configured ore_diamond_small size 4 /
    // discard 0.5。
    let r = row("ore_diamond");
    assert!(matches!(r.count, OreCount::Fixed(7)));
    assert_eq!(r.size, 4);
    assert_eq!(r.discard_on_air, 0.5);
    assert!(matches!(
        r.height,
        HeightDist::Trapezoid {
            lo: -144,
            hi: 16,
            plateau: 0
        }
    ));
    // large：rarity 1/9、size 12、discard 0.7（同一 trapezoid）。
    let l = row("ore_diamond_large");
    assert!(matches!(l.count, OreCount::Fixed(1)));
    assert_eq!(l.rarity, 9);
    assert_eq!(l.size, 12);
    assert_eq!(l.discard_on_air, 0.7);
    // medium：count 2 / uniform(absolute −64..−4) / size 8 / discard 0.5。
    let m = row("ore_diamond_medium");
    assert!(matches!(m.count, OreCount::Fixed(2)));
    assert_eq!(m.size, 8);
    assert_eq!(m.discard_on_air, 0.5);
    assert!(matches!(m.height, HeightDist::Uniform { lo: -64, hi: -4 }));
    // buried：count 4 / trapezoid(above_bottom −80..80) / size 8 /
    // discard 1.0（1.0 = 恒不贴空气面，OreFeature.java:187）。
    let b = row("ore_diamond_buried");
    assert!(matches!(b.count, OreCount::Fixed(4)));
    assert_eq!(b.size, 8);
    assert_eq!(b.discard_on_air, 1.0);
}

#[test]
fn gold_lower_matches_json() {
    // 实物唯一的 uniform count：{0,1}；uniform(absolute −64..−48)；
    // configured ore_gold_buried size 9 / discard 0.5。
    let r = row("ore_gold_lower");
    assert!(matches!(r.count, OreCount::Uniform { lo: 0, hi: 1 }));
    assert_eq!(r.size, 9);
    assert_eq!(r.discard_on_air, 0.5);
    assert!(matches!(r.height, HeightDist::Uniform { lo: -64, hi: -48 }));
}

#[test]
fn redstone_lapis_buried_variants_match_json() {
    // ore_redstone_lower：count 8 / trapezoid(above_bottom −32..32) →
    // −96..−32 / size 8。
    let r = row("ore_redstone_lower");
    assert!(matches!(r.count, OreCount::Fixed(8)));
    assert_eq!(r.size, 8);
    assert!(matches!(
        r.height,
        HeightDist::Trapezoid {
            lo: -96,
            hi: -32,
            plateau: 0
        }
    ));
    // ore_lapis：count 2 / trapezoid(absolute −32..32) / size 7。
    let l = row("ore_lapis");
    assert!(matches!(l.count, OreCount::Fixed(2)));
    assert_eq!(l.size, 7);
    assert!(matches!(
        l.height,
        HeightDist::Trapezoid {
            lo: -32,
            hi: 32,
            plateau: 0
        }
    ));
    // ore_lapis_buried：discard 1.0 / uniform(above_bottom 0 → absolute
    // 64) = −64..64 / count 4。
    let b = row("ore_lapis_buried");
    assert_eq!(b.discard_on_air, 1.0);
    assert!(matches!(b.height, HeightDist::Uniform { lo: -64, hi: 64 }));
    assert!(matches!(b.count, OreCount::Fixed(4)));
}

#[test]
fn scatter_replacements_target_base_stone() {
    // 26.1 实物：dirt/gravel/granite/diorite/andesite/tuff 行是 ore 型
    // 且单 target = #base_stone_overworld（不是 stone/deepslate 双 tag——
    // 与本仓 #178 框架口径的 base_stone 实物出处）；size 33/64、
    // granite/diorite/andesite upper 带 rarity 1/6。
    for (name, state, count, rarity, size) in [
        ("ore_dirt", "dirt", OreCount::Fixed(7), 0u32, 33u8),
        ("ore_gravel", "gravel", OreCount::Fixed(14), 0, 33),
        ("ore_granite_upper", "granite", OreCount::Fixed(1), 6, 64),
        ("ore_diorite_lower", "diorite", OreCount::Fixed(2), 0, 64),
        ("ore_andesite_upper", "andesite", OreCount::Fixed(1), 6, 64),
        ("ore_tuff", "tuff", OreCount::Fixed(2), 0, 64),
    ] {
        let r = row(name);
        assert_eq!(mcv_core::BLOCKS[r.targets[0].state as usize].name, state);
        assert_eq!(r.targets[0].tag.key.path, "base_stone_overworld");
        assert_eq!(r.targets.len(), 1);
        assert_eq!(size_of_count(&r.count), size_of_count(&count), "{name}");
        assert_eq!(r.rarity, rarity, "{name}");
        assert_eq!(r.size, size, "{name}");
        assert_eq!(r.discard_on_air, 0.0);
    }
}

fn size_of_count(c: &OreCount) -> u32 {
    match c {
        OreCount::Fixed(n) => *n,
        OreCount::Uniform { lo, .. } => *lo,
    }
}
