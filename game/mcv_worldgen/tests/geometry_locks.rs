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
const DIRT: u16 = 2;
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

/// 3) 地表材质带：滩带（est==SEA+1 乾柱）SAND、海床带（est≤SEA 湿柱）
/// GRAVEL/SAND、雪带 SNOW_GRASS（海洋 SURF-2 波重锚，见测内注记）。
#[test]
fn vanilla_surface_material_bands() {
    use wg::terrain::{GRAVEL, Orchestrator};
    let mut band_columns = 0usize;
    for seed in [7u64, 42] {
        let orch = Orchestrator::new(seed);
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
                        if est <= SEA_LEVEL {
                            // 海床带（湿柱，gen 柱层序里顶层以上必 open_column
                            // 灌水）：官方链按 notUnderDeepWater 门拆两支——
                            // - 门外（深水，y+1 < 63−6−sd）：rule3 ON_FLOOR 兜底
                            //   gravelOrStoneIfCeiling = GRAVEL（SurfaceRuleData
                            //   .java:258-265）；
                            // - 门内（浅水）：UNDER_FLOOR（stoneDepth ≤ 1+sd，
                            //   SurfaceRules.java:35/:684）恒罩 da==1 顶格 ⇒
                            //   biomeUnder 默认 DIRT（:124,253；暖/滩段经
                            //   commonSurfaceAndUnderRules :86 沙）；
                            // - 暖温度段（OBB.java:60 [0.55,1.0] = WARM_OCEAN）
                            //   rule2 :86 / rule3 :262 双覆盖恒沙。
                            // 门公式 blockY+stoneDepthAbove ≥ waterHeight−6−sd
                            // （SurfaceRules.java:802-815 waterStartCheck(-6,-1)
                            // addStoneDepth=true；waterHeight=首液位+1
                            // SurfaceSystem.java:131，26.1 海面顶水块 y=62
                            // 〔java-harness chunk dump 实证〕⇒ waterHeight=63
                            // = SEA——本仓灌水面 63 与若以 63 为顶水读数差一，
                            // 门锚同收敛于 SEA−6−sd）。
                            // —— 海波归因收口（重锚波②移交项）：旧 gate 锚
                            // est−8 ≈ 真顶−0..2，密度破面下沿裸石再被矿层
                            // 染色/碟岩（重锚波② 实证 top=37 安山岩）= 我们的
                            // SurfaceRules 漏支（门基错锚），**非官方 patch
                            // 行为**——官方在带内必铺 surface 材质。修复 =
                            // terrain.rs min_surface_level 官方 findTopSurface
                            // 结构实物 + 本海窗按官方链收紧（本波归因验收：
                            // stone 族 37 安山岩确证非法，期望值=官方链材质）。
                            // （诚实注记：本链气候近似下门基位型与官方差一
                            // 量级，见 KNOWN-DIVERGENCE ③/surface_rules
                            // prelim_gate_probe_constants_anchored。）
                            let cs = orch.column_state(cx * 16 + x as i32, cz * 16 + z as i32);
                            let sd = orch.surface_depth(cx * 16 + x as i32, cz * 16 + z as i32);
                            let gate = surface + 1 >= SEA_LEVEL - 6 - sd;
                            let want = if cs.temperature >= 0.55 {
                                SAND
                            } else if gate {
                                DIRT
                            } else {
                                GRAVEL
                            };
                            assert_eq!(
                                top, want,
                                "seed {seed} 海床带表层非 {want}（temp={:.3},gate={gate}）(x={x},z={z},y={surface},sd={sd})",
                                cs.temperature
                            );
                            band_columns += 1;
                        } else if est == SEA_LEVEL + 1 {
                            // 滩带（est==水线+1 乾柱）顶格沙（既有契约面，
                            // SurfaceRuleData.java:74,86 BEACH/WARM_OCEAN 沙）。
                            assert_eq!(
                                top, SAND,
                                "seed {seed} 滩带表层非 SAND (x={x},z={z},y={surface})"
                            );
                            band_columns += 1;
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
    assert!(band_columns > 0, "抽样窗内无任何滩/海床柱（海陆比退化？）");
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
