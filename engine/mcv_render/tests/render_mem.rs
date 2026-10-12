//! 波「渲染内存治理」（任务 ①/②）：CPU 网格字节有界回收 + 批次重建瞬态
//! 放大治理。
//!
//! `harness = false`（同 mesh_memory.rs）：K-MEM 行落 CI 日志可取证；
//! 走 render-headless job（lavapipe，gpu-tests feature），纯 headless
//! 路径（MeshUploader::build_chunk/unload/flush），不开窗口不截图。
//!
//! 断言全部走计数/字节统计（trimmed_chunks/trimmed_bytes/evicted_chunks/
//! live_vertex_peak_bytes/transient_bytes），不靠墙钟：
//! - ②（本文件先行段）：重建窗口「新旧批并存」的顶点字节峰值
//!   live_vertex_peak_bytes ≤ 稳态驻留 × 1.5 + 常数（治理 = 先摘旧批再
//!   建新批；未治理时旧批在新批建成入册前始终挂在条目表上，push+rebuild
//!   的初次建批即 2× 并存）。契约注记：build_chunk 返回的 RenderChunk 是
//!   条目克隆，调用方可以在重建后仍绘制旧克隆（render.rs 水 pass/木族样本
//!   两个像素测即此语义）——被替换世代的在途拷贝必须保留到 flush 送达，
//!   只有「从未出册的世代」（同调用内 push+rebuild 的内部世代）与「chunk
//!   死亡（卸载/逐出）」才允许摘除拷贝。
//! - ①（#174 改版口径）：只 build 不 unload 的滞留场景下，池字节必须被
//!   预算门控住（≤ CPU_POOL_BUDGET_BYTES）；越界只修剪**保留环外**
//!   （默认锚点 (0,0)、半径 DEFAULT_RETENTION_RADIUS=11）的远场滞留，
//!   修剪有计数实证（trimmed_chunks ≥ 1）且被逐块入 remesh 债账；
//!   环内盲逐出警报面 evicted_chunks 恒 0（隐形墙契约）；
//!   全量 unload + flush 后池/条目/瞬态/债账清零。
//! - 释放路径：卸载即摘当前世代在途拷贝（句柄失效语义），瞬态字节严格
//!   下降；池与条目清零。

use mcv_core::ChunkPos;
use mcv_render::gpu::{MeshUploader, TERRAIN_STRIDE};

