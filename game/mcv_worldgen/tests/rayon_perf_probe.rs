//! rayon 收口性能探针（K-RAYON-GEN，E0 观测面 #155 的 CI 侧数据源）：
//! 同 seed 批量 worldgen 两组计时——串行逐块（主线程纯函数直呼）vs
//! `TerrainScheduler` 生产装配（`TaskPool::shared` = rayon 全局池 +
//! 配额门）。**只打印，不设速度断言**（数字入 CI 日志，收账进分支
//! docs/rayon-switch.md；正确性断言保留：两路产物按 pos 逐位一致——
//! 世界生成掷骰确定性锁的旁证，输出序/内容与调度顺序无关）。

use std::collections::HashMap;
use std::sync::mpsc;

use mcv_core::{ChunkPos, TaskPool};
use mcv_worldgen::{TerrainBackend, TerrainOutput, generate_terrain_with};

/// 对拍 seed（与 tests/parallel.rs 的 SEED 区分开，避免交叉解读）。
const SEED: u64 = 0xE2_C0DE_0002;
/// 批量块数（对角带跨正负坐标；CI debug/O3 单块数 ms 级 × 32 可控）。
const N: u32 = 32;

fn grid() -> Vec<ChunkPos> {
    (0..N)
        .map(|i| ChunkPos::new(200 + i as i32, -80 - i as i32 * 2))
        .collect()
}

/// 逐位比较体（voxels + heightmap）。
fn assert_same(a: &TerrainOutput, b: &TerrainOutput, pos: ChunkPos) {
    assert_eq!(
        a.voxels.as_u16_slice(),
        b.voxels.as_u16_slice(),
        "voxels 逐位不符 pos={pos:?}"
    );
    assert_eq!(a.heightmap, b.heightmap, "heightmap 不符 pos={pos:?}");
}

#[test]
fn gen_batch_serial_vs_rayon_shared_pool() {
    let backend = TerrainBackend::Vanilla;
    let positions = grid();

    // 串行参照（主线程纯函数直呼）。
    let t0 = std::time::Instant::now();
    let serial: HashMap<ChunkPos, TerrainOutput> = positions
        .iter()
        .map(|&p| {
            (
                p,
                generate_terrain_with(backend, SEED, p).expect("serial gen"),
            )
        })
        .collect();
    let serial_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // 生产装配同款：TerrainScheduler 构造路径 = TaskPool::shared(workers)
    // （rayon 全局池 + 配额门）。直接量池尾到回执齐（与 E0 init/remesh
    // 子段同口径：主线程提交 → 回执收齐）。
    let workers = mcv_core::world_worker_count();
    let pool = TaskPool::shared(workers);
    let (tx, rx) = mpsc::channel::<(ChunkPos, TerrainOutput)>();
    let t1 = std::time::Instant::now();
    for &p in &positions {
        let tx = tx.clone();
        pool.spawn_lo(Box::new(move || {
            let out = generate_terrain_with(backend, SEED, p).expect("pool gen");
            let _ = tx.send((p, out));
        }));
    }
    drop(tx);
    let pooled: HashMap<ChunkPos, TerrainOutput> = (0..positions.len())
        .map(|_| rx.recv().expect("pool 回执缺失"))
        .collect();
    let pool_ms = t1.elapsed().as_secs_f64() * 1000.0;

    for p in &positions {
        assert_same(&serial[p], &pooled[p], *p);
    }
    println!(
        "K-RAYON-GEN: backend=vanilla n={N} workers={workers} cpus={} serial_ms={serial_ms:.1} shared_pool_ms={pool_ms:.1} speedup={:.2}x",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0),
        serial_ms / pool_ms.max(f64::MIN_POSITIVE),
    );
}
