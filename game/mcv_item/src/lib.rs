//! Items: stacks, tool materials, durability, enchanting, anvil repair,
//! crafting. Constants from mc-ref/NOTES-2.md (MC 26.1 decompiled).

pub mod anvil;
pub mod bow;
pub mod crafting;
pub mod enchant;
pub mod inventory;
pub mod mining;

pub use inventory::{HOTBAR_SLOTS, Hotbar, MAIN_SLOTS, drops_for_block};
pub use mining::ToolKind;

use mcv_core::BlockId;

/// Tool material stats (NOTES-2 §3: ToolMaterial.java:24-32).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolMaterial {
    pub durability: u16,
    pub speed: u8,
    pub attack_bonus: u8,
    pub enchant_value: u8,
}

pub const WOOD: ToolMaterial = ToolMaterial {
    durability: 59,
    speed: 2,
    attack_bonus: 0,
    enchant_value: 15,
};
pub const STONE_M: ToolMaterial = ToolMaterial {
    durability: 131,
    speed: 4,
    attack_bonus: 1,
    enchant_value: 5,
};
pub const COPPER: ToolMaterial = ToolMaterial {
    durability: 190,
    speed: 5,
    attack_bonus: 1,
    enchant_value: 13,
};
pub const IRON: ToolMaterial = ToolMaterial {
    durability: 250,
    speed: 6,
    attack_bonus: 2,
    enchant_value: 14,
};
pub const DIAMOND: ToolMaterial = ToolMaterial {
    durability: 1561,
    speed: 8,
    attack_bonus: 3,
    enchant_value: 10,
};
pub const GOLD: ToolMaterial = ToolMaterial {
    durability: 32,
    speed: 12,
    attack_bonus: 0,
    enchant_value: 22,
};
pub const NETHERITE: ToolMaterial = ToolMaterial {
    durability: 2031,
    speed: 9,
    attack_bonus: 4,
    enchant_value: 15,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Block(BlockId),
    Sword(ToolMaterial),
    Pickaxe(ToolMaterial),
    Axe(ToolMaterial),
    Shovel(ToolMaterial),
    Stick,
    Coal,
    IronIngot,
    Diamond,
    Lapis,
    Book,
    EnchantedBook,
    /// 弓（26.1 BowItem；右键蓄力放箭，曲线见 [`crate::bow`]）。
    Bow,
    /// 杂物（怪物掉落：腐肉/骨头等，64 堆、无耐久、不参战）。
    Material,
}

/// Item definition: baseline attack, attack speed, durability, repair tags.
pub struct ItemDef {
    pub name: &'static str,
    pub kind: ItemKind,
    /// Baseline attack bonus (sword 3.0, pickaxe 1.0; NOTES-2 §3 Items.java).
    pub baseline_attack: f32,
    /// Attacks per second = 4.0 + modifier (sword -2.4, pickaxe -2.8).
    pub attack_speed_modifier: f32,
    /// Max durability (0 = no durability).
    pub max_damage: u16,
    /// Durability lost per attack (Weapon component; sword 1, tools 2).
    pub item_damage_per_attack: u8,
}

macro_rules! def {
    ($name:literal, $kind:expr, $base:expr, $speed:expr, $dur:expr, $dpa:expr) => {
        ItemDef {
            name: $name,
            kind: $kind,
            baseline_attack: $base,
            attack_speed_modifier: $speed,
            max_damage: $dur,
            item_damage_per_attack: $dpa,
        }
    };
}

