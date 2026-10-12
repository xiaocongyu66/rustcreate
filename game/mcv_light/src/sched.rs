//! E5 光照 init 并行调度（蓝图 docs/engine-roadmap.md §3 E5）：init 双通道
//! BFS 从主线程（2 块/帧串行）移入 worker 池——快照-计算-回传三段式的
//! 「计算」与「回传」两端，结构对齐 E3 先例 `mcv_mesher::MeshScheduler`
//! （seq 守卫 / 提交地板 / 积压护栏 / 近优先派发在上层 game.rs）。
//!
//! - **计算端** [`LightScheduler`]：主线程把 voxels+heightmap 快照字节
//!   （[`LightJob`]，~197 KiB/笔）投入专属 `TaskPool`，worker 只见字节、
//!   无共享可变状态，调 [`crate::init`]（sky+block 双通道 BFS，引擎本体
//!   零改动）后把整份光照字节（98304 B）+ 边界脏掩码经 mpsc 回传。
//!   **光照字节不进任务**：`init` 的第一步就是 `chunk.light.fill(0)`
//!   （lib.rs:355），光源与传播完全由 voxels 决定——job 只带 voxels+
//!   heightmap，worker 本地起零光照缓冲（每笔省 98 KiB 拷贝 + 98 KiB
//!   线上字节）。写回 = 主线程整缓冲覆盖：派发与写回之间没有任何光照
//!   写入方（`update_block`/边同步都门在 ≥LightLocalReady，game.rs
//!   relight_block_edit :411），覆盖即权威。
//! - **回传端** [`LightResults`]：主线程侧取产物；按「提交地板」拒收
//!   僵尸产物（区块已卸载——防把光写进已出环的旧键，卸载→重进环后旧
//!   快照不得覆盖新读盘内容）。**边同步（`sync_light_edges`）不经过本
//!   模块**——跨块边同步保持主线程串行（蓝图铁纪律：LIFO push→pull 序、
//!   REMOVE-before-ADD 两相、in-flight 去重、512 步预算、两端
//!   LightLocalReady 门五项时序假设全部建立在「边任务按确定序在同一线
//!   程消化」之上，game.rs:358-439）。
//!
//! 原版同构（光照引擎跑在服务器后台执行线程）：ChunkMap 持
//! `ThreadedLevelLightEngine`（ChunkMap.java:197），其任务经
//! `ChunkTaskDispatcher` 投入 `Util.backgroundExecutor()` 线程池
//! （ChunkMap.java:194-196、MinecraftServer.java:352）；单块布光
//! `lightChunk` = PRE_UPDATE 入队 `propagateLightSources(pos)` +
//! POST_UPDATE 回执 `setLightCorrect(true)`（ThreadedLevelLightEngine.
//! java:164-180）——即「计算在后台、完成回执回主链路」的两段式；
//! 按块优先级出队（ChunkTaskPriorityQueue.java:63-78 pop 恒取最低票
//! 级别 = 玩家近优先；本仓近优先环序写回在上层 `sorted_keys`）。
//!
//! 回滚开关：`MCV_LIGHT_WORKERS=0` → 上层不装配本模块，整条走 E5 前的
//! 主线程串行 init（对齐 `MCV_MESHER_WORKERS` 先例，mcv_mesher/src/
//! sched.rs `worker_count_from`）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mcv_core::ChunkPos;
use mcv_core::pool::TaskPool;

/// 主线程 → worker 的光照 init 任务：voxels+heightmap 快照裸字节
/// （布局 `mcv_core::vidx` / `(z<<4)|x`，与串行路径同源装配）。
/// 光照字节不在任务里——见模块文档（`init` 自零化，worker 本地起零缓冲）。
pub struct LightJob {
    pub pos: ChunkPos,
    pub voxels: Vec<u16>,
    pub heightmap: Vec<i16>,
}

/// worker → 主线程的光照 init 产物。
pub struct LightDone {
    pub pos: ChunkPos,
    /// 派发序号（调度器内单调递增；提交地板过滤的依据）。
    pub seq: u64,
    /// 整份光照字节（98304 = CHUNK_VOL；低 nibble 块光 / 高 nibble 天光）。
    pub light: Vec<u8>,
    /// `init` 的边界脏掩码（bits 0..3 = +X,-X,+Z,-Z）。串行路径本就弃用
    /// 该返回值（跨块对齐由成对边标记驱动，game.rs border pairing），随
    /// 产物带回仅供测试/取证对拍，不参与写回决策。
    pub border_mask: u8,
}

