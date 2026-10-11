//! #140 地表内核波——26.1 SurfaceRules 取证行为锁（红相位先落）。
//!
//! 取证锚点（26.1 原版，/root/mc-ref/src-26.1，铁律 1；Java 相对
//! `net/minecraft/`，下同）：
//! - **表层由「上下文实际方块/相对位置」决定，不是样条估计面**：
//!   `SurfaceSystem.buildSurface`（world/level/levelgen/SurfaceSystem.java:
//!   106-163）逐列从 WORLD_SURFACE_WG 高度图顶（:110/:117）自顶向下扫描，
//!   空气重置 stoneAboveDepth（:126-128）、实心则 `stoneAboveDepth++`（:146）
//!   ——`ON_FLOOR` = `stoneDepthCheck(0,false,FLOOR)` 即
//!   `stoneDepthAbove <= 1`（SurfaceRules.java:680，顶层实心格），
//!   `UNDER_FLOOR` = `stoneDepthCheck(0,true,FLOOR)` 再加 surfaceDepth 噪声带
//!   （SurfaceRules.java:676-680；SurfaceSystem.java:165-168 深度 0..4）。
//! - **顶格方块按群系与水情**：内陆默认
//!   `grassOrDirtIfUnderwater = sequence(ifTrue(aboveWater, GRASS_BLOCK), DIRT)`
//!   （data/worldgen/SurfaceRuleData.java:71,165）——水上顶格 = 草方块、
//!   水下沉底才允许泥土；沙滩带 `biomesWithSandAndSandstone`
//!   （:74,86 BEACH/WARM_OCEAN/SNOWY_BEACH）与沙漠（:75,87 DESERT）顶格/亚表
//!   = 沙；石质崖/峰（steep，:100/:108/:141）只作用于 FROZEN_PEAKS/
//!   SNOW_SLOPES/JAGGED_PEAKS 三个本仓缺席群系——内陆草群系的悬崖侧面各列
//!   实顶照常长草（每列自己的顶），无独立「steep 换石」规则。
//! - **带外回落石**：全部表层规则包在 `abovePreliminarySurface` 门内
//!   （SurfaceRuleData.java:278；SurfaceRules.java:379
//!   `preliminarySurfaceLevel + surfaceDepth - 8`）——带下（深洞底）保持石头。
//!
//! 本仓 v1 缺口（KNOWN-DIVERGENCE SURF-1）：旧 pass 1 把表层锚在样条估计面
//! `surface_ours = 128+128·offset`（与密度场真实过零面差 ±1-2 格），
//! `y == surface → GRASS`、`surface-3 ≤ y < surface → DIRT`——真实实顶低于
//! 估计面时暴露层落在 DIRT/STONE 带 = 用户可见的「整片裸土」。

use mcv_core::{ChunkPos, SEA_LEVEL, WORLD_MAX_Y, WORLD_MIN_Y};
use mcv_worldgen::trees::ids;
use mcv_worldgen::vanilla::terrain::Orchestrator;
use mcv_worldgen::vanilla::{Biome, biome_at, surface_block_y};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

/// 主测种子（与 quality.rs / trees_wiring.rs 同源）。
const SEED: u64 = 0x7E_A20_0B1_u64;

const AIR: u16 = 0;
const STONE: u16 = 1;
const DIRT: u16 = 2;
const GRASS: u16 = 3;
const SAND: u16 = 4;
const WATER: u16 = 5;
const BEDROCK: u16 = 10;
const SNOW_GRASS: u16 = 11;

/// 六树种原木/叶（树基列排除：原版树基把表土换成 dirt，TreeConfiguration
/// .java:20-26 经 tree_pass ChunkClipper 落地——不计入表层裸土症状）。
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

fn vidx(x: usize, y: i32, z: usize) -> usize {
    mcv_core::vidx(x, y, z)
}

fn is_tree(id: u16) -> bool {
    LOG_IDS.contains(&id) || LEAF_IDS.contains(&id)
}

