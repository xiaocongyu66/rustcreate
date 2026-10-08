//! Terrain kernel behavior tests against the C++ implementation.

use mcv_core::{BlockId, CHUNK_VOL, ChunkPos};
use mcv_worldgen::generate_terrain;

const AIR: u16 = 0;
const STONE: u16 = 1;
const DIRT: u16 = 2;
const GRASS: u16 = 3;
const SAND: u16 = 4;
const WATER: u16 = 5;
const LOG: u16 = 6;
const LEAVES: u16 = 7;
const BEDROCK: u16 = 10;
const SNOW_GRASS: u16 = 11;
const FLOWER_RED: u16 = 12;
const FLOWER_YELLOW: u16 = 13;

fn vidx(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

fn voxels_of(t: &mcv_worldgen::TerrainOutput) -> &[u16] {
    t.voxels.as_u16_slice()
}

#[test]
fn deterministic_same_seed() {
    let a = generate_terrain(42, ChunkPos::new(3, -7)).expect("gen");
    let b = generate_terrain(42, ChunkPos::new(3, -7)).expect("gen");
    assert_eq!(voxels_of(&a), voxels_of(&b));
    assert_eq!(a.heightmap, b.heightmap);
}

#[test]
fn different_seed_differs() {
    let a = generate_terrain(1, ChunkPos::new(0, 0)).expect("gen");
    let b = generate_terrain(2, ChunkPos::new(0, 0)).expect("gen");
    assert_ne!(voxels_of(&a), voxels_of(&b));
}

#[test]
fn heightmap_matches_topmost_blocking() {
    let t = generate_terrain(42, ChunkPos::new(0, 0)).expect("gen");
    let vox = voxels_of(&t);
    for z in 0..16usize {
        for x in 0..16usize {
            let mut top = 0usize;
            for y in (0..256).rev() {
                let id = vox[vidx(x, y, z)];
                if id != AIR && id != WATER && id != FLOWER_RED && id != FLOWER_YELLOW {
                    top = y;
                    break;
                }
            }
            assert_eq!(t.heightmap[(z << 4) | x] as usize, top + 1);
        }
    }
}

#[test]
fn bedrock_floor_and_sea_water() {
    // Sample a spread of chunks; at least one must contain water (ocean) and
    // every chunk must have a bedrock floor.
    // 范围 ±6：新常数的大陆度波长被可步行尺度封顶在 256 格（MC 2048 的 ÷8
    // 压缩，见 NOTES-terrain.md §7），~200 格窗口只覆盖 0.8 个波长，放宽到
    // ±6 保证窗口内出现洋盆。
    let mut saw_water = false;
    for cx in -6..=6 {
        for cz in -6..=6 {
            let t = generate_terrain(7, ChunkPos::new(cx, cz)).expect("gen");
            let vox = voxels_of(&t);
            assert_eq!(vox[vidx(0, 0, 0)], BEDROCK);
            assert_eq!(vox[vidx(15, 0, 15)], BEDROCK);
            if vox.contains(&WATER) {
                saw_water = true;
            }
        }
    }
    assert!(saw_water, "expected ocean water somewhere in range");
}

#[test]
fn cave_rate_in_band() {
    // Statistical: cave air below the surface should sit in a 1..25% band
    // over a batch of chunks (deterministic across platforms).
    // 带宽对应 MC 26.1 意面雕刻带 |n|∈0.065..0.088 + cheese 阈值 0.60/0.66
    // （NoiseRouterData，NOTES-terrain.md §3）在新常数下的合理包络。
    let mut solid = 0usize;
    let mut cave = 0usize;
    for cx in -2..=2 {
        for cz in -2..=2 {
            let t = generate_terrain(99, ChunkPos::new(cx, cz)).expect("gen");
            let vox = voxels_of(&t);
            for z in 0..16usize {
                for x in 0..16usize {
                    let surface = t.heightmap[(z << 4) | x] as usize - 1;
                    for y in 8..surface.saturating_sub(4) {
                        match vox[vidx(x, y, z)] {
                            AIR => cave += 1,
                            STONE | DIRT => solid += 1,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    let rate = cave as f64 / (cave + solid) as f64;
    assert!(
        (0.01..=0.25).contains(&rate),
        "cave rate {rate:.3} outside 1-25%"
    );
}

/// 地面表层块：从柱顶向下跳过空气/水/树/花后的第一个实心块。
fn ground_surface(vox: &[u16], x: usize, z: usize) -> (u16, usize) {
    for y in (0..256usize).rev() {
        let id = vox[vidx(x, y, z)];
        if id != AIR
            && id != WATER
            && id != LOG
            && id != LEAVES
            && id != FLOWER_RED
            && id != FLOWER_YELLOW
        {
            return (id, y);
        }
    }
    (AIR, 0)
}

#[test]
fn height_distribution_spans_band() {
    // 新常数（NOTES-terrain.md §7：cont±26 + 山脊×38 + 细节±3，海平面 96）
    // 应给出跨盆带的柱高：深海盆底 ~SEA−22 到山脊 ~SEA+40+。
    // 窗口 ±8（256 格 = 大陆度一个完整波长，可步行尺度上限），保证窗口
    // 内至少经历一次盆带起伏；跨度 ≥24 可捕获“通道波长退化→全常数”回归
    // （全常数时跨度只剩细节 ±3 ≈ 6）。
    let mut min = usize::MAX;
    let mut max = 0usize;
    for cx in -8..=8 {
        for cz in -8..=8 {
            let t = generate_terrain(3, ChunkPos::new(cx, cz)).expect("gen");
            let vox = voxels_of(&t);
            for z in 0..16usize {
                for x in 0..16usize {
                    let (_, y) = ground_surface(vox, x, z);
                    min = min.min(y);
                    max = max.max(y);
                }
            }
        }
    }
    assert!(max - min >= 24, "height span {}-{} too flat", min, max);
    assert!(min > 10, "lowest column y={min} hits the bedrock band");
    assert!(max < 220, "highest column y={max} beyond ridge band");
}

#[test]
fn surface_dominated_by_grass_and_stone() {
    // 地表家族：草（含雪草）为主，海洋盆底为沙，洞穴穿破处露石/土。
    // MC SurfaceRuleData.overworldLike：ON_FLOOR+无水 → grass，海床沙、
    // 石头仅在洞穿处出现（NOTES-terrain.md §5）。
    // 多 seed 合并采样：单个 ±3 窗口可能整体落在洋盆（大陆度波长 256 ≈
    // 窗口尺寸），三个独立世界合并后该风险可忽略。
    let mut total = 0usize;
    let mut grassy = 0usize; // GRASS + SNOW_GRASS
    let mut valid = 0usize; // GRASS/SNOW_GRASS/SAND/STONE/DIRT
    for seed in [3u64, 7, 11] {
        for cx in -3..=3 {
            for cz in -3..=3 {
                let t = generate_terrain(seed, ChunkPos::new(cx, cz)).expect("gen");
                let vox = voxels_of(&t);
                for z in 0..16usize {
                    for x in 0..16usize {
                        let (id, _) = ground_surface(vox, x, z);
                        total += 1;
                        if id == GRASS || id == SNOW_GRASS {
                            grassy += 1;
                        }
                        if id == GRASS
                            || id == SNOW_GRASS
                            || id == SAND
                            || id == STONE
                            || id == DIRT
                        {
                            valid += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(
        grassy * 100 >= total * 35,
        "grassy share too low: {grassy}/{total}"
    );
    assert!(
        valid * 100 >= total * 95,
        "unexpected surface blocks: {valid}/{total}"
    );
}

#[test]
fn trees_and_grass_present() {
    // Over a wide sample there must be grass columns, logs and leaves.
    let mut grass = 0;
    let mut logs = 0;
    let mut leaves = 0;
    for cx in -6..=6 {
        for cz in -6..=6 {
            let t = generate_terrain(5, ChunkPos::new(cx, cz)).expect("gen");
            let vox = voxels_of(&t);
            grass += vox.iter().filter(|&&b| b == GRASS).count();
            logs += vox.iter().filter(|&&b| b == LOG).count();
            leaves += vox.iter().filter(|&&b| b == LEAVES).count();
        }
    }
    assert!(grass > 1000, "grass {grass}");
    assert!(logs > 0, "no trees generated");
    assert!(leaves > logs * 4, "canopy too sparse: leaves {leaves}");
}

#[test]
fn forest_density_shapes_tree_count() {
    // Trees appear in a wide area; count must be moderate (not barren, not
    // solid forest).
    let mut logs = 0;
    for cx in -10..=10 {
        for cz in -10..=10 {
            let t = generate_terrain(11, ChunkPos::new(cx, cz)).expect("gen");
            let vox = voxels_of(&t);
            logs += vox.iter().filter(|&&b| b == LOG).count();
        }
    }
    assert!(
        (50..5000).contains(&logs),
        "tree count {logs} outside expectations"
    );
}

#[test]
fn beach_sand_below_sea_level() {
    // Some chunk near sea level must expose SAND at its surface layer.
    let mut found = false;
    'outer: for cx in -8..=8 {
        for cz in -8..=8 {
            let t = generate_terrain(7, ChunkPos::new(cx, cz)).expect("gen");
            let vox = voxels_of(&t);
            for z in 0..16usize {
                for x in 0..16usize {
                    let hm = t.heightmap[(z << 4) | x] as usize - 1;
                    if hm > 2 && vox[vidx(x, hm, z)] == SAND {
                        found = true;
                        break 'outer;
                    }
                }
            }
        }
    }
    assert!(found, "no sand beach found near sea level");
}

#[test]
fn ffi_registry_ids_match() {
    // The C++ terrain kernel hardcodes ids; keep the registry in sync.
    assert_eq!(BlockId(1).def().name, "stone");
    assert_eq!(BlockId(3).def().name, "grass");
    assert_eq!(BlockId(10).def().name, "bedrock");
    assert_eq!(CHUNK_VOL, 65536);
}
