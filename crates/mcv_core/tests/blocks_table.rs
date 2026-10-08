//! 方块表校准测试：逐项对照反编译 Minecraft 26.1
//! `net/minecraft/world/level/block/Blocks.java` 中的 `BlockBehaviour.Properties`
//! （`strength(x)` 的第一参数 = 生存挖掘硬度 hardness）。
//! 完整数据出处见仓库外笔记 `/root/mc-ref/NOTES-blocks.md`。

use mcv_core::BlockDef;

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
    assert_eq!(
        table.len(),
        mcv_core::BLOCKS.len(),
        "对照表应覆盖全部注册方块"
    );
    for &(ours, mc, h) in table {
        let def = by_name(ours);
        assert_eq!(def.hardness, h, "方块 {ours}（MC: {mc}）硬度应为 {h}");
    }
}

/// 发光等级对照：当前子集在 MC 26.1 中全部 lightLevel=0
/// （火把 14 / 荧石 15 / 岩浆 15 等发光块尚未进注册表）。
#[test]
fn light_emit_matches_mc_26_1() {
    for def in mcv_core::BLOCKS.iter() {
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
    // 反向保护：别把别的方块误标成不可挖
    for def in mcv_core::BLOCKS.iter() {
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

/// BlockId 索引必须与注册表顺序一致（0 = 空气）。
#[test]
fn block_id_indices_stable() {
    assert_eq!(mcv_core::AIR.def().name, "air");
    for (i, def) in mcv_core::BLOCKS.iter().enumerate() {
        assert_eq!(mcv_core::BlockId(i as u16).def().name, def.name);
    }
}
