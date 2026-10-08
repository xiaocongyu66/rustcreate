//! Worker pool with a two-level priority queue (no external deps).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

pub type Job = Box<dyn FnOnce() + Send + 'static>;

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
}

/// Fixed-size worker pool. Jobs are `FnOnce + Send`; high-priority jobs are
/// drained before low-priority ones.
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
        });
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let shared = Arc::clone(&shared);
            handles.push(std::thread::spawn(move || loop {
                let job = {
                    let mut q = shared.queue.lock().unwrap();
                    loop {
                        if shared.shutdown.load(Ordering::Acquire) {
                            return;
                        }
                        if let Some(job) = q.pop() {
                            break job;
                        }
                        q = shared.signal.wait(q).unwrap();
                    }
                };
                job();
            }));
        }
        Self {
            shared,
            workers: handles,
        }
    }

    pub fn spawn_hi(&self, job: Job) {
        self.shared.queue.lock().unwrap().push_hi(job);
        self.shared.signal.notify_one();
    }

    pub fn spawn_lo(&self, job: Job) {
        self.shared.queue.lock().unwrap().push_lo(job);
        self.shared.signal.notify_one();
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

/// Default worker count for world tasks: leave 2 cores for main/render.
pub fn world_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(2).clamp(2, 6))
        .unwrap_or(2)
}
