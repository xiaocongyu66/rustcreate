//! Terrain orchestration: drives the terrain kernel per chunk and commits
//! results into chunk handles. Pure-function entry point is testable;
//! [`TerrainScheduler`] adds background execution.
//!
//! Backend switch（任务板 #91 地形 2.0）：`MCV_TERRAIN_BACKEND` 取值
//! `vanilla`（26.1 机制 clean-room 编排管线，src/vanilla/，未设置时的新
//! 默认）或 `legacy`（value noise + fBm 四通道回归基线，src/rust_terrain.rs）。
//! 任务板 #77 拆除 C++ frozen oracle（原 mcv_ffi → cpp/src/terrain.cpp）后，
//! 旧 "rust" 值与其余一切取值统一回退 legacy——legacy 是 oracle 的逐位
//! Rust 移植，其输出由 tests/golden.rs 对黄金数据（删除前 oracle 落盘）
//! 逐字节回归锁定。

pub mod os2s;
pub mod vanilla;

mod rust_noise;
mod rust_terrain;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use mcv_core::{ChunkHandle, ChunkPos, ChunkVoxels, Stage, TaskPool};

/// 地形生成后端。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerrainBackend {
    /// legacy：value noise + fBm 四通道（回归基线，src/rust_terrain.rs）。
    Legacy,
    /// vanilla：26.1 机制 clean-room 编排管线（新默认，src/vanilla/）。
    Vanilla,
}

impl TerrainBackend {
    /// `MCV_TERRAIN_BACKEND=vanilla|legacy`；未设置 → 新默认 vanilla；
    /// 旧 "rust" 值与其余一切取值回退 legacy（#77 前 oracle 充当冻结
    /// 基线，现由逐位移植的 legacy + 黄金数据承担同一职责）。
    pub fn from_env() -> Self {
        Self::from_value(std::env::var("MCV_TERRAIN_BACKEND").ok().as_deref())
    }

    /// [`Self::from_env`] 的纯函数核（edition 2024 的 `set_var` 是 unsafe
    /// 且与并行测试的 `getenv` 有数据竞争，故语义单测走这里，不动真实环境）。
    fn from_value(v: Option<&str>) -> Self {
        match v {
            Some("vanilla") => Self::Vanilla,
            None => Self::Vanilla,
            // "legacy"、旧 "rust" 以及其余一切取值 → 冻结基线。
            Some(_) => Self::Legacy,
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
    /// 地表高度图，**绝对 y**（i16；全空列 = WORLD_MIN_Y 哨兵）。
    pub heightmap: Box<[i16; 256]>,
}

/// Pure function: generate one chunk's voxels + heightmap（按环境变量选后端）。
pub fn generate_terrain(seed: u64, pos: ChunkPos) -> Result<TerrainOutput, i32> {
    generate_terrain_with(TerrainBackend::from_env(), seed, pos)
}

/// 显式后端生成（测试/调度器用，避免环境变量歧义）。
pub fn generate_terrain_with(
    backend: TerrainBackend,
    seed: u64,
    pos: ChunkPos,
) -> Result<TerrainOutput, i32> {
    let mut voxels = ChunkVoxels::filled(mcv_core::BlockId(0));
    let mut heightmap = vec![mcv_core::WORLD_MIN_Y; 256];
    match backend {
        TerrainBackend::Legacy => {
            rust_terrain::generate(
                seed,
                pos.x,
                pos.z,
                voxels.as_u16_slice_mut(),
                &mut heightmap,
            )?;
        }
        TerrainBackend::Vanilla => {
            vanilla::terrain::generate(
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
/// the single Rust-side producer; the retired terrain kernel's pass 3
/// (cpp/src/terrain.cpp，黄金数据锚定的冻结基线，exclusion =
/// air/water/flowers) and this function agree on every block the generator
/// emits: the skip set here is
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
/// v6 几何：值 = **绝对 y**（i16），扫描域 [WORLD_MIN_Y, WORLD_MAX_Y)；
/// 全空列 = `WORLD_MIN_Y` 哨兵（旧 256 基准的「全开 ⇒ 1」约定随坐标域
/// 一并作废）。Lighting does NOT read this array (source columns are
/// voxel-derived, see `mcv_light::init`).
pub fn recompute_heightmap(voxels: &[u16]) -> Box<[i16; 256]> {
    let mut hm = Box::new([mcv_core::WORLD_MIN_Y as i16; 256]);
    for z in 0..16usize {
        for x in 0..16usize {
            let mut found = None;
            for y in (mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).rev() {
                // 掩掉状态位（bit12-15，半砖/楼梯朝向）：按基础方块查表，
                // 否则带状态体素越界误判未注册=全挡。
                let id = (voxels[mcv_core::vidx(x, y, z)]) & mcv_core::ID_MASK;
                // 跳过集：damp==0（全透光：空气/花/玻璃/火把…）或流体。
                // 不计流体是自研地表基线（出生点取水下地表）；对照原版：
                // MOTION_BLOCKING 计流体（Heightmap.java:151），不计流体的是
                // OCEAN_FLOOR=blocksMotion（Heightmap.java:31/149-151，流体
                // 无碰撞不计，BlockBehaviour.java:540-543）。生成期排除集
                // air/water/flowers 恰是该跳过集的 legacy 子集。
                let damp = mcv_core::OPACITY.get(id as usize).copied().unwrap_or(15);
                let liquid = mcv_core::BLOCKS.get(id as usize).is_some_and(|b| b.liquid);
                if damp != 0 && !liquid {
                    found = Some(y);
                    break;
                }
            }
            // heightmap 语义 =「首个非遮光格绝对 y」；y=WORLD_MAX_Y-1 遮光
            // 时钳到列顶（任务板 #60 饱和钳制的 384 版——i16 虽不绕回，
            // 「+1 出界」仍不是合法地表槽位）。
            hm[(z << 4) | x] = match found {
                Some(y) => (y + 1).min(mcv_core::WORLD_MAX_Y - 1) as i16,
                None => mcv_core::WORLD_MIN_Y as i16,
            };
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

    /// 后端开关语义锁：精确 "vanilla" 选 vanilla、未设置 → 新默认 vanilla；
    /// "legacy"、旧 "rust" 与其余一切取值回冻结基线 legacy（#77 前 oracle
    /// 承担该角色，删除后由逐位移植 + 黄金数据承担）。纯函数核不触真实环境。
    #[test]
    fn backend_switch_semantics() {
        for (val, want) in [
            (Some("legacy"), TerrainBackend::Legacy),
            (Some("vanilla"), TerrainBackend::Vanilla),
            (Some("rust"), TerrainBackend::Legacy),
            (Some("Rust"), TerrainBackend::Legacy),
            (Some("ffi"), TerrainBackend::Legacy),
            (Some(""), TerrainBackend::Legacy),
            (None, TerrainBackend::Vanilla),
        ] {
            assert_eq!(
                TerrainBackend::from_value(val),
                want,
                "MCV_TERRAIN_BACKEND={val:?} 语义漂移"
            );
        }
    }
}