/// 一列的「地面方块」：跳过空气/水/树木原木与叶后的首个实心块
/// （对应原版每列自顶向下扫描的 ON_FLOOR 顶层格，SurfaceSystem.java:124-148）。
fn ground_block(vox: &[u16], x: usize, z: usize) -> Option<(i32, u16)> {
    for y in ((WORLD_MIN_Y + 1)..(WORLD_MAX_Y - 1)).rev() {
        let id = vox[vidx(x, y, z)];
        if id == AIR || id == WATER || is_tree(id) {
            continue;
        }
        return Some((y, id));
    }
    None
}

/// 一列是否含树干（树基列——原版基下换土合法，SurfaceRuleData 语义之外）。
fn column_has_log(vox: &[u16], x: usize, z: usize) -> bool {
    for y in (WORLD_MIN_Y..WORLD_MAX_Y).rev() {
        if LOG_IDS.contains(&vox[vidx(x, y, z)]) {
            return true;
        }
    }
    false
}

/// 扫描一个「全 want 群系、全部列在非滩非雪陆带」的 16×16 群系窗
/// （trees_wiring.rs 同约定：窗口前置断言锁窗，气候漂移先红在断言）。
/// 每块先 4 角预筛再全 256 列确认；群系/估计面走同一 Orchestrator
/// （biome_at/表面样条口径与生成路径一致）。
fn find_biome_window(orch: &Orchestrator, want: Biome) -> Option<(i32, i32)> {
    for cz in -96..96 {
        for cx in -96..96 {
            // 4 角预筛（滩带/雪带/群系不对即弃）。
            let corners_ok = [(0, 0), (15, 0), (0, 15), (15, 15)]
                .iter()
                .all(|&(dx, dz)| {
                    let wx = cx * 16 + dx;
                    let wz = cz * 16 + dz;
                    let est = orch.column_state(wx, wz).surface_ours;
                    est > SEA_LEVEL + 1 && est <= SEA_LEVEL + 46 && orch.biome_at(wx, wz) == want
                });
            if !corners_ok {
                continue;
            }
            let mut all = true;
            for dz in 0..16_i32 {
                for dx in 0..16_i32 {
                    let wx = cx * 16 + dx;
                    let wz = cz * 16 + dz;
                    let est = orch.column_state(wx, wz).surface_ours;
                    if !(est > SEA_LEVEL + 1 && est <= SEA_LEVEL + 46)
                        || orch.biome_at(wx, wz) != want
                    {
                        all = false;
                        break;
                    }
                }
                if !all {
                    break;
                }
            }
            if all {
                return Some((cx, cz));
            }
        }
    }
    None
}

