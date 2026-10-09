//! 挖掘公式（26.1 BlockBehaviour:355-363）表驱动：徒手/正确工具 tick 数、
//! 层级门控（铁矿要石镐、钻石矿要铁镐）、种类不匹配不吃速度、基岩恒 0、
//! 硬度 ≤0 秒碎。tick 数按 hardness 直接推（石头 1.5、泥土 0.5）。

use mcv_core::BlockId;
use mcv_item::mining::{block_mining, destroy_speed, has_correct_tool, progress_per_tick};
use mcv_item::{
    DIAMOND_PICKAXE_INDEX, IRON_PICKAXE_INDEX, ItemStack, STONE_PICKAXE_INDEX, ToolKind,
    WOODEN_AXE_INDEX, WOODEN_PICKAXE_INDEX, WOODEN_SHOVEL_INDEX, mining,
};

fn tool(id: u16) -> Option<ItemStack> {
    Some(ItemStack::new(id, 1))
}

const STONE: BlockId = BlockId(1);
const DIRT: BlockId = BlockId(2);
const LOG: BlockId = BlockId(6);
const PLANKS: BlockId = BlockId(8);
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
    // 层级不足时慢速可破（/100），不是不可破。
    let p = progress_per_tick(IRON_ORE, tool(WOODEN_PICKAXE_INDEX).as_ref());
    assert!(
        p > 0.0
            && (ticks_to_break(IRON_ORE, tool(WOODEN_PICKAXE_INDEX).as_ref()) - 300.0).abs() < 1e-3
    );
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
    // 未注册进挖掘表的方块：徒手正确、无工具速度（宁缺勿错）。
    assert!(!has_correct_tool(COBBLE, None), "圆石要镐");
    let bm = block_mining(BlockId(485));
    assert!(bm.need.is_none() && bm.speed_tool.is_none());
    assert_eq!(block_mining(STONE).speed_tool, Some(ToolKind::Pickaxe));
    assert_eq!(mining::mining_tier(mcv_item::COPPER), 3);
}
