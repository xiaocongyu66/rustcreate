//! 票表：原版 TicketStorage + DistanceManager 级别派生的本仓裁剪镜像（刀 3a）。
//!
//! 原版机制（src-26.1，file:line 均已核验）：
//! - 三级票级别：FULL=33 / BLOCK_TICKING=32 / ENTITY_TICKING=31
//!   （ChunkLevel.java:10-12）；`MAX_LEVEL = 33 + RADIUS_AROUND_FULL_CHUNK`
//!   （:14-15）。RADIUS 原版 = 生成金字塔 `getStepTo(FULL)` 的
//!   `accumulatedDependencies().getRadius()`——26.1 实测 = 1（FULL 步仅继承
//!   LIGHT 对 INITIALIZE_LIGHT 的半径 1 需求，ChunkPyramid.java:36-39）。
//!   本仓无 generation pyramid 结构，RADIUS 写死 1——**显式裁剪债**：取值
//!   与 26.1 相同，但来源是常量而非依赖半径派生。
//! - 玩家加载票（PLAYER_LOADING）：级别 [`ENTITY_TICKING_LEVEL`]，对距玩家
//!   Chebyshev ≤ viewDistance 的**每个**区块各发一张（DistanceManager.java
//!   :276,:278-285；haveTicketFor = level ≤ viewDistance，:319-321）。
//! - 玩家模拟票（PLAYER_SIMULATION）：**单锚点**（玩家所在区块一张），级别
//!   `max(0, 31 − simulationDistance)`（DistanceManager.java:48 默认 10、
//!   :110-117 addPlayer、:132-134 getPlayerTicketLevel；sim 距离变化时
//!   整型替换 :152-157）。
//! - 级别传播：ChunkTracker 对 8 邻域 +1/环扩散（ChunkTracker.java:23-27、
//!   :66 `fromLevel + 1`，即 Chebyshev 度量），区块级别 = 对全部票取
//!   `min(ticket_level + Chebyshev 距离)` 的传播不动点（min-ticket 存储）。
//! - 卸载迟滞 = 票级别语义：级别 > MAX_LEVEL → toDrop 回收
//!   （ChunkMap.java:376-381 updateChunkScheduling 的 `!ChunkLevel.isLoaded`）。
//! - 允许推进的最高 status 由级别派生：ChunkLevel.generationStatus(:17-19)
//!   = getStatusAroundFullChunk(level−33)；distance ≤ 0 → FULL，1..RADIUS →
//!   accumulated dependency 对应中间态（26.1 唯一中间档 distance=1 → LIGHT）。
//!   ChunkMap 经 runAllUpdates → updateHighestAllowedStatus 消费
//!   （DistanceManager.java:75-84）。
//!
//! 本仓裁剪（逐条显式登记，禁止沉默近似）：
//! - **传播队列 → 解析式**：票集合恒小（≤ (2·view_dist+1)² + 1 张），对在册
//!   区块解析求 min 即传播不动点；原版 queueLevels/runUpdates 的增量性不复制。
//! - **每帧全量重建玩家票**：原版 addPlayer/removePlayer + updateViewDistance
//!   走增量 tracker；本仓锚点随 center 迁移每帧重发，不动点等价。
//! - **票种只接两类**：PLAYER_LOADING / PLAYER_SIMULATION（本仓无 force/
//!   chunk-debug/post-teleport 等票种）。
//! - **每类每块单票**：原版同 pos 同 type 可持票集（SortedArraySet），本仓
//!   add 同类覆盖（玩家票语义下等价——同类票级别恒同）。
//! - **模拟距离不独立设置**：sim_dist = render_dist + 1（含 +1 支撑环，语义
//!   见 `game.rs` sim_dist 节注释），PLAYER_SIMULATION 级别 = 31 − (rd+1)，
//!   模拟环（sim 级别 ≤ 31）= Chebyshev ≤ rd+1 —— 与刀 2 前的请求环一致。

use std::collections::HashMap;

use mcv_core::{ChunkPos, ChunkStatus};