/// 内陆草群系窗（Plains/Forest）：每列地面 ∈ {GRASS, SNOW_GRASS}。
///
/// 26.1 语义：内陆非水下顶格 = GRASS_BLOCK（SurfaceRuleData.java:71,165
/// grassOrDirtIfUnderwater 的 aboveWater 分支）——泥土只允许出现在水下沉底
/// （grassOrDirtIfUnderwater 的 else 分支）与树基换土（TreeConfiguration
/// .java:20-26）两类合法位置。估计面样条与密度场真实过零面的 ±1-2 格偏差
/// （base_3d/jag）不得把暴露层打进 DIRT 带。
#[test]
fn grass_biome_window_top_is_grass() {
    let orch = Orchestrator::new(SEED);
    for want in [Biome::Plains, Biome::Forest] {
        let Some((cx, cz)) = find_biome_window(&orch, want) else {
            panic!("±96 区块内扫不到全 {want:?} 陆地窗（群系分布退化？）");
        };
        // 窗口前置断言（trees_wiring 同约定）。
        for dz in 0..16_i32 {
            for dx in 0..16_i32 {
                let wx = cx * 16 + dx;
                let wz = cz * 16 + dz;
                assert_eq!(biome_at(SEED, wx, wz), want, "窗内群系漂移");
                let est = surface_block_y(SEED, wx, wz);
                assert!(
                    est > SEA_LEVEL + 1 && est <= SEA_LEVEL + 46,
                    "窗内列 ({wx},{wz}) est={est} 落滩/雪带"
                );
            }
        }
        let t = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(cx, cz))
            .expect("vanilla gen");
        let vox = t.voxels.as_u16_slice();
        let mut dirt_top = 0usize;
        let mut stone_top = 0usize;
        let mut offenders: Vec<String> = Vec::new();
        let mut checked = 0usize;
        for z in 0..16usize {
            for x in 0..16usize {
                if column_has_log(vox, x, z) {
                    continue; // 树基列：原版基下换土合法
                }
                let Some((gy, g)) = ground_block(vox, x, z) else {
                    continue;
                };
                if gy < SEA_LEVEL {
                    continue; // 水下沉底：grassOrDirtIfUnderwater else 分支合法
                }
                if gy <= WORLD_MIN_Y + 5 {
                    continue; // 基岩带哨兵列不查材质（geometry_locks 同口径）
                }
                checked += 1;
                let ok = g == GRASS || g == SNOW_GRASS;
                if g == DIRT {
                    dirt_top += 1;
                } else if g == STONE {
                    stone_top += 1;
                }
                if !ok && offenders.len() < 8 {
                    offenders.push(format!("({x},{z}) y={gy} id={g}"));
                }
            }
        }
        assert!(checked > 128, "窗内可查列过少（{checked}）——窗口退化");
        assert!(
            dirt_top == 0 && stone_top == 0,
            "{want:?} 窗 ({cx},{cz})：泥土顶列 dirt_top={dirt_top} 石头顶列 \
             stone_top={stone_top} / 已查 {checked} 列（示例 {:?}）——表层锚在样条 \
             估计面而非密度场真实过零面（26.1 SurfaceSystem.buildSurface 逐列 \
             实顶语义，见文件头取证）",
            offenders
        );
    }
}

/// 草群系窗亚表层：顶格下方首个实心格 ∈ {DIRT}（UNDER_FLOOR 带落
/// biomeUnderSurfaceRule 默认 DIRT，SurfaceRuleData.java:124；悬空浮岛
/// 底/带外石按构造跳过）。
#[test]
fn grass_biome_window_subsoil_is_dirt() {
    let orch = Orchestrator::new(SEED);
    for want in [Biome::Plains, Biome::Forest] {
        let Some((cx, cz)) = find_biome_window(&orch, want) else {
            panic!("±96 区块内扫不到全 {want:?} 陆地窗");
        };
        let t = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(cx, cz))
            .expect("vanilla gen");
        let vox = t.voxels.as_u16_slice();
        let mut bad_subsoil = 0usize;
        for z in 0..16usize {
            for x in 0..16usize {
                if column_has_log(vox, x, z) {
                    continue;
                }
                let Some((gy, g)) = ground_block(vox, x, z) else {
                    continue;
                };
                if gy < SEA_LEVEL || gy <= WORLD_MIN_Y + 5 || g != GRASS {
                    continue;
                }
                let below = vox[vidx(x, gy - 1, z)];
                if below == AIR || below == WATER {
                    continue; // 悬浮顶（浮岛/檐顶）：无亚表语义
                }
                if below != DIRT {
                    bad_subsoil += 1;
                }
            }
        }
        assert!(
            bad_subsoil == 0,
            "{want:?} 窗 ({cx},{cz})：亚表非 DIRT 列 {bad_subsoil}（UNDER_FLOOR 带 \
             SurfaceRuleData.java:124 默认 DIRT）"
        );
    }
}

