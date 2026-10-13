//! 真机隐形墙取证（2026-10-12 用户报告，build f4cdec80）：洞穴灌水期
//! 「石墙整面透明不可见、可穿过碰撞在场、打掉一块整块显形、走近偶愈」。
//!
//! 本测锁**跨层契约**：游戏层 `build_chunk` 提交的区块几何，在游戏层
//! 自己 `unload` 之前必须保持可绘制。渲染内存治理波 ① 的 CPU 池预算
//! LRU（`enforce_cpu_pool_budget`）把「越预算逐出」当作泄漏兜底，注释
//! 口径「RD=8 稳态（48.34 MiB < 60 MiB）永不触发」的标定依据是合成
//! 高度场（`mesh_memory.rs`，无洞穴、无灌水）。真机洞穴灌水地形（洞壁
//! 面，灌水空腔的水面与水岩界面双 pass）单块网格字节显著更高，稳态池
//! 越预算后每次 `build_chunk` 尾部逐出一个最旧块，即「玩家周围最先
//! 加载的块」；游戏层 meshed 集/stage=Uploaded/MESH 脏位对此一无所知
//! （`MeshStats` 无 evicted 面、无重网格通知），该块直到下一次编辑或
//! 边同步置 MESH 脏才重建——正是用户报告的三个现象。
//!
//! 场景 = 真 mcv_mesher 产线的洞穴+灌水断面（RD=8 网格环 361 块），再
//! 按实测单块字节续传至预算越界，随后断言契约：**游戏从未卸载任何块
//! → 一个都不许瞎**。红 = 逐出即失明（症状锁），绿 = 契约成立（治理
//! 波必须改道：逐出须通知游戏层置脏重建，或预算门只清可再生产物）。
//!
//! 无 GPU 环境（workspace test job）无 adapter → 打印 SKIP 跳过；
//! render-headless（lavapipe）必跑。不用 wgpu 截图，纯计数/字节断言。

use mcv_core::{CHUNK_VOL, vidx};
use mcv_mesher::{MESH_OPAQUE, MESH_WATER, Mesher, Slot};
use mcv_render::gpu::{CPU_POOL_BUDGET_BYTES, MeshUploader};

const RING: i32 = 9; // 与 mesh_memory 的 RD=8 网格环同径（19²=361）
const SEA: i32 = 63;

fn device_queue_opt() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("inviswall-eviction"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .ok()?;
    Some((device, queue))
}

/// 地表高度场（mesh_memory 同款双频正弦 + 哈希抖动，含水下盆地）。
fn terrain_height(wx: i32, wz: i32) -> i32 {
    use std::f32::consts::TAU;
    let fx = (wx as f32 / 23.0) * TAU;
    let fz = (wz as f32 / 17.0) * TAU;
    let jitter = ((wx.wrapping_mul(7).wrapping_add(wz.wrapping_mul(13)) % 9) + 9) % 9;
    64 + (6.0 * fx.sin() + 4.0 * fz.sin()).round() as i32 + jitter / 4
}

/// 哈希（洞穴凿空用；确定性、跨平台同值）。
fn hash3(wx: i32, y: i32, wz: i32) -> u32 {
    let h = (wx as u32).wrapping_mul(73_856_093)
        ^ (y as u32).wrapping_mul(19_349_663)
        ^ (wz as u32).wrapping_mul(83_492_791);
    h.wrapping_mul(2_246_822_519).rotate_left(13)
}

