//! E3 网格化并行调度（蓝图 docs/engine-roadmap.md §3 E3）：worker 跑
//! greedy mesh，主线程只做快照与提交——快照-计算-回传三段式的「计算」与
//! 「回传」两端。
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
use mcv_core::pool::TaskPool;

use crate::{KindProducts, MESH_OPAQUE, MESH_WATER, Mesher, Rows, Slot};

/// 一个区块两种 kind 的分 section 产物（#172 增量重放的中转态；一笔
/// 网格任务同时算 opaque+water，产物成对存取）。
pub type ChunkProducts = (KindProducts, KindProducts);

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
    /// #172 重放工作量观测面：本笔重算的 (轴,向,层) 扫描行数 + 形状
    /// section 数。整柱直算 = [`crate::TOTAL_SCANNED_ROWS`] +
    /// [`crate::SECTION_COUNT`]（收益断言的分母）。
    pub work: usize,
    /// #172 增量路产物：[`MeshResults::drain_committed`] 经提交地板过滤
    /// 后复制入调度器侧缓存（供下一笔编辑增量重放），本副本原样保留
    /// （观测面）；直算路 = None（准入时作废既有缓存——直算落地后任何
    /// 旧产物都不再与最新快照同源，见 [`MeshScheduler::submit`]）。
    pub products: Option<ChunkProducts>,
}

