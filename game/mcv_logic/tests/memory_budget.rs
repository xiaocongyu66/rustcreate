//! 整区块重构刀 5a（任务板 #115）：区块 resident 内存预算断言（CI 硬门）
//! + 实测值取证。
//!
//! `harness = false`：main() 直接运行，stdout 不经测试 harness 捕获，
//! CI 绿跑也落日志——`K5-MEAS` 行即 resident 字节实测值（本刀交付证据；
//! 普通测试的 eprintln 绿跑不可见，实测无从取证）。
//!
//! 纯计数法：Σ(voxels+light+heightmap 字节) 按各块实际数组实累加，不读
//! /proc（CI 稳定性；RSS 还含调度器/堆噪声，不是预算门的正确口径）。
//!
//! v6 单块 resident **最坏上界**算式（docs/memory-budget.md 全表）：
//! `voxels 24×(4096×16bit)=196608 B + light 98304×1B + heightmap 256×2B
//! = 295424 B`（288.5 KiB）。E7a 起 voxels 为 palette 压缩驻留
//! （`VoxelStore`：全空气 section 0bit、典型地形 ≤4bit），196608 B 是
//! global 16bit 档的**最坏上界**（平铺同值），不再等于典型值——
//! 典型柱实测门见下 `typical_generated_column` 断言与新预算：
//! rd=8 → 361 块 ≤ 60 MiB；rd=6 → 225 块 ≤ 35 MiB（口径 =
//! Σ VoxelStore::resident_bytes + light + heightmap，实测落
//! K5-MEAS-VOX 行）。稳态在册集 = 请求环 = sim_dist = rd+1（game.rs
//! stream 螺旋 `0..=sim_dist`；无移动 → 无卸载 → 无迟滞滞留、无复活
//! 缓存）。预算数来自派单书（#115 + E7a 派单），口径 = 静止稳态。
//!
//! 已知口径外足迹（登记为债，不在本门内）：移动时迟滞环（卸载边界
//! Chebyshev rd+4，game.rs unload_dist）使在册包络涨到 ~19×21（直线行走）
//! ≈ 115 MiB，叠加复活缓存（≤64 块硬顶）最坏 ≈ 137 MiB > 120 MiB——
//! 预算门只锁静止稳态；移动足迹的回收策略留后续刀。

use std::time::Duration;

use glam::Vec3;
use mcv_logic::game::{GameMode, GameRuntime};

/// 稳态判定窗：在册数连续 400 帧不变且在途生成清零（timeout 类预算按
/// 派单纪律留 2 倍余量——插入只随 worldgen 结果推进，400 ms 静默 + 在途
/// 为 0 是「全部插入完成」的确定性信号）。
const STEADY_FRAMES: u32 = 400;
/// 帧预算硬顶（liveness 兜底）。361 块全插入需 ~90 轮 worldgen（在途
/// 上限 4/帧），flap 测实测 81 块收敛 ~900 帧（streaming.rs 注释），按
/// 4.5 倍块数 × 2 倍争用余量取 60000 帧（≈60 s + stream 开销）。
const FRAME_CAP: u32 = 60_000;
/// 在册数上界余量（派单「19²+ε」的 ε）：覆盖单帧淘汰时间片 ≤4 × 2 帧
/// 的过渡窗。本测静止无淘汰，实测恒 = 期望值；ε 只防时序毛刺误报。
const COUNT_EPS: i32 = 8;
/// v6 单块常量字节数（算式锁：改世界尺寸/类型宽度必须重算预算门）。
const PER_CHUNK_BYTES: u64 = (mcv_core::CHUNK_VOL * 2 + mcv_core::CHUNK_VOL + 256 * 2) as u64;

/// 唯一临时存档目录（镜像 streaming.rs 的做法：并行安全、不撞真实存档）。
fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-membudget-{tag}-{:x}", h.finish()))
}

