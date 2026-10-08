//! 方块挖掘进度：机制与数值取自反编译 Minecraft 26.1（只提取机制，不搬运代码）。
//!
//! 参照（本机 `/root/mc-ref/src-26.1/net/minecraft/`，仓库外，禁止入库）：
//!
//! - `world/level/block/state/BlockBehaviour.java:355-363`（Blocks#getDestroyProgress）：
//!   ```text
//!   hardness = state.getDestroySpeed(level, pos)   // 注册表 strength 硬度；-1 = 不可挖
//!   if (hardness == -1) return 0                   // 基岩等：进度恒 0
//!   modifier = hasCorrectToolForDrops(player, state) ? 30 : 100
//!   progress_per_tick = player.getDestroySpeed(state) / hardness / modifier
//!   ```
//!   每 tick（1/20 s）累加一次；硬度 0 的方块（花、草类）除以 0 得 +∞ ≥ 1，
//!   按下瞬间破坏。
//! - `server/level/ServerPlayerGameMode.java:114-141`：progress ≥ 1.0 → 破坏；
//!   裂纹阶段 = `(progress * 10) as i32`（0..9）。
//! - `world/entity/player/Player.java:586-620`（getDestroySpeed 及 hasCorrectToolForDrops）：
//!   空手速度 1.0（`Item.java:183-185`，无 Tool 组件即 1.0）；
//!   不站立地面（空中）`speed /= 5`；眼在水中 `speed *= 0.2`
//!   （`Attributes.SUBMERGED_MINING_SPEED` 默认值）；
//!   `requiresCorrectToolForDrops == false` 的方块（泥土/沙/木/花等）空手也算
//!   "正确工具"走 30 档，石头类需要镐、空手走 100 档 —— 即"空手惩罚"。
//!
//! tick→秒 换算与完整推导见仓库外笔记 `/root/mc-ref/NOTES-physics.md`。

use mcv_core::{BlockId, BlockPos};

/// 与 `mcv_core::BLOCKS` 注册顺序一致的方块 id。
/// 待 mcv_core 导出命名 id 常量后迁移（另一代理正在校准 BLOCKS）。
const STONE: u8 = 1;
const COBBLE: u8 = 9;

/// MC tick 长度（秒）：挖掘进度按 tick 制定义，连续帧按 `dt / MC_TICK` 折算。
pub const MC_TICK: f32 = 1.0 / 20.0;

/// 正确工具（可掉落）时的进度分母，MC `modifier`（BlockBehaviour.java:361）。
pub const MODIFIER_CORRECT: f32 = 30.0;
/// 错误工具 / 空手挖需要工具的方块时的进度分母（同上行）。
/// 相对 30 档即 MC 的空手惩罚：同方块同速度慢 100/30 ≈ 3.33 倍。
pub const MODIFIER_INCORRECT: f32 = 100.0;

/// 空手挖掘速度（MC 无 Tool 组件时 getDestroySpeed 返回 1.0）。
pub const BARE_HAND_SPEED: f32 = 1.0;

// ---- 工具倍率接口点（背包系统未实现，此处仅预留档位常数） ----
// MC `ToolMaterial`（ToolMaterial.java:23-31）的 speed 字段。规则：仅当手持
// 工具的 Tool.Rule.minesAndDrops 命中该方块时速度取档位值，否则仍为 1.0。
// 背包/物品系统就绪后，应改为从手持物品构造 [`HeldTool`]（speed=档位值、
// correct_for_drops=isCorrectToolForDrops 判定结果）；本 crate 不发明背包。

