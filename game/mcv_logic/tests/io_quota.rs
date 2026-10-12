//! E4/E2 合流债闭合：IO 投递配额消费侧门控回归。
//!
//! E4 波把存/读盘搬到独立 `mcv-io` 线程，但「在途笔数」只有记账没有
//! 消费侧门控：读盘投递仅受 4/帧投递预算约束（慢盘时在途队列无上界）；
//! E2 波登记的名额函数 [`mcv_logic::ticket::in_flight_quota`] 也一直是
//! 纯记账（game.rs 票驱动请求环读死常量，ticket.rs 债注原文）。本文件
//! 锁配额接线后的四条语义：
//!
//! 1. **读侧打满零新投递**：在途读笔数 ≥ 配额（= in_flight_quota，
//!    E2 名额与读配额同数——原版读盘发生在被节流的票任务流内）时，
//!    请求环不再投递新读、也不得绕过读盘直发生成（存档覆盖锁同构）。
//! 2. **排空恢复投递**：在途读清零后投递恢复（配额在完成回调释放）。
//! 3. **写侧高水位 64**：在途写打满高水位后续投被拦，回执排空后恢复
//!    （原版 MAX_ACTIVE_CHUNK_WRITES=128 的规模缩减形态，ChunkMap.java:
//!    122-125,:505,:771/:779；高水位与复活缓存 64 硬顶同量级）。
//! 4. **读写门独立**：写积压打满不拦读盘（原版 128 门只管写侧，读走
//!    FOREGROUND 优先——IOWorker.java:192-204,:216-226）。
//!
//! 驱动纪律：全部用测试钩子（[`GameRuntime::io_debug_mark_loading`] /
//! [`GameRuntime::io_debug_clear_loading`] / [`GameRuntime::io_debug_drain`]）
//! 与单次投递 API 驱动，不依赖 IO/worker 线程到达时序——除写盘回执排空
//! 一处（等待环即 io_async.rs 同款取证式轮询，断言仍为精确值）。

use std::sync::Arc;
use std::time::Duration;

use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage, dirty, vidx};
use mcv_logic::game::{GameMode, GameRuntime};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-io-quota-{tag}-{:x}", h.finish()))
}

fn headless_io(dir: std::path::PathBuf) -> GameRuntime {
    GameRuntime::new_headless_with_io(20261012, dir, GameMode::Survival, true)
}

/// 读盘在途配额（与实装同源：E2 名额函数 × 构造时点 worker 数）。
fn read_quota() -> usize {
    mcv_logic::ticket::in_flight_quota(mcv_core::world_worker_count())
}

/// 在途写高水位（game.rs 私有常量的测试侧镜像；64 = 复活缓存硬顶量级，
/// 原版 128 的规模缩减）。
const IO_HIGH_WATER: usize = 64;

/// 预置一块 ≥TerrainReady 且带 SAVE 位的区块（地板两层 BlockId(1)，
/// io_async.rs 同款 fixture）。
fn terrain_chunk(pos: ChunkPos) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(pos));
    {
        let mut vg = h.voxels.write().unwrap();
        for y in -64..=-63 {
            for zz in 0..16usize {
                for xx in 0..16usize {
                    vg.set_flat(vidx(xx, y, zz), BlockId(1));
                }
            }
        }
    }
    let ids: Vec<u16> = h.voxels.read().unwrap().decode_column();
    *h.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&ids);
    h.advance_to(Stage::TerrainReady);
    h.mark_dirty(dirty::SAVE);
    h
}