/// 洞穴 + 灌水断面：地表以下按 4³ 粗格哈希凿空（~15% 体积、连通感洞
/// 腔），海平面以下的空腔一律灌水——用户场景「海平面以下洞穴灌水」的
/// 字节化（洞壁面、水空界面与水岩界面，opaque 与 water 双 pass 全负载）。
fn cave_water_field(cx: i32, cz: i32, stone: u16, water: u16) -> Vec<u16> {
    let mut v = vec![0u16; CHUNK_VOL];
    for z in 0..16usize {
        for x in 0..16usize {
            let wx = cx * 16 + x as i32;
            let wz = cz * 16 + z as i32;
            let h = terrain_height(wx, wz).clamp(-60, 300);
            for y in mcv_core::WORLD_MIN_Y..h {
                let carved = y < h - 3 && hash3(wx >> 2, y >> 2, wz >> 2) % 100 < 15;
                if carved {
                    if y <= SEA {
                        v[vidx(x, y, z)] = water;
                    }
                } else {
                    v[vidx(x, y, z)] = stone;
                }
            }
            if h < SEA {
                for y in h..=SEA {
                    v[vidx(x, y, z)] = water;
                }
            }
        }
    }
    v
}

fn field_at(field: &[Vec<u16>], side: usize, cx: i32, cz: i32) -> &Vec<u16> {
    &field[((cz + RING + 1) as usize) * side + (cx + RING + 1) as usize]
}

/// 阶段 2 续传样本 = 阶段 1 真产物字节（opaque 顶点/索引 + water
/// 顶点/索引，顶点 28B 步长交错 = 真 TERRAIN_STRIDE 形态（#191 顶点色））。
struct Sample {
    v: Vec<u8>,
    idx: Vec<u32>,
    wv: Vec<u8>,
    wi: Vec<u32>,
}

