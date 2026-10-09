//! A* 寻路（26.1 `GroundPathNavigation` + `WalkNodeEvaluator`/`PathFinder`
//! 的等价简化；审计 N-1）。此前 mob 追击是直线冲脸 + `blocked` 跳跃兜底，
//! 复杂地形（2 格高墙、断崖）不可达。
//!
//! 对源还原要点（类名:行号均为 26.1 反编译实读）：
//! - **可行走判据**：`WalkNodeEvaluator.getPathTypeStatic`（:465-484）+
//!   `getPathTypeWithinMobBB`（:422-454）——站立格与头顶
//!   `clearance = floor(bbHeight + 1)`（`NodeEvaluator.prepare`，:31）柱内
//!   任一实心 → BLOCKED；脚下实心 → WALKABLE(0)、液体 → WATER(8)
//!   （`PathType.getMalus`，PathType.java:3-30）；脚下悬空 → OPEN，
//!   下坠扫描 `tryFindFirstGroundNodeBelow`（:350-368），深度超过
//!   `getMaxFallDistance` 记 BLOCKED（getBlockedNode :282-287）。
//! - **跳跃上台阶**：`tryJumpOn`（:297-332）+ `getMobJumpHeight`
//!   （:271-273）= max(1.125, maxUpStep)——1 格台阶可上，2 格墙不可。
//! - **对角**：`isDiagonalValid`（:167-181）——两侧正交邻居存在且都不高于
//!   当前节点才允许对角（防穿角）。
//! - **A\* 主循环**：`PathFinder.findPath`（:65-147）——h = 欧氏距离 ×1.5
//!   （:117 `getBestH(...) * 1.5F`）、开放集弹出计数预算 `maxVisitedNodes`
//!   （:81-85）、路径长度门 `walkedDistance < maxPathLength`（:114）+
//!   `current.distanceTo(from) >= maxPathLength` 不再扩展（:106）、
//!   命中判据 = 曼哈顿距离 ≤ reachRange（:91-92）。
//! - **预算**：`maxVisitedNodes = floor(FOLLOW_RANGE × 16)`
//!   （PathNavigation.java:62/70-72）；`maxPathLength = max(FOLLOW_RANGE, 16)`
//!   （PathNavigation.java:71-75）。
//! - **不可达降级**：目标没碰到时取 **h 最小**（离目标最近）节点回溯成
//!   降级路径（PathFinder.java:133-135 `min(comparingDouble(getDistToTarget))`
//!   + `Target.updateBest`，Target.java:24-33）。
//!
//! 有意简化（KNOWN-DIVERGENCE）：门/栅栏/铁轨等 PathType 细分不存在（本仓
//! 方块集只有 solid/liquid）；`canReachWithoutCollision` 碰撞扫掠（:196-212）
//! 不做，宽体怪（spider 1.4）对角以 clearance 判据近似；水柱按 WATER malus
//! 8.0 直接可走（原版非浮水怪 `tryFindFirstNonWaterBelow` 下潜，:334-348
//! ——本仓无游泳，怪会涉水）。

use glam::Vec3;
use mcv_core::{BlockId, BlockPos};
use mcv_game::VoxelAccess;

/// 节点额外代价（`PathType.getMalus`，PathType.java:3-30，本仓只用到这几档）。
pub const MALUS_WALKABLE: f32 = 0.0;
pub const MALUS_WATER: f32 = 8.0;

/// `getMobJumpHeight`（WalkNodeEvaluator.java:271-273）：
/// `max(1.125, maxUpStep)`——本仓 mob 无 STEP_HEIGHT 扰动，取 1.125。
pub const MOB_JUMP_HEIGHT: f32 = 1.125;

/// `Entity.getMaxFallDistance`（Entity.java:3163-3166）= 3。
pub const BASE_FALL_DISTANCE: i32 = 3;

