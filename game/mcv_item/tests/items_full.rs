//! 物品注册全量扩表判据（#144 全方块计划·物品地基波）。
//!
//! 对账基准（仓库外 /root/mc-ref/src-26.1，路径不入库）：
//! - `net/minecraft/world/item/Items.java` —— 注册名权威（registerBlock /
//!   registerItem / registerSpawnEgg / WeatheringCopperItems 四种注册形态，
//!   本表收前两种 + 铜件展开；spawn egg/药水/桶等未入配方/loot 面的 140 件
//!   不注册 = 宁缺勿错，债登记 ci/recipes-gap.md）；
//! - `game/mcv_core/src/blocks_gen.inc.rs` —— 方块 id 主键（Block 物品引用
//!   既有 id，不碰生成文件）；
//! - `data/minecraft/recipe/*.json` + `tags/item/*.json` —— 配方/tag 面；
//! - `data/minecraft/loot_table/blocks/*.json` —— 自掉语义取证。
//!
//! 锚三方一致：物品注册名 ↔ blocks_gen 方块名（含 legacy 别名）↔ 官方 lang
//! key ↔ 配方 JSON。gen 复现锁见 recipes_gen.rs（配方产物）与
//! mcv_app/tests/lang_items.rs（lang 产物）；本文件锁注册表本体与行为。

use mcv_core::BlockId;
use mcv_item::{ItemKind, ItemStack, crafting, drops_for_block, item_by_name};

fn id(name: &str) -> u16 {
    item_by_name(name).unwrap_or_else(|| panic!("ITEMS 缺 {name}"))
}

fn def(name: &str) -> &'static mcv_item::ItemDef {
    &mcv_item::ITEMS[id(name) as usize]
}

fn bid(name: &str) -> BlockId {
    let i = mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("blocks_gen 缺 {name}"));
    BlockId(i as u16)
}

/// 按配方 JSON 的 pattern/keys 书写测试摆放（rows ≤3；' ' 空）。
fn from_pattern(rows: &[&str], keys: &[(char, u16)]) -> Vec<u16> {
    let w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);
    let mut grid = vec![crafting::EMPTY_SLOT; w * rows.len()];
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == ' ' {
                continue;
            }
            let item = keys
                .iter()
                .find(|(k, _)| *k == c)
                .map(|(_, it)| *it)
                .unwrap_or_else(|| panic!("测试 keys 缺字符 {c}"));
            grid[y * w + x] = item;
        }
    }
    grid
}

// ---- 1. 规模与唯一性 --------------------------------------------------------

/// 全表规模锁：152（旧表，id 序列化契约）+ 1229 新件 = 1381。
/// 增量波只许尾插 + 同步改此锁与 legacy_prefix_order_lock。
#[test]
fn items_count_full_table_lock() {
    assert_eq!(mcv_item::ITEMS.len(), 1381, "ITEMS 全表规模");
}

/// 注册名全局唯一（配方 JSON / lang key / drops 表的名字解析前提）。
#[test]
fn item_names_unique() {
    let mut seen = std::collections::HashSet::new();
    for d in mcv_item::ITEMS.iter() {
        assert!(seen.insert(d.name), "注册名重复: {}", d.name);
    }
}

// ---- 2. 旧 152 序列（追加不改头）-------------------------------------------

