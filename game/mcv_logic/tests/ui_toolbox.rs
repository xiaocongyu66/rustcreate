//! 容器 UI 工具箱波（派单 #152）：shift 快移（QUICK_MOVE）+ 数字键直换
//! （SWAP）在 ui.rs 纯状态机面的语义测试。tooltip 物品名查询的 modern/
//! legacy 两档断言在 game/mcv_app/src/app.rs（i18n 表在 mcv_app，mcv_logic
//! 不依赖它）。
//!
//! 引注约定（26.1 反编译树 /root/mc-ref/src-26.1）：ACM =
//! AbstractContainerMenu.java、CM = CraftingMenu.java、AM =
//! AbstractFurnaceMenu.java、IM = InventoryMenu.java、ACS =
//! AbstractContainerScreen.java、Slot = Slot.java；行号见逐断言注释。

use mcv_item::{HOTBAR_SLOTS, Hotbar, ItemStack};
use mcv_logic::ui::{CraftScreen, FurnaceScreen};

fn stack(item: u16, n: u8) -> ItemStack {
    ItemStack::new(item, n)
}

fn empty_hotbar() -> Hotbar {
    Hotbar::empty()
}

#[cfg(test)]
mod quick_move_craft {
    use super::*;
    use mcv_item::{COBBLESTONE, PLANKS, STICK};

    /// 背包 → 网格：同物并堆（CM:123 moveItemStackTo(1,10,false) 并堆段）。
    #[test]
    fn inv_to_grid_merges_partial() {
        let mut cs = CraftScreen::new(3);
        cs.grid[0] = stack(PLANKS, 5);
        let mut hb = empty_hotbar();
        hb.main[0] = stack(PLANKS, 10); // 全局槽 9
        cs.quick_move_inv(9, &mut hb);
        assert_eq!(cs.grid[0].count, 15, "并堆进部分堆");
        assert!(hb.main[0].is_empty(), "源槽清空");
    }

    /// 背包 → 网格：空格整段放入（ACM:683-689 空位段）。
    #[test]
    fn inv_to_grid_fills_empty_slot() {
        let mut cs = CraftScreen::new(3);
        let mut hb = empty_hotbar();
        hb.slots[0] = stack(COBBLESTONE, 7); // 全局槽 0（热栏段同样先试网格）
        cs.quick_move_inv(0, &mut hb);
        assert_eq!(cs.grid[0].item, COBBLESTONE);
        assert_eq!(cs.grid[0].count, 7);
        assert!(hb.slots[0].is_empty());
    }

    /// 网格满：3x3 先试网格段失败，再按来源段对调——主背包 → 热栏
    /// （CM:124-126 段 37..46）。
    #[test]
    fn inv_to_grid_falls_back_to_hotbar_when_grid_full() {
        let mut cs = CraftScreen::new(3);
        for s in cs.grid.iter_mut() {
            *s = stack(PLANKS, 64);
        }
        let mut hb = empty_hotbar();
        hb.main[0] = stack(PLANKS, 10);
        cs.quick_move_inv(9, &mut hb);
        assert_eq!(hb.slots[0].count, 10, "热栏段承接（CM:125）");
        assert!(hb.main[0].is_empty());
        assert!(cs.grid.iter().all(|s| s.count == 64), "网格满不动");
    }

    /// 网格 → 背包（CM:129 else 段 moveItemStackTo(10,46,false) = 正向：
    /// 并堆段扫全区间 + 空位段只放首个空格）。
    #[test]
    fn grid_to_inv_merges_then_fills_first_empty() {
        let mut cs = CraftScreen::new(3);
        cs.grid[4] = stack(STICK, 10);
        let mut hb = empty_hotbar();
        hb.main[0] = stack(STICK, 60);
        cs.quick_move_grid(4, &mut hb);
        assert!(cs.grid[4].is_empty(), "网格槽清空");
        assert_eq!(hb.main[0].count, 64, "并堆封顶（ACM:657-662）");
        assert_eq!(
            (hb.main[1].item, hb.main[1].count),
            (STICK, 6),
            "余量入首个空格"
        );
        assert!(hb.slots.iter().all(|s| s.is_empty()), "正向主背包先于热栏");
    }

    /// 网格 → 背包：主背包全满 → 热栏承接（菜单序 10..46 尾段）。
    #[test]
    fn grid_to_inv_spills_into_hotbar_when_main_full() {
        let mut cs = CraftScreen::new(3);
        cs.grid[4] = stack(STICK, 10);
        let mut hb = empty_hotbar();
        for s in hb.main.iter_mut() {
            *s = stack(PLANKS, 64);
        }
        cs.quick_move_grid(4, &mut hb);
        assert_eq!((hb.slots[0].item, hb.slots[0].count), (STICK, 10));
    }

