//! 流体流动机（水/岩浆挖破支撑不流动波）：`LiquidBlock` 的 scheduled
//! tick 触发面 + `FlowingFluid.tick` spread/fall 决策核 + tick 队列。
//!
//! 机制出处（26.1 反编译树 `/root/mc-ref/src-26.1`，移植铁律 1；缩写：
//! `FF` = `net/minecraft/world/level/material/FlowingFluid.java`、
//! `LB` = `.../block/LiquidBlock.java`、`WF` = `.../material/WaterFluid.java`、
//! `LF` = `.../material/LavaFluid.java`、`GR` = `.../gamerules/GameRules.java`；
//! 行号为反编译实物现状）：
//!
//! - **触发（LB:150-153/:196-207/:180-182）**：`onPlace`/`neighborChanged`/
//!   `updateShape` 三入口统一归 `scheduleTick(pos, fluid, tickDelay)`
//!   （water 5 = WF:112-114、lava 30 = LF:175-177 非下界面）。本仓无方块
//!   更新管线，合成「体素写后对写格+六邻逐格调 [`update_fluid_at` 思路的
//!   宿主面]」触发。
//! - **tick（FF:442-459）**：非源格先 `getNewLiquid`（:160-195）自算新
//!   液位——空 → 自身置空气（:446-449 脱水）；变 → 自写 + schedule 重排
//!   （:450-455）；随后 `spread`（:117-140）向下优先：通过则下层落格、
//!   按「源邻 ≥3 才侧流」（:128）收口；向下不通（或本体是源，:136）→
//!   `spreadToSides`（:142-158）。
//! - **液位语义**：amount 1..8 = 流体满度（源恒 8 且 isSource，WF:149-159）；
//!   落格态 = `LB.LEVEL`（LB:53 + 状态缓存 :69-77 + getFluidState :124-127
//!   钳 8）：0=源、1..7=流动量 8−L、8=falling 满柱——与千块表状态
//!   nibble（bit12-15）同宽。
//! - **常数**：water dropOff 1（WF:107-109）、坡寻 4（WF:92-94）；lava
//!   dropOff 2（LF:165-167 非下界）、坡寻 2（LF:150-152）。源再生 gamerule
//!   默认 water true / lava false（GR:92/:47）。lava 上升延迟 ×4
//!   （LF:180-192，3/4 概率）在 [`spread_delay`]。
//! - **石化**：岩浆格被更新触发扫 上+四横 见水 → 源化黑曜石／非源化圆石
//!   （LB:215-239 shouldSpreadLiquid；扫向 = UP,NORTH,SOUTH,WEST,EAST，
//!   :221-222 relative(getOpposite()) 对 LB:56-58 的反演）；岩浆向下漫进
//!   水格 → 石头（LF:204-218）。蓝冰+灵魂土 → 玄武岩支线不建模。
//! - **门谓词**：浸润 canHoldAnyFluid（FF:408-424）碰撞块拒、甘蔗/泡柱/
//!   传送门类明拒；穿墙 canPassThroughWall（:197-251）在流体侧退化成
//!   「对格整立方（Shapes.block()）才阻」——流体列碰撞恒空；替换门
//!   canBeReplacedWith：水仅可被 DOWN 向非水替（WF:117-119）、岩浆被水以
//!   自高 ≥4/9 替（LF:170-172）、空恒可替（EmptyFluid.java:22-24）。
//!
//! 简化/分歧登记（KNOWN-DIVERGENCE 同步维护）：
//! ① 原版 LevelTicks 允许同格同型多笔按时间归并；本仓 (格,流体) 单账
//!    去重、最早到者优先——迟到的重调度不重入，决策本体纯函数重读世界，
//!    终态等价，队列规模恒界。
//! ② 液位渲染高度（FF.getShape :483-487）归 #148 渲染波；本波流动水渲染
//!    仍满格。
//! ③ LiquidBlockContainer（kelp/海草 waterlogged，FF:408-411 首分支）不
//!    建模：按普通可侵块销毁走掉落。
//! ④ 灵魂土+蓝冰 → 玄武岩（LB:230-234）不建模。
//! ⑤ getHeight 的「上方同族 → 1.0」加成（FF:469-472）在替换门/延迟规则
//!    处取自有 amount/9（本仓 falling 柱落格常为满量 8，剪切面一致）。

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

