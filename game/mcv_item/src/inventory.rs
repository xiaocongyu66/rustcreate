//! 物品栏逻辑:9 格 Hotbar + 27 格主背包(vanilla `Inventory.add` 语义的
//! 36 格子集)与方块→掉落映射。

use mcv_core::BlockId;

use crate::{ItemKind, ItemStack};

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

    /// 单种堆叠上限(vanilla Items:maxStackSize:工具/附魔书 1——Item.java
    /// durability 注册时同步 MAX_STACK_SIZE=1、Items 附魔书 stacksTo(1);
    /// 书=64 为 Item 默认值,16 是成书 written_book 的上限,其余 64)。
    pub fn max_stack(item: u16) -> u8 {
        use crate::ITEMS;
        match ITEMS[item as usize].kind {
            ItemKind::Sword(_)
            | ItemKind::Pickaxe(_)
            | ItemKind::Axe(_)
            | ItemKind::Shovel(_)
            | ItemKind::EnchantedBook => 1,
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

    /// vanilla `Inventory.add`(INV:195-302):合并扫描 = 选中槽 → 全局 0..35 序
    /// (多趟 do-while,每趟把余量填进找到的每个部分堆,直到放尽或无处可填);
    /// 空位 = 全局 0..35 首个空槽(`getFreeSlot`,不偏选中槽)。副手槽未实现,
    /// 跳过其"选中→副手→全序"中的副手一档。返回放不下的剩余(满栏时 Some)。
    pub fn add(&mut self, sel: usize, mut stack: ItemStack) -> Option<ItemStack> {
        if stack.is_empty() {
            return None;
        }
        let max = Self::max_stack(stack.item);
        let sel = sel % HOTBAR_SLOTS;
        loop {
            if max > 1 {
                let mut merged = false;
                for i in std::iter::once(sel).chain(0..HOTBAR_SLOTS + MAIN_SLOTS) {
                    let slot = self.slot_mut(i);
                    if Self::mergeable(slot, &stack, max) {
                        let mv = stack.count.min(max - slot.count);
                        slot.count += mv;
                        stack.count -= mv;
                        merged = true;
                        if stack.count == 0 {
                            return None;
                        }
                    }
                }
                if merged {
                    continue;
                }
            }
            if let Some(i) = (0..HOTBAR_SLOTS + MAIN_SLOTS).find(|&i| self.slot_mut(i).is_empty()) {
                *self.slot_mut(i) = stack;
                return None;
            }
            return Some(stack);
        }
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

/// 一条掉落规则(26.1 `data/minecraft/loot_table/blocks/*.json` 的已注册
/// 子集;silk touch / 时运附魔未实现,只取无附魔基数语义)。
struct DropRow {
    /// 适用方块(blocks_gen 注册名;旧表别名如 "cobble" 在此对齐原版名)。
    blocks: &'static [&'static str],
    /// 掉落物品注册名(ITEMS.name;运行时经 item_by_name 解析,缺失跳过)。
    item: &'static str,
    /// 数量区间 [min, max](26.1 set_count / ore_drops 基数)。
    min: u8,
    max: u8,
    /// 触发概率(千分比):1000 = 必掉(survives_explosion 池)。
    permille: u16,
    /// 26.1 alternatives 语义:主项未触发时改掉 fallback(砂砾=燧石 10%
    /// 否则砂砾,gravel.json)。
    fallback: Option<(&'static str, u8, u8)>,
}

const fn row(
    blocks: &'static [&'static str],
    item: &'static str,
    min: u8,
    max: u8,
    permille: u16,
) -> DropRow {
    DropRow {
        blocks,
        item,
        min,
        max,
        permille,
        fallback: None,
    }
}

/// 26.1 blocks loot 表 → 本引擎物品注册表的已注册子集。
/// 抽查对账(/root/mc-ref/src-26.1/data/minecraft/loot_table/blocks/):
/// stone.json→cobblestone、deepslate.json→cobbled_deepslate、
/// grass_block.json→dirt、gravel.json→flint 10%、farmland.json→dirt、
/// iron_ore.json→raw_iron、copper_ore.json→raw_copper 2-5、
/// redstone_ore.json→redstone 4-5、lapis_ore.json→lapis 4-9、
/// oak_leaves.json→树苗 5%/苹果 0.5%/木棍 2%×1-2 三独立池。
static DROP_TABLE: &[DropRow] = &[
    // 石头族(silk 掉自身分支未实现)。
    row(&["stone", "cobble"], "cobblestone", 1, 1, 1000),
    row(&["deepslate"], "cobbled_deepslate", 1, 1, 1000),
    // 泥土族(grass_block 雪态 snow_grass 同表掉 dirt;耕地掉 dirt)。
    row(&["dirt"], "dirt", 1, 1, 1000),
    row(&["grass", "snow_grass", "farmland"], "dirt", 1, 1, 1000),
    row(&["sand"], "sand", 1, 1, 1000),
    // 砂砾:燧石 10%(table_bonus 无时运基数 0.1),否则掉自身。
    DropRow {
        blocks: &["gravel"],
        item: "flint",
        min: 1,
        max: 1,
        permille: 100,
        fallback: Some(("gravel", 1, 1)),
    },
    // 矿石族(必掉、基数计数;时运 ore_drops 公式未实现)。
    row(&["coal_ore", "deepslate_coal_ore"], "coal", 1, 1, 1000),
    row(&["iron_ore", "deepslate_iron_ore"], "raw_iron", 1, 1, 1000),
    row(
        &["copper_ore", "deepslate_copper_ore"],
        "raw_copper",
        2,
        5,
        1000,
    ),
    row(&["gold_ore", "deepslate_gold_ore"], "raw_gold", 1, 1, 1000),
    row(
        &["redstone_ore", "deepslate_redstone_ore"],
        "redstone",
        4,
        5,
        1000,
    ),
    row(&["lapis_ore", "deepslate_lapis_ore"], "lapis", 4, 9, 1000),
    row(
        &["diamond_ore", "deepslate_diamond_ore"],
        "diamond",
        1,
        1,
        1000,
    ),
    row(
        &["emerald_ore", "deepslate_emerald_ore"],
        "emerald",
        1,
        1,
        1000,
    ),
    // 木与木板(既有)。
    row(&["log"], "log", 1, 1, 1000),
    row(&["planks"], "planks", 1, 1, 1000),
    // 树叶族(oak_leaves.json;注册表无 oak_leaves,旧表 "leaves" 即橡树叶位):
    // 树苗/木棍/苹果三独立池,table_bonus 基数=无 Fortune 概率。木棍池:
    // 六树叶各有(全 *_leaves.json chances[0]=0.02, set_count uniform 1-2;
    // 门 = inverted(shears|silk),本引擎未注册剪刀/silk → 恒放行)。苹果池:
    // 橡树与深色橡树各有 0.5%(oak_leaves.json 与 dark_oak_leaves.json 均带
    // apple 池 chances[0]=0.005;旧注释"苹果只橡树"与 26.1 相悖已改)。
    // 树苗:橡/桦/云杉/金合欢/深板橡 5%、丛林 2.5%(table_bonus 基数)。
    row(&["leaves"], "oak_sapling", 1, 1, 50),
    row(&["leaves"], "apple", 1, 1, 5),
    row(&["leaves"], "stick", 1, 2, 20),
    row(&["birch_leaves"], "birch_sapling", 1, 1, 50),
    row(&["birch_leaves"], "stick", 1, 2, 20),
    row(&["spruce_leaves"], "spruce_sapling", 1, 1, 50),
    row(&["spruce_leaves"], "stick", 1, 2, 20),
    row(&["acacia_leaves"], "acacia_sapling", 1, 1, 50),
    row(&["acacia_leaves"], "stick", 1, 2, 20),
    row(&["dark_oak_leaves"], "dark_oak_sapling", 1, 1, 50),
    row(&["dark_oak_leaves"], "apple", 1, 1, 5),
    row(&["dark_oak_leaves"], "stick", 1, 2, 20),
    row(&["jungle_leaves"], "jungle_sapling", 1, 1, 25),
    row(&["jungle_leaves"], "stick", 1, 2, 20),
    // 雪(snow.json 按 layers 1..8 各分支 set_count=layers 掉雪球;本引擎无
    // 方块状态、雪按单层建模 → 1 个)。snow_block.json 无 silk 分支掉 4 个
    // (silk 分支掉自身;本引擎无 silk 附魔,恒走非 silk 侧)。
    row(&["snow"], "snowball", 1, 1, 1000),
    row(&["snow_block"], "snowball", 4, 4, 1000),
];

/// 明确"silk touch 才有掉落"的方块(glass.json:池条件只放行 silk)。
/// 本引擎无 silk → 恒不掉;单列出来是给"误加进 DROP_TABLE"设一道测试闸。
/// TODO(registry):16 色染色玻璃/玻璃板同语义,进全表数据化时补全。
pub const NO_DROP: &[&str] = &["glass", "glass_pane"];

/// 方块破坏掉落(26.1 `Block.playerDestroy` → `dropResources` 语义,表驱动)。
///
/// 调用方必须先过 `hasCorrectToolForDrops` 门(`mining::has_correct_tool`,
/// ServerPlayerGameMode.java:295-299:canDestroy 为假时不走 playerDestroy)。
/// 概率池与数量区间用调用方 `rng` 掷点;无规则方块返回空表(未注册 =
/// 宁缺勿错,玻璃族 = `NO_DROP` 明确不掉)。树叶族多池可同 tick 多掉。
pub fn drops_for_block(id: BlockId, rng: &mut impl FnMut() -> u32) -> Vec<ItemStack> {
    let name = mcv_core::BLOCKS[id.id() as usize].name;
    let mut out = Vec::new();
    for row in DROP_TABLE.iter().filter(|r| r.blocks.contains(&name)) {
        // 必掉池不消耗 rng(确定性路径与掉落点抖动序列解耦)。
        let hit = row.permille >= 1000 || ((rng() % 1000) as u16) < row.permille;
        let (item, min, max) = match (hit, row.fallback) {
            (true, _) => (row.item, row.min, row.max),
            (false, Some(fb)) => fb,
            (false, None) => continue,
        };
        let Some(item_id) = crate::item_by_name(item) else {
            continue;
        };
        let count = if max > min {
            min + (rng() % u32::from(max - min + 1)) as u8
        } else {
            min
        };
        if count > 0 {
            out.push(ItemStack::new(item_id, count));
        }
    }
    out
}