/// 提交地板（调度器与回传端共享）：seq 单调源 + per-pos 拒收线。
struct Gate {
    seq: AtomicU64,
    /// 已完成、滞留 mpsc 的产物数（测试观测面；非调度依据）。
    pending: AtomicU64,
    /// per-pos 提交地板：seq ≤ 地板的产物拒收。kill（卸载）抬到当前
    /// seq（杀全部在途），提交抬到所提交 seq（杀更旧的乱序产物）。
    floor: Mutex<HashMap<ChunkPos, u64>>,
    /// #172 per-pos 分 section 产物缓存（增量重放的 `old`）：只在
    /// [`MeshResults::drain_committed`] 准入点后写入（僵尸/陈旧产物不入
    /// 缓存），kill/卸载与直算路派发时删除——不变式：缓存存在 ⇒ 产物与
    /// 最近一次准入提交的快照逐字节同源，下一笔增量只需重放脏行。
    products: Mutex<HashMap<ChunkPos, ChunkProducts>>,
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
    /// 解析（0 = 同步路径，不进本函数）。
    pub fn new(workers: usize) -> (Self, MeshResults) {
        let pool = TaskPool::new(workers);
        let (tx, rx) = std::sync::mpsc::channel();
        let high_water = workers + 2;
        let gate = Arc::new(Gate {
            seq: AtomicU64::new(0),
            pending: AtomicU64::new(0),
            floor: Mutex::new(HashMap::new()),
            products: Mutex::new(HashMap::new()),
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

    /// 投递一笔网格任务（整柱直算路，E3 原语义逐字节不动）。`hi` = 编辑
    /// 源块优先通道（mesh_priority），else 预算流。seq 在投递时点分配——
    /// 「拿到快照的那一刻」的派发序。
    ///
    /// #172：派发即作废该块的产物缓存——直算不回传产物，直算提交落地后
    /// 旧产物缓存不再与任何快照同源，留着只会被后续增量误用。
    pub fn submit(&self, job: MeshJob, hi: bool) {
        self.gate.products.lock().unwrap().remove(&job.pos);
        self.spawn(job, hi, None, None);
    }

    /// #172 增量投递：worker 用 [`crate::Mesher::build_products`] 只重放
    /// `rows` 覆盖的扫描行、其余桶复用 `old`，combine 后产物字节与整柱
    /// 直呼逐字节相同（tests/sections.rs 锁）；产物经准入后入缓存，供
    /// 下一笔编辑继续增量。`old = None` = 全柱产物化播种笔（字节同直算，
    /// 额外产出缓存，此后该块可走窄行增量）。
    pub fn submit_incremental(
        &self,
        job: MeshJob,
        hi: bool,
        rows: Rows,
        old: Option<ChunkProducts>,
    ) {
        self.spawn(job, hi, Some(rows), old);
    }

    fn spawn(&self, job: MeshJob, hi: bool, rows: Option<Rows>, old: Option<ChunkProducts>) {
        let seq = self.gate.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let tx = self.tx.clone();
        // 闭包持整份 Arc<Gate>：seq 源/pending 计数/地板表同源（Send+Sync）。
        let gate = Arc::clone(&self.gate);
        let run = move || {
            let vol = mcv_core::CHUNK_VOL;
            let slots = neighborhood_slots(&job.voxels, &job.lights);
            debug_assert_eq!(job.voxels.len(), 9 * vol);
            debug_assert_eq!(job.lights.len(), 9 * vol);
            // Mesher 零尺寸 Copy，worker 本地实例；同输入同产物（等价锁
            // tests/mesh_parallel.rs::worker_output_bit_identical 锁定）。
            let mesher = Mesher::new();
            // #172 两路：None = 整柱直算（golden 锁面的 build_mesh 直呼，
            // 零额外拷贝）；Some = 产物化重放 + combine（字节同一直算，
            // sections.rs/tests 双侧锁定），work 为收益观测面。
            let (opaque, water, work, products) = match rows {
                None => {
                    let full = crate::TOTAL_SCANNED_ROWS + crate::SECTION_COUNT;
                    let opaque = mesher.build(&slots, MESH_OPAQUE).ok();
                    let water = mesher.build(&slots, MESH_WATER).ok();
                    (opaque, water, full, None)
                }
                Some(rows) => {
                    let (old_o, old_w) = old.map_or((None, None), |(o, w)| (Some(o), Some(w)));
                    match (
                        mesher.build_products(&slots, MESH_OPAQUE, Some(rows), old_o),
                        mesher.build_products(&slots, MESH_WATER, Some(rows), old_w),
                    ) {
                        (Ok((po, work)), Ok((pw, _))) => {
                            let (mo, mw) = (po.combine(), pw.combine());
                            (Some(mo), Some(mw), work, Some((po, pw)))
                        }
                        // BAD_ARG 不可达（kind 为编译期常量）；真失败退化为
                        // 产物缺失 + 不产缓存，与同步路失败同级别的响亮
                        // 记账在主线程 flush 侧（drain 见 None 会作废缓存）。
                        _ => (
                            None,
                            None,
                            crate::TOTAL_SCANNED_ROWS + crate::SECTION_COUNT,
                            None,
                        ),
                    }
                }
            };
            let done = MeshDone {
                pos: job.pos,
                origin: job.origin,
                hi,
                seq,
                opaque: opaque.map(|m| MeshBytes {
                    vertex: m.vertices,
                    index: m.indices,
                }),
                water: water.map(|m| MeshBytes {
                    vertex: m.vertices,
                    index: m.indices,
                }),
                work,
                products,
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
    }

    /// #172 取该块的分 section 产物缓存（派发方决策用：有缓存才能窄行
    /// 增量；主线程调用，与准入写缓存同线程序——无撕裂风险）。
    pub fn product_of(&self, pos: ChunkPos) -> Option<ChunkProducts> {
        self.gate.products.lock().unwrap().get(&pos).cloned()
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
                // #172 产物缓存准入（锁序恒 floor→products，勿反）：
                // Some = 增量提交，缓存与之同源；None = 直算/退化提交，
                // 任何既有缓存不再与最新快照同源，作废之（保守：下一笔
                // 最多多跑一次播种笔，不会误用陈旧产物）。
                let pos = done.pos;
                match &done.products {
                    Some(p) => {
                        self.gate.products.lock().unwrap().insert(pos, p.clone());
                    }
                    None => {
                        self.gate.products.lock().unwrap().remove(&pos);
                    }
                }
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
        // #172：卸载 = 快照失去唯一真相源，产物缓存一并作废（重载后
        // 首笔必然播种/直算，绝不用跨装载的陈旧产物做增量）。
        self.gate.products.lock().unwrap().remove(&pos);
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

/// #172 `MCV_SUBCHUNK_REMESH` 语义（对齐 `MCV_MESHER_WORKERS=0` 回滚
/// 开关先例）：显式 "0" = 关闭 sub-chunk 增量重网格（编辑回到整柱直算
/// 派发，逐字节原语义）；未设/其余值 = 启用。
pub fn subchunk_enabled_from(v: Option<&str>) -> bool {
    !matches!(v, Some("0"))
}

/// 生产解析：env 每次构造读一次（主线程；同 [`mesh_worker_count`] 纪律）。
pub fn subchunk_enabled() -> bool {
    subchunk_enabled_from(std::env::var("MCV_SUBCHUNK_REMESH").ok().as_deref())
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