use mcv_core::{BlockId, BlockPos};
use mcv_worldgen::trees::Rng;

/// "water"（blocks_gen.inc.rs:38）。
pub const WATER: u16 = 5;
/// "lava"（blocks_gen.inc.rs:549）。
pub const LAVA: u16 = 516;
/// "stone"（LF:209 石化，blocks_gen.inc.rs:22）。
pub const STONE: u16 = 1;
/// "cobble"（LB:224 流岩浆 → 圆石，blocks_gen.inc.rs:30）。
pub const COBBLE: u16 = 9;
/// "obsidian"（LB:224 源岩浆 → 黑曜石，blocks_gen.inc.rs:690）。
pub const OBSIDIAN: u16 = 657;

/// 流体族（26.1 Fluid.isSame 同族判据在本仓的两实例）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FluidKind {
    Water,
    Lava,
}

impl FluidKind {
    pub fn raw(self) -> u16 {
        match self {
            FluidKind::Water => WATER,
            FluidKind::Lava => LAVA,
        }
    }
}

/// tick 延迟（WF:112-114 / LF:175-177；本仓无维度环境属性，主世界恒面）。
pub const WATER_TICK_DELAY: u32 = 5;
pub const LAVA_TICK_DELAY: u32 = 30;
/// 侧流衰减 dropOff（WF:107-109 / LF:165-167）。
pub const WATER_DROP_OFF: i32 = 1;
pub const LAVA_DROP_OFF: i32 = 2;
/// 坡寻深度 getSlopeFindDistance（WF:92-94 / LF:150-152）。
pub const WATER_SLOPE_FIND_DISTANCE: i32 = 4;
pub const LAVA_SLOPE_FIND_DISTANCE: i32 = 2;
/// 源再生 gamerule 默认（GR:92 / :47）。
pub const WATER_SOURCE_CONVERSION: bool = true;
pub const LAVA_SOURCE_CONVERSION: bool = false;

/// 岩浆被水替换的高度门（LF:171 getHeight≥0.44444445 = amount/9 → ≥4）。
const LAVA_WATER_CUTOFF: u8 = 4;

/// 每固定 tick 的流体 tick 处理上限（防御预算；队列按 (格,流体) 去重，
/// 常规编辑扰动远达不到；超限余量留堆下拍续，不丢活账）。
pub const FLUID_TICKS_STEP_BUDGET: usize = 16384;

pub fn fluid_of(raw: u16) -> Option<FluidKind> {
    match raw {
        WATER => Some(FluidKind::Water),
        LAVA => Some(FluidKind::Lava),
        _ => None,
    }
}

pub fn tick_delay(kind: FluidKind) -> u32 {
    match kind {
        FluidKind::Water => WATER_TICK_DELAY,
        FluidKind::Lava => LAVA_TICK_DELAY,
    }
}

pub fn drop_off(kind: FluidKind) -> i32 {
    match kind {
        FluidKind::Water => WATER_DROP_OFF,
        FluidKind::Lava => LAVA_DROP_OFF,
    }
}

pub fn slope_find_distance(kind: FluidKind) -> i32 {
    match kind {
        FluidKind::Water => WATER_SLOPE_FIND_DISTANCE,
        FluidKind::Lava => LAVA_SLOPE_FIND_DISTANCE,
    }
}

pub fn can_convert_to_source(kind: FluidKind) -> bool {
    match kind {
        FluidKind::Water => WATER_SOURCE_CONVERSION,
        FluidKind::Lava => LAVA_SOURCE_CONVERSION,
    }
}

/// 一格流体态（26.1 FluidState 裁剪面：amount 1..8 / FALLING / isSource）。
/// legacy 编码（LB:69-77 状态缓存 + :124-127 `min(level,8)` 钳）：0=源；
/// 1..7=流动量 8−L；8..15=falling 满柱（钳后与 8 同）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FluidCell {
    pub amount: u8,
    pub falling: bool,
    pub source: bool,
}