/// 旧 152 物品名序逐值锁（26.1 数据扩表只许尾插，id 序列化稳定）。
#[test]
fn legacy_prefix_order_lock() {
    const LEGACY_PREFIX: [&str; 152] = [
        "stick",
        "coal",
        "iron_ingot",
        "diamond",
        "lapis",
        "book",
        "enchanted_book",
        "wooden_sword",
        "stone_sword",
        "iron_sword",
        "diamond_sword",
        "golden_sword",
        "netherite_sword",
        "wooden_pickaxe",
        "stone_pickaxe",
        "iron_pickaxe",
        "diamond_pickaxe",
        "golden_pickaxe",
        "netherite_pickaxe",
        "wooden_axe",
        "stone_axe",
        "iron_axe",
        "diamond_axe",
        "iron_shovel",
        "diamond_shovel",
        "wooden_shovel",
        "planks",
        "cobblestone",
        "stone",
        "iron_ore",
        "diamond_ore",
        "log",
        "dirt",
        "grass",
        "sand",
        "leaves",
        "rotten_flesh",
        "bone",
        "arrow",
        "gunpowder",
        "string",
        "spider_eye",
        "bow",
        "raw_iron",
        "raw_gold",
        "raw_copper",
        "redstone",
        "emerald",
        "flint",
        "apple",
        "oak_sapling",
        "birch_sapling",
        "spruce_sapling",
        "jungle_sapling",
        "acacia_sapling",
        "dark_oak_sapling",
        "gravel",
        "cobbled_deepslate",
        "snowball",
        "gold_ingot",
        "netherite_ingot",
        "granite",
        "diorite",
        "andesite",
        "tuff",
        "coarse_dirt",
        "podzol",
        "mycelium",
        "red_sand",
        "clay_ball",
        "moss_block",
        "mud",
        "pumpkin",
        "melon_slice",
        "glowstone_dust",
        "prismarine_crystals",
        "birch_log",
        "spruce_log",
        "jungle_log",
        "acacia_log",
        "dark_oak_log",
        "netherrack",
        "soul_sand",
        "soul_soil",
        "obsidian",
        "magma_block",
        "basalt",
        "ancient_debris",
        "gold_nugget",
        "quartz",
        "spruce_planks",
        "birch_planks",
        "jungle_planks",
        "acacia_planks",
        "dark_oak_planks",
        "oak_stairs",
        "spruce_stairs",
        "birch_stairs",
        "jungle_stairs",
        "acacia_stairs",
        "dark_oak_stairs",
        "oak_slab",
        "spruce_slab",
        "birch_slab",
        "jungle_slab",
        "acacia_slab",
        "dark_oak_slab",
        "oak_fence",
        "spruce_fence",
        "birch_fence",
        "jungle_fence",
        "acacia_fence",
        "dark_oak_fence",
        "oak_fence_gate",
        "spruce_fence_gate",
        "birch_fence_gate",
        "jungle_fence_gate",
        "acacia_fence_gate",
        "dark_oak_fence_gate",
        "oak_door",
        "spruce_door",
        "birch_door",
        "jungle_door",
        "acacia_door",
        "dark_oak_door",
        "oak_trapdoor",
        "spruce_trapdoor",
        "birch_trapdoor",
        "jungle_trapdoor",
        "acacia_trapdoor",
        "dark_oak_trapdoor",
        "oak_button",
        "spruce_button",
        "birch_button",
        "jungle_button",
        "acacia_button",
        "dark_oak_button",
        "oak_pressure_plate",
        "spruce_pressure_plate",
        "birch_pressure_plate",
        "jungle_pressure_plate",
        "acacia_pressure_plate",
        "dark_oak_pressure_plate",
        "honeycomb",
        "honey_bottle",
        "glass_bottle",
        "honeycomb_block",
        "honey_block",
        "beehive",
        "bee_nest",
        "crafting_table",
        "chest",
    ];
    for (i, want) in LEGACY_PREFIX.iter().enumerate() {
        assert_eq!(mcv_item::ITEMS[i].name, *want, "旧位 {} 漂移", i);
    }
}

// ---- 3. 代表新物品逐值 ------------------------------------------------------

/// 任务书点名代表：torch（方块物品/图标层）、iron_ingot（旧 kind 不变）、
/// wheat（纯材料）+ 各来源族抽验。
#[test]
fn representative_new_items_values() {
    // torch：方块物品挂 blocks_gen id 1009（"torch"）；图标层 = 图集 tile
    // 面 tiles[2]（game.rs hand_item → HandItem::Block → 方块图集）。
    let torch = def("torch");
    assert!(
        matches!(torch.kind, ItemKind::Block(b) if b == BlockId(1009)),
        "torch 应为 Block(BlockId(1009))"
    );
    assert_eq!(torch.baseline_attack, 0.0);
    assert_eq!(mcv_core::BLOCKS[1009].name, "torch");
    assert_eq!(
        mcv_core::BLOCKS[1009].tiles[2],
        755,
        "torch 图集层（blocks_gen id 1009 行）"
    );

    // iron_ingot：旧位 2、kind IronIngot 不变（扩表不动既有行）。
    assert_eq!(id("iron_ingot"), 2);
    assert!(matches!(def("iron_ingot").kind, ItemKind::IronIngot));

    // wheat：26.1 配方/tag/作物 loot 引用的纯材料（无方块；作物方块 "wheat"
    // 的物品是 wheat_seeds，见 block_items_map_to_blocks_gen 的 SPECIAL 表）。
    assert!(matches!(def("wheat").kind, ItemKind::Material));

    // 方块物品族：furnace / barrel / anvil / bricks。
    assert!(matches!(def("furnace").kind, ItemKind::Block(b) if b == bid("furnace")));
    assert!(matches!(def("barrel").kind, ItemKind::Block(b) if b == bid("barrel")));
    assert!(matches!(def("anvil").kind, ItemKind::Block(b) if b == bid("anvil")));
    assert!(matches!(def("bricks").kind, ItemKind::Block(b) if b == bid("bricks")));

    // 材料族：gap 报告 top 挡路件全部转正。
    for name in [
        "bread",
        "copper_ingot",
        "light_gray_dye",
        "netherite_scrap",
        "paper",
        "iron_nugget",
        "lapis_lazuli",
    ] {
        assert!(
            matches!(def(name).kind, ItemKind::Material),
            "{name} 应为 Material"
        );
    }

    // 铜工具（26.1 新增工具族；COPPER 材料 NOTES-2 §3）。
    let pick = def("copper_pickaxe");
    assert!(matches!(pick.kind, ItemKind::Pickaxe(m) if m == mcv_item::COPPER));
    assert_eq!(pick.max_damage, 190);
    let sword = ItemStack::new(id("copper_sword"), 1);
    // full_attack = baseline 3.0 + 材料加成 1.0（玩家手 1.0 不在面板内，
    // 同 items.rs::sword_attack_values 口径：木 3.0、铁 5.0）。
    assert!((sword.full_attack() - 4.0).abs() < 1e-4, "铜剑 = 3+1");
}

