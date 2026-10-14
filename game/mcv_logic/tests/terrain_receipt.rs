//! 世界生成 worker 路死账锁（worker-accounting DC-2/DC-3 + 静默门 B 修）。
//!
//! 三段式（请求-计算-回执）把 `generate_terrain` 移入 worker 池后，主线程
//! 的 req_frames（节流配额记账）与 Empty 手柄（请求环重请求判据）都依赖
//! 「每笔派发必有回执」——本文件钉死两条丢失路径的自愈回路：
//! 1. **失败回执锁**（DC-3）：worker 生成 panic → `Err` 失败回执必达 →
//!    配额名额当帧释放（pops 恢复）+ Empty 手柄移除（重请求回路打开）
//!    → 终态 commit_terrain 完成；连续 panic 不泄漏配额（加载不停摆）。
//!    注入机制 = [`TerrainScheduler::install_loss_hook`]（Deliver/Swallow/
//!    Panic，语义见 [`mcv_core::pool::LossMode`]）。
//! 2. **静默门漏态锁**（B 修）：「已产出未消化」的回执窗口必须算在途——
//!    旧 `pool_quiesced()` 只看池内状态，回执躺在通道等 drain 的窗口里
//!    误报静默 → 卸载段逐出 Empty 键 → 同帧 drain 落到已删键（白生成）
//!    → 振荡段回拉重请求（flap 两连红根因，run 38478972059）。测试缝 =
//!    [`GameRuntime::req_debug_mark`] 登记在途账 + 假回执不消化，推 300 帧
//!    断言 Empty 键不被逐出；回执消化后断路解除（恢复可逐出，不焊死）。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use mcv_core::pool::LossMode;
use mcv_core::{ChunkHandle, ChunkPos, Stage};
use mcv_logic::game::{GameMode, GameRuntime};

/// 唯一临时存档目录（并行安全，镜像 streaming.rs temp_dir 的做法）。
fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-terrain-{tag}-{:x}", h.finish()))
}

fn headless(tag: &str) -> GameRuntime {
    GameRuntime::new_headless(20261014, temp_dir(tag), GameMode::Survival)
}

/// 失败回执锁（DC-2 主锁）：前两笔生成 panic → 失败回执当帧释放 req_frames
/// 名额 + Err 分支移除 Empty 手柄（重请求回路打开）→ 第三笔正常 commit。
/// 断言终态：区块 TerrainReady、投递次数 ≥3（两次丢失 + 一次成功）、收尾
/// 在途名额归零（配额不泄漏——旧实现漏态下 req_frames 永真 → 名额泄漏满
/// in_flight_quota 后 pops≡0 全图生成停摆）。
#[test]
fn panicking_generate_releases_quota_and_recommits() {
    let mut rt = headless("terrain-loss");
    rt.render_dist = 0;
    let p0 = ChunkPos::new(0, 0);

    let requests = Arc::new(AtomicU32::new(0));
    {
        let requests = Arc::clone(&requests);
        let hit = p0;
        rt.scheduler.install_loss_hook(Box::new(move |p| {
            if p != hit {
                return LossMode::Deliver;
            }
            let n = requests.fetch_add(1, Ordering::Relaxed);
            if n < 2 {
                LossMode::Panic // 前两笔确定性 panic
            } else {
                LossMode::Deliver
            }
        }));
    }

    // 泵至中心块 commit_terrain 完成（丢失两轮 + 重请求 + 成功）。
    let t0 = Instant::now();
    loop {
        rt.stream();
        if (rt.chunks[&p0].stage() as u8) >= (Stage::TerrainReady as u8) {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(120),
            "生成死账未自愈：投递 {} 次、在途名额 {}",
            requests.load(Ordering::Relaxed),
            rt.gen_inflight()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        requests.load(Ordering::Relaxed) >= 3,
        "丢失路径必须走「失败回执 → 名额释放 → 重请求」回路（实投 {} 次）",
        requests.load(Ordering::Relaxed)
    );

    // 收尾：泵至生成侧静默（全部回执消化，含外环块的合法在途）→ 名额
    // 归零断言才有确定语义（中心块就绪时外环可能仍在途）。
    let t0b = Instant::now();
    while !rt.scheduler.pool_quiesced() {
        rt.stream();
        assert!(
            t0b.elapsed() < Duration::from_secs(60),
            "生成侧未静默（回执未消化 = 配额泄漏面）"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        rt.gen_inflight(),
        0,
        "全部回执消化后在途名额必须归零（泄漏 = 生成停摆）"
    );
}

/// 静默门漏态锁（B 修主锁）：登记在途记账 + 合成失败回执入通道**不消化前
/// 推 300 驻留帧** → Empty 远端键不得被卸载段逐出（「已产出未消化」必须
/// 算在途）；回执被 drain 消化后（计数归零）恢复可逐出（断路不焊死）。
#[test]
fn undrained_receipt_window_does_not_evict_empty_chunk() {
    let mut rt = headless("terrain-quiesce");
    rt.render_dist = 0;
    // 远端键：Chebyshev 5 > 卸载边界（rd+4 = 4）→ 卸载候选。
    let far = ChunkPos::new(0, 5);
    rt.chunks_mut().insert(far, Arc::new(ChunkHandle::new(far)));
    rt.req_debug_mark(far); // 在途登记 + 自记计数 +1（模拟已投递未回执）

    // 320 帧：跨过 300 驻留窗（逐帧 drain 不动合成回执之外的账）。
    for _ in 0..320u32 {
        rt.stream();
        assert!(
            rt.chunks.contains_key(&far),
            "帧 {}：回执未消化窗口内 Empty 键被逐出（静默门误报静默 → 白生成振荡 = flap 根因）",
            rt.stream_frame
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    // 回执消化（真/假同路径）：计数归零 → Err 分支移除 Empty 手柄（重请求
    // 回路打开）→ 远端键不再驻留。
    rt.scheduler.debug_inject_receipt(far);
    rt.stream();
    rt.stream();
    assert!(
        !rt.chunks.contains_key(&far),
        "回执消化后必须恢复可逐出（驻留不焊死）"
    );
    assert!(
        rt.scheduler.pool_quiesced(),
        "全部回执消化后生成侧必须静默（静默门语义）"
    );
}
