//! 千块表全名 SoundType 覆盖：mcv_core::BLOCKS 的每个注册名都必须能直接
//! （Blocks.java 注册名）或经旧名别名（gen-blocks.py LEGACY 8 名）命中
//! mcv_audio::BLOCK_SOUND_TYPES——不允许落到「未知名 → STONE 默认」的兜底，
//! 否则就是映射缺口而非原版归属。
use mcv_audio::{SOUND_TYPES, try_sound_type_for_block};

#[test]
fn every_engine_block_name_resolves() {
    let missing: Vec<&str> = mcv_core::BLOCKS
        .iter()
        .map(|b| b.name)
        .filter(|n| try_sound_type_for_block(n).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "千块表 SoundType 缺口（{} 个）: {missing:?}",
        missing.len()
    );
}

#[test]
fn vanilla_names_dominate_and_sound_spread_is_real() {
    // 表规模 = 千块表方块数（旧名 8 个是别名条目，官方名同表在场）。
    assert_eq!(mcv_core::BLOCKS.len(), 1171, "千块表规模锚点漂移");
    // 实际用到的 SoundType 种类必须覆盖主要材质族（>40 种），防退化成全 STONE。
    let mut used = std::collections::HashSet::new();
    for b in mcv_core::BLOCKS.iter() {
        if let Some(st) = try_sound_type_for_block(b.name) {
            // 指针等价 → 类型在 SOUND_TYPES 的下标。
            let idx = SOUND_TYPES
                .iter()
                .position(|(_, v)| std::ptr::eq(v, st))
                .expect("返回值必须出自 SOUND_TYPES");
            used.insert(idx);
        }
    }
    assert!(used.len() > 40, "材质族过少({}): 映射疑似退化", used.len());
}

#[test]
fn terrain_anchor_blocks() {
    // 地形生成器直接产出的 id 0..13 锚点（旧名）逐一对值。
    use mcv_audio::sound_type_for_block;
    let st = |n: &str| sound_type_for_block(n);
    let by_name = |n: &str| mcv_audio::sound_type_by_name(n).unwrap();
    assert!(std::ptr::eq(st("stone"), by_name("STONE")));
    assert!(std::ptr::eq(st("dirt"), by_name("GRAVEL")));
    assert!(std::ptr::eq(st("grass"), by_name("GRASS")));
    assert!(std::ptr::eq(st("sand"), by_name("SAND")));
    assert!(std::ptr::eq(st("water"), by_name("EMPTY")));
    assert!(std::ptr::eq(st("log"), by_name("WOOD")));
    assert!(std::ptr::eq(st("leaves"), by_name("GRASS")));
    assert!(std::ptr::eq(st("planks"), by_name("WOOD")));
    assert!(std::ptr::eq(st("cobble"), by_name("STONE")));
    assert!(std::ptr::eq(st("bedrock"), by_name("STONE")));
    assert!(std::ptr::eq(st("snow_grass"), by_name("GRASS")));
}

#[test]
fn wood_family_ids_are_wood() {
    // W 波木族（feat/wood-content）依赖的全树种 log/planks 全部归 WOOD——
    // 音效与材质同源断言（6 树种抽样 + 挂果两族）。
    use mcv_audio::sound_type_by_name;
    let wood = sound_type_by_name("WOOD").unwrap();
    for name in [
        "oak_log",
        "spruce_log",
        "birch_log",
        "jungle_log",
        "acacia_log",
        "dark_oak_log",
        "mangrove_log",
        "cherry_log",
        "oak_planks",
        "spruce_planks",
        "birch_planks",
        "acacia_planks",
        "crafting_table",
        "beehive",
        "honeycomb_block",
        "oak_stairs",
        "spruce_slab",
        "acacia_fence",
    ] {
        assert!(
            std::ptr::eq(mcv_audio::try_sound_type_for_block(name).unwrap(), wood),
            "{name} 应为 WOOD"
        );
    }
}
