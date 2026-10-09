//! Items: stacks, tool materials, durability, enchanting, anvil repair,
//! crafting. Constants from /root/mc-ref/NOTES-2.md (MC 26.1 decompiled).

pub mod anvil;
pub mod crafting;
pub mod enchant;
pub mod inventory;

pub use inventory::{HOTBAR_SLOTS, Hotbar, drop_for_block};

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
pub static ITEMS: [ItemDef; 32] = [
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
];

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

// tool registry indices for recipes/tests
pub const WOODEN_SWORD_INDEX: u16 = 7;
pub const STONE_SWORD_INDEX: u16 = 8;
pub const IRON_SWORD_INDEX: u16 = 9;
pub const WOODEN_PICKAXE_INDEX: u16 = 13;
pub const IRON_PICKAXE_INDEX: u16 = 15;

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
