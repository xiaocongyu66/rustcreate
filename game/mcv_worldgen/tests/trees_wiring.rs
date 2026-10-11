//! #139 树接线波——vanilla 后端 pass 2 树投影行为锁。
//!
//! vanilla `terrain::generate()` 原本没有任何装饰 pass（HANDOFF-TERRAIN T0
//! 取证结论 1：游戏里见不到树的实链根因）；本文件把 #135 的
//! `trees::place_tree` 接入后的可观测行为锁死：
//! 1. Forest 窗 logs ∈ 带（密度门 + 树投影生效）；
//! 2. 树基 y > 海平面且落点方块为草地类（基座门 + 立地门）；
//! 3. 同 seed 两次 generate 字节一致（树 pass 世界坐标纯函数，无共享流）；
//! 4. Ocean/Desert 窗 logs == 0（群系密度门 0 生效）；
//! 5. 每棵树基下方非空气（无悬空树）。
//!
//! 窗口常量由 fixture 扫描选定（用 main 上已有的 `biome_at`/
//! `surface_block_y`，每列前置断言锁窗——气候漂移先红在窗口断言，不静默
//! 改靶）：Forest 窗 256 列全 Forest 且 surface_ours 恒 65（非滩 >SEA+1、
//! 非雪带 ≤SEA+46）；Ocean/Desert 窗 256 列全同群系且全陆地。
//!
//! 密度带出处（26.1 数据，非拍脑袋）：`data/minecraft/worldgen/placed_feature
//! /trees_birch_and_oak_leaf_litter.json` count = weighted_list(10 w9,
//! 11 w1) → E=10.1 次尝试/区块；本仓 5×5 树池每区块恰交 4×4=16 池
//! （任意 16 格窗口与 5 格池交叠恒 4 池/轴），每池成树概率 = 10.1/16。
//! 每树 logs = 树干高 4..6（oak，TreeFeatures.java:136 (4,2,0)）/
//! 5..7（birch :181 (5,2,0)）→ 期望 logs ≈ 10.1 × 5.4 ≈ 54。

use mcv_core::{ChunkPos, SEA_LEVEL};
use mcv_worldgen::trees::ids;
use mcv_worldgen::vanilla::{Biome, biome_at, surface_block_y};
use mcv_worldgen::{TerrainBackend, TerrainOutput, generate_terrain_with};

const FOREST_SEED: u64 = 0x7E_A20_0B1;
const FOREST_CX: i32 = -21;
const FOREST_CZ: i32 = 24;

const OCEAN_SEED: u64 = 0x7E_A20_0B1;
const OCEAN_CX: i32 = -24;
const OCEAN_CZ: i32 = -24;

const DESERT_SEED: u64 = 99;
const DESERT_CX: i32 = 49;
const DESERT_CZ: i32 = 20;

/// Forest 窗 logs 计数带宽（绿相位以实测校准；红相位先放全带锁 logs>0）。
const FOREST_LOGS_MIN: usize = 1;
const FOREST_LOGS_MAX: usize = 10_000;

/// 六树种原木 id（trees::ids，blocks_gen.inc.rs 引注）。
const LOG_IDS: [u16; 6] = [
    ids::OAK_LOG,
    ids::ACACIA_LOG,
    ids::BIRCH_LOG,
    ids::DARK_OAK_LOG,
    ids::JUNGLE_LOG,
    ids::SPRUCE_LOG,
];
/// 草地类立地（vanilla SUPPORTS_VEGETATION 的本仓生成子集：草方块/泥土/
/// 雪草——biome/沙/石/水按原版 would_survive 语义放弃该位）。
const GROUND_IDS: [u16; 3] = [3, 2, 11];

const AIR: u16 = 0;
const WATER: u16 = 5;

fn is_log(id: u16) -> bool {
    LOG_IDS.contains(&id)
}

fn gen_vanilla(seed: u64, cx: i32, cz: i32) -> TerrainOutput {
    generate_terrain_with(TerrainBackend::Vanilla, seed, ChunkPos::new(cx, cz))
        .expect("vanilla gen")
}

fn count_logs(vox: &[u16]) -> usize {
    vox.iter().filter(|&&v| is_log(v)).count()
}