/// 流体态读出（LB:124-127 + WF:139-159/LF:239-255 的 amount/isSource
/// 双实例合流）。
pub fn decode(b: BlockId) -> Option<FluidCell> {
    fluid_of(b.id())?;
    Some(match b.state() {
        0 => FluidCell {
            amount: 8,
            falling: false,
            source: true,
        },
        8..=15 => FluidCell {
            amount: 8,
            falling: true,
            source: false,
        },
        level => FluidCell {
            amount: 8 - level,
            falling: false,
            source: false,
        },
    })
}

/// 落格方块态（WF:97-99 createLegacyBlock → FF:461-463 getLegacyLevel）。
pub fn encode(kind: FluidKind, cell: FluidCell) -> BlockId {
    let level = if cell.source {
        0u8
    } else if cell.falling {
        8u8
    } else {
        8 - cell.amount.min(8)
    };
    BlockId(kind.raw()).with_state(level)
}

/// 方向（+X=东 / −X=西 / +Y=上 / −Y=下 / +Z=南 / −Z=北，26.1 同名映射）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    North,
    South,
    West,
    East,
    Up,
    Down,
}

impl Dir {
    pub fn opposite(self) -> Self {
        match self {
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
        }
    }

    fn delta(self) -> (i32, i32, i32) {
        match self {
            Dir::North => (0, 0, -1),
            Dir::South => (0, 0, 1),
            Dir::West => (-1, 0, 0),
            Dir::East => (1, 0, 0),
            Dir::Up => (0, 1, 0),
            Dir::Down => (0, -1, 0),
        }
    }
}

/// `BlockPos` 邻格位移扩展 trait：`BlockPos` 定义在 mcv_core（crate 外
/// 类型，E0116 不能补 inherent impl）——以 trait 供本模块与宿主 game.rs
/// 的六邻/坡寻位移共用（Java 原文 `pos.relative(dir)`，FF:169 等）。
pub trait BlockPosExt {
    fn step(self, d: Dir) -> BlockPos;
}

impl BlockPosExt for BlockPos {
    #[inline]
    fn step(self, d: Dir) -> BlockPos {
        let (dx, dy, dz) = d.delta();
        BlockPos::new(self.x + dx, self.y + dy, self.z + dz)
    }
}

/// `Direction.Plane.HORIZONTAL`（values 序过滤 = 北南西东；聚合仅 max/
/// count/距离，序只锁 spread 输出的确定序）。
const HORIZONTAL: [Dir; 4] = [Dir::North, Dir::South, Dir::West, Dir::East];

/// 六邻全向（updateShape/neighborChanged 广播域）。
pub const ALL_DIRS: [Dir; 6] = [
    Dir::North,
    Dir::South,
    Dir::West,
    Dir::East,
    Dir::Up,
    Dir::Down,
];

// ---------------------------------------------------------------------------
// 世界读视图 + 门谓词（FF 私有静态门的本仓退化复刻，依据见模块头注）
// ---------------------------------------------------------------------------

/// 一次决策的只读世界（宿主用 `WorldView::block` 装配；未装载 = 空气）。
pub type Read<'a> = dyn Fn(BlockPos) -> BlockId + 'a;

/// 穿墙门退化面（FF:207-214 `targetShape == Shapes.block()` 侧；流体列
/// 碰撞恒空 → 只判对格）：整立方碰撞（solid ∩ Cube）才阻。
fn full_solid(b: BlockId) -> bool {
    let def = b.def();
    def.solid && def.shape == mcv_core::shape::Shape::Cube as u8
}

/// `blocksMotion`（碰撞非空；千块表 solid 列）。
fn blocks_motion(b: BlockId) -> bool {
    b.def().solid
}

const fn name_eq(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    if x.len() != y.len() {
        return false;
    }
    let mut same = true;
    let mut i = 0;
    while i < x.len() {
        if x[i] != y[i] {
            same = false;
            break;
        }
        i += 1;
    }
    same
}

/// 浸润门（canHoldAnyFluid FF:408-424）：碰撞块拒；门/梯在碰撞位已拒；
/// 明拒名单按注册名（FF:415-423）。
fn can_hold_any(b: BlockId) -> bool {
    if b.0 == 0 {
        return true;
    }
    let def = b.def();
    if def.solid {
        return false;
    }
    let n = def.name;
    !(name_eq(n, "sugar_cane")
        || name_eq(n, "bubble_column")
        || name_eq(n, "nether_portal")
        || name_eq(n, "end_portal")
        || name_eq(n, "end_gateway")
        || name_eq(n, "structure_void"))
}

