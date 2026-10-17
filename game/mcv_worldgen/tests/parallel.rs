//! E2 世界生成并行确定性锁（蓝图 docs/engine-roadmap.md §3 E2）。
//!
//! 把「同 seed 字节不变」从事实升格为测试锁，未来任何 worldgen 波
//! （表层/树/结构）回红即知破坏纯度：
//! ① 同 seed 下 3×3 块并行生成（多 worker 池、提交序打乱、结果到达序
//!    乱序）vs 单线程串行逐块生成——`TerrainOutput.voxels`/`heightmap`
//!    逐位对拍，legacy + vanilla 双后端都要；
//! ② 同配置跑两轮互拍（杀掉 mpsc 到达序影响——commit 顺序本就不影响
//!    内容，因为每块是纯函数，但这从未被测试锁住）。
//!
//! 纯函数性依据（蓝图勘察确认）：树跨块冠由邻域池扫描按同 seed 重算
//! （vanilla/tree_pass.rs:227-234，「树干在本块的池负责全树，邻池只贡献
//! 伸入本块的冠」）；表内核/vanilla 管线均无共享可变状态（terrain.rs
//! 全文仅 'static 只读常数表）。因此内容 = f(seed, pos)，与到达序无关。
//!
//! 乱序注入纪律（CI 计时稳定）：受控延迟用**计数门**（Mutex+Condvar
//! 发号 turnstile），不用 sleep 猜时序——每个任务生成完毕后阻塞在
//! 「放行名次 == 自己的编号」上，到号才 tx.send。send 先于 pass 发生，
//! 通道内形成 happens-before 链，到达序 = 放行序**确定性成立**（与线程
//! 调度抖动无关）；放行序与提交序逐位错开（构造保证），故对拍确实跑在
//! 「到达序 ≠ 提交序」的乱序域上。测试块数取小（3×3）保 CI 时限。

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

use mcv_core::{ChunkPos, TaskPool};
use mcv_worldgen::{TerrainBackend, TerrainOutput, generate_terrain_with};

/// 对拍 seed（任取不平凡值；锁与具体 seed 无关）。
const SEED: u64 = 0xE2_C0DE_0001;

/// 提交序置换（元素 = [`grid3x3`] 的下标）：任务按该序 spawn_lo 入队。
const SUBMIT_ORDER: [usize; 9] = [5, 0, 7, 2, 8, 1, 4, 6, 3];
/// 放行序置换：任务在该名次到来后才投递结果。与 [`SUBMIT_ORDER`] 逐位
/// 错开（构造保证，run_parallel 断言），使到达序相对提交序为非恒等置换。
const RELEASE_ORDER: [usize; 9] = [3, 6, 0, 8, 2, 7, 1, 5, 4];

/// 3×3 块网格（蓝图：测试块数取小保 CI 时限）。基点正负号混合，覆盖
/// 树邻域池扫描 div_euclid 对负坐标的两个半空间。
fn grid3x3() -> Vec<ChunkPos> {
    let (bx, bz) = (-13, 7);
    let mut v = Vec::with_capacity(9);
    for dz in -1..=1 {
        for dx in -1..=1 {
            v.push(ChunkPos::new(bx + dx, bz + dz));
        }
    }
    v
}

/// 计数门（发号器）：第 k 个放行者阻塞到已放行计数 == k。Mutex+Condvar
/// 提供 happens-before；不用 sleep——到达序由门序确定，与 OS 调度无关。
struct Turnstile {
    passed: Mutex<usize>,
    cv: Condvar,
}

impl Turnstile {
    fn new() -> Self {
        Self {
            passed: Mutex::new(0),
            cv: Condvar::new(),
        }
    }

    fn wait_turn(&self, turn: usize) {
        let mut n = self.passed.lock().unwrap();
        while *n != turn {
            n = self.cv.wait(n).unwrap();
        }
    }

    fn pass(&self) {
        let mut n = self.passed.lock().unwrap();
        *n += 1;
        self.cv.notify_all();
    }
}

