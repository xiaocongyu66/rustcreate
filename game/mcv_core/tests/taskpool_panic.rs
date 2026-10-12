//! TaskPool panic 隔离与优先级语义锁（E1 / 蓝图 docs/engine-roadmap.md §3 E1、
//! §4 不变量 #9：任务 panic ≠ 池缩编）。
//!
//! 现状（红相位）：worker 裸调用 `job()`（pool.rs:79），任务 panic 炸死 worker
//! 线程且池永久缩编、无告警（Drop join Err 被忽略，pool.rs:105）。本组测试在
//! 修复前必须红（panic 用例），修复后全绿。

use std::sync::mpsc;
use std::time::Duration;

use mcv_core::TaskPool;

const WAIT: Duration = Duration::from_secs(30);

/// 任务 panic 后 worker 必须存活并继续取任务：panic 任务不得影响后续 hi/lo
/// 任务执行（E1 catch_unwind 的直接红驱动）。单 worker 池让「缩编 = 全停」
/// 最大化可见；recv_timeout 保证旧实现下按超时失败而非挂死 CI。
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
