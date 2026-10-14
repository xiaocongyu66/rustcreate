//! Worker pool with a two-level priority queue (no external deps).
//!
//! 蓝图 docs/engine-roadmap.md §3 E1 / §4 不变量 #9：任务 panic ≠ 池缩编——
//! 任务执行包 `catch_unwind`，panic 记 error 日志后 worker 存活继续取任务；
//! 池暴露 running/queued/workers 三计数（`stats()`，E0 的任务侧数据源）。
//! 并发选型总决策（蓝图 §3 E1）：不引入 rayon/tokio-util，手写
//! Mutex+Condvar 池不动摇——本仓并发单位是「一个区块一个任务」（~10ms
//! 粒度），需要的是确定性纪律与可观测性，不是调度器换代。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::ChunkPos;

pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// 池遥测快照（只计数、不改调度；计数不是调度正确性的依据）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// 在途：已出队、执行中（含 panic 隔离执行中）的任务数。
    pub running: usize,
    /// 排队：已入队、尚未出队（hi+lo 合计）的任务数。
    pub queued: usize,
    /// worker 占用：存活 worker 线程数——任务 panic 不减员（不变量 #9），
    /// 仅 shutdown 退出时递减。
    pub workers: usize,
}

pub struct PriorityQ<T> {
    hi: VecDeque<T>,
    lo: VecDeque<T>,
}

impl<T> Default for PriorityQ<T> {
    fn default() -> Self {
        Self {
            hi: VecDeque::new(),
            lo: VecDeque::new(),
        }
    }
}

impl<T> PriorityQ<T> {
    pub fn push_hi(&mut self, v: T) {
        self.hi.push_back(v);
    }

    pub fn push_lo(&mut self, v: T) {
        self.lo.push_back(v);
    }

    pub fn pop(&mut self) -> Option<T> {
        self.hi.pop_front().or_else(|| self.lo.pop_front())
    }

    pub fn is_empty(&self) -> bool {
        self.hi.is_empty() && self.lo.is_empty()
    }
}

struct Shared {
    queue: Mutex<PriorityQ<Job>>,
    signal: Condvar,
    shutdown: AtomicBool,
    // 三计数器（telemetry only，Relaxed 即可；与 queue 锁同点增减，供
    // stats() 与 E0 分段计时读取）。
    running: AtomicUsize,
    queued: AtomicUsize,
    workers: AtomicUsize,
}

/// Fixed-size worker pool. Jobs are `FnOnce + Send`; high-priority jobs are
/// drained before low-priority ones. 任务 panic 被隔离：worker 记日志后存活。
pub struct TaskPool {
    shared: Arc<Shared>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl TaskPool {
    pub fn new(workers: usize) -> Self {
        let n = workers.max(1);
        let shared = Arc::new(Shared {
            queue: Mutex::new(PriorityQ::default()),
            signal: Condvar::new(),
            shutdown: AtomicBool::new(false),
            running: AtomicUsize::new(0),
            queued: AtomicUsize::new(0),
            workers: AtomicUsize::new(0),
        });
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let shared = Arc::clone(&shared);
            handles.push(std::thread::spawn(move || {
                shared.workers.fetch_add(1, Ordering::Relaxed);
                loop {
                    let job = {
                        let mut q = shared.queue.lock().unwrap();
                        loop {
                            if shared.shutdown.load(Ordering::Acquire) {
                                shared.workers.fetch_sub(1, Ordering::Relaxed);
                                return;
                            }
                            if let Some(job) = q.pop() {
                                // 出队与计数在同一把锁窗口内增减（锁外只余
                                // running 完成递减），stats() 读到的快照自洽。
                                shared.queued.fetch_sub(1, Ordering::Relaxed);
                                shared.running.fetch_add(1, Ordering::Relaxed);
                                break job;
                            }
                            q = shared.signal.wait(q).unwrap();
                        }
                    };
                    run_isolated(job);
                    shared.running.fetch_sub(1, Ordering::Relaxed);
                }
            }));
        }
        Self {
            shared,
            workers: handles,
        }
    }

    pub fn spawn_hi(&self, job: Job) {
        {
            let mut q = self.shared.queue.lock().unwrap();
            q.push_hi(job);
            self.shared.queued.fetch_add(1, Ordering::Relaxed);
        }
        self.shared.signal.notify_one();
    }

    pub fn spawn_lo(&self, job: Job) {
        {
            let mut q = self.shared.queue.lock().unwrap();
            q.push_lo(job);
            self.shared.queued.fetch_add(1, Ordering::Relaxed);
        }
        self.shared.signal.notify_one();
    }

    /// 在途/排队/worker 占用计数快照（E0 任务侧数据源；只读，不改调度）。
    pub fn stats(&self) -> PoolStats {
        PoolStats {
            running: self.shared.running.load(Ordering::Relaxed),
            queued: self.shared.queued.load(Ordering::Relaxed),
            workers: self.shared.workers.load(Ordering::Relaxed),
        }
    }
}

