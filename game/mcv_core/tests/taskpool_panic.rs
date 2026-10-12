//! TaskPool panic 隔离与优先级语义锁（E1 / 蓝图 docs/engine-roadmap.md §3 E1、
//! §4 不变量 #9：任务 panic ≠ 池缩编）。
//!
//! 红相位：worker 裸调用 `job()`（旧 pool.rs:79），任务 panic 炸死 worker
//! 线程且池永久缩编、无告警（Drop join Err 被忽略，旧 pool.rs:105）。
//! panic 用例在修复前红（30s recv_timeout 超时失败，不挂死 CI）、修复后全绿。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use mcv_core::pool::{PoolStats, TaskPool};

const WAIT: Duration = Duration::from_secs(30);

/// 轮询等待池计数收敛（任务完成后的 running 递减发生在任务体之后，完成
/// 信号无法先于计数递减，只能有界轮询；任务均为平凡任务，收敛在毫秒级）。
fn wait_until(pool: &TaskPool, want: PoolStats) {
    let deadline = Instant::now() + WAIT;
    loop {
        if pool.stats() == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "池计数未收敛: {:?}（期望 {want:?}）",
            pool.stats()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 任务 panic 后 worker 必须存活并继续取任务：panic 任务不得影响后续 hi/lo
/// 任务执行，且 `stats().workers` 不减员（不变量 #9）。单 worker 池让
/// 「缩编 = 全停」最大化可见；recv_timeout 保证旧实现下按超时失败而非挂死。
#[test]
fn pool_keeps_scheduling_after_panic() {
    let pool = TaskPool::new(1);
    pool.spawn_hi(Box::new(|| panic!("taskpool panic isolation probe")));
    let (hi_tx, hi_rx) = mpsc::channel::<&'static str>();
    let (lo_tx, lo_rx) = mpsc::channel::<&'static str>();
    pool.spawn_hi(Box::new(move || hi_tx.send("hi-after-panic").unwrap()));
    pool.spawn_lo(Box::new(move || lo_tx.send("lo-after-panic").unwrap()));
    // 旧实现：唯一 worker 死于 panic，池永久缩编，后续任务无人消费。
    assert_eq!(
        hi_rx.recv_timeout(WAIT).ok(),
        Some("hi-after-panic"),
        "worker 死于任务 panic，池永久缩编（蓝图 §4 不变量 #9）"
    );
    assert_eq!(
        lo_rx.recv_timeout(WAIT).ok(),
        Some("lo-after-panic"),
        "panic 后 lo 任务也必须被执行"
    );
    // 满编保留：worker 只在 shutdown 退出时减员，panic 不减。
    assert_eq!(
        pool.stats().workers,
        1,
        "任务 panic 导致池缩编（蓝图 §4 不变量 #9）"
    );
}

/// 在途/排队/worker 占用三计数（E0 的任务侧数据源；只计数、不改调度）。
/// 双 worker 各自停在 gate 任务内：running=2、queued=3、workers=2 的快照
/// 完全确定（出队与计数增减在同一把锁窗口内；gate 释放后排空归零）。
#[test]
fn stats_count_running_queued_and_workers() {
    let pool = TaskPool::new(2);
    let (gate_tx1, gate_rx1) = mpsc::channel::<()>();
    let (gate_tx2, gate_rx2) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    for gate_rx in [gate_rx1, gate_rx2] {
        let ready = ready_tx.clone();
        pool.spawn_hi(Box::new(move || {
            ready.send(()).expect("ready 发送失败");
            let _ = gate_rx.recv(); // gate：worker 停在此处占住 running
        }));
    }
    drop(ready_tx);
    for _ in 0..3 {
        pool.spawn_hi(Box::new(move || { /* 纯排队任务 */ }));
    }
    // 两个 worker 都已到达 gate：此刻 running/queued/workers 精确确定。
    assert_eq!(ready_rx.recv_timeout(WAIT).ok(), Some(()));
    assert_eq!(ready_rx.recv_timeout(WAIT).ok(), Some(()));
    assert_eq!(
        pool.stats(),
        PoolStats {
            running: 2,
            queued: 3,
            workers: 2,
        },
        "三计数快照应精确（在途=busy worker、排队=未出队、满编=2）"
    );
    gate_tx1.send(()).expect("gate 释放失败");
    gate_tx2.send(()).expect("gate 释放失败");
    drop(gate_tx1);
    drop(gate_tx2);
    // 全部排空后在途/排队归零，worker 满编不减。
    wait_until(
        &pool,
        PoolStats {
            running: 0,
            queued: 0,
            workers: 2,
        },
    );
}

/// 双级优先队列既有语义补锁：hi 先于 lo 排空。1 worker 串行化出队序，
/// gate 任务保证全部提交完成后才开始消费，消除「提交竞态先于出队」的
/// 时序面——单 worker 下执行序 == 出队序，断言完全确定。
#[test]
fn hi_drains_before_lo() {
    let pool = TaskPool::new(1);
    let (gate_tx, gate_rx) = mpsc::channel::<()>();
    pool.spawn_hi(Box::new(move || {
        let _ = gate_rx.recv(); // gate：等主线程把全部任务提交完
    }));
    let (tx, rx) = mpsc::channel::<u8>();
    for v in [1u8, 2] {
        let tx = tx.clone();
        pool.spawn_lo(Box::new(move || {
            let _ = tx.send(v);
        }));
    }
    for v in [3u8, 4] {
        let tx = tx.clone();
        pool.spawn_hi(Box::new(move || {
            let _ = tx.send(v);
        }));
    }
    drop(tx);
    gate_tx.send(()).expect("gate 任务丢失");
    let got: Vec<u8> = (0..4)
        .map(|_| rx.recv_timeout(WAIT).expect("任务未被执行"))
        .collect();
    assert_eq!(got, vec![3, 4, 1, 2], "hi 队列必须先于 lo 队列排空");
}
