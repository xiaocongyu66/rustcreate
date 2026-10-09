//! 合成/创造界面的纯状态机(无 wgpu、可无头测试)。
//! [`CraftScreen`] 是 2x2(玩家随身)/3x3(工作台)合成网格 + 光标手持物,
//! 左键交换/合并、右键拆分/单放,vanilla-ish 语义(无 shift-click、无拖动)。
//! [`CreativePicker`] 是创造模式物品选择页。
//! 物品守恒是硬约束:`close()` 必须归还网格 + 光标的全部物品。

use mcv_item::crafting::{self, EMPTY_SLOT};
use mcv_item::{HOTBAR_SLOTS, Hotbar, ItemStack, MAIN_SLOTS};

/// 合成界面状态。`width` = 2(随身)或 3(工作台),grid 行优先 w×w。
#[derive(Clone, Debug)]
pub struct CraftScreen {
    pub width: usize,
    pub grid: Vec<ItemStack>,
    /// 光标手持堆(鼠标抓着走)。
    pub cursor: ItemStack,
}

/// 两堆可合并(同物、同耐久、双方无附魔、上限 >1;同 Hotbar::mergeable 判据)。
fn can_merge(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item
        && a.damage == b.damage
        && a.enchants.is_empty()
        && b.enchants.is_empty()
        && Hotbar::max_stack(a.item) > 1
}

/// 单槽交互(网格槽与背包槽共用),`cursor` 为光标手持物。
/// 左键:光标↔槽交换;同物可堆时先整段填入(封顶 max_stack,余量留光标)。
/// 右键:光标空 → 槽取一半(vanilla 上取整);光标有物 → 往槽放 1。
fn click_slot(slot: &mut ItemStack, cursor: &mut ItemStack, left: bool) {
    if left {
        if cursor.is_empty() && slot.is_empty() {
            return;
        }
        if !cursor.is_empty()
            && !slot.is_empty()
            && can_merge(cursor, slot)
            && slot.count < Hotbar::max_stack(slot.item)
        {
            // 整段填进部分堆:移 min(cursor, 槽余量)。
            let max = Hotbar::max_stack(slot.item);
            let mv = cursor.count.min(max - slot.count);
            slot.count += mv;
            cursor.count -= mv;
            if cursor.count == 0 {
                *cursor = ItemStack::empty();
            }
            return;
        }
        std::mem::swap(cursor, slot);
        return;
    }
    // 右键
    if cursor.is_empty() {
        if slot.is_empty() {
            return;
        }
        // 取一半(奇数上取整,vanilla pickUp 语义)。
        let take = slot.count.div_ceil(2);
        let mut c = slot.clone();
        c.count = take;
        slot.count -= take;
        if slot.count == 0 {
            *slot = ItemStack::empty();
        }
        *cursor = c;
    } else {
        let max = Hotbar::max_stack(cursor.item);
        if slot.is_empty() {
            let mut one = cursor.clone();
            one.count = 1;
            *slot = one;
            cursor.count -= 1;
            if cursor.count == 0 {
                *cursor = ItemStack::empty();
            }
        } else if can_merge(cursor, slot) && slot.count < max {
            slot.count += 1;
            cursor.count -= 1;
            if cursor.count == 0 {
                *cursor = ItemStack::empty();
            }
        } else if slot.item != cursor.item {
            // 右键异物槽 = 交换（ACM:451-458 SECONDARY 落入同一 swap 分支；
            // 数量上限守卫在现注册表下恒真，与左键同）。
            std::mem::swap(cursor, slot);
        }
    }
}

impl CraftScreen {
    /// `width` 非 2/3 时钳到 2(防御式,UI 层只会传 2 或 3)。
    pub fn new(width: usize) -> Self {
        let width = if width == 3 { 3 } else { 2 };
        Self {
            width,
            grid: vec![ItemStack::empty(); width * width],
            cursor: ItemStack::empty(),
        }
    }

    /// 合成引擎视角的格子(物品 id,空格 = EMPTY_SLOT)。
    fn ids(&self) -> Vec<u16> {
        self.grid
            .iter()
            .map(|s| if s.count > 0 { s.item } else { EMPTY_SLOT })
            .collect()
    }

    /// 当前网格的合成结果(None = 无配方)。
    pub fn result(&self) -> Option<ItemStack> {
        crafting::find_result(&self.ids(), self.width)
            .map(|(item, count)| ItemStack::new(item, count))
    }

    /// 点击网格槽(左/右键语义见 [`click_slot`])。
    pub fn click_grid(&mut self, i: usize, left: bool) {
        if i >= self.grid.len() {
            return;
        }
        click_slot(&mut self.grid[i], &mut self.cursor, left);
    }

