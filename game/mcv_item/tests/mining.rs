//! 挖掘公式（26.1 BlockBehaviour:355-363）表驱动：徒手/正确工具 tick 数、
//! 层级门控（铁矿要石镐、钻石矿要铁镐）、种类不匹配不吃速度、基岩恒 0、
//! 硬度 ≤0 秒碎。tick 数按 hardness 直接推（石头 1.5、泥土 0.5）。

use mcv_core::BlockId;
use mcv_item::mining::{block_mining, destroy_speed, has_correct_tool, progress_per_tick};
use mcv_item::{
    DIAMOND_PICKAXE_INDEX, IRON, IRON_PICKAXE_INDEX, IRON_SWORD_INDEX, ItemKind, ItemStack,
    STONE_PICKAXE_INDEX, ToolKind, WOODEN_AXE_INDEX, WOODEN_PICKAXE_INDEX, WOODEN_SHOVEL_INDEX,
    mining,
};

fn tool(id: u16) -> Option<ItemStack> {
    Some(ItemStack::new(id, 1))
}

const STONE: BlockId = BlockId(1);
const DIRT: BlockId = BlockId(2);
const LOG: BlockId = BlockId(6);
const COBBLE: BlockId = BlockId(9);
const BEDROCK: BlockId = BlockId(10);
const FLOWER: BlockId = BlockId(12);
const COAL_ORE: BlockId = BlockId(217);
const DIAMOND_ORE: BlockId = BlockId(361);
const IRON_ORE: BlockId = BlockId(484);

/// 破坏所需 tick 数（1/20 s 一步），供断言精确到小数。
fn ticks_to_break(block: BlockId, stack: Option<&ItemStack>) -> f32 {
    1.0 / progress_per_tick(block, stack)
}

#[test]
fn vanilla_break_times() {
    // 石头徒手：无正确工具 → /100 → 150 tick = 7.5 s（wiki 一致）。
    assert!((ticks_to_break(STONE, None) - 150.0).abs() < 1e-3);
    // 木镐(speed 2)正确 → /30 → 22.5 tick ≈ 1.125 s（wiki 1.15）。
    assert!(
        (ticks_to_break(STONE, tool(WOODEN_PICKAXE_INDEX).as_ref()) - 22.5).abs() < 1e-3,
        "wiki: 木镐石头 1.15s"
    );
    // 钻石镐(speed 8)石头：30/8*1.5 = 5.625 tick。
    assert!((ticks_to_break(STONE, tool(DIAMOND_PICKAXE_INDEX).as_ref()) - 5.625).abs() < 1e-3);
    // 泥土徒手（无工具要求=正确）：30*0.5 = 15 tick = 0.75 s。
    assert!((ticks_to_break(DIRT, None) - 15.0).abs() < 1e-3);
    // 木铲(speed 2)泥土：30/2*0.5 = 7.5 tick。
    assert!((ticks_to_break(DIRT, tool(WOODEN_SHOVEL_INDEX).as_ref()) - 7.5).abs() < 1e-3);
    // 圆石硬度 2：徒手 200 tick。
    assert!((ticks_to_break(COBBLE, None) - 200.0).abs() < 1e-3);
}

#[test]
fn tool_kind_mismatch_gives_no_speed() {
    // 斧头挖石头：速度不生效（1.0），且不算正确工具（掉落也要镐）。
    assert_eq!(destroy_speed(STONE, tool(WOODEN_AXE_INDEX).as_ref()), 1.0);
    assert!(!has_correct_tool(STONE, tool(WOODEN_AXE_INDEX).as_ref()));
    // 斧头挖原木才吃速度。
    assert_eq!(destroy_speed(LOG, tool(WOODEN_AXE_INDEX).as_ref()), 2.0);
    assert!(has_correct_tool(LOG, tool(WOODEN_AXE_INDEX).as_ref()));
    assert!(has_correct_tool(LOG, None), "原木无掉落门控,徒手也算正确");
}