/// ① 预算门控的预算（集中定义在 gpu.rs；#174 重标定 = 合成稳态 48.34
/// MiB × 2 = 96 MiB，旧 60 MiB 按无洞穴合成场标定已被洞穴实测证伪）。
const BUDGET: u64 = mcv_render::gpu::CPU_POOL_BUDGET_BYTES;
/// 滞留场景单块顶点字节：200_000 顶点 × 24B ≈ 4.8 MiB（> MAX_BATCH_BYTES
/// 4 MiB → 恒单块批次，字节记账最直白）。
const BIG_VERTS: usize = 200_000;
const BIG_BYTES: usize = BIG_VERTS * TERRAIN_STRIDE;
/// ② 稳态场景：1×8 一行 8 块、单块 ≈ 2_800 顶点 ≈ 65 KiB → 合成单一
/// 8 成员批次（并存放大最大化：旧+新双批 = 2× 稳态）。
const SMALL_VERTS: usize = 2_800;
const ROW: i32 = 8;
/// ② 峰值公式的常数余量（缓冲 4B 对齐补零 + 占位条目 8B 级别）。
const PEAK_SLACK: u64 = 64 * 1024;

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

    // ================= ② 重建瞬态放大治理 =================
    // 场景：1×8 单批次。steady_resident = 全量 flush 后的 resident_bytes。
    let mut up = MeshUploader::new(device.clone(), queue.clone());
    let small = fake_verts(SMALL_VERTS);
    let idx = fake_idx();
    for cx in 0..ROW {
        up.build_chunk(origin(cx, 0), &small, &idx, None);
    }
    up.flush();
    let steady = up.batching_stats();
    assert_eq!(steady.draw_entries, 1, "② 稳态未合成单一 8 成员批次");
    let s = steady.resident_bytes;
    let e0 = up.entries().len();
    let r0 = renderable_opaque(&up);
    eprintln!(
        "K-MEM ② steady: entries={e0} resident={s} B pool={} B opaque_idx={r0}",
        steady.cpu_pool_bytes
    );

    // (A) 新旧批并存峰值：顶点字节账峰值 ≤ 稳态×1.5+常数。未治理实现中
    //     初次建批（push 后再 rebuild）与每次重网格都是旧批在新批建成时
    //     仍挂在条目表上，峰值 ≥ 稳态+单批 = 2× 稳态（本场景 1 批）。
    let peak = steady.live_vertex_peak_bytes;
    let bound = s * 3 / 2 + PEAK_SLACK;
    assert!(
        peak <= bound,
        "②(A) 顶点字节峰值 {peak} B > 稳态×1.5+常数 {bound} B（稳态 {s} B）——\
         重建窗口必须先摘旧批再建新批，不许新旧双批并存"
    );

    // (B) 生产节奏（重建后同帧 flush）× 3 轮：条目数与可渲染索引总量
    //     不塌（「重建窗口不渲染空帧」——重建同步完成，观测面永无空槽）。
    for round in 0..3u64 {
        for cx in 0..ROW {
            up.build_chunk(origin(cx, 0), &small, &idx, None);
        }
        assert_eq!(
            up.entries().len(),
            e0,
            "②(B) 第 {round} 轮重建后条目数塌缩（空帧面）"
        );
        assert_eq!(
            renderable_opaque(&up),
            r0,
            "②(B) 第 {round} 轮重建后可渲染索引总量塌缩（空帧面）"
        );
        up.flush();
    }
    let st = up.batching_stats();
    eprintln!(
        "K-MEM ② churn: rounds=3 live_peak={} B bound={bound} B transient={} B",
        st.live_vertex_peak_bytes, st.transient_bytes
    );
    assert!(
        st.live_vertex_peak_bytes <= bound,
        "②(A) 重建轮后峰值 {} B 越界 {bound} B",
        st.live_vertex_peak_bytes
    );
    // (B) 收尾：清场回空 uploader（否则 (C) 的隔格坐标会落进 (B) 的
    // 8 成员批次，账目混合）。顺带实证：节奏末卸载池/条目清零。
    for cx in 0..ROW {
        up.unload(ChunkPos { x: cx, z: 0 });
    }
    up.flush();
    let st = up.batching_stats();
    assert_eq!(st.cpu_pool_bytes, 0, "②(B) 收尾卸载后池未清零");
    assert!(up.entries().is_empty(), "②(B) 收尾卸载后条目未清空");

    // (C) 释放路径：重建（不 flush）后立即卸载——当前世代的在途拷贝
    //     必须当场摘除（chunk 死亡 = 句柄失效，无人再绘），瞬态字节严格
    //     下降；池与条目清零。注记：已出册世代（build_chunk 返回过克隆）
    //     的拷贝按契约保留到 flush（render.rs 像素测依赖），故只断严格
    //     下降而非清零。场景取隔一格布局（互不相邻 → 各自单块批次），
    //     卸载逐条删批次、不触发邻居重建，字节账干净。
    for cx in 0..ROW {
        up.build_chunk(origin(cx * 2, 0), &small, &idx, None);
    }
    for cx in 0..ROW {
        up.build_chunk(origin(cx * 2, 0), &small, &idx, None);
    }
    let mid = up.batching_stats();
    assert!(
        mid.transient_bytes > 0,
        "②(C) 前置未成立：重建后应有在途拷贝"
    );
    for cx in 0..ROW {
        up.unload(ChunkPos { x: cx * 2, z: 0 });
    }
    let st = up.batching_stats();
    assert!(
        st.transient_bytes < mid.transient_bytes,
        "②(C) 卸载后在途拷贝未摘除（{} B ≥ 卸载前 {} B，死缓冲的拷贝是纯泄漏）",
        st.transient_bytes,
        mid.transient_bytes
    );
    assert_eq!(st.cpu_pool_bytes, 0, "②(C) 卸载后池未清零");
    assert!(up.entries().is_empty(), "②(C) 卸载后条目未清空");
    up.flush();

    // ================= ① CPU 网格字节有界回收（#174 改版口径） ==========
    // 只 build 不 unload 的滞留场景：24 块 × 4.8 MB ≈ 115 MB > 96 MiB
    // 预算。默认锚点 (0,0)、保留半径 11 → 块 0..=11 在环内（绝对保护，
    // 合计 ≈ 55 MiB < 预算，池门控可达标），块 ≥12 在环外——越预算部分
    // 必须由**环外远场修剪**释放（原版 processUnloads 距离驱动卸载的
    // 本仓等价物），且逐出转入 remesh 债账（drain 前 meshed 记账不减）。
    let mut uploader = MeshUploader::new(device, queue);
    let big = fake_verts(BIG_VERTS);
    let n_big = 24;
    for i in 0..n_big {
        uploader.build_chunk(origin(i, 0), &big, &idx, None);
        uploader.flush();
        let st = uploader.batching_stats();
        assert!(
            st.cpu_pool_bytes <= BUDGET,
            "① 第 {i} 块 build 后 CPU 池 {} B 超预算 {BUDGET} B（环外滞留必须由修剪释放，不许无界驻留）",
            st.cpu_pool_bytes
        );
    }
    let st = uploader.batching_stats();
    let mib = st.cpu_pool_bytes as f64 / (1024.0 * 1024.0);
    eprintln!(
        "K-MEM ① leak-churn: built={n_big} pool={} B ({mib:.2} MiB) budget={BUDGET} B \
         trimmed_chunks={} trimmed_bytes={} evicted_chunks={} debt_chunks={} entries={}",
        st.cpu_pool_bytes,
        st.trimmed_chunks,
        st.trimmed_bytes,
        st.evicted_chunks,
        st.debt_chunks,
        st.draw_entries
    );
    assert!(
        st.trimmed_chunks >= 1,
        "① 越预算场景环外修剪计数恒 0（释放路径未接线，cpu_pool_bytes 靠运气达标不算实证）"
    );
    // 修剪量级：池稳在预算附近 → 环外仅留预算内尾部 ~21 块在场，
    // 至少 3 块被剪（实测 4；取 3 留平局判据的 ±1 余量）。
    assert!(
        st.trimmed_bytes >= 3 * BIG_BYTES as u64,
        "① 修剪字节统计与越界量级不符（记账漂移）"
    );
    // 隐形墙契约警报面：环内块被盲逐出 = #174 回归，必须恒 0。
    assert_eq!(
        st.evicted_chunks, 0,
        "① 保留环内出现逐出（近场失明，隐形墙回归——修剪只许发生在环外）"
    );
    // 债账在场：修剪未 drain 前 committed 记账不丢（inviswall 契约同式）。
    assert_eq!(
        st.meshed_chunks as u64, n_big,
        "① 修剪后 committed 记账（池+债）应仍等于 build 总数"
    );
    assert_eq!(
        st.debt_chunks as u64, st.trimmed_chunks,
        "① 债账条数与修剪计数不一致"
    );
    // 释放路径：全量 unload + flush → 池/条目/瞬态/债账清零。
    for i in 0..n_big {
        uploader.unload(ChunkPos { x: i, z: 0 });
    }
    uploader.flush();
    let st = uploader.batching_stats();
    assert_eq!(st.cpu_pool_bytes, 0, "① 全量 unload 后池未清零");
    assert_eq!(st.draw_entries, 0, "① 全量 unload 后条目未清零");
    assert_eq!(st.meshed_chunks, 0, "① 块数记账漂移");
    assert_eq!(st.debt_chunks, 0, "① 全量 unload 后债账未销");
    assert_eq!(st.transient_bytes, 0, "① flush 后瞬态滞留未清零");

    eprintln!("K-MEM render-mem: all assertions ok");
}
