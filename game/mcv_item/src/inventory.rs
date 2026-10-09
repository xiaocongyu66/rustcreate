//! 物品栏逻辑:9 格 Hotbar + 27 格主背包(vanilla `Inventory.add` 语义的
//! 36 格子集)与方块→掉落映射。

use mcv_core::BlockId;

use crate::{COAL, COBBLESTONE, DIAMOND_ITEM, ItemKind, ItemStack, LOG, PLANKS};

/// 快捷栏格数(vanilla Inventory.hotbarSize)。
pub const HOTBAR_SLOTS: usize = 9;
/// 主背包格数(vanilla Inventory.mainSize,快捷栏之外的 27 格)。
pub const MAIN_SLOTS: usize = 27;

/// 9 格快捷栏 + 27 格主背包。slot 0..8 = 主界面快捷栏;选中槽 = 玩家
/// `sel_slot % 9`;`main` 为背包 UI 上半区 27 格。
#[derive(Clone, Debug)]
pub struct Hotbar {
    pub slots: [ItemStack; HOTBAR_SLOTS],
    /// 主背包 36 格布局的 9..35 段(默认全空)。
    pub main: [ItemStack; MAIN_SLOTS],
}

impl Default for Hotbar {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| ItemStack::empty()),
            main: std::array::from_fn(|_| ItemStack::empty()),
        }
    }
}

impl Hotbar {
    pub fn empty() -> Self {
        Self::default()
    }

    /// 单种堆叠上限(vanilla Items 各 maxStackSize:工具/附魔书 1、成书 16、
    /// 其余 64)。
    pub fn max_stack(item: u16) -> u8 {
        use crate::ITEMS;
        match ITEMS[item as usize].kind {
            ItemKind::Sword(_)
            | ItemKind::Pickaxe(_)
            | ItemKind::Axe(_)
            | ItemKind::Shovel(_)
            | ItemKind::EnchantedBook => 1,
            ItemKind::Book => 16,
            _ => 64,
        }
    }

    pub fn selected(&self, sel: usize) -> &ItemStack {
        &self.slots[sel % HOTBAR_SLOTS]
    }

    pub fn selected_mut(&mut self, sel: usize) -> &mut ItemStack {
        &mut self.slots[sel % HOTBAR_SLOTS]
    }

    /// 可堆叠(同物、同耐久、无附魔、未满)才合并——附魔/耐久物品永不
    /// 合并(vanilla ItemStack.isStackable + areItemsEqual 语义)。
    fn mergeable(dst: &ItemStack, src: &ItemStack, max: u8) -> bool {
        src.item == dst.item
            && src.damage == dst.damage
            && src.enchants.is_empty()
            && dst.enchants.is_empty()
            && dst.count < max
    }

    /// vanilla `Inventory.add` 的 9 格语义:选中槽优先合并 → 任意槽合并 →
    /// 选中槽空位 → 任意空槽。返回放不下的剩余(满栏时 Some)。
    pub fn add(&mut self, sel: usize, mut stack: ItemStack) -> Option<ItemStack> {
        if stack.is_empty() {
            return None;
        }
        let max = Self::max_stack(stack.item);
        if max > 1 {
            let sel = sel % HOTBAR_SLOTS;
            if Self::mergeable(&self.slots[sel], &stack, max) {
                let room = max - self.slots[sel].count;
                let mv = stack.count.min(room);
                self.slots[sel].count += mv;
                stack.count -= mv;
                if stack.count == 0 {
                    return None;
                }
            }
            for i in 0..HOTBAR_SLOTS {
                if i == sel {
                    continue;
                }
                if Self::mergeable(&self.slots[i], &stack, max) {
                    let room = max - self.slots[i].count;
                    let mv = stack.count.min(room);
                    self.slots[i].count += mv;
                    stack.count -= mv;
                    if stack.count == 0 {
                        return None;
                    }
                }
            }
        }
        let sel = sel % HOTBAR_SLOTS;
        if self.slots[sel].is_empty() {
            self.slots[sel] = stack;
            return None;
        }
        for slot in self.slots.iter_mut() {
            if slot.is_empty() {
                *slot = stack;
                return None;
            }
        }
        Some(stack)
    }

