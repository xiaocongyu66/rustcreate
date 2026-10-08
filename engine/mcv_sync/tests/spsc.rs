//! SPSC 环形队列：FIFO、满退避、跨线程搬运、Drop 排空。

use mcv_sync::{Consumer, Producer, Spsc};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn fifo_order_single_thread() {
    let (p, c): (Producer<i32>, Consumer<i32>) = Spsc::new(4).split();
    assert!(c.pop().is_none());
    for i in 0..4 {
        p.push(i).unwrap();
    }
    // 容量 4 已满：第 5 个退回
    assert_eq!(p.push(99), Err(99));
    assert_eq!(p.len(), 4);
    for i in 0..4 {
        assert_eq!(c.pop(), Some(i));
    }
    assert_eq!(c.pop(), None);
    // 腾出空位后可再写
    assert!(p.push(7).is_ok());
    assert_eq!(c.pop(), Some(7));
}

#[test]
fn capacity_rounds_to_power_of_two() {
    let (p, c): (Producer<u8>, Consumer<u8>) = Spsc::new(3).split();
    // 3 → 4
    for i in 0..4 {
        assert!(p.push(i).is_ok());
    }
    assert!(p.push(9).is_err());
    assert_eq!(c.len(), 4);
}

/// 跨线程搬运 100 万递增整数：生产者允许忙等（队列满），
/// 消费者校验严格递增且无缺失——任何数据竞争都会破坏序列或内容。
#[test]
fn cross_thread_million_increments() {
    const N: u64 = 1_000_000;
    let (p, c): (Producer<u64>, Consumer<u64>) = Spsc::new(1024).split();
    let producer = std::thread::spawn(move || {
        for i in 0..N {
            while p.push(i).is_err() {
                std::hint::spin_loop();
            }
        }
    });
    let mut expect = 0u64;
    let mut seen = 0usize;
    while expect < N {
        match c.pop() {
            Some(v) => {
                assert_eq!(v, expect, "乱序/丢失：期望 {expect} 得 {v}");
                expect += 1;
                seen += 1;
                if seen % 100_000 == 0 {
                    std::hint::spin_loop();
                }
            }
            None => std::hint::spin_loop(),
        }
    }
    producer.join().unwrap();
    assert_eq!(expect, N);
}

/// 双端同时高频操作（模拟音频命令通道：一端间歇批量推、一端固定节奏吸）。
#[test]
fn interleaved_stress_no_loss_or_reorder() {
    const N: usize = 200_000;
    let (p, c): (Producer<u32>, Consumer<u32>) = Spsc::new(64).split();
    // 校验线程：严格 0..N 递增
    let validator = std::thread::spawn(move || {
        let mut expect = 0u32;
        while (expect as usize) < N {
            match c.pop() {
                Some(v) => {
                    assert_eq!(v, expect);
                    expect += 1;
                }
                None => std::hint::spin_loop(),
            }
        }
        expect
    });
    let producer = std::thread::spawn(move || {
        for i in 0..N as u32 {
            while p.push(i).is_err() {
                std::thread::yield_now();
            }
        }
    });
    assert_eq!(validator.join().unwrap(), N as u32);
    producer.join().unwrap();
}

/// 队列 Drop 时未消费的 T 必须全部析构（不泄漏）。
#[test]
fn drop_drains_pending_items() {
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    #[derive(Debug)]
    struct Probe;
    impl Probe {
        fn new() -> Self {
            LIVE.fetch_add(1, Ordering::SeqCst);
            Probe
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    {
        let (p, c): (Producer<Probe>, Consumer<Probe>) = Spsc::new(8).split();
        for _ in 0..5 {
            p.push(Probe::new()).unwrap();
        }
        drop(c.pop()); // 消费 1 个
        assert_eq!(LIVE.load(Ordering::SeqCst), 4);
        // p/c 出作用域，剩 4 个应随 Inner::drop 排空
    }
    assert_eq!(LIVE.load(Ordering::SeqCst), 0, "队列 Drop 未排空槽位");
}
