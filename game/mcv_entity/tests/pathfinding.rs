//! A* 寻路对源测试（审计 N-1）。三道派单验收 + 难度坠落曲线：
//! 绕 2 格高墙 / 坠落 ≤3 接受（>3 拒）/ 不可达返回降级路径。
//! 数值依据见 pathfinding.rs 模块注释（Java 行号）。

use std::collections::HashSet;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkPos};
use mcv_entity::pathfinding::{BASE_FALL_DISTANCE, PathParams, find_path, max_fall_distance};
use mcv_game::VoxelAccess;

/// 网格世界：实心集合外的体素全是空气（BlockId(0)）。
struct Grid {
    solid: HashSet<(i32, i32, i32)>,
}

impl Grid {
    fn flat() -> Self {
        // y=-1 一层地板，站立面 y=0；z 全宽（-24..24）。
        let mut solid = HashSet::new();
        for x in -24..24 {
            for z in -24..24 {
                solid.insert((x, -1, z));
            }
        }
        Self { solid }
    }

    /// 横墙：x 列、z ∈ range、y0..=y1 实心（留 z=13..23 绕行口）。
    fn wall(&mut self, x: i32, z0: i32, z1: i32, y0: i32, y1: i32) {
        for z in z0..=z1 {
            for y in y0..=y1 {
                self.solid.insert((x, y, z));
            }
        }
    }

    /// 全宽沟槽：x0..=x1 列的地板往下挖 depth 层（站立面降到 -depth）。
    fn trench(&mut self, x0: i32, x1: i32, depth: i32) {
        for x in x0..=x1 {
            for z in -24..24 {
                for y in 0..depth {
                    self.solid.remove(&(x, -1 - y, z));
                }
            }
        }
    }
}

impl VoxelAccess for Grid {
    fn block(&self, p: BlockPos) -> BlockId {
        if self.solid.contains(&(p.x, p.y, p.z)) {
            BlockId(1)
        } else {
            BlockId(0)
        }
    }
    fn light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, _c: ChunkPos) -> bool {
        true
    }
}

/// 大视距参数：墙/沟几何的绕行距离 > 16，用 follow 64
/// （maxPathLength 64 / 预算 1024，PathNavigation.java:62/71-75）。
fn params64() -> PathParams {
    PathParams::ground(64.0, 1.95, BASE_FALL_DISTANCE)
}

/// 小视距参数（预算/长度门测试）：follow 16 → maxVisited 256。
fn params() -> PathParams {
    PathParams::ground(16.0, 1.95, BASE_FALL_DISTANCE)
}

fn bp(x: i32, y: i32, z: i32) -> BlockPos {
    BlockPos::new(x, y, z)
}

/// 派单验收 1：2 格高墙不可直穿、不可跳上（jumpHeight 1.125 < 2），
/// A* 必须绕行且最终抵达（reached=true）。
#[test]
fn detours_around_two_high_wall() {
    let mut g = Grid::flat();
    // 墙在 x=4 列（z=-12..12），高 2（y=0/1），站立面 y=0。
    g.wall(4, -12, 12, 0, 1);
    let path = find_path(&g, Vec3::new(0.0, 0.0, 0.0), bp(8, 0, 0), &params64())
        .expect("可绕行：路径存在");
    assert!(path.reached, "绕行抵达：{path:?}");
    assert!(
        path.nodes
            .iter()
            .all(|n| !(n.x == 4 && (-12..=12).contains(&n.z))),
        "不穿墙列（只能绕 z=±13 端）：{path:?}"
    );
    assert!(
        path.nodes.iter().all(|n| n.y <= 1),
        "不飞墙顶（墙高 2 > jump 1.125）：{path:?}"
    );
}

/// 1 格台阶可跳上（tryJumpOn：rise 1 ≤ 1.125，WalkNodeEvaluator.java:271-273）。
#[test]
fn climbs_one_block_step() {
    let mut g = Grid::flat();
    g.wall(4, -12, 12, 0, 0); // 1 高横台阶
    let path = find_path(&g, Vec3::new(0.0, 0.0, 0.0), bp(8, 0, 0), &params64()).expect("可上台阶");
    assert!(path.reached, "{path:?}");
    assert!(
        path.nodes.iter().any(|n| n.y == 1),
        "路径含 y+1 跳跃节点：{path:?}"
    );
}