/// 树基收集：某列的 log 柱底（log 且下方非任意 log 且上方仍为同种 log）。
/// acacia 斜枝/暗橡树外圈枝的单格悬空枝干不满足「上方同 log」被排除——
/// 它们本就靠树干斜向悬挂，不是树基。
fn tree_bases(vox: &[u16]) -> Vec<(usize, i32, usize)> {
    let mut bases = Vec::new();
    for z in 0..16usize {
        for x in 0..16usize {
            for y in (mcv_core::WORLD_MIN_Y + 1)..(mcv_core::WORLD_MAX_Y - 1) {
                let id = vox[mcv_core::vidx(x, y, z)];
                let above = vox[mcv_core::vidx(x, y + 1, z)];
                let below = vox[mcv_core::vidx(x, y - 1, z)];
                if is_log(id) && above == id && !is_log(below) {
                    bases.push((x, y, z));
                }
            }
        }
    }
    bases
}

/// 1) Forest 窗：256 列前置锁窗 + logs ∈ 带。
#[test]
fn forest_window_tree_count_in_band() {
    for z in 0..16_i32 {
        for x in 0..16_i32 {
            let wx = FOREST_CX * 16 + x;
            let wz = FOREST_CZ * 16 + z;
            assert_eq!(
                biome_at(FOREST_SEED, wx, wz),
                Biome::Forest,
                "Forest 窗气候漂移 (x={wx},z={wz})"
            );
            let s = surface_block_y(FOREST_SEED, wx, wz);
            assert!(
                s > SEA_LEVEL + 1 && s <= SEA_LEVEL + 46,
                "Forest 窗地表 {s} 落入滩/雪带 (x={wx},z={wz})"
            );
        }
    }
    let t = gen_vanilla(FOREST_SEED, FOREST_CX, FOREST_CZ);
    let logs = count_logs(t.voxels.as_u16_slice());
    assert!(
        (FOREST_LOGS_MIN..=FOREST_LOGS_MAX).contains(&logs),
        "Forest 窗 logs={logs} 越带 [{FOREST_LOGS_MIN},{FOREST_LOGS_MAX}]（密度门失效？）"
    );
    assert!(logs > 0, "Forest 窗无树（vanilla 装饰 pass 未生效）");
}

/// 2)+5) 树基门：基 y > SEA、下方为草地类且非空气/水（无悬空树）。
#[test]
fn tree_bases_above_sea_on_grassy_ground() {
    let t = gen_vanilla(FOREST_SEED, FOREST_CX, FOREST_CZ);
    let vox = t.voxels.as_u16_slice();
    let bases = tree_bases(vox);
    assert!(!bases.is_empty(), "Forest 窗无任何树基");
    for &(x, y, z) in &bases {
        assert!(y > SEA_LEVEL, "树基 {y} ≤ 海平面 {SEA_LEVEL} (x={x},z={z})");
        let below = vox[mcv_core::vidx(x, y - 1, z)];
        assert_ne!(below, AIR, "树基下方悬空 (x={x},y={y},z={z})");
        assert_ne!(below, WATER, "树基下方悬水 (x={x},y={y},z={z})");
        assert!(
            GROUND_IDS.contains(&below),
            "树基下方 {below} 非草地类 (x={x},y={y},z={z})"
        );
    }
}

/// 3) 确定性：同 seed 两次 generate 字节一致（体素 + 高度图）。
#[test]
fn deterministic_bytes_with_trees() {
    let a = gen_vanilla(FOREST_SEED, FOREST_CX, FOREST_CZ);
    let b = gen_vanilla(FOREST_SEED, FOREST_CX, FOREST_CZ);
    assert_eq!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
    assert_eq!(a.heightmap, b.heightmap);
}

/// 4) 密度门 0 群系：Ocean/Desert 窗 logs == 0。
#[test]
fn ocean_and_desert_windows_treeless() {
    for (name, seed, cx, cz, want) in [
        ("Ocean", OCEAN_SEED, OCEAN_CX, OCEAN_CZ, Biome::Ocean),
        ("Desert", DESERT_SEED, DESERT_CX, DESERT_CZ, Biome::Desert),
    ] {
        for z in 0..16_i32 {
            for x in 0..16_i32 {
                let wx = cx * 16 + x;
                let wz = cz * 16 + z;
                assert_eq!(
                    biome_at(seed, wx, wz),
                    want,
                    "{name} 窗气候漂移 (x={wx},z={wz})"
                );
            }
        }
        let t = gen_vanilla(seed, cx, cz);
        let logs = count_logs(t.voxels.as_u16_slice());
        assert_eq!(logs, 0, "{name} 窗出现 {logs} 根原木（密度门 0 失效）");
    }
}
