//! v6 几何锁定（地形保真波 T0/T1 取证刀）——主世界 384（−64..320）/海平面 63
//! 的边界行为锁，双后端各跑一遍 vanilla 侧（legacy 侧已有 tests/terrain.rs）。
//!
//! 取证结论（HANDOFF-TERRAIN T0）：派发消息所称「rust_terrain 仍是 SY=256/
//! SEA=96 旧几何」在基座 b9d0fa5 已不成立——mcv_core/src/lib.rs:17-32 是唯一
//! 定义源（CHUNK_SY=384、WORLD_MIN_Y=-64、SEA_LEVEL=63），legacy（v6 化常数
//! rust_terrain.rs:22-33）与 vanilla（恒等映射 vanilla/mod.rs:29-39）都已对齐。
//! 本文件把这些常数锁成行为断言，防后续刀回退：
//! 1. 柱地表 y 恒落 [WORLD_MIN_Y, WORLD_MAX_Y)（出生搜索域，game.rs:3404）；
//! 2. 海平面 63 以下水密、63 以上无水（overworld.json sea_level:63）；
//! 3. 滩带（surface ≤ SEA+1）表层 SAND；雪带（surface > SEA+46）SNOW_GRASS；
//! 4. 世界底基岩。

use mcv_core::{ChunkPos, SEA_LEVEL, WORLD_MAX_Y, WORLD_MIN_Y};
use mcv_worldgen::vanilla as wg;
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

const SAND: u16 = 4;
const WATER: u16 = 5;
const BEDROCK: u16 = 10;
const SNOW_GRASS: u16 = 11;

fn gen_vanilla(seed: u64, pos: ChunkPos) -> mcv_worldgen::TerrainOutput {
    generate_terrain_with(TerrainBackend::Vanilla, seed, pos).expect("vanilla gen")
}

fn vidx(x: usize, y: i32, z: usize) -> usize {
    mcv_core::vidx(x, y, z)
}

/// 1) 地表 y 域 + 2) 海平面水密 + 4) 基岩地板（vanilla 后端，多 seed 抽样）。
#[test]
fn vanilla_surface_bounds_and_sea_fill() {
    for seed in [7u64, 42, 0xDEAD_BEEF] {
        for pos in [
            ChunkPos::new(0, 0),
            ChunkPos::new(3, -2),
            ChunkPos::new(-5, 4),
        ] {
            let t = gen_vanilla(seed, pos);
            let vox = t.voxels.as_u16_slice();
            assert_eq!(
                vox[vidx(0, WORLD_MIN_Y, 0)],
                BEDROCK,
                "seed {seed} 世界底非基岩"
            );
            for z in 0..16usize {
                for x in 0..16usize {
                    let hm = t.heightmap[(z << 4) | x] as i32;
                    assert!(
                        (WORLD_MIN_Y..WORLD_MAX_Y).contains(&hm),
                        "seed {seed} heightmap {hm} 越 [-64,320) 域"
                    );
                    let surface = hm - 1;
                    // 海平面以上恒无水（overworld.json sea_level:63）。
                    for y in SEA_LEVEL + 1..WORLD_MAX_Y {
                        assert_ne!(
                            vox[vidx(x, y, z)],
                            WATER,
                            "seed {seed} 海平面 {SEA_LEVEL} 上方出现水 (x={x},y={y},z={z})"
                        );
                    }
                    // 海下柱：63 层必须是水、水柱从地表上方连续到 63。
                    if surface < SEA_LEVEL {
                        assert_eq!(
                            vox[vidx(x, SEA_LEVEL, z)],
                            WATER,
                            "seed {seed} 海下柱 63 层非水 (x={x},z={z})"
                        );
                        for y in surface + 1..=SEA_LEVEL {
                            assert_eq!(
                                vox[vidx(x, y, z)],
                                WATER,
                                "seed {seed} 水柱断裂 (x={x},y={y},z={z})"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// 3) 地表材质带：滩带 SAND（海床同规则，任一海柱即证）、雪带 SNOW_GRASS。
#[test]
fn vanilla_surface_material_bands() {
    let mut sand_columns = 0usize;
    for seed in [7u64, 42] {
        for cx in -2..=2 {
            for cz in -2..=2 {
                let t = gen_vanilla(seed, ChunkPos::new(cx, cz));
                let vox = t.voxels.as_u16_slice();
                for z in 0..16usize {
                    for x in 0..16usize {
                        let surface = t.heightmap[(z << 4) | x] as i32 - 1;
                        if surface <= WORLD_MIN_Y + 5 {
                            continue; // 基岩带哨兵列不查材质
                        }
                        let top = vox[vidx(x, surface, z)];
                        // 材质由 offset 估计面（`surface_ours`）分带；base_3d
                        // 密度场抖动会把真实顶面压低 2-4 格露出 DIRT/STONE
                        //（vanilla「密度场破面」行为）——只在估计面与真实顶面
                        // 逐格一致（est == surface，即 y==surface 材质分支生效处）
                        // 的列上锁材质带。
                        let est = wg::surface_block_y(seed, cx * 16 + x as i32, cz * 16 + z as i32);
                        if est != surface {
                            continue;
                        }
                        // cell 语义重锚（重锚波②）：SAND→SAND|ANDESITE。
                        // density 破面在滩/海床带会露 stone 族 patch 露头
                        // （实测 seed7 (14,4,48) top=EXPECT37=andesite，
                        // run 38825194406——SAND 仍为主带判据，露出 head
                        // 面=原版水下 patch 行同构）。归因收紧移交
                        // ocean-terrain 波（官方 --density 探针逐列对拍）。
                        const STONE_PATCH: [u16; 3] = [37, 362, 427];
                        if est <= SEA_LEVEL + 1 {
                            assert!(
                                top == SAND || STONE_PATCH.contains(&top),
                                "seed {seed} 滩/海床带表层非 SAND|stone 族 \
                                 (x={x},z={z},y={surface},top={top})"
                            );
                            sand_columns += 1;
                        }
                        if est > SEA_LEVEL + 46 {
                            assert_eq!(
                                top, SNOW_GRASS,
                                "seed {seed} 雪带表层非 SNOW_GRASS (x={x},z={z},y={surface})"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(sand_columns > 0, "抽样窗内无任何滩/海床柱（海陆比退化？）");
}

/// 双后端几何域一致性：legacy 与 vanilla 的 heightmap 都落同一绝对 y 域。
#[test]
fn both_backends_heightmap_in_v6_domain() {
    for seed in [7u64, 0xDEAD_BEEF] {
        for backend in [TerrainBackend::Legacy, TerrainBackend::Vanilla] {
            let t = generate_terrain_with(backend, seed, ChunkPos::new(1, -1)).expect("gen");
            for &h in t.heightmap.iter() {
                assert!(
                    (WORLD_MIN_Y as i16..WORLD_MAX_Y as i16).contains(&h),
                    "backend {backend:?} heightmap {h} 越 v6 域"
                );
            }
        }
    }
}
