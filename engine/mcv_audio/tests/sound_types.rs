//! SoundType 全表对拍 26.1 Java 常数（逐值断言，先红后绿的判据）。
//!
//! 表源：`SoundType.java`（volume/pitch + dig/step/place/hit/fall 七参构造，
//! :828-844）与 `SoundEvents.java`（常数 → sounds.json 事件名）；方块归属取
//! `Blocks.java` `.sound(SoundType.X)` 注册（缺省 STONE，BlockBehaviour.java:973）。
//! 引注一律 SoundType.java 行号（S.java）/ SoundEvents.java 行号（SE.java）。
use mcv_audio::{
    SOUND_TYPES, SoundType, sound_type_by_name, sound_type_for_block, try_sound_type_for_block,
};

/// 按类型名取 SoundType 的便捷断言入口。
fn t(name: &str) -> &'static SoundType {
    sound_type_by_name(name).unwrap_or_else(|| panic!("SoundType {name} 缺失"))
}

/// 一族五事件全对拍（S.java 行号写进断言消息供审计）。
#[allow(clippy::too_many_arguments)] // 断言元组天然多（七参构造 + 源引注）
fn assert_family(
    st: &SoundType,
    vol: f32,
    pitch: f32,
    dig: &str,
    step: &str,
    place: &str,
    hit: &str,
    fall: &str,
    src: &str,
) {
    assert_eq!(st.volume, vol, "{src} volume");
    assert_eq!(st.pitch, pitch, "{src} pitch");
    assert_eq!(st.dig, dig, "{src} dig");
    assert_eq!(st.step, step, "{src} step");
    assert_eq!(st.place, place, "{src} place");
    assert_eq!(st.hit, hit, "{src} hit");
    assert_eq!(st.fall, fall, "{src} fall");
}

#[test]
fn wood_family_matches_java() {
    // S.java:10-12 + SE.java（block.wood.* 五事件）。
    assert_family(
        t("WOOD"),
        1.0,
        1.0,
        "block.wood.break",
        "block.wood.step",
        "block.wood.place",
        "block.wood.hit",
        "block.wood.fall",
        "SoundType.java:10-12",
    );
}

#[test]
fn stone_and_gravel_families() {
    // S.java:28-30 / :13-15。
    assert_family(
        t("STONE"),
        1.0,
        1.0,
        "block.stone.break",
        "block.stone.step",
        "block.stone.place",
        "block.stone.hit",
        "block.stone.fall",
        "SoundType.java:28-30",
    );
    assert_family(
        t("GRAVEL"),
        1.0,
        1.0,
        "block.gravel.break",
        "block.gravel.step",
        "block.gravel.place",
        "block.gravel.hit",
        "block.gravel.fall",
        "SoundType.java:13-15",
    );
}

#[test]
fn metal_pitch_1_5() {
    // S.java:31-33：METAL pitch=1.5（全表少数非 1 音高）。
    assert_family(
        t("METAL"),
        1.0,
        1.5,
        "block.metal.break",
        "block.metal.step",
        "block.metal.place",
        "block.metal.hit",
        "block.metal.fall",
        "SoundType.java:31-33",
    );
}

#[test]
fn anvil_volume_0_3() {
    // S.java:58-60：ANVIL volume=0.3（全表唯一 <1 音量）。
    assert_family(
        t("ANVIL"),
        0.3,
        1.0,
        "block.anvil.break",
        "block.anvil.step",
        "block.anvil.place",
        "block.anvil.hit",
        "block.anvil.fall",
        "SoundType.java:58-60",
    );
}

#[test]
fn twisting_vines_pitch_0_5_reuses_weeping_events() {
    // S.java:160-168：TWISTING_VINES pitch=0.5，五事件全部引用 WEEPING_VINES 组。
    assert_family(
        t("TWISTING_VINES"),
        1.0,
        0.5,
        "block.weeping_vines.break",
        "block.weeping_vines.step",
        "block.weeping_vines.place",
        "block.weeping_vines.hit",
        "block.weeping_vines.fall",
        "SoundType.java:160-168",
    );
}

#[test]
fn lily_pad_place_differs_from_dig() {
    // S.java:19-27：LILY_PAD place=block.lily_pad.place（SE.java:1692），
    // 其余四事件走 BIG_DRIPLEAF 组。
    assert_family(
        t("LILY_PAD"),
        1.0,
        1.0,
        "block.big_dripleaf.break",
        "block.big_dripleaf.step",
        "block.lily_pad.place",
        "block.big_dripleaf.hit",
        "block.big_dripleaf.fall",
        "SoundType.java:19-27",
    );
}

