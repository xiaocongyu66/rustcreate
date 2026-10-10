//! 方块表校准测试：逐项对照反编译 Minecraft 26.1
//! `net/minecraft/world/level/block/Blocks.java` 中的 `BlockBehaviour.Properties`
//! （`strength(x)` 的第一参数 = 生存挖掘硬度 hardness）。
//! 完整数据出处见仓库外笔记 `mc-ref/NOTES-blocks.md`。

use mcv_core::BlockDef;

include!("vanilla_blocks_gen.inc");

/// 按名字查 BLOCKS 注册表条目。
fn by_name(name: &str) -> &'static BlockDef {
    mcv_core::BLOCKS
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("BLOCKS 注册表缺少方块: {name}"))
}

/// MC 用 strength(-1) 表示不可挖，本引擎的既有表示法是 f32::INFINITY。
const UNBREAKABLE: f32 = f32::INFINITY;

/// 硬度逐项对照 MC 26.1 注册表值。
#[test]
fn hardness_matches_mc_26_1() {
    // (我们的方块名, MC 26.1 方块, strength 硬度)
    let table: &[(&str, &str, f32)] = &[
        ("air", "air", 0.0),                 // instabreak/air
        ("stone", "stone", 1.5),             // strength(1.5, 6.0)
        ("dirt", "dirt", 0.5),               // strength(0.5)
        ("grass", "grass_block", 0.6),       // strength(0.6)
        ("sand", "sand", 0.5),               // strength(0.5)
        ("water", "water", 100.0),           // strength(100.0)
        ("log", "oak_log", 2.0),             // logProperties: strength(2.0)
        ("leaves", "oak_leaves", 0.2),       // leavesProperties: strength(0.2)
        ("planks", "oak_planks", 2.0),       // strength(2.0, 3.0)
        ("cobble", "cobblestone", 2.0),      // strength(2.0, 6.0)
        ("bedrock", "bedrock", UNBREAKABLE), // strength(-1, 3600000)
        // 复合外观方块：本体按 grass_block 0.6（雪层 strength(0.1) 属独立方块）
        ("snow_grass", "grass_block(覆雪)", 0.6),
        ("flower_red", "poppy", 0.0),        // instabreak
        ("flower_yellow", "dandelion", 0.0), // instabreak
    ];
    // 千块表（1171 项）接入后，本对照表只覆盖旧 14 方块基线；
    // 其余条目按字典序随生成表进表（源头见 blocks_gen.inc.rs 注释）。
    assert!(
        mcv_core::BLOCKS.len() >= table.len(),
        "千块表接入后 BLOCKS 应 ≥ 基线对照表"
    );
    for &(ours, mc, h) in table {
        let def = by_name(ours);
        assert_eq!(def.hardness, h, "方块 {ours}（MC: {mc}）硬度应为 {h}");
    }
}

/// 发光等级对照：旧 14 方块基线在 MC 26.1 中全部 lightLevel=0
/// （火把 14 / 荧石 15 / 岩浆 15 等发光块由千块表带入，不在本表断言范围）。
#[test]
fn light_emit_matches_mc_26_1() {
    // 基线 14 方块 id 0..=13（mcv_core::tests::first_14_match_legacy_table 锁序）
    for def in mcv_core::BLOCKS[..14].iter() {
        assert_eq!(
            def.light_emit, 0,
            "{} 在 MC 26.1 中不发光，应为 0",
            def.name
        );
    }
}

/// 基岩必须不可破坏；普通方块必须可破坏（硬度为有限值）。
#[test]
fn bedrock_is_unbreakable() {
    let bedrock = by_name("bedrock");
    assert_eq!(bedrock.hardness, UNBREAKABLE, "基岩必须用不可挖表示法");
    assert!(bedrock.hardness.is_infinite(), "基岩硬度必须是无穷大");
    // 反向保护：基线 14 方块里别把别的方块误标成不可挖
    // （千块表里屏障/末地门框架等官方也是 -1 硬度，不在本断言范围）
    for def in mcv_core::BLOCKS[..14].iter() {
        if def.name != "bedrock" {
            assert!(
                def.hardness.is_finite(),
                "非基岩方块 {} 的硬度必须是有限值",
                def.name
            );
        }
    }
}