/// 同族源判（`!isSourceBlockOfThisType` 的反，FF:347-349）。
fn is_source_of(kind: FluidKind, b: BlockId) -> bool {
    b.id() == kind.raw() && b.state() == 0
}

/// 同族判（FF.isSame：WF:102-104 / LF:160-162）。
fn same_fluid(kind: FluidKind, b: BlockId) -> bool {
    b.id() == kind.raw()
}

/// 既有流体可被新液替换。既有 = 非流体（空/普通块无流体态）→ 恒可替
/// （EmptyFluid.java:22-24）；`new_cell = None` = 新液 EMPTY（写空气），
/// 流体型按 EMPTY 参与门判（WF:118 `!other.is(WATER)` 由此放行排水）。
fn can_be_replaced_with(
    existing: BlockId,
    dir: Dir,
    kind: FluidKind,
    new_cell: Option<FluidCell>,
) -> bool {
    let Some(ek) = fluid_of(existing.id()) else {
        return true;
    };
    match ek {
        FluidKind::Water => {
            // WF:117-119：仅 DOWN，且新液型非水（新液 Some+同族 → 水型）。
            dir == Dir::Down && !(kind == FluidKind::Water && new_cell.is_some())
        }
        FluidKind::Lava => {
            // LF:170-172：自高 ≥4/9 且新液型为水（EMPTY 型非水不成立）。
            decode(existing).is_some_and(|c| c.amount >= LAVA_WATER_CUTOFF)
                && kind == FluidKind::Water
                && new_cell.is_some()
        }
    }
}

// ---------------------------------------------------------------------------
// 决策核（FlowingFluid.java 移植；一切都是纯读世界 + 输出计划）
// ---------------------------------------------------------------------------

/// 一格流体 tick 的产出计划（宿主 [`super::game`] 按序执行：自写 → 掉落 →
/// spread 落格 → 石化，每笔写后走 setBlock(flags=3) 的触发面）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelfWrite {
    #[default]
    Stay,
    /// FF:446-449：newLiquid 空 → setBlock(AIR)（脱水）。
    Air,
    /// FF:450-455：newLiquid 变 → setBlock(new) + scheduleTick(spreadDelay)。
    Change(BlockId),
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub self_write: SelfWrite,
    /// 自写后是否按 spreadDelay 重排自身（FF:454；lava ×4 规则在此生效）。
    pub self_reschedule: bool,
    /// spreadTo 落格（FF:275 createLegacyBlock；含写空气的排水送达）。
    pub spread: Vec<(BlockPos, BlockId)>,
    /// beforeDestroyingBlock（FF:272 :279 + LB dropResources）——被漫毁的
    /// 非空块（key=位置、value=被毁 raw），宿主出掉落。
    pub destroyed: Vec<(BlockPos, u16)>,
    /// LF:204-218：岩浆向下漫进含水体格 → 置石头（+嘶声略）。
    pub lava_stone: Vec<BlockPos>,
}

/// FF:442-459 tick 本体。`cur` = 调用方已按队列验配的同族流体 raw。
pub fn tick(kind: FluidKind, at: &Read<'_>, pos: BlockPos, cur: BlockId) -> Plan {
    let mut plan = Plan::default();
    let self_cell = decode(cur);
    let mut own = self_cell;
    if let Some(f) = self_cell
        && !f.source
    {
        let new = get_new_liquid(kind, at, pos);
        if new.is_none() {
            plan.self_write = SelfWrite::Air;
            own = None;
        } else if let Some(c) = new {
            plan.self_write = SelfWrite::Change(encode(kind, c));
            plan.self_reschedule = true; // FF:454
            own = Some(c);
        }
    }
    spread(kind, at, pos, own, &mut plan);
    plan
}