/// 驱动一个视距档到稳态并做预算断言。返回 (在册块数, 实测字节) 供取证行。
fn run_case(tag: &str, rd: i32, budget_mib: u64) -> (usize, u64) {
    let mut rt = GameRuntime::new_headless(20261011, temp_dir(tag), GameMode::Survival);
    rt.render_dist = rd;
    // 钉住玩家位置（pos ≠ ZERO）：出生投放（game.rs spawn 投放分支要求
    // pos == ZERO）会把 center 平移到投放槽（±load_radius 搜索窗内），迟滞
    // 环 union 可达 22² 块，破坏「稳态 = 请求环」的预算口径。钉位后投放
    // 分支永不触发、center 恒 (0,0)，稳态在册集 = sim_dist 环，与派单 v6
    // 算式（19²/15²）逐块对应。
    rt.player.pos = Vec3::new(8.5, 80.0, 8.5);

    let sim = rt.sim_dist();
    let expected = (2 * sim + 1) * (2 * sim + 1); // 19² / 15²
    let render_ring = (2 * rd + 1) * (2 * rd + 1); // 17² / 13²

    let mut stable = 0u32;
    let mut frames = 0u32;
    let mut last = usize::MAX;
    while stable < STEADY_FRAMES {
        assert!(
            frames < FRAME_CAP,
            "{tag}: {FRAME_CAP} 帧内未达稳态（在册 {last}，期望 {expected}）"
        );
        rt.stream();
        let n = rt.chunks.len();
        if n == last && rt.inflight_generation_count() == 0 {
            stable += 1;
        } else {
            stable = 0;
        }
        last = n;
        frames += 1;
        std::thread::sleep(Duration::from_millis(1));
    }

    // ①在册块数上界（派单 19²+ε；稳态=请求环，实测应恰为 expected）。
    assert!(
        last as i32 <= expected + COUNT_EPS,
        "{tag}: 稳态在册 {last} 块超过请求环 {expected}+{COUNT_EPS}（请求环失控/迟滞泄漏）"
    );
    // 下界（liveness）：render 环必须全部在册，防「提前冻结虚过」——
    // 上界断言对 0 块恒真，没有下界预算门是空的。
    assert!(
        last as i32 >= render_ring,
        "{tag}: 稳态在册 {last} 块不足 render 环 {render_ring}（加载停摆）"
    );
    // 稳态无淘汰 → 复活缓存必须为空（其字节同为 resident，口径见模块注释）。
    assert_eq!(
        rt.pending_unload_count(),
        0,
        "{tag}: 静止稳态出现复活缓存积压（无移动却发生淘汰）"
    );

    // ②Σ(voxels resident + light + heightmap) 实累加（锁各块一次锁，
    // 避免长持写锁）。E7a：voxels 走 VoxelStore::resident_bytes()
    // （palette 实际堆驻留，口径见 mcv_core::VoxelStore 文档注释）。
    let mut total: u64 = 0;
    let mut vox_total: u64 = 0;
    for h in rt.chunks.values() {
        // v6 算式锁：light/heightmap 尺寸漂移 = 预算门失效。
        assert_eq!(h.light.read().unwrap().len(), mcv_core::CHUNK_VOL);
        assert_eq!(h.heightmap.read().unwrap().len(), 256);
        let vox = h.voxels.read().unwrap().resident_bytes() as u64;
        // 最坏上界锁：palette 驻留不得超过平铺同宽（16bit global）。
        assert!(
            vox <= (mcv_core::CHUNK_VOL * 2) as u64,
            "voxels resident {vox} 超最坏上界 {} B",
            mcv_core::CHUNK_VOL * 2
        );
        vox_total += vox;
        total += vox;
        total += h.light.read().unwrap().len() as u64;
        total += (h.heightmap.read().unwrap().len() * std::mem::size_of::<i16>()) as u64;
    }

    let mib = total as f64 / (1024.0 * 1024.0);
    let headroom = 100.0 * (1.0 - mib / budget_mib as f64);
    eprintln!(
        "K5-MEAS {tag}: rd={rd} steady_frames={frames} resident_chunks={last} (expected {expected}+eps{COUNT_EPS}) \
per_chunk_worst_bytes={PER_CHUNK_BYTES} resident_bytes={total} ({mib:.1} MiB) budget={budget_mib} MiB headroom={headroom:.1}%"
    );
    eprintln!(
        "K5-MEAS-VOX {tag}: voxels_resident={vox_total} B ({:.1} MiB), avg/chunk={} B (平铺 196608 B 基线)",
        vox_total as f64 / (1024.0 * 1024.0),
        vox_total / last as u64
    );
    assert_eq!(
        PER_CHUNK_BYTES, 295424,
        "v6 单块最坏上界算式漂移（chunk.rs 尺寸变更须重算预算门）"
    );
    assert!(
        total <= budget_mib * 1024 * 1024,
        "{tag}: resident {total} B ({mib:.1} MiB) 超预算 {budget_mib} MiB \
（E7a 口径：voxels palette 驻留 + light + heightmap；超出即尺寸漂移或在册失控）"
    );
    (last, total)
}

/// E7a 新增门①：全空气柱 voxels 驻留 ≤ 常数（0bit 单值档生效的
/// 硬证据；平铺基线恒为 196608 B，0bit 后只剩结构开销）。
fn assert_all_air_column_constant() {
    let h = mcv_core::ChunkHandle::new(mcv_core::ChunkPos::new(0, 0));
    let vox = h.voxels.read().unwrap().resident_bytes();
    eprintln!("K5-MEAS-VOX all_air_column: resident={vox} B");
    assert!(
        vox <= 1024,
        "全空气柱驻留 {vox} B 超常数预算（0bit 档失效？）"
    );
}

/// E7a 新增门②：典型生成柱（vanilla 后端、固定 seed 多点采样）
/// voxels 驻留 ≤ 32 KiB。
fn assert_typical_generated_column() {
    for (cx, cz) in [(0, 0), (3, -2), (-7, 11), (64, 64)] {
        let pos = mcv_core::ChunkPos::new(cx, cz);
        let out = mcv_worldgen::generate_terrain_with(
            mcv_worldgen::TerrainBackend::Vanilla,
            20261011,
            pos,
        )
        .expect("worldgen");
        let store = mcv_core::VoxelStore::from_flat_u16(out.voxels.as_u16_slice());
        let vox = store.resident_bytes();
        eprintln!("K5-MEAS-VOX generated({cx},{cz}): resident={vox} B");
        assert!(
            vox <= 32 * 1024,
            "({cx},{cz}) 生成柱 voxels {vox} B 超 32 KiB"
        );
    }
}

fn main() {
    // E7a 收紧后预算门：vd8 ≤ 60 MiB、vd6 ≤ 35 MiB（平铺基线实测
    // 101.7 / 63.4 MiB；voxels palette 化后 light 96 KiB/块成为主体，
    // 361×~106 KiB ≈ 37 MiB、225×~106 KiB ≈ 23 MiB，预算留 ~1.6×
    // 余量吸收地形波动）。收紧即本刀收益的门禁化。
    assert_all_air_column_constant();
    assert_typical_generated_column();
    let (n8, b8) = run_case("vd8", 8, 60);
    let (n6, b6) = run_case("vd6", 6, 35);
    eprintln!(
        "K5-MEAS summary: vd8 chunks={n8} bytes={b8} | vd6 chunks={n6} bytes={b6} | all budgets ok"
    );
}
