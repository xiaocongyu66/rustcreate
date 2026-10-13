//! 挖掘规则（26.1 `BlockBehaviour#getDestroyProgress` / `Tool` 组件语义的
//! 数据驱动全量版，#178 刀2b）：
//!
//! - 每 tick 进度 = `getDestroyProgress`：`getDestroySpeed(手持, 方块) /
//!   hardness / (30 | 100)`，分母 30 = 工具对掉落正确、100 = 不正确
//!   （Player.java:586-619 `hasCorrectToolForDrops`）。
//! - 工具速度只在方块命中该工具的 `mineable/*` tag 时生效（原版
//!   `minesAndDrops` 规则；248 个原版 tag 已在 `mcv_core::tags`，
//!   手写速度族硬编码已删净，TODO(registry) 销账）。
//! - 掉落正确性：`!requiresCorrectToolForDrops || Tool.isCorrectForDrops`
//!   （Player.java:619）——数据侧 = `mcv_core::tool`（掉落门生成列 +
//!   mineable/needs tag），`has_correct_tool` 还要求层级达标。
//! - 掉落受同一门控约束（见 inventory::drops_for_block 的调用门）。
//! - 硬度无限（基岩）→ 进度恒 0，永不可破。
//!
//! 已知未建模登记（#180 波）：① 锄物品在 ITEMS 是 `ItemKind::Material`
//! （非工具），[`ToolKind::Hoe`] 暂由方块侧 tag 产生、无手持侧生产者，
//! 锄具接线（ItemKind::Hoe + 原版数值）待物品波；② 剑/剪刀对 cobweb 等
//! 的专用 Tool 组件规则（对应方块 = tool::Requirement::Never，任何工具
//! 不正确，与「剪刀可掉」的原版差异登记于此）；③ crafter 原版怪癖
//! （无掉落门但 incorrect_for_wooden/gold 含它 → 木/金镐挖不掉、徒手反
//! 可掉，Blocks.java:5753）——引擎掉落表两者都掉，速度档木/金镐 30 vs
//! 原版 100。

use crate::{
    COPPER, DIAMOND, GOLD, IRON, ItemKind, ItemStack, NETHERITE, STONE_M, ToolMaterial, WOOD,
};
use mcv_core::BlockId;
use mcv_registry::TagEntry;

/// 可挖工具种类（原版 `mineable/*` 四 tag 全量，方块侧 tag 查询产出；
/// 手持侧现只有镐/斧/铲三类物品，Hoe 见模块头登记①）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    Pickaxe,
    Axe,
    Shovel,
    /// 原版 MINEABLE_WITH_HOE（29 成员，泥/草翻土族 + 瓜/叶等）。引擎
    /// 锄物品尚未工具化（登记①），查询侧已按原版落地。
    Hoe,
}

/// 方块掉落门（原版 `requiresCorrectToolForDrops` ∩ `Tool.Rule` 匹配）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropNeed {
    /// 无掉落门：徒手即正确（Player.java:619 `!requiresCorrectToolForDrops`）。
    None,
    /// 需要种类 + 最低层级（`mining_tier` 序）的工具。
    Tool(ToolKind, u8),
    /// gate=true 且不在任何 mineable tag（命令块族/cobweb）：一切工具
    /// `Tool.isCorrectForDrops` = false（Tool.java:50-57）。
    Never,
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
    /// 匹配此种类的工具才提供 speed；None = 只按徒手 1.0
    /// （原版 Tool.defaultMiningSpeed）。
    pub speed_tool: Option<ToolKind>,
    /// 掉落正确性门。
    pub need: DropNeed,
}

/// `mineable/<tool>` tag → 种类（四 tag 两两交集 = 0，实测登记见刀2
/// 交卷报告；至多命中一个，查序不影响结果）。
fn speed_kind(row: u16) -> Option<ToolKind> {
    let hit = |t: &TagEntry| t.set.contains(row);
    if hit(&mcv_core::tags::TAG_BLOCK_MINEABLE__PICKAXE) {
        Some(ToolKind::Pickaxe)
    } else if hit(&mcv_core::tags::TAG_BLOCK_MINEABLE__AXE) {
        Some(ToolKind::Axe)
    } else if hit(&mcv_core::tags::TAG_BLOCK_MINEABLE__SHOVEL) {
        Some(ToolKind::Shovel)
    } else if hit(&mcv_core::tags::TAG_BLOCK_MINEABLE__HOE) {
        Some(ToolKind::Hoe)
    } else {
        None
    }
}

/// 全表挖掘属性（#178 刀2b：速度种类 = `mineable/*` tag 全量查询；
/// 掉落门 = `mcv_core::tool`（GEN_BLOCKS requires_tool 列 + needs_*_tool
/// tag）。snow_grass 等引擎状态行经 `mcv_core::tag_row_of` 别名回官方行）。
pub fn block_mining(id: BlockId) -> BlockMining {
    let row = mcv_core::tag_row_of(id.id());
    BlockMining {
        speed_tool: speed_kind(row),
        need: match mcv_core::tool::requirement_by_id(id) {
            Some(mcv_core::tool::Requirement::Tool { kind, tier }) => DropNeed::Tool(
                match kind {
                    mcv_core::tool::ToolReq::Pickaxe => ToolKind::Pickaxe,
                    mcv_core::tool::ToolReq::Shovel => ToolKind::Shovel,
                },
                tier,
            ),
            Some(mcv_core::tool::Requirement::Never) => DropNeed::Never,
            None => DropNeed::None,
        },
    }
}