/// 木质档倍率（ToolMaterial.WOOD）。
pub const TOOL_SPEED_WOOD: f32 = 2.0;
/// 石质档倍率（ToolMaterial.STONE）。
pub const TOOL_SPEED_STONE: f32 = 4.0;
/// 铜质档倍率（ToolMaterial.COPPER，26.1 新增档）。
pub const TOOL_SPEED_COPPER: f32 = 5.0;
/// 铁质档倍率（ToolMaterial.IRON）。
pub const TOOL_SPEED_IRON: f32 = 6.0;
/// 钻石档倍率（ToolMaterial.DIAMOND）。
pub const TOOL_SPEED_DIAMOND: f32 = 8.0;
/// 金质档倍率（ToolMaterial.GOLD）。
pub const TOOL_SPEED_GOLD: f32 = 12.0;
/// 下界合金档倍率（ToolMaterial.NETHERITE）。
pub const TOOL_SPEED_NETHERITE: f32 = 9.0;

/// 手持物品的挖掘状态 —— **工具倍率接口点**。
///
/// 当前只有空手实现（[`HeldTool::BARE_HAND`]）：`speed = 1.0`、
/// `correct_for_drops = false`。MC 的 `correct_for_drops` 按目标方块判定
/// （`ItemStack#isCorrectToolForDrops(state)`），本结构按"当前挖掘目标"
/// 预先求值传入；接背包系统时只需替换构造逻辑，公式不动。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeldTool {
    /// 工具档位倍率（见 [`TOOL_SPEED_WOOD`] 等）；1.0 = 空手或不适用工具。
    pub speed: f32,
    /// 对当前目标方块是否为"正确工具"（MC hasCorrectToolForDrops 的手持侧判定）。
    pub correct_for_drops: bool,
}

impl HeldTool {
    /// 空手：速度 1.0，对需要工具的方块非正确工具。
    pub const BARE_HAND: HeldTool = HeldTool {
        speed: BARE_HAND_SPEED,
        correct_for_drops: false,
    };
}

impl Default for HeldTool {
    fn default() -> Self {
        Self::BARE_HAND
    }
}

/// 方块硬度查表（MC 注册表 strength，秒制基准；`f32::INFINITY` = 不可挖）。
///
/// 现直接读 `mcv_core::BLOCKS[].hardness`（该字段已由 BLOCKS 校准代理按
/// 26.1 Blocks.java 对齐：stone 1.5、cobble 2.0、log/planks 2.0、dirt 0.5、
/// bedrock 用 INFINITY 表示 MC 的 -1）。保留本函数作为 mcv_game 内的
/// 稳定入口；MC 的 `requiresCorrectToolForDrops` 语义字段若将来进 BLOCKS，
/// 一并迁移 [`requires_correct_tool`]。
pub fn hardness(id: BlockId) -> f32 {
    id.def().hardness
}

/// 该方块是否"需要正确工具才可掉落"（MC `requiresCorrectToolForDrops`）。
///
/// 临时查表：mcv_core::BlockDef 尚无对应字段（另一代理正在改 BLOCKS），
/// 先按 MC 26.1 语义覆盖本引擎方块集合 —— 石头/圆石需镐，其余（泥土/沙/
/// 草/木/木板/叶/花）不需工具。**待 BLOCKS 字段就绪后迁移。**
pub fn requires_correct_tool(id: BlockId) -> bool {
    matches!(id.0, STONE | COBBLE)
}

/// MC `Player#hasCorrectToolForDrops(state)`：方块不需工具 → 恒真；
/// 需工具 → 取手持侧判定（空手为假 → 走 [`MODIFIER_INCORRECT`] 档）。
pub fn has_correct_tool_for_drops(id: BlockId, tool: &HeldTool) -> bool {
    !requires_correct_tool(id) || tool.correct_for_drops
}