/// solid/opaque/liquid 标志对照 MC 26.1 语义：
/// - `solid`   ≈ hasCollision（MC `noCollision()` → false）
/// - `opaque`  ≈ canOcclude（MC `noOcclusion()`/`noCollision()` → false）
/// - `liquid`  ≈ Properties.liquid()
#[test]
fn flags_match_mc_26_1() {
    // (方块名, solid, opaque, liquid)
    let table: &[(&str, bool, bool, bool)] = &[
        ("air", false, false, false), // noCollision()
        ("stone", true, true, false),
        ("dirt", true, true, false),
        ("grass", true, true, false),
        ("sand", true, true, false),
        ("water", false, false, true), // noCollision() + liquid()
        ("log", true, true, false),
        ("leaves", true, false, false), // 有碰撞但 noOcclusion()
        ("planks", true, true, false),
        ("cobble", true, true, false),
        ("bedrock", true, true, false),
        ("snow_grass", true, true, false), // 本体 grass_block：solid + occluding
        ("flower_red", false, false, false), // noCollision()
        ("flower_yellow", false, false, false),
    ];
    for &(name, solid, opaque, liquid) in table {
        let def = by_name(name);
        assert_eq!(def.solid, solid, "{name}: solid(hasCollision) 不符");
        assert_eq!(def.opaque, opaque, "{name}: opaque(canOcclude) 不符");
        assert_eq!(def.liquid, liquid, "{name}: liquid 不符");
    }
}

/// B2 条件光表：`litBlockEmission(n)`（`state -> lit ? n : 0`，
/// Blocks.java:5853-5855）是【按 LIT 状态】的条件发光，本引擎无方块状态、
/// 表行 = **默认放置态**的发光值——即 `registerDefaultState` 里各块自己的
/// LIT 默认值。因此分两族：
/// - 默认未点燃（LIT=false）→ 0：炉族 litBlockEmission(13/13/13)
///   （Blocks.java:1064/4535/4545）、红石矿/深板岩红石矿(9)（:1581）、
///   红石灯(15)（:2167）、铜灯族（CopperBulbBlock.java:30 默认
///   LIT=false）、蜡烛蛋糕（:5032）与蜡烛（CandleBlock.java:76 默认
///   LIT=false，LIGHT_EMISSION `lit ? 3*candles : 0`，:43）。
/// - 默认点亮（registerDefaultState LIT=true）→ 保留 n：营火(15)
///   （CampfireBlock.java:84）、灵魂营火(10)、红石火把/墙 torch(7)
///   （RedstoneTorchBlock.java:40）。这些放置即发光，与 26.1 一致。
#[test]
fn conditional_light_matches_default_lit_state() {
    for n in [
        "furnace",
        "blast_furnace",
        "smoker",
        "redstone_lamp",
        "redstone_ore",
        "deepslate_redstone_ore",
        "copper_bulb",
        "waxed_oxidized_copper_bulb",
        "candle_cake",
        "candle",
        "white_candle",
    ] {
        assert_eq!(by_name(n).light_emit, 0, "{n} 默认未点燃，光级必须 0");
    }
    // 默认放置态即点亮的一族：放置就发光。
    assert_eq!(by_name("campfire").light_emit, 15, "营火默认 lit=true");
    assert_eq!(
        by_name("soul_campfire").light_emit,
        10,
        "灵魂营火默认 lit=true"
    );
    assert_eq!(
        by_name("redstone_torch").light_emit,
        7,
        "红石火把默认 lit=true"
    );
    assert_eq!(
        by_name("redstone_wall_torch").light_emit,
        7,
        "红石墙 torch 同上"
    );
    // 反向保护：恒发光（不看 LIT 态）的方块不得被连带清零——
    // torch 14（Blocks.java:966）、glowstone 15、fire 15、lantern 15。
    assert_eq!(by_name("torch").light_emit, 14, "torch 恒发光 14 不受影响");
    assert_eq!(by_name("glowstone").light_emit, 15);
    assert_eq!(by_name("fire").light_emit, 15);
    assert_eq!(by_name("lantern").light_emit, 15);
}