/// 并行生成一批块：多 worker 池 + 提交序打乱 + 计数门注入到达序乱序。
/// 返回（pos → 输出，到达序）。
///
/// worker 数 = 任务数：每个任务生成后都阻塞在计数门等自己的放行名次，
/// 只有全员并发在跑时门才转得动（少一个 worker 即死锁）——这是受控
/// 乱序注入的构造前提，不是对池的通用约束。
fn run_parallel(
    backend: TerrainBackend,
    positions: &[ChunkPos],
) -> (HashMap<ChunkPos, TerrainOutput>, Vec<ChunkPos>) {
    assert_eq!(SUBMIT_ORDER.len(), positions.len(), "置换须与网格同长");
    assert_eq!(RELEASE_ORDER.len(), positions.len(), "置换须与网格同长");
    // 乱序注入的结构前提：放行序（=到达序）不得是提交序的重演。
    assert!(
        SUBMIT_ORDER
            .iter()
            .zip(RELEASE_ORDER.iter())
            .all(|(a, b)| a != b),
        "SUBMIT_ORDER 与 RELEASE_ORDER 必须逐位错开"
    );
    let pool = TaskPool::new(positions.len());
    let (tx, rx) = mpsc::channel();
    let gate = Arc::new(Turnstile::new());
    for &slot in &SUBMIT_ORDER {
        let pos = positions[slot];
        let tx = tx.clone();
        let gate = Arc::clone(&gate);
        let turn = RELEASE_ORDER.iter().position(|&s| s == slot).unwrap();
        pool.spawn_lo(Box::new(move || {
            let out = generate_terrain_with(backend, SEED, pos).expect("parallel gen");
            // 受控延迟（计数门）：生成完毕后阻塞到自己的放行名次——
            // 到达序与生成完成序/线程调度解耦，仅由 RELEASE_ORDER 决定。
            gate.wait_turn(turn);
            // 先 send 后 pass：send(turn k) happens-before pass(k)
            // happens-before turn k+1 醒来 happens-before send(k+1)——
            // 通道到达序 = 放行序，确定性成立（零计时面）。接收端全程
            // 在场（rx 在收集完才 drop），send 不可能失败——同
            // TerrainScheduler::request 的 `let _ = tx.send` 形态。
            let _ = tx.send((pos, out));
            gate.pass();
        }));
    }
    drop(tx);
    let mut arrival = Vec::with_capacity(positions.len());
    let mut by_pos = HashMap::with_capacity(positions.len());
    for (pos, out) in rx {
        arrival.push(pos);
        by_pos.insert(pos, out);
        if by_pos.len() == positions.len() {
            break;
        }
    }
    (by_pos, arrival)
}

fn voxels_bits(out: &TerrainOutput) -> &[u16] {
    out.voxels.as_u16_slice()
}

/// ① 并行 vs 串行逐位对拍（legacy + vanilla 双后端）：同 seed 3×3 块，
/// 提交序打乱 + 到达序乱序的并行结果，与单线程串行逐块生成的
/// `voxels`/`heightmap` 逐位相等。这是 E2 新语义锁：内容 = f(seed, pos)，
/// 与提交/到达/commit 顺序无关——任何 worldgen 波破坏纯度即在此回红
/// （蓝图 §3 E2：乱序 commit 本身无害，新增跨块缓存才破纯度）。
#[test]
fn parallel_matches_serial_bitwise_both_backends() {
    for backend in [TerrainBackend::Legacy, TerrainBackend::Vanilla] {
        let positions = grid3x3();
        // 串行参照：单线程纯函数逐块。
        let serial: HashMap<ChunkPos, TerrainOutput> = positions
            .iter()
            .map(|&p| {
                let out = generate_terrain_with(backend, SEED, p).expect("serial gen");
                (p, out)
            })
            .collect();
        let (parallel, arrival) = run_parallel(backend, &positions);
        // 乱序注入必须真的发生：到达序 = 放行序（≠ 提交序）。
        let want: Vec<ChunkPos> = RELEASE_ORDER.iter().map(|&s| positions[s]).collect();
        assert_eq!(
            arrival, want,
            "计数门注入失效：到达序 ≠ 放行序（对拍未跑在乱序域上）"
        );
        for p in &positions {
            let s = &serial[p];
            let q = parallel.get(p).unwrap_or_else(|| {
                panic!("并行结果缺块 backend={backend:?} pos={p:?}（生成 panic 或通道丢失）")
            });
            assert_eq!(
                voxels_bits(s),
                voxels_bits(q),
                "voxels 逐位不符 backend={backend:?} pos={p:?}"
            );
            assert_eq!(
                s.heightmap, q.heightmap,
                "heightmap 不符 backend={backend:?} pos={p:?}"
            );
        }
    }
}

