//! 物品覆盖快照审计（#147 审计波）——ITEMS × BLOCKS 交叉基线，只读两表。
//!
//! 快照时点：基座 main=abf83707（2026-10-12 实测）。items 扩表波（任务板
//! #144）并行进行中：其合并后本测试的快照断言按新实测值 bump（单调增长方向），
//! 名单硬约束（引用越界/名字一致性）不受扩表影响。
//!
//! 审计快照：`ITEMS` 152 条，其中 `ItemKind::Block` 100 条、指向 98 个不同
//! 方块 id → 可放置物品覆盖 98/1171 = 8.37%（完整缺口矩阵见
//! docs/block-coverage.md §物品轴）。

use mcv_core::{BLOCKS, BlockId};
use mcv_item::{ITEMS, ItemKind};

/// 物品表规模快照：基座 152 条（注册序即序列化 id 序，禁重排——lib.rs 注）。
/// #144 扩表合并后按新实测更新（只许增长）。
#[test]
fn items_registry_snapshot_baseline() {
    assert_eq!(
        ITEMS.len(),
        152,
        "ITEMS 规模偏离审计快照——若为 #144 扩表合并请按新实测 bump 本断言"
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
    // 覆盖下限（快照 98/1171 = 8.37%）：#144 扩表后只许增长。
    assert!(
        covered.len() >= 98,
        "可放置方块物品覆盖 {} < 审计基线 98（物品表被削减？）",
        covered.len()
    );
    eprintln!(
        "ITEM-COVERAGE 快照: {}/{} 方块有物品（{:.2}%）",
        covered.len(),
        BLOCKS.len(),
        100.0 * covered.len() as f64 / BLOCKS.len() as f64
    );
}

/// 放置一致性：新表（id ≥ 14）方块物品的物品名必须与方块注册名一致
/// （放置/挖掘往返的既定约定）；旧 14 时代的占位别名白名单如下——
/// `cobblestone`→`cobble`（旧名方块）、`iron_ore`/`diamond_ore`→`stone`
/// （矿物方块未扩表前的占位，均登记 docs/block-coverage.md §物品轴）。
#[test]
fn modern_block_items_name_match_block() {
    const LEGACY_ALIAS: [&str; 3] = ["cobblestone", "diamond_ore", "iron_ore"];
    for it in ITEMS.iter() {
        if let ItemKind::Block(BlockId(id)) = it.kind {
            let b = &BLOCKS[id as usize];
            if it.name != b.name {
                assert!(
                    LEGACY_ALIAS.contains(&it.name),
                    "物品 {} 与方块 {} 名字不一致且不在旧表占位别名白名单",
                    it.name,
                    b.name
                );
                assert!(
                    id < 14,
                    "新表方块物品 {}（→{}）必须名字一致（id {id} ≥ 14）",
                    it.name,
                    b.name
                );
            }
        }
    }
}