/// 派单验收 2：坠落 ≤3 接受（tryFindFirstGroundNodeBelow 门
/// WalkNodeEvaluator.java:350-368），>3 拒。
#[test]
fn falls_up_to_three_blocks_only() {
    // 深 3 的沟槽（x=4..6，站立面 y=-3）：目标在沟内——mob 愿意跳下追击。
    let mut g3 = Grid::flat();
    g3.trench(4, 6, 3);
    let p3 = find_path(&g3, Vec3::new(0.0, 0.0, 0.0), bp(5, -3, 0), &params64())
        .expect("3 格坠落可接受");
    assert!(p3.reached, "下沟抵达：{p3:?}");
    assert!(
        p3.nodes.iter().any(|n| n.y == -3 && (4..=6).contains(&n.x)),
        "落沟节点 {p3:?}"
    );

    // 深 4 的沟槽：跳不得（maxFall=3）→ 不可达降级。
    let mut g4 = Grid::flat();
    g4.trench(4, 6, 4);
    let p4 = find_path(&g4, Vec3::new(0.0, 0.0, 0.0), bp(5, -4, 0), &params64());
    match p4 {
        Some(p) => {
            assert!(!p.reached, "4 格深沟不可达 {p:?}");
            assert!(
                !p.nodes.iter().any(|n| n.y <= -4),
                "降级路径也不下沟：{p:?}"
            );
        }
        None => {} // 完全无路径也合法。
    }
}

/// 派单验收 3：不可达 → 降级路径（reached=false，取离目标最近节点，
/// PathFinder.java:133-135 + Target.updateBest）。
#[test]
fn unreachable_returns_closest_degraded_path() {
    let mut g = Grid::flat();
    // 目标 (8,0,0) 四面 2 高墙封死（墙顶 jump 不上：rise 2 > 1.125）。
    g.wall(7, -2, 2, 0, 1);
    g.wall(9, -2, 2, 0, 1);
    g.wall(8, -2, -2, 0, 1);
    g.wall(8, 2, 2, 0, 1);
    let start = Vec3::new(0.0, 0.0, 0.0);
    let path = find_path(&g, start, bp(8, 0, 0), &params64()).expect("降级路径存在");
    assert!(!path.reached, "不可达：{path:?}");
    // 降级终点必须比起点更接近目标（best-h 账本生效）。
    let end = path.nodes[path.nodes.len() - 1];
    let d0 = (start - Vec3::new(8.0, 0.0, 0.0)).length();
    let d1 =
        (Vec3::new(end.x as f32, end.y as f32, end.z as f32) - Vec3::new(8.0, 0.0, 0.0)).length();
    assert!(d1 < d0, "降级终点接近目标 {d1} < {d0}");
}

/// 预算上限不挂死：远超 maxPathLength 的目标 → 预算/长度门终止，返回降级。
#[test]
fn budget_exhausts_into_degraded_path() {
    let g = Grid::flat();
    let path = find_path(&g, Vec3::new(0.0, 0.0, 0.0), bp(40, 0, 0), &params());
    let p = path.expect("预算耗尽 → 降级");
    assert!(!p.reached, "超出 maxPathLength 不可达 {p:?}");
    assert!(!p.nodes.is_empty());
}

/// Mob.java:799-811 — getMaxFallDistance 难度/血量曲线：
/// 无目标 3；满血索敌 = floor((20−6.6)) −(3−难度)×4 + 3。
#[test]
fn fall_distance_difficulty_curve() {
    assert_eq!(max_fall_distance(false, 20.0, 20.0, 2), 3);
    // 满血索敌：sacrifice = (20−6.6)=13 → normal −4 → 9；+3 = 12。
    assert_eq!(max_fall_distance(true, 20.0, 20.0, 2), 12);
    // hard：sacrifice −0 → 16；easy −8 → 8；peaceful −12 → 4。
    assert_eq!(max_fall_distance(true, 20.0, 20.0, 3), 16);
    assert_eq!(max_fall_distance(true, 20.0, 20.0, 1), 8);
    assert_eq!(max_fall_distance(true, 20.0, 20.0, 0), 4);
    // 残血收紧：health=7 → (7−6.6)=0 → normal −4 → clamp 0 → 3。
    assert_eq!(max_fall_distance(true, 7.0, 20.0, 2), 3);
}