    /// 2x2 随身：热栏 → 主背包（IM:133-134 段 9..36），无网格路由
    /// （IM:128-131 装备分支 = 审计 #24 同债）。
    #[test]
    fn two_by_two_hotbar_to_main() {
        let mut cs = CraftScreen::new(2);
        let mut hb = empty_hotbar();
        hb.slots[3] = stack(STICK, 20);
        hb.main[5] = stack(STICK, 30);
        cs.quick_move_inv(3, &mut hb);
        assert_eq!(hb.main[5].count, 50, "并堆进主背包（IM:134）");
        assert!(hb.slots[3].is_empty());
        assert!(cs.grid.iter().all(|s| s.is_empty()), "2x2 无网格路由");
    }

    /// shift+结果槽：整份并入热栏部分堆（CM:117 段 10..46 倒序 = 热栏先）
    /// + 消耗一次网格（ACM:413-427 QUICK_MOVE → ResultSlot.onTake 链）。
    #[test]
    fn result_moves_whole_batch_hotbar_first() {
        let mut cs = CraftScreen::new(3);
        cs.grid[0] = stack(PLANKS, 1);
        cs.grid[3] = stack(PLANKS, 1);
        assert_eq!(cs.result().map(|s| (s.item, s.count)), Some((STICK, 4)));
        let mut hb = empty_hotbar();
        hb.slots[8] = stack(STICK, 60);
        assert!(cs.quick_move_result(&mut hb), "整份放得下");
        assert_eq!(hb.slots[8].count, 64);
        assert!(
            cs.grid[0].is_empty() && cs.grid[3].is_empty(),
            "网格消耗一次"
        );
    }

    /// shift+结果槽满栏：放不下整份 = 原样不动（满则停）。结果槽是重算
    /// 模型，「部分移动 + 消耗网格」会凭空损耗余量——本仓取整份全进才
    /// 消耗的守恒投影（债登记 HANDOFF-UITOOLS.md）。
    #[test]
    fn result_all_or_nothing_when_inventory_full() {
        let mut cs = CraftScreen::new(3);
        cs.grid[0] = stack(PLANKS, 1);
        cs.grid[3] = stack(PLANKS, 1);
        let mut hb = empty_hotbar();
        for s in hb.main.iter_mut().chain(hb.slots.iter_mut()) {
            *s = stack(PLANKS, 64);
        }
        assert!(!cs.quick_move_result(&mut hb), "放不下整份 → 拒绝");
        assert_eq!(cs.grid[0].count, 1, "网格原样");
        assert_eq!(cs.result().map(|s| (s.item, s.count)), Some((STICK, 4)));
        assert!(hb.slots.iter().all(|s| s.item == PLANKS), "背包未动");
    }

    /// 守恒不变式：随机快移序列只搬迁不灭失（并堆/满停/双向混合）。
    #[test]
    fn quick_move_sequence_conserves_items() {
        let mut cs = CraftScreen::new(3);
        cs.grid[0] = stack(PLANKS, 30);
        cs.grid[4] = stack(COBBLESTONE, 17);
        let mut hb = empty_hotbar();
        hb.slots[2] = stack(STICK, 40);
        hb.main[7] = stack(PLANKS, 9);
        hb.main[13] = stack(STICK, 3);
        let seed_items: u32 = 30 + 17 + 40 + 9 + 3;
        let mut lcg = 991u32;
        for step in 0..80 {
            lcg = lcg.wrapping_mul(1664525).wrapping_add(1013904223);
            if step % 2 == 0 {
                let i = ((lcg >> 8) as usize) % (HOTBAR_SLOTS + 27);
                cs.quick_move_inv(i, &mut hb);
            } else {
                let i = ((lcg >> 8) as usize) % 9;
                cs.quick_move_grid(i, &mut hb);
            }
        }
        let count = |hb: &Hotbar, item: u16| -> u32 {
            hb.all_slots()
                .iter()
                .filter(|s| s.item == item)
                .map(|s| s.count as u32)
                .sum::<u32>()
        };
        let total = count(&hb, PLANKS)
            + count(&hb, COBBLESTONE)
            + count(&hb, STICK)
            + cs.grid.iter().map(|s| s.count as u32).sum::<u32>()
            + if cs.cursor.is_empty() {
                0
            } else {
                cs.cursor.count as u32
            };
        assert_eq!(total, seed_items, "快移只搬迁：总数一颗不少");
    }
}