    /// 点击背包全局槽(0..8 快捷栏、9..35 主背包,`Hotbar::slot_mut` 同序;
    /// 左/右键语义见 [`click_slot`])。越界无操作。
    pub fn click_inv(&mut self, i: usize, left: bool, hb: &mut Hotbar) {
        if i >= HOTBAR_SLOTS + MAIN_SLOTS {
            return;
        }
        click_slot(hb.slot_mut(i), &mut self.cursor, left);
    }

    /// 从结果槽取物(ACM:438-465 + ResultSlot.onTake)。`right` = true 右键取
    /// ceil(半)(ACM:445);同物光标装不下时取部分(min(所需,光标余量),
    /// Slot.tryRemove 语义,ACM:459-465)。成功则网格各占用格 −1(整体消耗
    /// 一次,半取同样消耗)。
    pub fn take_result(&mut self, right: bool) -> bool {
        let Some(res) = self.result() else {
            return false;
        };
        let want = if right {
            res.count.div_ceil(2)
        } else {
            res.count
        };
        let take = if self.cursor.is_empty() {
            want
        } else if self.cursor.item == res.item
            && self.cursor.damage == 0
            && self.cursor.enchants.is_empty()
        {
            let room = Hotbar::max_stack(res.item) - self.cursor.count;
            want.min(room)
        } else {
            return false;
        };
        if take == 0 {
            return false;
        }
        crafting::consume_grid(&mut self.grid);
        if self.cursor.is_empty() {
            let mut c = res;
            c.count = take;
            self.cursor = c;
        } else {
            self.cursor.count += take;
        }
        true
    }

    /// 关界面:返回光标的全部物品放最前、再网格(vanilla 关容器先归还 carried
    /// 再逐格网格,ACM:586-594 + CM:98-101)。调用方负责塞回背包/掉落。
    pub fn close(self) -> Vec<ItemStack> {
        let mut out = Vec::new();
        if !self.cursor.is_empty() {
            out.push(self.cursor);
        }
        out.extend(self.grid.into_iter().filter(|s| !s.is_empty()));
        out
    }
}

/// 创造模式物品选择器:36 个一页,点击返回要发的 (物品, 数量)。
#[derive(Clone, Debug, Default)]
pub struct CreativePicker {
    pub page: usize,
}

impl CreativePicker {
    /// 每页格数(vanilla CreativeModeInventory 一页 9×4 物品格)。
    pub const PER_PAGE: usize = 36;

    pub fn new() -> Self {
        Self::default()
    }

    /// 总页数(list 为空也算 1 页,避免除零)。
    pub fn pages(&self, list_len: usize) -> usize {
        list_len.div_ceil(Self::PER_PAGE).max(1)
    }