/// 单个 MC tick（1/20 s）的挖掘进度增量，对应 MC getDestroyProgress 返回值。
///
/// 返回 `0.0`：不可挖（基岩，MC hardness == -1 / 本引擎 INFINITY；
/// 或速度 ≤ 0）；返回 `f32::INFINITY`：硬度 ≤ 0 的瞬间破坏方块（花等）。
pub fn progress_per_tick(
    id: BlockId,
    tool: &HeldTool,
    on_ground: bool,
    submerged: bool,
) -> f32 {
    let h = hardness(id);
    if h.is_infinite() {
        return 0.0; // MC: destroySpeed == -1 → return 0
    }
    if h <= 0.0 {
        return f32::INFINITY; // MC: 除以 0 → +∞ ≥ 1 → 瞬间破坏
    }
    let modifier = if has_correct_tool_for_drops(id, tool) {
        MODIFIER_CORRECT
    } else {
        MODIFIER_INCORRECT
    };
    let mut speed = tool.speed;
    if !on_ground {
        speed /= 5.0; // Player.java:611-613 空中挖掘惩罚
    }
    if submerged {
        speed *= 0.2; // Player.java:607-609，SUBMERGED_MINING_SPEED 默认 0.2
    }
    if speed <= 0.0 {
        return 0.0;
    }
    speed / h / modifier
}

/// 连续制接口：`dt` 秒内的进度增量（= [`progress_per_tick`] × dt 折算 tick 数）。
/// [`crate::consts::FIXED_DT`]（1/60 s）下每帧约得 tick 值的 1/3。
pub fn progress_for(
    id: BlockId,
    tool: &HeldTool,
    on_ground: bool,
    submerged: bool,
    dt: f32,
) -> f32 {
    scale_by_dt(progress_per_tick(id, tool, on_ground, submerged), dt)
}

/// 瞬时进度（per tick）按 dt 秒折算；INF 保持 INF（避免 dt=0 时 0·INF=NaN）。
fn scale_by_dt(per_tick: f32, dt: f32) -> f32 {
    if per_tick.is_infinite() {
        f32::INFINITY
    } else {
        per_tick * (dt / MC_TICK)
    }
}

/// 预期总挖掘时长（秒）：`hardness × modifier × MC_TICK / speed`。
/// `INFINITY` = 不可挖；`0.0` = 瞬间破坏。用于数值对照与 HUD 预估。
pub fn break_seconds(
    id: BlockId,
    tool: &HeldTool,
    on_ground: bool,
    submerged: bool,
) -> f32 {
    let per = progress_per_tick(id, tool, on_ground, submerged);
    if per == 0.0 {
        f32::INFINITY
    } else if per.is_infinite() {
        0.0
    } else {
        MC_TICK / per
    }
}

/// 挖掘累积状态：逐帧 [`DigState::advance`]，进度 ≥ 1 破坏（MC
/// ServerPlayerGameMode 语义）。目标变化自动重置（MC 换目标即清零）。
#[derive(Clone, Copy, Debug, Default)]
pub struct DigState {
    /// 当前挖掘目标格；`None` = 未在挖。
    pub target: Option<BlockPos>,
    /// 累积进度（≥1 时由 advance 清零并报告破坏）。
    pub progress: f32,
}

impl DigState {
    /// 推进 `dt` 秒。返回 `true` = 本步达到 MC 破坏阈值（progress ≥ 1.0），
    /// 状态已自动清空，调用方负责写空气与掉落。
    pub fn advance(
        &mut self,
        pos: BlockPos,
        id: BlockId,
        tool: &HeldTool,
        on_ground: bool,
        submerged: bool,
        dt: f32,
    ) -> bool {
        if self.target != Some(pos) {
            self.target = Some(pos);
            self.progress = 0.0;
        }
        self.progress += scale_by_dt(progress_per_tick(id, tool, on_ground, submerged), dt);
        if self.progress >= 1.0 {
            self.progress = 0.0;
            self.target = None;
            return true;
        }
        false
    }

    /// 裂纹阶段 0..=9，MC `(int)(progress * 10)`（破坏动画分级）。
    pub fn stage(&self) -> u8 {
        (self.progress * 10.0).clamp(0.0, 9.0) as u8
    }

    /// 取消挖掘（松开按键 / 目标丢失）。
    pub fn cancel(&mut self) {
        self.target = None;
        self.progress = 0.0;
    }
}
