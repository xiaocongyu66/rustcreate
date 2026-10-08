//! Pure-Rust lighting engine: dual-channel (sky/block) BFS with removal
//! propagation and cross-chunk border sync. Lands in M4.
//!
//! Layout contract (matches `mcv_core`):
//! - light: 65536 bytes, index `(y<<8)|(z<<4)|x`, low nibble = block light,
//!   high nibble = sky light (0..=15 each).
//! - heightmap: 256 bytes, index `(z<<4)|x`, value = highest light-blocking
//!   block y + 1 (0 = open column, direct sky all the way down).
//!
//! BFS semantics follow vanilla `LightEngine.propagateIncreases` /
//! `propagateDecreases`: plain FIFO queues with a head cursor, stale entries
//! skipped by re-reading the stored level, and removal waves feeding
//! re-light sources into the increase queue.

use mcv_core::{vidx, BlockId, CHUNK_VOL};

/// Channel selector for the packed light byte.
const SKY_SHIFT: u32 = 4;
const BLK_SHIFT: u32 = 0;

/// Border sides. `bit0..3` of every dirty mask = `+X, -X, +Z, -Z`.
pub const SIDE_PLUS_X: u8 = 0;
pub const SIDE_MINUS_X: u8 = 1;
pub const SIDE_PLUS_Z: u8 = 2;
pub const SIDE_MINUS_Z: u8 = 3;

/// Borrowed views over one chunk's voxel, light and heightmap storage.
pub struct LightChunk<'a> {
    pub voxels: &'a [u8],
    pub light: &'a mut [u8],
    pub heightmap: &'a [u8],
}

/// One boundary-cell change to hand to the neighbouring chunk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BorderSeed {
    /// 0=+X 1=-X 2=+Z 3=-Z.
    pub side: u8,
    /// Horizontal coordinate along the edge: `z` for ±X, `x` for ±Z.
    pub xz: u8,
    pub y: u16,
    pub sky: u8,
    pub blk: u8,
}

/// Per-block light dampening, calibrated against decompiled Minecraft
/// 26.1 `BlockBehaviour.getLightDampening` (consumed by
/// `LightEngine.getOpacity` as `max(1, dampening)`):
/// - solid-render full cubes = 15 (stone, dirt, log, …);
/// - water = 1 (`LiquidBlock.propagatesSkylightDown == false` → damp 1;
///   the pre-1.20.5 "water costs 3" rule is *not* 26.1 behaviour);
/// - leaves = 1 (`LeavesBlock.getLightDampening` override);
/// - flowers / air = 0 (`CrossCollisionBlock.propagatesSkylightDown` →
///   damp 0; glass would land here too once registered);
/// - unknown ids are treated as fully opaque (15).
/// `mcv_core::BLOCKS` has no opacity column and must not gain one, so
/// this table is the single source of truth.
#[inline]
pub const fn opacity(id: BlockId) -> u8 {
    match id.0 {
        0 => 0,       // air
        5 => 1,       // water
        7 => 1,       // leaves
        12 | 13 => 0, // flowers
        _ => 15,      // opaque solids (and unknown ids)
    }
}

/// Cached `light_emit` mirror of `mcv_core::BLOCKS` so seeding can index raw
/// voxel bytes without bounds concerns.
static BLOCKS_EMIT: [u8; 14] = {
    let mut t = [0u8; 14];
    let mut i = 0;
    while i < 14 {
        t[i] = mcv_core::BLOCKS[i].light_emit;
        i += 1;
    }
    t
};

#[inline]
fn light_emit(id: BlockId) -> u8 {
    match BLOCKS_EMIT.get(id.0 as usize) {
        Some(v) => *v,
        None => 0,
    }
}

#[inline]
fn get_ch(light: &[u8], idx: usize, shift: u32) -> u8 {
    (light[idx] >> shift) & 0xF
}

#[inline]
fn set_ch(light: &mut [u8], idx: usize, shift: u32, v: u8) {
    let keep = if shift == SKY_SHIFT { 0x0F } else { 0xF0 };
    light[idx] = (light[idx] & keep) | (v << shift);
}

#[inline]
fn clamp_hm(h: usize) -> usize {
    if h > 256 {
        256
    } else {
        h
    }
}

/// Neighbour offsets: (dx, dy, dz).
const NEIGHBORS: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

/// Decompose a voxel index into `(x, y, z)`.
#[inline]
fn coords(idx: u16) -> (usize, usize, usize) {
    (
        (idx & 0xF) as usize,
        (idx >> 8) as usize,
        ((idx >> 4) & 0xF) as usize,
    )
}