/// BlockId 索引必须与注册表顺序一致（0 = 空气）。
#[test]
fn block_id_indices_stable() {
    assert_eq!(mcv_core::AIR.def().name, "air");
    for (i, def) in mcv_core::BLOCKS.iter().enumerate() {
        assert_eq!(mcv_core::BlockId(i as u16).def().name, def.name);
    }
}

// ---------------------------------------------------------------------------
// 覆盖率与完整性（M7a 全方块覆盖审计新增）
// ---------------------------------------------------------------------------

/// 旧 14 方块与官方注册表名的对应（id 0-13 兼容约束：旧名不可改，按别名覆盖）。
const LEGACY_ALIASES: [(&str, &str); 7] = [
    ("grass_block", "grass"),
    ("oak_log", "log"),
    ("oak_leaves", "leaves"),
    ("oak_planks", "planks"),
    ("cobblestone", "cobble"),
    ("poppy", "flower_red"),
    ("dandelion", "flower_yellow"),
];

/// 官方 26.1 注册表全名单（vanilla_blocks_gen.inc，1168 名）必须 100% 被表覆盖：
/// 按官方名直查，旧 14 方块的 7 个官方名走别名。这是「还原官方几千个方块」
/// 的名字层基线：名单由 ci/gen-blocks.py 与方块表同一次生成，永不漂移。
#[test]
fn vanilla_registry_coverage_100_percent() {
    // 表行数 = 官方 1168 − 13 个被旧 14 行按别名覆盖的官方名 + 16
    // （旧 14 行，其中 snow_grass 是 grass_block[snowy] 状态分裂 + item_frame
    // /glow_item_frame 引擎实体占位）= 1171。
    assert_eq!(
        mcv_core::BLOCKS.len(),
        VANILLA_26_1_BLOCKS.len() + 3,
        "表行数与官方名单的关系被破坏（新增方块须同步 gen-blocks.py 产物）"
    );
    let find = |n: &str| mcv_core::BLOCKS.iter().any(|b| b.name == n);
    let mut miss = Vec::new();
    for &name in VANILLA_26_1_BLOCKS.iter() {
        if find(name)
            || LEGACY_ALIASES
                .iter()
                .any(|(off, ours)| *off == name && find(ours))
        {
            continue;
        }
        miss.push(name);
    }
    let covered = VANILLA_26_1_BLOCKS.len() - miss.len();
    let pct = covered * 100 / VANILLA_26_1_BLOCKS.len();
    assert!(
        pct >= 99,
        "官方注册表覆盖率 {pct}%（{covered}/{}），缺失：{miss:?}",
        VANILLA_26_1_BLOCKS.len()
    );
    assert!(miss.is_empty(), "缺失官方方块：{miss:?}");
}

/// 完整性：所有方块硬度非负且非 NaN；纯立方实体块（solid+opaque）硬度必须
/// >0（或基岩类 inf），除非命中原版 destroyTime=0 的瞬破白名单。
#[test]
fn table_integrity_hardness_and_layers() {
    // 原版无 .strength()（destroyTime 默认 0.0F）的纯立方实体块，逐个对照
    // Blocks.java 核实：tnt(0.0)、resin_block(0.0)、虫蚀石族 InfestedBlock。
    const INSTANT_CUBE_ALLOWLIST: [&str; 9] = [
        "tnt",
        "resin_block",
        "infested_stone",
        "infested_cobblestone",
        "infested_stone_bricks",
        "infested_chiseled_stone_bricks",
        "infested_cracked_stone_bricks",
        "infested_mossy_stone_bricks",
        "infested_deepslate",
    ];
    for def in mcv_core::BLOCKS.iter() {
        assert!(
            !def.hardness.is_nan() && def.hardness >= 0.0,
            "{} 硬度必须非负非 NaN",
            def.name
        );
        if def.solid && def.opaque && !def.liquid && def.hardness == 0.0 {
            assert!(
                INSTANT_CUBE_ALLOWLIST.contains(&def.name),
                "纯立方实体块 {} 硬度 0（瞬破）却不在白名单，疑似占位洞",
                def.name
            );
        }
    }
}

