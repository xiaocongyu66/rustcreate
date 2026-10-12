//! 波「渲染内存治理」（任务 ①/②）：CPU 网格字节有界回收 + 批次重建瞬态
//! 放大治理——红基线（先红后绿工作流）。
//!
//! `harness = false`（同 mesh_memory.rs）：K-MEM 行落 CI 日志可取证；
//! 走 render-headless job（lavapipe，gpu-tests feature），纯 headless
//! 路径（MeshUploader::build_chunk/unload/flush），不开窗口不截图。
//!
//! 断言全部走计数/字节统计（evicted_chunks/transient_bytes/
//! transient_peak_bytes），不靠墙钟：
//! - ①：只 build 不 unload 的滞留场景下，池字节必须被预算门控住
//!   （≤ CPU_POOL_BUDGET_BYTES，越界由 LRU 逐出，逐出有计数实证）；
//!   全量 unload + flush 后池/瞬态清零（释放路径字节实证）。
//! - ②：稳态后连续多轮「重建全部批次且不 flush」，瞬态滞留字节峰值
//!   ≤ 稳态驻留 × 1.5 + 常数（旧批次的拷贝必须随批次替换被摘除）；
//!   重建前后条目数与可渲染索引总量不塌（无空帧面）；重建未 flush 即
//!   unload，滞留必须清零（替换/卸载路径都摘拷贝）。

use mcv_core::ChunkPos;
use mcv_render::gpu::{MeshUploader, TERRAIN_STRIDE};

/// ① 预算门控的预算（集中定义在 gpu.rs，依据 RD=8 稳态实测 +25%）。
const BUDGET: u64 = mcv_render::gpu::CPU_POOL_BUDGET_BYTES;
/// 滞留场景单块顶点字节：200_000 顶点 × 24B ≈ 4.8 MiB（> MAX_BATCH_BYTES
/// 4 MiB → 恒单块批次，字节记账最直白）。
const BIG_VERTS: usize = 200_000;
const BIG_BYTES: usize = BIG_VERTS * TERRAIN_STRIDE;
/// ② 稳态场景：4×4=16 块、单块 ≈ 2_800 顶点 ≈ 65 KiB → 合批成 2 批。
const SMALL_VERTS: usize = 2_800;
const GRID: i32 = 4;
/// ② 瞬态放大公式的常数余量（缓冲 4B 对齐补零 + 占位条目 8B 级别）。
const TRANSIENT_SLACK: u64 = 64 * 1024;

fn device_queue() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no adapter: install mesa-vulkan-drivers for lavapipe");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("render-mem"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("device");
    (device, queue)
}

/// 合成顶点字节（24B 整倍数，内容任意——本测只走上传统计，不比对像素）。
fn fake_verts(n: usize) -> Vec<u8> {
    vec![0x5Au8; n * TERRAIN_STRIDE]
}

/// 一个最小的合法索引三角形。
fn fake_idx() -> Vec<u32> {
    vec![0, 1, 2]
}

fn origin(cx: i32, cz: i32) -> [f32; 3] {
    [16.0 * cx as f32, 0.0, 16.0 * cz as f32]
}

/// Σ条目可渲染 opaque 索引数（「重建前后批次可渲染面不塌」的口径）。
fn renderable_opaque(up: &MeshUploader) -> u64 {
    up.entries()
        .iter()
        .map(|e| (e.opaque_range.end - e.opaque_range.start) as u64)
        .sum()
}