#[test]
fn tier_gate_on_ore_drops() {
    // 铁矿：石镐(tier2)才正确；木镐(tier1)能磨但错工具。
    assert!(has_correct_tool(
        IRON_ORE,
        tool(STONE_PICKAXE_INDEX).as_ref()
    ));
    assert!(!has_correct_tool(
        IRON_ORE,
        tool(WOODEN_PICKAXE_INDEX).as_ref()
    ));
    assert!(!has_correct_tool(IRON_ORE, None));
    // 钻石矿：铁镐(tier4)起。
    assert!(has_correct_tool(
        DIAMOND_ORE,
        tool(IRON_PICKAXE_INDEX).as_ref()
    ));
    assert!(!has_correct_tool(
        DIAMOND_ORE,
        tool(STONE_PICKAXE_INDEX).as_ref()
    ));
    assert!(has_correct_tool(
        DIAMOND_ORE,
        tool(DIAMOND_PICKAXE_INDEX).as_ref()
    ));
    // 煤矿：木镐即正确。
    assert!(has_correct_tool(
        COAL_ORE,
        tool(WOODEN_PICKAXE_INDEX).as_ref()
    ));
    assert!(!has_correct_tool(COAL_ORE, None));
    // 矿石族全量对齐 26.1 needs_*_tool tag（数据表见 mcv_core::tool）：
    // 铁/铜/青金 = 石镐级（needs_stone_tool），金/红石/钻石/绿宝石 = 铁镐级
    //（needs_iron_tool）；深板岩矿同层（Blocks.java ofLegacyCopy 继承门 +
    // tag 显式列名）。表驱动逐行：
    let stone_pick = tool(STONE_PICKAXE_INDEX);
    let iron_pick = tool(IRON_PICKAXE_INDEX);
    // (矿石方块 id, 深板岩变体 id, 层级)
    let ore_table: &[(u16, u16, u8)] = &[
        (217, 347, 1), // coal_ore / deepslate_coal_ore
        (484, 352, 2), // iron_ore / deepslate_iron_ore
        (241, 348, 2), // copper_ore / deepslate_copper_ore
        (513, 353, 2), // lapis_ore / deepslate_lapis_ore
        (425, 351, 4), // gold_ore / deepslate_gold_ore
        (877, 354, 4), // redstone_ore / deepslate_redstone_ore
        (361, 349, 4), // diamond_ore / deepslate_diamond_ore
        (376, 350, 4), // emerald_ore / deepslate_emerald_ore
    ];
    for (ore, ds, tier) in ore_table {
        let want_pick = |t: u8| match t {
            1 => WOODEN_PICKAXE_INDEX,
            2 => STONE_PICKAXE_INDEX,
            _ => IRON_PICKAXE_INDEX,
        };
        assert!(
            has_correct_tool(BlockId(*ore), tool(want_pick(*tier)).as_ref()),
            "{} 层级门",
            mcv_core::BLOCKS[*ore as usize].name
        );
        assert!(
            !has_correct_tool(BlockId(*ore), None),
            "{} 徒手非正确工具（26.1 徒手挖矿不掉）",
            mcv_core::BLOCKS[*ore as usize].name
        );
        assert!(
            !has_correct_tool(BlockId(*ds), None),
            "{} 深板岩矿徒手非正确工具",
            mcv_core::BLOCKS[*ds as usize].name
        );
        assert!(
            has_correct_tool(
                BlockId(*ds),
                if *tier <= 2 {
                    stone_pick.as_ref()
                } else {
                    iron_pick.as_ref()
                }
            ),
            "{} 深板岩矿层级门",
            mcv_core::BLOCKS[*ds as usize].name
        );
    }
    // 层级不足仍慢速可破（/100），且速度按种类匹配吃镐速（26.1：层级只管
    // 掉落正确性，不管速度）——木镐挖铁矿 = 2/3/100 → 150 tick。
    let p = progress_per_tick(IRON_ORE, tool(WOODEN_PICKAXE_INDEX).as_ref());
    assert!(p > 0.0);
    assert!((ticks_to_break(IRON_ORE, tool(WOODEN_PICKAXE_INDEX).as_ref()) - 150.0).abs() < 1e-3);
    // 徒手铁矿（速度 1、错工具）：100×3 = 300 tick。
    assert!((ticks_to_break(IRON_ORE, None) - 300.0).abs() < 1e-3);
}

