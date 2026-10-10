//! Terrain orchestration: drives the terrain kernel per chunk and commits
//! results into chunk handles. Pure-function entry point is testable;
//! [`TerrainScheduler`] adds background execution.
//!
//! Backend switch（任务板 #78 第一阶段：C++ 地形内核移植为纯 Rust）：
//! 默认仍走 C++ oracle 路径（mcv_ffi → cpp/src/terrain.cpp，冻结基线）；
//! `MCV_TERRAIN_BACKEND=rust` 切到纯 Rust 移植（src/rust_terrain.rs）。
//! 与 mesher 开关同款模式；生产默认切换属后续任务。两路输出由
//! tests/parity.rs 逐字节对拍锁定（对拍不过修 Rust 侧，禁改 cpp/**）。

pub mod os2s;
pub mod vanilla;

mod rust_noise;
mod rust_terrain;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use mcv_core::{ChunkHandle, ChunkPos, ChunkVoxels, Stage, TaskPool};
use mcv_ffi::terrain_generate_raw;

/// 地形生成后端。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerrainBackend {
    /// C++ oracle（mcv_ffi → cpp/src/terrain.cpp）。默认：本次只交付移植
    /// + 对拍门禁，生产默认切换留后续任务。
    Ffi,
    /// 纯 Rust 移植（src/rust_terrain.rs）。
    Rust,
}

impl TerrainBackend {
    /// `MCV_TERRAIN_BACKEND=rust` → 纯 Rust；其余（含未设置）→ oracle。
    pub fn from_env() -> Self {
        Self::from_value(std::env::var("MCV_TERRAIN_BACKEND").ok().as_deref())
    }

    /// [`Self::from_env`] 的纯函数核（edition 2024 的 `set_var` 是 unsafe
    /// 且与并行测试的 `getenv` 有数据竞争，故语义单测走这里，不动真实环境）。
    fn from_value(v: Option<&str>) -> Self {
        if v == Some("rust") {
            Self::Rust
        } else {
            Self::Ffi
        }
    }
}

impl Default for TerrainBackend {
    fn default() -> Self {
        Self::from_env()
    }
}

pub struct TerrainOutput {
    pub pos: ChunkPos,
    pub voxels: ChunkVoxels,
    pub heightmap: Box<[u8; 256]>,
}

/// Pure function: generate one chunk's voxels + heightmap（按环境变量选后端）。
pub fn generate_terrain(seed: u64, pos: ChunkPos) -> Result<TerrainOutput, i32> {
    generate_terrain_with(TerrainBackend::from_env(), seed, pos)
}

/// 显式后端生成（对拍测试用，避免环境变量歧义；与
/// `mcv_mesher::Mesher::with_backend` 同款）。
pub fn generate_terrain_with(
    backend: TerrainBackend,
    seed: u64,
    pos: ChunkPos,
) -> Result<TerrainOutput, i32> {
    let mut voxels = ChunkVoxels::filled(mcv_core::BlockId(0));
    let mut heightmap = vec![0u8; 256];
    match backend {
        TerrainBackend::Ffi => {
            terrain_generate_raw(
                seed,
                pos.x,
                pos.z,
                voxels.as_u16_slice_mut(),
                &mut heightmap,
            )?;
        }
        TerrainBackend::Rust => {
            rust_terrain::generate(
                seed,
                pos.x,
                pos.z,
                voxels.as_u16_slice_mut(),
                &mut heightmap,
            )?;
        }
    }
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
/// "lightDampening == 0 (air/flowers/glass/…) **or** liquid (water/lava)".
/// Skipping fluids is *our* documented gameplay baseline (spawn surface
/// under sea water, matching the terrain kernel): vanilla 26.1
/// MOTION_BLOCKING counts fluids (`blocksMotion() || !fluidState.isEmpty()`,
/// Heightmap.java:151); the no-fluid analogue is OCEAN_FLOOR
/// (`MATERIAL_MOTION_BLOCKING = blocksMotion`, Heightmap.java:31/149-151 —
/// water/lava collide as empty, `BlockBehaviour.java:540-543`). Among the
/// legacy terrain ids exactly air+water+flowers fall in our skip set
/// (`mcv_core::OPACITY`).
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
                // 掩掉状态位（bit12-15，半砖/楼梯朝向）：按基础方块查表，
                // 否则带状态体素越界误判未注册=全挡。
                let id = (voxels[(y << 8) | (z << 4) | x]) & mcv_core::ID_MASK;
                // 跳过集：damp==0（全透光：空气/花/玻璃/火把…）或流体。
                // 不计流体是自研地表基线（出生点取水下地表）；对照原版：
                // MOTION_BLOCKING 计流体（Heightmap.java:151），不计流体的是
                // OCEAN_FLOOR=blocksMotion（Heightmap.java:31/149-151，流体
                // 无碰撞不计，BlockBehaviour.java:540-543）。生成期排除集
                // air/water/flowers 恰是该跳过集的 legacy 子集。
                let damp = mcv_core::OPACITY.get(id as usize).copied().unwrap_or(15);
                let liquid = mcv_core::BLOCKS.get(id as usize).is_some_and(|b| b.liquid);
                if damp != 0 && !liquid {
                    found = true;
                    break;
                }
                y -= 1;
            }
            // 饱和钳制（任务板 #60 / coords 审计 P2 并案）：y=255 遮光时
            // y+1=256 在 u8 上 debug panic / release 绕回 0——heightmap
            // 语义 =「首个非遮光格 y」，无解时钳到列顶 255。
            hm[(z << 4) | x] = if found { (y + 1).min(255) as u8 } else { 1 };
        }
    }
    hm
}

pub enum GenResult {
    Terrain(Result<TerrainOutput, (ChunkPos, i32)>),
}

/// Background terrain generation over a shared worker pool. Results arrive
/// on [`Self::results`]; the caller commits them on the main thread. 后端在
/// 构造时按 [`TerrainBackend::from_env`] 固定（后台线程反复读环境变量无益）。
pub struct TerrainScheduler {
    seed: u64,
    backend: TerrainBackend,
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
            backend: TerrainBackend::from_env(),
            pool,
            res_tx: tx,
            res_rx: rx,
        }
    }

    /// Queues one chunk for generation (low priority; edits run high).
    pub fn request(&self, pos: ChunkPos) {
        let tx = self.res_tx.clone();
        let seed = self.seed;
        let backend = self.backend;
        self.pool.spawn_lo(Box::new(move || {
            let _ = tx.send(GenResult::Terrain(
                generate_terrain_with(backend, seed, pos).map_err(|rc| (pos, rc)),
            ));
        }));
    }

    pub fn results(&self) -> &Receiver<GenResult> {
        &self.res_rx
    }
}

#[cfg(test)]
mod tests {
    use super::TerrainBackend;

    /// 后端开关语义锁：只有精确 "rust" 选 Rust；其余一切值（含未设置）回退
    /// oracle（派单约束「默认仍走 oracle」）。纯函数核不触真实环境变量。
    #[test]
    fn backend_switch_maps_only_exact_rust_value() {
        for (val, want) in [
            (Some("rust"), TerrainBackend::Rust),
            (Some("Rust"), TerrainBackend::Ffi),
            (Some("ffi"), TerrainBackend::Ffi),
            (Some(""), TerrainBackend::Ffi),
            (None, TerrainBackend::Ffi),
        ] {
            assert_eq!(
                TerrainBackend::from_value(val),
                want,
                "MCV_TERRAIN_BACKEND={val:?} 语义漂移"
            );
        }
    }
}