#[test]
fn crop_planted_is_item_namespace() {
    // S.java:115-117：CROP place=SoundEvents.CROP_PLANTED="item.crop.plant"
    // （SE.java:476），step/hit/fall 走 GRASS 组。
    assert_family(
        t("CROP"),
        1.0,
        1.0,
        "block.crop.break",
        "block.grass.step",
        "item.crop.plant",
        "block.grass.hit",
        "block.grass.fall",
        "SoundType.java:115-117",
    );
}

#[test]
fn nether_wart_mixes_stone_step_and_item_plant() {
    // S.java:124-126：NETHER_WART step=block.stone.step、
    // place=item.nether_wart.plant（SE.java:1084）、dig/hit/fall=STONE 组。
    assert_family(
        t("NETHER_WART"),
        1.0,
        1.0,
        "block.nether_wart.break",
        "block.stone.step",
        "item.nether_wart.plant",
        "block.stone.hit",
        "block.stone.fall",
        "SoundType.java:124-126",
    );
}

#[test]
fn bamboo_sapling_step_borrows_bamboo() {
    // S.java:94-102：BAMBOO_SAPLING step=BAMBOO_STEP（block.bamboo.step），
    // fall=BAMBOO_FALL，dig/place/hit=block.bamboo_sapling.*。
    assert_family(
        t("BAMBOO_SAPLING"),
        1.0,
        1.0,
        "block.bamboo_sapling.break",
        "block.bamboo.step",
        "block.bamboo_sapling.place",
        "block.bamboo_sapling.hit",
        "block.bamboo.fall",
        "SoundType.java:94-102",
    );
}

#[test]
fn silent_slots_map_to_intentionally_empty() {
    // S.java:802-804：RESIN hit=EMPTY（SE.java:1100 "intentionally_empty"）。
    assert_family(
        t("RESIN"),
        1.0,
        1.0,
        "block.resin.break",
        "block.resin.step",
        "block.resin.place",
        "intentionally_empty",
        "block.resin.fall",
        "SoundType.java:802-804",
    );
    // S.java:412-414：CACTUS_FLOWER step/hit/fall=EMPTY。
    assert_family(
        t("CACTUS_FLOWER"),
        1.0,
        1.0,
        "block.cactus_flower.break",
        "intentionally_empty",
        "block.cactus_flower.place",
        "intentionally_empty",
        "intentionally_empty",
        "SoundType.java:412-414",
    );
}

#[test]
fn frogspawn_typo_constant_kept() {
    // S.java:583-585：FROGSPAWN step 引 SE.java:667 FROGSPAWNSTEP（无下划线
    // 常数名），事件串仍为 block.frogspawn.step。
    assert_family(
        t("FROGSPAWN"),
        1.0,
        1.0,
        "block.frogspawn.break",
        "block.frogspawn.step",
        "block.frogspawn.place",
        "block.frogspawn.hit",
        "block.frogspawn.fall",
        "SoundType.java:583-585",
    );
}