    /// 当前页物品切片。
    pub fn page_items<'a>(&self, list: &'a [u16]) -> &'a [u16] {
        let start = self.page * Self::PER_PAGE;
        &list[start.min(list.len())..(start + Self::PER_PAGE).min(list.len())]
    }

    /// 翻页(钳到 [0, pages))。
    pub fn set_page(&mut self, p: usize, list_len: usize) {
        self.page = p.min(self.pages(list_len) - 1);
    }

    /// 点当前页第 i 格:左键 = 一组(max_stack),右键 = 1。越界 None。
    pub fn click(&self, i: usize, left: bool, list: &[u16]) -> Option<(u16, u8)> {
        let items = self.page_items(list);
        let &item = items.get(i)?;
        let count = if left { Hotbar::max_stack(item) } else { 1 };
        Some((item, count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcv_item::{COBBLESTONE, PLANKS, STICK, WOODEN_PICKAXE_INDEX, WOODEN_SWORD_INDEX};

    fn stack(item: u16, n: u8) -> ItemStack {
        ItemStack::new(item, n)
    }

    #[test]
    fn left_click_swaps_and_merges_partial() {
        let mut cs = CraftScreen::new(2);
        cs.cursor = stack(COBBLESTONE, 30);
        // 空格:直接交换(整体入槽)。
        cs.click_grid(0, true);
        assert!(cs.cursor.is_empty());
        assert_eq!(cs.grid[0].count, 30);
        // 同物部分堆:整段填,封顶 64,余量留光标(vanilla:30+50→64/16)。
        cs.cursor = stack(COBBLESTONE, 50);
        cs.click_grid(0, true);
        assert_eq!(cs.grid[0].count, 64);
        assert_eq!(cs.cursor.count, 16);
        // 满 64 后左键变纯交换。
        cs.grid[0].count = 64;
        cs.cursor = stack(PLANKS, 5);
        cs.click_grid(0, true);
        assert_eq!(cs.grid[0].item, PLANKS);
        assert_eq!(cs.cursor.count, 64);
        assert_eq!(cs.cursor.item, COBBLESTONE);
    }

    #[test]
    fn right_click_splits_and_places_one() {
        let mut cs = CraftScreen::new(2);
        cs.grid[0] = stack(COBBLESTONE, 5);
        cs.click_grid(0, false);
        assert_eq!(cs.cursor.count, 3, "奇数上取整");
        assert_eq!(cs.grid[0].count, 2);
        // 右键往空格放 1。
        cs.cursor = stack(COBBLESTONE, 9);
        cs.click_grid(3, false);
        assert_eq!(cs.grid[3].count, 1);
        assert_eq!(cs.cursor.count, 8);
        // 右键往同物部分堆加 1。
        cs.click_grid(3, false);
        assert_eq!(cs.grid[3].count, 2);
        // 右键异物槽 = 交换（ACM SECONDARY 与左键同落 swap 分支）。
        cs.cursor = stack(PLANKS, 1);
        cs.click_grid(3, false);
        assert_eq!(cs.cursor.item, COBBLESTONE);
        assert_eq!(cs.grid[3].item, PLANKS);
    }

    #[test]
    fn click_inv_swaps_merges_and_bounds() {
        let mut cs = CraftScreen::new(2);
        let mut hb = Hotbar::empty();
        // 左键:光标↔快捷栏槽交换。
        hb.slots[0] = stack(COBBLESTONE, 10);
        cs.cursor = stack(PLANKS, 3);
        cs.click_inv(0, true, &mut hb);
        assert_eq!(cs.cursor.item, COBBLESTONE);
        assert_eq!(hb.slots[0].item, PLANKS);
        // main 段(全局索引 9+)可达:右键往同物部分堆 +1。
        hb.main[0] = stack(STICK, 40);
        cs.cursor = stack(STICK, 30);
        cs.click_inv(9, false, &mut hb);
        assert_eq!(hb.main[0].count, 41);
        assert_eq!(cs.cursor.count, 29);
        // 越界无操作。
        cs.click_inv(36, true, &mut hb);
        assert_eq!(cs.cursor.count, 29);
        // 守恒不变式:网格 + 光标 + 36 背包同物计数。
        let total = |hb: &Hotbar| -> u32 {
            hb.all_slots()
                .iter()
                .filter(|s| s.item == STICK)
                .map(|s| s.count as u32)
                .sum::<u32>()
                + if cs.cursor.item == STICK {
                    cs.cursor.count as u32
                } else {
                    0
                }
                + cs.grid
                    .iter()
                    .filter(|s| s.item == STICK)
                    .map(|s| s.count as u32)
                    .sum::<u32>()
        };
        assert_eq!(total(&hb), 70, "70 根木棍一颗不少");
    }

    #[test]
    fn sword_requires_workbench_vertical_pattern() {
        // 26.1 剑 = 材料×2 + 棍 竖排 1x3：2x2 放不下（shrink 后仍 3 高）。
        let mut cs2 = CraftScreen::new(2);
        cs2.grid[0] = stack(PLANKS, 1);
        cs2.grid[2] = stack(PLANKS, 1);
        cs2.grid[3] = stack(STICK, 1);
        assert!(cs2.result().is_none(), "2x2 不可出剑");
        // 3x3 左列 M/M/S → 木剑。
        let mut cs3 = CraftScreen::new(3);
        cs3.grid[0] = stack(PLANKS, 1);
        cs3.grid[3] = stack(PLANKS, 1);
        cs3.grid[6] = stack(STICK, 1);
        let res = cs3.result().expect("木剑配方");
        assert_eq!((res.item, res.count), (WOODEN_SWORD_INDEX, 1));
    }

    #[test]
    fn craft_consumes_grid_and_cursor_guards() {
        // 木剑 3x3 竖排；多放的 1 板合成后应剩（网格全体占用格各 −1）。
        let mut cs = CraftScreen::new(3);
        cs.grid[0] = stack(PLANKS, 2); // 多出的 1 板
        cs.grid[3] = stack(PLANKS, 1);
        cs.grid[6] = stack(STICK, 1);
        // 光标是异物 → 拒绝取。
        cs.cursor = stack(COBBLESTONE, 10);
        assert!(!cs.take_result(false));
        // 光标空 → 取出，网格各占位消耗 1。
        cs.cursor = ItemStack::empty();
        assert!(cs.take_result(false));
        assert_eq!(cs.cursor.item, WOODEN_SWORD_INDEX);
        assert_eq!(cs.grid[0].count, 1, "同格剩余保留");
        assert!(cs.grid[3].is_empty() && cs.grid[6].is_empty());
        // 消耗后已无配方。
        assert!(cs.result().is_none());
    }

    #[test]
    fn result_half_take_and_partial_fill() {
        // 木棍配方：两板竖排 → 4 根（2x2 可放）。
        let mut cs = CraftScreen::new(2);
        cs.grid[0] = stack(PLANKS, 1);
        cs.grid[2] = stack(PLANKS, 1);
        assert_eq!(cs.result().map(|s| (s.item, s.count)), Some((STICK, 4)));
        // 右键取 ceil(4/2)=2，网格同样整体消耗一次。
        assert!(cs.take_result(true));
        assert_eq!(cs.cursor.count, 2);
        assert!(cs.grid.iter().all(|s| s.is_empty()), "半取也消耗整份网格");
        // 部分取：光标 63 根 + 结果 4 → 只取 1 补满 64（tryRemove 收敛）。
        cs.cursor = stack(STICK, 63);
        cs.grid[0] = stack(PLANKS, 1);
        cs.grid[2] = stack(PLANKS, 1);
        assert!(cs.take_result(false));
        assert_eq!(cs.cursor.count, 64);
        // 光标同物已满 → 拒绝（take=0）。
        cs.grid[0] = stack(PLANKS, 1);
        cs.grid[2] = stack(PLANKS, 1);
        assert!(!cs.take_result(false));
    }

    #[test]
    fn pickaxe_needs_3x3_center_shaft() {
        // 镐 MMM/_S_/_S_ 在 2x2 里放不下 → 无结果。
        let mut cs2 = CraftScreen::new(2);
        for s in cs2.grid.iter_mut() {
            *s = stack(PLANKS, 1);
        }
        assert!(cs2.result().is_none(), "2x2 满板不是镐配方");
        // 3x3 中列棍 → 木镐。
        let mut cs3 = CraftScreen::new(3);
        for x in 0..3 {
            cs3.grid[x] = stack(PLANKS, 1);
        }
        cs3.grid[3 + 1] = stack(STICK, 1);
        cs3.grid[6 + 1] = stack(STICK, 1);
        let res = cs3.result().expect("镐配方");
        assert_eq!(res.item, WOODEN_PICKAXE_INDEX);
        // 右列棍裁剪后仍偏右,与原版 shrink 语义一致 → 不匹配。
        let mut cs4 = CraftScreen::new(3);
        for x in 0..3 {
            cs4.grid[x] = stack(PLANKS, 1);
        }
        cs4.grid[3 + 2] = stack(STICK, 1);
        cs4.grid[6 + 2] = stack(STICK, 1);
        assert!(cs4.result().is_none());
    }

    #[test]
    fn close_returns_every_item() {
        // 守恒:纯移动(不合成)怎么点,close() 归还的总数必须与放入相等。
        let mut cs = CraftScreen::new(3);
        let seed_items = [stack(PLANKS, 64), stack(COBBLESTONE, 37), stack(STICK, 50)];
        let in_total: u32 = seed_items.iter().map(|s| s.count as u32).sum();
        for (i, s) in seed_items.into_iter().enumerate() {
            cs.grid[i] = s;
        }
        // 伪随机乱点(拆分/交换/单放交替)。
        let mut lcg = 7u32;
        for step in 0..60 {
            lcg = lcg.wrapping_mul(1664525).wrapping_add(1013904223);
            let slot = (lcg >> 8) as usize % 9;
            let left = ((lcg >> 4) & 1) == 0;
            cs.click_grid(slot, left);
            if step % 7 == 0 && !cs.cursor.is_empty() {
                // 光标有物时偶尔换个格,避免长期占用。
                cs.click_grid((slot + 5) % 9, true);
            }
        }
        let back = cs.close();
        let out_total: u32 = back.iter().map(|s| s.count as u32).sum();
        assert_eq!(out_total, in_total, "close 必须归还全部物品");
        // 且不许串种:close 里的物品只能来自放入的三种。
        assert!(
            back.iter()
                .all(|s| s.item == PLANKS || s.item == COBBLESTONE || s.item == STICK)
        );
    }

    #[test]
    fn creative_give_counts_and_page_bounds() {
        // 真物品 id 取模复用(max_stack 查 ITEMS，越界会 panic)。
        let list: Vec<u16> = (0..80u16).map(|i| i % 36).collect();
        let cp = CreativePicker::new();
        assert_eq!(cp.pages(list.len()), 3);
        // 末页大小。
        let mut last = CreativePicker::default();
        last.set_page(2, list.len());
        assert_eq!(last.page_items(&list).len(), 8);
        // 越界页钳制 + 越界格 None。
        last.set_page(99, list.len());
        assert_eq!(last.page, 2);
        assert!(last.click(8, true, &list).is_none());
        assert!(last.click(35, true, &list).is_none());
        assert_eq!(last.click(0, true, &list), Some((0, 64)));
        assert_eq!(last.click(0, false, &list), Some((0, 1)));
        // 空列表:1 页,全 None。
        assert_eq!(cp.pages(0), 1);
        assert!(cp.click(0, true, &[]).is_none());
    }
}
