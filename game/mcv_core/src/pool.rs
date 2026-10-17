//! Worker pool: 本仓调度纪律（双级优先队列、三计数、panic 隔离）+
//! rayon 执行端（线程生命周期与 work stealing 全归 rayon，零手写
//! worker 线程循环）。
//!
//! 蓝图 docs/engine-roadmap.md §3 E1 / §4 不变量 #9：任务 panic ≠ 池缩编——
//! 任务执行包 [`run_isolated`]（`catch_unwind`），panic 记 error 日志后任务
//! 体正常返回、rayon worker 存活继续取任务；池暴露 running/queued/workers
//! 三计数（`stats()`，E0 的任务侧数据源）。
//! 并发选型（用户拍板 2026-10-17「rayon 全量替换自研 TaskPool」，覆盖蓝图
//! §1.1/§3 E1「不引入 rayon」旧决策）：**执行端换代，纪律不动摇**——
//! 手写 Mutex+Condvar worker 线程循环拆除，rayon `ThreadPool` 出线程；
//! 本仓三调度器（terrain/mesh/light）赖以正确性的三件套全部保留：
//! ① hi/lo 双级优先出队（rayon 无优先级语义，由本模块「名额门 + 优先
//! 队列」在派发侧实现：在途 < 线程数才向 rayon 投递，队列 pop 纪律 =
//! 旧池出队序——E3 编辑源块优先通道的语义锁 tests/taskpool_panic.rs
//! `hi_drains_before_lo` 原样守住）；② 三计数增减在同一把锁窗口内
//! （`stats()` 快照自洽）；③ 任务级 panic 隔离（rayon 派发任务若裸
//! panic 会沿 work-stealing 树炸穿 worker——`run_isolated` 在闭包内
//! 接住，隔离语义同旧池）。

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use crate::ChunkPos;

pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// 池遥测快照（只计数、不改调度；计数不是调度正确性的依据）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// 在途：已派发进 rayon、执行中（含 panic 隔离执行中）的任务数。
    pub running: usize,
    /// 排队：已入队、尚未过名额门（未派发进 rayon）的任务数。
    pub queued: usize,
    /// worker 占用：rayon 池线程数（常驻满编——任务 panic 不减员，
    /// 不变量 #9 由任务级 `catch_unwind` 保障）。
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

    pub fn len(&self) -> usize {
        self.hi.len() + self.lo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hi.is_empty() && self.lo.is_empty()
    }
}

/// 名额门状态：优先队列 + 在途计数 + shutdown 位。**增减全部在同一把锁
/// 窗口内**（入队派发点 / 任务完成收尾点各持锁一次）——`stats()` 快照
/// 自洽的泉源（旧池同纪律：出队与计数同窗口）。
struct PoolState {
    queue: PriorityQ<Job>,
    /// 已派发进 rayon、尚未归还（执行中或正收尾记账）的任务数。
    running: usize,
    /// shutdown（`Drop` 落地即置位）：此后入队任务就地丢弃（旧池 shutdown
    /// 后 worker 先查位再取任务的同语义）。
    closed: bool,
}

/// 共享给执行闭包的池芯：rayon 池 + 名额门 + 完成等待交通灯。
struct Shared {
    pool: rayon::ThreadPool,
    threads: usize,
    state: Mutex<PoolState>,
    /// Drop 等待在途任务归还（running==0）的交通灯。
    idle: Condvar,
}

/// Fixed-size worker pool (rayon-backed)。Jobs are `FnOnce + Send`;
/// high-priority jobs are drained before low-priority ones. 任务 panic 被
/// 隔离：记日志后任务体正常返回。**panic 不得裸传播进 rayon**——spawn 的
/// 任务 panic 在 rayon 无回接点，会炸 worker（旧「池永久缩编」教训的
/// rayon 变体），一切执行都过 [`run_isolated`]。
pub struct TaskPool {
    shared: Arc<Shared>,
}

impl TaskPool {
    pub fn new(workers: usize) -> Self {
        let n = workers.max(1);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .expect("rayon worker pool build failed");
        let shared = Arc::new(Shared {
            pool,
            threads: n,
            state: Mutex::new(PoolState {
                queue: PriorityQ::default(),
                running: 0,
                closed: false,
            }),
            idle: Condvar::new(),
        });
        Self { shared }
    }

    pub fn spawn_hi(&self, job: Job) {
        self.enqueue(true, job);
    }

    pub fn spawn_lo(&self, job: Job) {
        self.enqueue(false, job);
    }

    /// 入队 + 名额门内派发（出队、计数增减、向 rayon 投递在同一把锁窗口）。
    fn enqueue(&self, hi: bool, job: Job) {
        let mut st = self.shared.state.lock().unwrap();
        if st.closed {
            // shutdown 后丢弃：镜像旧池「shutdown 位先查，不入队不执行」。
            return;
        }
        if hi {
            st.queue.push_hi(job);
        } else {
            st.queue.push_lo(job);
        }
        Self::pump_locked(&self.shared, &mut st);
    }

    /// 名额门：在途 < 线程数时按 hi 先 lo 后持续向 rayon 派发。持锁调用
    /// （入队点 / 任务完成 tail 各一次——任务完成必回填，名额空出即有用，
    /// 饥饿不发生）。执行闭包体 = [`run_isolated`] + 归还记账 + tail 回填；
    /// 闭包首次行动不碰门锁（先跑任务），无锁重入风险。
    fn pump_locked(shared: &Arc<Shared>, st: &mut PoolState) {
        while st.running < shared.threads {
            let Some(job) = st.queue.pop() else {
                break;
            };
            st.running += 1;
            let shared = Arc::clone(shared);
            shared.pool.spawn(move || {
                run_isolated(job);
                let mut st = shared.state.lock().unwrap();
                st.running -= 1;
                if !st.closed {
                    TaskPool::pump_locked(&shared, &mut st);
                }
                drop(st);
                shared.idle.notify_all();
            });
        }
    }

    /// 在途/排队/worker 占用计数快照（E0 任务侧数据源；只读，不改调度）。
    pub fn stats(&self) -> PoolStats {
        let st = self.shared.state.lock().unwrap();
        PoolStats {
            running: st.running,
            queued: st.queue.len(),
            workers: self.shared.threads,
        }
    }
}

impl Drop for TaskPool {
    fn drop(&mut self) {
        // ① 关门 + 丢弃未派发队列（旧池 shutdown 后 worker 不再取余量的同
        // 语义）；② 等在途任务全部归还（running==0，交通灯）；③ 等共享芯
        // 的 Arc 只剩本持有者——已归还的执行闭包可能还差最后几条指令才
        // 释放它的 Arc 克隆，若放任 Shared（内含 rayon 池）在 rayon worker
        // 线程内析构 = worker 内拆自身线程树（join 自身）。观察到克隆全
        // 释放后，Shared 的析构必然落在调用方线程：闭包侧已无持有者，不
        // 可能再从 worker 侧释放；且 running==0 && closed 时尾回填不再
        // 派发新闭包，计数不会再涨。
        let mut st = self.shared.state.lock().unwrap();
        st.closed = true;
        let _dropped = std::mem::take(&mut st.queue);
        drop(_dropped);
        while st.running > 0 {
            st = self.shared.idle.wait(st).unwrap();
        }
        while Arc::strong_count(&self.shared) > 1 {
            drop(st);
            std::thread::yield_now();
            st = self.shared.state.lock().unwrap();
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
/// panic 会炸死执行线程（rayon worker 同此）→ 池永久静默缩编（加载洪峰
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