#[test]
fn bedrock_never_and_flowers_instant() {
    assert_eq!(progress_per_tick(BEDROCK, None), 0.0);
    assert_eq!(
        progress_per_tick(BEDROCK, tool(DIAMOND_PICKAXE_INDEX).as_ref()),
        0.0
    );
    // 花硬度 0 → ∞（调用方按秒破处理）。
    assert!(progress_per_tick(FLOWER, None).is_infinite());
}

#[test]
fn unregistered_blocks_are_hand_mineable_plain() {
    // 不在工具表/速度族的方块：徒手正确、无工具速度（宁缺勿错）。
    assert!(!has_correct_tool(COBBLE, None), "圆石要镐");
    let bm = block_mining(BlockId(419)); // glass：无掉落门、无速度族
    assert!(bm.need.is_none() && bm.speed_tool.is_none());
    assert_eq!(block_mining(STONE).speed_tool, Some(ToolKind::Pickaxe));
    assert_eq!(mining::mining_tier(mcv_item::COPPER), 3);
}

/// 耐久消耗对齐 26.1 `Item.mineBlock`（Item.java:257-268，任务板 #90）：
/// 带 Tool 组件手持物镐/斧/锹 1、剑 2（ToolMaterial.applyToolProperties /
/// applySwordProperties 的 damagePerBlock）；无 Tool 组件（空手/方块物品/
/// 木棍）0；硬度 0（花草，destroySpeed == 0）不扣；**与掉落门解耦**——
/// 错误工具挖不动掉落但照样耗（ServerPlayerGameMode.java:296 无条件执行）。
#[test]
fn mine_durability_matches_item_mine_block() {
    use mcv_item::mining::{mine_damage, mine_durability_cost};
    let pick = ItemStack::new(IRON_PICKAXE_INDEX, 1);
    let sword = ItemStack::new(IRON_SWORD_INDEX, 1);
    assert_eq!(mine_durability_cost(STONE, Some(&pick)), 1, "镐每次 1");
    assert_eq!(mine_durability_cost(STONE, Some(&sword)), 2, "剑每次 2");
    // 无 Tool 组件 → 0。
    assert_eq!(mine_durability_cost(STONE, None), 0, "徒手不耗");
    let block_item = ItemStack::new(27, 1); // cobblestone 方块物品
    assert_eq!(mine_durability_cost(STONE, Some(&block_item)), 0);
    let stick = ItemStack::new(mcv_item::STICK, 1);
    assert_eq!(mine_durability_cost(STONE, Some(&stick)), 0);
    // 硬度 0（花）：destroySpeed == 0 → 不扣。
    assert_eq!(mine_durability_cost(FLOWER, Some(&pick)), 0);
    // 解耦：木镐挖钻石矿（不掉落）照样耗 1。
    assert_eq!(
        mine_durability_cost(DIAMOND_ORE, Some(&ItemStack::new(13, 1))),
        1
    );
    // damagePerBlock 原值对账。
    assert_eq!(mine_damage(ItemKind::Pickaxe(IRON)), 1);
    assert_eq!(mine_damage(ItemKind::Sword(IRON)), 2);
}

/// 耐久耗尽销毁：每次成功挖掘 damage 1，累计到 max_damage → `hurt` 返回
/// true，调用方清槽（26.1 `applyDamage` → `shrink(1)`，ItemStack.java:466-468）。
#[test]
fn tool_breaks_when_durability_exhausted_by_mining() {
    let mut wooden = ItemStack::new(WOODEN_PICKAXE_INDEX, 1); // 木镐耐久 59
    let mut rng = || 0u32;
    for i in 0..59 {
        assert!(!wooden.hurt(1, &mut rng), "第 {} 次挖掘不该坏", i + 1);
    }
    assert_eq!(wooden.damage, 59);
    assert!(wooden.hurt(1, &mut rng), "第 60 次 damage≥max → 工具销毁");
}
