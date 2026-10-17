//! E3 网格化并行调度（蓝图 docs/engine-roadmap.md §3 E3）：worker 跑
//! greedy mesh，主线程只做快照与提交——快照-计算-回传三段式的「计算」与
//! 「回传」两端。
//!
//! **统一原则（worker 路死账教训，docs/worker-accounting.md）**：记账推进
//! 必须与产物确认配对；失败路径必须有回执（DC-3 范本 = game.rs
//! `spawn_io_worker` 的失败回执合成）。本模块的配对面 = `submit` 返回
//! 派发 seq（上层登记在途账）↔ 产物/丢失标记经 mpsc 回传（上层销账或
//! 即时回滚）+ 主线程侧 K 帧看门狗超时兜底。
//!
//! - **计算端** [`MeshScheduler`]：主线程把 3×3 快照字节（[`MeshJob`]）
//!   投入专属 `TaskPool`（编辑源块 hi 通道 / 预算流 lo 通道），worker 只
//!   见字节、无共享可变状态，调 [`Mesher::build`]（opaque+water，mesher
//!   本体零改动）后把产物字节经 mpsc 回传。
//! - **回传端** [`MeshResults`]：主线程侧取产物；按「提交地板」过滤两类
//!   毒产物——①僵尸（区块已卸载，提交会复活网格条目）②陈旧（同区块更
//!   高 seq 的新快照已提交，旧快照不得覆盖）。**MeshUploader 状态机与
//!   wgpu 句柄不出主线程**（GLES/adapter 线程亲和 + #80 状态机时序锁），
//!   本模块只搬字节，不持任何 GPU 句柄。
//!
//! 回滚开关：`MCV_MESHER_WORKERS=0` → 上层不装配本模块，整条走 E3 前的
//! 同步路径（对齐 `MCV_TERRAIN_BACKEND` 先例，mcv_worldgen/src/lib.rs
//! `TerrainBackend::from_env`）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mcv_core::ChunkPos;
use mcv_core::pool::{LossHook, LossMode, TaskPool};

use crate::{MESH_OPAQUE, MESH_WATER, Mesher, Slot};

/// 一笔网格产物字节（顶点 24B 步长交错 + u32 索引，[`crate::MeshData`]
/// 的所有权化搬运）。
pub struct MeshBytes {
    pub vertex: Vec<u8>,
    pub index: Vec<u32>,
}

/// 主线程 → worker 的网格任务：3×3 快照裸字节（布局 `mcv_core::vidx`，
/// 行主序 dz 外层；与同步路径 [`neighborhood_slots`] 同源装配）。
pub struct MeshJob {
    pub pos: ChunkPos,
    /// 绘制原点（16 格网格点；回传时随产物原样带回）。
    pub origin: [f32; 3],
    pub voxels: Vec<u16>,
    pub lights: Vec<u8>,
    /// #191 群系 tint 场（主线程按世界坐标采样构建；worker 纯计算，
    /// None = 关闭染色 → 顶点色白）。区块世界方块原点 = pos × 16。
    pub tint: Option<mcv_core::tint::ChunkTint>,
}

/// worker → 主线程的网格产物。
pub struct MeshDone {
    pub pos: ChunkPos,
    pub origin: [f32; 3],
    /// 派发通道（true = 编辑源块优先通道）——顺序锁的观测位。
    pub hi: bool,
    /// 派发序号（调度器内单调递增；陈旧过滤的依据）。
    pub seq: u64,
    pub opaque: Option<MeshBytes>,
    pub water: Option<MeshBytes>,
}

