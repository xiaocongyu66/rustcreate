//! Synthetic-voxel tests for the BFS lighting engine.

use mcv_core::{vidx, BlockId, CHUNK_VOL};
use mcv_light::{
    apply_edge, extract_edge, init, opacity, propagate, update_block, BorderSeed, LightChunk,
};

const AIR: u8 = 0;
const STONE: u8 = 1;

struct World {
    voxels: Vec<u8>,
    light: Vec<u8>,
    hm: Vec<u8>,
}

impl World {
    fn new() -> Self {
        Self {
            voxels: vec![AIR; CHUNK_VOL],
            light: vec![0; CHUNK_VOL],
            hm: vec![0; 256],
        }
    }

    /// Flat ground: solid stone up to and including `top`.
    fn flat(top: usize) -> Self {
        let mut w = Self::new();
        w.box_fill(0, 15, 0, top, 0, 15, STONE);
        w.rebuild_heightmap();
        w
    }

    fn box_fill(
        &mut self,
        x0: usize,
        x1: usize,
        y0: usize,
        y1: usize,
        z0: usize,
        z1: usize,
        id: u8,
    ) {
        for y in y0..=y1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    self.voxels[vidx(x, y, z)] = id;
                }
            }
        }
    }

    /// Heightmap = highest light-blocking block y + 1 (0 = open column).
    fn rebuild_heightmap(&mut self) {
        for z in 0..16usize {
            for x in 0..16usize {
                let mut h = 0usize;
                for y in (0..256).rev() {
                    if opacity(BlockId(self.voxels[vidx(x, y, z)])) > 0 {
                        h = y + 1;
                        break;
                    }
                }
                self.hm[(z << 4) | x] = h as u8;
            }
        }
    }

    fn chunk(&mut self) -> LightChunk<'_> {
        LightChunk {
            voxels: &self.voxels,
            light: &mut self.light,
            heightmap: &self.hm,
        }
    }
}

fn sky(c: &LightChunk, x: usize, y: usize, z: usize) -> u8 {
    c.light[vidx(x, y, z)] >> 4
}

fn blk(c: &LightChunk, x: usize, y: usize, z: usize) -> u8 {
    c.light[vidx(x, y, z)] & 0xF
}

fn sky_w(w: &World, x: usize, y: usize, z: usize) -> u8 {
    w.light[vidx(x, y, z)] >> 4
}

/// Place/dig through `update_block`, writing the voxel first (the engine's
/// voxel view is read-only; it assumes the edit is already applied).
fn edit(
    w: &mut World,
    x: usize,
    y: usize,
    z: usize,
    old_block: u8,
    new_block: u8,
) -> (u8, Vec<BorderSeed>) {
    w.voxels[vidx(x, y, z)] = new_block;
    let mut seeds = Vec::new();
    let mask = {
        let mut c = w.chunk();
        update_block(
            &mut c, x as u32, y as u32, z as u32, old_block, new_block, &mut seeds,
        )
    };
    (mask, seeds)
}

#[test]
fn flat_ground_direct_sky() {
    let mut w = World::flat(40);
    let mut c = w.chunk();
    let mask = init(&mut c);
    // Every open column is direct sunlight all the way down to the surface.
    for z in 0..16usize {
        for x in 0..16usize {
            for y in 41..256 {
                assert_eq!(sky(&c, x, y, z), 15, "sky at ({x},{y},{z})");
                assert_eq!(blk(&c, x, y, z), 0);
            }
            // Surface block itself and everything below stay dark.
            assert_eq!(sky(&c, x, 40, z), 0);
            assert_eq!(sky(&c, x, 0, z), 0);
        }
    }
    // All four borders carry light.
    assert_eq!(mask, 0b1111);
}

#[test]
fn sealed_room_block_light() {
    let mut w = World::flat(40);
    // Carve a sealed room out of the solid ground: interior air with a
    // one-block stone shell (interior x/z 3..=12, y 32..=38).
    w.box_fill(3, 12, 32, 38, 3, 12, AIR);
    w.rebuild_heightmap();
    let mut c = w.chunk();
    init(&mut c);
    // Sealed interior: no sky, no block light.
    assert_eq!(sky(&c, 5, 35, 5), 0);
    assert_eq!(blk(&c, 5, 35, 5), 0);

    // Hand-seed a block-light source at level 14 and converge.
    c.light[vidx(5, 35, 5)] = 14;
    let mask = propagate(&mut c);
    assert_eq!(mask, 0);
    for d in 0..=7usize {
        let expect = (14 - d) as u8;
        assert_eq!(blk(&c, 5 + d, 35, 5), expect, "x+{d}");
        assert_eq!(blk(&c, 5, 35, 5 + d), expect, "z+{d}");
    }
    // Interior height limits the vertical falloff to 3 cells.
    for d in 1..=3usize {
        let expect = (14 - d) as u8;
        assert_eq!(blk(&c, 5, 35 - d, 5), expect, "y-{d}");
        assert_eq!(blk(&c, 5, 35 + d, 5), expect, "y+{d}");
    }
    // Walls swallow the light: shell and the world beyond stay 0.
    assert_eq!(blk(&c, 13, 35, 5), 0);
    assert_eq!(blk(&c, 2, 35, 5), 0);
    assert_eq!(blk(&c, 5, 31, 5), 0);
    assert_eq!(blk(&c, 5, 39, 5), 0);
    // Sky nibble untouched by the block-light BFS.
    assert_eq!(sky(&c, 5, 35, 5), 0);
}