/// 锁 1：在途读打满配额 → 请求环零新投递（不 Empty 在册、不在途记账
/// 上再加笔）。fake 在途读打在环外坐标——门是纯计数门，与位置语义无关，
/// 大核机配额 > 环格数时同样成立。
#[test]
fn read_inflight_quota_blocks_new_load_posts() {
    let mut rt = headless_io(temp_dir("qread"));
    rt.render_dist = 1;
    let q = read_quota();
    for i in 0..q as i32 {
        rt.io_debug_mark_loading(ChunkPos::new(500 + i, 500));
    }
    assert_eq!(rt.io_inflight().1, q, "前置：fake 在途读已打满配额");
    rt.stream();
    assert_eq!(
        rt.chunks.len(),
        0,
        "在途读打满时请求环不得投递新读（零新投递；Empty 在册 = 已投递）"
    );
    assert_eq!(rt.io_inflight().1, q, "在途读笔数不得超过配额（超限投递）");
}

/// 锁 2：配额释放 → 投递恢复。释放路径 = 完成回调消化（回执 drain 解除
/// io_frames 记账）；测试用 clear 钩子模拟「回执已全部消化」的排空态，
/// 不依赖真实 IO 时序。
#[test]
fn read_quota_release_resumes_posts() {
    let mut rt = headless_io(temp_dir("qrel"));
    rt.render_dist = 1;
    let q = read_quota();
    for i in 0..q as i32 {
        rt.io_debug_mark_loading(ChunkPos::new(500 + i, 500));
    }
    rt.stream();
    assert_eq!(rt.chunks.len(), 0, "前置：打满期零投递");
    rt.io_debug_clear_loading();
    rt.stream();
    assert!(
        !rt.chunks.is_empty() && rt.io_inflight().1 >= 1,
        "排空后必须恢复读投递（Empty 在册 + 在途读记账 ≥1）"
    );
}

/// 锁 3：在途写高水位——65 块脏表全量投递，第 65 笔被高水位 64 拦下；
/// 回执排空（= 配额释放）后续投恰好 1 笔。写盘回执由真实 mcv-io 线程
/// 产生，排空段用轮询（io_async.rs 同款），断言仍为精确计数。
#[test]
fn write_high_water_blocks_and_resumes_after_drain() {
    let dir = temp_dir("qwrite");
    let mut rt = headless_io(dir);
    rt.render_dist = 1;
    for i in 0..(IO_HIGH_WATER + 1) as i32 {
        let pos = ChunkPos::new(i, 400); // 环外：不触发读盘/生成，纯写侧场景
        rt.chunks_mut().insert(pos, terrain_chunk(pos));
    }
    rt.save_dirty(None);
    assert_eq!(
        rt.io_inflight().0,
        IO_HIGH_WATER,
        "在途写必须封顶在高水位（第 {} 笔被拦）",
        IO_HIGH_WATER + 1
    );
    let mut drained = false;
    for _ in 0..2000 {
        rt.io_debug_drain();
        if rt.io_inflight().0 == 0 {
            drained = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    assert!(drained, "写回执排空超时（在途写 ≠ 0：配额无法释放）");
    rt.save_dirty(None);
    assert_eq!(
        rt.io_inflight().0,
        1,
        "配额释放后此前被拦的最后一笔必须恢复投递"
    );
}

/// 锁 4：读写门独立——写侧打满高水位不拦读盘投递（裁决依据：原版
/// activeChunkWrites 门只限保存，ChunkMap.java:505,:771/:779；读盘走
/// IOWorker FOREGROUND 优先队列，不被写积压饿死，IOWorker.java:192-204）。
#[test]
fn write_backlog_does_not_gate_reads() {
    let mut rt = headless_io(temp_dir("qsep"));
    rt.render_dist = 1;
    for i in 0..IO_HIGH_WATER as i32 {
        let pos = ChunkPos::new(i, 500); // 环外脏块：只喂写侧
        rt.chunks_mut().insert(pos, terrain_chunk(pos));
    }
    rt.save_dirty(None);
    assert_eq!(rt.io_inflight().0, IO_HIGH_WATER, "前置：写在途打满");
    rt.stream();
    assert!(
        rt.io_inflight().1 >= 1,
        "写积压打满不得拦读盘投递（门独立：原版 128 门只管写侧）"
    );
    // 读盘投递读到的都是未存档位置 → 回执 miss 回退生成器，属既有语义；
    // 此处只锁「投递发生」。
}
