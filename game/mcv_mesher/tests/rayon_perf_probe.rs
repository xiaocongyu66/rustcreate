//! rayon 收口性能探针（K-RAYON-MESH，E0 remesh 子段的 CI 侧数据源）：
//! 同场景批量网格化两组计时——串行（主线程直呼 [`Mesher::build`]）vs
//! [`MeshScheduler`] 生产装配（`TaskPool::shared` = rayon 全局池 + 配额
//! 门，hi/lo 走 lo 预算流）。**只打印，不设速度断言**（数字入 CI 日志，
//! 收账进分支 docs/rayon-switch.md）。正确性断言保留：池路产物 vs 串行
//! 直呼**逐字节一致**（同 tests/mesh_parallel.rs::worker_output_bit_identical
//! 的等价锁语义，rayon 执行端下再证一次）。

use std::collections::HashMap;

use mcv_core::ChunkPos;
use mcv_mesher::{MESH_OPAQUE, MESH_WATER};
use mcv_mesher::{MeshJob, MeshScheduler, Mesher, neighborhood_slots};

/// 批量块数。
const N: u32 = 32;
/// 取回超时（防 CI 争用挂死）。
const WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// 确定性伪随机字节流（64 位 LCG；与种子绑定的纯函数）。
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
}

/// 3×3 邻域场景：空气/石/水混合 + 满幅光照字节（同 mesh_parallel.rs
/// fixture 结构，本地自含）。water pass 非平凡需要水方块 id。
fn scene_bytes(seed: u64) -> (Vec<u16>, Vec<u8>) {
    let vol = mcv_core::CHUNK_VOL;
    let water = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "water")
        .expect("原版方块表必有 water") as u16;
    let mut rng = Lcg(seed);
    let mut voxels = vec![0u16; 9 * vol];
    let mut lights = vec![0u8; 9 * vol];
    for i in 0..9 * vol {
        let r = rng.next();
        voxels[i] = match r % 11 {
            0..=2 => 1, // stone
            3 | 4 => water,
            _ => 0, // air
        };
        lights[i] = (r >> 8) as u8;
    }
    (voxels, lights)
}

#[test]
fn mesh_batch_serial_vs_rayon_shared_pool() {
    let scene = scene_bytes(0xE3_5EED);
    let (voxels, lights) = &scene;
    let positions: Vec<ChunkPos> = (0..N)
        .map(|i| ChunkPos::new(300 + (i as i32 % 8), -40 - (i as i32 / 8) * 3))
        .collect();

    // 串行参照：直呼 Mesher::build（opaque + water）。
    let mesher = Mesher::new();
    let slots = neighborhood_slots(voxels, lights);
    let t0 = std::time::Instant::now();
    let serial: HashMap<ChunkPos, Vec<u8>> = positions
        .iter()
        .map(|&p| {
            let mut buf = Vec::new();
            buf.extend_from_slice(mesher.build(&slots, MESH_OPAQUE).unwrap().vertex_data());
            buf.extend_from_slice(mesher.build(&slots, MESH_WATER).unwrap().vertex_data());
            (p, buf)
        })
        .collect();
    let serial_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // 生产装配：MeshScheduler::new(workers) = TaskPool::shared（rayon
    // 全局池 + 配额门）；lo 预算流派发，drain 收齐。
    let workers = mcv_core::world_worker_count();
    let (sched, mut results) = MeshScheduler::new(workers);
    let t1 = std::time::Instant::now();
    for &p in &positions {
        sched.submit(
            MeshJob {
                pos: p,
                origin: [16.0 * p.x as f32, 0.0, 16.0 * p.z as f32],
                voxels: voxels.clone(),
                lights: lights.clone(),
                tint: None,
            },
            false,
        );
    }
    let deadline = t1 + WAIT;
    let mut done = Vec::new();
    while done.len() < positions.len() {
        assert!(
            std::time::Instant::now() < deadline,
            "池产物超时：{}/{}（池饿死或通道断）",
            done.len(),
            positions.len()
        );
        done.extend(results.drain_all());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let pool_ms = t1.elapsed().as_secs_f64() * 1000.0;

    for d in &done {
        let mut buf = Vec::new();
        if let Some(op) = &d.opaque {
            buf.extend_from_slice(&op.vertex);
        }
        if let Some(w) = &d.water {
            buf.extend_from_slice(&w.vertex);
        }
        assert_eq!(
            &serial[&d.pos], &buf,
            "池路产物与串行直呼不一致 pos={:?}（seq={}）",
            d.pos, d.seq
        );
    }
    assert_eq!(done.len(), positions.len());
    println!(
        "K-RAYON-MESH: n={N} workers={workers} cpus={} serial_ms={serial_ms:.1} shared_pool_ms={pool_ms:.1} speedup={:.2}x",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0),
        serial_ms / pool_ms.max(f64::MIN_POSITIVE),
    );
}