/// 提交地板（调度器与回传端共享）：seq 单调源 + per-pos 拒收线。
struct Gate {
    seq: AtomicU64,
    /// 已完成、滞留 mpsc 的产物数（测试观测面；非调度依据）。
    pending: AtomicU64,
    /// per-pos 提交地板：seq ≤ 地板的产物拒收。kill（卸载）抬到当前
    /// seq（杀全部在途），提交抬到所提交 seq（杀更旧的乱序产物）。
    floor: Mutex<HashMap<ChunkPos, u64>>,
    /// 丢失注入钩子（`#[doc(hidden)]` 测试面；生产恒 None——查询短路
    /// Deliver）。放 Gate = 回传端（测试持有）可装钩、worker 闭包（持
    /// 同一 Arc）可查询，两端同源。
    loss_hook: Mutex<Option<LossHook>>,
}

/// 计算端：专属 worker 池 + 产物通道发送端。
pub struct MeshScheduler {
    pool: TaskPool,
    tx: std::sync::mpsc::Sender<MeshDone>,
    gate: Arc<Gate>,
    /// 背压高水位（在途排队+执行 ≥ 此值时 [`Self::saturated`]）。
    high_water: usize,
}

impl MeshScheduler {
    /// 装配计算端与回传端。`workers` 由上层按 `MCV_MESHER_WORKERS`
    /// 解析（0 = 同步路径，不进本函数）。执行端 = rayon 全局池
    /// （`TaskPool::shared`，三域超订收口——本数 = 配额/回压上限，
    /// 不再自建 OS 线程组）；hi/lo 优先与回执纪律在本模块原样。
    pub fn new(workers: usize) -> (Self, MeshResults) {
        let pool = TaskPool::shared(workers);
        let (tx, rx) = std::sync::mpsc::channel();
        let high_water = workers + 2;
        let gate = Arc::new(Gate {
            seq: AtomicU64::new(0),
            pending: AtomicU64::new(0),
            floor: Mutex::new(HashMap::new()),
            loss_hook: Mutex::new(None),
        });
        (
            Self {
                pool,
                tx,
                gate: Arc::clone(&gate),
                high_water,
            },
            MeshResults { rx, gate },
        )
    }