/// 旧 kind 面不回归：六剑仍是 Sword、弓耐久 384（扩表零削改）。
#[test]
fn legacy_kinds_unchanged() {
    for n in [
        "wooden_sword",
        "stone_sword",
        "iron_sword",
        "diamond_sword",
        "golden_sword",
        "netherite_sword",
    ] {
        assert!(matches!(def(n).kind, ItemKind::Sword(_)), "{n}");
    }
    assert!(matches!(def("bow").kind, ItemKind::Bow));
    assert_eq!(def("bow").max_damage, 384);
}

// ---- 4. Block 物品 ↔ blocks_gen 名字映射（全量）----------------------------

/// 26.1 方块物品名 = 方块注册名，唯若干例外（26.1 Items.java 实证）+ legacy
/// 旧 14 方块位（本仓 id 序列化契约）。SPECIAL 例外全集，新件必须同名录入。
#[test]
fn block_items_map_to_blocks_gen() {
    const SPECIAL: &[(&str, &str)] = &[
        // legacy 旧 14 方块时代物品（cobblestone 物品挂旧 cobble 位）。
        ("cobblestone", "cobble"),
        // 官方花名挂旧花位（blocks_gen id 12/13）。
        ("poppy", "flower_red"),
        ("dandelion", "flower_yellow"),
        // 26.1 非同名方块物品（Items.java registerItem +
        // createBlockItemWithCustomItemName 族，作物/藤蔓方块）。
        ("pumpkin_seeds", "pumpkin_stem"),
        ("melon_seeds", "melon_stem"),
        ("wheat_seeds", "wheat"),
        ("cocoa_beans", "cocoa"),
        ("carrot", "carrots"),
        ("potato", "potatoes"),
        ("torchflower_seeds", "torchflower_crop"),
        ("pitcher_pod", "pitcher_crop"),
        ("beetroot_seeds", "beetroots"),
        ("sweet_berries", "sweet_berry_bush"),
        ("glow_berries", "cave_vines"),
        // 旧表误接（iron_ore/diamond_ore 物品挂 stone 位——扩表前既有行为，
        // 修正随方块状态波，本锁仅固化现状）。
        ("iron_ore", "stone"),
        ("diamond_ore", "stone"),
    ];
    for (i, d) in mcv_item::ITEMS.iter().enumerate() {
        let ItemKind::Block(b) = d.kind else {
            continue;
        };
        let want_block = SPECIAL
            .iter()
            .find(|(n, _)| *n == d.name)
            .map(|(_, bn)| *bn)
            .unwrap_or(d.name);
        assert_eq!(
            mcv_core::BLOCKS[b.id() as usize].name,
            want_block,
            "ITEMS[{i}] {} 应挂 blocks_gen \"{want_block}\"",
            d.name
        );
    }
}

// ---- 5. 挖掘掉落：自掉回退（26.1 loot 自掉族 + getCloneItemStack 语义）------

/// 新注册方块挖掉掉自身 ×1（destroy_block → drops_for_block 回退路径；
/// torch/furnace/barrel/anvil/bricks/chest/bee_nest 的 26.1 loot 均为
/// survives_explosion 单池 item <自身>）。
#[test]
fn new_blocks_self_drop() {
    for name in [
        "torch", "furnace", "barrel", "anvil", "bricks", "chest", "bee_nest",
    ] {
        let mut rng = || 7u32;
        let got = drops_for_block(bid(name), &mut rng);
        assert_eq!(got.len(), 1, "{name} 应自掉");
        assert_eq!(got[0].item, id(name), "{name} 掉自身");
        assert_eq!(got[0].count, 1);
    }
}

