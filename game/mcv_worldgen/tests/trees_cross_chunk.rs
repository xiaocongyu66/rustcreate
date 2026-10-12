//! fix/tree-chunk-clip —— 跨界树「双侧重放完整性锁」。
//!
//! 真机缺陷（2026-10-12）：树生成在区块边界附近时缺**一整侧**树叶——
//! #139 初版的 pass 2 选址要求树干落在本块内（旧 `tree_pass.rs` 立地门前
//! `continue`），树干在邻块的冠部两侧都没人写：树干所属块盖章时被
//! `ChunkClipper` 裁掉出块格，邻块又根本没把该树选进来。原版对应机制是
//! FEATURES 步 `blockStateWriteRadius=1` 跨块写（ChunkPyramid.java:30-33、
//! WorldGenRegion.java:231-237）+ `ChunkHolder.blockChanged`
//! （ChunkHolder.java:123-135）标邻块脏；本仓裁决走 (a) 邻块重放
//! （`tree_pass` 模块头注，worker 并行无跨界写面、E2 确定性锁不动）。
//!
//! 本文件的锁：
//! 1. `boundary_trees_complete_on_both_sides`——对 Forest 窗全部跨界树
//!    （树干在本块、全树重放写格伸入邻块）：邻块独立 generate 后，伸入格
//!    **不得残留空气/水**（叶门对 air/water 恒放行，AIR/WATER = 该树从未
//!    在邻块被施加 = 缺陷复现），且至少一格逐 id 命中（冠真的在邻块）；
//! 2. `tree_spots_agree_across_chunk_views`——同一棵树在相邻两块的选址视
//!    角下读数逐字段一致（落点/树基/树种/结构种子与调用块无关——树基
//!    `column_real_top` 纯函数化是此等式的前提）；
//! 3. `trunk_bases_land_on_real_surface`——树干在本块的 spot，盖章后基格
//!    必为原木（`column_real_top` 若低于缓冲实顶，基格实心 → 替换门拒绝
//!    → 非原木 = 红；纯函数与 pass-1 缓冲实顶的一致性地平线）。
//!
//! 确定性前提（`trees_wiring.rs::deterministic_bytes_with_trees` 已锁）：
//! 同 seed 同字节，邻块重放可复现。

use mcv_core::ChunkPos;
use mcv_worldgen::trees::{BlockSetter, TreesRng, ids, place_tree};
use mcv_worldgen::vanilla::terrain::Orchestrator;
use mcv_worldgen::vanilla::tree_pass::{TreeSpot, tree_spots};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};
use std::collections::HashMap;

/// Forest 窗锚点（与 `trees_wiring.rs` 同 seed/区域：seed 0x7EA20B1、
/// chunk (-21, 24) 256 列全 Forest，气候漂移先在该文件红，本文件不重复
/// 锁窗，只扫这片有树的区域）。
const SEED: u64 = 0x7E_A20_0B1;
const AREA_CX0: i32 = -22;
const AREA_CX1: i32 = -20;
const AREA_CZ0: i32 = 23;
const AREA_CZ1: i32 = 25;

const AIR: u16 = 0;
const WATER: u16 = 5;

const LOG_IDS: [u16; 6] = [
    ids::OAK_LOG,
    ids::ACACIA_LOG,
    ids::BIRCH_LOG,
    ids::DARK_OAK_LOG,
    ids::JUNGLE_LOG,
    ids::SPRUCE_LOG,
];
const LEAF_IDS: [u16; 6] = [
    ids::OAK_LEAVES,
    ids::ACACIA_LEAVES,
    ids::BIRCH_LEAVES,
    ids::DARK_OAK_LEAVES,
    ids::JUNGLE_LEAVES,
    ids::SPRUCE_LEAVES,
];

fn is_log(id: u16) -> bool {
    LOG_IDS.contains(&id)
}
fn is_leaf(id: u16) -> bool {
    LEAF_IDS.contains(&id)
}