    /// 投递一笔网格任务。`hi` = 编辑源块优先通道（mesh_priority），else
    /// 预算流。seq 在投递时点分配——「拿到快照的那一刻」的派发序；**返回
    /// 值 = 派发序号**，上层 mesh 看门狗以 (pos, seq) 登记在途账（记账推进
    /// 必须与产物确认配对，docs/worker-accounting.md）。
    pub fn submit(&self, job: MeshJob, hi: bool) -> u64 {
        let seq = self.gate.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let tx = self.tx.clone();
        // 闭包持整份 Arc<Gate>：seq 源/pending 计数/地板表同源（Send+Sync）。
        let gate = Arc::clone(&self.gate);
        let run = move || {
            // 丢失注入钩子（测试面；生产恒 None）。
            let mode = gate
                .loss_hook
                .lock()
                .unwrap()
                .as_ref()
                .map_or(LossMode::Deliver, |f| f(job.pos));
            if mode == LossMode::Swallow {
                // 静默蒸发：不产生任何回执（看门狗 K 帧兜底路径的回归注入
                // ——DC-4 死账的历史形态，tests/mesh_parallel.rs 看门狗锁）。
                return;
            }
            // DC-3 范本（game.rs:1317-1342 IO 失败回执）：catch_unwind 包
            // 「计算」段，panic 时合成 opaque=None 丢失标记回传。
            // **删除此包 = panic 静默吞产物 → meshed 记账永真、重派门永久
            // 关闭（DC-1 死账复发 = 真机隐形墙）**——池层 run_isolated 只
            // 保 worker 线程存活，不保产物（pool.rs 池语义注）。
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // 丢失注入（测试面）：Panic 模式在包内引爆——被本 catch_unwind
                // 接住合成丢失标记（Swallow 在包外已拦截，生产恒 Deliver）。
                if mode == LossMode::Panic {
                    panic!("mesh worker loss injection (test) at {:?}", job.pos);
                }
                let vol = mcv_core::CHUNK_VOL;
                let slots = neighborhood_slots(&job.voxels, &job.lights);
                debug_assert_eq!(job.voxels.len(), 9 * vol);
                debug_assert_eq!(job.lights.len(), 9 * vol);
                // Mesher 零尺寸 Copy，worker 本地实例；同输入同产物（等价锁
                // tests/mesh_parallel.rs::worker_output_bit_identical 锁定）。
                let mesher = Mesher::new();
                // #191：opaque pass 带群系 tint 场（原点 = 区块世界方块坐标），
                // water pass 无 tint（流体常数色在着色器侧）。
                let tint = job.tint.clone().map(|t| crate::TintInput {
                    block_x: job.pos.x * 16,
                    block_z: job.pos.z * 16,
                    tint: t,
                });
                let opaque = mesher.build_with_tint(&slots, MESH_OPAQUE, tint).ok();
                let water = mesher.build(&slots, MESH_WATER).ok();
                (
                    opaque.map(|m| MeshBytes {
                        vertex: m.vertices,
                        index: m.indices,
                    }),
                    water.map(|m| MeshBytes {
                        vertex: m.vertices,
                        index: m.indices,
                    }),
                )
            }));
            let (opaque, water) = match built {
                Ok(pair) => pair,
                Err(payload) => {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("<non-string panic payload>");
                    log::error!(
                        "[mesh] worker build panicked at {:?}; worker survives, loss marker sent: {msg}",
                        job.pos
                    );
                    (None, None) // 丢失标记：回传端按「产物缺失」记账回滚
                }
            };
            let done = MeshDone {
                pos: job.pos,
                origin: job.origin,
                hi,
                seq,
                opaque,
                water,
            };
            if tx.send(done).is_ok() {
                gate.pending.fetch_add(1, Ordering::Relaxed);
            }
        };
        if hi {
            self.pool.spawn_hi(Box::new(run));
        } else {
            self.pool.spawn_lo(Box::new(run));
        }
        seq
    }

    /// 背压门：在途（排队+执行）≥ 高水位（workers+2）时为真——派发方
    /// 应停投（不耗预算），防 worker 饥饿时快照字节（~2.65 MiB/笔）与
    /// 产物（~2.5 MiB/笔）无界堆积。峰值在途 ≈ workers+2 笔。
    pub fn saturated(&self) -> bool {
        let s = self.pool.stats();
        s.queued + s.running >= self.high_water
    }

    /// 池遥测快照（E1 `stats()` 同源：在途/排队/worker 占用；队列观测面）。
    pub fn stats(&self) -> mcv_core::pool::PoolStats {
        self.pool.stats()
    }
}

/// 回传端：主线程侧产物取用（**只在主线程调用**；状态机与 GPU 句柄在
/// 上层 `mcv_render::gpu::MeshUploader`，不经过本模块）。
pub struct MeshResults {
    rx: std::sync::mpsc::Receiver<MeshDone>,
    gate: Arc<Gate>,
}

/// 地板修剪龄：seq 落后当前 seq 超过此值的地板条目可删——在途任务数被
/// 背压门压在 high_water 内，更旧的 seq 不可能再有在途产物（余量 4 倍
/// + 常数，测试侧无背压门也有界）。
const FLOOR_PRUNE_AGE: u64 = 256;