/// FULL 票级别（ChunkLevel.java:10）。
pub const FULL_LEVEL: i32 = 33;
/// BLOCK_TICKING 票级别（ChunkLevel.java:11）。
pub const BLOCK_TICKING_LEVEL: i32 = 32;
/// ENTITY_TICKING 票级别；玩家加载/模拟票的基准档（ChunkLevel.java:12）。
pub const ENTITY_TICKING_LEVEL: i32 = 31;
/// FULL 周围的生成晕圈半径（ChunkLevel.java:14）。**裁剪债**：原版 =
/// getStepTo(FULL).accumulatedDependencies().getRadius()（26.1 = 1）；本仓
/// 无 generation pyramid，常量写死。
pub const RADIUS_AROUND_FULL_CHUNK: i32 = 1;
/// 加载上限级别（ChunkLevel.java:15）：级别 ≤ MAX_LEVEL 视为在载。
pub const MAX_LEVEL: i32 = FULL_LEVEL + RADIUS_AROUND_FULL_CHUNK;
/// 无票缺省级别（> MAX_LEVEL = 不可达；对应原版 tracker 的 maxDistance+2 缺省）。
pub const NOT_LOADED: u8 = (MAX_LEVEL + 1) as u8;

/// 玩家票任务节流上限：在途生成集合达到该值前不再 pop 新玩家票任务
/// （ThrottlingChunkTaskDispatcher.java:13-14,:29-31；构造参数 4 见
/// DistanceManager.java:55）。仅玩家票任务受限——worldgen/light 走普通
/// dispatcher 无节流（ChunkMap.java:195-196）。
pub const MAX_CHUNKS_IN_EXECUTION: usize = 4;

/// 票种（本仓接入的两类玩家票；原版 TicketType 子集）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TicketType {
    /// PLAYER_LOADING：方环发放，级别 31，半径 = view_dist（=sim_dist）。
    PlayerLoading,
    /// PLAYER_SIMULATION：单锚点，级别 max(0, 31 − sim_dist)。
    PlayerSimulation,
}

/// 票存储：pos → [(票种, 级别)]。多票取 min = 原版 min-ticket 语义。
#[derive(Default)]
pub struct TicketTable {
    tickets: HashMap<ChunkPos, Vec<(TicketType, u8)>>,
}

impl TicketTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加/覆盖一张票（同 pos 同票种覆盖——裁剪见模块注释）。
    pub fn add_ticket(&mut self, pos: ChunkPos, ty: TicketType, level: u8) {
        let slot = self.tickets.entry(pos).or_default();
        match slot.iter_mut().find(|(t, _)| *t == ty) {
            Some(entry) => entry.1 = level,
            None => slot.push((ty, level)),
        }
    }

    pub fn remove_ticket(&mut self, pos: ChunkPos, ty: TicketType) {
        if let Some(slot) = self.tickets.get_mut(&pos) {
            slot.retain(|(t, _)| *t != ty);
            if slot.is_empty() {
                self.tickets.remove(&pos);
            }
        }
    }

    /// 全表清空（本仓玩家票每帧从锚点重建；接入非玩家票种后应改为按种清除）。
    pub fn clear(&mut self) {
        self.tickets.clear();
    }

    /// 某票种全部票的级别整型替换（原版 replaceTicketLevelOfType，
    /// DistanceManager.java:155——updateSimulationDistance 的替换路径）。
    pub fn replace_level_of_type(&mut self, ty: TicketType, level: u8) {
        for slot in self.tickets.values_mut() {
            for (t, lv) in slot.iter_mut() {
                if *t == ty {
                    *lv = level;
                }
            }
        }
    }

    pub fn has_tickets(&self) -> bool {
        !self.tickets.is_empty()
    }

    /// 区块级别 = min over 全部票 (票级别 + Chebyshev 距离)，无票 =
    /// [`NOT_LOADED`]。等价于原版 ChunkTracker 8 邻域 +1/环的传播不动点
    /// （ChunkTracker.java:23-27,:66）。
    pub fn level(&self, pos: ChunkPos) -> u8 {
        let mut best = NOT_LOADED as i32;
        for (tpos, slot) in &self.tickets {
            let d = cheb(pos, *tpos);
            for (_, lv) in slot {
                let l = (*lv as i32).saturating_add(d);
                if l < best {
                    best = l;
                }
            }
        }
        best.clamp(0, NOT_LOADED as i32) as u8
    }

    /// 仅统计某票种贡献的级别（原版分 tracker 视图：loading vs simulation，
    /// DistanceManager.java:144-146）。
    pub fn level_of_type(&self, ty: TicketType, pos: ChunkPos) -> u8 {
        let mut best = NOT_LOADED as i32;
        for (tpos, slot) in &self.tickets {
            let d = cheb(pos, *tpos);
            for (t, lv) in slot {
                if *t != ty {
                    continue;
                }
                let l = (*lv as i32).saturating_add(d);
                if l < best {
                    best = l;
                }
            }
        }
        best.clamp(0, NOT_LOADED as i32) as u8
    }

    /// 批量级别派生：一帧内对在册区块求一次（unload/status 两个消费点共用
    /// 形态，各算各的快照——票表在帧内不变，结果一致）。
    pub fn level_map<I>(&self, positions: I) -> HashMap<ChunkPos, u8>
    where
        I: Iterator<Item = ChunkPos>,
    {
        let mut out: HashMap<ChunkPos, u8> = positions.map(|p| (p, NOT_LOADED)).collect();
        for (tpos, slot) in &self.tickets {
            for (p, best) in out.iter_mut() {
                let d = cheb(*p, *tpos);
                for (_, lv) in slot {
                    let l = (*lv as i32).saturating_add(d);
                    if l < *best as i32 {
                        *best = l.clamp(0, NOT_LOADED as i32) as u8;
                    }
                }
            }
        }
        out
    }
}

