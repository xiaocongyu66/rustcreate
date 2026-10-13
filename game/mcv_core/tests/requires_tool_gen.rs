//! GEN_BLOCKS 第 9 列 `requires_tool` 对拍锁（#178 刀2a）。
//!
//! 三真源交叉验证生成列（ci/gen-blocks.py 解析 Blocks.java
//! `requiresCorrectToolForDrops()` + ofFullCopy/ofLegacyCopy :1054 继承闭包）：
//!   真源 A = 本列（属性侧，掉落门成员资格）；
//!   真源 B = needs_stone/iron/diamond_tool tag（层级门槛侧，
//!            tags_gen.rs 已独立对拍成员数）；
//!   真源 C = mineable/pickaxe|shovel|axe|hoe tag（可挖工具侧）。
//! 原版语义依据：BlockBehaviour.java:438/1213（属性）+ Player.java:619
//! （`!requiresCorrectToolForDrops() || Tool.isCorrectForDrops`）+
//! Tool.java:50-57（无规则匹配 = false）。
//! 版权卫生：只写方块名（事实）与计数，不含原版 JSON/Java 原文。

use mcv_core::tags::{
    TAG_BLOCK_MINEABLE__AXE, TAG_BLOCK_MINEABLE__HOE, TAG_BLOCK_MINEABLE__PICKAXE,
    TAG_BLOCK_MINEABLE__SHOVEL, TAG_BLOCK_NEEDS_DIAMOND_TOOL, TAG_BLOCK_NEEDS_IRON_TOOL,
    TAG_BLOCK_NEEDS_STONE_TOOL,
};
use mcv_core::{BLOCKS, BlockId};

fn gated_names() -> Vec<&'static str> {
    BLOCKS
        .iter()
        .filter(|b| b.requires_tool)
        .map(|b| b.name)
        .collect()
}

fn id_of(name: &str) -> u16 {
    BLOCKS
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("BLOCKS 缺方块 {name}")) as u16
}

fn requires(name: &str) -> bool {
    BLOCKS[id_of(name) as usize].requires_tool
}

/// 总量锁：411 方块带掉落门（Blocks.java 212 处直接置位 + 继承闭包 ∩
/// 引擎注册表；重生成漂移即红）。
#[test]
fn gate_total_count() {
    assert_eq!(gated_names().len(), 411);
}

/// 真源 B：needs_*_tool（层级门槛）成员除 crafter 外必须全部 gate=true
/// ——原版 needs tag 只给层级门槛、掉落门另有属性，两真源的包含关系
/// 在此锁死。crafter 例外 = 实物：Blocks.java:5753 属性链无
/// requiresCorrectToolForDrops（原版怪癖：徒手可掉、木/金镐反而 denies
/// ——incorrect_for_wooden 含之），引擎登记为已知偏差（见刀2 交卷报告）。
#[test]
fn needs_tags_gate_except_crafter() {
    for t in [
        &TAG_BLOCK_NEEDS_STONE_TOOL,
        &TAG_BLOCK_NEEDS_IRON_TOOL,
        &TAG_BLOCK_NEEDS_DIAMOND_TOOL,
    ] {
        for id in t.set.iter().copied() {
            if BLOCKS[id as usize].name == "crafter" {
                continue;
            }
            assert!(
                BLOCKS[id as usize].requires_tool,
                "needs tag 成员 {} 未入门",
                BLOCKS[id as usize].name
            );
        }
    }
    assert!(!requires("crafter"), "crafter 原版属性无掉落门");
}