/// 手持物品的工具规格（方块物品/杂物 = None；锄见登记①）。
pub fn tool_of(stack: Option<&ItemStack>) -> Option<(ToolKind, ToolMaterial)> {
    let kind = stack?.def().kind;
    match kind {
        ItemKind::Pickaxe(m) => Some((ToolKind::Pickaxe, m)),
        ItemKind::Axe(m) => Some((ToolKind::Axe, m)),
        ItemKind::Shovel(m) => Some((ToolKind::Shovel, m)),
        _ => None,
    }
}

/// 26.1 `Player.java:619` `hasCorrectToolForDrops`：无门恒真；Never 恒
/// 假（Tool.java:50-57 无规则匹配）；否则种类 + 层级双达标。
pub fn has_correct_tool(block: BlockId, stack: Option<&ItemStack>) -> bool {
    match block_mining(block).need {
        DropNeed::None => true,
        DropNeed::Never => false,
        DropNeed::Tool(need_kind, tier) => match tool_of(stack) {
            Some((k, m)) => k == need_kind && mining_tier(m) >= tier,
            None => false,
        },
    }
}

/// 26.1 `Tool.getMiningSpeed`（Tool.java:39-48）：方块命中本工具的
/// minesAndDrops 规则（= `mineable/<kind>` tag）才吃工具速度，否则
/// defaultMiningSpeed = 1.0。（效率附魔/急迫效果未实现，跳过。）
pub fn destroy_speed(block: BlockId, stack: Option<&ItemStack>) -> f32 {
    if let (Some((kind, m)), Some(want)) = (tool_of(stack), block_mining(block).speed_tool)
        && kind == want
    {
        return f32::from(m.speed);
    }
    1.0
}

/// 带 Tool 组件物品挖掘一次的耐久消耗（26.1 `Item.mineBlock` 的
/// `tool.damagePerBlock()`，`Item.java:257-268`）：镐/斧/锹 1
/// （ToolMaterial.applyToolProperties，ToolMaterial.java:37-63 末参 1）、
/// 剑 2（applySwordProperties，ToolMaterial.java:79-94 末参 2）；其余物品
/// 无 Tool 组件 → 0。**与掉落门解耦**：原版耐久不问 correct-for-drops，
/// 木镐挖不动钻石矿的掉落但照样掉耐久（ServerPlayerGameMode.java:296
/// `itemStack.mineBlock` 无条件于 canDestroy 之前执行）。
pub fn mine_damage(kind: ItemKind) -> u16 {
    match kind {
        ItemKind::Pickaxe(_) | ItemKind::Axe(_) | ItemKind::Shovel(_) => 1,
        ItemKind::Sword(_) => 2,
        _ => 0,
    }
}

/// 26.1 `ServerPlayerGameMode.destroyBlock` 耐久段（:296 `itemStack.mineBlock`
/// → `Item.mineBlock`，`Item.java:262`）的完整判定：目标方块
/// `destroySpeed != 0`（硬度非 0；花草 0 不扣，基岩 -1 不可破无所谓）且手持
/// 带 Tool 组件时，返回本次成功挖掘应扣的耐久值；否则 0。创造豁免由调用方
/// 的模式门处理（26.1 `ItemStack.processDurabilityChange` :454-456
/// `hasInfiniteMaterials` → 0）。耐久耗尽的销毁走 `ItemStack::hurt`
/// 返回 true → 调用方清槽（applyDamage → shrink(1)，`ItemStack.java:466-468`）。
pub fn mine_durability_cost(block: BlockId, stack: Option<&ItemStack>) -> u16 {
    if mcv_core::BLOCKS[block.id() as usize].hardness == 0.0 {
        return 0;
    }
    stack.map(|s| mine_damage(s.def().kind)).unwrap_or(0)
}

/// 空中挖掘速度除数（26.1 `Player#getDestroySpeed`：`!onGround()` → `speed /= 5`，
/// `world/entity/player/Player.java:611-612`；与引擎参考实现
/// `mcv_game::mining::progress_per_tick` 同值，那边是同一公式的无背包版本）。
pub const AIR_MINING_DIVISOR: f32 = 5.0;

/// 眼在水中挖掘速度倍率（26.1 `Player.java:607-608` `isEyeInFluid(WATER)` →
/// `Attributes.SUBMERGED_MINING_SPEED` 默认 0.2，`Attributes.java:88-90`。
/// 判定基准是**眼睛**所在流体，不是脚、也不是目标方块）。
pub const SUBMERGED_MINING_SPEED: f32 = 0.2;

/// 每游戏 tick 进度，带 26.1 惩罚链的完整版（`Player#getDestroySpeed` 尾部
/// 两段，`Player.java:586-614`）：`on_ground = false`（空中）速度 ÷5、
/// `submerged = true`（眼在水中）速度 ×0.2。原版每 tick 重算
/// `getDestroyProgress`（`ServerPlayerGameMode.tick()` :107-130），所以跳起/
/// 入水当 tick 速率即变。惩罚乘在速度上再除 hardness/modifier，与原式
/// 乘法交换等价。
pub fn progress_per_tick_env(
    block: BlockId,
    stack: Option<&ItemStack>,
    on_ground: bool,
    submerged: bool,
) -> f32 {
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
    let mut speed = destroy_speed(block, stack);
    if !on_ground {
        speed /= AIR_MINING_DIVISOR;
    }
    if submerged {
        speed *= SUBMERGED_MINING_SPEED;
    }
    speed / hardness / modifier
}

/// 每游戏 tick（1/20 s）的破坏进度（地面 + 不在水中，无惩罚基准）。
/// 0 = 不可破坏（硬度无限，基岩）；硬度 ≤0 = 瞬碎（花草类，返回 ∞ 让调用方走秒破分支）。
/// 空中/水下惩罚版见 [`progress_per_tick_env`]。
pub fn progress_per_tick(block: BlockId, stack: Option<&ItemStack>) -> f32 {
    progress_per_tick_env(block, stack, true, false)
}
