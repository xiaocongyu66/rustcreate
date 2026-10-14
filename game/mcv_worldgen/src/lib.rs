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
pub mod trees;
pub mod vanilla;

mod rust_noise;
mod rust_terrain;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    let mut heightmap = vec![mcv_core::WORLD_MIN_Y as i16; 256];
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
    // E7a：worldgen 仍是平铺 u16 产出（同 seed 同字节的确定性门
    // 不动），入库时一次 encode 成 palette 驻留。
    *handle.voxels.write().unwrap() =
        mcv_core::VoxelStore::from_flat_u16(out.voxels.as_u16_slice());
    *handle.heightmap.write().unwrap() = out.heightmap;
    handle.mark_dirty(mcv_core::dirty::SAVE);
    handle.advance_to(Stage::TerrainReady);
    // status 阶梯不再在此推进（刀 3a）：TerrainReady = SURFACE 的世界生成
    // 完成度改由票级别派生统一驱动（game.rs advance_status_by_level——授权
    // 门 = 票级别 target，证据门 = 本函数落下的 stage 位；替换刀 2 的
    // 「同点同条件」接线，语义等价：commit 只发生在请求环内 = level ≤ 33
    // 授权域，推进时机同帧）。
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

/// worker 生成 panic 时失败回执携带的 rc（仅日志取证粒度；drain 端
/// `Err` 分支释放名额的判据是 `Err` 本身，不读 rc 值）。
pub const ERR_PANIC: i32 = -7;