#[inline]
fn out_of_bounds(x: i32, y: i32, z: i32) -> bool {
    !(0..=15).contains(&x) || !(0..=255).contains(&y) || !(0..=15).contains(&z)
}

/// Vanilla spread rule (26.1 `LightEngine.propagateIncrease`): every step
/// into a cell costs `max(1, opacity)` — in *both* engines and in *all*
/// six directions; `BlockLightEngine` (torch 14 → 13 → … vertically too)
/// has no exceptions at all.
///
/// The single carve-out here — sky 15 flowing straight down through a
/// cell whose `opacity == 0`, staying at 15 — mirrors the vanilla sky
/// *source column* (`ChunkSkyLightSources.lowestSourceY` +
/// `SkyLightEngine.addSourcesAbove` write 15 down every damp==0 column
/// until the first damp!=0 block truncates it). In this engine a stored
/// 15 only ever comes from the heightmap seeds (`init`) or the dug-column
/// writes (`update_block`), i.e. from exactly those source columns, so
/// the carve-out reproduces the reference: open-air shafts stay 15 all
/// the way down, while water/leaves (damp 1) truncate the column and
/// attenuate 15 → 14 → 13 … vertically as well (there `dec = opacity =
/// 1`), and opaque blocks (15) clamp straight to 0.
#[inline]
fn spread_target(shift: u32, level: u8, down: bool, nopacity: u8) -> u8 {
    let direct_down = shift == SKY_SHIFT && down && level == 15;
    let dec = if direct_down {
        nopacity
    } else {
        nopacity.max(1)
    };
    level.saturating_sub(dec)
}

/// Increase-only BFS over one channel. The queue holds `(idx, level)` pairs
/// written just before enqueueing; stale entries (storage moved on) are
/// skipped by re-reading the stored level, like the vanilla queue.
fn propagate_channel(voxels: &[u8], light: &mut [u8], shift: u32, queue: &mut Vec<(u16, u8)>) {
    let mut head = 0usize;
    while head < queue.len() {
        let (idx, level) = queue[head];
        head += 1;
        let cur = get_ch(light, idx as usize, shift);
        if cur == 0 || cur > level {
            continue; // dark, or superseded by a brighter write
        }
        let level = cur;
        let (x, y, z) = coords(idx);
        for (dx, dy, dz) in NEIGHBORS {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            let nz = z as i32 + dz;
            if out_of_bounds(nx, ny, nz) {
                continue;
            }
            let nidx = vidx(nx as usize, ny as usize, nz as usize);
            let target = spread_target(shift, level, dy < 0, opacity(BlockId(voxels[nidx])));
            if target == 0 {
                continue;
            }
            if get_ch(light, nidx, shift) < target {
                set_ch(light, nidx, shift, target);
                queue.push((nidx as u16, target));
            }
        }
    }
}

/// Removal BFS over one channel. Queue entries must have their storage
/// already zeroed before enqueue; neighbours brighter than or equal to the
/// removed level are re-light sources and go into `readd`.
fn removal_channel(
    light: &mut [u8],
    shift: u32,
    queue: &mut Vec<(u16, u8)>,
    readd: &mut Vec<(u16, u8)>,
) {
    let mut head = 0usize;
    while head < queue.len() {
        let (idx, old) = queue[head];
        head += 1;
        let (x, y, z) = coords(idx);
        for (dx, dy, dz) in NEIGHBORS {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            let nz = z as i32 + dz;
            if out_of_bounds(nx, ny, nz) {
                continue;
            }
            let nidx = vidx(nx as usize, ny as usize, nz as usize);
            let ncur = get_ch(light, nidx, shift);
            if ncur == 0 {
                continue;
            }
            if ncur < old {
                set_ch(light, nidx, shift, 0);
                queue.push((nidx as u16, ncur));
            } else {
                readd.push((nidx as u16, ncur));
            }
        }
    }
}

/// Highest light-blocking y in a column, or -1 for an open column.
/// `subst` overrides one cell (reconstructs the pre-edit column top, because
/// `LightChunk.voxels` is already written with the new block).
fn column_top(voxels: &[u8], x: usize, z: usize, subst: Option<(usize, u8)>) -> i32 {
    for y in (0..256).rev() {
        let id = match subst {
            Some((sy, sid)) if sy == y => sid,
            _ => voxels[vidx(x, y, z)],
        };
        if opacity(BlockId(id)) > 0 {
            return y as i32;
        }
    }
    -1
}

