//! 真机隐形墙取证波（2026-10-12 用户报告）：洞穴水线下方「石墙整面透明、
//! 打掉一块即显形」——指纹 = 首建网格消费了空/旧体素快照，自愈靠
//! 编辑/边活动 remesh。本文件是**红测试取证**（不改共享接线，只用 pub
//! API 与既有测试装配），锁两条排除线：
//!
//! 1. **E7a palette 解码锁**：水下洞穴/水线断面的真实形状（同 section
//!    >16 相异值换 5bit 档、>256 相异值进 global 档、状态 nibble 高 4 位、
//!    负 y section）经 `from_flat_u16`（commit_terrain / load_voxels 入库
//!    路）与逐格 `set_flat`（编辑路）两条写入路后，`decode_column`
//!    （= 网格快照 `snapshot_neighborhood` 的唯一读法）必须与平铺源逐
//!    字节一致。红 = palette 换档/解码错位即隐形墙根因。
//! 2. **首建网格快照锁（真地形端到端）**：真 Vanilla worldgen 经完整
//!    stream（gen commit → 光照 init → 3×3 门槛 → worker 派发）静默后，
//!    中心块最后回传产物必须与该块**当前**体素直接建出的网格逐字节
//!    相同且非空。红 = 首建消费了空/旧快照（数据在场、几何缺席）。
//!
//! 两条全绿 → 「存储/快照消费路无空快照」排除成立，根因指向渲染层
//! 网格驻留被静默回收（见 engine/mcv_render/tests/inviswall_eviction.rs）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mcv_core::{BlockId, CHUNK_VOL, ChunkHandle, ChunkPos, Stage, VoxelStore, WORLD_MIN_Y, vidx};
use mcv_logic::game::{GameMode, GamePhase, GameRuntime};
use mcv_mesher::{MESH_OPAQUE, MESH_WATER, MeshDone, MeshResults, Mesher, neighborhood_slots};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

fn temp_dir(tag: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-inviswall-{tag}-{:x}", h.finish()))
}

fn block_id(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == name)
        .unwrap_or_else(|| panic!("registry has {name}")) as u16
}

/// 水下洞穴/水线断面（用户场景的字节化）：
/// - section 7（y 48..=63，含水线 63）：石/水/空气/沙/砾石 + 大量带状态
///   nibble 的变体（bit12-15，mcv_core::BlockId::with_state 同款编码），
///   同 section 相异值 >16 → 强制 linear(4bit) → hashmap(5bit) 换档；
/// - section 12（y 128..=143）：300 个相异值 → global(16bit) 档；
/// - section 3 / 全石：单值 0bit 档（全水柱空气段 / 深部全石段）；
/// - 负 y（section 0，y -64..=-49）洞穴：空气/石/水混合。
fn underwater_cave_flat() -> Vec<u16> {
    let stone = block_id("stone");
    let water = block_id("water");
    let sand = block_id("sand");
    let gravel = block_id("gravel");
    let dirt = block_id("dirt");
    let mut flat = vec![0u16; CHUNK_VOL];
    // 全石单值 section（deepslate 替代：直接石）+ 全空气 section 保持 0。
    let sy_stone = 3usize;
    for i in 0..4096usize {
        flat[sy_stone * 4096 + i] = stone;
    }
    // 水线断面：>16 相异值（含状态 nibble 变体）。
    let sy_wl = 7usize; // y 48..=63，绝对 y = WORLD_MIN_Y + 7*16 = 48
    let variants: Vec<u16> = (0..20u16)
        .map(|k| match k {
            0 => stone,
            1 => water,
            2 => 0, // air
            3 => sand,
            4 => gravel,
            5 => dirt,
            _ => stone | (k << 12), // 状态 nibble 变体（半砖/楼梯类编码）
        })
        .collect();
    for yl in 0..16usize {
        for z in 0..16usize {
            let y_abs = WORLD_MIN_Y + (sy_wl * 16) as i32 + yl as i32;
            for x in 0..16usize {
                let v = variants[(x * 7 + z * 5 + yl * 3) % variants.len()];
                flat[vidx(x, y_abs, z)] = v;
                // 水线以下统一再叠一层水（模拟洞穴灌水面的重复水值）。
                if y_abs <= 63 && (x + z) % 3 == 0 {
                    flat[vidx(x, y_abs, z)] = water;
                }
            }
        }
    }
    // global 档 section：300 个相异值。
    let sy_g = 12usize;
    for yl in 0..16usize {
        for z in 0..16usize {
            let y_abs = WORLD_MIN_Y + (sy_g * 16) as i32 + yl as i32;
            for x in 0..16usize {
                let k = (x + z * 16 + yl * 3) % 300;
                flat[vidx(x, y_abs, z)] = k as u16 + 1;
            }
        }
    }
    // 负 y 洞穴断面（section 0：y -64..=-49）。
    for yl in 0..16usize {
        for z in 0..16usize {
            let y_abs = WORLD_MIN_Y + yl as i32;
            for x in 0..16usize {
                flat[vidx(x, y_abs, z)] = match (x + z + yl) % 4 {
                    0 => 0,
                    1 => stone,
                    2 => water,
                    _ => stone | (3 << 12),
                };
            }
        }
    }
    flat
}

