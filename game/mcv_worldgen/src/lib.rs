//! Terrain orchestration: drives the C++ terrain kernel per chunk and
//! commits results into chunk handles. Pure-function entry point is
//! testable; [`TerrainScheduler`] adds background execution.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use mcv_core::{ChunkHandle, ChunkPos, ChunkVoxels, Stage, TaskPool};
use mcv_ffi::terrain_generate_raw;

pub struct TerrainOutput {
    pub pos: ChunkPos,
    pub voxels: ChunkVoxels,
    pub heightmap: Box<[u8; 256]>,
}

/// Pure function: generate one chunk's voxels + heightmap.
pub fn generate_terrain(seed: u64, pos: ChunkPos) -> Result<TerrainOutput, i32> {
    let mut voxels = ChunkVoxels::filled(mcv_core::BlockId(0));
    let mut heightmap = vec![0u8; 256];
    terrain_generate_raw(
        seed,
        pos.x,
        pos.z,
        voxels.as_u16_slice_mut(),
        &mut heightmap,
    )?;
    Ok(TerrainOutput {
        pos,
        voxels,
        heightmap: heightmap.into_boxed_slice().try_into().unwrap(),
    })
}

/// Commits a terrain result into a chunk handle (stage Empty -> TerrainReady).
pub fn commit_terrain(handle: &Arc<ChunkHandle>, out: TerrainOutput) {
    debug_assert_eq!(handle.pos, out.pos);
    *handle.voxels.write().unwrap() = out.voxels.0;
    *handle.heightmap.write().unwrap() = out.heightmap;
    handle.mark_dirty(mcv_core::dirty::SAVE);
    handle.advance_to(Stage::TerrainReady);
}

/// Blocking convenience for tests and save loading.
pub fn generate_into(handle: &Arc<ChunkHandle>, seed: u64) -> Result<(), i32> {
    let out = generate_terrain(seed, handle.pos)?;
    commit_terrain(handle, out);
    Ok(())
}

/// Rebuilds the heightmap from voxel data (saves don't store it). This is
/// the single Rust-side producer; the terrain kernel's pass 3
/// (`cpp/src/terrain.cpp`, exclusion = air/water/flowers) and this function
/// agree on every block the generator emits: the skip set here is
/// "lightDampening == 0 (air/flowers/glass/…) **or** liquid (water/lava,
/// vanilla MOTION_BLOCKING ignores fluids)", and among the legacy terrain
/// ids exactly air+water+flowers fall in it (`mcv_core::OPACITY`).
///
/// C1 heightmap maintenance: after a player places/breaks a block the
/// runtime recomputes the whole map through this function, so the skip
/// predicate generalises from "the 3 legacy exclusions" to "damp == 0"
/// (light-penetrating blocks — torch/glass/plates/… — must not raise the
/// gameplay surface; opaque/attenuating blocks do). The generator never
/// emits such blocks, so C++ output and this function stay identical.
/// The "column fully open ⇒ 1" convention is kept verbatim (audit §已核实
/// 12). Lighting does NOT read this array (source columns are
/// voxel-derived, see `mcv_light::init`).
pub fn recompute_heightmap(voxels: &[u16]) -> Box<[u8; 256]> {
    let mut hm = Box::new([0u8; 256]);
    for z in 0..16usize {
        for x in 0..16usize {
            let mut y = 255usize;
            let mut found = false;
            while y > 0 {
                let id = voxels[(y << 8) | (z << 4) | x];
                // 跳过集：damp==0（全透光：空气/花/玻璃/火把…）或流体
                // （水/岩浆——原版 MOTION_BLOCKING=blocksMotion，流体无碰撞
                // 不计入，Heightmap.java:31 + BlockBehaviour.java:540-543；
                // 生成期排除集 air/water/flowers 恰是该规则的 legacy 子集）。
                let damp = mcv_core::OPACITY.get(id as usize).copied().unwrap_or(15);
                let liquid = mcv_core::BLOCKS.get(id as usize).is_some_and(|b| b.liquid);
                if damp != 0 && !liquid {
                    found = true;
                    break;
                }
                y -= 1;
            }
            hm[(z << 4) | x] = if found { y as u8 + 1 } else { 1 };
        }
    }
    hm
}

pub enum GenResult {
    Terrain(Result<TerrainOutput, (ChunkPos, i32)>),
}

/// Background terrain generation over a shared worker pool. Results arrive
/// on [`Self::results`]; the caller commits them on the main thread.
pub struct TerrainScheduler {
    seed: u64,
    pool: TaskPool,
    res_tx: Sender<GenResult>,
    res_rx: Receiver<GenResult>,
}

impl TerrainScheduler {
    pub fn new(seed: u64, workers: usize) -> Self {
        let pool = TaskPool::new(workers);
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            seed,
            pool,
            res_tx: tx,
            res_rx: rx,
        }
    }

    /// Queues one chunk for generation (low priority; edits run high).
    pub fn request(&self, pos: ChunkPos) {
        let tx = self.res_tx.clone();
        let seed = self.seed;
        self.pool.spawn_lo(Box::new(move || {
            let _ = tx.send(GenResult::Terrain(
                generate_terrain(seed, pos).map_err(|rc| (pos, rc)),
            ));
        }));
    }

    pub fn results(&self) -> &Receiver<GenResult> {
        &self.res_rx
    }
}