/// Byte snapshot of all four boundary edges; edge `side` is stored at
/// `side * 4096 .. (side + 1) * 4096`, indexed `(y<<4)|h`.
fn snapshot_edges(chunk: &LightChunk) -> Box<[u8; 16384]> {
    let mut out = Box::new([0u8; 16384]);
    for side in 0..4usize {
        for y in 0..256usize {
            for h in 0..16usize {
                let (x, z) = edge_cell(side, h);
                out[side * 4096 + (y << 4) + h] = chunk.light[vidx(x, y, z)];
            }
        }
    }
    out
}

/// `(x, z)` of the boundary cell on `side` at horizontal offset `h`.
#[inline]
fn edge_cell(side: usize, h: usize) -> (usize, usize) {
    match side {
        0 => (15, h),
        1 => (0, h),
        2 => (h, 15),
        _ => (h, 0),
    }
}

/// Diff boundary edges against a snapshot; emits `BorderSeed`s and returns a
/// dirty mask. `exclude` suppresses one side bit (a side just synced from a
/// neighbour must not be reported back, or direct-sky columns ping-pong).
fn diff_edges(
    before: &[u8; 16384],
    chunk: &LightChunk,
    exclude: Option<u8>,
    mut out_seeds: Option<&mut Vec<BorderSeed>>,
) -> u8 {
    let mut mask = 0u8;
    for side in 0..4usize {
        for y in 0..256usize {
            for h in 0..16usize {
                let (x, z) = edge_cell(side, h);
                let now = chunk.light[vidx(x, y, z)];
                if now == before[side * 4096 + (y << 4) + h] {
                    continue;
                }
                mask |= 1 << side;
                if let Some(seeds) = out_seeds.as_deref_mut() {
                    seeds.push(BorderSeed {
                        side: side as u8,
                        xz: h as u8,
                        y: y as u16,
                        sky: now >> 4,
                        blk: now & 0xF,
                    });
                }
            }
        }
    }
    if let Some(side) = exclude {
        mask &= !(1 << side);
    }
    mask
}

/// Cold-init a chunk: zero light, seed direct-sky columns from the
/// heightmap, seed emitters, then run both BFS channels to convergence.
/// Returns the border dirty mask (bits 0..3 = +X, -X, +Z, -Z).
pub fn init(chunk: &mut LightChunk) -> u8 {
    chunk.light.fill(0);
    let mut sky_q: Vec<(u16, u8)> = Vec::new();
    let mut blk_q: Vec<(u16, u8)> = Vec::new();

    // Direct sunlight: every cell at or above the column heightmap is 15.
    for z in 0..16usize {
        for x in 0..16usize {
            let hm = clamp_hm(chunk.heightmap[(z << 4) | x] as usize);
            for y in hm..256usize {
                let idx = vidx(x, y, z);
                set_ch(chunk.light, idx, SKY_SHIFT, 15);
                sky_q.push((idx as u16, 15));
            }
        }
    }
    // Block-light emitters.
    for idx in 0..CHUNK_VOL {
        let emit = light_emit(BlockId(chunk.voxels[idx]));
        if emit > 0 {
            set_ch(chunk.light, idx, BLK_SHIFT, emit);
            blk_q.push((idx as u16, emit));
        }
    }

    propagate_channel(chunk.voxels, chunk.light, SKY_SHIFT, &mut sky_q);
    propagate_channel(chunk.voxels, chunk.light, BLK_SHIFT, &mut blk_q);

    // Every nonzero boundary cell is new information for the neighbours.
    let mut mask = 0u8;
    for y in 0..256usize {
        for h in 0..16usize {
            if chunk.light[vidx(15, y, h)] != 0 {
                mask |= 1 << SIDE_PLUS_X;
            }
            if chunk.light[vidx(0, y, h)] != 0 {
                mask |= 1 << SIDE_MINUS_X;
            }
            if chunk.light[vidx(h, y, 15)] != 0 {
                mask |= 1 << SIDE_PLUS_Z;
            }
            if chunk.light[vidx(h, y, 0)] != 0 {
                mask |= 1 << SIDE_MINUS_Z;
            }
        }
    }
    mask
}

/// Serialise one boundary edge. `side`: 0=+X 1=-X 2=+Z 3=-Z.
/// ±X edges are indexed `(y<<4)|z`, ±Z edges `(y<<4)|x`.
pub fn extract_edge(chunk: &LightChunk, side: u8) -> [u8; 4096] {
    let mut out = [0u8; 4096];
    if side > 3 {
        return out;
    }
    for y in 0..256usize {
        for h in 0..16usize {
            let (x, z) = edge_cell(side as usize, h);
            out[(y << 4) + h] = chunk.light[vidx(x, y, z)];
        }
    }
    out
}