/// 自写后的重排延迟（FF:437-439 默认 = tickDelay；LF:180-192 双非空、双
/// 非 falling、新高于旧 → ×4，1/4 概率免——`rng` 抽取消费面同 drop 流）。
pub fn spread_delay(
    kind: FluidKind,
    old: Option<FluidCell>,
    new: Option<FluidCell>,
    rng: &mut impl Rng,
) -> u32 {
    let base = tick_delay(kind);
    if kind != FluidKind::Lava {
        return base;
    }
    if let (Some(o), Some(n)) = (old, new)
        && !o.falling
        && !n.falling
        && n.amount > o.amount
        && rng.next_int(4) != 0
    {
        return base * 4;
    }
    base
}

/// FF:160-195 getNewLiquid：横向同族最高位 − dropOff；2+ 邻源 + 再生门 +
/// 底部碰撞/源 → 源（:178-184）；上方同族 → falling 满柱（:186-191）。
pub fn get_new_liquid(kind: FluidKind, at: &Read<'_>, pos: BlockPos) -> Option<FluidCell> {
    let mut highest: i32 = 0;
    let mut sources = 0;
    for d in HORIZONTAL {
        let n = pos.step(d);
        let nb = at(n);
        if !same_fluid(kind, nb) || full_solid(nb) {
            continue; // FF:169 isSame && canPassThroughWall
        }
        let Some(nf) = decode(nb) else { continue };
        if nf.source {
            sources += 1;
        }
        highest = highest.max(nf.amount as i32);
    }
    if sources >= 2 && can_convert_to_source(kind) {
        // FF:179-184：底部 solid（blocksMotion 面）或同族源。
        let below = at(pos.step(Dir::Down));
        if blocks_motion(below) || is_source_of(kind, below) {
            return Some(FluidCell {
                amount: 8,
                falling: false,
                source: true,
            });
        }
    }
    let above = at(pos.step(Dir::Up));
    if same_fluid(kind, above) && !full_solid(above) {
        // FF:189-191：上方同族 → getFlowing(8, true)。
        return Some(FluidCell {
            amount: 8,
            falling: true,
            source: false,
        });
    }
    let amount = highest - drop_off(kind);
    (amount > 0).then_some(FluidCell {
        amount: amount as u8,
        falling: false,
        source: false,
    })
}

/// FF:117-140 spread（`own` = 自写后的本体流体态；None = 已脱水不再漫）。
fn spread(kind: FluidKind, at: &Read<'_>, pos: BlockPos, own: Option<FluidCell>, plan: &mut Plan) {
    let Some(f) = own else { return }; // FF:118 !fluidState.isEmpty()
    let below_pos = pos.step(Dir::Down);
    let below_block = at(below_pos);
    // FF:122 canMaybePassThrough(DOWN)：非同族源 && 可侵 && 不被整立方阻。
    let down_ok =
        !is_source_of(kind, below_block) && can_hold_any(below_block) && !full_solid(below_block);
    if down_ok {
        let new_below = get_new_liquid(kind, at, below_pos);
        // FF:125 既有流体替换门 + FF:126 容器门（本仓恒过）。
        if can_be_replaced_with(below_block, Dir::Down, kind, new_below) {
            spread_to(kind, below_pos, below_block, Dir::Down, new_below, plan);
            // FF:128-130：3+ 源邻才继续侧流（下行通道被围水主导）。
            if source_neighbor_count(kind, at, pos) >= 3 {
                spread_to_sides(kind, at, pos, f, plan);
            }
            return;
        }
    }
    // FF:136-138：本体是源 / 下方不是「可渗洞」→ 侧流。
    if f.source || !is_water_hole(kind, below_block) {
        spread_to_sides(kind, at, pos, f, plan);
    }
}

/// FF:309-317 isWaterHole：下方不被整立方阻 且（同族 或 可侵空白）——判
/// 据落在下方格自身，below_pos 的延迟读不参与（FF 里同为即时值）。
fn is_water_hole(kind: FluidKind, below_block: BlockId) -> bool {
    if full_solid(below_block) {
        return false;
    }
    if same_fluid(kind, below_block) {
        return true;
    }
    can_hold_any(below_block)
}

/// FF:353-365 sourceNeighborCount：水平四向同族源计数。
fn source_neighbor_count(kind: FluidKind, at: &Read<'_>, pos: BlockPos) -> i32 {
    let mut count = 0;
    for d in HORIZONTAL {
        if is_source_of(kind, at(pos.step(d))) {
            count += 1;
        }
    }
    count
}

