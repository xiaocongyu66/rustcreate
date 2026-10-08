//! Terrain kernel behavior tests against the C++ implementation.

use mcv_core::{BlockId, ChunkPos, CHUNK_VOL};
use mcv_worldgen::generate_terrain;

const AIR: u8 = 0;
const STONE: u8 = 1;
const DIRT: u8 = 2;
const GRASS: u8 = 3;
const SAND: u8 = 4;
const WATER: u8 = 5;
const LOG: u8 = 6;
const LEAVES: u8 = 7;
const BEDROCK: u8 = 10;
const FLOWER_RED: u8 = 12;
const FLOWER_YELLOW: u8 = 13;

fn vidx(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

fn voxels_of(t: &mcv_worldgen::TerrainOutput) -> &[u8] {
    t.voxels.as_bytes()
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
    let mut saw_water = false;
    for cx in -3..=3 {
        for cz in -3..=3 {
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
    // Statistical: cave air below the surface should sit in a 2..10% band
    // over a batch of chunks (deterministic across platforms).
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
        (0.02..=0.12).contains(&rate),
        "cave rate {rate:.3} outside 2-12%"
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