    /// 全局 36 格可写视图:0..8 = 快捷栏,9..35 = 主背包(背包 UI/溢出
    /// 放物共用)。
    pub fn slot_mut(&mut self, i: usize) -> &mut ItemStack {
        if i < HOTBAR_SLOTS {
            &mut self.slots[i]
        } else {
            &mut self.main[i - HOTBAR_SLOTS]
        }
    }

    /// 全局 36 格只读视图(克隆;渲染/遍历用)。
    pub fn all_slots(&self) -> [ItemStack; HOTBAR_SLOTS + MAIN_SLOTS] {
        std::array::from_fn(|i| {
            if i < HOTBAR_SLOTS {
                self.slots[i].clone()
            } else {
                self.main[i - HOTBAR_SLOTS].clone()
            }
        })
    }

    /// `slot` 能否并入 `item` 的干净新堆(同物、无耐久/附魔、未满)。
    fn can_accept(slot: &ItemStack, item: u16, max: u8) -> bool {
        slot.item == item && slot.damage == 0 && slot.enchants.is_empty() && slot.count < max
    }

    /// 溢出放物(vanilla `add` 无选中槽优先的形态):先快捷栏合并、再主背包
    /// 合并,然后快捷栏空位、主背包空位。返回放不下的剩余数量(0 = 全收)。
    pub fn add_overflow(&mut self, item: u16, count: u8) -> u8 {
        if count == 0 {
            return 0;
        }
        let max = Self::max_stack(item);
        let mut rest = count;
        if max > 1 {
            for i in 0..HOTBAR_SLOTS + MAIN_SLOTS {
                let slot = self.slot_mut(i);
                if Self::can_accept(slot, item, max) {
                    let mv = rest.min(max - slot.count);
                    slot.count += mv;
                    rest -= mv;
                    if rest == 0 {
                        return 0;
                    }
                }
            }
        }
        for i in 0..HOTBAR_SLOTS + MAIN_SLOTS {
            if self.slot_mut(i).is_empty() {
                *self.slot_mut(i) = ItemStack::new(item, rest);
                return 0;
            }
        }
        rest
    }

    /// 从尾到头取最后一个非空槽(36 格全局序;关界面归还光标/网格物品时
    /// 反填用)。
    pub fn take_last_nonempty(&mut self) -> Option<ItemStack> {
        for i in (0..HOTBAR_SLOTS + MAIN_SLOTS).rev() {
            if !self.slot_mut(i).is_empty() {
                return Some(std::mem::replace(self.slot_mut(i), ItemStack::empty()));
            }
        }
        None
    }

    /// 消耗选中槽一个(vanilla `consumeItem(1)`):归零则槽清空。返回是否
    /// 真的消耗了。
    pub fn take_one(&mut self, sel: usize) -> bool {
        let s = self.selected_mut(sel);
        if s.is_empty() {
            return false;
        }
        s.count -= 1;
        if s.count == 0 {
            *s = ItemStack::empty();
        }
        true
    }
}

/// 方块破坏掉落(26.1 `BlockBehaviour.dropResources` 语义的已注册子集):
/// 石头掉圆石、煤矿掉煤炭、钻石矿掉钻石;无对应物品的方块不掉落
/// (TODO(registry):圆石/泥土/沙等方块与粗铁物品进注册表后补全)。
pub fn drop_for_block(id: BlockId) -> Option<ItemStack> {
    let name = mcv_core::BLOCKS[id.0 as usize].name;
    let item = match name {
        // 方块表 id 9 的注册名是 "cobble"(blocks_gen),非原版 cobblestone。
        "stone" | "cobble" => COBBLESTONE,
        "planks" => PLANKS,
        "log" => LOG,
        "coal_ore" => COAL,
        "diamond_ore" => DIAMOND_ITEM,
        // 26.1 原版掉粗铁(raw_iron),物品表尚无粗铁——宁缺勿错。
        _ => return None,
    };
    // 掉落量恒 1(26.1 dropResource 默认;时运等附魔未实现)。
    Some(ItemStack::new(item, 1))
}