/// FF:142-158 spreadToSides：侧液位 = 本量 − dropOff（falling 时恒 7）；
/// 方向集由 [`get_spread`]（坡寻最低优先）定，逐向落格。
fn spread_to_sides(kind: FluidKind, at: &Read<'_>, pos: BlockPos, f: FluidCell, plan: &mut Plan) {
    let mut neighbor = f.amount as i32 - drop_off(kind);
    if f.falling {
        neighbor = 7; // FF:144-146
    }
    if neighbor <= 0 {
        return; // FF:148
    }
    for (d, nf) in get_spread(kind, at, pos) {
        let npos = pos.step(d);
        let nblock = at(npos);
        spread_to(kind, npos, nblock, d, nf, plan);
    }
}

/// FF:267-277 spreadTo（+ LF:204-218 復写：向下漫入含水体 → 石头）。
fn spread_to(
    kind: FluidKind,
    target: BlockPos,
    target_block: BlockId,
    dir: Dir,
    new_cell: Option<FluidCell>,
    plan: &mut Plan,
) {
    if kind == FluidKind::Lava
        && dir == Dir::Down
        && target_block.id() == WATER
        && decode(target_block).is_some()
    {
        // LF:207-214：LiquidBlock 含水 → 置石头（嘶声不做）。
        plan.lava_stone.push(target);
        return;
    }
    // FF:271-273 beforeDestroyingBlock：非空才毁（掉落由宿主结算）。
    if target_block.0 != 0 {
        plan.destroyed.push((target, target_block.0));
    }
    let raw = match new_cell {
        Some(c) => encode(kind, c).0,
        None => 0, // FF:448 createLegacyBlock(EMPTY) = AIR
    };
    plan.spread.push((target, BlockId(raw)));
}

/// FF:367-406 getSpread：水平候选中坡距最低者胜（洞 0 距短路）。
fn get_spread(kind: FluidKind, at: &Read<'_>, pos: BlockPos) -> Vec<(Dir, Option<FluidCell>)> {
    let mut lowest = 1000i32;
    let mut result: Vec<(Dir, Option<FluidCell>)> = Vec::new();
    for d in HORIZONTAL {
        let test = pos.step(d);
        let tblock = at(test);
        // FF:376 canMaybePassThrough：非同族源 && 可侵 && 不被整立方阻。
        if is_source_of(kind, tblock) || !can_hold_any(tblock) || full_solid(tblock) {
            continue;
        }
        let new_fluid = get_new_liquid(kind, at, test);
        // FF:383-388：洞 → 距 0；否则坡寻（from = 来向的反向）。
        let distance = if is_hole(kind, at, test) {
            0
        } else {
            slope_distance(kind, at, test, 1, d.opposite())
        };
        if distance < lowest {
            result.clear(); // FF:390-391
        }
        if distance <= lowest {
            // FF:394-400：替换门过才入结果（EMPTY 新液也入——漫毁面），
            // lowest 不论门过与否都收敛到当前坡距。
            if can_be_replaced_with(tblock, d, kind, new_fluid) {
                result.push((d, new_fluid));
            }
            lowest = distance;
        }
    }
    result
}

/// FF:522-528 SpreadContext.isHole。
fn is_hole(kind: FluidKind, at: &Read<'_>, pos: BlockPos) -> bool {
    let below_pos = pos.step(Dir::Down);
    is_water_hole(kind, at(below_pos))
}

/// FF:281-307 getSlopeDistance：DFS 坡寻（深度 ≤ slopeFindDistance，洞短路
/// 返回当前 pass）。
fn slope_distance(kind: FluidKind, at: &Read<'_>, pos: BlockPos, pass: i32, from: Dir) -> i32 {
    let mut lowest = 1000i32;
    for d in HORIZONTAL {
        if d == from {
            continue; // FF:287
        }
        let n = pos.step(d);
        let nblock = at(n);
        // FF:291 canPassThrough = canMaybePassThrough && 容器门（恒过）。
        if is_source_of(kind, nblock) || !can_hold_any(nblock) || full_solid(nblock) {
            continue;
        }
        if is_hole(kind, at, n) {
            return pass; // FF:292-294 短路
        }
        if pass < slope_find_distance(kind) {
            let v = slope_distance(kind, at, n, pass + 1, d.opposite());
            if v < lowest {
                lowest = v;
            }
        }
    }
    lowest
}