#[test]
fn place_and_dig_casts_shadow() {
    let mut w = World::flat(40);
    {
        let mut c = w.chunk();
        init(&mut c);
    }

    // Float an opaque block in the open sky at (8, 45, 8).
    let (mask, seeds) = edit(&mut w, 8, 45, 8, AIR, STONE);
    assert_eq!(mask, 0);
    assert!(seeds.is_empty());
    assert_eq!(sky_w(&w, 8, 45, 8), 0);
    // Direct column below is cut: shadow filled at 14 from the sides.
    for y in 41..45 {
        assert_eq!(sky_w(&w, 8, y, 8), 14, "shadow at y={y}");
    }
    // Neighbour columns keep full sunlight.
    assert_eq!(sky_w(&w, 7, 44, 8), 15);
    assert_eq!(sky_w(&w, 9, 44, 8), 15);
    assert_eq!(sky_w(&w, 8, 46, 8), 15);

    // Dig it back out: direct column is restored.
    let (mask, seeds) = edit(&mut w, 8, 45, 8, STONE, AIR);
    assert_eq!(mask, 0);
    assert!(seeds.is_empty());
    for y in 41..=45 {
        assert_eq!(sky_w(&w, 8, y, 8), 15, "restored at y={y}");
    }
}

#[test]
fn edge_add_feeds_neighbour() {
    let mut a = World::flat(40);
    let mut b = World::flat(40);
    let edge = {
        let mut ca = a.chunk();
        init(&mut ca);
        extract_edge(&ca, 0)
    };
    let mut cb = b.chunk();
    init(&mut cb);
    // Simulate an unlit B: wipe its sky channel through the live view.
    cb.light.iter_mut().for_each(|l| *l &= 0x0F);

    // A's +X edge feeds B's -X edge.
    for y in 41..256 {
        assert_eq!(edge[((y << 4) + 3) as usize], 0xF0);
    }
    let mask = apply_edge(&mut cb, &edge, 1, 0); // side 1 = -X, ADD
    assert_eq!(sky(&cb, 0, 50, 0), 14);
    assert_eq!(sky(&cb, 1, 50, 0), 13);
    assert_eq!(sky(&cb, 3, 50, 0), 11);
    assert_eq!(sky(&cb, 0, 41, 7), 14);
    // Opaque ground stays dark; far side of B untouched.
    assert_eq!(sky(&cb, 0, 40, 0), 0);
    assert_eq!(sky(&cb, 15, 50, 8), 0);
    // The synced side is never reported back; the z-borders are.
    assert_eq!(mask & 0b0011, 0);
    assert_eq!(mask & 0b1100, 0b1100);
}

#[test]
fn edge_remove_retracts_light() {
    let mut a = World::flat(40);
    let mut b = World::flat(40);
    let edge = {
        let mut ca = a.chunk();
        init(&mut ca);
        extract_edge(&ca, 0)
    };
    let mut cb = b.chunk();
    init(&mut cb);
    cb.light.iter_mut().for_each(|l| *l &= 0x0F);

    apply_edge(&mut cb, &edge, 1, 0);
    assert_eq!(sky(&cb, 0, 50, 0), 14);

    // The neighbour's light went away: retract the whole gradient.
    let dark = [0u8; 4096];
    apply_edge(&mut cb, &dark, 1, 1); // REMOVE
    assert_eq!(sky(&cb, 0, 50, 0), 0);
    assert_eq!(sky(&cb, 1, 50, 0), 0);
    assert_eq!(sky(&cb, 5, 50, 0), 0);
    assert_eq!(sky(&cb, 0, 42, 7), 0);
    assert_eq!(sky(&cb, 13, 50, 8), 0);
}

#[test]
fn tunnel_light_falloff() {
    let mut w = World::flat(39);
    // Mountain slab over x/z 2..=15, y 40..=60; columns below stay open.
    w.box_fill(2, 15, 40, 60, 2, 15, STONE);
    // East-west tunnel at y=50, z=8, mouth at x=2 (x=1 is open air).
    w.box_fill(2, 12, 50, 50, 8, 8, AIR);
    w.rebuild_heightmap();
    let mut c = w.chunk();
    init(&mut c);

    // The mouth cell is lit by the open column beside it, then light
    // decays by exactly 1 per tunnel cell.
    assert_eq!(sky(&c, 1, 50, 8), 15);
    for d in 0..=10usize {
        let expect = (14 - d) as u8;
        assert_eq!(sky(&c, 2 + d, 50, 8), expect, "tunnel depth {d}");
    }
    // Tunnel walls stay dark.
    assert_eq!(sky(&c, 6, 49, 8), 0);
    assert_eq!(sky(&c, 6, 51, 8), 0);
    assert_eq!(sky(&c, 6, 50, 7), 0);
}