fn main() {
    let (device, queue) = device_queue();

    // ---- ① 滞留场景：只 build 不 unload，池必须被预算门控 ----
    // 18 块 × 4.8 MiB ≈ 86 MiB > 60 MiB 预算：越界部分必须由 LRU 逐出
    // （原版 processUnloads 的兜底类比：越界驻留 = 该卸未卸）。
    let mut uploader = MeshUploader::new(device.clone(), queue.clone());
    let big = fake_verts(BIG_VERTS);
    let idx = fake_idx();
    let n_big = 18;
    for i in 0..n_big {
        uploader.build_chunk(origin(i, 0), &big, &idx, None);
        uploader.flush();
        let st = uploader.batching_stats();
        assert!(
            st.cpu_pool_bytes <= BUDGET,
            "① 第 {i} 块 build 后 CPU 池 {} B 超预算 {BUDGET} B（越界必须由 LRU 逐出，不许无界驻留）",
            st.cpu_pool_bytes
        );
    }
    let st = uploader.batching_stats();
    let mib = st.cpu_pool_bytes as f64 / (1024.0 * 1024.0);
    eprintln!(
        "K-MEM ① leak-churn: built={n_big} pool={} B ({mib:.2} MiB) budget={BUDGET} B \
         evicted_chunks={} evicted_bytes={} entries={}",
        st.cpu_pool_bytes, st.evicted_chunks, st.evicted_bytes, st.draw_entries
    );
    assert!(
        st.evicted_chunks >= 1,
        "① 越预算场景逐出计数恒 0（释放路径未接线，cpu_pool_bytes 靠运气达标不算实证）"
    );
    assert!(
        st.evicted_bytes >= (n_big as u64 - 14) * BIG_BYTES as u64,
        "① 逐出字节统计与越界量级不符（记账漂移）"
    );
    // 释放路径：全量 unload + flush → 池/条目/瞬态清零。
    for i in 0..n_big {
        uploader.unload(ChunkPos { x: i, z: 0 });
    }
    uploader.flush();
    let st = uploader.batching_stats();
    assert_eq!(st.cpu_pool_bytes, 0, "① 全量 unload 后池未清零");
    assert_eq!(st.draw_entries, 0, "① 全量 unload 后条目未清零");
    assert_eq!(st.meshed_chunks, 0, "① 块数记账漂移");
    assert_eq!(st.transient_bytes, 0, "① flush 后瞬态滞留未清零");

    // ---- ② 重建瞬态放大：连续多轮重建全部批次且不 flush ----
    // 全新 uploader：峰值记账从头计，稳态驻留 = 场景全量 flush 后的
    // resident_bytes。
    let mut up = MeshUploader::new(device, queue);
    let small = fake_verts(SMALL_VERTS);
    for cz in 0..GRID {
        for cx in 0..GRID {
            up.build_chunk(origin(cx, cz), &small, &idx, None);
        }
    }
    up.flush();
    let steady = up.batching_stats();
    assert!(
        steady.draw_entries >= 2,
        "② 稳态未形成多成员批次（合批失效）"
    );
    let steady_resident = steady.resident_bytes;
    let e0 = up.entries().len();
    let r0 = renderable_opaque(&up);
    let peak0 = up.batching_stats().transient_peak_bytes;
    eprintln!(
        "K-MEM ② steady: entries={e0} resident={} B pool={} B opaque_idx={r0} peak0={peak0}",
        steady.resident_bytes, steady.cpu_pool_bytes
    );

    // 3 轮「重建全部批次、轮内不 flush」：旧实现里被替换批次的拷贝滞留
    // 在 uploads 里不摘除，峰值随轮数线性放大；治理后每轮重建先摘旧批
    // 拷贝，瞬态 ≤ 一轮 ≈ 稳态。
    for round in 0..3u64 {
        for cz in 0..GRID {
            for cx in 0..GRID {
                up.build_chunk(origin(cx, cz), &small, &idx, None);
            }
        }
        let st = up.batching_stats();
        assert_eq!(
            up.entries().len(),
            e0,
            "② 第 {round} 轮重建后条目数塌缩（空帧面）"
        );
        assert_eq!(
            renderable_opaque(&up),
            r0,
            "② 第 {round} 轮重建后可渲染索引总量塌缩（空帧面）"
        );
        let peak = st.transient_peak_bytes.saturating_sub(peak0);
        let bound = steady_resident * 3 / 2 + TRANSIENT_SLACK;
        assert!(
            peak <= bound,
            "② 第 {round} 轮重建瞬态峰值 {peak} B > 稳态×1.5+常数 {bound} B\
             （稳态驻留 {steady_resident} B；旧批拷贝必须随批次替换摘除）"
        );
    }

    // 释放路径：重建（未 flush）后立即 unload——被卸载批次的滞留拷贝
    // 必须当场摘除，瞬态清零。
    for cz in 0..GRID {
        for cx in 0..GRID {
            up.build_chunk(origin(cx, cz), &small, &idx, None);
        }
    }
    let mid = up.batching_stats();
    assert!(mid.transient_bytes > 0, "② 前置未成立：重建后应有在途拷贝");
    for cz in 0..GRID {
        for cx in 0..GRID {
            up.unload(ChunkPos { x: cx, z: cz });
        }
    }
    let st = up.batching_stats();
    assert_eq!(
        st.transient_bytes, 0,
        "② 卸载后在途拷贝滞留 {} B（死缓冲的拷贝是纯泄漏）",
        st.transient_bytes
    );
    assert_eq!(st.cpu_pool_bytes, 0, "② 卸载后池未清零");
    assert!(up.entries().is_empty(), "② 卸载后条目未清空");
    up.flush();
    eprintln!(
        "K-MEM ② churn: steady_resident={steady_resident} B peak={} B → ok",
        up.batching_stats()
            .transient_peak_bytes
            .saturating_sub(peak0)
    );
    eprintln!("K-MEM render-mem: all assertions ok");
}