/// Pull border light from a neighbour. `op`: 0 = ADD, 1 = REMOVE.
/// ADD: boundary cells darker than justified by the neighbour are raised to
/// `neighbour - max(1, opacity)` and re-propagated. REMOVE: boundary cells
/// brighter than `neighbour + 1` are retracted with a removal BFS; light the
/// neighbour no longer justifies falls back onto internal re-light sources.
/// Returns a dirty mask over this chunk's *other* three borders.
pub fn apply_edge(chunk: &mut LightChunk, edge: &[u8; 4096], side: u8, op: u8) -> u8 {
    if side > 3 || op > 1 {
        return 0;
    }
    let before = snapshot_edges(chunk);
    let mut sky_rem: Vec<(u16, u8)> = Vec::new();
    let mut blk_rem: Vec<(u16, u8)> = Vec::new();
    let mut sky_add: Vec<(u16, u8)> = Vec::new();
    let mut blk_add: Vec<(u16, u8)> = Vec::new();

    for y in 0..256usize {
        for h in 0..16usize {
            let (x, z) = edge_cell(side as usize, h);
            let sidx = vidx(x, y, z);
            let n = edge[(y << 4) + h];
            let n_sky = n >> 4;
            let n_blk = n & 0xF;
            let dec = opacity(BlockId(chunk.voxels[sidx])).max(1);
            let s_sky = get_ch(chunk.light, sidx, SKY_SHIFT);
            let s_blk = get_ch(chunk.light, sidx, BLK_SHIFT);
            if op == 0 {
                let t_sky = n_sky.saturating_sub(dec);
                let t_blk = n_blk.saturating_sub(dec);
                if s_sky < t_sky {
                    set_ch(chunk.light, sidx, SKY_SHIFT, t_sky);
                    sky_add.push((sidx as u16, t_sky));
                }
                if s_blk < t_blk {
                    set_ch(chunk.light, sidx, BLK_SHIFT, t_blk);
                    blk_add.push((sidx as u16, t_blk));
                }
            } else {
                if s_sky > n_sky + 1 && s_sky > 0 {
                    set_ch(chunk.light, sidx, SKY_SHIFT, 0);
                    sky_rem.push((sidx as u16, s_sky));
                }
                if s_blk > n_blk + 1 && s_blk > 0 {
                    set_ch(chunk.light, sidx, BLK_SHIFT, 0);
                    blk_rem.push((sidx as u16, s_blk));
                }
            }
        }
    }

    let mut sky_readd = Vec::new();
    let mut blk_readd = Vec::new();
    if op == 1 {
        removal_channel(chunk.light, SKY_SHIFT, &mut sky_rem, &mut sky_readd);
        removal_channel(chunk.light, BLK_SHIFT, &mut blk_rem, &mut blk_readd);
    }
    sky_add.extend(sky_readd);
    blk_add.extend(blk_readd);
    propagate_channel(chunk.voxels, chunk.light, SKY_SHIFT, &mut sky_add);
    propagate_channel(chunk.voxels, chunk.light, BLK_SHIFT, &mut blk_add);

    // Never report the synced side back: the neighbour already owns that
    // state and a REMOVE bounce could erode its direct-sky columns.
    diff_edges(&before, chunk, Some(side), None)
}

/// Re-run both increase channels to convergence (full-scan seeding: every
/// currently-lit cell is a candidate source; stale entries cost nothing).
/// Returns the border dirty mask over all four sides.
pub fn propagate(chunk: &mut LightChunk) -> u8 {
    let before = snapshot_edges(chunk);
    let mut sky_q: Vec<(u16, u8)> = Vec::new();
    let mut blk_q: Vec<(u16, u8)> = Vec::new();
    for idx in 0..CHUNK_VOL {
        let l = chunk.light[idx];
        if l & 0xF0 != 0 {
            sky_q.push((idx as u16, l >> 4));
        }
        if l & 0x0F != 0 {
            blk_q.push((idx as u16, l & 0xF));
        }
    }
    propagate_channel(chunk.voxels, chunk.light, SKY_SHIFT, &mut sky_q);
    propagate_channel(chunk.voxels, chunk.light, BLK_SHIFT, &mut blk_q);
    diff_edges(&before, chunk, None, None)
}

