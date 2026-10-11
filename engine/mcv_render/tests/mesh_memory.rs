//! 整区块重构刀 5a（任务板 #115）：RD=8 网格域字节记账实测 + 预算断言。
//!
//! `harness = false`：stdout 不经测试 harness 捕获，CI 绿跑也落日志——
//! `K5-MEAS` 行即 mesh 字节实测值（本刀交付证据）。CI 侧跑在 lavapipe
//! （render-headless job，gpu-tests feature；workspace test job 因
//! `required-features` 不构建本 target）。
//!
//! 场景口径：生产 remesh 环 = sim_dist = rd+1（game.rs 环过滤「可见网格
//! 半径 ≤ sim_dist」）→ 桌面默认 rd=8 的网格域 = 19²=361 块（比可见环
//! 17²=289 多一圈支撑网格）。合成地形（双频正弦高度场 + 海面水柱）经
//! 真 mcv_mesher 双 pass 产出顶点/索引字节，走生产同款
//! `MeshUploader::build_chunk` + `flush`（staging 环）。
//!
//! 断言语义 = 合成场景回归门（catch 记账量级突变/泄漏），不是真机内存
//! 保证——真实地形（树冠/洞穴/水）网格更大，真机侧以 render-stats 日志
//! 的 resident_kb/pool_kb 取证（docs/real-machine-checklist.md）。断言数
//! 按派单纪律「实跑一轮取实测值后定断言数并留 2 倍余量」：首跑 run
//! 38114958737 实测 48.34 MiB（40 MiB 暂定门如约触发取证红），收紧为
//! 2× 实测 = 96 MiB（MESH_BUDGET_MIB 注释登记实测值与 run 号）。

use std::f32::consts::TAU;

use mcv_core::{CHUNK_VOL, vidx};
use mcv_mesher::{MESH_OPAQUE, MESH_WATER, Mesher, Slot};
use mcv_render::gpu::MeshUploader;

/// 桌面默认视距（options.rs 桌面分支；移动端 6 归 memory_budget.rs 的
/// vd6 档锁，网格域同式缩环）。
const RD: i32 = 8;
/// 网格环半径 = sim_dist = rd+1（game.rs remesh 环过滤口径）。
const RING: i32 = RD + 1;
/// 网格块数 = (2*RING+1)² = 19² = 361。
const MESH_CHUNKS: usize = ((2 * RING + 1) * (2 * RING + 1)) as usize;
/// 首跑暂定派单估值 40 MiB → 实测后按「2× 实测」收紧：run 38114958737
/// （7a35424，lavapipe，361 块）实测 resident_bytes = cpu_pool_bytes =
/// **50 690 400 B ≈ 48.34 MiB**（draw_entries=51，批均 ~7.1 块），断言
/// 取整 96 MiB ≈ 2×。超门 = 记账量级突变/泄漏回归。
const MESH_BUDGET_MIB: u64 = 96;
/// 海平面：高度场 60..71 起伏，65 以下成水域（水 pass 有真实输入）。
const SEA_LEVEL: i32 = 65;

/// 确定性高度场：双频正弦 + 逐列哈希抖动（无外部 RNG 依赖，跨平台同值）。
fn terrain_height(wx: i32, wz: i32) -> i32 {
    let fx = (wx as f32 / 23.0) * TAU;
    let fz = (wz as f32 / 17.0) * TAU;
    let jitter = ((wx.wrapping_mul(7).wrapping_add(wz.wrapping_mul(13)) % 9) + 9) % 9;
    64 + (6.0 * fx.sin() + 4.0 * fz.sin()).round() as i32 + jitter / 4
}

/// 体素场寻块：场按 cz 外/cx 内铺 `(2*RING+3)²` 块，原点 (-RING-1, -RING-1)。
fn field_at<'a>(field: &'a [Vec<u16>], side: usize, cx: i32, cz: i32) -> &'a Vec<u16> {
    let ix = (cx + RING + 1) as usize;
    let iz = (cz + RING + 1) as usize;
    &field[iz * side + ix]
}

fn device_queue() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no adapter: install mesa-vulkan-drivers for lavapipe");
    // wgpu 30：request_device 单参（照 render.rs 同款；图集层数上限与本测
    // 无关——MeshUploader 只建网格缓冲，不建纹理数组）。
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("mesh-memory"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("device");
    (device, queue)
}