impl MeshResults {
    /// 取走全部**获准提交**的产物：僵尸（seq ≤ kill 地板）与陈旧（seq ≤
    /// 已提交 seq 的更旧产物）在此拒收并就地弃置。每次取走后按龄修剪
    /// 地板表（有界内存）。
    pub fn drain_committed(&mut self) -> Vec<MeshDone> {
        let mut out = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            self.gate.pending.fetch_sub(1, Ordering::Relaxed);
            let mut floor = self.gate.floor.lock().unwrap();
            let admit = done.seq > *floor.get(&done.pos).unwrap_or(&0);
            if admit {
                floor.insert(done.pos, done.seq);
                out.push(done);
            }
            // 拒收分支：产物就地 drop（~2.5 MiB 字节随之释放）。
        }
        let cur = self.gate.seq.load(Ordering::Relaxed);
        self.gate
            .floor
            .lock()
            .unwrap()
            .retain(|_, f| cur.saturating_sub(*f) <= FLOOR_PRUNE_AGE);
        out
    }

    /// 区块卸载记账：把该块的提交地板抬到当前 seq——其后回传的全部在途
    /// 产物（seq ≤ 当前）拒收，防 `build_chunk` 把网格条目复活到已卸载
    /// 键上。重进视野后的新派发 seq 更高，不受影响。
    pub fn kill(&mut self, pos: ChunkPos) {
        let cur = self.gate.seq.load(Ordering::Relaxed);
        self.gate.floor.lock().unwrap().insert(pos, cur);
    }

    /// 已完成、滞留通道的产物数（测试观测面；非消费、非调度依据）。
    #[doc(hidden)]
    pub fn pending_done(&self) -> usize {
        self.gate.pending.load(Ordering::Relaxed) as usize
    }

    /// 取走全部原始产物（不过滤；测试取证面——生产路径只走
    /// [`Self::drain_committed`]）。
    #[doc(hidden)]
    pub fn drain_all(&mut self) -> Vec<MeshDone> {
        let mut out = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            self.gate.pending.fetch_sub(1, Ordering::Relaxed);
            out.push(done);
        }
        out
    }

    /// 丢失注入钩子（`#[doc(hidden)]` 测试面）：按区块决定 worker 侧结局
    /// （Deliver/Swallow/Panic，语义见 [`mcv_core::pool::LossMode`]）。
    /// 生产装配不装钩子。看门狗/丢失回执回归锁的注入机制
    /// （tests/mesh_parallel.rs）。
    #[doc(hidden)]
    pub fn install_loss_hook(&self, hook: LossHook) {
        *self.gate.loss_hook.lock().unwrap() = Some(hook);
    }
}

/// `MCV_MESHER_WORKERS` 语义：未设 → `default`（world 默认池策略）；
/// "0" → 0（同步路径锚点，回滚开关）；正整数 → 该值；其余一切（非法/
/// 空/负/带空白）→ `default`（0 只能显式要求，对齐 Integer.parseInt
/// 不 trim 语义，同 pool.rs `worker_count_for` 的理由纯函数化）。
pub fn worker_count_from(v: Option<&str>, default: usize) -> usize {
    match v {
        None => default,
        Some(s) => s.parse::<usize>().unwrap_or(default),
    }
}

/// 生产解析：env 每次构造时读一次（主线程；后台线程不反复读 env，
/// 同 `TerrainScheduler::new` 的后端固定纪律）。默认对齐 world 任务池
/// 策略 `mcv_core::world_worker_count()`。
pub fn mesh_worker_count() -> usize {
    worker_count_from(
        std::env::var("MCV_MESHER_WORKERS").ok().as_deref(),
        mcv_core::world_worker_count(),
    )
}

/// 从 3×3 快照字节装配邻域槽位（行主序 dz 外层，`(dz+1)*3+(dx+1)`，
/// 中心 = [4]）。同步路径（`RustMesher::build`）与 worker
/// （[`MeshScheduler::submit`]）共用同一装配，保证两路喂给
/// [`Mesher::build`] 的输入布局逐字节同源。
pub fn neighborhood_slots<'a>(voxels: &'a [u16], lights: &'a [u8]) -> [Option<Slot<'a>>; 9] {
    let vol = mcv_core::CHUNK_VOL;
    debug_assert!(voxels.len() >= 9 * vol);
    debug_assert!(lights.len() >= 9 * vol);
    std::array::from_fn(|i| {
        Some(Slot {
            voxels: &voxels[i * vol..(i + 1) * vol],
            light: &lights[i * vol..(i + 1) * vol],
        })
    })
}