#[test]
fn all_java_types_present() {
    // SoundType.java 全部常数名（:7-819 声明序抽全量名单；生成表必须逐一在场）。
    const ALL: &[&str] = &[
        "EMPTY",
        "WOOD",
        "GRAVEL",
        "GRASS",
        "LILY_PAD",
        "STONE",
        "METAL",
        "GLASS",
        "WOOL",
        "SAND",
        "SNOW",
        "POWDER_SNOW",
        "LADDER",
        "ANVIL",
        "SLIME_BLOCK",
        "HONEY_BLOCK",
        "WET_GRASS",
        "CORAL_BLOCK",
        "BAMBOO",
        "BAMBOO_SAPLING",
        "SCAFFOLDING",
        "SWEET_BERRY_BUSH",
        "CROP",
        "HARD_CROP",
        "VINE",
        "NETHER_WART",
        "LANTERN",
        "STEM",
        "NYLIUM",
        "FUNGUS",
        "ROOTS",
        "SHROOMLIGHT",
        "WEEPING_VINES",
        "TWISTING_VINES",
        "SOUL_SAND",
        "SOUL_SOIL",
        "BASALT",
        "WART_BLOCK",
        "NETHERRACK",
        "NETHER_BRICKS",
        "NETHER_SPROUTS",
        "NETHER_ORE",
        "BONE_BLOCK",
        "NETHERITE_BLOCK",
        "ANCIENT_DEBRIS",
        "LODESTONE",
        "CHAIN",
        "NETHER_GOLD_ORE",
        "GILDED_BLACKSTONE",
        "CANDLE",
        "AMETHYST",
        "AMETHYST_CLUSTER",
        "SMALL_AMETHYST_BUD",
        "MEDIUM_AMETHYST_BUD",
        "LARGE_AMETHYST_BUD",
        "TUFF",
        "TUFF_BRICKS",
        "POLISHED_TUFF",
        "CALCITE",
        "DRIPSTONE_BLOCK",
        "POINTED_DRIPSTONE",
        "COPPER",
        "COPPER_BULB",
        "COPPER_GRATE",
        "COPPER_GOLEM_STATUE",
        "CAVE_VINES",
        "SPORE_BLOSSOM",
        "CACTUS_FLOWER",
        "AZALEA",
        "FLOWERING_AZALEA",
        "MOSS_CARPET",
        "PINK_PETALS",
        "LEAF_LITTER",
        "MOSS",
        "BIG_DRIPLEAF",
        "SMALL_DRIPLEAF",
        "ROOTED_DIRT",
        "HANGING_ROOTS",
        "AZALEA_LEAVES",
        "SCULK_SENSOR",
        "SCULK_CATALYST",
        "SCULK",
        "SCULK_VEIN",
        "SCULK_SHRIEKER",
        "GLOW_LICHEN",
        "DEEPSLATE",
        "DEEPSLATE_BRICKS",
        "DEEPSLATE_TILES",
        "POLISHED_DEEPSLATE",
        "FROGLIGHT",
        "FROGSPAWN",
        "MANGROVE_ROOTS",
        "MUDDY_MANGROVE_ROOTS",
        "MUD",
        "MUD_BRICKS",
        "PACKED_MUD",
        "HANGING_SIGN",
        "NETHER_WOOD_HANGING_SIGN",
        "BAMBOO_WOOD_HANGING_SIGN",
        "BAMBOO_WOOD",
        "NETHER_WOOD",
        "CHERRY_WOOD",
        "CHERRY_SAPLING",
        "CHERRY_LEAVES",
        "CHERRY_WOOD_HANGING_SIGN",
        "CHISELED_BOOKSHELF",
        "SHELF",
        "SUSPICIOUS_SAND",
        "SUSPICIOUS_GRAVEL",
        "DECORATED_POT",
        "DECORATED_POT_CRACKED",
        "TRIAL_SPAWNER",
        "SPONGE",
        "WET_SPONGE",
        "VAULT",
        "CREAKING_HEART",
        "HEAVY_CORE",
        "COBWEB",
        "SPAWNER",
        "RESIN",
        "RESIN_BRICKS",
        "IRON",
        "DRIED_GHAST",
    ];
    assert_eq!(
        SOUND_TYPES.len(),
        ALL.len(),
        "SoundType 类型数须与 Java 全表一致"
    );
    for name in ALL {
        assert!(
            sound_type_by_name(name).is_some(),
            "SoundType.java 类型 {name} 缺失"
        );
    }
}

#[test]
fn every_family_has_five_event_names() {
    // 五事件名一律非空且不带下划线前缀：EMPTY 槽位 = intentionally_empty；
    // place 槽位存在 item.* 命名空间（CROP/HARD_CROP = item.crop.plant
    // SE.java:476，NETHER_WART = item.nether_wart.plant），其余均 block.*。
    for (name, st) in SOUND_TYPES {
        for (kind, ev) in [
            ("dig", st.dig),
            ("step", st.step),
            ("place", st.place),
            ("hit", st.hit),
            ("fall", st.fall),
        ] {
            assert!(!ev.is_empty(), "{name}.{kind} 事件名空");
            assert!(
                ev == "intentionally_empty" || ev.starts_with("block.") || ev.starts_with("item."),
                "{name}.{kind} 事件名非法: {ev}"
            );
        }
    }
}

