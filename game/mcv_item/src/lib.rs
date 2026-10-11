//! Items: stacks, tool materials, durability, enchanting, anvil repair,
//! crafting. Constants from mc-ref/NOTES-2.md (MC 26.1 decompiled).

pub mod anvil;
pub mod bow;
pub mod crafting;
pub mod enchant;
pub mod food;
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
pub static ITEMS: [ItemDef; 1381] = [
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
    def!("wooden_pickaxe", ItemKind::Pickaxe(WOOD), 1.0, -2.8, 59, 2),
    def!(
        "stone_pickaxe",
        ItemKind::Pickaxe(STONE_M),
        1.0,
        -2.8,
        131,
        2
    ),
    def!("iron_pickaxe", ItemKind::Pickaxe(IRON), 1.0, -2.8, 250, 2),
    def!(
        "diamond_pickaxe",
        ItemKind::Pickaxe(DIAMOND),
        1.0,
        -2.8,
        1561,
        2
    ),
    def!("golden_pickaxe", ItemKind::Pickaxe(GOLD), 1.0, -2.8, 32, 2),
    def!(
        "netherite_pickaxe",
        ItemKind::Pickaxe(NETHERITE),
        1.0,
        -2.8,
        2031,
        2
    ),
    // axes (speed -3.0, baseline 5.0 in 26.1; NOTES-2 notes baseline only for
    // sword/pickaxe — axe values approximate MC data, marked TODO(research))
    def!("wooden_axe", ItemKind::Axe(WOOD), 5.0, -3.0, 59, 2),
    def!("stone_axe", ItemKind::Axe(STONE_M), 5.0, -3.0, 131, 2),
    def!("iron_axe", ItemKind::Axe(IRON), 5.0, -3.0, 250, 2),
    def!("diamond_axe", ItemKind::Axe(DIAMOND), 5.0, -3.0, 1561, 2),
    // shovels (speed -3.0, baseline 1.5, TODO(research))
    def!("iron_shovel", ItemKind::Shovel(IRON), 1.5, -3.0, 250, 2),
    def!(
        "diamond_shovel",
        ItemKind::Shovel(DIAMOND),
        1.5,
        -3.0,
        1561,
        2
    ),
    def!("wooden_shovel", ItemKind::Shovel(WOOD), 1.5, -3.0, 59, 2),
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
    // 树苗 = 方块物品（可放置；原版 oak_sapling 等是 block item，方块 id 见
    // blocks_gen——#137 W 波从 Material 转正，图标走方块图集 tile）。
    def!("oak_sapling", ItemKind::Block(BlockId(647)), 0.0, 0.0, 0, 0),
    def!(
        "birch_sapling",
        ItemKind::Block(BlockId(86)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "spruce_sapling",
        ItemKind::Block(BlockId(951)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "jungle_sapling",
        ItemKind::Block(BlockId(499)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "acacia_sapling",
        ItemKind::Block(BlockId(23)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_sapling",
        ItemKind::Block(BlockId(307)),
        0.0,
        0.0,
        0,
        0
    ),
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
    // 雪球（snow.json 单层 1 个 / snow_block.json 无 silk 4 个的掉落物；
    // 追加在尾部保持旧 id 序列化稳定，堆叠沿用 Material 全局 64，原版 16
    // 的差异 def! 无 stack 参数槽、非本轮引入的引擎约定差）。
    def!("snowball", ItemKind::Material, 0.0, 0.0, 0, 0),
    // 铁砧修料补齐（tags/item/gold_tool_materials.json = gold_ingot、
    // netherite_tool_materials.json = netherite_ingot；旧 ITEMS 缺这两件，
    // 金/下界具无修料）。追加在尾部保持旧 id 序列化稳定。
    def!("gold_ingot", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_ingot", ItemKind::Material, 0.0, 0.0, 0, 0),
    // ---- 掉落表族级补全（族 1-7）：26.1 loot_table/blocks/*.json 的自掉/
    // 转化产物物品。全部追加在尾部保持旧 id 序列化稳定；方块物品 BlockId
    // 用 blocks_gen 注册 id（mine→item、place→block 双向，同 "dirt"/"sand"
    // 先例）。未注册方块不在此列（宁缺勿错）。
    // 族 1 石头变体（granite/diorite/andesite/tuff.json：自掉）。
    def!("granite", ItemKind::Block(BlockId(427)), 0.0, 0.0, 0, 0),
    def!("diorite", ItemKind::Block(BlockId(362)), 0.0, 0.0, 0, 0),
    def!("andesite", ItemKind::Block(BlockId(37)), 0.0, 0.0, 0, 0),
    def!("tuff", ItemKind::Block(BlockId(1020)), 0.0, 0.0, 0, 0),
    // 族 2 泥土变体（coarse_dirt.json 自掉；podzol/mycelium.json 无 silk
    // 掉 dirt，自身物品供注册/拾取路径）。
    def!("coarse_dirt", ItemKind::Block(BlockId(218)), 0.0, 0.0, 0, 0),
    def!("podzol", ItemKind::Block(BlockId(741)), 0.0, 0.0, 0, 0),
    def!("mycelium", ItemKind::Block(BlockId(626)), 0.0, 0.0, 0, 0),
    // 族 3 地表（red_sand.json 自掉；clay.json 无 silk 掉 clay_ball×4）。
    def!("red_sand", ItemKind::Block(BlockId(863)), 0.0, 0.0, 0, 0),
    def!("clay_ball", ItemKind::Material, 0.0, 0.0, 0, 0),
    // 族 3/5（moss_block/mud/pumpkin.json 自掉；melon.json 掉 melon_slice
    // 3-7；glowstone.json 掉 glowstone_dust 2-4；sea_lantern.json 掉
    // prismarine_crystals 2-3）。
    def!("moss_block", ItemKind::Block(BlockId(608)), 0.0, 0.0, 0, 0),
    def!("mud", ItemKind::Block(BlockId(619)), 0.0, 0.0, 0, 0),
    def!("pumpkin", ItemKind::Block(BlockId(820)), 0.0, 0.0, 0, 0),
    def!("melon_slice", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("glowstone_dust", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("prismarine_crystals", ItemKind::Material, 0.0, 0.0, 0, 0),
    // 族 4 非橡 log 族（birch/spruce/jungle/acacia/dark_oak_log.json 自掉；
    // 树叶族五行早已注册，log 补齐族内对称）。
    def!("birch_log", ItemKind::Block(BlockId(83)), 0.0, 0.0, 0, 0),
    def!("spruce_log", ItemKind::Block(BlockId(948)), 0.0, 0.0, 0, 0),
    def!("jungle_log", ItemKind::Block(BlockId(496)), 0.0, 0.0, 0, 0),
    def!("acacia_log", ItemKind::Block(BlockId(20)), 0.0, 0.0, 0, 0),
    def!(
        "dark_oak_log",
        ItemKind::Block(BlockId(304)),
        0.0,
        0.0,
        0,
        0
    ),
    // 族 7 下界（netherrack/soul_sand/soul_soil/obsidian/magma_block/
    // basalt/ancient_debris.json 自掉；nether_gold_ore.json 无 silk 掉
    // gold_nugget 2-6；nether_quartz_ore.json 掉 quartz×ore_drops 基数 1）。
    def!("netherrack", ItemKind::Block(BlockId(639)), 0.0, 0.0, 0, 0),
    def!("soul_sand", ItemKind::Block(BlockId(935)), 0.0, 0.0, 0, 0),
    def!("soul_soil", ItemKind::Block(BlockId(936)), 0.0, 0.0, 0, 0),
    def!("obsidian", ItemKind::Block(BlockId(657)), 0.0, 0.0, 0, 0),
    def!("magma_block", ItemKind::Block(BlockId(585)), 0.0, 0.0, 0, 0),
    def!("basalt", ItemKind::Block(BlockId(69)), 0.0, 0.0, 0, 0),
    def!(
        "ancient_debris",
        ItemKind::Block(BlockId(36)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("gold_nugget", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("quartz", ItemKind::Material, 0.0, 0.0, 0, 0),
    // ---- 木族+蜂窝道具（任务板 #137 W 波）。90 起尾插保持旧 id 序列化稳定。
    // BlockId 均为 blocks_gen.inc.rs 官方方块（名字同名，映射测试
    // tests/wood_family.rs::wood_family_block_mapping 锁死）；oak 族 log/planks
    // 沿用旧物品位 LOG=31(块 6 "log")/PLANKS=26(块 8 "planks")，不新造重复。
    // 五树种 planks（oak = 旧 PLANKS=26）。
    def!(
        "spruce_planks",
        ItemKind::Block(BlockId(949)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("birch_planks", ItemKind::Block(BlockId(84)), 0.0, 0.0, 0, 0),
    def!(
        "jungle_planks",
        ItemKind::Block(BlockId(497)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "acacia_planks",
        ItemKind::Block(BlockId(21)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_planks",
        ItemKind::Block(BlockId(305)),
        0.0,
        0.0,
        0,
        0
    ),
    // 楼梯 ×6
    def!("oak_stairs", ItemKind::Block(BlockId(651)), 0.0, 0.0, 0, 0),
    def!(
        "spruce_stairs",
        ItemKind::Block(BlockId(955)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("birch_stairs", ItemKind::Block(BlockId(90)), 0.0, 0.0, 0, 0),
    def!(
        "jungle_stairs",
        ItemKind::Block(BlockId(503)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "acacia_stairs",
        ItemKind::Block(BlockId(27)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_stairs",
        ItemKind::Block(BlockId(311)),
        0.0,
        0.0,
        0,
        0
    ),
    // 台阶 ×6
    def!("oak_slab", ItemKind::Block(BlockId(650)), 0.0, 0.0, 0, 0),
    def!("spruce_slab", ItemKind::Block(BlockId(954)), 0.0, 0.0, 0, 0),
    def!("birch_slab", ItemKind::Block(BlockId(89)), 0.0, 0.0, 0, 0),
    def!("jungle_slab", ItemKind::Block(BlockId(502)), 0.0, 0.0, 0, 0),
    def!("acacia_slab", ItemKind::Block(BlockId(26)), 0.0, 0.0, 0, 0),
    def!(
        "dark_oak_slab",
        ItemKind::Block(BlockId(310)),
        0.0,
        0.0,
        0,
        0
    ),
    // 栅栏 ×6
    def!("oak_fence", ItemKind::Block(BlockId(643)), 0.0, 0.0, 0, 0),
    def!(
        "spruce_fence",
        ItemKind::Block(BlockId(944)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("birch_fence", ItemKind::Block(BlockId(79)), 0.0, 0.0, 0, 0),
    def!(
        "jungle_fence",
        ItemKind::Block(BlockId(492)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("acacia_fence", ItemKind::Block(BlockId(16)), 0.0, 0.0, 0, 0),
    def!(
        "dark_oak_fence",
        ItemKind::Block(BlockId(300)),
        0.0,
        0.0,
        0,
        0
    ),
    // 栅栏门 ×6
    def!(
        "oak_fence_gate",
        ItemKind::Block(BlockId(644)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "spruce_fence_gate",
        ItemKind::Block(BlockId(945)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "birch_fence_gate",
        ItemKind::Block(BlockId(80)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "jungle_fence_gate",
        ItemKind::Block(BlockId(493)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "acacia_fence_gate",
        ItemKind::Block(BlockId(17)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_fence_gate",
        ItemKind::Block(BlockId(301)),
        0.0,
        0.0,
        0,
        0
    ),
    // 门 ×6
    def!("oak_door", ItemKind::Block(BlockId(642)), 0.0, 0.0, 0, 0),
    def!("spruce_door", ItemKind::Block(BlockId(943)), 0.0, 0.0, 0, 0),
    def!("birch_door", ItemKind::Block(BlockId(78)), 0.0, 0.0, 0, 0),
    def!("jungle_door", ItemKind::Block(BlockId(491)), 0.0, 0.0, 0, 0),
    def!("acacia_door", ItemKind::Block(BlockId(15)), 0.0, 0.0, 0, 0),
    def!(
        "dark_oak_door",
        ItemKind::Block(BlockId(299)),
        0.0,
        0.0,
        0,
        0
    ),
    // 活板门 ×6
    def!(
        "oak_trapdoor",
        ItemKind::Block(BlockId(652)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "spruce_trapdoor",
        ItemKind::Block(BlockId(956)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "birch_trapdoor",
        ItemKind::Block(BlockId(91)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "jungle_trapdoor",
        ItemKind::Block(BlockId(504)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "acacia_trapdoor",
        ItemKind::Block(BlockId(28)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_trapdoor",
        ItemKind::Block(BlockId(312)),
        0.0,
        0.0,
        0,
        0
    ),
    // 按钮 ×6
    def!("oak_button", ItemKind::Block(BlockId(641)), 0.0, 0.0, 0, 0),
    def!(
        "spruce_button",
        ItemKind::Block(BlockId(942)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("birch_button", ItemKind::Block(BlockId(77)), 0.0, 0.0, 0, 0),
    def!(
        "jungle_button",
        ItemKind::Block(BlockId(490)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "acacia_button",
        ItemKind::Block(BlockId(14)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_button",
        ItemKind::Block(BlockId(298)),
        0.0,
        0.0,
        0,
        0
    ),
    // 压力板 ×6
    def!(
        "oak_pressure_plate",
        ItemKind::Block(BlockId(646)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "spruce_pressure_plate",
        ItemKind::Block(BlockId(950)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "birch_pressure_plate",
        ItemKind::Block(BlockId(85)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "jungle_pressure_plate",
        ItemKind::Block(BlockId(498)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "acacia_pressure_plate",
        ItemKind::Block(BlockId(22)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_pressure_plate",
        ItemKind::Block(BlockId(306)),
        0.0,
        0.0,
        0,
        0
    ),
    // 蜂窝族（honeycomb/honey_bottle/glass_bottle 为纯物品，图标走 #125
    // ITEM_SPRITES 表；方块件直连 blocks_gen）。
    def!("honeycomb", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("honey_bottle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("glass_bottle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "honeycomb_block",
        ItemKind::Block(BlockId(466)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("honey_block", ItemKind::Block(BlockId(465)), 0.0, 0.0, 0, 0),
    def!("beehive", ItemKind::Block(BlockId(72)), 0.0, 0.0, 0, 0),
    def!("bee_nest", ItemKind::Block(BlockId(71)), 0.0, 0.0, 0, 0),
    // 功能方块（crafting_table.json/chest.json 的产物；UI 已有合成屏/容器面板）。
    def!(
        "crafting_table",
        ItemKind::Block(BlockId(252)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("chest", ItemKind::Block(BlockId(198)), 0.0, 0.0, 0, 0),
    // ---- 全方块计划·物品地基波（任务板 #144）：152 起尾插保持旧 id 序列化
    // 稳定。注册名 = 26.1 官方注册名（net/minecraft/world/item/Items.java
    // / 配方 JSON / 官方 lang key 三方一致；参考树 /root/mc-ref/src-26.1 仓库外，
    // 不入库）。共 1229 件：
    // 1) 方块物品 923 件：blocks_gen 已生成方块的官方同名物品（Block kind，
    //    挖→掉自身 / 放→方块双向，图标走方块图集 tile）。oak 族 log/planks/
    //    leaves 与 grass_block/cobblestone 由旧 14 方块时代物品覆盖，不造重复；
    //    poppy/dandelion 挂旧 flower_red/flower_yellow 位（blocks_gen id 12/13）。
    // 2) 铜工具 4 件：26.1 新增铜工具族（COPPER 材料，NOTES-2 §3；
    //    recipe/copper_{sword,pickaxe,axe,shovel}.json）。
    // 3) 材料物品 302 件：26.1 配方/tag/方块 loot 引用的非方块物品
    //    （Material，64 堆、无耐久：染料 16、锭/矿产物、食物、船/矿车、
    //    盔甲、bundle/harness、陶片、纹饰模板、音乐碟等）。
    def!("copper_axe", ItemKind::Axe(COPPER), 5.0, -3.0, 190, 2),
    def!(
        "copper_pickaxe",
        ItemKind::Pickaxe(COPPER),
        1.0,
        -2.8,
        190,
        2
    ),
    def!("copper_shovel", ItemKind::Shovel(COPPER), 1.5, -3.0, 190, 2),
    def!("copper_sword", ItemKind::Sword(COPPER), 3.0, -2.4, 190, 1),
    // ---- 方块物品（blocks_gen id 序）----
    def!(
        "acacia_hanging_sign",
        ItemKind::Block(BlockId(18)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "acacia_leaves",
        ItemKind::Block(BlockId(19)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("acacia_shelf", ItemKind::Block(BlockId(24)), 0.0, 0.0, 0, 0),
    def!("acacia_sign", ItemKind::Block(BlockId(25)), 0.0, 0.0, 0, 0),
    def!("acacia_wood", ItemKind::Block(BlockId(31)), 0.0, 0.0, 0, 0),
    def!(
        "activator_rail",
        ItemKind::Block(BlockId(32)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("allium", ItemKind::Block(BlockId(33)), 0.0, 0.0, 0, 0),
    def!(
        "amethyst_block",
        ItemKind::Block(BlockId(34)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "amethyst_cluster",
        ItemKind::Block(BlockId(35)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "andesite_slab",
        ItemKind::Block(BlockId(38)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "andesite_stairs",
        ItemKind::Block(BlockId(39)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "andesite_wall",
        ItemKind::Block(BlockId(40)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("anvil", ItemKind::Block(BlockId(41)), 0.0, 0.0, 0, 0),
    def!("azalea", ItemKind::Block(BlockId(44)), 0.0, 0.0, 0, 0),
    def!(
        "azalea_leaves",
        ItemKind::Block(BlockId(45)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("azure_bluet", ItemKind::Block(BlockId(46)), 0.0, 0.0, 0, 0),
    def!("bamboo", ItemKind::Block(BlockId(47)), 0.0, 0.0, 0, 0),
    def!("bamboo_block", ItemKind::Block(BlockId(48)), 0.0, 0.0, 0, 0),
    def!(
        "bamboo_button",
        ItemKind::Block(BlockId(49)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("bamboo_door", ItemKind::Block(BlockId(50)), 0.0, 0.0, 0, 0),
    def!("bamboo_fence", ItemKind::Block(BlockId(51)), 0.0, 0.0, 0, 0),
    def!(
        "bamboo_fence_gate",
        ItemKind::Block(BlockId(52)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bamboo_hanging_sign",
        ItemKind::Block(BlockId(53)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bamboo_mosaic",
        ItemKind::Block(BlockId(54)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bamboo_mosaic_slab",
        ItemKind::Block(BlockId(55)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bamboo_mosaic_stairs",
        ItemKind::Block(BlockId(56)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bamboo_planks",
        ItemKind::Block(BlockId(57)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bamboo_pressure_plate",
        ItemKind::Block(BlockId(58)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("bamboo_shelf", ItemKind::Block(BlockId(60)), 0.0, 0.0, 0, 0),
    def!("bamboo_sign", ItemKind::Block(BlockId(61)), 0.0, 0.0, 0, 0),
    def!("bamboo_slab", ItemKind::Block(BlockId(62)), 0.0, 0.0, 0, 0),
    def!(
        "bamboo_stairs",
        ItemKind::Block(BlockId(63)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bamboo_trapdoor",
        ItemKind::Block(BlockId(64)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("barrel", ItemKind::Block(BlockId(67)), 0.0, 0.0, 0, 0),
    def!("barrier", ItemKind::Block(BlockId(68)), 0.0, 0.0, 0, 0),
    def!("beacon", ItemKind::Block(BlockId(70)), 0.0, 0.0, 0, 0),
    def!("bedrock", ItemKind::Block(BlockId(10)), 0.0, 0.0, 0, 0),
    def!(
        "beetroot_seeds",
        ItemKind::Block(BlockId(73)),
        0.0,
        0.0,
        0,
        0
    ), // 旧名 beetroots
    def!("bell", ItemKind::Block(BlockId(74)), 0.0, 0.0, 0, 0),
    def!("big_dripleaf", ItemKind::Block(BlockId(75)), 0.0, 0.0, 0, 0),
    def!(
        "birch_hanging_sign",
        ItemKind::Block(BlockId(81)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("birch_leaves", ItemKind::Block(BlockId(82)), 0.0, 0.0, 0, 0),
    def!("birch_shelf", ItemKind::Block(BlockId(87)), 0.0, 0.0, 0, 0),
    def!("birch_sign", ItemKind::Block(BlockId(88)), 0.0, 0.0, 0, 0),
    def!("birch_wood", ItemKind::Block(BlockId(94)), 0.0, 0.0, 0, 0),
    def!("black_banner", ItemKind::Block(BlockId(95)), 0.0, 0.0, 0, 0),
    def!("black_bed", ItemKind::Block(BlockId(96)), 0.0, 0.0, 0, 0),
    def!("black_candle", ItemKind::Block(BlockId(97)), 0.0, 0.0, 0, 0),
    def!("black_carpet", ItemKind::Block(BlockId(99)), 0.0, 0.0, 0, 0),
    def!(
        "black_concrete",
        ItemKind::Block(BlockId(100)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "black_concrete_powder",
        ItemKind::Block(BlockId(101)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "black_glazed_terracotta",
        ItemKind::Block(BlockId(102)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "black_shulker_box",
        ItemKind::Block(BlockId(103)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "black_stained_glass",
        ItemKind::Block(BlockId(104)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "black_stained_glass_pane",
        ItemKind::Block(BlockId(105)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "black_terracotta",
        ItemKind::Block(BlockId(106)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("black_wool", ItemKind::Block(BlockId(108)), 0.0, 0.0, 0, 0),
    def!("blackstone", ItemKind::Block(BlockId(109)), 0.0, 0.0, 0, 0),
    def!(
        "blackstone_slab",
        ItemKind::Block(BlockId(110)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "blackstone_stairs",
        ItemKind::Block(BlockId(111)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "blackstone_wall",
        ItemKind::Block(BlockId(112)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "blast_furnace",
        ItemKind::Block(BlockId(113)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("blue_banner", ItemKind::Block(BlockId(114)), 0.0, 0.0, 0, 0),
    def!("blue_bed", ItemKind::Block(BlockId(115)), 0.0, 0.0, 0, 0),
    def!("blue_candle", ItemKind::Block(BlockId(116)), 0.0, 0.0, 0, 0),
    def!("blue_carpet", ItemKind::Block(BlockId(118)), 0.0, 0.0, 0, 0),
    def!(
        "blue_concrete",
        ItemKind::Block(BlockId(119)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "blue_concrete_powder",
        ItemKind::Block(BlockId(120)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "blue_glazed_terracotta",
        ItemKind::Block(BlockId(121)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("blue_ice", ItemKind::Block(BlockId(122)), 0.0, 0.0, 0, 0),
    def!("blue_orchid", ItemKind::Block(BlockId(123)), 0.0, 0.0, 0, 0),
    def!(
        "blue_shulker_box",
        ItemKind::Block(BlockId(124)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "blue_stained_glass",
        ItemKind::Block(BlockId(125)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "blue_stained_glass_pane",
        ItemKind::Block(BlockId(126)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "blue_terracotta",
        ItemKind::Block(BlockId(127)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("blue_wool", ItemKind::Block(BlockId(129)), 0.0, 0.0, 0, 0),
    def!("bone_block", ItemKind::Block(BlockId(130)), 0.0, 0.0, 0, 0),
    def!("bookshelf", ItemKind::Block(BlockId(131)), 0.0, 0.0, 0, 0),
    def!("brain_coral", ItemKind::Block(BlockId(132)), 0.0, 0.0, 0, 0),
    def!(
        "brain_coral_block",
        ItemKind::Block(BlockId(133)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brain_coral_fan",
        ItemKind::Block(BlockId(134)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brewing_stand",
        ItemKind::Block(BlockId(136)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("brick_slab", ItemKind::Block(BlockId(137)), 0.0, 0.0, 0, 0),
    def!(
        "brick_stairs",
        ItemKind::Block(BlockId(138)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("brick_wall", ItemKind::Block(BlockId(139)), 0.0, 0.0, 0, 0),
    def!("bricks", ItemKind::Block(BlockId(140)), 0.0, 0.0, 0, 0),
    def!(
        "brown_banner",
        ItemKind::Block(BlockId(141)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("brown_bed", ItemKind::Block(BlockId(142)), 0.0, 0.0, 0, 0),
    def!(
        "brown_candle",
        ItemKind::Block(BlockId(143)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_carpet",
        ItemKind::Block(BlockId(145)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_concrete",
        ItemKind::Block(BlockId(146)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_concrete_powder",
        ItemKind::Block(BlockId(147)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_glazed_terracotta",
        ItemKind::Block(BlockId(148)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_mushroom",
        ItemKind::Block(BlockId(149)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_mushroom_block",
        ItemKind::Block(BlockId(150)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_shulker_box",
        ItemKind::Block(BlockId(151)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_stained_glass",
        ItemKind::Block(BlockId(152)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_stained_glass_pane",
        ItemKind::Block(BlockId(153)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "brown_terracotta",
        ItemKind::Block(BlockId(154)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("brown_wool", ItemKind::Block(BlockId(156)), 0.0, 0.0, 0, 0),
    def!(
        "bubble_coral",
        ItemKind::Block(BlockId(158)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bubble_coral_block",
        ItemKind::Block(BlockId(159)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "bubble_coral_fan",
        ItemKind::Block(BlockId(160)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "budding_amethyst",
        ItemKind::Block(BlockId(162)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("bush", ItemKind::Block(BlockId(163)), 0.0, 0.0, 0, 0),
    def!("cactus", ItemKind::Block(BlockId(164)), 0.0, 0.0, 0, 0),
    def!(
        "cactus_flower",
        ItemKind::Block(BlockId(165)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cake", ItemKind::Block(BlockId(166)), 0.0, 0.0, 0, 0),
    def!("calcite", ItemKind::Block(BlockId(167)), 0.0, 0.0, 0, 0),
    def!(
        "calibrated_sculk_sensor",
        ItemKind::Block(BlockId(168)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("campfire", ItemKind::Block(BlockId(169)), 0.0, 0.0, 0, 0),
    def!("candle", ItemKind::Block(BlockId(170)), 0.0, 0.0, 0, 0),
    def!("carrot", ItemKind::Block(BlockId(172)), 0.0, 0.0, 0, 0), // 旧名 carrots
    def!(
        "cartography_table",
        ItemKind::Block(BlockId(173)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "carved_pumpkin",
        ItemKind::Block(BlockId(174)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cauldron", ItemKind::Block(BlockId(175)), 0.0, 0.0, 0, 0),
    def!(
        "chain_command_block",
        ItemKind::Block(BlockId(179)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cherry_button",
        ItemKind::Block(BlockId(180)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cherry_door", ItemKind::Block(BlockId(181)), 0.0, 0.0, 0, 0),
    def!(
        "cherry_fence",
        ItemKind::Block(BlockId(182)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cherry_fence_gate",
        ItemKind::Block(BlockId(183)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cherry_hanging_sign",
        ItemKind::Block(BlockId(184)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cherry_leaves",
        ItemKind::Block(BlockId(185)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cherry_log", ItemKind::Block(BlockId(186)), 0.0, 0.0, 0, 0),
    def!(
        "cherry_planks",
        ItemKind::Block(BlockId(187)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cherry_pressure_plate",
        ItemKind::Block(BlockId(188)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cherry_sapling",
        ItemKind::Block(BlockId(189)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cherry_shelf",
        ItemKind::Block(BlockId(190)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cherry_sign", ItemKind::Block(BlockId(191)), 0.0, 0.0, 0, 0),
    def!("cherry_slab", ItemKind::Block(BlockId(192)), 0.0, 0.0, 0, 0),
    def!(
        "cherry_stairs",
        ItemKind::Block(BlockId(193)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cherry_trapdoor",
        ItemKind::Block(BlockId(194)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cherry_wood", ItemKind::Block(BlockId(197)), 0.0, 0.0, 0, 0),
    def!(
        "chipped_anvil",
        ItemKind::Block(BlockId(199)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_bookshelf",
        ItemKind::Block(BlockId(200)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_copper",
        ItemKind::Block(BlockId(201)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_deepslate",
        ItemKind::Block(BlockId(202)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_nether_bricks",
        ItemKind::Block(BlockId(203)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_polished_blackstone",
        ItemKind::Block(BlockId(204)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_quartz_block",
        ItemKind::Block(BlockId(205)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_red_sandstone",
        ItemKind::Block(BlockId(206)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_resin_bricks",
        ItemKind::Block(BlockId(207)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_sandstone",
        ItemKind::Block(BlockId(208)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_stone_bricks",
        ItemKind::Block(BlockId(209)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_tuff",
        ItemKind::Block(BlockId(210)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chiseled_tuff_bricks",
        ItemKind::Block(BlockId(211)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chorus_flower",
        ItemKind::Block(BlockId(212)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "chorus_plant",
        ItemKind::Block(BlockId(213)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("clay", ItemKind::Block(BlockId(214)), 0.0, 0.0, 0, 0),
    def!(
        "closed_eyeblossom",
        ItemKind::Block(BlockId(215)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("coal_block", ItemKind::Block(BlockId(216)), 0.0, 0.0, 0, 0),
    def!("coal_ore", ItemKind::Block(BlockId(217)), 0.0, 0.0, 0, 0),
    def!(
        "cobbled_deepslate_slab",
        ItemKind::Block(BlockId(220)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cobbled_deepslate_stairs",
        ItemKind::Block(BlockId(221)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cobbled_deepslate_wall",
        ItemKind::Block(BlockId(222)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cobblestone_slab",
        ItemKind::Block(BlockId(223)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cobblestone_stairs",
        ItemKind::Block(BlockId(224)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cobblestone_wall",
        ItemKind::Block(BlockId(225)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cobweb", ItemKind::Block(BlockId(226)), 0.0, 0.0, 0, 0),
    def!("cocoa_beans", ItemKind::Block(BlockId(227)), 0.0, 0.0, 0, 0), // 旧名 cocoa
    def!(
        "command_block",
        ItemKind::Block(BlockId(228)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("comparator", ItemKind::Block(BlockId(229)), 0.0, 0.0, 0, 0),
    def!("composter", ItemKind::Block(BlockId(230)), 0.0, 0.0, 0, 0),
    def!("conduit", ItemKind::Block(BlockId(231)), 0.0, 0.0, 0, 0),
    def!("copper_bars", ItemKind::Block(BlockId(232)), 0.0, 0.0, 0, 0),
    def!(
        "copper_block",
        ItemKind::Block(BlockId(233)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("copper_bulb", ItemKind::Block(BlockId(234)), 0.0, 0.0, 0, 0),
    def!(
        "copper_chain",
        ItemKind::Block(BlockId(235)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "copper_chest",
        ItemKind::Block(BlockId(236)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("copper_door", ItemKind::Block(BlockId(237)), 0.0, 0.0, 0, 0),
    def!(
        "copper_golem_statue",
        ItemKind::Block(BlockId(238)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "copper_grate",
        ItemKind::Block(BlockId(239)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "copper_lantern",
        ItemKind::Block(BlockId(240)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("copper_ore", ItemKind::Block(BlockId(241)), 0.0, 0.0, 0, 0),
    def!(
        "copper_torch",
        ItemKind::Block(BlockId(242)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "copper_trapdoor",
        ItemKind::Block(BlockId(243)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cornflower", ItemKind::Block(BlockId(245)), 0.0, 0.0, 0, 0),
    def!(
        "cracked_deepslate_bricks",
        ItemKind::Block(BlockId(246)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cracked_deepslate_tiles",
        ItemKind::Block(BlockId(247)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cracked_nether_bricks",
        ItemKind::Block(BlockId(248)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cracked_polished_blackstone_bricks",
        ItemKind::Block(BlockId(249)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cracked_stone_bricks",
        ItemKind::Block(BlockId(250)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("crafter", ItemKind::Block(BlockId(251)), 0.0, 0.0, 0, 0),
    def!(
        "creaking_heart",
        ItemKind::Block(BlockId(253)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "creeper_head",
        ItemKind::Block(BlockId(254)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_button",
        ItemKind::Block(BlockId(256)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_door",
        ItemKind::Block(BlockId(257)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_fence",
        ItemKind::Block(BlockId(258)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_fence_gate",
        ItemKind::Block(BlockId(259)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_fungus",
        ItemKind::Block(BlockId(260)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_hanging_sign",
        ItemKind::Block(BlockId(261)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_hyphae",
        ItemKind::Block(BlockId(262)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_nylium",
        ItemKind::Block(BlockId(263)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_planks",
        ItemKind::Block(BlockId(264)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_pressure_plate",
        ItemKind::Block(BlockId(265)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_roots",
        ItemKind::Block(BlockId(266)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_shelf",
        ItemKind::Block(BlockId(267)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_sign",
        ItemKind::Block(BlockId(268)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_slab",
        ItemKind::Block(BlockId(269)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_stairs",
        ItemKind::Block(BlockId(270)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_stem",
        ItemKind::Block(BlockId(271)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crimson_trapdoor",
        ItemKind::Block(BlockId(272)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "crying_obsidian",
        ItemKind::Block(BlockId(275)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cut_copper", ItemKind::Block(BlockId(276)), 0.0, 0.0, 0, 0),
    def!(
        "cut_copper_slab",
        ItemKind::Block(BlockId(277)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cut_copper_stairs",
        ItemKind::Block(BlockId(278)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cut_red_sandstone",
        ItemKind::Block(BlockId(279)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cut_red_sandstone_slab",
        ItemKind::Block(BlockId(280)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cut_sandstone",
        ItemKind::Block(BlockId(281)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cut_sandstone_slab",
        ItemKind::Block(BlockId(282)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cyan_banner", ItemKind::Block(BlockId(283)), 0.0, 0.0, 0, 0),
    def!("cyan_bed", ItemKind::Block(BlockId(284)), 0.0, 0.0, 0, 0),
    def!("cyan_candle", ItemKind::Block(BlockId(285)), 0.0, 0.0, 0, 0),
    def!("cyan_carpet", ItemKind::Block(BlockId(287)), 0.0, 0.0, 0, 0),
    def!(
        "cyan_concrete",
        ItemKind::Block(BlockId(288)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cyan_concrete_powder",
        ItemKind::Block(BlockId(289)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cyan_glazed_terracotta",
        ItemKind::Block(BlockId(290)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cyan_shulker_box",
        ItemKind::Block(BlockId(291)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cyan_stained_glass",
        ItemKind::Block(BlockId(292)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cyan_stained_glass_pane",
        ItemKind::Block(BlockId(293)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "cyan_terracotta",
        ItemKind::Block(BlockId(294)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("cyan_wool", ItemKind::Block(BlockId(296)), 0.0, 0.0, 0, 0),
    def!(
        "damaged_anvil",
        ItemKind::Block(BlockId(297)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("dandelion", ItemKind::Block(BlockId(13)), 0.0, 0.0, 0, 0), // 旧名 flower_yellow
    def!(
        "dark_oak_hanging_sign",
        ItemKind::Block(BlockId(302)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_leaves",
        ItemKind::Block(BlockId(303)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_shelf",
        ItemKind::Block(BlockId(308)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_sign",
        ItemKind::Block(BlockId(309)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_oak_wood",
        ItemKind::Block(BlockId(315)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_prismarine",
        ItemKind::Block(BlockId(316)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_prismarine_slab",
        ItemKind::Block(BlockId(317)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dark_prismarine_stairs",
        ItemKind::Block(BlockId(318)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "daylight_detector",
        ItemKind::Block(BlockId(319)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_brain_coral",
        ItemKind::Block(BlockId(320)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_brain_coral_block",
        ItemKind::Block(BlockId(321)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_brain_coral_fan",
        ItemKind::Block(BlockId(322)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_bubble_coral",
        ItemKind::Block(BlockId(324)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_bubble_coral_block",
        ItemKind::Block(BlockId(325)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_bubble_coral_fan",
        ItemKind::Block(BlockId(326)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("dead_bush", ItemKind::Block(BlockId(328)), 0.0, 0.0, 0, 0),
    def!(
        "dead_fire_coral",
        ItemKind::Block(BlockId(329)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_fire_coral_block",
        ItemKind::Block(BlockId(330)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_fire_coral_fan",
        ItemKind::Block(BlockId(331)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_horn_coral",
        ItemKind::Block(BlockId(333)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_horn_coral_block",
        ItemKind::Block(BlockId(334)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_horn_coral_fan",
        ItemKind::Block(BlockId(335)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_tube_coral",
        ItemKind::Block(BlockId(337)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_tube_coral_block",
        ItemKind::Block(BlockId(338)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dead_tube_coral_fan",
        ItemKind::Block(BlockId(339)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "decorated_pot",
        ItemKind::Block(BlockId(341)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("deepslate", ItemKind::Block(BlockId(342)), 0.0, 0.0, 0, 0),
    def!(
        "deepslate_brick_slab",
        ItemKind::Block(BlockId(343)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_brick_stairs",
        ItemKind::Block(BlockId(344)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_brick_wall",
        ItemKind::Block(BlockId(345)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_bricks",
        ItemKind::Block(BlockId(346)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_coal_ore",
        ItemKind::Block(BlockId(347)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_copper_ore",
        ItemKind::Block(BlockId(348)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_diamond_ore",
        ItemKind::Block(BlockId(349)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_emerald_ore",
        ItemKind::Block(BlockId(350)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_gold_ore",
        ItemKind::Block(BlockId(351)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_iron_ore",
        ItemKind::Block(BlockId(352)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_lapis_ore",
        ItemKind::Block(BlockId(353)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_redstone_ore",
        ItemKind::Block(BlockId(354)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_tile_slab",
        ItemKind::Block(BlockId(355)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_tile_stairs",
        ItemKind::Block(BlockId(356)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_tile_wall",
        ItemKind::Block(BlockId(357)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "deepslate_tiles",
        ItemKind::Block(BlockId(358)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "detector_rail",
        ItemKind::Block(BlockId(359)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "diamond_block",
        ItemKind::Block(BlockId(360)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "diorite_slab",
        ItemKind::Block(BlockId(363)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "diorite_stairs",
        ItemKind::Block(BlockId(364)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "diorite_wall",
        ItemKind::Block(BlockId(365)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("dirt_path", ItemKind::Block(BlockId(366)), 0.0, 0.0, 0, 0),
    def!("dispenser", ItemKind::Block(BlockId(367)), 0.0, 0.0, 0, 0),
    def!("dragon_egg", ItemKind::Block(BlockId(368)), 0.0, 0.0, 0, 0),
    def!("dragon_head", ItemKind::Block(BlockId(369)), 0.0, 0.0, 0, 0),
    def!("dried_ghast", ItemKind::Block(BlockId(371)), 0.0, 0.0, 0, 0),
    def!(
        "dried_kelp_block",
        ItemKind::Block(BlockId(372)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "dripstone_block",
        ItemKind::Block(BlockId(373)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("dropper", ItemKind::Block(BlockId(374)), 0.0, 0.0, 0, 0),
    def!(
        "emerald_block",
        ItemKind::Block(BlockId(375)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("emerald_ore", ItemKind::Block(BlockId(376)), 0.0, 0.0, 0, 0),
    def!(
        "enchanting_table",
        ItemKind::Block(BlockId(377)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "end_portal_frame",
        ItemKind::Block(BlockId(380)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("end_rod", ItemKind::Block(BlockId(381)), 0.0, 0.0, 0, 0),
    def!("end_stone", ItemKind::Block(BlockId(382)), 0.0, 0.0, 0, 0),
    def!(
        "end_stone_brick_slab",
        ItemKind::Block(BlockId(383)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "end_stone_brick_stairs",
        ItemKind::Block(BlockId(384)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "end_stone_brick_wall",
        ItemKind::Block(BlockId(385)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "end_stone_bricks",
        ItemKind::Block(BlockId(386)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("ender_chest", ItemKind::Block(BlockId(387)), 0.0, 0.0, 0, 0),
    def!(
        "exposed_chiseled_copper",
        ItemKind::Block(BlockId(388)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper",
        ItemKind::Block(BlockId(389)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_bars",
        ItemKind::Block(BlockId(390)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_bulb",
        ItemKind::Block(BlockId(391)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_chain",
        ItemKind::Block(BlockId(392)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_chest",
        ItemKind::Block(BlockId(393)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_door",
        ItemKind::Block(BlockId(394)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_golem_statue",
        ItemKind::Block(BlockId(395)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_grate",
        ItemKind::Block(BlockId(396)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_lantern",
        ItemKind::Block(BlockId(397)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_copper_trapdoor",
        ItemKind::Block(BlockId(398)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_cut_copper",
        ItemKind::Block(BlockId(399)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_cut_copper_slab",
        ItemKind::Block(BlockId(400)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_cut_copper_stairs",
        ItemKind::Block(BlockId(401)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "exposed_lightning_rod",
        ItemKind::Block(BlockId(402)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("farmland", ItemKind::Block(BlockId(403)), 0.0, 0.0, 0, 0),
    def!("fern", ItemKind::Block(BlockId(404)), 0.0, 0.0, 0, 0),
    def!("fire_coral", ItemKind::Block(BlockId(406)), 0.0, 0.0, 0, 0),
    def!(
        "fire_coral_block",
        ItemKind::Block(BlockId(407)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "fire_coral_fan",
        ItemKind::Block(BlockId(408)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "firefly_bush",
        ItemKind::Block(BlockId(410)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "fletching_table",
        ItemKind::Block(BlockId(411)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("flower_pot", ItemKind::Block(BlockId(412)), 0.0, 0.0, 0, 0),
    def!(
        "flowering_azalea",
        ItemKind::Block(BlockId(413)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "flowering_azalea_leaves",
        ItemKind::Block(BlockId(414)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("frogspawn", ItemKind::Block(BlockId(415)), 0.0, 0.0, 0, 0),
    def!("furnace", ItemKind::Block(BlockId(417)), 0.0, 0.0, 0, 0),
    def!(
        "gilded_blackstone",
        ItemKind::Block(BlockId(418)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("glass", ItemKind::Block(BlockId(419)), 0.0, 0.0, 0, 0),
    def!("glass_pane", ItemKind::Block(BlockId(420)), 0.0, 0.0, 0, 0),
    def!(
        "glow_berries",
        ItemKind::Block(BlockId(177)),
        0.0,
        0.0,
        0,
        0
    ), // 旧名 cave_vines
    def!("glow_lichen", ItemKind::Block(BlockId(422)), 0.0, 0.0, 0, 0),
    def!("glowstone", ItemKind::Block(BlockId(423)), 0.0, 0.0, 0, 0),
    def!("gold_block", ItemKind::Block(BlockId(424)), 0.0, 0.0, 0, 0),
    def!("gold_ore", ItemKind::Block(BlockId(425)), 0.0, 0.0, 0, 0),
    def!(
        "golden_dandelion",
        ItemKind::Block(BlockId(426)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "granite_slab",
        ItemKind::Block(BlockId(428)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "granite_stairs",
        ItemKind::Block(BlockId(429)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "granite_wall",
        ItemKind::Block(BlockId(430)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("gray_banner", ItemKind::Block(BlockId(432)), 0.0, 0.0, 0, 0),
    def!("gray_bed", ItemKind::Block(BlockId(433)), 0.0, 0.0, 0, 0),
    def!("gray_candle", ItemKind::Block(BlockId(434)), 0.0, 0.0, 0, 0),
    def!("gray_carpet", ItemKind::Block(BlockId(436)), 0.0, 0.0, 0, 0),
    def!(
        "gray_concrete",
        ItemKind::Block(BlockId(437)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "gray_concrete_powder",
        ItemKind::Block(BlockId(438)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "gray_glazed_terracotta",
        ItemKind::Block(BlockId(439)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "gray_shulker_box",
        ItemKind::Block(BlockId(440)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "gray_stained_glass",
        ItemKind::Block(BlockId(441)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "gray_stained_glass_pane",
        ItemKind::Block(BlockId(442)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "gray_terracotta",
        ItemKind::Block(BlockId(443)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("gray_wool", ItemKind::Block(BlockId(445)), 0.0, 0.0, 0, 0),
    def!(
        "green_banner",
        ItemKind::Block(BlockId(446)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("green_bed", ItemKind::Block(BlockId(447)), 0.0, 0.0, 0, 0),
    def!(
        "green_candle",
        ItemKind::Block(BlockId(448)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "green_carpet",
        ItemKind::Block(BlockId(450)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "green_concrete",
        ItemKind::Block(BlockId(451)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "green_concrete_powder",
        ItemKind::Block(BlockId(452)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "green_glazed_terracotta",
        ItemKind::Block(BlockId(453)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "green_shulker_box",
        ItemKind::Block(BlockId(454)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "green_stained_glass",
        ItemKind::Block(BlockId(455)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "green_stained_glass_pane",
        ItemKind::Block(BlockId(456)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "green_terracotta",
        ItemKind::Block(BlockId(457)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("green_wool", ItemKind::Block(BlockId(459)), 0.0, 0.0, 0, 0),
    def!("grindstone", ItemKind::Block(BlockId(460)), 0.0, 0.0, 0, 0),
    def!(
        "hanging_roots",
        ItemKind::Block(BlockId(461)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("hay_block", ItemKind::Block(BlockId(462)), 0.0, 0.0, 0, 0),
    def!("heavy_core", ItemKind::Block(BlockId(463)), 0.0, 0.0, 0, 0),
    def!(
        "heavy_weighted_pressure_plate",
        ItemKind::Block(BlockId(464)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("hopper", ItemKind::Block(BlockId(467)), 0.0, 0.0, 0, 0),
    def!("horn_coral", ItemKind::Block(BlockId(468)), 0.0, 0.0, 0, 0),
    def!(
        "horn_coral_block",
        ItemKind::Block(BlockId(469)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "horn_coral_fan",
        ItemKind::Block(BlockId(470)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("ice", ItemKind::Block(BlockId(472)), 0.0, 0.0, 0, 0),
    def!(
        "infested_chiseled_stone_bricks",
        ItemKind::Block(BlockId(473)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "infested_cobblestone",
        ItemKind::Block(BlockId(474)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "infested_cracked_stone_bricks",
        ItemKind::Block(BlockId(475)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "infested_deepslate",
        ItemKind::Block(BlockId(476)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "infested_mossy_stone_bricks",
        ItemKind::Block(BlockId(477)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "infested_stone",
        ItemKind::Block(BlockId(478)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "infested_stone_bricks",
        ItemKind::Block(BlockId(479)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("iron_bars", ItemKind::Block(BlockId(480)), 0.0, 0.0, 0, 0),
    def!("iron_block", ItemKind::Block(BlockId(481)), 0.0, 0.0, 0, 0),
    def!("iron_chain", ItemKind::Block(BlockId(482)), 0.0, 0.0, 0, 0),
    def!("iron_door", ItemKind::Block(BlockId(483)), 0.0, 0.0, 0, 0),
    def!(
        "iron_trapdoor",
        ItemKind::Block(BlockId(485)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "jack_o_lantern",
        ItemKind::Block(BlockId(487)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("jigsaw", ItemKind::Block(BlockId(488)), 0.0, 0.0, 0, 0),
    def!("jukebox", ItemKind::Block(BlockId(489)), 0.0, 0.0, 0, 0),
    def!(
        "jungle_hanging_sign",
        ItemKind::Block(BlockId(494)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "jungle_leaves",
        ItemKind::Block(BlockId(495)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "jungle_shelf",
        ItemKind::Block(BlockId(500)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("jungle_sign", ItemKind::Block(BlockId(501)), 0.0, 0.0, 0, 0),
    def!("jungle_wood", ItemKind::Block(BlockId(507)), 0.0, 0.0, 0, 0),
    def!("kelp", ItemKind::Block(BlockId(508)), 0.0, 0.0, 0, 0),
    def!("ladder", ItemKind::Block(BlockId(510)), 0.0, 0.0, 0, 0),
    def!("lantern", ItemKind::Block(BlockId(511)), 0.0, 0.0, 0, 0),
    def!("lapis_block", ItemKind::Block(BlockId(512)), 0.0, 0.0, 0, 0),
    def!("lapis_ore", ItemKind::Block(BlockId(513)), 0.0, 0.0, 0, 0),
    def!(
        "large_amethyst_bud",
        ItemKind::Block(BlockId(514)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("large_fern", ItemKind::Block(BlockId(515)), 0.0, 0.0, 0, 0),
    def!("leaf_litter", ItemKind::Block(BlockId(518)), 0.0, 0.0, 0, 0),
    def!("lectern", ItemKind::Block(BlockId(519)), 0.0, 0.0, 0, 0),
    def!("lever", ItemKind::Block(BlockId(520)), 0.0, 0.0, 0, 0),
    def!("light", ItemKind::Block(BlockId(521)), 0.0, 0.0, 0, 0),
    def!(
        "light_blue_banner",
        ItemKind::Block(BlockId(522)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_bed",
        ItemKind::Block(BlockId(523)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_candle",
        ItemKind::Block(BlockId(524)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_carpet",
        ItemKind::Block(BlockId(526)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_concrete",
        ItemKind::Block(BlockId(527)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_concrete_powder",
        ItemKind::Block(BlockId(528)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_glazed_terracotta",
        ItemKind::Block(BlockId(529)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_shulker_box",
        ItemKind::Block(BlockId(530)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_stained_glass",
        ItemKind::Block(BlockId(531)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_stained_glass_pane",
        ItemKind::Block(BlockId(532)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_terracotta",
        ItemKind::Block(BlockId(533)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_blue_wool",
        ItemKind::Block(BlockId(535)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_banner",
        ItemKind::Block(BlockId(536)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_bed",
        ItemKind::Block(BlockId(537)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_candle",
        ItemKind::Block(BlockId(538)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_carpet",
        ItemKind::Block(BlockId(540)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_concrete",
        ItemKind::Block(BlockId(541)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_concrete_powder",
        ItemKind::Block(BlockId(542)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_glazed_terracotta",
        ItemKind::Block(BlockId(543)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_shulker_box",
        ItemKind::Block(BlockId(544)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_stained_glass",
        ItemKind::Block(BlockId(545)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_stained_glass_pane",
        ItemKind::Block(BlockId(546)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_terracotta",
        ItemKind::Block(BlockId(547)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_gray_wool",
        ItemKind::Block(BlockId(549)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "light_weighted_pressure_plate",
        ItemKind::Block(BlockId(550)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "lightning_rod",
        ItemKind::Block(BlockId(551)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("lilac", ItemKind::Block(BlockId(552)), 0.0, 0.0, 0, 0),
    def!(
        "lily_of_the_valley",
        ItemKind::Block(BlockId(553)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("lily_pad", ItemKind::Block(BlockId(554)), 0.0, 0.0, 0, 0),
    def!("lime_banner", ItemKind::Block(BlockId(555)), 0.0, 0.0, 0, 0),
    def!("lime_bed", ItemKind::Block(BlockId(556)), 0.0, 0.0, 0, 0),
    def!("lime_candle", ItemKind::Block(BlockId(557)), 0.0, 0.0, 0, 0),
    def!("lime_carpet", ItemKind::Block(BlockId(559)), 0.0, 0.0, 0, 0),
    def!(
        "lime_concrete",
        ItemKind::Block(BlockId(560)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "lime_concrete_powder",
        ItemKind::Block(BlockId(561)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "lime_glazed_terracotta",
        ItemKind::Block(BlockId(562)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "lime_shulker_box",
        ItemKind::Block(BlockId(563)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "lime_stained_glass",
        ItemKind::Block(BlockId(564)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "lime_stained_glass_pane",
        ItemKind::Block(BlockId(565)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "lime_terracotta",
        ItemKind::Block(BlockId(566)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("lime_wool", ItemKind::Block(BlockId(568)), 0.0, 0.0, 0, 0),
    def!("lodestone", ItemKind::Block(BlockId(569)), 0.0, 0.0, 0, 0),
    def!("loom", ItemKind::Block(BlockId(570)), 0.0, 0.0, 0, 0),
    def!(
        "magenta_banner",
        ItemKind::Block(BlockId(571)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("magenta_bed", ItemKind::Block(BlockId(572)), 0.0, 0.0, 0, 0),
    def!(
        "magenta_candle",
        ItemKind::Block(BlockId(573)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_carpet",
        ItemKind::Block(BlockId(575)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_concrete",
        ItemKind::Block(BlockId(576)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_concrete_powder",
        ItemKind::Block(BlockId(577)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_glazed_terracotta",
        ItemKind::Block(BlockId(578)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_shulker_box",
        ItemKind::Block(BlockId(579)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_stained_glass",
        ItemKind::Block(BlockId(580)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_stained_glass_pane",
        ItemKind::Block(BlockId(581)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_terracotta",
        ItemKind::Block(BlockId(582)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "magenta_wool",
        ItemKind::Block(BlockId(584)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_button",
        ItemKind::Block(BlockId(586)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_door",
        ItemKind::Block(BlockId(587)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_fence",
        ItemKind::Block(BlockId(588)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_fence_gate",
        ItemKind::Block(BlockId(589)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_hanging_sign",
        ItemKind::Block(BlockId(590)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_leaves",
        ItemKind::Block(BlockId(591)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_log",
        ItemKind::Block(BlockId(592)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_planks",
        ItemKind::Block(BlockId(593)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_pressure_plate",
        ItemKind::Block(BlockId(594)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_propagule",
        ItemKind::Block(BlockId(595)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_roots",
        ItemKind::Block(BlockId(596)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_shelf",
        ItemKind::Block(BlockId(597)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_sign",
        ItemKind::Block(BlockId(598)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_slab",
        ItemKind::Block(BlockId(599)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_stairs",
        ItemKind::Block(BlockId(600)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_trapdoor",
        ItemKind::Block(BlockId(601)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mangrove_wood",
        ItemKind::Block(BlockId(604)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "medium_amethyst_bud",
        ItemKind::Block(BlockId(605)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("melon", ItemKind::Block(BlockId(606)), 0.0, 0.0, 0, 0),
    def!("melon_seeds", ItemKind::Block(BlockId(607)), 0.0, 0.0, 0, 0), // 旧名 melon_stem
    def!("moss_carpet", ItemKind::Block(BlockId(609)), 0.0, 0.0, 0, 0),
    def!(
        "mossy_cobblestone",
        ItemKind::Block(BlockId(610)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mossy_cobblestone_slab",
        ItemKind::Block(BlockId(611)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mossy_cobblestone_stairs",
        ItemKind::Block(BlockId(612)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mossy_cobblestone_wall",
        ItemKind::Block(BlockId(613)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mossy_stone_brick_slab",
        ItemKind::Block(BlockId(614)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mossy_stone_brick_stairs",
        ItemKind::Block(BlockId(615)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mossy_stone_brick_wall",
        ItemKind::Block(BlockId(616)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mossy_stone_bricks",
        ItemKind::Block(BlockId(617)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mud_brick_slab",
        ItemKind::Block(BlockId(620)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mud_brick_stairs",
        ItemKind::Block(BlockId(621)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mud_brick_wall",
        ItemKind::Block(BlockId(622)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("mud_bricks", ItemKind::Block(BlockId(623)), 0.0, 0.0, 0, 0),
    def!(
        "muddy_mangrove_roots",
        ItemKind::Block(BlockId(624)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "mushroom_stem",
        ItemKind::Block(BlockId(625)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "nether_brick_fence",
        ItemKind::Block(BlockId(627)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "nether_brick_slab",
        ItemKind::Block(BlockId(628)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "nether_brick_stairs",
        ItemKind::Block(BlockId(629)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "nether_brick_wall",
        ItemKind::Block(BlockId(630)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "nether_bricks",
        ItemKind::Block(BlockId(631)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "nether_gold_ore",
        ItemKind::Block(BlockId(632)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "nether_quartz_ore",
        ItemKind::Block(BlockId(634)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "nether_sprouts",
        ItemKind::Block(BlockId(635)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("nether_wart", ItemKind::Block(BlockId(636)), 0.0, 0.0, 0, 0),
    def!(
        "nether_wart_block",
        ItemKind::Block(BlockId(637)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "netherite_block",
        ItemKind::Block(BlockId(638)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("note_block", ItemKind::Block(BlockId(640)), 0.0, 0.0, 0, 0),
    def!(
        "oak_hanging_sign",
        ItemKind::Block(BlockId(645)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("oak_shelf", ItemKind::Block(BlockId(648)), 0.0, 0.0, 0, 0),
    def!("oak_sign", ItemKind::Block(BlockId(649)), 0.0, 0.0, 0, 0),
    def!("oak_wood", ItemKind::Block(BlockId(655)), 0.0, 0.0, 0, 0),
    def!("observer", ItemKind::Block(BlockId(656)), 0.0, 0.0, 0, 0),
    def!(
        "ochre_froglight",
        ItemKind::Block(BlockId(658)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "open_eyeblossom",
        ItemKind::Block(BlockId(659)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_banner",
        ItemKind::Block(BlockId(660)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("orange_bed", ItemKind::Block(BlockId(661)), 0.0, 0.0, 0, 0),
    def!(
        "orange_candle",
        ItemKind::Block(BlockId(662)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_carpet",
        ItemKind::Block(BlockId(664)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_concrete",
        ItemKind::Block(BlockId(665)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_concrete_powder",
        ItemKind::Block(BlockId(666)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_glazed_terracotta",
        ItemKind::Block(BlockId(667)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_shulker_box",
        ItemKind::Block(BlockId(668)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_stained_glass",
        ItemKind::Block(BlockId(669)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_stained_glass_pane",
        ItemKind::Block(BlockId(670)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_terracotta",
        ItemKind::Block(BlockId(671)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "orange_tulip",
        ItemKind::Block(BlockId(672)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("orange_wool", ItemKind::Block(BlockId(674)), 0.0, 0.0, 0, 0),
    def!("oxeye_daisy", ItemKind::Block(BlockId(675)), 0.0, 0.0, 0, 0),
    def!(
        "oxidized_chiseled_copper",
        ItemKind::Block(BlockId(676)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper",
        ItemKind::Block(BlockId(677)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_bars",
        ItemKind::Block(BlockId(678)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_bulb",
        ItemKind::Block(BlockId(679)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_chain",
        ItemKind::Block(BlockId(680)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_chest",
        ItemKind::Block(BlockId(681)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_door",
        ItemKind::Block(BlockId(682)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_golem_statue",
        ItemKind::Block(BlockId(683)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_grate",
        ItemKind::Block(BlockId(684)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_lantern",
        ItemKind::Block(BlockId(685)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_copper_trapdoor",
        ItemKind::Block(BlockId(686)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_cut_copper",
        ItemKind::Block(BlockId(687)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_cut_copper_slab",
        ItemKind::Block(BlockId(688)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_cut_copper_stairs",
        ItemKind::Block(BlockId(689)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "oxidized_lightning_rod",
        ItemKind::Block(BlockId(690)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("packed_ice", ItemKind::Block(BlockId(691)), 0.0, 0.0, 0, 0),
    def!("packed_mud", ItemKind::Block(BlockId(692)), 0.0, 0.0, 0, 0),
    def!(
        "pale_hanging_moss",
        ItemKind::Block(BlockId(693)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_moss_block",
        ItemKind::Block(BlockId(694)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_moss_carpet",
        ItemKind::Block(BlockId(695)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_button",
        ItemKind::Block(BlockId(696)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_door",
        ItemKind::Block(BlockId(697)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_fence",
        ItemKind::Block(BlockId(698)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_fence_gate",
        ItemKind::Block(BlockId(699)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_hanging_sign",
        ItemKind::Block(BlockId(700)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_leaves",
        ItemKind::Block(BlockId(701)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_log",
        ItemKind::Block(BlockId(702)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_planks",
        ItemKind::Block(BlockId(703)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_pressure_plate",
        ItemKind::Block(BlockId(704)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_sapling",
        ItemKind::Block(BlockId(705)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_shelf",
        ItemKind::Block(BlockId(706)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_sign",
        ItemKind::Block(BlockId(707)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_slab",
        ItemKind::Block(BlockId(708)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_stairs",
        ItemKind::Block(BlockId(709)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_trapdoor",
        ItemKind::Block(BlockId(710)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pale_oak_wood",
        ItemKind::Block(BlockId(713)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pearlescent_froglight",
        ItemKind::Block(BlockId(714)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("peony", ItemKind::Block(BlockId(715)), 0.0, 0.0, 0, 0),
    def!(
        "petrified_oak_slab",
        ItemKind::Block(BlockId(716)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("piglin_head", ItemKind::Block(BlockId(717)), 0.0, 0.0, 0, 0),
    def!("pink_banner", ItemKind::Block(BlockId(719)), 0.0, 0.0, 0, 0),
    def!("pink_bed", ItemKind::Block(BlockId(720)), 0.0, 0.0, 0, 0),
    def!("pink_candle", ItemKind::Block(BlockId(721)), 0.0, 0.0, 0, 0),
    def!("pink_carpet", ItemKind::Block(BlockId(723)), 0.0, 0.0, 0, 0),
    def!(
        "pink_concrete",
        ItemKind::Block(BlockId(724)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pink_concrete_powder",
        ItemKind::Block(BlockId(725)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pink_glazed_terracotta",
        ItemKind::Block(BlockId(726)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("pink_petals", ItemKind::Block(BlockId(727)), 0.0, 0.0, 0, 0),
    def!(
        "pink_shulker_box",
        ItemKind::Block(BlockId(728)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pink_stained_glass",
        ItemKind::Block(BlockId(729)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pink_stained_glass_pane",
        ItemKind::Block(BlockId(730)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pink_terracotta",
        ItemKind::Block(BlockId(731)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("pink_tulip", ItemKind::Block(BlockId(732)), 0.0, 0.0, 0, 0),
    def!("pink_wool", ItemKind::Block(BlockId(734)), 0.0, 0.0, 0, 0),
    def!("piston", ItemKind::Block(BlockId(735)), 0.0, 0.0, 0, 0),
    def!(
        "pitcher_plant",
        ItemKind::Block(BlockId(738)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("pitcher_pod", ItemKind::Block(BlockId(737)), 0.0, 0.0, 0, 0), // 旧名 pitcher_crop
    def!("player_head", ItemKind::Block(BlockId(739)), 0.0, 0.0, 0, 0),
    def!(
        "pointed_dripstone",
        ItemKind::Block(BlockId(742)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_andesite",
        ItemKind::Block(BlockId(743)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_andesite_slab",
        ItemKind::Block(BlockId(744)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_andesite_stairs",
        ItemKind::Block(BlockId(745)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_basalt",
        ItemKind::Block(BlockId(746)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone",
        ItemKind::Block(BlockId(747)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_brick_slab",
        ItemKind::Block(BlockId(748)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_brick_stairs",
        ItemKind::Block(BlockId(749)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_brick_wall",
        ItemKind::Block(BlockId(750)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_bricks",
        ItemKind::Block(BlockId(751)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_button",
        ItemKind::Block(BlockId(752)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_pressure_plate",
        ItemKind::Block(BlockId(753)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_slab",
        ItemKind::Block(BlockId(754)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_stairs",
        ItemKind::Block(BlockId(755)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_blackstone_wall",
        ItemKind::Block(BlockId(756)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_deepslate",
        ItemKind::Block(BlockId(757)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_deepslate_slab",
        ItemKind::Block(BlockId(758)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_deepslate_stairs",
        ItemKind::Block(BlockId(759)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_deepslate_wall",
        ItemKind::Block(BlockId(760)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_diorite",
        ItemKind::Block(BlockId(761)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_diorite_slab",
        ItemKind::Block(BlockId(762)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_diorite_stairs",
        ItemKind::Block(BlockId(763)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_granite",
        ItemKind::Block(BlockId(764)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_granite_slab",
        ItemKind::Block(BlockId(765)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_granite_stairs",
        ItemKind::Block(BlockId(766)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_tuff",
        ItemKind::Block(BlockId(767)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_tuff_slab",
        ItemKind::Block(BlockId(768)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_tuff_stairs",
        ItemKind::Block(BlockId(769)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "polished_tuff_wall",
        ItemKind::Block(BlockId(770)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("poppy", ItemKind::Block(BlockId(12)), 0.0, 0.0, 0, 0), // 旧名 flower_red
    def!("potato", ItemKind::Block(BlockId(771)), 0.0, 0.0, 0, 0), // 旧名 potatoes
    def!(
        "powered_rail",
        ItemKind::Block(BlockId(812)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("prismarine", ItemKind::Block(BlockId(813)), 0.0, 0.0, 0, 0),
    def!(
        "prismarine_brick_slab",
        ItemKind::Block(BlockId(814)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "prismarine_brick_stairs",
        ItemKind::Block(BlockId(815)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "prismarine_bricks",
        ItemKind::Block(BlockId(816)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "prismarine_slab",
        ItemKind::Block(BlockId(817)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "prismarine_stairs",
        ItemKind::Block(BlockId(818)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "prismarine_wall",
        ItemKind::Block(BlockId(819)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "pumpkin_seeds",
        ItemKind::Block(BlockId(821)),
        0.0,
        0.0,
        0,
        0
    ), // 旧名 pumpkin_stem
    def!(
        "purple_banner",
        ItemKind::Block(BlockId(822)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("purple_bed", ItemKind::Block(BlockId(823)), 0.0, 0.0, 0, 0),
    def!(
        "purple_candle",
        ItemKind::Block(BlockId(824)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purple_carpet",
        ItemKind::Block(BlockId(826)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purple_concrete",
        ItemKind::Block(BlockId(827)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purple_concrete_powder",
        ItemKind::Block(BlockId(828)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purple_glazed_terracotta",
        ItemKind::Block(BlockId(829)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purple_shulker_box",
        ItemKind::Block(BlockId(830)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purple_stained_glass",
        ItemKind::Block(BlockId(831)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purple_stained_glass_pane",
        ItemKind::Block(BlockId(832)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purple_terracotta",
        ItemKind::Block(BlockId(833)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("purple_wool", ItemKind::Block(BlockId(835)), 0.0, 0.0, 0, 0),
    def!(
        "purpur_block",
        ItemKind::Block(BlockId(836)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "purpur_pillar",
        ItemKind::Block(BlockId(837)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("purpur_slab", ItemKind::Block(BlockId(838)), 0.0, 0.0, 0, 0),
    def!(
        "purpur_stairs",
        ItemKind::Block(BlockId(839)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "quartz_block",
        ItemKind::Block(BlockId(840)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "quartz_bricks",
        ItemKind::Block(BlockId(841)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "quartz_pillar",
        ItemKind::Block(BlockId(842)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("quartz_slab", ItemKind::Block(BlockId(843)), 0.0, 0.0, 0, 0),
    def!(
        "quartz_stairs",
        ItemKind::Block(BlockId(844)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("rail", ItemKind::Block(BlockId(845)), 0.0, 0.0, 0, 0),
    def!(
        "raw_copper_block",
        ItemKind::Block(BlockId(846)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "raw_gold_block",
        ItemKind::Block(BlockId(847)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "raw_iron_block",
        ItemKind::Block(BlockId(848)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("red_banner", ItemKind::Block(BlockId(849)), 0.0, 0.0, 0, 0),
    def!("red_bed", ItemKind::Block(BlockId(850)), 0.0, 0.0, 0, 0),
    def!("red_candle", ItemKind::Block(BlockId(851)), 0.0, 0.0, 0, 0),
    def!("red_carpet", ItemKind::Block(BlockId(853)), 0.0, 0.0, 0, 0),
    def!(
        "red_concrete",
        ItemKind::Block(BlockId(854)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_concrete_powder",
        ItemKind::Block(BlockId(855)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_glazed_terracotta",
        ItemKind::Block(BlockId(856)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_mushroom",
        ItemKind::Block(BlockId(857)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_mushroom_block",
        ItemKind::Block(BlockId(858)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_nether_brick_slab",
        ItemKind::Block(BlockId(859)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_nether_brick_stairs",
        ItemKind::Block(BlockId(860)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_nether_brick_wall",
        ItemKind::Block(BlockId(861)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_nether_bricks",
        ItemKind::Block(BlockId(862)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_sandstone",
        ItemKind::Block(BlockId(864)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_sandstone_slab",
        ItemKind::Block(BlockId(865)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_sandstone_stairs",
        ItemKind::Block(BlockId(866)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_sandstone_wall",
        ItemKind::Block(BlockId(867)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_shulker_box",
        ItemKind::Block(BlockId(868)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_stained_glass",
        ItemKind::Block(BlockId(869)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_stained_glass_pane",
        ItemKind::Block(BlockId(870)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "red_terracotta",
        ItemKind::Block(BlockId(871)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("red_tulip", ItemKind::Block(BlockId(872)), 0.0, 0.0, 0, 0),
    def!("red_wool", ItemKind::Block(BlockId(874)), 0.0, 0.0, 0, 0),
    def!(
        "redstone_block",
        ItemKind::Block(BlockId(875)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "redstone_lamp",
        ItemKind::Block(BlockId(876)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "redstone_ore",
        ItemKind::Block(BlockId(877)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "redstone_torch",
        ItemKind::Block(BlockId(878)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "reinforced_deepslate",
        ItemKind::Block(BlockId(881)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("repeater", ItemKind::Block(BlockId(882)), 0.0, 0.0, 0, 0),
    def!(
        "repeating_command_block",
        ItemKind::Block(BlockId(883)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("resin_block", ItemKind::Block(BlockId(884)), 0.0, 0.0, 0, 0),
    def!(
        "resin_brick_slab",
        ItemKind::Block(BlockId(885)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "resin_brick_stairs",
        ItemKind::Block(BlockId(886)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "resin_brick_wall",
        ItemKind::Block(BlockId(887)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "resin_bricks",
        ItemKind::Block(BlockId(888)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("resin_clump", ItemKind::Block(BlockId(889)), 0.0, 0.0, 0, 0),
    def!(
        "respawn_anchor",
        ItemKind::Block(BlockId(890)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("rooted_dirt", ItemKind::Block(BlockId(891)), 0.0, 0.0, 0, 0),
    def!("rose_bush", ItemKind::Block(BlockId(892)), 0.0, 0.0, 0, 0),
    def!("sandstone", ItemKind::Block(BlockId(893)), 0.0, 0.0, 0, 0),
    def!(
        "sandstone_slab",
        ItemKind::Block(BlockId(894)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "sandstone_stairs",
        ItemKind::Block(BlockId(895)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "sandstone_wall",
        ItemKind::Block(BlockId(896)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("scaffolding", ItemKind::Block(BlockId(897)), 0.0, 0.0, 0, 0),
    def!("sculk", ItemKind::Block(BlockId(898)), 0.0, 0.0, 0, 0),
    def!(
        "sculk_catalyst",
        ItemKind::Block(BlockId(899)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "sculk_sensor",
        ItemKind::Block(BlockId(900)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "sculk_shrieker",
        ItemKind::Block(BlockId(901)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("sculk_vein", ItemKind::Block(BlockId(902)), 0.0, 0.0, 0, 0),
    def!("sea_lantern", ItemKind::Block(BlockId(903)), 0.0, 0.0, 0, 0),
    def!("sea_pickle", ItemKind::Block(BlockId(904)), 0.0, 0.0, 0, 0),
    def!("seagrass", ItemKind::Block(BlockId(905)), 0.0, 0.0, 0, 0),
    def!(
        "short_dry_grass",
        ItemKind::Block(BlockId(906)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("short_grass", ItemKind::Block(BlockId(907)), 0.0, 0.0, 0, 0),
    def!("shroomlight", ItemKind::Block(BlockId(908)), 0.0, 0.0, 0, 0),
    def!("shulker_box", ItemKind::Block(BlockId(909)), 0.0, 0.0, 0, 0),
    def!(
        "skeleton_skull",
        ItemKind::Block(BlockId(910)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("slime_block", ItemKind::Block(BlockId(912)), 0.0, 0.0, 0, 0),
    def!(
        "small_amethyst_bud",
        ItemKind::Block(BlockId(913)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "small_dripleaf",
        ItemKind::Block(BlockId(914)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smithing_table",
        ItemKind::Block(BlockId(915)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("smoker", ItemKind::Block(BlockId(916)), 0.0, 0.0, 0, 0),
    def!(
        "smooth_basalt",
        ItemKind::Block(BlockId(917)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_quartz",
        ItemKind::Block(BlockId(918)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_quartz_slab",
        ItemKind::Block(BlockId(919)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_quartz_stairs",
        ItemKind::Block(BlockId(920)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_red_sandstone",
        ItemKind::Block(BlockId(921)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_red_sandstone_slab",
        ItemKind::Block(BlockId(922)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_red_sandstone_stairs",
        ItemKind::Block(BlockId(923)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_sandstone",
        ItemKind::Block(BlockId(924)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_sandstone_slab",
        ItemKind::Block(BlockId(925)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_sandstone_stairs",
        ItemKind::Block(BlockId(926)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_stone",
        ItemKind::Block(BlockId(927)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "smooth_stone_slab",
        ItemKind::Block(BlockId(928)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("sniffer_egg", ItemKind::Block(BlockId(929)), 0.0, 0.0, 0, 0),
    def!("snow", ItemKind::Block(BlockId(930)), 0.0, 0.0, 0, 0),
    def!("snow_block", ItemKind::Block(BlockId(931)), 0.0, 0.0, 0, 0),
    def!(
        "soul_campfire",
        ItemKind::Block(BlockId(932)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "soul_lantern",
        ItemKind::Block(BlockId(934)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("soul_torch", ItemKind::Block(BlockId(937)), 0.0, 0.0, 0, 0),
    def!("spawner", ItemKind::Block(BlockId(939)), 0.0, 0.0, 0, 0),
    def!("sponge", ItemKind::Block(BlockId(940)), 0.0, 0.0, 0, 0),
    def!(
        "spore_blossom",
        ItemKind::Block(BlockId(941)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "spruce_hanging_sign",
        ItemKind::Block(BlockId(946)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "spruce_leaves",
        ItemKind::Block(BlockId(947)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "spruce_shelf",
        ItemKind::Block(BlockId(952)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("spruce_sign", ItemKind::Block(BlockId(953)), 0.0, 0.0, 0, 0),
    def!("spruce_wood", ItemKind::Block(BlockId(959)), 0.0, 0.0, 0, 0),
    def!(
        "sticky_piston",
        ItemKind::Block(BlockId(960)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stone_brick_slab",
        ItemKind::Block(BlockId(961)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stone_brick_stairs",
        ItemKind::Block(BlockId(962)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stone_brick_wall",
        ItemKind::Block(BlockId(963)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stone_bricks",
        ItemKind::Block(BlockId(964)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stone_button",
        ItemKind::Block(BlockId(965)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stone_pressure_plate",
        ItemKind::Block(BlockId(966)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("stone_slab", ItemKind::Block(BlockId(967)), 0.0, 0.0, 0, 0),
    def!(
        "stone_stairs",
        ItemKind::Block(BlockId(968)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("stonecutter", ItemKind::Block(BlockId(969)), 0.0, 0.0, 0, 0),
    def!(
        "stripped_acacia_log",
        ItemKind::Block(BlockId(970)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_acacia_wood",
        ItemKind::Block(BlockId(971)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_bamboo_block",
        ItemKind::Block(BlockId(972)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_birch_log",
        ItemKind::Block(BlockId(973)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_birch_wood",
        ItemKind::Block(BlockId(974)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_cherry_log",
        ItemKind::Block(BlockId(975)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_cherry_wood",
        ItemKind::Block(BlockId(976)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_crimson_hyphae",
        ItemKind::Block(BlockId(977)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_crimson_stem",
        ItemKind::Block(BlockId(978)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_dark_oak_log",
        ItemKind::Block(BlockId(979)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_dark_oak_wood",
        ItemKind::Block(BlockId(980)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_jungle_log",
        ItemKind::Block(BlockId(981)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_jungle_wood",
        ItemKind::Block(BlockId(982)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_mangrove_log",
        ItemKind::Block(BlockId(983)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_mangrove_wood",
        ItemKind::Block(BlockId(984)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_oak_log",
        ItemKind::Block(BlockId(985)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_oak_wood",
        ItemKind::Block(BlockId(986)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_pale_oak_log",
        ItemKind::Block(BlockId(987)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_pale_oak_wood",
        ItemKind::Block(BlockId(988)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_spruce_log",
        ItemKind::Block(BlockId(989)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_spruce_wood",
        ItemKind::Block(BlockId(990)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_warped_hyphae",
        ItemKind::Block(BlockId(991)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "stripped_warped_stem",
        ItemKind::Block(BlockId(992)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "structure_block",
        ItemKind::Block(BlockId(993)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "structure_void",
        ItemKind::Block(BlockId(994)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("sugar_cane", ItemKind::Block(BlockId(995)), 0.0, 0.0, 0, 0),
    def!("sunflower", ItemKind::Block(BlockId(996)), 0.0, 0.0, 0, 0),
    def!(
        "suspicious_gravel",
        ItemKind::Block(BlockId(997)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "suspicious_sand",
        ItemKind::Block(BlockId(998)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "sweet_berries",
        ItemKind::Block(BlockId(999)),
        0.0,
        0.0,
        0,
        0
    ), // 旧名 sweet_berry_bush
    def!(
        "tall_dry_grass",
        ItemKind::Block(BlockId(1000)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("tall_grass", ItemKind::Block(BlockId(1001)), 0.0, 0.0, 0, 0),
    def!("target", ItemKind::Block(BlockId(1003)), 0.0, 0.0, 0, 0),
    def!("terracotta", ItemKind::Block(BlockId(1004)), 0.0, 0.0, 0, 0),
    def!("test_block", ItemKind::Block(BlockId(1005)), 0.0, 0.0, 0, 0),
    def!(
        "test_instance_block",
        ItemKind::Block(BlockId(1006)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "tinted_glass",
        ItemKind::Block(BlockId(1007)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("tnt", ItemKind::Block(BlockId(1008)), 0.0, 0.0, 0, 0),
    def!("torch", ItemKind::Block(BlockId(1009)), 0.0, 0.0, 0, 0),
    def!(
        "torchflower",
        ItemKind::Block(BlockId(1010)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "torchflower_seeds",
        ItemKind::Block(BlockId(1011)),
        0.0,
        0.0,
        0,
        0
    ), // 旧名 torchflower_crop
    def!(
        "trapped_chest",
        ItemKind::Block(BlockId(1012)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "trial_spawner",
        ItemKind::Block(BlockId(1013)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "tripwire_hook",
        ItemKind::Block(BlockId(1015)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("tube_coral", ItemKind::Block(BlockId(1016)), 0.0, 0.0, 0, 0),
    def!(
        "tube_coral_block",
        ItemKind::Block(BlockId(1017)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "tube_coral_fan",
        ItemKind::Block(BlockId(1018)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "tuff_brick_slab",
        ItemKind::Block(BlockId(1021)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "tuff_brick_stairs",
        ItemKind::Block(BlockId(1022)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "tuff_brick_wall",
        ItemKind::Block(BlockId(1023)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "tuff_bricks",
        ItemKind::Block(BlockId(1024)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("tuff_slab", ItemKind::Block(BlockId(1025)), 0.0, 0.0, 0, 0),
    def!(
        "tuff_stairs",
        ItemKind::Block(BlockId(1026)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("tuff_wall", ItemKind::Block(BlockId(1027)), 0.0, 0.0, 0, 0),
    def!("turtle_egg", ItemKind::Block(BlockId(1028)), 0.0, 0.0, 0, 0),
    def!(
        "twisting_vines",
        ItemKind::Block(BlockId(1029)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("vault", ItemKind::Block(BlockId(1031)), 0.0, 0.0, 0, 0),
    def!(
        "verdant_froglight",
        ItemKind::Block(BlockId(1032)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("vine", ItemKind::Block(BlockId(1033)), 0.0, 0.0, 0, 0),
    def!(
        "warped_button",
        ItemKind::Block(BlockId(1036)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_door",
        ItemKind::Block(BlockId(1037)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_fence",
        ItemKind::Block(BlockId(1038)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_fence_gate",
        ItemKind::Block(BlockId(1039)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_fungus",
        ItemKind::Block(BlockId(1040)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_hanging_sign",
        ItemKind::Block(BlockId(1041)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_hyphae",
        ItemKind::Block(BlockId(1042)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_nylium",
        ItemKind::Block(BlockId(1043)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_planks",
        ItemKind::Block(BlockId(1044)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_pressure_plate",
        ItemKind::Block(BlockId(1045)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_roots",
        ItemKind::Block(BlockId(1046)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_shelf",
        ItemKind::Block(BlockId(1047)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_sign",
        ItemKind::Block(BlockId(1048)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_slab",
        ItemKind::Block(BlockId(1049)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_stairs",
        ItemKind::Block(BlockId(1050)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_stem",
        ItemKind::Block(BlockId(1051)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_trapdoor",
        ItemKind::Block(BlockId(1052)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_wart_block",
        ItemKind::Block(BlockId(1055)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_chiseled_copper",
        ItemKind::Block(BlockId(1057)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_bars",
        ItemKind::Block(BlockId(1058)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_block",
        ItemKind::Block(BlockId(1059)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_bulb",
        ItemKind::Block(BlockId(1060)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_chain",
        ItemKind::Block(BlockId(1061)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_chest",
        ItemKind::Block(BlockId(1062)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_door",
        ItemKind::Block(BlockId(1063)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_golem_statue",
        ItemKind::Block(BlockId(1064)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_grate",
        ItemKind::Block(BlockId(1065)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_lantern",
        ItemKind::Block(BlockId(1066)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_copper_trapdoor",
        ItemKind::Block(BlockId(1067)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_cut_copper",
        ItemKind::Block(BlockId(1068)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_cut_copper_slab",
        ItemKind::Block(BlockId(1069)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_cut_copper_stairs",
        ItemKind::Block(BlockId(1070)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_chiseled_copper",
        ItemKind::Block(BlockId(1071)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper",
        ItemKind::Block(BlockId(1072)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_bars",
        ItemKind::Block(BlockId(1073)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_bulb",
        ItemKind::Block(BlockId(1074)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_chain",
        ItemKind::Block(BlockId(1075)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_chest",
        ItemKind::Block(BlockId(1076)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_door",
        ItemKind::Block(BlockId(1077)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_golem_statue",
        ItemKind::Block(BlockId(1078)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_grate",
        ItemKind::Block(BlockId(1079)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_lantern",
        ItemKind::Block(BlockId(1080)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_copper_trapdoor",
        ItemKind::Block(BlockId(1081)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_cut_copper",
        ItemKind::Block(BlockId(1082)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_cut_copper_slab",
        ItemKind::Block(BlockId(1083)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_cut_copper_stairs",
        ItemKind::Block(BlockId(1084)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_exposed_lightning_rod",
        ItemKind::Block(BlockId(1085)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_lightning_rod",
        ItemKind::Block(BlockId(1086)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_chiseled_copper",
        ItemKind::Block(BlockId(1087)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper",
        ItemKind::Block(BlockId(1088)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_bars",
        ItemKind::Block(BlockId(1089)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_bulb",
        ItemKind::Block(BlockId(1090)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_chain",
        ItemKind::Block(BlockId(1091)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_chest",
        ItemKind::Block(BlockId(1092)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_door",
        ItemKind::Block(BlockId(1093)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_golem_statue",
        ItemKind::Block(BlockId(1094)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_grate",
        ItemKind::Block(BlockId(1095)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_lantern",
        ItemKind::Block(BlockId(1096)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_copper_trapdoor",
        ItemKind::Block(BlockId(1097)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_cut_copper",
        ItemKind::Block(BlockId(1098)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_cut_copper_slab",
        ItemKind::Block(BlockId(1099)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_cut_copper_stairs",
        ItemKind::Block(BlockId(1100)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_oxidized_lightning_rod",
        ItemKind::Block(BlockId(1101)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_chiseled_copper",
        ItemKind::Block(BlockId(1102)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper",
        ItemKind::Block(BlockId(1103)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_bars",
        ItemKind::Block(BlockId(1104)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_bulb",
        ItemKind::Block(BlockId(1105)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_chain",
        ItemKind::Block(BlockId(1106)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_chest",
        ItemKind::Block(BlockId(1107)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_door",
        ItemKind::Block(BlockId(1108)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_golem_statue",
        ItemKind::Block(BlockId(1109)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_grate",
        ItemKind::Block(BlockId(1110)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_lantern",
        ItemKind::Block(BlockId(1111)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_copper_trapdoor",
        ItemKind::Block(BlockId(1112)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_cut_copper",
        ItemKind::Block(BlockId(1113)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_cut_copper_slab",
        ItemKind::Block(BlockId(1114)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_cut_copper_stairs",
        ItemKind::Block(BlockId(1115)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "waxed_weathered_lightning_rod",
        ItemKind::Block(BlockId(1116)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_chiseled_copper",
        ItemKind::Block(BlockId(1117)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper",
        ItemKind::Block(BlockId(1118)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_bars",
        ItemKind::Block(BlockId(1119)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_bulb",
        ItemKind::Block(BlockId(1120)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_chain",
        ItemKind::Block(BlockId(1121)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_chest",
        ItemKind::Block(BlockId(1122)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_door",
        ItemKind::Block(BlockId(1123)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_golem_statue",
        ItemKind::Block(BlockId(1124)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_grate",
        ItemKind::Block(BlockId(1125)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_lantern",
        ItemKind::Block(BlockId(1126)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_copper_trapdoor",
        ItemKind::Block(BlockId(1127)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_cut_copper",
        ItemKind::Block(BlockId(1128)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_cut_copper_slab",
        ItemKind::Block(BlockId(1129)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_cut_copper_stairs",
        ItemKind::Block(BlockId(1130)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weathered_lightning_rod",
        ItemKind::Block(BlockId(1131)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "weeping_vines",
        ItemKind::Block(BlockId(1132)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("wet_sponge", ItemKind::Block(BlockId(1134)), 0.0, 0.0, 0, 0),
    def!(
        "wheat_seeds",
        ItemKind::Block(BlockId(1135)),
        0.0,
        0.0,
        0,
        0
    ), // 旧名 wheat
    def!(
        "white_banner",
        ItemKind::Block(BlockId(1136)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("white_bed", ItemKind::Block(BlockId(1137)), 0.0, 0.0, 0, 0),
    def!(
        "white_candle",
        ItemKind::Block(BlockId(1138)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_carpet",
        ItemKind::Block(BlockId(1140)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_concrete",
        ItemKind::Block(BlockId(1141)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_concrete_powder",
        ItemKind::Block(BlockId(1142)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_glazed_terracotta",
        ItemKind::Block(BlockId(1143)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_shulker_box",
        ItemKind::Block(BlockId(1144)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_stained_glass",
        ItemKind::Block(BlockId(1145)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_stained_glass_pane",
        ItemKind::Block(BlockId(1146)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_terracotta",
        ItemKind::Block(BlockId(1147)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "white_tulip",
        ItemKind::Block(BlockId(1148)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("white_wool", ItemKind::Block(BlockId(1150)), 0.0, 0.0, 0, 0),
    def!(
        "wildflowers",
        ItemKind::Block(BlockId(1151)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "wither_rose",
        ItemKind::Block(BlockId(1152)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "wither_skeleton_skull",
        ItemKind::Block(BlockId(1153)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_banner",
        ItemKind::Block(BlockId(1155)),
        0.0,
        0.0,
        0,
        0
    ),
    def!("yellow_bed", ItemKind::Block(BlockId(1156)), 0.0, 0.0, 0, 0),
    def!(
        "yellow_candle",
        ItemKind::Block(BlockId(1157)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_carpet",
        ItemKind::Block(BlockId(1159)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_concrete",
        ItemKind::Block(BlockId(1160)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_concrete_powder",
        ItemKind::Block(BlockId(1161)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_glazed_terracotta",
        ItemKind::Block(BlockId(1162)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_shulker_box",
        ItemKind::Block(BlockId(1163)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_stained_glass",
        ItemKind::Block(BlockId(1164)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_stained_glass_pane",
        ItemKind::Block(BlockId(1165)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_terracotta",
        ItemKind::Block(BlockId(1166)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "yellow_wool",
        ItemKind::Block(BlockId(1168)),
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "zombie_head",
        ItemKind::Block(BlockId(1169)),
        0.0,
        0.0,
        0,
        0
    ),
    // ---- 材料物品（注册名字典序）----
    def!("acacia_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("acacia_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("amethyst_shard", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("angler_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("archer_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("armadillo_scute", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("armor_stand", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("arms_up_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("baked_potato", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("bamboo_chest_raft", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("bamboo_raft", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("beef", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("beetroot", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("beetroot_soup", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("birch_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("birch_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("black_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("black_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("black_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("blade_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("blaze_powder", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("blaze_rod", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("blue_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("blue_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("blue_egg", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("blue_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "bolt_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("bone_meal", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "bordure_indented_banner_pattern",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("bowl", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("bread", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("breeze_rod", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("brewer_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("brick", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("brown_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("brown_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("brown_egg", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("brown_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("brush", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("bucket", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("burn_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("carrot_on_a_stick", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("chainmail_boots", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("chainmail_chestplate", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("chainmail_helmet", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("chainmail_leggings", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("charcoal", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cherry_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cherry_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("chest_minecart", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("chicken", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("chorus_fruit", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("clock", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "coast_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("cod", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cod_bucket", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("compass", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cooked_beef", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cooked_chicken", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cooked_cod", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cooked_mutton", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cooked_porkchop", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cooked_rabbit", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cooked_salmon", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cookie", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("copper_boots", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("copper_chestplate", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("copper_helmet", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("copper_hoe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("copper_ingot", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("copper_leggings", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("copper_nugget", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("copper_spear", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("creeper_banner_pattern", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("crossbow", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cyan_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cyan_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("cyan_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("danger_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("dark_oak_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("dark_oak_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("diamond_boots", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("diamond_chestplate", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("diamond_helmet", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("diamond_hoe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("diamond_leggings", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("diamond_spear", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("disc_fragment_5", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("dried_kelp", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "dune_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("echo_shard", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("egg", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("enchanted_golden_apple", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("end_crystal", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("ender_eye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("ender_pearl", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("explorer_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "eye_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("feather", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("fermented_spider_eye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "field_masoned_banner_pattern",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("fire_charge", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("firework_rocket", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("fishing_rod", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("flint_and_steel", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "flow_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("flow_banner_pattern", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("flow_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("flower_banner_pattern", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("friend_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("furnace_minecart", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("ghast_tear", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("glistering_melon_slice", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("globe_banner_pattern", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("glow_ink_sac", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("glow_item_frame", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_apple", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_axe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_boots", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_carrot", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_chestplate", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_helmet", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_hoe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_horse_armor", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_leggings", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_nautilus_armor", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_shovel", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("golden_spear", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("gray_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("gray_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("gray_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("green_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("green_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("green_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("guster_banner_pattern", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("guster_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("heart_of_the_sea", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("heart_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "heartbreak_pottery_sherd",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("hopper_minecart", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "host_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("howl_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("ink_sac", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("iron_boots", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("iron_chestplate", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("iron_helmet", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("iron_hoe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("iron_leggings", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("iron_nugget", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("iron_spear", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("item_frame", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("jungle_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("jungle_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("knowledge_book", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("lapis_lazuli", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("lead", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("leather", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("leather_boots", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("leather_chestplate", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("leather_helmet", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("leather_horse_armor", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("leather_leggings", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("light_blue_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("light_blue_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("light_blue_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("light_gray_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("light_gray_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("light_gray_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("lime_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("lime_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("lime_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("mace", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("magenta_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("magenta_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("magenta_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("magma_cream", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("mangrove_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("mangrove_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("map", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("milk_bucket", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("minecart", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("miner_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("mojang_banner_pattern", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("mourner_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("mushroom_stew", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_11", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_13", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_5", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_blocks", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_cat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_chirp", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_far", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_mall", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_mellohi", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_stal", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_strad", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_wait", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("music_disc_ward", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("mutton", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("name_tag", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("nautilus_shell", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("nether_brick", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("nether_star", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_axe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_boots", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_chestplate", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_helmet", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_hoe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_leggings", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_scrap", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_shovel", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("netherite_spear", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "netherite_upgrade_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("oak_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("oak_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("orange_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("orange_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("orange_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("painting", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("pale_oak_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("pale_oak_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("paper", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("piglin_banner_pattern", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("pink_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("pink_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("pink_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("plenty_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("poisonous_potato", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("popped_chorus_fruit", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("porkchop", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("prismarine_shard", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("prize_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("pufferfish", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("pufferfish_bucket", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("pumpkin_pie", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("purple_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("purple_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("purple_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("rabbit", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("rabbit_foot", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("rabbit_hide", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("rabbit_stew", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "raiser_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("recovery_compass", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("red_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("red_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("red_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("resin_brick", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "rib_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("saddle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("salmon", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("salmon_bucket", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("scrape_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "sentry_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "shaper_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("sheaf_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("shears", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("shelter_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("shield", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("shulker_shell", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "silence_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("skull_banner_pattern", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("skull_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("slime_ball", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("snort_pottery_sherd", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "snout_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("spectral_arrow", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "spire_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("spruce_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("spruce_chest_boat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("spyglass", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("stone_hoe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("stone_shovel", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("stone_spear", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("sugar", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "tide_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("tipped_arrow", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("tnt_minecart", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("trident", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("tropical_fish", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("tropical_fish_bucket", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("turtle_helmet", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("turtle_scute", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "vex_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "ward_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "warped_fungus_on_a_stick",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!(
        "wayfinder_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("wheat", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("white_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("white_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("white_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!(
        "wild_armor_trim_smithing_template",
        ItemKind::Material,
        0.0,
        0.0,
        0,
        0
    ),
    def!("wind_charge", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("wolf_armor", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("wooden_hoe", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("wooden_spear", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("writable_book", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("written_book", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("yellow_bundle", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("yellow_dye", ItemKind::Material, 0.0, 0.0, 0, 0),
    def!("yellow_harness", ItemKind::Material, 0.0, 0.0, 0, 0),
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
// 怪物掉落杂物（36 起与 ITEMS 表序一致；食物属性见 [`food::FOODS`]）。
pub const ROTTEN_FLESH: u16 = 36;
pub const SPIDER_EYE: u16 = 41;
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
/// 弓（ITEMS 下标 42：`durability(384)` → 不可堆叠，见 inventory::max_stack）。
pub const BOW: u16 = 42;
/// 金/下界锭（59/60）：金/下界具铁砧修料（*_tool_materials tags）。
pub const GOLD_INGOT: u16 = 59;
pub const NETHERITE_INGOT: u16 = 60;
// 掉落族补全物品（61-89，与 ITEMS 追加序一致，测试
// item_family_const_order_lock 锁死；loot 语义见 inventory::DROP_TABLE）。
pub const GRANITE: u16 = 61;
pub const DIORITE: u16 = 62;
pub const ANDESITE: u16 = 63;
pub const TUFF: u16 = 64;
pub const COARSE_DIRT: u16 = 65;
pub const PODZOL: u16 = 66;
pub const MYCELIUM: u16 = 67;
pub const RED_SAND: u16 = 68;
pub const CLAY_BALL: u16 = 69;
pub const MOSS_BLOCK: u16 = 70;
pub const MUD: u16 = 71;
pub const PUMPKIN: u16 = 72;
pub const MELON_SLICE: u16 = 73;
pub const GLOWSTONE_DUST: u16 = 74;
pub const PRISMARINE_CRYSTALS: u16 = 75;
pub const BIRCH_LOG: u16 = 76;
pub const SPRUCE_LOG: u16 = 77;
pub const JUNGLE_LOG: u16 = 78;
pub const ACACIA_LOG: u16 = 79;
pub const DARK_OAK_LOG: u16 = 80;
pub const NETHERRACK: u16 = 81;
pub const SOUL_SAND: u16 = 82;
pub const SOUL_SOIL: u16 = 83;
pub const OBSIDIAN: u16 = 84;
pub const MAGMA_BLOCK: u16 = 85;
pub const BASALT: u16 = 86;
pub const ANCIENT_DEBRIS: u16 = 87;
pub const GOLD_NUGGET: u16 = 88;
pub const QUARTZ: u16 = 89;

// ---- 木族+蜂窝道具（#137 W 波，90-151，与 ITEMS 追加序一致；映射与
// 图标层测试 tests/wood_family.rs 锁死）。oak 族 log/planks = 旧 LOG/PLANKS。
pub const SPRUCE_PLANKS: u16 = 90;
pub const BIRCH_PLANKS: u16 = 91;
pub const JUNGLE_PLANKS: u16 = 92;
pub const ACACIA_PLANKS: u16 = 93;
pub const DARK_OAK_PLANKS: u16 = 94;
pub const OAK_STAIRS: u16 = 95;
pub const SPRUCE_STAIRS: u16 = 96;
pub const BIRCH_STAIRS: u16 = 97;
pub const JUNGLE_STAIRS: u16 = 98;
pub const ACACIA_STAIRS: u16 = 99;
pub const DARK_OAK_STAIRS: u16 = 100;
pub const OAK_SLAB: u16 = 101;
pub const SPRUCE_SLAB: u16 = 102;
pub const BIRCH_SLAB: u16 = 103;
pub const JUNGLE_SLAB: u16 = 104;
pub const ACACIA_SLAB: u16 = 105;
pub const DARK_OAK_SLAB: u16 = 106;
pub const OAK_FENCE: u16 = 107;
pub const SPRUCE_FENCE: u16 = 108;
pub const BIRCH_FENCE: u16 = 109;
pub const JUNGLE_FENCE: u16 = 110;
pub const ACACIA_FENCE: u16 = 111;
pub const DARK_OAK_FENCE: u16 = 112;
pub const OAK_FENCE_GATE: u16 = 113;
pub const SPRUCE_FENCE_GATE: u16 = 114;
pub const BIRCH_FENCE_GATE: u16 = 115;
pub const JUNGLE_FENCE_GATE: u16 = 116;
pub const ACACIA_FENCE_GATE: u16 = 117;
pub const DARK_OAK_FENCE_GATE: u16 = 118;
pub const OAK_DOOR: u16 = 119;
pub const SPRUCE_DOOR: u16 = 120;
pub const BIRCH_DOOR: u16 = 121;
pub const JUNGLE_DOOR: u16 = 122;
pub const ACACIA_DOOR: u16 = 123;
pub const DARK_OAK_DOOR: u16 = 124;
pub const OAK_TRAPDOOR: u16 = 125;
pub const SPRUCE_TRAPDOOR: u16 = 126;
pub const BIRCH_TRAPDOOR: u16 = 127;
pub const JUNGLE_TRAPDOOR: u16 = 128;
pub const ACACIA_TRAPDOOR: u16 = 129;
pub const DARK_OAK_TRAPDOOR: u16 = 130;
pub const OAK_BUTTON: u16 = 131;
pub const SPRUCE_BUTTON: u16 = 132;
pub const BIRCH_BUTTON: u16 = 133;
pub const JUNGLE_BUTTON: u16 = 134;
pub const ACACIA_BUTTON: u16 = 135;
pub const DARK_OAK_BUTTON: u16 = 136;
pub const OAK_PRESSURE_PLATE: u16 = 137;
pub const SPRUCE_PRESSURE_PLATE: u16 = 138;
pub const BIRCH_PRESSURE_PLATE: u16 = 139;
pub const JUNGLE_PRESSURE_PLATE: u16 = 140;
pub const ACACIA_PRESSURE_PLATE: u16 = 141;
pub const DARK_OAK_PRESSURE_PLATE: u16 = 142;
pub const HONEYCOMB: u16 = 143;
pub const HONEY_BOTTLE: u16 = 144;
pub const GLASS_BOTTLE: u16 = 145;
pub const HONEYCOMB_BLOCK: u16 = 146;
pub const HONEY_BLOCK: u16 = 147;
pub const BEEHIVE: u16 = 148;
pub const BEE_NEST: u16 = 149;
pub const CRAFTING_TABLE: u16 = 150;
pub const CHEST: u16 = 151;

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