/// Chebyshev 距离（原版 ChunkTracker 的 8 邻域传播度量）。i64 中转防
/// i32 端界相减溢出。
fn cheb(a: ChunkPos, b: ChunkPos) -> i32 {
    let dx = (a.x as i64 - b.x as i64).abs();
    let dz = (a.z as i64 - b.z as i64).abs();
    dx.max(dz).min(i32::MAX as i64) as i32
}

/// 级别 → 是否在载（ChunkLevel.java:65-67 isLoaded）。级别 > MAX_LEVEL =
/// 卸载候选（ChunkMap.java:376-381）。
pub fn is_loaded(level: i32) -> bool {
    level <= MAX_LEVEL
}

/// 级别 → 是否实体 tick 环（ChunkLevel.java:57-59）。模拟侧谓词请对
/// PLAYER_SIMULATION 视图的级别调用（DistanceManager.java:136-138）。
pub fn is_entity_ticking(level: i32) -> bool {
    level <= ENTITY_TICKING_LEVEL
}

/// 级别 → 是否方块 tick 环（ChunkLevel.java:61-63）。
pub fn is_block_ticking(level: i32) -> bool {
    level <= BLOCK_TICKING_LEVEL
}

/// 级别 → 允许推进到的最高 worldgen status（ChunkLevel.java:17-19
/// generationStatus 镜像）：level ≤ 33 → `Full`；level = 34（晕圈环）→
/// `Light`（FULL 步 accumulated dependency @1 = LIGHT，RADIUS=1 唯一中间档）；
/// level > MAX_LEVEL → None（级别不授权任何推进 = 卸载候选）。
/// 本仓 worldgen 一次产出（NOISE+SURFACE 合并档，见 ChunkStatus 裁剪映射），
/// 实际推进仍以各 pass 完成的 stage 证据为门——见 game.rs
/// `advance_status_by_level`。
pub fn generation_status(level: i32) -> Option<ChunkStatus> {
    let distance = level - FULL_LEVEL;
    if distance > RADIUS_AROUND_FULL_CHUNK {
        return None;
    }
    if distance <= 0 {
        return Some(ChunkStatus::Full);
    }
    Some(ChunkStatus::Light)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_tickets(table: &mut TicketTable, center: ChunkPos, view_dist: i32, level: u8) {
        for dx in -view_dist..=view_dist {
            for dz in -view_dist..=view_dist {
                table.add_ticket(
                    ChunkPos::new(center.x + dx, center.z + dz),
                    TicketType::PlayerLoading,
                    level,
                );
            }
        }
    }

    /// min-ticket 语义：同块多票取最低级别；撤票后级别回升；无票 = NOT_LOADED。
    #[test]
    fn min_ticket_semantics() {
        let mut t = TicketTable::new();
        let c = ChunkPos::new(3, -2);
        assert_eq!(t.level(c), NOT_LOADED, "无票 = NOT_LOADED");
        t.add_ticket(c, TicketType::PlayerLoading, ENTITY_TICKING_LEVEL as u8);
        assert_eq!(t.level(c), ENTITY_TICKING_LEVEL as u8);
        t.add_ticket(c, TicketType::PlayerSimulation, 26);
        assert_eq!(t.level(c), 26, "多票取 min");
        assert!(t.has_tickets());
        t.remove_ticket(c, TicketType::PlayerSimulation);
        assert_eq!(t.level(c), ENTITY_TICKING_LEVEL as u8, "撤票后回升");
        t.remove_ticket(c, TicketType::PlayerLoading);
        assert_eq!(t.level(c), NOT_LOADED);
        assert!(!t.has_tickets());
    }

    /// 加载票方环的级别场与卸载边界：级别 = 31 + max(0, d − view_dist)，
    /// `is_loaded` 边界恰在 Chebyshev = view_dist + RADIUS + 2。取
    /// view_dist = sim_dist = rd+1 时即 rd+4 —— 与刀 2 前的自研
    /// `unload_dist` 阈值**逐块等价**（本测试就是行为等价锁）。
    #[test]
    fn loading_spread_unload_boundary_matches_legacy_distance_rule() {
        for rd in 1..=8i32 {
            let vd = rd + 1;
            let mut t = TicketTable::new();
            let center = ChunkPos::new(100, -100);
            square_tickets(&mut t, center, vd, ENTITY_TICKING_LEVEL as u8);
            let unload_r = rd + 4; // 旧自研阈值 unload_dist = rd+4
            for dx in -(vd + 7)..=(vd + 7) {
                for dz in -(vd + 7)..=(vd + 7) {
                    let p = ChunkPos::new(center.x + dx, center.z + dz);
                    let d = dx.abs().max(dz.abs());
                    let expected = ENTITY_TICKING_LEVEL + (d - vd).max(0);
                    assert_eq!(
                        t.level(p) as i32,
                        expected.min(NOT_LOADED as i32),
                        "rd={rd} d={d} 级别场不符"
                    );
                    assert_eq!(
                        is_loaded(t.level(p) as i32),
                        d <= unload_r,
                        "rd={rd} d={d}：票级别卸载边界必须与旧距离阈值逐块等价"
                    );
                }
            }
        }
    }

    /// 模拟票单锚点：级别 = max(0, 31 − sim_dist) + d，实体 tick 环
    /// （级别 ≤ 31）恰为 Chebyshev ≤ sim_dist（原版
    /// DistanceManager.java:132-138 语义）。
    #[test]
    fn simulation_anchor_entity_ticking_ring() {
        for sim_dist in [1, 5, 10, 32] {
            let mut t = TicketTable::new();
            let anchor = ChunkPos::new(-7, 9);
            let level = (ENTITY_TICKING_LEVEL - sim_dist).max(0) as u8;
            t.add_ticket(anchor, TicketType::PlayerSimulation, level);
            for d in 0..=12 {
                let p = ChunkPos::new(anchor.x + d, anchor.z);
                let lv = t.level_of_type(TicketType::PlayerSimulation, p) as i32;
                assert_eq!(
                    lv,
                    (level as i32 + d).min(NOT_LOADED as i32),
                    "sim_dist={sim_dist} d={d}"
                );
                assert_eq!(
                    is_entity_ticking(lv),
                    d <= sim_dist,
                    "sim_dist={sim_dist} d={d}：实体 tick 环边界不符"
                );
            }
        }
    }

    /// replace_level_of_type：整类替换（updateSimulationDistance 路径）。
    #[test]
    fn replace_level_of_type_replaces_whole_class() {
        let mut t = TicketTable::new();
        let a = ChunkPos::new(0, 0);
        let b = ChunkPos::new(1, 0);
        t.add_ticket(a, TicketType::PlayerSimulation, 26);
        t.add_ticket(b, TicketType::PlayerSimulation, 26);
        t.add_ticket(a, TicketType::PlayerLoading, 31);
        t.replace_level_of_type(TicketType::PlayerSimulation, 21);
        assert_eq!(t.level_of_type(TicketType::PlayerSimulation, a), 21);
        assert_eq!(t.level_of_type(TicketType::PlayerSimulation, b), 22);
        assert_eq!(
            t.level_of_type(TicketType::PlayerLoading, a),
            ENTITY_TICKING_LEVEL as u8,
            "其他票种不受替换影响"
        );
    }

    /// generation_status 镜像：level ≤ 33 → Full；34（晕圈）→ Light；
    /// > MAX_LEVEL → None（不授权推进 = 卸载候选）。
    #[test]
    fn generation_status_mirror() {
        assert_eq!(generation_status(0), Some(ChunkStatus::Full));
        assert_eq!(generation_status(31), Some(ChunkStatus::Full));
        assert_eq!(generation_status(32), Some(ChunkStatus::Full));
        assert_eq!(generation_status(FULL_LEVEL), Some(ChunkStatus::Full));
        assert_eq!(generation_status(MAX_LEVEL), Some(ChunkStatus::Light));
        assert_eq!(generation_status(MAX_LEVEL + 1), None);
        assert_eq!(generation_status(1000), None);
    }

    /// 级别边界：MAX_LEVEL 在载、NOT_LOADED 不在载；tick 环边界。
    #[test]
    fn level_boundaries() {
        assert!(is_loaded(MAX_LEVEL));
        assert!(!is_loaded(MAX_LEVEL + 1));
        assert!(is_entity_ticking(ENTITY_TICKING_LEVEL));
        assert!(!is_entity_ticking(ENTITY_TICKING_LEVEL + 1));
        assert!(is_block_ticking(BLOCK_TICKING_LEVEL));
        assert!(!is_block_ticking(BLOCK_TICKING_LEVEL + 1));
        assert_eq!(NOT_LOADED, 35);
    }

    /// 运行时接线：stream() 每帧从玩家锚点重建票表——玩家块级别 =
    /// 模拟锚点级别（< 31），票发放半径 = view_dist = sim_dist = rd+1，
    /// 加载晕圈到 rd+4（is_loaded 边界）。
    #[test]
    fn stream_maintains_player_ticket_table() {
        let dir = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (
                "ticket-table",
                std::process::id(),
                std::time::SystemTime::now(),
            )
                .hash(&mut h);
            std::env::temp_dir().join(format!("mcv-ticket-{:x}", h.finish()))
        };
        let mut rt =
            crate::game::GameRuntime::new_headless(20261011, dir, crate::game::GameMode::Survival);
        rt.stream();
        let center = ChunkPos::new(0, 0); // 投放前 pos = ZERO
        let vd = rt.sim_dist();
        let sim_level = (ENTITY_TICKING_LEVEL - vd).max(0) as i32;
        assert_eq!(
            rt.tickets.level(center) as i32,
            sim_level,
            "玩家块级别 = 模拟锚点级别（min-ticket）"
        );
        let edge = ChunkPos::new(center.x + vd, center.z);
        assert_eq!(rt.tickets.level(edge) as i32, sim_level + vd);
        let halo = ChunkPos::new(center.x + vd + 3, center.z); // = rd+4
        assert!(
            is_loaded(rt.tickets.level(halo) as i32),
            "晕圈末环（rd+4）仍在载"
        );
        let out = ChunkPos::new(center.x + vd + 4, center.z); // = rd+5
        assert!(
            !is_loaded(rt.tickets.level(out) as i32),
            "rd+5 环必须已在票级别上不可加载（卸载候选域）"
        );
    }
}