#[cfg(test)]
mod quick_move_furnace {
    use super::*;
    use mcv_item::{COAL, COBBLESTONE, IRON_INGOT, RAW_IRON, STICK, STONE_ITEM};

    /// 主背包可熔物 → 输入槽（AM:104-108 canSmelt → moveItemStackTo(0,1)），
    /// 经 set_input 走 AFBE setItem 面（cook_total 置配方时长、cook 清零）。
    #[test]
    fn inv_smeltable_to_input_resets_cook() {
        let mut f = FurnaceScreen::new();
        f.cook_time = 77; // 非零起点：换品必须清零（AFBE:292-302）
        let mut hb = empty_hotbar();
        hb.main[0] = stack(RAW_IRON, 3);
        f.quick_move_inv(9, &mut hb);
        assert_eq!((f.input.item, f.input.count), (RAW_IRON, 3));
        assert_eq!(f.cook_time, 0, "换品 cook 清零（AFBE:299-301）");
        assert_eq!(
            f.cook_total,
            mcv_item::crafting::find_cooking(RAW_IRON).map_or(0, |r| r.cook_time as u32),
            "cook_total 随配方（AFBE:298）"
        );
        assert!(hb.main[0].is_empty());
    }

    /// 主背包燃料 → 燃料槽（AM:109-112 isFuel → moveItemStackTo(1,2)）；
    /// 燃料优先级低于可熔（同一物品不会同属两类时互斥成立：coal 非熔物）。
    #[test]
    fn inv_fuel_to_fuel_slot() {
        let mut f = FurnaceScreen::new();
        let mut hb = empty_hotbar();
        hb.main[0] = stack(COAL, 5);
        f.quick_move_inv(9, &mut hb);
        assert_eq!((f.fuel.item, f.fuel.count), (COAL, 5));
        assert!(f.input.is_empty(), "可熔分支不劫燃料（canSmelt 先判）");
    }

    /// 既非可熔也非燃料：热栏 → 主背包（AM:117-119 段 3..30）。
    #[test]
    fn inv_inert_hotbar_to_main() {
        let mut f = FurnaceScreen::new();
        let mut hb = empty_hotbar();
        hb.slots[0] = stack(IRON_INGOT, 7);
        f.quick_move_inv(0, &mut hb);
        assert_eq!((hb.main[0].item, hb.main[0].count), (IRON_INGOT, 7));
        assert!(hb.slots[0].is_empty());
    }

    /// 可熔物且输入槽已满同品：只试输入段失败即止（AM:107-108 return，
    /// 无背包对调回退——与 CM 的先网格后对调两段式不同构）。
    #[test]
    fn inv_smeltable_full_input_stops() {
        let mut f = FurnaceScreen::new();
        f.input = stack(RAW_IRON, 64);
        let mut hb = empty_hotbar();
        hb.main[0] = stack(RAW_IRON, 5);
        f.quick_move_inv(9, &mut hb);
        assert_eq!(f.input.count, 64, "输入槽满不动");
        assert_eq!(hb.main[0].count, 5, "无回退：主背包原样");
    }

    /// 输入槽 → 背包（AM:120-122 段 3..39 正向 = 主背包先）；出料经
    /// set_input（清空 = 品变，cook_total/timer 重置，AFBE setItem 面）。
    #[test]
    fn slot_input_to_inventory_forward() {
        let mut f = FurnaceScreen::new();
        f.input = stack(RAW_IRON, 5);
        f.cook_time = 30;
        let mut hb = empty_hotbar();
        hb.slots[0] = stack(STICK, 1); // 热栏非空：正向仍须主背包先
        f.quick_move_slot(0, &mut hb);
        assert!(f.input.is_empty());
        assert_eq!((hb.main[0].item, hb.main[0].count), (RAW_IRON, 5));
        assert_eq!(hb.slots[0].count, 1, "热栏未被扰动");
        assert_eq!(f.cook_time, 0, "清输入 = setItem(EMPTY) 面，cook 归零");
    }

    /// 结果槽 → 背包倒序（AM:99 段 3..39 backwards = 热栏先）：并堆段扫
    /// 全区间，热栏部分堆先填满、主背包部分堆随后（ACM:646-670）。
    #[test]
    fn result_to_inventory_backward_hotbar_first() {
        let mut f = FurnaceScreen::new();
        f.result = stack(STONE_ITEM, 5);
        let mut hb = empty_hotbar();
        hb.main[0] = stack(STONE_ITEM, 60);
        hb.slots[2] = stack(STONE_ITEM, 60);
        f.quick_move_result(&mut hb);
        assert!(f.result.is_empty(), "物理结果槽：全部并堆后清空");
        assert_eq!(hb.slots[2].count, 64, "热栏先");
        assert_eq!(hb.main[0].count, 61, "主背包随后补 1");
    }