#[test]
fn block_mapping_direct_registrations() {
    // Blocks.java 注册归属抽查（行号在断言消息）。
    // oak_planks → WOOD（Blocks.java:90-91）。
    assert!(std::ptr::eq(
        try_sound_type_for_block("oak_planks").unwrap(),
        t("WOOD")
    ));
    // stone → STONE（Blocks.java:60-62，显式 .sound(SoundType.STONE)）。
    assert!(std::ptr::eq(sound_type_for_block("stone"), t("STONE")));
    // grass_block → GRASS（Blocks.java:82）。
    assert!(std::ptr::eq(
        sound_type_for_block("grass_block"),
        t("GRASS")
    ));
    // dirt → GRAVEL（Blocks.java:84，register(BlockIds.DIRT,…)+GRAVEL）。
    assert!(std::ptr::eq(sound_type_for_block("dirt"), t("GRAVEL")));
    // iron_block → IRON 而非 METAL（Blocks.java:853-861）。
    assert!(std::ptr::eq(sound_type_for_block("iron_block"), t("IRON")));
    // water → EMPTY（Blocks.java:197-208 显式 .sound(SoundType.EMPTY)）。
    assert!(std::ptr::eq(sound_type_for_block("water"), t("EMPTY")));
    // ladder → LADDER。
    assert!(std::ptr::eq(sound_type_for_block("ladder"), t("LADDER")));
}

#[test]
fn block_mapping_copy_and_helper_registrations() {
    // oak_stairs 走 registerLegacyStair(OAK_PLANKS) 属性拷贝（Blocks.java:1013 +
    // :5989-5996 ofLegacyCopy）→ WOOD。
    assert!(std::ptr::eq(sound_type_for_block("oak_stairs"), t("WOOD")));
    // cobblestone_wall 走 ofLegacyCopy(COBBLESTONE)（Blocks.java:2219）→ STONE。
    assert!(std::ptr::eq(
        sound_type_for_block("cobblestone_wall"),
        t("STONE")
    ));
    // white_bed 走 registerBed → WOOD（Blocks.java:548 + :5873-5885）。
    assert!(std::ptr::eq(sound_type_for_block("white_bed"), t("WOOD")));
    // red_stained_glass 走 registerStainedGlass → GLASS（:5916-5931）。
    assert!(std::ptr::eq(
        sound_type_for_block("red_stained_glass"),
        t("GLASS")
    ));
    // acacia_log 走 logProperties(…,SoundType.WOOD)（Blocks.java:304 + :5887-5894）。
    assert!(std::ptr::eq(sound_type_for_block("acacia_log"), t("WOOD")));
    // oak_leaves 走 leavesProperties(SoundType.GRASS)（Blocks.java:450）。
    assert!(std::ptr::eq(sound_type_for_block("oak_leaves"), t("GRASS")));
}

#[test]
fn block_mapping_legacy_engine_names() {
    // 千块表 id 0..13 旧名（ci/gen-blocks.py LEGACY 对应官方名）。
    assert!(std::ptr::eq(sound_type_for_block("grass"), t("GRASS"))); // grass_block
    assert!(std::ptr::eq(sound_type_for_block("snow_grass"), t("GRASS"))); // grass_block[snowy]
    assert!(std::ptr::eq(sound_type_for_block("log"), t("WOOD"))); // oak_log
    assert!(std::ptr::eq(sound_type_for_block("leaves"), t("GRASS"))); // oak_leaves
    assert!(std::ptr::eq(sound_type_for_block("planks"), t("WOOD"))); // oak_planks
    assert!(std::ptr::eq(sound_type_for_block("cobble"), t("STONE"))); // cobblestone
    assert!(std::ptr::eq(sound_type_for_block("flower_red"), t("GRASS"))); // poppy（Blocks.java:764-768 GRASS）
    assert!(std::ptr::eq(
        sound_type_for_block("flower_yellow"),
        t("GRASS")
    )); // dandelion
}

#[test]
fn unknown_block_defaults_to_stone() {
    // BlockBehaviour.Properties 默认 soundType=STONE（BlockBehaviour.java:973）。
    assert!(try_sound_type_for_block("no_such_block").is_none());
    assert!(std::ptr::eq(
        sound_type_for_block("no_such_block"),
        t("STONE")
    ));
}

#[test]
fn block_table_sorted_for_binary_search() {
    // BLOCK_SOUND_TYPES 按名字节序升序 + 无重名（sound_type_for_block 的
    // binary_search 前提）。
    for w in mcv_audio::BLOCK_SOUND_TYPES.windows(2) {
        assert!(w[0].0 < w[1].0, "乱序/重名: {} >= {}", w[0].0, w[1].0);
    }
}