/// ② 多次运行互拍（legacy + vanilla 双后端）：同配置（同 seed/同池构造/
/// 同两条置换）跑两轮逐位相等。锁「commit 顺序不影响内容」——两轮的
/// 生成完成序随线程调度各次不同，内容仍必须逐位一致。
#[test]
fn two_runs_bitwise_identical_both_backends() {
    for backend in [TerrainBackend::Legacy, TerrainBackend::Vanilla] {
        let positions = grid3x3();
        let (run1, _) = run_parallel(backend, &positions);
        let (run2, _) = run_parallel(backend, &positions);
        for p in &positions {
            let a = run1.get(p).expect("第一轮缺块");
            let b = run2.get(p).expect("第二轮缺块");
            assert_eq!(
                voxels_bits(a),
                voxels_bits(b),
                "两轮 voxels 逐位不符 backend={backend:?} pos={p:?}"
            );
            assert_eq!(
                a.heightmap, b.heightmap,
                "两轮 heightmap 不符 backend={backend:?} pos={p:?}"
            );
        }
    }
}

/// ③ rayon par_iter 逐 index 回填（rayon 收口波红线①）：并行调度器
/// （无论自研池还是 rayon 全局池/工作窃取）完成序不确定，**输出序必须
/// 按 index 原位回填钉死**——黄金对拍与下游消费的输出序 bit 级不变。
/// 提交序打乱（SUBMIT_ORDER 置换）喂 par_iter，收集 (slot, out) 完成对
/// 后按原格位 index 回填，对拍按格位序逐位比对（legacy + vanilla）。
#[test]
fn rayon_par_iter_index_backfill_bitwise_both_backends() {
    use rayon::prelude::*;
    for backend in [TerrainBackend::Legacy, TerrainBackend::Vanilla] {
        let positions = grid3x3();
        // 串行参照：单线程纯函数逐块。
        let serial: HashMap<ChunkPos, TerrainOutput> = positions
            .iter()
            .map(|&p| {
                let out = generate_terrain_with(backend, SEED, p).expect("serial gen");
                (p, out)
            })
            .collect();
        // 乱序提交 + 完成对收集：par_iter 内部顺序不确定（调度即序），
        // 完成对 (slot, out) 只带格位 index，不带到达承诺。
        let done: Vec<(usize, TerrainOutput)> = SUBMIT_ORDER
            .par_iter()
            .map(|&slot| {
                let out = generate_terrain_with(backend, SEED, positions[slot]).expect("par gen");
                (slot, out)
            })
            .collect();
        // 逐 index 回填：输出序 = 网格序，与完成/到达序解耦。
        let mut backfilled: Vec<Option<TerrainOutput>> =
            (0..positions.len()).map(|_| None).collect();
        for (slot, out) in done {
            assert!(
                backfilled[slot].is_none(),
                "同格位重复回填 slot={slot}（提交面 bug）"
            );
            backfilled[slot] = Some(out);
        }
        for (slot, out) in backfilled.iter().enumerate() {
            let p = positions[slot];
            let q = out.as_ref().unwrap_or_else(|| {
                panic!("index 回填缺格 slot={slot} pos={p:?} backend={backend:?}")
            });
            let s = &serial[&p];
            assert_eq!(
                voxels_bits(s),
                voxels_bits(q),
                "voxels 逐位不符 backend={backend:?} pos={p:?}（输出序回填面）"
            );
            assert_eq!(
                s.heightmap, q.heightmap,
                "heightmap 不符 backend={backend:?} pos={p:?}"
            );
        }
    }
}