/// worker 任务的结局指令（`#[doc(hidden)]` 测试注入面）：任务闭包在真正
/// 干活前查询钩子，按模式演绎三种结局。生产装配恒不装钩子（查询短路为
/// [`LossMode::Deliver`]，零成本分支）。
///
/// 统一命名面（三个调度器 mesh/light/terrain 共用本类型）：`Deliver` =
/// 正常干活回传；`Swallow` = 任务静默蒸发（**不产生任何回执**——回执通道
/// 本身丢失的极端面，只有主线程侧看门狗超时能兜底，mesh 专用回归注入）；
/// `Panic` = 任务闭包 panic（池隔离存活 + 调度器 catch_unwind 合成失败
/// 回执——DC-3 范本，docs/worker-accounting.md）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum LossMode {
    Deliver,
    Swallow,
    Panic,
}

/// 丢失注入钩子（测试装配）：按区块坐标决定该笔任务的结局。
#[doc(hidden)]
pub type LossHook = Box<dyn Fn(ChunkPos) -> LossMode + Send + Sync>;

/// 执行单个任务并做 panic 隔离（蓝图 §4 不变量 #9：任务 panic ≠ 池缩编）。
/// 旧实现裸调用 job()，panic 会炸死 worker 线程、池永久静默缩编（加载洪峰
/// 无声变慢、零告警）。panic 经默认 hook 打印位置信息（Android 由
/// mcv_app/src/lib.rs 的 hook 写 logcat），这里再记一条池侧 error：worker
/// 存活、满编保留。`AssertUnwindSafe`：Job 是 owned 闭包，池不承诺任务
/// panic 后仍处于一致共享状态——隔离的目标是 worker 线程存活，不是任务重试。
///
/// **池语义（为什么这里不管产物）**：隔离保证 worker 线程存活，**不保证
/// 任务产物**——被吞的 panic 不产生任何回执，任何依赖「每笔派发必有回执」
/// 推进记账的消费者（在途表/配额/网格记账）必须在闭包内自带 catch_unwind
/// 合成失败回执（DC-3 范本 = game.rs `spawn_io_worker` 的
/// `failed_io_done`；三调度器 mesh/light/terrain 均已照做），否则记账
/// 永真、重派门永久关闭——真机隐形墙（mesh）/黑块（light）/生成停摆
/// （terrain）三案同根，教训见 docs/worker-accounting.md。
fn run_isolated(job: Job) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        log::error!("[taskpool] task panicked; worker survives, pool stays full: {msg}");
    }
}

impl Drop for TaskPool {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.signal.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

/// 原版 `max.bg.threads` 上限默认值：属性未设/非法时 `getMaxThreads()` 回落
/// 255（Util.java:190-206；属性仅接受 1..=255，非法值记 error 后走默认）。
const MAX_BG_THREADS_DEFAULT: usize = 255;

/// 原版 `Util.maxAllowedExecutorThreads()`（Util.java:187-189）：
/// `clamp(available_processors - 1, 1, max.bg.threads)`。`max_bg_threads` 为
/// 属性值镜像（None = 未设）。纯函数便于语义单测直接驱动，不动真实环境
/// （env 读写在并行测试间有数据竞争，同 mcv_worldgen/src/lib.rs:43 的理由）。
fn worker_count_for(cpu: usize, max_bg_threads: Option<&str>) -> usize {
    let cap = max_bg_threads
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| (1..=255).contains(v))
        .unwrap_or(MAX_BG_THREADS_DEFAULT);
    cpu.saturating_sub(1).clamp(1, cap)
}

/// Default worker count for world tasks, aligned with vanilla
/// `Util.maxAllowedExecutorThreads()`（`/root/mc-ref/src-26.1/net/minecraft/
/// util/Util.java:187-189`）：`clamp(available_processors - 1, 1,
/// max.bg.threads)`。替代旧策略 `cpu - 2, clamp(2, 6)`（pool.rs 旧 111-115）：
/// 6 核上限在大核机上过保守（蓝图 §3 E1）。env 覆盖位 `MCV_POOL_WORKERS`
/// 镜像原版系统属性 `max.bg.threads` 的上限语义（1..=255 有效，非法值回落
/// 默认 255，Util.java:190-206）。
pub fn world_worker_count() -> usize {
    let cap = std::env::var("MCV_POOL_WORKERS").ok();
    std::thread::available_parallelism()
        .map(|n| worker_count_for(n.get(), cap.as_deref()))
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::worker_count_for;

    /// 原版池策略逐点锁（Util.java:187-189）：clamp(cpu−1, 1, max.bg.threads)。
    #[test]
    fn worker_count_matches_vanilla_policy() {
        assert_eq!(worker_count_for(1, None), 1);
        assert_eq!(worker_count_for(2, None), 1);
        assert_eq!(worker_count_for(4, None), 3);
        assert_eq!(worker_count_for(8, None), 7);
        assert_eq!(worker_count_for(16, None), 15);
        // max.bg.threads / MCV_POOL_WORKERS 上限语义。
        assert_eq!(worker_count_for(8, Some("3")), 3);
        assert_eq!(worker_count_for(2, Some("8")), 1);
        // 非法值回落默认 255（Util.java:200-205 语义：记 error 后走默认）。
        assert_eq!(worker_count_for(8, Some("0")), 7);
        assert_eq!(worker_count_for(8, Some("256")), 7);
        assert_eq!(worker_count_for(8, Some("abc")), 7);
        assert_eq!(worker_count_for(8, Some(" 4 ")), 7); // 不 trim，同 Integer.parseInt
    }
}
