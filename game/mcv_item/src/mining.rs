//! 挖掘规则（26.1 `BlockBehaviour#getDestroyProgress` / `ToolManager` 语义的
//! 已注册子集）：
//!
//! - 每 tick 进度 = `getDestroySpeed(手持, 方块) / hardness / (30 | 100)`，
//!   分母 30 = 工具对掉落正确、100 = 不正确（徒手也能磨但慢 3.33 倍）。
//! - 工具速度只在**种类**匹配方块可挖类型时生效，否则 1.0；
//!   `hasCorrectToolForDrops` 还要求**层级**达标（石镐挖不动铁矿掉落）。
//! - 掉落受同一门控约束（错误工具破坏不掉东西，见 drop_for_block）。
//! - 硬度无限（基岩）→ 进度恒 0，永不可破。
//! - 工具只覆盖已注册方块子集；未知方块按徒手无要求处理
//!   （TODO(registry):原版 mineable/* 标签全量进表后由数据驱动）。

use crate::{
    COPPER, DIAMOND, GOLD, IRON, ItemKind, ItemStack, NETHERITE, STONE_M, ToolMaterial, WOOD,
};
use mcv_core::BlockId;

/// 可挖工具种类（原版 `mineable/*` 标签）。剑对树叶/南瓜的加速未注册，跳过。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    Pickaxe,
    Axe,
    Shovel,
}

/// 挖掘层级（原版 ToolTiers：木/金=1、石=2、铁=3、钻/下界=4；本引擎自加
/// 铜工具插在石铁之间 = 2.5 取整 → 3）。
pub fn mining_tier(m: ToolMaterial) -> u8 {
    if m == WOOD || m == GOLD {
        1
    } else if m == STONE_M {
        2
    } else if m == COPPER {
        3
    } else if m == IRON {
        4
    } else if m == DIAMOND || m == NETHERITE {
        5
    } else {
        // 未知材料按木级兜底（ITEMS 只用已知材料，不会走到）。
        1
    }
}

/// 方块的挖掘属性（速度工具种类 + 掉落所需工具 + 不可破坏）。
#[derive(Clone, Copy, Debug)]
pub struct BlockMining {
    /// 匹配此种类的工具才提供 speed；None = 只按徒手 1.0。
    pub speed_tool: Option<ToolKind>,
    /// 掉落正确性：(必需种类, 最低层级)；None = 徒手即正确。
    pub need: Option<(ToolKind, u8)>,
}

/// 已注册子集的挖掘表（键 = blocks_gen 注册名；数字对齐原版数值/层级）。
pub fn block_mining(id: BlockId) -> BlockMining {
    let name = mcv_core::BLOCKS[id.id() as usize].name;
    // 镐系：石头家族木镐起、煤矿石镐、铁矿深板岩系石镐、钻石铁镐。
    let pick = |tier: u8| BlockMining {
        speed_tool: Some(ToolKind::Pickaxe),
        need: Some((ToolKind::Pickaxe, tier)),
    };
    match name {
        "stone" | "cobble" => pick(1),
        "coal_ore" => pick(1),
        "iron_ore" => pick(2),
        "diamond_ore" => pick(4),
        "log" | "planks" => BlockMining {
            speed_tool: Some(ToolKind::Axe),
            need: None,
        },
        "dirt" | "grass" | "sand" | "snow_grass" => BlockMining {
            speed_tool: Some(ToolKind::Shovel),
            need: None,
        },
        _ => BlockMining {
            speed_tool: None,
            need: None,
        },
    }
}

/// 手持物品的工具规格（方块物品/杂物 = None）。
pub fn tool_of(stack: Option<&ItemStack>) -> Option<(ToolKind, ToolMaterial)> {
    let kind = stack?.def().kind;
    match kind {
        ItemKind::Pickaxe(m) => Some((ToolKind::Pickaxe, m)),
        ItemKind::Axe(m) => Some((ToolKind::Axe, m)),
        ItemKind::Shovel(m) => Some((ToolKind::Shovel, m)),
        _ => None,
    }
}

/// 26.1 `ToolManager.withCorrectToolForDrops` 子集：种类 + 层级双达标。
pub fn has_correct_tool(block: BlockId, stack: Option<&ItemStack>) -> bool {
    match block_mining(block).need {
        None => true,
        Some((need_kind, tier)) => match tool_of(stack) {
            Some((k, m)) => k == need_kind && mining_tier(m) >= tier,
            None => false,
        },
    }
}

/// 26.1 `ItemStack#getDestroySpeed` 子集：种类匹配才吃工具速度，否则 1.0。
/// （效率附魔/急迫效果未实现，跳过。）
pub fn destroy_speed(block: BlockId, stack: Option<&ItemStack>) -> f32 {
    if let (Some((kind, m)), Some(want)) = (tool_of(stack), block_mining(block).speed_tool)
        && kind == want
    {
        return f32::from(m.speed);
    }
    1.0
}

/// 每游戏 tick（1/20 s）的破坏进度。0 = 不可破坏（硬度无限，基岩）；
/// 硬度 ≤0 = 瞬碎（花草类，返回 ∞ 让调用方走秒破分支）。
pub fn progress_per_tick(block: BlockId, stack: Option<&ItemStack>) -> f32 {
    let hardness = mcv_core::BLOCKS[block.id() as usize].hardness;
    if hardness.is_infinite() {
        return 0.0;
    }
    if hardness <= 0.0 {
        return f32::INFINITY;
    }
    let modifier = if has_correct_tool(block, stack) {
        30.0
    } else {
        100.0
    };
    destroy_speed(block, stack) / hardness / modifier
}
