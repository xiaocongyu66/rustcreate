//! Hotbar:add 的 vanilla 优先级(合并扫描 = 选中槽→全局 0..35 序、
//! 空位 = 全局首空槽 getFreeSlot)、堆叠上限、附魔/耐久不合并、
//! 满栏剩余、take_one。

use mcv_item::inventory::{HOTBAR_SLOTS, Hotbar};
use mcv_item::{COBBLESTONE, IRON_SWORD_INDEX, ItemKind, ItemStack, PLANKS};

#[test]
fn max_stack_matches_vanilla_items() {
    assert_eq!(Hotbar::max_stack(IRON_SWORD_INDEX), 1);
    assert_eq!(Hotbar::max_stack(COBBLESTONE), 64);
    // 书 = 64(Item 默认上限);16 是成书 written_book,不是本书。
    assert_eq!(Hotbar::max_stack(mcv_item::BOOK), 64);
    assert_eq!(
        Hotbar::max_stack(mcv_item::ENCHANTED_BOOK),
        1,
        "附魔书不可堆叠(26.1)"
    );
}

#[test]
fn add_merges_selected_first_then_empty_global_first() {
    // 空栏无同物:进全局首空槽(getFreeSlot 无选中槽优先,INV:102-110)。
    let mut h = Hotbar::empty();
    assert!(h.add(3, ItemStack::new(COBBLESTONE, 10)).is_none());
    assert_eq!(h.slots[0].count, 10);
    assert!(h.slots[3].is_empty());
    // 合并扫描选中槽优先(getSlotWithRemainingSpace 先查 selected,
    // INV:224-227),余量按全局序续填(do-while 多趟,INV:277-285)。
    let mut h = Hotbar::empty();
    h.slots[0] = ItemStack::new(COBBLESTONE, 40);
    h.slots[3] = ItemStack::new(COBBLESTONE, 60);
    assert!(h.add(3, ItemStack::new(COBBLESTONE, 10)).is_none());
    assert_eq!(h.slots[3].count, 64, "选中槽优先填满");
    assert_eq!(h.slots[0].count, 46, "余量 6 续填全局序部分堆");
    // 无处可合并 → 全局首空槽放整堆。
    let mut h2 = Hotbar::empty();
    h2.slots[3] = ItemStack::new(COBBLESTONE, 64);
    assert!(h2.add(3, ItemStack::new(PLANKS, 5)).is_none());
    assert_eq!(h2.slots[0].item, PLANKS);
    // main 段可达(全 36 格序)。
    let mut h3 = Hotbar::empty();
    for s in h3.slots.iter_mut() {
        *s = ItemStack::new(COBBLESTONE, 64);
    }
    assert!(h3.add(0, ItemStack::new(PLANKS, 1)).is_none());
    assert_eq!(h3.main[0].item, PLANKS, "快捷栏满进主背包");
}

#[test]
fn enchanted_or_damaged_stacks_never_merge() {
    let mut h = Hotbar::empty();
    let mut ench = ItemStack::new(COBBLESTONE, 1);
    ench.enchants.push(mcv_item::EnchStack {
        ench_id: 0,
        level: 1,
    });
    assert!(h.add(0, ench).is_none());
    assert!(h.add(0, ItemStack::new(COBBLESTONE, 1)).is_none());
    assert!(h.slots[1].item == COBBLESTONE, "附魔堆与干净堆不得合并");

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
fn full_inventory_keeps_leftover() {
    // 占满全 36 格(新 add 覆盖 main 段,只满快捷栏会落进 main)。
    let mut h = Hotbar::empty();
    let seed = |i: usize, h: &mut Hotbar| {
        let mut s = ItemStack::new(COBBLESTONE, 64);
        // 每格挂附魔防合并。
        s.enchants.push(mcv_item::EnchStack {
            ench_id: 0,
            level: 1,
        });
        *h.slot_mut(i) = s;
    };
    for i in 0..HOTBAR_SLOTS + mcv_item::inventory::MAIN_SLOTS {
        seed(i, &mut h);
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
    let zero = || 0u32;
    // 石头→圆石(26.1 dropResources)。
    let d = mcv_item::drops_for_block(BlockId(1), &mut zero);
    assert_eq!(d.len(), 1);
    assert_eq!((d[0].item, d[0].count), (COBBLESTONE, 1));
    // 已注册方块各归其位。
    let planks = mcv_item::drops_for_block(BlockId(8), &mut zero);
    assert_eq!(planks[0].item, PLANKS);
    // 圆石块注册名是 "cobble"(blocks_gen),必须能掉。
    let cobble = mcv_item::drops_for_block(BlockId(9), &mut zero);
    assert_eq!(cobble[0].item, COBBLESTONE);
    // 泥土掉自身(26.1 dirt.json;全族表见 tests/drops.rs)。
    let dirt = mcv_item::drops_for_block(BlockId(2), &mut zero);
    assert_eq!(dirt[0].item, mcv_item::DIRT_ITEM);
}

#[test]
fn hotbar_kind_roundtrip() {
    let s = ItemStack::new(COBBLESTONE, 1);
    assert!(matches!(s.def().kind, ItemKind::Block(_)));
}

#[test]
fn add_overflow_fills_hotbar_then_main() {
    let mut h = Hotbar::empty();
    // 快捷栏先吸收(每格 64,9 格 = 576;参数 u8 分次塞满)。
    for _ in 0..9 {
        assert_eq!(h.add_overflow(COBBLESTONE, 64), 0);
    }
    assert!(h.slots.iter().all(|s| s.count == 64));
    assert!(h.main.iter().all(|s| s.is_empty()), "快捷栏未满不进 main");
    // 溢出进主背包,合并优先再空位。
    assert_eq!(h.add_overflow(COBBLESTONE, 10), 0);
    assert_eq!(h.main[0].item, COBBLESTONE);
    assert_eq!(h.main[0].count, 10);
    // 同物未满可继续合并进 main[0](不另开格)。
    assert_eq!(h.add_overflow(COBBLESTONE, 20), 0);
    assert_eq!(h.main[0].count, 30);
    assert_eq!(h.main.iter().filter(|s| !s.is_empty()).count(), 1);
}

#[test]
fn add_overflow_respects_max_stack_and_reports_remainder() {
    // 工具 max=1:add_overflow 每次一件,占满 36 格后第 37 件无处降为剩余。
    let mut h = Hotbar::empty();
    for _ in 0..36 {
        assert_eq!(h.add_overflow(IRON_SWORD_INDEX, 1), 0);
    }
    assert_eq!(h.add_overflow(IRON_SWORD_INDEX, 1), 1, "满栏返回剩余");
    // 工具 max=1:两把永远分两格。
    let mut h2 = Hotbar::empty();
    assert_eq!(h2.add_overflow(IRON_SWORD_INDEX, 1), 0);
    assert_eq!(h2.add_overflow(IRON_SWORD_INDEX, 1), 0);
    assert!(h2.slots[0].count == 1 && h2.slots[1].count == 1);
}

#[test]
fn take_last_nonempty_scans_from_back() {
    let mut h = Hotbar::empty();
    h.slots[0] = ItemStack::new(COBBLESTONE, 1);
    h.main[5] = ItemStack::new(PLANKS, 3);
    let last = h.take_last_nonempty().expect("有物");
    assert_eq!((last.item, last.count), (PLANKS, 3), "先取全局尾段(main)");
    assert!(h.main[5].is_empty());
    let second = h.take_last_nonempty().expect("还有");
    assert_eq!(second.item, COBBLESTONE);
    assert!(h.take_last_nonempty().is_none(), "空栏返回 None");
}