fn main() {
    let (device, queue) = device_queue();
    let mut uploader = MeshUploader::new(device.clone(), queue.clone());

    // 方块 id 按注册表名解析（mesher 内部同款查法）：石=实体柱，水=海面。
    let stone = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "stone")
        .expect("registry has stone") as u16;
    let water = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "water")
        .expect("registry has water") as u16;

    // 世界体素场：(2*RING+3)² 块（网格环 19² + 每侧 1 圈邻域供给 3×3）。
    let side = (2 * RING + 3) as usize;
    let mut field: Vec<Vec<u16>> = Vec::with_capacity(side * side);
    for cz in -RING - 1..=RING + 1 {
        for cx in -RING - 1..=RING + 1 {
            let mut v = vec![0u16; CHUNK_VOL];
            for z in 0..16usize {
                for x in 0..16usize {
                    let wx = cx * 16 + x as i32;
                    let wz = cz * 16 + z as i32;
                    let h = terrain_height(wx, wz).clamp(-60, 300);
                    for y in mcv_core::WORLD_MIN_Y..h {
                        v[vidx(x, y, z)] = stone;
                    }
                    if h < SEA_LEVEL {
                        for y in h..=SEA_LEVEL {
                            v[vidx(x, y, z)] = water;
                        }
                    }
                }
            }
            field.push(v);
        }
    }

    // 全域天光（照 RustMesher 占位：高半字节 sky=15）。
    let light: Vec<u8> = vec![0xF0; CHUNK_VOL];

    let mesher = Mesher::new();
    let mut meshed = 0usize;
    for cz in -RING..=RING {
        for cx in -RING..=RING {
            // 行主序 dz 外层/dx 内层，index 4 = 中心块（Mesher::build 契约）。
            let mut slots: [Option<Slot<'_>>; 9] = std::array::from_fn(|_| None);
            for dz in -1i32..=1 {
                for dx in -1i32..=1 {
                    slots[((dz + 1) * 3 + (dx + 1)) as usize] = Some(Slot {
                        voxels: field_at(&field, side, cx + dx, cz + dz),
                        light: &light,
                    });
                }
            }
            let origin = [16.0 * cx as f32, 0.0, 16.0 * cz as f32];
            let opaque = mesher
                .build(&slots, MESH_OPAQUE)
                .expect("opaque mesh build");
            let water_mesh = mesher.build(&slots, MESH_WATER).expect("water mesh build");
            let wf = if water_mesh.indices().is_empty() {
                None
            } else {
                Some((water_mesh.vertex_data(), water_mesh.indices()))
            };
            uploader.build_chunk(origin, opaque.vertex_data(), opaque.indices(), wf);
            meshed += 1;
        }
    }
    uploader.flush();

    let st = uploader.batching_stats();
    let mib = st.resident_bytes as f64 / (1024.0 * 1024.0);
    let pool_mib = st.cpu_pool_bytes as f64 / (1024.0 * 1024.0);
    eprintln!(
        "K5-MEAS mesh: rd={RD} ring={RING} meshed_chunks={} draw_entries={} resident_bytes={} ({mib:.2} MiB) cpu_pool_bytes={} ({pool_mib:.2} MiB) budget={MESH_BUDGET_MIB} MiB",
        st.meshed_chunks, st.draw_entries, st.resident_bytes, st.cpu_pool_bytes
    );

    assert_eq!(meshed, MESH_CHUNKS, "网格域块数不符（环口径漂移）");
    assert_eq!(
        st.meshed_chunks, MESH_CHUNKS,
        "uploader 池记账块数不符（入池/卸载记账漂移）"
    );
    assert!(st.draw_entries > 0, "批次条目为空（合批记账断裂）");
    assert!(
        st.resident_bytes > 0 && st.cpu_pool_bytes > 0,
        "字节记账恒零（现算路径断裂）"
    );
    assert!(
        st.resident_bytes <= MESH_BUDGET_MIB * 1024 * 1024,
        "GPU 网格域 {mib:.2} MiB 超预算 {MESH_BUDGET_MIB} MiB（合成场景回归门；\
         记账口径见 BatchingStats.resident_bytes 文档）"
    );
    assert!(
        st.cpu_pool_bytes <= MESH_BUDGET_MIB * 1024 * 1024,
        "CPU 池 {pool_mib:.2} MiB 超预算 {MESH_BUDGET_MIB} MiB（#80 无 LRU 池计入面）"
    );
    eprintln!("K5-MEAS mesh: all assertions ok");
}