/// Registry order must stay stable (ids are serialized).
pub static ITEMS: [ItemDef; 58] = [
    def!("stick", ItemKind::Stick, 0.0, 0.0, 0, 0),
    def!("coal", ItemKind::Coal, 0.0, 0.0, 0, 0),
    def!("iron_ingot", ItemKind::IronIngot, 0.0, 0.0, 0, 0),
    def!("diamond", ItemKind::Diamond, 0.0, 0.0, 0, 0),
    def!("lapis", ItemKind::Lapis, 0.0, 0.0, 0, 0),
    def!("book", ItemKind::Book, 0.0, 0.0, 0, 0),
    def!("enchanted_book", ItemKind::EnchantedBook, 0.0, 0.0, 0, 0),
    // swords: attack = 1.0(player) + 3.0(baseline) + material bonus
    def!("wooden_sword", ItemKind::Sword(WOOD), 3.0, -2.4, 59, 1),
    def!("stone_sword", ItemKind::Sword(STONE_M), 3.0, -2.4, 131, 1),
    def!("iron_sword", ItemKind::Sword(IRON), 3.0, -2.4, 250, 1),
    def!(
        "diamond_sword",
        ItemKind::Sword(DIAMOND),
        3.0,
        -2.4,
        1561,
        1
    ),
    def!("golden_sword", ItemKind::Sword(GOLD), 3.0, -2.4, 32, 1),
    def!(
        "netherite_sword",
        ItemKind::Sword(NETHERITE),
        3.0,
        -2.4,
        2031,
        1
    ),
    // pickaxes: baseline 1.0, speed -2.8
    def!("wooden_pickaxe", ItemKind::Pickaxe(WOOD), 1.0, -2.8, 59, 1),
    def!(
        "stone_pickaxe",
        ItemKind::Pickaxe(STONE_M),
        1.0,
        -2.8,
        131,
        1
    ),
    def!("iron_pickaxe", ItemKind::Pickaxe(IRON), 1.0, -2.8, 250, 1),
    def!(
        "diamond_pickaxe",
        ItemKind::Pickaxe(DIAMOND),
        1.0,
        -2.8,
        1561,
        1
    ),
    def!("golden_pickaxe", ItemKind::Pickaxe(GOLD), 1.0, -2.8, 32, 1),
    def!(
        "netherite_pickaxe",
        ItemKind::Pickaxe(NETHERITE),
        1.0,
        -2.8,
        2031,
        1
    ),
    // axes (speed -3.0, baseline 5.0 in 26.1; NOTES-2 notes baseline only for
    // sword/pickaxe — axe values approximate MC data, marked TODO(research))
    def!("wooden_axe", ItemKind::Axe(WOOD), 5.0, -3.0, 59, 1),
    def!("stone_axe", ItemKind::Axe(STONE_M), 5.0, -3.0, 131, 1),
    def!("iron_axe", ItemKind::Axe(IRON), 5.0, -3.0, 250, 1),
    def!("diamond_axe", ItemKind::Axe(DIAMOND), 5.0, -3.0, 1561, 1),
    // shovels (speed -3.0, baseline 1.5, TODO(research))
    def!("iron_shovel", ItemKind::Shovel(IRON), 1.5, -3.0, 250, 1),
    def!(
        "diamond_shovel",
        ItemKind::Shovel(DIAMOND),
        1.5,
        -3.0,
        1561,
        1
    ),
    def!("wooden_shovel", ItemKind::Shovel(WOOD), 1.5, -3.0, 59, 1),
    // block items (mine -> item; place -> block)
    def!("planks", ItemKind::Block(BlockId(8)), 0.0, 0.0, 0, 0),
    def!("cobblestone", ItemKind::Block(BlockId(9)), 0.0, 0.0, 0, 0),
    def!("stone", ItemKind::Block(BlockId(1)), 0.0, 0.0, 0, 0),
    def!("iron_ore", ItemKind::Block(BlockId(1)), 0.0, 0.0, 0, 0),
    def!("diamond_ore", ItemKind::Block(BlockId(1)), 0.0, 0.0, 0, 0),
    def!("log", ItemKind::Block(BlockId(6)), 0.0, 0.0, 0, 0),
    // 创造初始快捷栏用的方块物品(32 之后追加,旧 id 序列化稳定)。
    def!("dirt", ItemKind::Block(BlockId(2)), 0.0, 0.0, 0, 0),
    def!("grass", ItemKind::Block(BlockId(3)), 0.0, 0.0, 0, 0),
    def!("sand", ItemKind::Block(BlockId(4)), 0.0, 0.0, 0, 0),
    def!("leaves", ItemKind::Block(BlockId(7)), 0.0, 0.0, 0, 0),
    // 怪物掉落杂物（drops.rs loot id 对应；64 堆、不参战、无 GUI 精灵前
    // HUD 画通用色块）。36 起追加，旧 id 序列化稳定。
    def!("rotten_flesh", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("bone", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("arrow", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("gunpowder", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("string", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("spider_eye", ItemKind::Material, 0.0, 0.0, 0, 0),
    // 弓（26.1 Items.java BOW：耐久 384、attackSpeed 无近战面板；追加在
    // 尾部保持旧 id 序列化稳定，同上注释）。
    def!("bow", ItemKind::Bow, 0.0, 0.0, 384, 0),
    // ---- 掉落经济闭环（任务板 #90）：矿石产物与杂项。26.1 loot 表语义
    //（data/minecraft/loot_table/blocks/*_ore.json 等）缺这些物品就只能
    // "宁缺勿错"不掉了；追加在尾部保持旧 id 序列化稳定。Material = 64 堆、
    // 无耐久、无 GUI 精灵（HUD 走 sprite_full 缺省回退，同怪物掉落杂物）。
    def!("raw_iron", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("raw_gold", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("raw_copper", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("redstone", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("emerald", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("flint", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("apple", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("oak_sapling", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("birch_sapling", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("spruce_sapling", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("jungle_sapling", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("acacia_sapling", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("dark_oak_sapling", ItemKind::Material, 0.0, 0.0, 0, 0),
    // 方块物品（mine->item; place->block 双向路径）：砂砾（gravel.json
    // 未触发燧石分支时掉自身）与深板岩圆石（deepslate.json）。
    def!("gravel", ItemKind::Block(BlockId(431)), 0.0, 0.0, 0, 0),
    def!(
        "cobbled_deepslate",
        ItemKind::Block(BlockId(219)),
        0.0,
        0.0,
        0,
        0
    ),
];

/// 内核名 → 物品 id（loot 表 `&'static str` 接线用；未注册 None）。
pub fn item_by_name(name: &str) -> Option<u16> {
    ITEMS.iter().position(|d| d.name == name).map(|i| i as u16)
}

pub const STICK: u16 = 0;
pub const COAL: u16 = 1;
pub const IRON_INGOT: u16 = 2;
pub const DIAMOND_ITEM: u16 = 3;
pub const LAPIS: u16 = 4;
pub const BOOK: u16 = 5;
pub const ENCHANTED_BOOK: u16 = 6;
pub const PLANKS: u16 = 26;
pub const COBBLESTONE: u16 = 27;
pub const STONE_ITEM: u16 = 28;
pub const IRON_ORE_ITEM: u16 = 29;
pub const DIAMOND_ORE_ITEM: u16 = 30;
pub const LOG: u16 = 31;
pub const DIRT_ITEM: u16 = 32;
pub const GRASS_ITEM: u16 = 33;
pub const SAND_ITEM: u16 = 34;
pub const LEAVES_ITEM: u16 = 35;
// 掉落经济（任务板 #90）：矿石产物/杂项，id 与 ITEMS 追加序一致（43 起）。
pub const RAW_IRON: u16 = 43;
pub const RAW_GOLD: u16 = 44;
pub const RAW_COPPER: u16 = 45;
pub const REDSTONE: u16 = 46;
pub const EMERALD: u16 = 47;
pub const FLINT: u16 = 48;
pub const APPLE: u16 = 49;
pub const OAK_SAPLING: u16 = 50;
pub const BIRCH_SAPLING: u16 = 51;
pub const SPRUCE_SAPLING: u16 = 52;
pub const JUNGLE_SAPLING: u16 = 53;
pub const ACACIA_SAPLING: u16 = 54;
pub const DARK_OAK_SAPLING: u16 = 55;
pub const GRAVEL_ITEM: u16 = 56;
pub const COBBLED_DEEPSLATE: u16 = 57;

// tool registry indices for recipes/tests
pub const WOODEN_SWORD_INDEX: u16 = 7;
pub const STONE_SWORD_INDEX: u16 = 8;
pub const IRON_SWORD_INDEX: u16 = 9;
pub const WOODEN_PICKAXE_INDEX: u16 = 13;
pub const STONE_PICKAXE_INDEX: u16 = 14;
pub const IRON_PICKAXE_INDEX: u16 = 15;
pub const DIAMOND_PICKAXE_INDEX: u16 = 16;
pub const WOODEN_AXE_INDEX: u16 = 19;
pub const WOODEN_SHOVEL_INDEX: u16 = 25;

#[derive(Clone, Debug, PartialEq)]
pub struct EnchStack {
    pub ench_id: u16,
    pub level: u8,
}

/// One item stack. `count` 0 = empty.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub item: u16,
    pub count: u8,
    pub damage: u16,
    pub enchants: Vec<EnchStack>,
}

impl ItemStack {
    pub fn new(item: u16, count: u8) -> Self {
        Self {
            item,
            count,
            damage: 0,
            enchants: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self::new(0, 0)
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0 || self.item == 0 && self.count == 0
    }

    pub fn def(&self) -> &'static ItemDef {
        &ITEMS[self.item as usize]
    }

    pub fn full_attack(&self) -> f32 {
        self.def().baseline_attack + f32::from(self.material_bonus())
    }

    fn material_bonus(&self) -> u8 {
        match self.def().kind {
            ItemKind::Sword(m) | ItemKind::Pickaxe(m) | ItemKind::Axe(m) | ItemKind::Shovel(m) => {
                m.attack_bonus
            }
            _ => 0,
        }
    }

    pub fn attacks_per_second(&self) -> f32 {
        4.0 + self.def().attack_speed_modifier
    }

    pub fn max_damage(&self) -> u16 {
        self.def().max_damage
    }

    pub fn enchant_value(&self) -> u8 {
        match self.def().kind {
            ItemKind::Sword(m) | ItemKind::Pickaxe(m) | ItemKind::Axe(m) | ItemKind::Shovel(m) => {
                m.enchant_value
            }
            _ => 0,
        }
    }

    /// Applies damage with Unbreaking (NOTES-2 §4: binomial skip,
    /// non-armor p = level/(level+1)). Returns true if the item broke.
    pub fn hurt(&mut self, amount: u16, rng: &mut impl FnMut() -> u32) -> bool {
        if self.max_damage() == 0 {
            return false;
        }
        let level = self
            .enchants
            .iter()
            .find(|e| e.ench_id == enchant::UNBREAKING)
            .map(|e| e.level as u32);
        let mut pending = amount;
        if let Some(l) = level {
            // Non-armor Unbreaking: each damage point is ignored with
            // probability l/(l+1) → taken with probability 1/(l+1)
            // (NOTES-2 §4, RemoveBinomial).
            let mut taken = 0u32;
            for _ in 0..pending {
                if rng().is_multiple_of(l + 1) {
                    taken += 1;
                }
            }
            pending = taken as u16;
        }
        self.damage = self.damage.saturating_add(pending);
        self.damage >= self.max_damage()
    }

    /// Mending: converts XP to durability at 2 durability per point.
    pub fn mend(&mut self, xp: u32) -> u32 {
        if self.max_damage() == 0 || self.damage == 0 {
            return xp;
        }
        let heal = (xp * 2).min(u32::from(self.damage));
        self.damage -= heal as u16;
        xp - heal.div_ceil(2)
    }
}