/// 取证锁 1a：`from_flat_u16`（commit_terrain / load_voxels 入库路）→
/// `decode_column`（网格快照唯一读法）必须逐字节还原平铺源；换档档型
/// 本身顺带取证（水线 5bit 档、global 16bit、单值 0bit）。
#[test]
fn palette_decode_roundtrip_underwater_cave() {
    let flat = underwater_cave_flat();
    let store = VoxelStore::from_flat_u16(&flat);
    assert_eq!(store.decode_column(), flat, "decode_column != 平铺源");
    // 档型取证（与 mcv_core/tests/palette.rs 梯子表同口径）：
    assert_eq!(store.section(3).bits(), 0, "全石段应驻留 0bit 单值档");
    assert!(
        store.section(7).bits() >= 5,
        "水线断面（>16 相异值）必须已换 ≥5bit 档，实际 {}",
        store.section(7).bits()
    );
    assert_eq!(
        store.section(12).bits(),
        16,
        "300 相异值段应落 global 16bit"
    );
    // 逐格读法一致性（get_flat/get 与 decode 同源）。
    for probe in [
        0usize,
        4095,
        4096,
        7 * 4096 + 2050,
        12 * 4096 + 3,
        CHUNK_VOL - 1,
    ] {
        assert_eq!(store.get_flat(probe).0, flat[probe], "get_flat({probe})");
    }
    // clone（Arc<RwLock> 句柄复活的克隆语义）不得漂移。
    assert_eq!(store.clone().decode_column(), flat, "clone 后解码漂移");
}

/// 取证锁 1b：逐格 `set_flat`（编辑/生长写入路）填满后再解码，两条
/// 写入路必须收敛到同一字节（隐形墙若是「写侧换档丢值」在此现形）。
#[test]
fn palette_set_flat_fill_decodes_identical() {
    let flat = underwater_cave_flat();
    let mut store = VoxelStore::new();
    for (i, &v) in flat.iter().enumerate() {
        store.set_flat(i, BlockId(v));
    }
    assert_eq!(store.decode_column(), flat, "set_flat 全量填充解码漂移");
    // 编辑触发的换档中点：水线 section 再插入全新值（可能推 5→6bit）。
    let probe = vidx(3, WORLD_MIN_Y + 7 * 16 + 8, 4);
    store.set_flat(probe, BlockId(0xABCD));
    let out = store.decode_column();
    assert_eq!(out[probe], 0xABCD, "换档中点编辑丢失");
    for i in 0..CHUNK_VOL {
        if i != probe {
            assert_eq!(out[i], flat[i], "换档 copyFrom 损伤 idx={i}");
        }
    }
}