/// `Mob.getMaxFallDistance`（Mob.java:799-811）完整曲线：索敌时按"血量高于
/// 33% 的余量"放宽坠落意愿，且随难度收紧——`sacrifice − (3 − 难度id)×4`；
/// 无目标时 `getComfortableFallDistance(0)` = floor(0 + 3)（LivingEntity
/// .java:1745-1751）。返回可接受的最大下落格数。
pub fn max_fall_distance(has_target: bool, health: f32, max_health: f32, difficulty_id: u8) -> i32 {
    if !has_target {
        return BASE_FALL_DISTANCE;
    }
    let sacrifice = (health - max_health * 0.33) as i32 - (3 - i32::from(difficulty_id.min(3))) * 4;
    BASE_FALL_DISTANCE + sacrifice.max(0)
}

/// 寻路参数（`PathNavigation` 构造 + `WalkNodeEvaluator.prepare` 派生）。
#[derive(Clone, Copy, Debug)]
pub struct PathParams {
    /// `maxPathLength = max(FOLLOW_RANGE, 16)`（PathNavigation.java:71-75）。
    pub max_path_length: f32,
    /// `maxVisitedNodes = floor(FOLLOW_RANGE × 16)`（PathNavigation.java:62）。
    pub max_visited: usize,
    /// `clearance = floor(bbHeight + 1)`（NodeEvaluator.java:31）——
    /// 头顶需连续无实心的格数。
    pub clearance: i32,
    /// `getMaxFallDistance`（Mob.java:799-811）。
    pub max_fall: i32,
}

impl PathParams {
    /// 按 FOLLOW_RANGE 属性与实体高度构造（zombie 1.95 → clearance 2）。
    pub fn ground(follow_range: f32, bb_height: f32, max_fall: i32) -> Self {
        Self {
            max_path_length: follow_range.max(16.0),
            max_visited: (follow_range * 16.0).floor().max(64.0) as usize,
            clearance: (bb_height + 1.0).floor() as i32,
            max_fall,
        }
    }
}

/// 一条寻路结果：`nodes` 为起→终的**后续**航点（起点脚底格已剔除——
/// 它就是当前所在格，不是航点）。
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub nodes: Vec<BlockPos>,
    /// true = 曼哈顿 ≤ reachRange 命中目标（PathFinder.java:91-98）；
    /// false = 预算耗尽/不连通 → 最近降级路径（:133-135）。
    pub reached: bool,
}

impl Path {
    /// 下一个航点世界坐标（节点中心，y 取格底——mob 双脚站在格底）。
    pub fn waypoint(&self, idx: usize) -> Option<Vec3> {
        self.nodes
            .get(idx)
            .map(|p| Vec3::new(p.x as f32 + 0.5, p.y as f32, p.z as f32 + 0.5))
    }
}

#[derive(Clone, Copy)]
struct Node {
    g: f32,
    walked: f32,
    parent: Option<(i32, i32, i32)>,
    closed: bool,
    in_open: bool,
    /// best-h 降级账本（Target.updateBest，Target.java:24-33）。
    best_h: f32,
}

/// 邻居候选：坐标 + 步距（欧氏）+ PathType malus。
type Cand = (i32, i32, i32, f32, f32);