/// 真源 C：gate=true 但 ∉ 任何 mineable tag 的方块 = 原版「任何工具都
/// 不正确」（Tool.isCorrectForDrops 无规则匹配 → false，Player.java:619
/// gate 又真）的实物全集，恰好这 6 个（命令块族 5 = 原版永无掉落途径；
/// cobweb = 只有剪刀/剑特例 Tool 组件可掉，引擎未建模该组件）。
/// 集合外的 gate 方块必须有 pickaxe 或 shovel 工具途径。
#[test]
fn gate_without_mineable_kind_is_exactly_six() {
    let mut got: Vec<&str> = BLOCKS
        .iter()
        .enumerate()
        .filter(|(i, b)| {
            b.requires_tool
                && !TAG_BLOCK_MINEABLE__PICKAXE.set.contains(*i as u16)
                && !TAG_BLOCK_MINEABLE__SHOVEL.set.contains(*i as u16)
                && !TAG_BLOCK_MINEABLE__AXE.set.contains(*i as u16)
                && !TAG_BLOCK_MINEABLE__HOE.set.contains(*i as u16)
        })
        .map(|(_, b)| b.name)
        .collect();
    got.sort_unstable();
    assert_eq!(
        got,
        [
            "chain_command_block",
            "cobweb",
            "command_block",
            "jigsaw",
            "repeating_command_block",
            "structure_block",
        ]
    );
    // gate 与 axe/hoe tag 零交集（26.1 实物；故 ToolReq 无需扩 Axe/Hoe）。
    for (i, b) in BLOCKS.iter().enumerate() {
        if b.requires_tool {
            let id = i as u16;
            assert!(
                !TAG_BLOCK_MINEABLE__AXE.set.contains(id)
                    && !TAG_BLOCK_MINEABLE__HOE.set.contains(id),
                "{} gate∩axe/hoe 意外成员",
                b.name
            );
        }
    }
}

/// 实物抽查（旧名单三档代表 + 反例 + 旧 14 特例 bedrock：原版走
/// noLootTable 不走掉落门属性 → false）。
#[test]
fn spot_checks_match_vanilla() {
    for n in [
        "stone",
        "cobble",
        "iron_ore",
        "copper_ore",
        "gold_ore",
        "obsidian",
        "ancient_debris",
        "snow",
        "snow_block",
        "deepslate_diamond_ore",
        "copper_bars",
        "waxed_weathered_copper_chain",
    ] {
        assert!(requires(n), "{n} 应有掉落门");
    }
    for n in [
        "dirt",
        "grass",
        "snow_grass",
        "sand",
        "log",
        "planks",
        "leaves",
        "glass",
        "torch",
        "bedrock",
        "crafter",
    ] {
        assert!(!requires(n), "{n} 不应有掉落门");
    }
    // needs_diamond_tool 成员全集 = 钻镐级 5（与旧 TIER_DIAMOND_PICKAXE 同集）。
    let mut d: Vec<&str> = TAG_BLOCK_NEEDS_DIAMOND_TOOL
        .set
        .iter()
        .map(|i| BLOCKS[*i as usize].name)
        .collect();
    d.sort_unstable();
    assert_eq!(
        d,
        [
            "ancient_debris",
            "crying_obsidian",
            "netherite_block",
            "obsidian",
            "respawn_anchor"
        ]
    );
}

/// id 域一致性：tag 成员 id 全部 < BLOCKS.len()（二分前置）。
#[test]
fn tag_ids_in_registry_domain() {
    let n = BLOCKS.len() as u16;
    for t in [
        &TAG_BLOCK_MINEABLE__PICKAXE as &mcv_registry::TagEntry,
        &TAG_BLOCK_MINEABLE__SHOVEL,
        &TAG_BLOCK_MINEABLE__AXE,
        &TAG_BLOCK_MINEABLE__HOE,
        &TAG_BLOCK_NEEDS_STONE_TOOL,
        &TAG_BLOCK_NEEDS_IRON_TOOL,
        &TAG_BLOCK_NEEDS_DIAMOND_TOOL,
    ] {
        assert!(
            t.set.iter().all(|i| *i < n),
            "{} 成员越出注册表",
            t.key.path
        );
    }
    // BlockId 路径等价：requires_tool 经 BlockId::def() 与直接索引一致。
    let b = BlockId(id_of("stone"));
    assert!(b.def().requires_tool);
}
