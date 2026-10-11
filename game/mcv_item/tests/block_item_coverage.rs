//! 物品覆盖快照审计（#147 审计波）——ITEMS × BLOCKS 交叉基线，只读两表。
//!
//! 快照时点：#144 物品注册全量扩表波合并后实测（2026-10-12）。快照断言
//! 按扩波实测值 bump（单调增长方向）；名单硬约束（引用越界）不受扩表
//! 影响；名字一致性新增「原版名实不一族」白名单，见
//! `modern_block_items_name_match_block` 注。
//!
//! 审计快照：`ITEMS` 1381 条，其中 `ItemKind::Block` 1023 条、指向 1021 个
//! 不同方块 id → 可放置物品覆盖 1021/1171 = 87.19%（完整缺口矩阵见
//! docs/block-coverage.md §物品轴）。

use mcv_core::{BLOCKS, BlockId};
use mcv_item::{ITEMS, ItemKind};

/// 物品表规模快照：#144 扩表波合并实测 1381 条（注册序即序列化 id 序，
/// 禁重排——lib.rs 注）。此后只许增长。
#[test]
fn items_registry_snapshot_baseline() {
    assert_eq!(
        ITEMS.len(),
        1381,
        "ITEMS 规模偏离审计快照——扩表合并请按新实测 bump 本断言"
    );
}

/// 方块物品引用硬不变量：每个 `ItemKind::Block(BlockId(n))` 必须指向
/// BLOCKS 注册表内的有效 id（越界=放置路径 panic 面）。
#[test]
fn block_item_refs_are_valid_block_ids() {
    let mut covered = std::collections::BTreeSet::new();
    for it in ITEMS.iter() {
        if let ItemKind::Block(BlockId(id)) = it.kind {
            assert!(
                (id as usize) < BLOCKS.len(),
                "物品 {} 引用越界方块 id {}（BLOCKS.len()={}）",
                it.name,
                id,
                BLOCKS.len()
            );
            covered.insert(id);
        }
    }
    // 覆盖下限（#144 扩表后实测 1021/1171 = 87.19%）：只许增长。
    assert!(
        covered.len() >= 1021,
        "可放置方块物品覆盖 {} < 审计基线 1021（物品表被削减？）",
        covered.len()
    );
    eprintln!(
        "ITEM-COVERAGE 快照: {}/{} 方块有物品（{:.2}%）",
        covered.len(),
        BLOCKS.len(),
        100.0 * covered.len() as f64 / BLOCKS.len() as f64
    );
}

/// 放置一致性：方块物品名与方块注册名的关系分两档——
/// ①旧 14 时代占位别名（id < 14，登记 docs/block-coverage.md §物品轴）：
/// `cobblestone`→`cobble`（旧名方块）、`iron_ore`/`diamond_ore`→`stone`
/// （矿物方块未扩表前的占位）、`poppy`/`dandelion`→`flower_red`/
/// `flower_yellow`（旧花槽位的官方名别名，#144 扩波新增 def 挂旧位）；
/// ②id ≥ 14 的新表只允许「原版名实不一族」：26.1 Items.java 以
/// `createBlockItemWithCustomItemName(Blocks.Y)` 用**物品名**注册方块物品
/// （放置/挖掘往返走 BlockId，不依赖名字相等），物品名 ≠ 方块注册名是
/// 官方语义，逐对（物品名, 方块注册名, Items.java:行）锁定如下；族外
/// 名实不一仍判红。
#[test]
fn modern_block_items_name_match_block() {
    const LEGACY_ALIAS: [&str; 5] = [
        "cobblestone",
        "dandelion",
        "diamond_ore",
        "iron_ore",
        "poppy",
    ];
    const VANILLA_DIVERGED: [(&str, &str, u32); 11] = [
        // 26.1 net/minecraft/world/item/Items.java（注册名权威，src-26.1）
        ("beetroot_seeds", "beetroots", 1817),
        ("carrot", "carrots", 1639),
        ("cocoa_beans", "cocoa", 1442),
        ("glow_berries", "cave_vines", 2085),
        ("melon_seeds", "melon_stem", 1496),
        ("pitcher_pod", "pitcher_crop", 1815),
        ("potato", "potatoes", 1640),
        ("pumpkin_seeds", "pumpkin_stem", 1495),
        ("sweet_berries", "sweet_berry_bush", 2082),
        ("torchflower_seeds", "torchflower_crop", 1814),
        ("wheat_seeds", "wheat", 1177),
    ];
    for it in ITEMS.iter() {
        if let ItemKind::Block(BlockId(id)) = it.kind {
            let b = &BLOCKS[id as usize];
            if it.name != b.name {
                if id < 14 {
                    assert!(
                        LEGACY_ALIAS.contains(&it.name),
                        "物品 {} 与方块 {} 名字不一致且不在旧表占位别名白名单",
                        it.name,
                        b.name
                    );
                } else {
                    assert!(
                        VANILLA_DIVERGED
                            .iter()
                            .any(|(n, bn, _)| it.name == *n && b.name == *bn),
                        "新表方块物品 {}（→{}）名实不一且不在原版 \
                         createBlockItemWithCustomItemName 族白名单",
                        it.name,
                        b.name
                    );
                }
            }
        }
    }
}