/// 占位洞清零抽查：此前属性提取漏掉的族，逐项对照 Java 值
/// （copper 系 Blocks.java:1934/1944/4602；button 5970；piston 5960；
/// 默认 destroyTime=0.0 BlockBehaviour.java:976）。
#[test]
fn attribute_hotspots_match_java() {
    let by = by_name;
    // 铜栏杆/铜锁链：strength(5.0, 6.0) + noOcclusion；八变体共享。
    for n in [
        "copper_bars",
        "waxed_oxidized_copper_bars",
        "weathered_copper_chain",
    ] {
        let d = by(n);
        assert_eq!(d.hardness, 5.0, "{n} 硬度");
        assert!(!d.opaque, "{n} noOcclusion → opaque=false");
        assert!(d.solid, "{n} 有碰撞");
        assert_eq!(d.light_emit, 0, "{n} 不发光");
    }
    // 铜灯笼：strength(3.5) + lightLevel(15) + noOcclusion。
    for n in [
        "copper_lantern",
        "exposed_copper_lantern",
        "waxed_copper_lantern",
    ] {
        let d = by(n);
        assert_eq!(d.hardness, 3.5, "{n} 硬度");
        assert_eq!(d.light_emit, 15, "{n} lightLevel=15");
        assert!(!d.opaque, "{n} noOcclusion → opaque=false");
    }
    // 按钮：buttonProperties() = noCollision + strength(0.5)。
    for n in [
        "stone_button",
        "oak_button",
        "acacia_button",
        "bamboo_button",
        "warped_button",
    ] {
        let d = by(n);
        assert_eq!(d.hardness, 0.5, "{n} 硬度");
        assert!(!d.solid, "{n} noCollision → solid=false");
    }
    // 活塞：pistonProperties() = strength(1.5)。
    for n in ["piston", "sticky_piston"] {
        assert_eq!(by(n).hardness, 1.5, "{n} 硬度");
    }
    // 无 strength() 即原版默认 0.0（此前误落 2.0）。
    for n in [
        "honey_block",
        "slime_block",
        "infested_stone",
        "cave_air",
        "void_air",
        "structure_void",
        "bubble_column",
        "nether_wart",
        "tripwire",
        "resin_clump",
        "scaffolding",
        "pink_petals",
    ] {
        assert_eq!(by(n).hardness, 0.0, "{n} 原版 destroyTime 默认 0.0");
    }
    // 屏障/光源：strength(-1) → inf，有碰撞（Blocks.java:2564/2576）。
    for n in ["barrier", "light"] {
        let d = by(n);
        assert!(d.hardness.is_infinite(), "{n} 不可挖");
        assert!(d.solid, "{n} 有碰撞");
        assert!(!d.opaque, "{n} noOcclusion → opaque=false");
    }
    // 隐形方块在生成表黄金件（#77 删除 cpp/ 前的 cpp/src/blocks_gen.inc
    // 逐字节快照）里 geom=false（对照行内注释锁定）。
    let golden = include_str!("../../mcv_mesher/tests/golden/blocks_gen.inc");
    for n in [
        "barrier",
        "light",
        "cave_air",
        "void_air",
        "structure_void",
        "bubble_column",
    ] {
        let line = golden
            .lines()
            .find(|l| l.trim_end().ends_with(&format!("// {n}")))
            .unwrap_or_else(|| panic!("生成表缺行：{n}"));
        let geom_true = line.contains(", true, {");
        assert!(
            !geom_true,
            "隐形方块 {n} 的 geom 必须为 false（原版不可见）"
        );
    }
}
