//! 合成/创造界面的纯状态机(无 wgpu、可无头测试)。
//! [`CraftScreen`] 是 2x2(玩家随身)/3x3(工作台)合成网格 + 光标手持物,
//! 现作为网格/光标存储与配方重算;线上交互(左右键/shift/双击/三段
//! 拖拽/Q 丢弃/面板外丢弃)由 engine/mcv_ui 的 ContainerScreen
//! (AbstractContainerMenu.doClick 高保真移植)裁决,app 层喂事件、每次
//! 交互后回写本网格/光标/背包(click_slot 系语义保留作兼容/回退路径)。
//! [`CreativePicker`] 是创造模式物品选择页。[`FurnaceScreen`] 是熔炉屏
//! 状态机(输入/燃料/产物槽 + 26.1 AbstractFurnaceBlockEntity.serverTick
//! 直译的烧炼 tick,见该结构体文档)。
//! 容器工具箱(#152):shift+点击快移(QUICK_MOVE,ACM:413-427 →
//! CM/AM/IM quickMoveStack + ACM:638-700 moveItemStackTo 直译的
//! [`move_stack_to`])与数字键 1-9 直换(SWAP,ACM:471-507,
//! [`swap_with_hotbar`] 族)在 CraftScreen/FurnaceScreen 两屏同构接入。
//! 物品守恒是硬约束:关屏归还 carried(先背包后 drop 余量,ACM:588-596)
//! + 网格(**掉世界**,CM:96-99),见 `close_to_world` + [`drop_container_items`]。