/// A* 求起点（mob 双脚所在格）→ 目标格的可行走路径。
/// `reach_range`：目标曼哈顿命中半径（`moveTo(Entity)` 用 1，
/// PathNavigation.java:186 `createPath(target, 1)`）。
pub fn find_path<V: VoxelAccess>(
    view: &V,
    start: Vec3,
    target: BlockPos,
    p: &PathParams,
) -> Option<Path> {
    // 起点格：`WalkNodeEvaluator.getStart`（:53-101）onGround 分支
    // `floor(y + 0.5)`——本仓 mob pos 即双脚所在世界坐标，floor 即格。
    let from = BlockPos::new(
        start.x.floor() as i32,
        start.y.floor() as i32,
        start.z.floor() as i32,
    );
    let mut nodes: std::collections::HashMap<(i32, i32, i32), Node> =
        std::collections::HashMap::new();
    // 开放集：BinaryHeap(f, 坐标)——正浮点 f32 的 to_bits 保序（f 无 Ord）。
    let mut open: std::collections::BinaryHeap<(std::cmp::Reverse<u32>, (i32, i32, i32))> =
        std::collections::BinaryHeap::new();
    let tgt = (target.x, target.y, target.z);
    let h0 = euclid(from.x, from.y, from.z, target.x, target.y, target.z);
    nodes.insert(
        (from.x, from.y, from.z),
        Node {
            g: 0.0,
            walked: 0.0,
            parent: None,
            closed: false,
            in_open: true,
            best_h: h0,
        },
    );
    open.push((
        std::cmp::Reverse(0.0_f32.to_bits()),
        (from.x, from.y, from.z),
    ));

    // 降级账本：任何插入节点离目标更近就记它（Target.updateBest）。
    let mut best = (from.x, from.y, from.z);
    let mut best_h = h0;
    let mut reached = false;
    let mut popped = 0usize;

    while let Some((_, cur)) = open.pop() {
        let (g_cur, walked_cur) = {
            let c = match nodes.get_mut(&cur) {
                Some(c) if !c.closed => c,
                _ => continue, // 旧条目（f 已刷新）或已关闭。
            };
            c.closed = true;
            c.in_open = false;
            (c.g, c.walked)
        };
        // 命中：曼哈顿 ≤ reachRange（PathFinder.java:91-92，moveTo(Entity)→1）。
        let manh = (cur.0 - tgt.0).abs() + (cur.1 - tgt.1).abs() + (cur.2 - tgt.2).abs();
        if manh <= 1 {
            reached = true;
            best = cur;
            break;
        }
        popped += 1;
        if popped >= p.max_visited {
            break; // 预算耗尽（PathFinder.java:83-86）→ 降级路径。
        }
        // `current.distanceTo(from) >= maxPathLength` 不再扩展（:106）。
        if walked_cur >= p.max_path_length {
            continue;
        }
        // getNeighbors（:121-161）：4 正交 + 4 对角（isDiagonalValid :167-181）。
        let jump_ok = !solid_any(view, cur.0, cur.1 + 1, cur.2, p.clearance);
        let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        let mut cardinal: [Option<Cand>; 4] = [None; 4];
        for (i, (dx, dz)) in dirs.iter().enumerate() {
            cardinal[i] = accepted_node(view, cur.0 + dx, cur.1, cur.2 + dz, p, jump_ok, cur);
        }
        let mut candidates: Vec<Cand> = Vec::with_capacity(8);
        for c in cardinal.iter().flatten() {
            candidates.push(*c);
        }
        for i in 0..4 {
            // 两侧正交都存在且都不高于当前节点才允许对角。
            let j = (i + 1) % 4;
            let (Some(a), Some(b)) = (cardinal[i], cardinal[j]) else {
                continue;
            };
            if a.1 > cur.1 || b.1 > cur.1 {
                continue;
            }
            let (dx, dz) = (dirs[i].0 + dirs[j].0, dirs[i].1 + dirs[j].1);
            if let Some(n) = accepted_node(view, cur.0 + dx, cur.1, cur.2 + dz, p, jump_ok, cur) {
                candidates.push(n);
            }
        }
        for (nx, ny, nz, dist, malus) in candidates {
            // PathFinder.java:113 `tentativeG = g + distance + costMalus`。
            let key = (nx, ny, nz);
            let h = euclid(nx, ny, nz, target.x, target.y, target.z) * 1.5; // :117
            if h < best_h {
                best_h = h;
                best = key;
            }
            if let Some(n) = nodes.get(&key) {
                if n.closed || (n.in_open && g_cur + dist + malus >= n.g) {
                    continue;
                }
            }
            let n = nodes.entry(key).or_insert(Node {
                g: f32::INFINITY,
                walked: 0.0,
                parent: None,
                closed: false,
                in_open: false,
                best_h: h,
            });
            n.parent = Some(cur);
            n.g = g_cur + dist + malus;
            n.walked = walked_cur + dist;
            if !n.in_open {
                n.in_open = true;
                open.push((std::cmp::Reverse((n.g + h).to_bits()), key));
            }
        }
    }

    // 回溯（PathFinder.reconstructPath，:165-176）。
    let mut path = Vec::new();
    let mut cur = best;
    loop {
        let n = nodes.get(&cur)?;
        path.push(BlockPos::new(cur.0, cur.1, cur.2));
        match n.parent {
            Some(prev) => cur = prev,
            None => break,
        }
    }
    path.reverse();
    // 起点即脚底格：航点从下一格开始才有意义；单节点（已贴脸）无航点。
    if path.len() > 1 {
        path.remove(0);
    } else {
        return None;
    }
    Some(Path {
        nodes: path,
        reached,
    })
}