/// 取证锁 1c：真 Vanilla worldgen 产出经 commit_terrain 同款入库
/// （from_flat_u16）必须无损还原——至少一个种子带水（用户场景在水下）。
#[test]
fn generated_terrain_store_roundtrip_lossless() {
    let water = block_id("water");
    let mut water_seen = false;
    for seed in [20261012u64, 7, 99, 1234] {
        let out = generate_terrain_with(TerrainBackend::Vanilla, seed, ChunkPos::new(0, 0))
            .expect("terrain gen");
        let flat = out.voxels.as_u16_slice();
        let store = VoxelStore::from_flat_u16(flat);
        assert_eq!(
            store.decode_column().as_slice(),
            flat,
            "seed {seed}：worldgen→VoxelStore→快照读法 漂移"
        );
        if flat.contains(&water) {
            water_seen = true;
        }
    }
    assert!(water_seen, "取证场景必须有含水种子（水线断面是症状现场）");
}

/// 手工推进到光照就绪的真地形块（streaming.rs lit_chunk 同款：commit +
/// 真 init + LightLocalReady）。
fn lit_generated_chunk(pos: ChunkPos, seed: u64) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(pos));
    let out = generate_terrain_with(TerrainBackend::Vanilla, seed, pos).expect("terrain gen");
    *h.voxels.write().unwrap() = VoxelStore::from_flat_u16(out.voxels.as_u16_slice());
    *h.heightmap.write().unwrap() = out.heightmap;
    {
        let vg = h.voxels.read().unwrap();
        let mut lg = h.light.write().unwrap();
        let hg = h.heightmap.read().unwrap();
        let voxels = vg.decode_column();
        let mut view = mcv_light::LightChunk {
            voxels: &voxels,
            light: &mut lg[..],
            heightmap: &hg[..],
        };
        mcv_light::init(&mut view);
    }
    h.advance_to(Stage::TerrainReady);
    h.advance_to(Stage::LightLocalReady);
    h
}