/// silk 门语义取证：无 silk 恒不掉的族（NO_DROP 全集）与 loot 恒空/掉他物
/// 的方块（NO_SELF_DROP 名单）不因回退误掉。
#[test]
fn silk_only_and_deviant_blocks_do_not_self_drop() {
    for name in [
        "glass",
        "glass_pane",
        "ice",
        "blue_ice",
        "bedrock",
        "barrier",
        "spawner",
        "infested_stone",
        "cake",
        "short_grass",
        "campfire",
        "amethyst_cluster",
    ] {
        let mut rng = || 7u32;
        assert!(
            drops_for_block(bid(name), &mut rng).is_empty(),
            "{name} 无 silk 语义不掉，回退不得误掉"
        );
    }
}

/// 既有 DROP_TABLE 行优先不回归（行覆盖的方块不走回退）：
/// 石头→圆石、草方块→泥土、树叶全不中仍空。
#[test]
fn drop_table_rows_take_priority_over_fallback() {
    let mut rng = || 0u32;
    assert_eq!(
        drops_for_block(bid("stone"), &mut rng),
        [ItemStack::new(id("cobblestone"), 1)]
    );
    assert_eq!(
        drops_for_block(bid("grass"), &mut rng),
        [ItemStack::new(id("dirt"), 1)]
    );
    // 树叶族行在场但全不中（600‰ > 全池）→ 空而不自掉。
    let mut rng = || 600u32;
    assert!(drops_for_block(bid("leaves"), &mut rng).is_empty());
}

// ---- 6. 配方解锁抽测（gap→已解锁；craft_result 出物）-----------------------

#[test]
fn unlocked_recipes_craft_results() {
    // recipe/torch.json：X∈[coal,charcoal]（26.1 显式列表）/ #=stick → ×4。
    // pattern 1 宽 2 高 → 网格宽 1。
    let grid = from_pattern(&["X", "#"], &[('X', id("coal")), ('#', id("stick"))]);
    assert_eq!(crafting::find_result(&grid, 1), Some((id("torch"), 4)));
    // charcoal 备选同出（charcoal 本波转正，torch 双材料全注册）。
    let grid = from_pattern(&["X", "#"], &[('X', id("charcoal")), ('#', id("stick"))]);
    assert_eq!(crafting::find_result(&grid, 1), Some((id("torch"), 4)));
    // recipe/bread.json：3×wheat 行 → ×1。
    let grid = from_pattern(&["###"], &[('#', id("wheat"))]);
    assert_eq!(crafting::find_result(&grid, 3), Some((id("bread"), 1)));
    // recipe/furnace.json：3×3 环 #=#minecraft:stone_crafting_materials → ×1。
    let ring = |m: u16| {
        let mut g = [crafting::EMPTY_SLOT; 9];
        for i in [0usize, 1, 2, 3, 5, 6, 7, 8] {
            g[i] = m;
        }
        g
    };
    assert_eq!(
        crafting::find_result(&ring(id("cobblestone")), 3),
        Some((id("furnace"), 1))
    );
    assert_eq!(
        crafting::find_result(&ring(id("blackstone")), 3),
        Some((id("furnace"), 1)),
        "stone_crafting_materials tag 全成员注册后黑石环同出"
    );
    // recipe/iron_ingot_from_nuggets.json：9×iron_nugget → ×1。
    let full = [id("iron_nugget"); 9];
    assert_eq!(
        crafting::find_result(&full, 3),
        Some((mcv_item::IRON_INGOT, 1))
    );
}

#[test]
fn unlocked_smelting_craft_results() {
    // copper_ingot_from_smelting_raw_copper.json。
    assert_eq!(
        crafting::smelt_result(mcv_item::RAW_COPPER),
        Some((id("copper_ingot"), 1))
    );
    // glass.json（sand → glass，方块物品转正后解锁）。
    assert_eq!(
        crafting::smelt_result(mcv_item::SAND_ITEM),
        Some((id("glass"), 1))
    );
    // charcoal.json（log → charcoal；torch 备选材料随之全注册）。
    assert_eq!(
        crafting::smelt_result(mcv_item::LOG),
        Some((id("charcoal"), 1))
    );
    // stone → smooth_stone（smooth_stone.json，方块物品转正后解锁）。
    assert_eq!(
        crafting::smelt_result(mcv_item::STONE_ITEM),
        Some((id("smooth_stone"), 1))
    );
    // 反例：钻石（成品）无熔炼配方。
    assert_eq!(crafting::smelt_result(mcv_item::DIAMOND_ITEM), None);
}