    /// 结果槽部分移动：放不下的余量留槽（满则停，ACM:124-128）。背包
    /// 全满（并堆段无同品余量、空位段无空格）才会触发部分移动。
    #[test]
    fn result_partial_move_keeps_remainder() {
        let mut f = FurnaceScreen::new();
        f.result = stack(STONE_ITEM, 10);
        let mut hb = empty_hotbar();
        hb.slots[2] = stack(STONE_ITEM, 60);
        for s in hb.main.iter_mut().chain(hb.slots.iter_mut()) {
            if s.is_empty() {
                *s = stack(COBBLESTONE, 64);
            }
        }
        f.quick_move_result(&mut hb);
        assert_eq!(f.result.count, 6, "余量留结果槽");
        assert_eq!(hb.slots[2].count, 64);
        assert!(hb.main.iter().all(|s| s.item == COBBLESTONE), "主背包未动");
        assert!(
            hb.slots
                .iter()
                .all(|s| s.item == COBBLESTONE || s.item == STONE_ITEM),
            "热栏无额外空位段落地"
        );
    }

    /// 数字键直换（ACM:471-507 SWAP）：燃料槽 mayPlace=isFuel 门——非燃料
    /// 源对燃料槽 = 无操作（ACM:484/493 mayPlace 门）。
    #[test]
    fn swap_fuel_slot_gates_non_fuel_source() {
        let mut f = FurnaceScreen::new();
        let mut hb = empty_hotbar();
        hb.slots[1] = stack(IRON_INGOT, 3);
        assert!(f.hotbar_swap(1, &mut hb, 1).is_none());
        assert!(f.fuel.is_empty(), "非燃料源不得入燃料槽");
        assert_eq!(hb.slots[1].count, 3, "源槽原样");
        hb.slots[1] = stack(COAL, 3);
        assert!(f.hotbar_swap(1, &mut hb, 1).is_none());
        assert_eq!((f.fuel.item, f.fuel.count), (COAL, 3), "燃料源可入");
        assert!(hb.slots[1].is_empty());
    }

    /// 数字键直换·输入槽：异物互换（ACM:502-505 整堆互换分支）。
    #[test]
    fn swap_input_slot_swaps_stacks() {
        let mut f = FurnaceScreen::new();
        f.input = stack(RAW_IRON, 2);
        let mut hb = empty_hotbar();
        hb.slots[6] = stack(COAL, 4);
        assert!(f.hotbar_swap(0, &mut hb, 6).is_none(), "互换无落地余量");
        assert_eq!((f.input.item, f.input.count), (COAL, 4));
        assert_eq!((hb.slots[6].item, hb.slots[6].count), (RAW_IRON, 2));
    }

    /// 数字键直换·结果槽：mayPlace=false（FurnaceResultSlot.java:24-27）→
    /// 只允许「空热栏槽取件」单向（ACM:476-482，Slot.mayPickup 默认真）。
    #[test]
    fn swap_result_takes_only_to_empty_hotbar_slot() {
        let mut f = FurnaceScreen::new();
        f.result = stack(STONE_ITEM, 2);
        let mut hb = empty_hotbar();
        hb.slots[3] = stack(STICK, 1);
        assert!(f.hotbar_swap(2, &mut hb, 3).is_none());
        assert_eq!(f.result.count, 2, "热栏槽非空 = 回放被 mayPlace 挡");
        assert_eq!(hb.slots[3].count, 1);
        hb.slots[3] = ItemStack::empty();
        assert!(f.hotbar_swap(2, &mut hb, 3).is_none());
        assert!(f.result.is_empty(), "空热栏槽取件");
        assert_eq!((hb.slots[3].item, hb.slots[3].count), (STONE_ITEM, 2));
    }
}

#[cfg(test)]
mod hotbar_swap {
    use super::*;
    use mcv_item::{BOW, COBBLESTONE, PLANKS, STICK};

    /// 网格空槽 + 快捷栏有物：整体移入（ACM:483-492 target 空分支）。
    #[test]
    fn swaps_empty_grid_slot_with_hotbar() {
        let mut cs = CraftScreen::new(3);
        let mut hb = empty_hotbar();
        hb.slots[4] = stack(PLANKS, 12);
        assert!(cs.hotbar_swap(2, &mut hb, 4).is_none());
        assert_eq!((cs.grid[2].item, cs.grid[2].count), (PLANKS, 12));
        assert!(hb.slots[4].is_empty());
    }