/// 滩窗（Beach）：水上顶格 = SAND（SurfaceRuleData.java:74,86
/// biomesWithSandAndSandstone → sandOrSandstoneIfCeiling）；水下沉底
/// 沙/土均合法（原版水底可泥土，不锁死）；亚表（顶下首实心）= SAND
/// （UNDER_FLOOR → 同 sandOrSandstoneIfCeiling）。
#[test]
fn beach_window_top_is_sand() {
    let orch = Orchestrator::new(SEED);
    let Some((cx, cz)) = find_biome_window(&orch, Biome::Beach) else {
        panic!("±96 区块内扫不到全 Beach 陆地窗");
    };
    let t = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(cx, cz))
        .expect("vanilla gen");
    let vox = t.voxels.as_u16_slice();
    let mut land = 0usize;
    let mut bad_land_top = 0usize;
    let mut bad_subsoil = 0usize;
    for z in 0..16usize {
        for x in 0..16usize {
            let Some((gy, g)) = ground_block(vox, x, z) else {
                continue;
            };
            if gy <= WORLD_MIN_Y + 5 {
                continue;
            }
            if gy >= SEA_LEVEL {
                land += 1;
                if g != SAND {
                    bad_land_top += 1;
                }
            } else {
                assert!(
                    g == SAND || g == DIRT || g == BEDROCK,
                    "Beach 窗水下沉底 {g} ∉ {{SAND,DIRT,BEDROCK}}（y={gy}）"
                );
            }
            let below = vox[vidx(x, gy - 1, z)];
            if gy - 1 > WORLD_MIN_Y + 5 && below != AIR && below != WATER && below != SAND {
                bad_subsoil += 1;
            }
        }
    }
    assert!(land > 0, "Beach 窗无水上列——窗落位退化");
    assert!(
        bad_land_top == 0 && bad_subsoil == 0,
        "Beach 窗 ({cx},{cz})：水上顶非 SAND {bad_land_top} 列、亚表非 SAND \
         {bad_subsoil} 列（SurfaceRuleData.java:74,86）"
    );
}

/// 海窗（Ocean）：水下沉底顶格 ∈ {SAND, DIRT}（原版海洋底为砾石
/// SurfaceRuleData.java:258-265 gravelOrStoneIfCeiling——本仓无砾石 id，
/// 沙代之并登记 KNOWN-DIVERGENCE SURF-2；此处只防草/石外溢，不锁死沙/土）。
/// 窗条件与草/滩窗不同：深海要求全列 est ≤ SEA−8（Ocean 群系在海岸带
/// 可为干地——密度场说了算，不属本锁面）。
#[test]
fn ocean_window_underwater_top_is_sand_or_dirt() {
    let orch = Orchestrator::new(SEED);
    let mut win = None;
    'outer: for cz in -96..96 {
        for cx in -96..96 {
            let mut all_ocean = true;
            for dz in (0..16_i32).step_by(5) {
                for dx in (0..16_i32).step_by(5) {
                    let wx = cx * 16 + dx;
                    let wz = cz * 16 + dz;
                    if orch.biome_at(wx, wz) != Biome::Ocean
                        || orch.column_state(wx, wz).surface_ours > SEA_LEVEL - 8
                    {
                        all_ocean = false;
                        break;
                    }
                }
                if !all_ocean {
                    break;
                }
            }
            if all_ocean {
                win = Some((cx, cz));
                break 'outer;
            }
        }
    }
    let Some((cx, cz)) = win else {
        panic!("±192 区块内扫不到深海 Ocean 窗（海陆比退化？）");
    };
    let t = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(cx, cz))
        .expect("vanilla gen");
    let vox = t.voxels.as_u16_slice();
    let mut checked = 0usize;
    for z in 0..16usize {
        for x in 0..16usize {
            let Some((gy, g)) = ground_block(vox, x, z) else {
                continue;
            };
            if gy > SEA_LEVEL || gy <= WORLD_MIN_Y + 5 {
                continue; // 只锁水下沉底
            }
            checked += 1;
            assert!(
                g == SAND || g == DIRT || g == BEDROCK,
                "Ocean 窗 ({cx},{cz}) 列 ({x},{z}) y={gy} 海床 {g} ∉ {{SAND,DIRT,BEDROCK}}"
            );
        }
    }
    assert!(checked > 128, "海窗可查列过少（{checked}）");
}

/// 确定性：同 seed 两 generate 字节一致（含新表层路径；quality.rs 全局
/// 确定性门的 surface_rules 侧镜像，独立文件可单独跑）。
#[test]
fn surface_rules_deterministic() {
    let a =
        generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(5, 9)).expect("gen a");
    let b =
        generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(5, 9)).expect("gen b");
    assert_eq!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
    assert_eq!(a.heightmap[..], b.heightmap[..]);
}