// ---------------------------------------------------------------------------
// scheduled tick 队列（简化面 ① 见模块头注）
// ---------------------------------------------------------------------------

/// 流体 scheduled tick 队列：`(到时, seq)` 最小堆 + (格, 流体) 在途单账。
/// 与原版 LevelTicks 的时间序等价（_due 最早先出；同拍按 seq FIFO）。
/// 最小堆条目：((_due, seq), (x, y, z), 流体编码)——时间序先出、
/// 同拍 seq FIFO（别名化以满足 clippy type_complexity）。
type QueueEntry = Reverse<((u64, u64), (i32, i32, i32), u16)>;

#[derive(Default)]
pub struct FluidQueue {
    heap: BinaryHeap<QueueEntry>,
    pending: HashSet<(i32, i32, i32, u16)>,
    seq: u64,
}

impl FluidQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// LB:152/:181/:200 scheduleTick 面：同 (格,流体) 在途即吞（最早到者
    /// 先出，决策重读世界终态等价，简化面 ①）。
    pub fn schedule(&mut self, due: u64, pos: BlockPos, fluid: u16) {
        let key = (pos.x, pos.y, pos.z, fluid);
        if !self.pending.insert(key) {
            return;
        }
        self.heap
            .push(Reverse(((due, self.seq), (pos.x, pos.y, pos.z), fluid)));
        self.seq += 1;
    }

    /// 到时出队（`now >= due`）；出账即销在途单账——后续重调度重新入队。
    pub fn pop_due(&mut self, now: u64) -> Option<(BlockPos, u16)> {
        let Reverse(((due, _), xyz, fluid)) = self.heap.peek().copied()?;
        if due > now {
            return None;
        }
        self.heap.pop();
        self.pending.remove(&(xyz.0, xyz.1, xyz.2, fluid));
        Some((BlockPos::new(xyz.0, xyz.1, xyz.2), fluid))
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// 在途笔数（观测/测试面）。
    pub fn len(&self) -> usize {
        self.heap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 只读小世界（未列格 = 空气，WorldView::block 语义）。键用
    /// (x,y,z) 三元组——mcv_core::BlockPos 未 derive Hash（core 不为
    /// logic 的测试面拉派生），tuple 是同语义零依赖键。
    /// 拥有权闭包（map move 变换，无输入借用）。`use<>` 空捕获清单：
    /// Rust 2024 RPIT 隐式捕获全部输入寿命，`&[(..)]` 的寿命会被写进
    /// opaque 导致 pairs 临时数组被判悬垂（E0716×4，runs 38990640360/
    /// 38990879544）；空清单=opaque 只依赖返回值自身所有权。
    fn world(pairs: &[(BlockPos, BlockId)]) -> impl Fn(BlockPos) -> BlockId + use<> {
        let map: HashMap<(i32, i32, i32), BlockId> =
            pairs.iter().map(|(p, b)| ((p.x, p.y, p.z), *b)).collect();
        move |p| map.get(&(p.x, p.y, p.z)).copied().unwrap_or(BlockId(0))
    }

    /// legacy LEVEL 编码（LB:69-77 + :124-127 钳 8）：0=源、8=falling、
    /// 1..7=流动量 8−L；encode→decode 回路保距，非流体无流体态。
    #[test]
    fn encode_decode_legacy_level_roundtrip() {
        let cases = [
            (
                FluidCell {
                    amount: 8,
                    falling: false,
                    source: true,
                },
                0u8,
            ),
            (
                FluidCell {
                    amount: 8,
                    falling: true,
                    source: false,
                },
                8,
            ),
            (
                FluidCell {
                    amount: 7,
                    falling: false,
                    source: false,
                },
                1,
            ),
            (
                FluidCell {
                    amount: 1,
                    falling: false,
                    source: false,
                },
                7,
            ),
        ];
        for (cell, level) in cases {
            let b = encode(FluidKind::Water, cell);
            assert_eq!(b.state(), level);
            assert_eq!(decode(b), Some(cell));
        }
        assert!(decode(BlockId(0)).is_none());
    }

    /// getNewLiquid 三门（FF:160-195）：双源+实底 → 源再生（water=GR:92
    /// true）；lava 同条件不源化（GR:47 false）只回落差；四周空 → 脱水
    /// None；上方同族 → falling 满柱（:186-191）。
    #[test]
    fn get_new_liquid_regen_dry_falling() {
        let at = BlockPos::new(0, 64, 0);
        let read = world(&[
            (BlockPos::new(1, 64, 0), BlockId(WATER)),
            (BlockPos::new(-1, 64, 0), BlockId(WATER)),
            (BlockPos::new(0, 63, 0), BlockId(STONE)),
        ]);
        assert!(get_new_liquid(FluidKind::Water, &read, at).unwrap().source);

        let read = world(&[
            (BlockPos::new(1, 64, 0), BlockId(LAVA)),
            (BlockPos::new(0, 63, 0), BlockId(STONE)),
        ]);
        let cell = get_new_liquid(FluidKind::Lava, &read, at).unwrap();
        assert!(!cell.source);
        assert_eq!((cell.amount, cell.falling), (6, false)); // 8 − dropOff 2

        assert_eq!(get_new_liquid(FluidKind::Water, &world(&[]), at), None);

        let read = world(&[(BlockPos::new(0, 65, 0), BlockId(WATER).with_state(3))]);
        let cell = get_new_liquid(FluidKind::Water, &read, at).unwrap();
        assert_eq!((cell.amount, cell.falling), (8, true));
    }

    /// tick 脱水面（FF:446-449）：流动格四周空 → SelfWrite::Air、无
    /// spread；源横流面（FF:136-138 → :142-158）：实底挡下 → 四向侧漫
    /// 各落 amount−dropOff（态 1=8−7…即 LEVEL 1）。
    #[test]
    fn tick_plan_dry_and_side_spread() {
        let at = BlockPos::new(0, 64, 0);
        let plan = tick(
            FluidKind::Water,
            &world(&[]),
            at,
            BlockId(WATER).with_state(2),
        );
        assert_eq!(plan.self_write, SelfWrite::Air);
        assert!(plan.spread.is_empty() && !plan.self_reschedule);

        // 世界读 = 真实关卡面：含 tick 本格（宿主 view.block 覆盖全格；
        // 侧邻格 get_new_liquid 经 at() 读回本格源才有 −dropOff 液位——
        // 漏本格源时邻格算空、spread 落 AIR，曾红：id=0 decode=None）。
        let read = world(&[
            (at, BlockId(WATER)),
            (BlockPos::new(0, 63, 0), BlockId(STONE)),
        ]);
        let plan = tick(FluidKind::Water, &read, at, BlockId(WATER));
        assert_eq!(plan.self_write, SelfWrite::Stay);
        assert_eq!(plan.spread.len(), 4);
        assert!(plan.spread.iter().all(|(p, b)| {
            p.y == at.y && p != &at && b.state() == 1 && decode(*b).unwrap().amount == 7
        }));
    }

    /// 队列面（简化面 ①）：同 (格,流体) 在途单账去重；最早到者优先；
    /// 未到时不许出；出账后重调度不再被吞。
    #[test]
    fn fluid_queue_dedup_and_order() {
        let mut q = FluidQueue::new();
        let a = BlockPos::new(1, 2, 3);
        q.schedule(10, a, WATER);
        q.schedule(10, a, WATER);
        assert_eq!(q.len(), 1);
        q.schedule(5, BlockPos::new(0, 0, 0), WATER);
        assert_eq!(q.pop_due(9).unwrap().0, BlockPos::new(0, 0, 0));
        assert!(q.pop_due(9).is_none()); // due=10 未到
        assert_eq!(q.pop_due(10).unwrap().0, a);
        q.schedule(12, a, WATER);
        assert_eq!(q.len(), 1);
        assert!(q.pop_due(11).is_none());
        assert_eq!(q.pop_due(12).unwrap().0, a);
        assert!(q.is_empty());
    }
}