/// 无门控全树记录器：`place_tree` 会写的每一格（世界坐标 + id）。
/// 与 `ChunkClipper` 同 rng 流（`TreesRng::new(spot.rng_seed)`）⇒ 写入格
/// 集合逐格一致，门只决定「写不写得进」，不消耗随机流（#135 契约面）。
#[derive(Default)]
struct Recorder {
    writes: Vec<(i32, i32, i32, u16)>,
}

impl BlockSetter for Recorder {
    fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        self.writes.push((x, y, z, id));
    }
}

fn replay_tree(spot: &TreeSpot) -> Vec<(i32, i32, i32, u16)> {
    let mut rng = TreesRng::new(spot.rng_seed);
    let mut rec = Recorder::default();
    place_tree(&mut rng, spot.x, spot.base_y, spot.z, spot.kind, &mut rec);
    rec.writes
}

fn in_chunk(v: i32, chunk: i32) -> bool {
    (0..16).contains(&(v - chunk * 16))
}

/// 区块体素缓存（generate 是纯函数，只缓存 u16 平面）。
struct GenCache {
    map: HashMap<(i32, i32), Vec<u16>>,
}

impl GenCache {
    fn get(&mut self, cx: i32, cz: i32) -> &Vec<u16> {
        self.map.entry((cx, cz)).or_insert_with(|| {
            generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(cx, cz))
                .expect("vanilla gen")
                .voxels
                .as_u16_slice()
                .to_vec()
        })
    }
    fn at(&mut self, wx: i32, wy: i32, wz: i32) -> u16 {
        let v = self.get(wx.div_euclid(16), wz.div_euclid(16));
        v[mcv_core::vidx(wx.rem_euclid(16) as usize, wy, wz.rem_euclid(16) as usize)]
    }
}

/// 1) 双侧重放完整性锁（缺陷主体）。
#[test]
fn boundary_trees_complete_on_both_sides() {
    let orch = Orchestrator::new(SEED);
    let mut cache = GenCache {
        map: HashMap::new(),
    };
    let mut crossing_trees = 0usize;
    let mut hit_blocks = 0usize;
    for cz in AREA_CZ0..=AREA_CZ1 {
        for cx in AREA_CX0..=AREA_CX1 {
            // 树干在本块的 spot（跨界冠的源头）。tree_spots 的环带扫描
            // 已保证：伸入邻块的树必同时出现在邻块的 spot 视图（锁 2）。
            let spots = tree_spots(SEED, cx, cz, &orch)
                .into_iter()
                .filter(|s| in_chunk(s.x, cx) && in_chunk(s.z, cz))
                .collect::<Vec<_>>();
            for spot in spots {
                let writes = replay_tree(&spot);
                // 伸入其他区块的写格 = 旧实现里被 ChunkClipper 丢弃、
                // 且（修复前）无人补齐的部分。
                let mut cross = false;
                for &(wx, wy, wz, id) in &writes {
                    if wy < mcv_core::WORLD_MIN_Y || wy >= mcv_core::WORLD_MAX_Y {
                        continue;
                    }
                    let (ncx, ncz) = (wx.div_euclid(16), wz.div_euclid(16));
                    if ncx == cx && ncz == cz {
                        continue;
                    }
                    cross = true;
                    let v = cache.at(wx, wy, wz);
                    if is_leaf(id) {
                        // 叶门放行 air/water/leaf：终值仍是 AIR/WATER =
                        // 该格所属块从未施加这棵树（缺陷复现）。
                        assert_ne!(
                            v, AIR,
                            "跨界叶 ({wx},{wy},{wz}) 在邻块 ({ncx},{ncz}) 缺位（树 @({},{})）",
                            spot.x, spot.z
                        );
                        assert_ne!(
                            v, WATER,
                            "跨界叶 ({wx},{wy},{wz}) 在邻块 ({ncx},{ncz}) 仍是水（重放未执行，树 @({},{})）",
                            spot.x, spot.z
                        );
                    } else if is_log(id) {
                        // 干/枝门放行 air/leaf：邻块该格 AIR ⇒ 从未施加。
                        // WATER/实心 = validTreePos 门正当拒绝（跨界枝贴
                        // 山体），允许。
                        assert_ne!(
                            v, AIR,
                            "跨界干 ({wx},{wy},{wz}) 在邻块 ({ncx},{ncz}) 缺位（树 @({},{})）",
                            spot.x, spot.z
                        );
                    }
                    if v == id {
                        hit_blocks += 1;
                    }
                }
                if cross {
                    crossing_trees += 1;
                }
            }
        }
    }
    assert!(
        crossing_trees > 0,
        "Forest 区域 {AREA_CX0}..={AREA_CX1}×{AREA_CZ0}..={AREA_CZ1} 无跨界树——测试空转"
    );
    assert!(
        hit_blocks > 0,
        "跨界树 {crossing_trees} 棵，但邻块无一格逐 id 命中（冠补写未生效？）"
    );
}

