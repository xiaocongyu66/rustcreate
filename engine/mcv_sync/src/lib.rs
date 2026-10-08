//! 无锁 SPSC（单生产者-单消费者）有界环形队列。
//!
//! Lamport 环形队列：头尾各归一端原子、槽位每圈至多一写一读，
//! 全程无锁自旋；容量在 [`Spsc::new`] 一次性分配（向上取 2 的幂），
//! push/pop 路径零分配——适合实时线程（音频回调）与游戏线程之间的
//! 命令通道。跨端同步全靠 acquire/release：生产者发布槽位写入用
//! Release 存 head，消费者可见后用 Acquire 读；反向同理于 tail。
//!
//! 用法：`Spsc::new(cap)` → `split()` 得 [`Producer`]/[`Consumer`]
//! 两个句柄，各自 `Send` 到目标线程；句柄 `!Sync`（结构上强制
//! "单"生产者/消费者）。

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Inner<T> {
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    mask: usize,
    /// 生产者写入位（单调递增，取 `& mask` 落槽）。
    head: AtomicUsize,
    /// 消费者读取位。
    tail: AtomicUsize,
}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        let h = *self.head.get_mut();
        let mut t = *self.tail.get_mut();
        while t != h {
            unsafe { (*self.slots[t & self.mask].get()).assume_init_drop() };
            t += 1;
        }
    }
}

/// SPSC 队列构造器（拆分后仅剩引用计数职责）。
pub struct Spsc<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Spsc<T> {
    /// 新建容量 `cap`（向上取 2 的幂，最小 2）的队列。
    pub fn new(cap: usize) -> Self {
        let cap = cap.next_power_of_two().max(2);
        let slots = (0..cap)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect();
        Self {
            inner: Arc::new(Inner {
                slots,
                mask: cap - 1,
                head: AtomicUsize::new(0),
                tail: AtomicUsize::new(0),
            }),
        }
    }

    /// 拆成生产者/消费者两端（通常各移交一个线程）。
    pub fn split(self) -> (Producer<T>, Consumer<T>) {
        (
            Producer {
                inner: Arc::clone(&self.inner),
                _not_sync: std::marker::PhantomData,
            },
            Consumer {
                inner: self.inner,
                _not_sync: std::marker::PhantomData,
            },
        )
    }
}

/// 生产者端：至多一线程持有（`Send`，`!Sync`）。
pub struct Producer<T> {
    inner: Arc<Inner<T>>,
    _not_sync: std::marker::PhantomData<*mut ()>,
}

// 安全性：槽位按索引分工（head 端仅本端写、tail 端仅对端写），
// 跨线程可见性由 head/tail 的 release/acquire 配对保证。
unsafe impl<T: Send> Send for Producer<T> {}

/// 消费者端：至多一线程持有（`Send`，`!Sync`）。
pub struct Consumer<T> {
    inner: Arc<Inner<T>>,
    _not_sync: std::marker::PhantomData<*mut ()>,
}

unsafe impl<T: Send> Send for Consumer<T> {}

impl<T> Producer<T> {
    /// 入队；满则原样退回（实时生产者不应阻塞）。
    pub fn push(&self, v: T) -> Result<(), T> {
        let inner = &*self.inner;
        let h = inner.head.load(Ordering::Relaxed); // 仅本端写 head
        if h - inner.tail.load(Ordering::Acquire) == inner.slots.len() {
            return Err(v);
        }
        unsafe { inner.slots[h & inner.mask].get().write(MaybeUninit::new(v)) };
        inner.head.store(h + 1, Ordering::Release); // 发布槽位内容
        Ok(())
    }

    /// 当前队列长度（瞬时近似值，仅诊断用）。
    pub fn len(&self) -> usize {
        let inner = &*self.inner;
        inner.head.load(Ordering::Relaxed) - inner.tail.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Consumer<T> {
    /// 出队；空返回 `None`。
    pub fn pop(&self) -> Option<T> {
        let inner = &*self.inner;
        let t = inner.tail.load(Ordering::Relaxed); // 仅本端写 tail
        if t == inner.head.load(Ordering::Acquire) {
            return None;
        }
        let v = unsafe { (*inner.slots[t & inner.mask].get()).assume_init_read() };
        inner.tail.store(t + 1, Ordering::Release); // 释放槽位给生产者
        Some(v)
    }

    /// 当前队列长度（瞬时近似值，仅诊断用）。
    pub fn len(&self) -> usize {
        let inner = &*self.inner;
        inner.head.load(Ordering::Acquire) - inner.tail.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
