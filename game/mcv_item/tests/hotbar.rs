//! Hotbar:add 的 vanilla 优先级(选中槽合并→全局合并→选中空→任意空)、
//! 堆叠上限、附魔/耐久不合并、满栏剩余、take_one。

use mcv_item::inventory::{HOTBAR_SLOTS, Hotbar};
use mcv_item::{COBBLESTONE, IRON_SWORD_INDEX, ItemKind, ItemStack, PLANKS};

#[test]
fn max_stack_matches_vanilla_items() {
    assert_eq!(Hotbar::max_stack(IRON_SWORD_INDEX), 1);
    assert_eq!(Hotbar::max_stack(COBBLESTONE), 64);
    assert_eq!(Hotbar::max_stack(mcv_item::BOOK), 16);
    assert_eq!(
        Hotbar::max_stack(mcv_item::ENCHANTED_BOOK),
        1,
        "附魔书不可堆叠(26.1)"
    );
}

#[test]
fn add_prefers_selected_slot_then_merges_then_empty() {
    let mut h = Hotbar::empty();
    // 空栏:进选中槽(非 0 槽)。
    assert!(h.add(3, ItemStack::new(COBBLESTONE, 10)).is_none());
    assert_eq!(h.slots[3].count, 10);
    // 选中槽可合并:60 + 10 → 64 + 6(剩余滚入任意空槽,选中槽优先规则下
    // 第二个空槽按扫描序)。
    h.slots[3].count = 60;
    assert!(h.add(3, ItemStack::new(COBBLESTONE, 10)).is_none());
    assert_eq!(h.slots[3].count, 64);
    assert_eq!(
        h.slots.iter().filter(|s| s.count > 0).count(),
        2,
        "溢出的 6 必须另开一格"
    );
    // 选中槽优先放新物:另一种物品进选中槽(若空)。
    let mut h2 = Hotbar::empty();
    h2.slots[5] = ItemStack::new(PLANKS, 1);
    assert!(h2.add(2, ItemStack::new(COBBLESTONE, 1)).is_none());
    assert_eq!(h2.slots[2].item, COBBLESTONE);
}

#[test]
fn enchanted_or_damaged_stacks_never_merge() {
    let mut h = Hotbar::empty();
    let mut ench = ItemStack::new(COBBLESTONE, 1);
    ench.enchants
        .push(mcv_item::EnchStack { ench_id: 0, level: 1 });
    assert!(h.add(0, ench).is_none());
    assert!(h.add(0, ItemStack::new(COBBLESTONE, 1)).is_none());
    assert!(
        h.slots[1].item == COBBLESTONE,
        "附魔堆与干净堆不得合并"
    );

    let mut h2 = Hotbar::empty();
    let mut dmg = ItemStack::new(IRON_SWORD_INDEX, 1);
    dmg.damage = 10;
    assert!(h2.add(0, dmg).is_none());
    assert!(h2.add(0, ItemStack::new(IRON_SWORD_INDEX, 1)).is_none());
    assert_eq!(
        h2.slots.iter().filter(|s| !s.is_empty()).count(),
        2,
        "损伤工具与满耐久工具各占一格"
    );
}

#[test]
fn full_hotbar_keeps_leftover() {
    let mut h = Hotbar::empty();
    for i in 0..HOTBAR_SLOTS {
        let mut s = ItemStack::new(COBBLESTONE, 64);
        // 每格挂附魔防合并,占满 9 格。
        s.enchants
            .push(mcv_item::EnchStack { ench_id: 0, level: 1 });
        h.slots[i] = s;
    }
    let left = h.add(0, ItemStack::new(PLANKS, 3));
    assert_eq!(left.map(|s| (s.item, s.count)), Some((PLANKS, 3)));
}

#[test]
fn take_one_consumes_and_clears() {
    let mut h = Hotbar::empty();
    h.slots[0] = ItemStack::new(COBBLESTONE, 2);
    assert!(h.take_one(0));
    assert_eq!(h.slots[0].count, 1);
    assert!(h.take_one(0));
    assert!(h.slots[0].is_empty());
    assert!(!h.take_one(0), "空格消耗失败");
}

#[test]
fn block_drops_follow_261_semantics() {
    use mcv_core::BlockId;
    // 石头→圆石(26.1 dropResources)。
    let d = mcv_item::drop_for_block(BlockId(1)).expect("stone 必须掉圆石");
    assert_eq!((d.item, d.count), (COBBLESTONE, 1));
    // 已注册方块各归其位。
    assert_eq!(
        mcv_item::drop_for_block(BlockId(8)).map(|d| d.item),
        Some(PLANKS)
    );
    // 无对应物品的方块不掉落(宁缺勿错)。
    assert!(mcv_item::drop_for_block(BlockId(2)).is_none());
}

#[test]
fn hotbar_kind_roundtrip() {
    let s = ItemStack::new(COBBLESTONE, 1);
    assert!(matches!(s.def().kind, ItemKind::Block(_)));
}