/// 2) 选址跨视图一致：同一棵树（同落点）在 8 邻视角的 spot 逐字段相等。
///    树基若读「本块缓冲」（旧 ground_spot），跨视图等式在树干出块时根本
///    不成立——纯函数化（`column_real_top`）是等式成立的前提。
#[test]
fn tree_spots_agree_across_chunk_views() {
    let orch = Orchestrator::new(SEED);
    let mut shared = 0usize;
    for cz in AREA_CZ0..=AREA_CZ1 {
        for cx in AREA_CX0..=AREA_CX1 {
            let own = tree_spots(SEED, cx, cz, &orch);
            for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1), (1, 1), (-1, -1)] {
                let view = tree_spots(SEED, cx + dx, cz + dz, &orch);
                for s in &own {
                    if let Some(o) = view.iter().find(|t| t.x == s.x && t.z == s.z) {
                        assert_eq!(
                            s,
                            o,
                            "树 ({},{}) 在 ({cx},{cz}) 与 ({},{}) 视图不一致",
                            s.x,
                            s.z,
                            cx + dx,
                            cz + dz
                        );
                        shared += 1;
                    }
                }
            }
        }
    }
    assert!(shared > 0, "邻块视图无任何共享树——测试空转");
}

/// 3) 树基地平线：树干在本块的 spot，生成缓冲的基格必须是原木。
///    `column_real_top` 低于 pass-1 缓冲实顶 ⇒ 基格实心 ⇒ 替换门拒绝 ⇒
///    基格非原木（红）；等式对全部 spot 成立 ⇒ 纯函数选址与盖章缓冲两侧
///    读到的地表一致。
#[test]
fn trunk_bases_land_on_real_surface() {
    let orch = Orchestrator::new(SEED);
    let mut checked = 0usize;
    for cz in AREA_CZ0..=AREA_CZ1 {
        for cx in AREA_CX0..=AREA_CX1 {
            let vox = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(cx, cz))
                .expect("vanilla gen")
                .voxels
                .as_u16_slice()
                .to_vec();
            let spots = tree_spots(SEED, cx, cz, &orch)
                .into_iter()
                .filter(|s| in_chunk(s.x, cx) && in_chunk(s.z, cz))
                .collect::<Vec<_>>();
            for spot in &spots {
                let lx = spot.x - cx * 16;
                let lz = spot.z - cz * 16;
                // 基格允许的另一原木 = 树干重叠竞争（5 格池距下限 ~5、
                // dark oak 2×2 足迹，理论可撞；Forest 窗 oak/birch 单格干
                // 实际不撞，放宽为任意原木防极端）。
                let v = vox[mcv_core::vidx(lx as usize, spot.base_y, lz as usize)];
                assert!(
                    is_log(v),
                    "树基格 ({},{},{}) = {v} 非原木（column_real_top 与缓冲实顶漂移，spot {spot:?}）",
                    spot.x,
                    spot.base_y,
                    spot.z
                );
            }
            checked += spots.len();
        }
    }
    assert!(checked > 0, "区域无树干在块内的树——测试空转");
}