    /// 双方有物：整堆互换（ACM:502-505；本仓注册表堆叠上限恒 ≥ 源堆量，
    /// 劈堆分支（ACM:495-500）实际不可达，守恒由互换分支覆盖）。
    #[test]
    fn swaps_full_stacks_both_nonempty() {
        let mut cs = CraftScreen::new(3);
        cs.grid[2] = stack(COBBLESTONE, 3);
        let mut hb = empty_hotbar();
        hb.slots[4] = stack(PLANKS, 7);
        assert!(cs.hotbar_swap(2, &mut hb, 4).is_none());
        assert_eq!((cs.grid[2].item, cs.grid[2].count), (PLANKS, 7));
        assert_eq!((hb.slots[4].item, hb.slots[4].count), (COBBLESTONE, 3));
    }

    /// 悬停热栏槽 i 按数字键 k：热栏内两槽对调（ACM SWAP 语义同构）。
    #[test]
    fn swaps_two_hotbar_slots() {
        let mut hb = empty_hotbar();
        hb.slots[0] = stack(PLANKS, 5);
        hb.slots[3] = stack(STICK, 7);
        assert!(mcv_logic::ui::swap_inv_with_hotbar(&mut hb, 0, 3).is_none());
        assert_eq!((hb.slots[0].item, hb.slots[0].count), (STICK, 7));
        assert_eq!((hb.slots[3].item, hb.slots[3].count), (PLANKS, 5));
    }

    /// 主背包槽 ↔ 快捷栏（全局 9..35 段）。
    #[test]
    fn swaps_main_slot_with_hotbar() {
        let mut hb = empty_hotbar();
        hb.main[2] = stack(BOW, 1);
        hb.slots[5] = stack(PLANKS, 3);
        assert!(mcv_logic::ui::swap_inv_with_hotbar(&mut hb, 9 + 2, 5).is_none());
        assert_eq!((hb.slots[5].item, hb.slots[5].count), (BOW, 1));
        assert_eq!((hb.main[2].item, hb.main[2].count), (PLANKS, 3));
    }

    /// i == k（悬停热栏槽按自己的数字键）= 原地无操作。
    #[test]
    fn same_slot_is_noop() {
        let mut hb = empty_hotbar();
        hb.slots[2] = stack(PLANKS, 5);
        assert!(mcv_logic::ui::swap_inv_with_hotbar(&mut hb, 2, 2).is_none());
        assert_eq!((hb.slots[2].item, hb.slots[2].count), (PLANKS, 5));
    }

    /// 合成结果槽：空热栏槽整份取件 + 消耗一次网格（ResultSlot mayPickup
    /// 链）；热栏槽有物 = no-op（mayPlace=false 挡回放）。
    #[test]
    fn craft_result_swap_takes_to_empty_slot_only() {
        let mut cs = CraftScreen::new(2);
        cs.grid[0] = stack(PLANKS, 1);
        cs.grid[2] = stack(PLANKS, 1);
        let mut hb = empty_hotbar();
        hb.slots[6] = stack(PLANKS, 2);
        assert!(!cs.hotbar_swap_result(&mut hb, 6), "热栏槽有物 → 拒绝");
        assert_eq!(cs.result().map(|s| s.count), Some(4), "结果原样");
        hb.slots[6] = ItemStack::empty();
        assert!(cs.hotbar_swap_result(&mut hb, 6));
        assert_eq!((hb.slots[6].item, hb.slots[6].count), (STICK, 4));
        assert!(
            cs.grid[0].is_empty() && cs.grid[2].is_empty(),
            "网格消耗一次"
        );
    }

    /// 越界防御：槽号/键号越界一律无操作。
    #[test]
    fn out_of_bounds_is_noop() {
        let mut cs = CraftScreen::new(2);
        let mut hb = empty_hotbar();
        assert!(cs.hotbar_swap(4, &mut hb, 0).is_none()); // 2x2 网格只有 0..4
        assert!(mcv_logic::ui::swap_inv_with_hotbar(&mut hb, 36, 0).is_none());
        assert!(mcv_logic::ui::swap_inv_with_hotbar(&mut hb, 0, 9).is_none());
        assert!(cs.grid.iter().all(|s| s.is_empty()));
        assert!(hb.all_slots().iter().all(|s| s.is_empty()));
    }
}