/// Background terrain generation over a shared worker pool. Results arrive
/// on [`Self::results`]; the caller commits them on the main thread. 后端在
/// 构造时按 [`TerrainBackend::from_env`] 固定（后台线程反复读环境变量无益）。
pub struct TerrainScheduler {
    seed: u64,
    backend: TerrainBackend,
    pool: TaskPool,
    res_tx: Sender<GenResult>,
    res_rx: Receiver<GenResult>,
    /// 自记在途计数（B 修静默门）：request +1 → **回执被主线程消化时 −1**
    /// （[`Self::recv_result`]）。语义 = 「池内排队/执行 + 已产出未消化」
    /// 全零，正是「静默后回执不可能再产出」的原意——只看池内状态会在
    /// 「send 完成后、回执躺在通道等 drain」的窗口里误报静默（flap 两连
    /// 红根因，run 38478972059：误静默 → 卸载段逐出 → 同帧 drain 落到已
    /// 删键 → 白生成振荡）。主线程独占读写（request/recv_result/seam 全
    /// 在主线程），Atomic 只为 Send/Sync 无谓。
    gen_inflight: Arc<AtomicU64>,
    /// 丢失注入钩子（`#[doc(hidden)]` 测试面；生产恒 None）。Arc 与请求
    /// 闭包共享——回执通道两端同源（同 E3/E5 Gate 先例）。
    loss_hook: Arc<std::sync::Mutex<Option<mcv_core::pool::LossHook>>>,
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
            gen_inflight: Arc::new(AtomicU64::new(0)),
            loss_hook: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Queues one chunk for generation (low priority; edits run high).
    pub fn request(&self, pos: ChunkPos) {
        let tx = self.res_tx.clone();
        let seed = self.seed;
        let backend = self.backend;
        let loss_hook = Arc::clone(&self.loss_hook);
        let inflight = Arc::clone(&self.gen_inflight);
        // 自记在途 +1（入池前——任何路径的回执消化都会配对 −1，见
        // recv_result；Swallow 无回执，闭包内自行归零）。
        self.gen_inflight.fetch_add(1, Ordering::AcqRel);
        self.pool.spawn_lo(Box::new(move || {
            // 丢失注入钩子（测试面；生产恒 None）。
            let mode = loss_hook
                .lock()
                .unwrap()
                .as_ref()
                .map_or(mcv_core::pool::LossMode::Deliver, |f| f(pos));
            if mode == mcv_core::pool::LossMode::Swallow {
                // 静默蒸发：无回执（此形态 = req_frames 配额死账的历史形态，
                // 无自愈回路；生产不可达——send 只在主线程弃收时失败，即关
                // 停。回归锁只注入可自愈的 Panic 路径）。计数自行归零
                // （等价「该请求从未投出」——不欠回执）。
                inflight.fetch_sub(1, Ordering::AcqRel);
                return;
            }
            // DC-3 范本（game.rs:1317-1342 IO 失败回执）：catch_unwind 包
            // 「计算」段，panic 时合成 `Err` 失败回执——drain 端既有 `Err`
            // 分支释放 req_frames 名额（零新增记账）。**删除此包 = panic
            // 静默吞产物 → req_frames 永真 → 配额槽泄漏满额后 pops≡0 全图
            // 生成停摆（DC-2 死账复发）**——池层 run_isolated 只保 worker
            // 线程存活，不保产物（pool.rs 池语义注）。
            let generated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                generate_terrain_with(backend, seed, pos).map_err(|rc| (pos, rc))
            }));
            let result = match generated {
                Ok(r) => GenResult::Terrain(r),
                Err(payload) => {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("<non-string panic payload>");
                    log::error!(
                        "[terrain] generate panicked at {pos:?}; worker survives, failure receipt sent: {msg}"
                    );
                    GenResult::Terrain(Err((pos, ERR_PANIC)))
                }
            };
            let _ = tx.send(result);
            // 注意：在途计数**不在 send 后减**——「已产出未消化」的回执
            // 窗口必须算在途（静默门原意），归零点在主线程消化回执处
            // （[`Self::recv_result`]）。
        }));
    }

    /// 丢失注入钩子安装（`#[doc(hidden)]` 测试面）：按区块决定 worker 侧
    /// 结局（Deliver/Swallow/Panic，语义见 [`mcv_core::pool::LossMode`]）。
    /// 失败回执回归锁的注入机制（tests/terrain_receipt.rs）。
    #[doc(hidden)]
    pub fn install_loss_hook(
        &self,
        hook: Box<dyn Fn(ChunkPos) -> mcv_core::pool::LossMode + Send + Sync>,
    ) {
        *self.loss_hook.lock().unwrap() = Some(hook);
    }

    /// 消化一笔生成回执（主线程 drain 环唯一入口）：try_recv + 在途计数
    /// 配对 −1——计数归零点 = 回执消化点（静默门语义见
    /// [`Self::pool_quiesced`]）。None = 暂无回执。
    pub fn recv_result(&self) -> Option<GenResult> {
        let r = self.res_rx.try_recv().ok()?;
        self.gen_inflight.fetch_sub(1, Ordering::AcqRel);
        Some(r)
    }

    /// 测试钩子：合成一笔失败回执入通道（不跑生成——复现「已产出未消化」
    /// 漏态窗；消化路径与真回执同走 [`Self::recv_result`]，计数配对归零）。
    #[doc(hidden)]
    pub fn debug_inject_receipt(&self, pos: ChunkPos) {
        let _ = self.res_tx.send(GenResult::Terrain(Err((pos, ERR_PANIC))));
    }

    /// 测试钩子：在途计数 +1（模拟「已投递未回执」——静默门漏态锁的
    /// 登记缝，与 [`GameRuntime` 上层 req_debug_mark] 配套）。
    #[doc(hidden)]
    pub fn debug_mark_inflight(&self) {
        self.gen_inflight.fetch_add(1, Ordering::AcqRel);
    }

    pub fn results(&self) -> &Receiver<GenResult> {
        &self.res_rx
    }

    /// 生成侧是否已排空静默：**自记在途计数 == 0**（池内排队/执行 +
    /// 已产出未消化的回执全零）。Empty 驻留的孤儿回收以此为必要条件，
    /// 近似原版「ticket 释放等 future」（DistanceManager.java:87-104）：
    /// 只要未静默，请求可能仍在排队/执行、或产物已产出待消化，回收即
    /// 误逐活体（#178 刀 3c 取证：矿石 pass 后队列+生成时延合法超 300
    /// stream-frame，flap 测 fr99-101 逐帧复现——python 建模红窗
    /// S0∈[202,212] 泵长）。旧判据 `stats().running==0 && queued==0`
    /// 只看池内状态，在「send 完成后、回执等 drain」窗口误报静默
    /// （flap 两连红根因，run 38478972059）——B 修改为计数判据。
    #[must_use]
    pub fn pool_quiesced(&self) -> bool {
        self.gen_inflight.load(Ordering::Acquire) == 0
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