/// 收流：停止 stream 后把在途产物排空（300ms 无新产物 = 静默；60s 顶）。
fn drain_quiet(results: &mut MeshResults) -> Vec<MeshDone> {
    let t0 = Instant::now();
    let mut all = Vec::new();
    let mut last_rx = Instant::now();
    loop {
        let got = results.drain_all();
        if !got.is_empty() {
            last_rx = Instant::now();
            all.extend(got);
        } else if last_rx.elapsed() >= Duration::from_millis(300) {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(60),
            "worker 产物排空超时"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    all
}

/// 取证锁 2（真地形端到端首建快照）：真 Vanilla 3×3 夹具 + 真 worker
/// 派发；回传静默后，中心块最后产物必须 = 当前体素直接建的网格，且
/// 几何非空。红 = 首建网格消费了空/旧体素快照（症状本体）。
#[test]
fn first_worker_mesh_matches_current_voxels() {
    const SEED: u64 = 20261012;
    let dir = temp_dir("stale-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let (mut rt, mut results) =
        GameRuntime::new_headless_with_mesh(SEED, dir, GameMode::Creative, 1);
    rt.phase = GamePhase::Playing;
    rt.render_dist = 1;
    // 5×5 灯就绪夹具（外圈两环补齐，同 mesh_parallel 夹具纪律：3×3 邻域
    // 门槛 + 邻块的邻块），全部为真 Vanilla 地形。
    for dz in -2i32..=2 {
        for dx in -2i32..=2 {
            let p = ChunkPos::new(dx, dz);
            rt.chunks_mut().insert(p, lit_generated_chunk(p, SEED));
        }
    }
    // 玩家站中心块地表（出生锚点），stream 驱动 remesh 预算流。
    rt.player.pos = glam::Vec3::new(8.5, 70.0, 8.5);
    let center = ChunkPos::new(0, 0);
    // 阶段 1：中心 3×3 全部派发（stage = Uploaded，派发即记账语义）。
    let mut got: Vec<MeshDone> = Vec::new();
    let t0 = Instant::now();
    loop {
        rt.stream();
        got.extend(results.drain_all());
        let up = (-1i32..=1).all(|dz| {
            (-1i32..=1).all(|dx| rt.chunks[&ChunkPos::new(dx, dz)].stage() == Stage::Uploaded)
        });
        if up || t0.elapsed() > Duration::from_secs(60) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        rt.chunks[&center].stage(),
        Stage::Uploaded,
        "首建网格未派发（stream 门槛/预算问题，非本锁范围）"
    );
    // 阶段 2：继续 stream 让「成对边同步 / 生长」等后续 MESH 脏全部
    // 消耗完——静默判据 = 连续 60 帧中心块 MESH 干净且 3×3 无脏。
    let mut quiet = 0usize;
    for _ in 0..600 {
        rt.stream();
        got.extend(results.drain_all());
        let clean = (-1i32..=1).all(|dz| {
            (-1i32..=1)
                .all(|dx| rt.chunks[&ChunkPos::new(dx, dz)].dirty() & mcv_core::dirty::MESH == 0)
        });
        quiet = if clean { quiet + 1 } else { 0 };
        if quiet >= 60 {
            break;
        }
    }
    assert!(
        quiet >= 60,
        "600 帧窗口内 MESH 脏位未收敛（remesh 预算饿死？）"
    );
    got.extend(drain_quiet(&mut results));
    let done = got
        .into_iter()
        .filter(|d| d.pos == center)
        .max_by_key(|d| d.seq)
        .expect("中心块必有 worker 回传产物（派发即记账已观测到）");
    let opaque = done.opaque.expect("非空气场景必有 opaque 产物");

    // 参照：与 snapshot_neighborhood 同款装配（当前夹具状态，静默后 =
    // 派发时点状态），直呼 mesher。
    let vol = CHUNK_VOL;
    let mut handles: [Arc<ChunkHandle>; 9] = core::array::from_fn(|_| rt.chunks[&center].clone());
    for dz in -1i32..=1 {
        for dx in -1i32..=1 {
            handles[((dz + 1) * 3 + (dx + 1)) as usize] = rt.chunks[&ChunkPos::new(dx, dz)].clone();
        }
    }
    let mut voxels = vec![0u16; 9 * vol];
    let mut lights = vec![0xF0u8; 9 * vol];
    for (i, h) in handles.iter().enumerate() {
        voxels[i * vol..(i + 1) * vol].copy_from_slice(&h.voxels.read().unwrap().decode_column());
        lights[i * vol..(i + 1) * vol].copy_from_slice(&h.light.read().unwrap()[..]);
    }
    let slots = neighborhood_slots(&voxels, &lights);
    let mesher = Mesher::new();
    let ref_opaque = mesher.build(&slots, MESH_OPAQUE).expect("kind 校验");
    let ref_water = mesher.build(&slots, MESH_WATER).expect("kind 校验");

    assert!(
        !ref_opaque.indices().is_empty(),
        "参照网格恒空——夹具失效（真地形中心块必有实体面）"
    );
    assert!(
        !opaque.index.is_empty(),
        "首建网格为空 = 隐形墙症状复现（数据在场、几何缺席）"
    );
    assert_eq!(
        opaque.vertex,
        ref_opaque.vertex_data(),
        "首建网格顶点与当前体素漂移（首建消费了空/旧快照）"
    );
    assert_eq!(opaque.index, ref_opaque.indices(), "首建网格索引漂移");
    if let Some(w) = done.water.as_ref() {
        assert_eq!(w.vertex, ref_water.vertex_data(), "水 pass 顶点漂移");
        assert_eq!(w.index, ref_water.indices(), "水 pass 索引漂移");
    }
}