/// Apply a single block edit and relight around it. The caller must already
/// have written `new_block` into `chunk.voxels` (the view is read-only here);
/// `old_block` is the id that produced the light currently stored at the
/// cell. Emits `BorderSeed`s for boundary cells whose light changed and
/// returns the border dirty mask (bits 0..3 = +X, -X, +Z, -Z).
pub fn update_block(
    chunk: &mut LightChunk,
    x: u32,
    y: u32,
    z: u32,
    old_block: u8,
    new_block: u8,
    out_seeds: &mut Vec<BorderSeed>,
) -> u8 {
    if x > 15 || y > 255 || z > 15 || old_block == new_block {
        return 0;
    }
    let (x, y, z) = (x as usize, y as usize, z as usize);
    let before = snapshot_edges(chunk);
    let idx = vidx(x, y, z);
    let cur = chunk.light[idx];
    let old_sky = cur >> 4;
    let old_blk = cur & 0xF;

    // Column tops: the edit is already in `voxels`, so the pre-edit top is
    // reconstructed by substituting `old_block` back over the edited cell.
    // This is the heightmap-column fix-up (placing an opaque block raises the
    // direct column, digging lowers it); the heightmap slice is caller-owned.
    let new_top = column_top(chunk.voxels, x, z, None);
    let old_top = column_top(chunk.voxels, x, z, Some((y, old_block)));

    let mut sky_rem: Vec<(u16, u8)> = Vec::new();
    let mut blk_rem: Vec<(u16, u8)> = Vec::new();
    let mut sky_add: Vec<(u16, u8)> = Vec::new();
    let mut blk_add: Vec<(u16, u8)> = Vec::new();

    // 1. Retract the edited cell's old light.
    if old_sky > 0 {
        set_ch(chunk.light, idx, SKY_SHIFT, 0);
        sky_rem.push((idx as u16, old_sky));
    }
    if old_blk > 0 {
        set_ch(chunk.light, idx, BLK_SHIFT, 0);
        blk_rem.push((idx as u16, old_blk));
    }

    // 2. Direct-column fix for the edited (x, z) column.
    if new_top > old_top {
        // Placed a blocker higher up: cells between the old top and the new
        // blocker lost direct sunlight. Zero and retract them.
        for yy in (old_top + 1)..=new_top {
            let i2 = vidx(x, yy as usize, z);
            let s = get_ch(chunk.light, i2, SKY_SHIFT);
            if s > 0 {
                set_ch(chunk.light, i2, SKY_SHIFT, 0);
                sky_rem.push((i2 as u16, s));
            }
        }
    } else if new_top < old_top {
        // Dug the top blocker open: newly exposed cells gain direct sunlight.
        for yy in (new_top + 1)..=old_top {
            let i2 = vidx(x, yy as usize, z);
            set_ch(chunk.light, i2, SKY_SHIFT, 15);
            sky_add.push((i2 as u16, 15));
        }
    }

    // 3. Removal waves; cells keeping their level become re-light sources.
    let mut sky_readd = Vec::new();
    let mut blk_readd = Vec::new();
    removal_channel(chunk.light, SKY_SHIFT, &mut sky_rem, &mut sky_readd);
    removal_channel(chunk.light, BLK_SHIFT, &mut blk_rem, &mut blk_readd);
    sky_add.extend(sky_readd);
    blk_add.extend(blk_readd);

    // 4. Re-add: re-light sources plus every live neighbour of the edit, so
    // light also flows *into* cells dug out of shadow (they had no light to
    // retract).
    for (dx, dy, dz) in NEIGHBORS {
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        let nz = z as i32 + dz;
        if out_of_bounds(nx, ny, nz) {
            continue;
        }
        let nidx = vidx(nx as usize, ny as usize, nz as usize);
        let s = get_ch(chunk.light, nidx, SKY_SHIFT);
        if s > 0 {
            sky_add.push((nidx as u16, s));
        }
        let b = get_ch(chunk.light, nidx, BLK_SHIFT);
        if b > 0 {
            blk_add.push((nidx as u16, b));
        }
    }
    let emit = light_emit(BlockId(new_block));
    if emit > 0 {
        set_ch(chunk.light, idx, BLK_SHIFT, emit);
        blk_add.push((idx as u16, emit));
    }

    propagate_channel(chunk.voxels, chunk.light, SKY_SHIFT, &mut sky_add);
    propagate_channel(chunk.voxels, chunk.light, BLK_SHIFT, &mut blk_add);

    diff_edges(&before, chunk, None, Some(out_seeds))
}