/// 提交地板（调度器与回传端共享）：seq 单调源 + per-pos 拒收线 + 在途
/// 同键记账（单键单笔——上层派发以 `in_flight` 为门，同键重复投递即
/// 上层记账 bug，debug_assert 兜底）。
struct Gate {
    seq: AtomicU64,
    /// 已完成、滞留 mpsc 的产物数（测试观测面；非调度依据）。
    pending: AtomicU64,
    /// per-pos 提交地板：seq ≤ 地板的产物拒收。kill（卸载）抬到当前
    /// seq（杀全部在途）。
    floor: Mutex<HashMap<ChunkPos, u64>>,
    /// 在途记账：pos → 派发 seq。产物回传（无论获准与否）且 seq 相符才
    /// 解除——防陈旧产物误清新派发的在途位。
    inflight: Mutex<HashMap<ChunkPos, u64>>,
}

/// 计算端：专属 worker 池 + 产物通道发送端。
pub struct LightScheduler {
    pool: TaskPool,
    tx: std::sync::mpsc::Sender<LightDone>,
    gate: Arc<Gate>,
    /// 装配时的 worker 数（派发预算的倍增基数；池遥测另有 stats()）。
    workers: usize,
    /// 背压高水位（在途排队+执行 ≥ 此值时 [`Self::saturated`]）。预算
    /// = 2×workers/帧，积压护水量给满一帧派发量 + 2 的余量。
    high_water: usize,
}

impl LightScheduler {
    /// 装配计算端与回传端。`workers` 由上层按 `MCV_LIGHT_WORKERS` 解析
    /// （0 = 同步路径，不进本函数）。
    pub fn new(workers: usize) -> (Self, LightResults) {
        let pool = TaskPool::new(workers);
        let (tx, rx) = std::sync::mpsc::channel();
        let high_water = 2 * workers + 2;
        let gate = Arc::new(Gate {
            seq: AtomicU64::new(0),
            pending: AtomicU64::new(0),
            floor: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
        });
        (
            Self {
                pool,
                tx,
                gate: Arc::clone(&gate),
                workers,
                high_water,
            },
            LightResults { rx, gate },
        )
    }

    /// 投递一笔光照 init 任务。seq 在投递时点分配——「拿到快照的那一刻」
    /// 的派发序（回传序不保证与派发序一致，多 worker 并行完成乱序；写回
    /// 环序由上层 `sorted_keys` 重排，见 game.rs `light_writeback`）。
    pub fn submit(&self, job: LightJob) {
        let pos = job.pos;
        let seq = self.gate.seq.fetch_add(1, Ordering::Relaxed) + 1;
        debug_assert!(
            !self.in_flight(&pos),
            "同键重复派发（上层 in_flight 门失效）：{pos:?}"
        );
        self.gate.inflight.lock().unwrap().insert(pos, seq);
        let tx = self.tx.clone();
        // 闭包持整份 Arc<Gate>：seq 源/pending 计数/地板与在途表同源
        // （Send+Sync）。任务字节（voxels/heightmap）随闭包移入 worker，
        // 光照缓冲 worker 本地起零（见模块文档）。
        let gate = Arc::clone(&self.gate);
        let run = move || {
            let mut light = vec![0u8; mcv_core::CHUNK_VOL];
            let mut view = crate::LightChunk {
                voxels: &job.voxels,
                light: &mut light,
                heightmap: &job.heightmap,
            };
            let border_mask = crate::init(&mut view);
            let done = LightDone {
                pos,
                seq,
                light,
                border_mask,
            };
            if tx.send(done).is_ok() {
                gate.pending.fetch_add(1, Ordering::Relaxed);
            }
        };
        self.pool.spawn_lo(Box::new(run));
    }

    /// 该块是否有一笔任务在途（排队或执行中）。上层派发门：TerrainReady
    /// 且不在途才投递，防同键双发。
    pub fn in_flight(&self, pos: &ChunkPos) -> bool {
        self.gate.inflight.lock().unwrap().contains_key(pos)
    }

    /// 装配时的 worker 数（派发预算倍增基数；池占用另有 stats()）。
    pub fn workers(&self) -> usize {
        self.workers
    }

    /// 背压门：在途（排队+执行）≥ 高水位时为真——派发方应停投（不耗
    /// 预算，下帧预算流自动重试），防 worker 饥饿时快照（~197 KiB/笔）
    /// 与产物（~98 KiB/笔）字节无界堆积。峰值在途 ≈ 2×workers+2 笔。
    pub fn saturated(&self) -> bool {
        let s = self.pool.stats();
        s.queued + s.running >= self.high_water
    }

    /// 派发预算（块/帧）：串行基线 2 块/帧 × workers——加载期布光波推进
    /// 速度 ≈ worker 数倍（蓝图 E5 目标）；实际推进由积压护栏自限
    /// （worker 慢时 `saturated` 截停派发）。
    pub fn dispatch_budget(&self) -> usize {
        2 * self.workers
    }