/// 欧氏距离（Node.distanceTo，Node.java:47-49）。
fn euclid(x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> f32 {
    let (dx, dy, dz) = ((x0 - x1) as f32, (y0 - y1) as f32, (z0 - z1) as f32);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// 节点判据（`findAcceptedNode` :229-269 简化）：返回
/// `(x, y, z, 步距, malus)`。步距 = 节点间欧氏距离（PathFinder.distance
/// :149-151），malus 见 PathType.java:3-30。
fn accepted_node<V: VoxelAccess>(
    view: &V,
    x: i32,
    y: i32,
    z: i32,
    p: &PathParams,
    jump_ok: bool,
    cur: (i32, i32, i32),
) -> Option<Cand> {
    if solid_any(view, x, y, z, p.clearance) {
        // tryJumpOn（:297-332）：上方 clearance 柱无实心且抬升 ≤ 1.125 →
        // 跳上 y+1（1 格台阶）；2 格墙 rise=2 > 1.125 拒。
        if jump_ok
            && !solid_any(view, x, y + 1, z, p.clearance)
            && (y + 1 - cur.1) as f32 <= MOB_JUMP_HEIGHT
        {
            let d = euclid(cur.0, cur.1, cur.2, x, y + 1, z);
            return Some((x, y + 1, z, d, MALUS_WALKABLE));
        }
        return None;
    }
    match floor_kind(view, x, y, z) {
        // WALKABLE（脚下实心，malus 0）。
        FloorKind::Ground => {
            let d = euclid(cur.0, cur.1, cur.2, x, y, z);
            Some((x, y, z, d, MALUS_WALKABLE))
        }
        // WATER malus 8（PathType.java:13）。
        FloorKind::Liquid => {
            let d = euclid(cur.0, cur.1, cur.2, x, y, z);
            Some((x, y, z, d, MALUS_WATER))
        }
        // OPEN → tryFindFirstGroundNodeBelow（:350-368）：向下找首个"脚下
        // 实心"的落脚格，深度 > max_fall → BLOCKED（getBlockedNode :282-287）。
        FloorKind::Void => {
            for drop in 1..=p.max_fall {
                let stand = y - drop;
                if view.block(BlockPos::new(x, stand - 1, z)).def().solid {
                    if solid_any(view, x, stand, z, p.clearance) {
                        return None;
                    }
                    let d = euclid(cur.0, cur.1, cur.2, x, stand, z);
                    return Some((x, stand, z, d, MALUS_WALKABLE));
                }
            }
            None
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FloorKind {
    Ground,
    Liquid,
    Void,
}

/// 脚下格判定（getPathTypeStatic :465-484 OPEN 分支 + getPathTypeFromState
/// :515-582 简化到 solid/liquid 二态）。
fn floor_kind<V: VoxelAccess>(view: &V, x: i32, y: i32, z: i32) -> FloorKind {
    let b: BlockId = view.block(BlockPos::new(x, y - 1, z));
    let def = b.def();
    if def.solid {
        FloorKind::Ground
    } else if def.liquid {
        FloorKind::Liquid
    } else {
        FloorKind::Void
    }
}

/// `[y, y+clearance)` 柱内任一实心（getPathTypeWithinMobBB :422-454）。
fn solid_any<V: VoxelAccess>(view: &V, x: i32, y: i32, z: i32, clearance: i32) -> bool {
    for dy in 0..clearance.max(1) {
        if view.block(BlockPos::new(x, y + dy, z)).def().solid {
            return true;
        }
    }
    false
}