use crate::game::GameRuntime;
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
        if !cursor.is_empty() && !slot.is_empty() && can_merge(cursor, slot) {
            let max = Hotbar::max_stack(slot.item);
            if slot.count >= max {
                // 同类满槽 = 原地无操作(vanilla isSameItemSameComponents →
                // safeInsert(cap=0),ACM:452-454):绝不把满堆"钓"进光标。
                return;
            }
            // 整段填进部分堆:移 min(cursor, 槽余量)。
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

/// moveItemStackTo（ACM:638-700）在槽引用序列上的直译：先同物并堆（封顶
/// max_stack，ACM:645-671），再首个空格整段放入（ACM:673-696，vanilla 空位
/// 段放一处即 break，ACM:688）；`backwards` = 序列从尾端向前（原版倒序
/// 区间）。返回是否移动了任何数量。mayPlace 准入门在调用方（本函数面向
/// 恒 mayPlace 的网格/背包/单槽段；燃料/结果槽的准入先经 quick_move/
/// SWAP 路由筛选）。
fn move_stack_to(st: &mut ItemStack, slots: &mut [&mut ItemStack], backwards: bool) -> bool {
    let seq: Vec<usize> = if backwards {
        (0..slots.len()).rev().collect()
    } else {
        (0..slots.len()).collect()
    };
    let mut changed = false;
    // 并堆段（isStackable + isSameItemSameComponents = can_merge 投影）。
    for &i in &seq {
        if st.is_empty() {
            break;
        }
        if slots[i].is_empty() || !can_merge(st, slots[i]) {
            continue;
        }
        let max = Hotbar::max_stack(slots[i].item);
        let total = slots[i].count + st.count;
        if total <= max {
            slots[i].count = total;
            *st = ItemStack::empty();
            changed = true;
        } else if slots[i].count < max {
            let mv = max - slots[i].count;
            slots[i].count = max;
            st.count -= mv;
            changed = true;
        }
    }
    // 空位段：首个空格放 min(余量, 上限)（ACM:683-689）。
    if !st.is_empty() {
        for &i in &seq {
            if slots[i].is_empty() {
                let max = Hotbar::max_stack(st.item);
                let mv = st.count.min(max);
                let mut placed = st.clone();
                placed.count = mv;
                *slots[i] = placed;
                st.count -= mv;
                if st.count == 0 {
                    *st = ItemStack::empty();
                }
                changed = true;
                break;
            }
        }
    }
    changed
}

/// 数字键直换核心（26.1 AbstractContainerMenu SWAP，ACM:471-507）：悬停槽
/// `target` ↔ 快捷栏槽 `k`（0..8）。源空 = 悬停物整体入栏（ACM:476-482，
/// mayPickup 调用方判）；目标空 = 超上限劈堆进上限、余量留快捷栏，否则
/// 整移（ACM:483-492）；双非空 = 整堆互换（ACM:502-505）/ 劈堆换——被换下
/// 物走 `Hotbar::add`（Inventory.add 投影），放不下的余量返回调用方落地
/// （ACM:498-500；本仓堆叠上限 64/1 下实际不可达，语义留全）。
pub fn swap_with_hotbar(target: &mut ItemStack, hb: &mut Hotbar, k: usize) -> Option<ItemStack> {
    if k >= HOTBAR_SLOTS {
        return None;
    }
    if hb.slots[k].is_empty() {
        if target.is_empty() {
            return None; // 双空（ACM:475 门）。
        }
        std::mem::swap(&mut hb.slots[k], target);
        return None;
    }
    if target.is_empty() {
        let max = Hotbar::max_stack(hb.slots[k].item);
        if hb.slots[k].count > max {
            let mut head = hb.slots[k].clone();
            head.count = max;
            hb.slots[k].count -= max;
            *target = head;
        } else {
            std::mem::swap(&mut hb.slots[k], target);
        }
        return None;
    }
    let max = Hotbar::max_stack(hb.slots[k].item);
    if hb.slots[k].count > max {
        let displaced = std::mem::replace(target, ItemStack::empty());
        let mut head = hb.slots[k].clone();
        head.count = max;
        hb.slots[k].count -= max;
        *target = head;
        hb.add(k, displaced)
    } else {
        std::mem::swap(&mut hb.slots[k], target);
        None
    }
}

/// 数字键直换·背包槽（ACM SWAP 的 `inventory.getItem(buttonNum)` 侧）：
/// 全局槽 `i`（0..8 快捷栏 / 9..35 主背包，[`Hotbar::slot_mut`] 同序）↔
/// 快捷栏槽 `k`。i == k = 原地无操作（vanilla 同槽 set 回自身）。
pub fn swap_inv_with_hotbar(hb: &mut Hotbar, i: usize, k: usize) -> Option<ItemStack> {
    if i >= HOTBAR_SLOTS + MAIN_SLOTS || k >= HOTBAR_SLOTS {
        return None;
    }
    if i == k {
        return None;
    }
    if i < HOTBAR_SLOTS {
        std::mem::swap(&mut hb.slots[i], &mut hb.slots[k]);
        return None;
    }
    // 主背包槽：值拷出复用核心（规避同结构双 &mut），终态写回。
    let mut tgt = std::mem::replace(&mut hb.main[i - HOTBAR_SLOTS], ItemStack::empty());
    let rest = swap_with_hotbar(&mut tgt, hb, k);
    hb.main[i - HOTBAR_SLOTS] = tgt;
    rest
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

    /// shift+点击快移·背包槽（QUICK_MOVE：ACM:413-427 → CM:122-131）。
    /// 3x3：先试网格段（CM:118 moveItemStackTo(1,10,false)），颗粒无收再按
    /// 来源段对调——主背包→热栏（CM:125 段 37..46）/ 热栏→主背包（CM:
    /// 127-129 段 10..37）；2x2 无网格路由（IM:128-134 同构；装备/副手段 =
    /// 审计 #24 同债）。部分移动余量留原槽（守恒，ACM:136-139）。
    pub fn quick_move_inv(&mut self, i: usize, hb: &mut Hotbar) {
        if i >= HOTBAR_SLOTS + MAIN_SLOTS {
            return;
        }
        let mut st = std::mem::replace(hb.slot_mut(i), ItemStack::empty());
        if st.is_empty() {
            return;
        }
        let mut moved = false;
        if self.width == 3 {
            let mut grid: Vec<&mut ItemStack> = self.grid.iter_mut().collect();
            moved = move_stack_to(&mut st, &mut grid, false);
        }
        if !moved {
            let mut dst: Vec<&mut ItemStack> = if i < HOTBAR_SLOTS {
                hb.main.iter_mut().collect()
            } else {
                hb.slots.iter_mut().collect()
            };
            move_stack_to(&mut st, &mut dst, false);
        }
        *hb.slot_mut(i) = st;
    }

    /// shift+点击快移·网格槽（CM:132 else 段 moveItemStackTo(10,46,false) =
    /// 背包正向：主背包先、热栏后）。余量留网格槽。
    pub fn quick_move_grid(&mut self, i: usize, hb: &mut Hotbar) {
        if i >= self.grid.len() || self.grid[i].is_empty() {
            return;
        }
        let mut st = std::mem::replace(&mut self.grid[i], ItemStack::empty());
        let mut dst: Vec<&mut ItemStack> = hb.main.iter_mut().chain(hb.slots.iter_mut()).collect();
        move_stack_to(&mut st, &mut dst, false);
        self.grid[i] = st;
    }

    /// shift+点击快移·结果槽（CM:115-121 moveItemStackTo(10,46,true) = 背包
    /// 倒序、热栏先）。ACM:425-427 重移循环的本仓投影：整份结果全部放得下
    /// 才消耗网格并提交（克隆 dry-run）；放不下整份 = 原样不动（满则停）——
    /// 结果槽是配方重算模型，「部分移动 + 消耗」会凭空损耗余量，守恒硬
    /// 约束优先（债登记 HANDOFF-UITOOLS.md）。
    pub fn quick_move_result(&mut self, hb: &mut Hotbar) -> bool {
        let Some(res) = self.result() else {
            return false;
        };
        let fits = {
            let mut probe = hb.clone();
            let mut st = res.clone();
            let mut dst: Vec<&mut ItemStack> = probe
                .main
                .iter_mut()
                .chain(probe.slots.iter_mut())
                .collect();
            move_stack_to(&mut st, &mut dst, true);
            st.is_empty()
        };
        if !fits {
            return false;
        }
        let mut dst: Vec<&mut ItemStack> = hb.main.iter_mut().chain(hb.slots.iter_mut()).collect();
        let mut st = res;
        move_stack_to(&mut st, &mut dst, true);
        crafting::consume_grid(&mut self.grid);
        true
    }

    /// 数字键直换·网格槽（ACM:471-507 SWAP，核心见 [`swap_with_hotbar`]）。
    /// 返回放不进背包的换下余量（劈堆换分支，实际不可达；调用方落地）。
    pub fn hotbar_swap(&mut self, i: usize, hb: &mut Hotbar, k: usize) -> Option<ItemStack> {
        if i < self.grid.len() {
            swap_with_hotbar(&mut self.grid[i], hb, k)
        } else {
            None
        }
    }

    /// 数字键直换·结果槽：mayPlace=false（ResultSlot.java:25-27）→ 只允许
    /// 「空热栏槽整份取件」（ACM:476-482 源空分支，Slot.mayPickup 默认真，
    /// Slot.java:89-91）；取到即消耗一次网格（ResultSlot.onTake 链）。
    /// 热栏槽有物 = 无操作（mayPlace 门挡回放，ACM:493）。
    pub fn hotbar_swap_result(&mut self, hb: &mut Hotbar, k: usize) -> bool {
        if k >= HOTBAR_SLOTS || !hb.slots[k].is_empty() {
            return false;
        }
        let Some(res) = self.result() else {
            return false;
        };
        crafting::consume_grid(&mut self.grid);
        hb.slots[k] = res;
        true
    }

    /// 从结果槽取物(ACM:438-465 + ResultSlot.onTake)。空光标:`right` = true
    /// 取 ceil(半)(ACM:445)。光标同物(结果槽 mayPlace=false,落 ACM:459-465
    /// 并堆分支)= tryRemove(结果数, 光标余量) 一次补满,**右键不分半**;
    /// 装不下时取部分。成功则网格各占用格 −1(整体消耗一次,半取同样消耗)。
    pub fn take_result(&mut self, right: bool) -> bool {
        let Some(res) = self.result() else {
            return false;
        };
        let same_fill = !self.cursor.is_empty()
            && self.cursor.item == res.item
            && self.cursor.damage == 0
            && self.cursor.enchants.is_empty();
        // 光标同物(mayPlace=false 槽)= tryRemove(结果数, 光标余量) 一次
        // 补满,右键不分半(ACM:459-465);空光标才走左整右半(ACM:445)。
        let want = if right && !same_fill {
            res.count.div_ceil(2)
        } else {
            res.count
        };
        let take = if self.cursor.is_empty() {
            want
        } else if same_fill {
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

    /// 原版关屏语义的两路拆分(CM:96-99 `removed`→`clearContainer` = 网格物
    /// **drop 到世界**;carried 走 dropOrPlaceInInventory = 先背包后 drop 余量,
    /// ACM:588-596):返回 (carried, 网格掉落清单),调用方分别按
    /// add→drop 余量 / [`drop_container_items`] 处理,禁静默吞物(审计 #52)。
    pub fn close_to_world(self) -> (Option<ItemStack>, Vec<ItemStack>) {
        let cursor = if self.cursor.is_empty() {
            None
        } else {
            Some(self.cursor)
        };
        (
            cursor,
            self.grid.into_iter().filter(|s| !s.is_empty()).collect(),
        )
    }
}

/// 燃烧时长表（tick）。26.1 `FuelValues.vanillaBurnTimes`（FuelValues.java:
/// 42-110）按本仓已注册物品收编；baseUnit=200（:38-40），逐项引注：
/// coal/charcoal = baseUnit×8 = 1600（:47-48）；LOGS tag = ×3/2 = 300（:49）；
/// PLANKS tag = 300（:51）；WOODEN_STAIRS = 300（:53）；WOODEN_SLABS =
/// ×3/4 = 150（:55）；WOODEN_TRAPDOORS = 300（:57）；WOODEN_PRESSURE_PLATES
/// = 300（:58）；WOODEN_FENCES = 300（:60）；FENCE_GATES = 300（:61）；
/// WOODEN_DOORS = baseUnit = 200（:83）；BOW = 300（:72）；WOODEN_SHOVEL /
/// SWORD / SPEAR / HOE / AXE / PICKAXE = 200（:77-82）；SAPLINGS = ×1/2 =
/// 100（:88）；WOODEN_BUTTONS = 100（:86）；CHEST/CRAFTING_TABLE = 300
/// （:67/:69）。未收编物品（lava_bucket/coal_block/blaze_rod/charcoal 等
/// 未注册，缺口见 ci/recipes-gap.md）与本表外物品 = 0（非燃料，
/// FuelValues.isFuel containsKey 语义，FuelValues.java:26-28）。
/// 空堆调用方必须先 is_empty 短路（空堆 item=0 与 STICK 同号，见
/// [`FurnaceScreen::tick`] 的闸门次序）。
pub fn burn_duration(item: u16) -> u32 {
    use mcv_item::{
        ACACIA_DOOR, ACACIA_FENCE, ACACIA_LOG, ACACIA_PLANKS, BIRCH_DOOR, BIRCH_FENCE, BIRCH_LOG,
        BIRCH_PLANKS, BOW, CHEST, COAL, CRAFTING_TABLE, DARK_OAK_BUTTON, DARK_OAK_DOOR,
        DARK_OAK_FENCE, DARK_OAK_FENCE_GATE, DARK_OAK_LOG, DARK_OAK_PLANKS,
        DARK_OAK_PRESSURE_PLATE, DARK_OAK_SAPLING, DARK_OAK_SLAB, DARK_OAK_STAIRS,
        DARK_OAK_TRAPDOOR, JUNGLE_DOOR, JUNGLE_FENCE, JUNGLE_LOG, JUNGLE_PLANKS, LOG, OAK_BUTTON,
        OAK_DOOR, OAK_FENCE, OAK_FENCE_GATE, OAK_PRESSURE_PLATE, OAK_SAPLING, OAK_SLAB, OAK_STAIRS,
        OAK_TRAPDOOR, PLANKS, SPRUCE_DOOR, SPRUCE_FENCE, SPRUCE_LOG, SPRUCE_PLANKS, STICK,
        WOODEN_AXE_INDEX, WOODEN_PICKAXE_INDEX, WOODEN_SHOVEL_INDEX, WOODEN_SWORD_INDEX,
    };
    match item {
        COAL => 1600,
        STICK => 100,
        BOW => 300,
        PLANKS | SPRUCE_PLANKS | BIRCH_PLANKS | JUNGLE_PLANKS | ACACIA_PLANKS | DARK_OAK_PLANKS => {
            300
        }
        LOG | BIRCH_LOG | SPRUCE_LOG | JUNGLE_LOG | ACACIA_LOG | DARK_OAK_LOG => 300,
        OAK_SAPLING..=DARK_OAK_SAPLING => 100,
        OAK_STAIRS..=DARK_OAK_STAIRS => 300,
        OAK_SLAB..=DARK_OAK_SLAB => 150,
        OAK_FENCE | SPRUCE_FENCE | BIRCH_FENCE | JUNGLE_FENCE | ACACIA_FENCE | DARK_OAK_FENCE => {
            300
        }
        OAK_FENCE_GATE..=DARK_OAK_FENCE_GATE => 300,
        OAK_DOOR | SPRUCE_DOOR | BIRCH_DOOR | JUNGLE_DOOR | ACACIA_DOOR | DARK_OAK_DOOR => 200,
        OAK_TRAPDOOR..=DARK_OAK_TRAPDOOR => 300,
        OAK_BUTTON..=DARK_OAK_BUTTON => 100,
        OAK_PRESSURE_PLATE..=DARK_OAK_PRESSURE_PLATE => 300,
        WOODEN_SWORD_INDEX | WOODEN_PICKAXE_INDEX | WOODEN_AXE_INDEX | WOODEN_SHOVEL_INDEX => 200,
        CRAFTING_TABLE | CHEST => 300,
        _ => 0,
    }
}

/// 熔炉界面纯状态机（无渲染、可无头测试）。布局对齐 26.1 AbstractFurnaceMenu：
/// 槽 0 input / 1 fuel / 2 result（AM:26-28,75-77），背包段/结果槽 UI 坐标
/// 归渲染波（债：HUD 未画，见 HANDOFF-FURNACE.md）。烧炼节奏 =
/// `AbstractFurnaceBlockEntity.serverTick` 直译（AFBE:139-207）：每 tick 先
/// 递减 litTimeRemaining（:143-150）、lit 耗尽当帧可再抽燃料续燃（:163-173）、
/// CookTime 200 tick/件（BURN_TIME_STANDARD，:54）、结果未取且满/异品时
/// canBurn 门挡烧并把 cook 清零（:187-189）、离火 cook 按 BURN_COOL_SPEED=2
/// 退坡（:55,194-196）、换输入品 cook 归零（setItem，:292-302）。
/// 槽点击复用 [`click_slot`]（与 CraftScreen 同一判据族）；守恒硬约束同
/// CraftScreen：`close_to_world` 把光标与全部槽位交还调用方分别入栏/落地，
/// 禁静默吞物（审计 #52）。
#[derive(Clone, Debug)]
pub struct FurnaceScreen {
    /// 槽 0（AFBE:43）。
    pub input: ItemStack,
    /// 槽 1（AFBE:44）。只认 [`burn_duration`] 表内物品
    /// （FurnaceFuelSlot.mayPlace = isFuel，FurnaceFuelSlot.java:17-19）。
    pub fuel: ItemStack,
    /// 槽 2（AFBE:45）。只取不放（FurnaceResultSlot.mayPlace=false，
    /// FurnaceResultSlot.java:24-27）。
    pub result: ItemStack,
    /// 光标手持堆（鼠标抓着走；与 CraftScreen.cursor 同语义）。
    pub cursor: ItemStack,
    /// litTimeRemaining（AFBE:62）。
    pub lit_time: u32,
    /// litTotalTime（AFBE:63）= 抽入燃料的 burnDuration（:165-167）。
    pub lit_total: u32,
    /// cookingTimer（AFBE:64）。
    pub cook_time: u32,
    /// cookingTotalTime（AFBE:65）：输入换品置配方时长（无配方 orElse 200，
    /// :252-255），烧完一件重置为配方时长（:179）。
    pub cook_total: u32,
}

/// BURN_TIME_STANDARD（AFBE:54）：smelting 配方 cookingtime 全表 200
/// （ci/gen-recipes.py 普查 + recipes_gen.inc.rs 逐条 cook_time: 200）。
pub const COOK_TIME_STANDARD: u32 = 200;

/// BURN_COOL_SPEED（AFBE:55）：离火每 tick cook 退坡量（:194-196）。
pub const COOL_SPEED: u32 = 2;

/// isSameItemSameComponents 的本仓投影（数量不计入；空堆 item=0 与 STICK
/// 同号，故两侧都要求非空——vanilla 空 vs 非空恒 false，AFBE:294）。
fn same_stack(a: &ItemStack, b: &ItemStack) -> bool {
    !a.is_empty()
        && !b.is_empty()
        && a.item == b.item
        && a.damage == b.damage
        && a.enchants == b.enchants
}

/// 是否燃料（FuelValues.isFuel，FuelValues.java:26-28；空堆非燃料）。
fn is_fuel(item: u16) -> bool {
    burn_duration(item) > 0
}

impl Default for FurnaceScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl FurnaceScreen {
    pub fn new() -> Self {
        Self {
            input: ItemStack::empty(),
            fuel: ItemStack::empty(),
            result: ItemStack::empty(),
            cursor: ItemStack::empty(),
            lit_time: 0,
            lit_total: 0,
            cook_time: 0,
            cook_total: 0,
        }
    }

    /// `Container.setItem(0)`（AFBE:292-302）：写入 + limitSize + 品变时
    /// cookingTotalTime=getTotalCookTime（无配方 orElse 200，:252-255）、
    /// cookingTimer=0。同品补量（数量变化）不重置（isSameItemSameComponents，
    /// 判定先于 limitSize，:293-296；数量不计入判据）。
    fn set_input(&mut self, st: ItemStack) {
        let same = same_stack(&self.input, &st);
        let mut st = st;
        if !st.is_empty() {
            // limitSize（AFBE:296）：数量钳到物品堆叠上限。
            st.count = st.count.min(Hotbar::max_stack(st.item));
        }
        self.input = st;
        if !same {
            self.cook_total = mcv_item::crafting::find_cooking(self.input.item)
                .map(|r| r.cook_time as u32)
                .unwrap_or(COOK_TIME_STANDARD);
            self.cook_time = 0;
        }
    }

    /// 点击输入槽（槽 0 是普通 Slot——AM:76，任意物品可入；不匹配只是
    /// 不烧，AFBE:190-192 cook 清零）。左/右键语义见 [`click_slot`]。
    pub fn click_input(&mut self, left: bool) {
        let mut next = self.input.clone();
        click_slot(&mut next, &mut self.cursor, left);
        self.set_input(next);
    }

    /// 点击燃料槽。光标有非燃料 = 无操作（mayPlace=isFuel，
    /// FurnaceFuelSlot.java:17-19；异品交换同被 mayPlace 门挡，ACM PICKUP
    /// 链 only-mayPlace 分支可换）；取料不受限。
    pub fn click_fuel(&mut self, left: bool) {
        if !self.cursor.is_empty() && !is_fuel(self.cursor.item) {
            return;
        }
        click_slot(&mut self.fuel, &mut self.cursor, left);
    }

    /// 取出结果槽整堆（`FurnaceResultSlot` 只取不放，FurnaceResultSlot.java:
    /// 24-27；空光标单击取全堆的合成语义）。返回 None = 无物可取；取到
    /// 即计一件成品账（vanilla removeCount→checkTakeAchievements，
    /// :29-49——经验/成就面归后续波）。调用方走 Hotbar::add 既有拾取通道
    /// （game.rs:2738「give 路径唯一」），满栏余量走世界掉落。
    pub fn take_result(&mut self) -> Option<ItemStack> {
        if self.result.is_empty() {
            return None;
        }
        Some(std::mem::replace(&mut self.result, ItemStack::empty()))
    }

    /// shift+点击快移·背包槽（AM:104-119）。canSmelt → 输入段单试（AM:
    /// 106-108 失败即止，无背包对调回退——与 CM 的先网格后对调两段式
    /// 不同构）；否则 isFuel → 燃料段单试（AM:109-112，
    /// FurnaceFuelSlot.mayPlace=isFuel 门）；否则主↔热栏对调（AM:113-119
    /// 段 3..30 / 30..39）。入输入槽经 [`Self::set_input`]（AFBE setItem
    /// 面：换品清 cook）。
    pub fn quick_move_inv(&mut self, i: usize, hb: &mut Hotbar) {
        if i >= HOTBAR_SLOTS + MAIN_SLOTS {
            return;
        }
        let mut st = std::mem::replace(hb.slot_mut(i), ItemStack::empty());
        if st.is_empty() {
            return;
        }
        if mcv_item::crafting::find_cooking(st.item).is_some() {
            let mut next = self.input.clone();
            {
                let mut dst: Vec<&mut ItemStack> = vec![&mut next];
                move_stack_to(&mut st, &mut dst, false);
            }
            self.set_input(next);
        } else if is_fuel(st.item) {
            let mut dst: Vec<&mut ItemStack> = vec![&mut self.fuel];
            move_stack_to(&mut st, &mut dst, false);
        } else if i < HOTBAR_SLOTS {
            let mut dst: Vec<&mut ItemStack> = hb.main.iter_mut().collect();
            move_stack_to(&mut st, &mut dst, false);
        } else {
            let mut dst: Vec<&mut ItemStack> = hb.slots.iter_mut().collect();
            move_stack_to(&mut st, &mut dst, false);
        }
        *hb.slot_mut(i) = st;
    }

    /// shift+点击快移·容器槽（input=0 / fuel=1 → AM:120-122 段 3..39 正向 =
    /// 主背包先、热栏后）。出输入槽经 [`Self::set_input`]（清空 = 品变，
    /// cook_total/timer 重置，AFBE setItem 面）；结果槽走
    /// [`Self::quick_move_result`]。
    pub fn quick_move_slot(&mut self, slot: usize, hb: &mut Hotbar) {
        if slot > 1 {
            return;
        }
        let empty_now = match slot {
            0 => self.input.is_empty(),
            _ => self.fuel.is_empty(),
        };
        if empty_now {
            return;
        }
        let mut st = match slot {
            0 => std::mem::replace(&mut self.input, ItemStack::empty()),
            _ => std::mem::replace(&mut self.fuel, ItemStack::empty()),
        };
        let mut dst: Vec<&mut ItemStack> = hb.main.iter_mut().chain(hb.slots.iter_mut()).collect();
        move_stack_to(&mut st, &mut dst, false);
        match slot {
            0 => self.set_input(st),
            _ => self.fuel = st,
        }
    }

    /// shift+点击快移·结果槽（AM:98-103 moveItemStackTo(3,39,true) = 背包
    /// 倒序、热栏先）。物理槽：部分移动余量留槽（满则停，ACM:124-128）。
    pub fn quick_move_result(&mut self, hb: &mut Hotbar) {
        if self.result.is_empty() {
            return;
        }
        let mut st = std::mem::replace(&mut self.result, ItemStack::empty());
        let mut dst: Vec<&mut ItemStack> = hb.main.iter_mut().chain(hb.slots.iter_mut()).collect();
        move_stack_to(&mut st, &mut dst, true);
        self.result = st;
    }

    /// 数字键直换·悬停槽（ACM:471-507 SWAP，核心见 [`swap_with_hotbar`]）。
    /// 准入门：输入槽 mayPlace 恒真（AM:76 普通 Slot）；燃料槽 mayPlace =
    /// isFuel（FurnaceFuelSlot.java:17-19，源非空且非燃料 = 无操作）；
    /// 结果槽 mayPlace=false（FurnaceResultSlot.java:24-27）→ 只允许「空
    /// 热栏槽取件」单向（ACM:476-482，Slot.mayPickup 默认真）。返回放不
    /// 进背包的换下余量（劈堆换分支，实际不可达；调用方落地）。
    pub fn hotbar_swap(&mut self, slot: usize, hb: &mut Hotbar, k: usize) -> Option<ItemStack> {
        if k >= HOTBAR_SLOTS {
            return None;
        }
        match slot {
            0 => swap_with_hotbar(&mut self.input, hb, k),
            1 => {
                if hb.slots[k].is_empty() || is_fuel(hb.slots[k].item) {
                    swap_with_hotbar(&mut self.fuel, hb, k)
                } else {
                    None
                }
            }
            2 => {
                if hb.slots[k].is_empty() && !self.result.is_empty() {
                    hb.slots[k] = std::mem::replace(&mut self.result, ItemStack::empty());
                }
                None
            }
            _ => None,
        }
    }

    /// canBurn（AFBE:218-231）：结果空 = 可烧；异品 = 不可；同品按物品
    /// 堆叠上限计余量（容器 64 ⊇ 物品上限，min 恒为物品上限）。
    fn can_burn(&self, result: (u16, u8)) -> bool {
        if self.result.is_empty() {
            return true;
        }
        if self.result.item != result.0 {
            return false;
        }
        self.result.count as u32 + result.1 as u32 <= Hotbar::max_stack(result.0) as u32
    }

    /// `AbstractFurnaceBlockEntity.serverTick` 单 tick 直译（AFBE:139-207）。
    /// `ticks` = 本步跨过的 20 Hz tick 数（n>1 补齐跨步亏欠）。
    pub fn tick(&mut self, ticks: u32) {
        for _ in 0..ticks {
            // lit 递减先行（AFBE:143-150）。
            let was_lit = self.lit_time > 0;
            if was_lit {
                self.lit_time -= 1;
            }
            let mut is_lit = self.lit_time > 0;

            let has_ingredient = !self.input.is_empty();
            let has_fuel = !self.fuel.is_empty();
            if is_lit || (has_fuel && has_ingredient) {
                if has_ingredient {
                    // quickCheck.getRecipeFor（AFBE:158-159）→ smelt_result
                    // 的底层表（crafting.rs:657-659 同一 find_cooking；
                    // cook_time 须随配方走，不新增 mcv_item API）。
                    let recipe = mcv_item::crafting::find_cooking(self.input.item);
                    if let Some(r) = recipe {
                        let burn_result = r.result;
                        if self.can_burn(burn_result) {
                            if !is_lit {
                                // getBurnDuration（AFBE:165）；空堆必 0——
                                // item=0 与 STICK 同号，闸门先于查表。
                                let new_lit = if self.fuel.is_empty() {
                                    0
                                } else {
                                    burn_duration(self.fuel.item)
                                };
                                self.lit_time = new_lit;
                                self.lit_total = new_lit;
                                if new_lit > 0 {
                                    // consumeFuel（AFBE:209-216；本仓物品
                                    // 无 crafting remainder，余桶分支不存在）。
                                    self.fuel.count -= 1;
                                    if self.fuel.count == 0 {
                                        self.fuel = ItemStack::empty();
                                    }
                                    is_lit = true;
                                }
                            }
                            if is_lit {
                                self.cook_time += 1;
                                if self.cook_time == self.cook_total {
                                    self.cook_time = 0;
                                    self.cook_total = r.cook_time as u32;
                                    // burn（AFBE:233-246）：结果并堆、输入 −1。
                                    if self.result.is_empty() {
                                        self.result = ItemStack::new(burn_result.0, burn_result.1);
                                    } else {
                                        self.result.count += burn_result.1;
                                    }
                                    self.input.count -= 1;
                                    if self.input.count == 0 {
                                        self.input = ItemStack::empty();
                                    }
                                }
                            } else {
                                self.cook_time = 0;
                            }
                        } else {
                            self.cook_time = 0;
                        }
                    } else {
                        self.cook_time = 0;
                    }
                } else {
                    self.cook_time = 0;
                }
            } else if self.cook_time > 0 {
                // 离火退坡（AFBE:194-196）。
                self.cook_time = self
                    .cook_time
                    .saturating_sub(COOL_SPEED)
                    .min(self.cook_total);
            }
        }
    }

    /// AbstractFurnaceMenu.isLit（AM:166-168）。
    pub fn is_lit(&self) -> bool {
        self.lit_time > 0
    }

    /// getBurnProgress（AM:152-158）：箭头 0..1；total/current 有一为 0 = 0。
    pub fn cook_progress(&self) -> f32 {
        if self.cook_total != 0 && self.cook_time != 0 {
            (self.cook_time as f32 / self.cook_total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// getLitProgress（AM:160-166）：火焰 0..1；litTotalTime==0 按 200 计。
    pub fn lit_progress(&self) -> f32 {
        let total = if self.lit_total == 0 {
            COOK_TIME_STANDARD
        } else {
            self.lit_total
        };
        (self.lit_time as f32 / total as f32).clamp(0.0, 1.0)
    }

    /// 关界面两路拆分（同 CraftScreen::close_to_world）：光标归调用方
    /// 先背包后掉落（ACM:588-596），容器槽归落地清单。本仓无方块实体
    /// 存储，原版「炉内容留在炉里」（AFBE removed 不清容器）不可持久——
    /// 关屏即取回，守恒硬约束优先（审计 #52），债登记 HANDOFF-FURNACE.md。
    pub fn close_to_world(self) -> (Option<ItemStack>, Vec<ItemStack>) {
        let cursor = if self.cursor.is_empty() {
            None
        } else {
            Some(self.cursor)
        };
        let drops: Vec<ItemStack> = [self.input, self.fuel, self.result]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        (cursor, drops)
    }
}

/// 把成堆物品丢进世界(原版 `clearContainer`→`Level.dropItem`,CM:96-99 /
/// IM:87-94):容器界面关闭/死亡时归还网格与背包溢出余量的落地通道,
/// 数据丢失级兜底(审计 #52:关屏满栏静默丢弃禁止)。拾取延迟同普通
/// 挖掘掉落(PICKUP_DELAY),生成点 = 玩家上方 0.9(死亡掉落同基准)。
pub fn drop_container_items(rt: &mut GameRuntime, stacks: &[ItemStack]) {
    let at = rt.player.pos + glam::Vec3::new(0.0, 0.9, 0.0);
    let mut seed = rt.game_ticks.wrapping_mul(6364136223846793005) | 1;
    let mut rng = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    for s in stacks {
        if s.is_empty() {
            continue;
        }
        mcv_entity::spawn_item_drop(
            &mut rt.mobs_app.world,
            at,
            s.item,
            s.count,
            mcv_entity::PICKUP_DELAY,
            &mut rng,
        );
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
    use mcv_item::{
        COBBLESTONE, CRAFTING_TABLE, OAK_BUTTON, PLANKS, STICK, WOODEN_PICKAXE_INDEX,
        WOODEN_SWORD_INDEX,
    };

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
        // 剩余 1 板在 26.1 是合法按钮配方（oak_button.json：1 橡木板 shapeless）
        // ——消耗判据改锁「只剩板」：清空后无配方。
        assert_eq!(
            cs.result().map(|s| (s.item, s.count)),
            Some((OAK_BUTTON, 1))
        );
        cs.grid[0] = ItemStack::empty();
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
        // 镐 MMM/_S_/_S_ 在 2x2 里放不下 → 不是镐。26.1 下 2x2 满板本身是
        // 工作台配方（crafting_table.json 2×2 板 tag），结果锁工作台而非无。
        let mut cs2 = CraftScreen::new(2);
        for s in cs2.grid.iter_mut() {
            *s = stack(PLANKS, 1);
        }
        assert_eq!(
            cs2.result().map(|s| s.item),
            Some(CRAFTING_TABLE),
            "2x2 满板=工作台，不是镐"
        );
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
    fn left_click_same_kind_full_slot_noop() {
        // 审计 #36：同类满槽左键 no-op（vanilla isSameItemSameComponents →
        // safeInsert(cap=0)，ACM:452-454），不许把满堆"钓"进光标。
        let mut cs = CraftScreen::new(2);
        cs.grid[0] = stack(COBBLESTONE, 64);
        cs.cursor = stack(COBBLESTONE, 10);
        cs.click_grid(0, true);
        assert_eq!(cs.cursor.count, 10, "光标不许换到满堆");
        assert_eq!(cs.grid[0].count, 64);
        // 背包槽同判据。
        let mut hb = Hotbar::empty();
        hb.slots[0] = stack(COBBLESTONE, 64);
        cs.cursor = stack(COBBLESTONE, 10);
        cs.click_inv(0, true, &mut hb);
        assert_eq!(cs.cursor.count, 10);
        assert_eq!(hb.slots[0].count, 64);
    }

    #[test]
    fn right_click_result_fills_same_kind_cursor() {
        // 审计 #40：光标同物时右键结果槽 = ACM:459-465 一次补满，不分半。
        // 木棍配方两板竖排 → 4 根；光标 60 根右键 → 直接 64（旧实现只 +2）。
        let mut cs = CraftScreen::new(2);
        cs.grid[0] = stack(PLANKS, 1);
        cs.grid[2] = stack(PLANKS, 1);
        cs.cursor = stack(STICK, 60);
        assert!(cs.take_result(true));
        assert_eq!(cs.cursor.count, 64, "右键补满 = min(结果数, 光标余量)");
        assert!(cs.grid.iter().all(|s| s.is_empty()), "补满同样消耗网格");
    }

    #[test]
    fn close_to_world_splits_cursor_and_grid_drops() {
        // 审计 #52：关屏网格物走世界掉落通道（CM:96-99），carried 单独
        // 归还（先背包后 drop 余量），两路不许混。
        let mut cs = CraftScreen::new(2);
        cs.grid[0] = stack(PLANKS, 3);
        cs.grid[3] = stack(COBBLESTONE, 7);
        cs.cursor = stack(STICK, 5);
        let (cursor, drops) = cs.close_to_world();
        assert_eq!(cursor.map(|s| (s.item, s.count)), Some((STICK, 5)));
        assert_eq!(drops.len(), 2, "非空网格格全部进掉落清单");
        assert!(drops.iter().any(|s| s.item == PLANKS && s.count == 3));
        assert!(drops.iter().any(|s| s.item == COBBLESTONE && s.count == 7));
        // 空格不进清单；光标空 → None。
        let empty = CraftScreen::new(2);
        let (c, d) = empty.close_to_world();
        assert!(c.is_none() && d.is_empty());
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