    /// 池遥测快照（`mcv_core::pool::PoolStats` 同源：在途/排队/worker 占用）。
    pub fn stats(&self) -> mcv_core::pool::PoolStats {
        self.pool.stats()
    }
}

/// 回传端：主线程侧产物取用（**只在主线程调用**；写回与
/// `advance_to(LightLocalReady)` 在上层 game.rs，边同步不经本模块）。
pub struct LightResults {
    rx: std::sync::mpsc::Receiver<LightDone>,
    gate: Arc<Gate>,
}

/// 地板修剪龄：seq 落后当前 seq 超过此值的地板条目可删——在途任务数被
/// 背压门压在 high_water 内，更旧的 seq 不可能再有在途产物（余量 4 倍
/// + 常数，测试侧无背压门也有界）。同 E3 `FLOOR_PRUNE_AGE`。
const FLOOR_PRUNE_AGE: u64 = 256;

impl LightResults {
    /// 取走全部**获准套用**的产物：僵尸（seq ≤ kill 地板）在此拒收并
    /// 就地弃置（光照字节对已卸载键无意义）。每次取走后按龄修剪地板表
    /// （有界内存）；获准与否都解除在途记账（seq 相符时——陈旧产物不得
    /// 误清新派发的在途位）。
    pub fn drain(&mut self) -> Vec<LightDone> {
        let mut out = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            self.gate.pending.fetch_sub(1, Ordering::Relaxed);
            // 在途记账解除（seq 相符才动——kill 后重派发的新任务在途位
            // 不被旧产物清掉）。
            {
                let mut inflight = self.gate.inflight.lock().unwrap();
                if inflight.get(&done.pos) == Some(&done.seq) {
                    inflight.remove(&done.pos);
                }
            }
            let mut floor = self.gate.floor.lock().unwrap();
            let admit = done.seq > *floor.get(&done.pos).unwrap_or(&0);
            if admit {
                floor.insert(done.pos, done.seq);
                out.push(done);
            }
            // 拒收分支：产物就地 drop（~98 KiB 字节随之释放）。
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
    /// 产物（seq ≤ 当前）拒收，防把光照写进已卸载键 / 卸载后重进环
    /// （读盘内容可能已不同）时旧快照覆盖新数据。在途位同点解除；重进
    /// 视野后的新派发 seq 更高，不受影响。
    pub fn kill(&mut self, pos: ChunkPos) {
        let cur = self.gate.seq.load(Ordering::Relaxed);
        self.gate.floor.lock().unwrap().insert(pos, cur);
        self.gate.inflight.lock().unwrap().remove(&pos);
    }

    /// 已完成、滞留通道的产物数（测试观测面；非消费、非调度依据）。
    #[doc(hidden)]
    pub fn pending_done(&self) -> usize {
        self.gate.pending.load(Ordering::Relaxed) as usize
    }

    /// 取走全部原始产物（不过滤；测试取证面——生产路径只走 [`Self::drain`]）。
    #[doc(hidden)]
    pub fn drain_all(&mut self) -> Vec<LightDone> {
        let mut out = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            self.gate.pending.fetch_sub(1, Ordering::Relaxed);
            // 在途记账同步解除（seq 相符时；同 [`Self::drain`]）。
            let mut inflight = self.gate.inflight.lock().unwrap();
            if inflight.get(&done.pos) == Some(&done.seq) {
                inflight.remove(&done.pos);
            }
            out.push(done);
        }
        out
    }
}

/// `MCV_LIGHT_WORKERS` 语义（回滚开关）：未设 → `default`（world 默认池
/// 策略）；"0" → 0（主线程串行 init 锚点，回滚开关）；正整数 → 该值；
/// 其余一切（非法/空/负/带空白）→ `default`（0 只能显式要求，对齐
/// `Integer.parseInt` 不 trim 语义，同 mcv_mesher::worker_count_from
/// 的理由纯函数化）。
pub fn worker_count_from(v: Option<&str>, default: usize) -> usize {
    match v {
        None => default,
        Some(s) => s.parse::<usize>().unwrap_or(default),
    }
}

/// 生产解析：env 每次构造时读一次（主线程；后台线程不反复读 env，同
/// `TerrainScheduler::new` 的后端固定纪律）。默认对齐 world 任务池策略
/// `mcv_core::world_worker_count()`。
pub fn light_worker_count() -> usize {
    worker_count_from(
        std::env::var("MCV_LIGHT_WORKERS").ok().as_deref(),
        mcv_core::world_worker_count(),
    )
}