#[test]
fn cave_density_stream_blinds_committed_chunks() {
    let Some((device, queue)) = device_queue_opt() else {
        eprintln!("K-INVIS SKIP: no wgpu adapter (headless test job has no lavapipe)");
        return;
    };
    let stone = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "stone")
        .expect("registry has stone") as u16;
    let water = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "water")
        .expect("registry has water") as u16;

    let side = (2 * RING + 3) as usize;
    let mut field: Vec<Vec<u16>> = Vec::with_capacity(side * side);
    for cz in -RING - 1..=RING + 1 {
        for cx in -RING - 1..=RING + 1 {
            field.push(cave_water_field(cx, cz, stone, water));
        }
    }
    let light: Vec<u8> = vec![0xF0; CHUNK_VOL];

    let mesher = Mesher::new();
    let mut uploader = MeshUploader::new(device.clone(), queue.clone());

    // 阶段 1：RD=8 网格环 361 块真 mesher 产物全量提交（游戏层视角 =
    // 加载完成，一次 unload 都没有）。
    let mut built = 0usize;
    let mut mesh_bytes_sum: u64 = 0;
    let mut samples: Vec<Sample> = Vec::new();
    for cz in -RING..=RING {
        for cx in -RING..=RING {
            let mut slots: [Option<Slot<'_>>; 9] = std::array::from_fn(|_| None);
            for dz in -1i32..=1 {
                for dx in -1i32..=1 {
                    slots[((dz + 1) * 3 + (dx + 1)) as usize] = Some(Slot {
                        voxels: field_at(&field, side, cx + dx, cz + dz),
                        light: &light,
                    });
                }
            }
            let opaque = mesher.build(&slots, MESH_OPAQUE).expect("opaque");
            let wm = mesher.build(&slots, MESH_WATER).expect("water");
            let bytes = (opaque.vertex_data().len()
                + opaque.indices().len() * 4
                + wm.vertex_data().len()
                + wm.indices().len() * 4) as u64;
            mesh_bytes_sum += bytes;
            let origin = [16.0 * cx as f32, 0.0, 16.0 * cz as f32];
            let wf = if wm.indices().is_empty() {
                None
            } else {
                Some((wm.vertex_data(), wm.indices()))
            };
            uploader.build_chunk(origin, opaque.vertex_data(), opaque.indices(), wf);
            samples.push(Sample {
                v: opaque.vertex_data().to_vec(),
                idx: opaque.indices().to_vec(),
                wv: wm.vertex_data().to_vec(),
                wi: wm.indices().to_vec(),
            });
            built += 1;
        }
    }
    let avg = mesh_bytes_sum / built as u64;
    let st = uploader.batching_stats();
    eprintln!(
        "K-INVIS cave: chunks={built} avg_chunk_bytes={avg} \
         cpu_pool_bytes={} ({:.2} MiB) budget={CPU_POOL_BUDGET_BYTES} ({:.2} MiB) \
         evicted_chunks={} meshed_chunks={} draw_entries={}",
        st.cpu_pool_bytes,
        st.cpu_pool_bytes as f64 / (1024.0 * 1024.0),
        CPU_POOL_BUDGET_BYTES as f64 / (1024.0 * 1024.0),
        st.evicted_chunks,
        st.meshed_chunks,
        st.draw_entries,
    );

    // 阶段 2：洞穴洪流取证——真机加载不止一帧铺满 361 块，池字节随
    // 玩家推进持续增长。注意 `enforce_cpu_pool_budget` 在每次
    // build_chunk 尾部把池修剪回 ≤ 预算，池字节观测值恒不越界——越界
    // 的取证面是 evicted_chunks 计数，故以「观察到逐出」为续传终止
    // 条件（逐出若已发生在阶段 1，本段直接跳过，契约断言看最终态）。
    let mut extra = 0usize;
    while uploader.batching_stats().evicted_chunks == 0 && extra < 4096 {
        let s = &samples[extra % samples.len()];
        // 远处空位单块提交（模拟远端新块入环；不影响近端批次）。
        let gx = 2048 + (extra % 512) as i32;
        let gz = 2048 + (extra / 512) as i32;
        let water_arg = if s.wi.is_empty() {
            None
        } else {
            Some((&s.wv[..], &s.wi[..]))
        };
        uploader.build_chunk(
            [16.0 * gx as f32, 0.0, 16.0 * gz as f32],
            &s.v,
            &s.idx,
            water_arg,
        );
        extra += 1;
        assert!(extra < 8192, "顶-up 未触发逐出（预算门记账漂移？）");
    }
    uploader.flush();

    // —— 跨层契约断言（隐形墙锁）——
    // 游戏层视角：built+extra 个区块全部 build_chunk 提交、零 unload、
    // 零编辑。渲染层若静默逐出任何一块，游戏层无通知面（f4cdec80 的
    // MeshStats 无 evicted 字段）、meshed 集仍记「已建」、stage=Uploaded、
    // MESH 干净 → 该块永不重建 = 用户看到的隐形墙（编辑 remesh 才自愈）。
    let st = uploader.batching_stats();
    let committed = (built + extra) as u64;
    eprintln!(
        "K-INVIS verdict: committed={committed} meshed_chunks={} evicted_chunks={} \
         evicted_bytes={} cpu_pool_bytes={}",
        st.meshed_chunks, st.evicted_chunks, st.evicted_bytes, st.cpu_pool_bytes,
    );
    assert_eq!(
        st.evicted_chunks,
        0,
        "隐形墙症状锁：游戏零 unload 场景下渲染层逐出 {evicted} 块网格 \
         （evicted_bytes={eb}）。LRU 逐出走整体卸载路径但无任何面向游戏层 \
         的通知——被逐块 stage=Uploaded / meshed 记账在场 / MESH 干净，\
         永不重建，直到玩家编辑该块或邻块边同步置脏才『打掉一块全显形』。 \
         修复方向：①逐出通知游戏层置 MESH 脏（回灌 remesh）；②预算门 \
         改为可再生产物优先 + 与 CPU_POOL_BUDGET 的真机洞穴标定（当前 \
         60 MiB 门按无洞穴合成场标定，洞穴灌水地形稳态即越界）。",
        evicted = st.evicted_chunks,
        eb = st.evicted_bytes,
    );
    assert_eq!(
        st.meshed_chunks as u64, committed,
        "池块数记账漂移（与 evicted 同源症状）：committed={committed}"
    );
}
