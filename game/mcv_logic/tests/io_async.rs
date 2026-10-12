//! E4 存/读盘 IO 异步化回归（docs/engine-roadmap.md §3 E4 红测试三件）。
//!
//! 1. **写失败保脏位**：测试替身 IO（region/ 为普通文件 → RegionFile::open
//!    必败）→ SAVE 位保留 → 解除替身后重试成功清位（失败保脏位语义锁，
//!    同步/异步两路都必须满足）。
//! 2. **同帧编辑 + 落盘竞态窗**：快照 A 在途期间再编辑 B → 最终盘上内容 =
//!    最后编辑 B（两段式「编辑再置位则下轮以新快照重写」的行为锁）。
//! 3. **在途读防乒乓**：读盘在途（io_frames 记账）期间该块 Empty 驻留
//!    不卸载，300 帧超时后可卸载（复用 req_frames 同款驻留机制）。
//!
//! 另锁：异步读回走 `advance_to(TerrainReady)` 既有接线（读回内容 = 盘上
//! 内容）与 `MCV_IO_ASYNC` 回滚开关语义（对齐 `MCV_MESHER_WORKERS` 先例）。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use glam::Vec3;
use mcv_core::{BlockId, CHUNK_VOL, ChunkHandle, ChunkPos, Stage, WORLD_MIN_Y, dirty, vidx};
use mcv_logic::game::{GameMode, GameRuntime};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-io-async-{tag}-{:x}", h.finish()))
}

/// 异步路测试装配（独立 IO 线程开启）。
fn headless_io(_tag: &str, dir: std::path::PathBuf) -> GameRuntime {
    GameRuntime::new_headless_with_io(20261010, dir, GameMode::Survival, true)
}

/// 预置一块 ≥TerrainReady 且带 SAVE 位的区块（地板两层 BlockId(1)）。
fn terrain_chunk(pos: ChunkPos, save: bool) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(pos));
    {
        let mut vg = h.voxels.write().unwrap();
        for y in -64..=-63 {
            for zz in 0..16usize {
                for xx in 0..16usize {
                    vg[vidx(xx, y, zz)] = BlockId(1);
                }
            }
        }
    }
    let ids: Vec<u16> = bytemuck::cast_slice(h.voxels.read().unwrap().as_slice()).to_vec();
    *h.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&ids);
    h.advance_to(Stage::TerrainReady);
    if save {
        h.mark_dirty(dirty::SAVE);
    }
    h
}

/// 用同步路把 terrain_chunk 预存到盘上（E4 前语义：save_dirty 即写盘）。
fn presave(tag: &str, pos: ChunkPos) -> std::path::PathBuf {
    let dir = temp_dir(tag);
    let mut rt = GameRuntime::new_headless(20261010, dir.clone(), GameMode::Survival);
    rt.chunks.insert(pos, terrain_chunk(pos, true));
    rt.save_dirty(None);
    drop(rt);
    dir
}

/// 盘上体素读回（None = 无存档/读失败）。
fn disk_ids(dir: &Path, pos: ChunkPos) -> Option<Vec<u16>> {
    let (rx, rz) = mcv_save::chunk_region(pos.x, pos.z);
    let path = dir.join(format!("region/r.{rx}.{rz}.mcrv"));
    if !path.exists() {
        return None;
    }
    let mut region = mcv_save::RegionFile::open(dir, rx, rz).ok()?;
    let mut ids = vec![0u16; CHUNK_VOL];
    let mut hm = [WORLD_MIN_Y as i16; 256];
    region
        .load_chunk(mcv_save::chunk_local(pos.x, pos.z), &mut ids, &mut hm)
        .ok()?;
    Some(ids)
}

/// 帧驱动直到条件满足（每帧一次 stream + 短眠让 IO 线程推进）。
fn pump(rt: &mut GameRuntime, mut cond: impl FnMut(&GameRuntime) -> bool, max: usize) -> bool {
    for _ in 0..max {
        rt.stream();
        if cond(rt) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    cond(rt)
}

/// 红测试 ①：写失败（测试替身 IO）→ SAVE 保留 → 重试成功清位 + 盘上可读回。
#[test]
fn write_failure_keeps_dirty_bit_then_retry_clears() {
    let dir = temp_dir("wfail");
    std::fs::create_dir_all(&dir).unwrap();
    // 测试替身：region/ 是普通文件 → RegionFile::open 的 create_dir_all 必败
    //（与权限无关，root 下同样生效的失败注入）。
    std::fs::write(dir.join("region"), b"not a directory").unwrap();
    let mut rt = headless_io("wfail", dir.clone());
    let pos = ChunkPos::new(3, 5);
    rt.chunks.insert(pos, terrain_chunk(pos, true));

    // 单块投递（不走 stream 的预算写回——避免预算流竞争让脏位稳态不可
    // 观测），再用 drain-only 钩子消化失败回执：投递即清位，回执失败重置。
    rt.save_dirty(Some(pos));
    assert_eq!(rt.io_inflight().0, 1, "单块存盘应已投递在途（两段式写）");
    let mut retained = false;
    for _ in 0..500 {
        rt.io_debug_drain();
        if rt.chunks[&pos].dirty() & dirty::SAVE != 0 && rt.io_inflight() == (0, 0) {
            retained = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    assert!(retained, "写失败后 SAVE 位必须保留（失败保脏位）");
    assert!(disk_ids(&dir, pos).is_none(), "写失败不得伪造盘上成功");

    // 解除替身 → 重试成功 → 清位且盘上可读回（内容 = 快照）。
    std::fs::remove_file(dir.join("region")).unwrap();
    rt.save_dirty(Some(pos));
    let mut cleared = false;
    for _ in 0..500 {
        rt.io_debug_drain();
        if rt.chunks[&pos].dirty() & dirty::SAVE == 0 && rt.io_inflight() == (0, 0) {
            cleared = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    assert!(cleared, "重试成功后 SAVE 位应清");
    let ids = disk_ids(&dir, pos).expect("重试成功后盘上应可读回");
    assert!(ids.contains(&1), "盘上内容应等于快照（地板 BlockId(1)）");
}

/// 红测试 ②：同帧编辑 + 落盘 → 最终盘上内容 = 最后编辑（竞态窗行为锁）。
#[test]
fn edit_during_inflight_write_wins_on_disk() {
    let dir = temp_dir("race");
    let mut rt = headless_io("race", dir.clone());
    // 缩环降噪：预算写回不被生成脏集淹没。pos 取 Chebyshev 3 < rd+4=5
    //（卸载迟滞边界），保证断言窗口内不被驱逐出活动表。
    rt.render_dist = 1;
    let pos = ChunkPos::new(1, 3);
    let handle = terrain_chunk(pos, true);
    rt.chunks.insert(pos, handle.clone());

    // 基线落盘：盘上 = 地板 BlockId(1)。
    let base = pump(
        &mut rt,
        |rt| rt.chunks[&pos].dirty() & dirty::SAVE == 0 && rt.io_inflight().0 == 0,
        500,
    );
    assert!(base, "基线落盘应在预算内完成");
    let ids = disk_ids(&dir, pos).expect("基线盘上可读");
    assert_eq!(ids[vidx(0, -63, 0)], 1);
    assert_eq!(ids[vidx(1, -63, 0)], 0);

    // 编辑 A → 同帧投递快照 A（在途）。
    handle.voxels.write().unwrap()[vidx(0, -63, 0)] = BlockId(42);
    handle.mark_dirty(dirty::SAVE);
    rt.stream();
    assert!(
        rt.io_inflight().0 >= 1,
        "编辑 A 的快照应已投递在途（两段式写）"
    );

    // 竞态窗内再编辑 B（置 SAVE）→ 完成回执后下轮以新快照重写。
    handle.voxels.write().unwrap()[vidx(1, -63, 0)] = BlockId(43);
    handle.mark_dirty(dirty::SAVE);
    let done = pump(
        &mut rt,
        |rt| rt.chunks[&pos].dirty() & dirty::SAVE == 0 && rt.io_inflight().0 == 0,
        500,
    );
    assert!(done, "竞态窗后的重写应在预算内完成");
    let ids = disk_ids(&dir, pos).expect("重写后盘上可读");
    assert_eq!(ids[vidx(0, -63, 0)], 42, "盘上内容 = 快照 A（首次编辑）");
    assert_eq!(
        ids[vidx(1, -63, 0)],
        43,
        "盘上内容 = 最后编辑（在途期间的再编辑必须落盘）"
    );
}

/// 红测试 ③：在途读期间该块 Empty 驻留不卸载（300 帧防乒乓锁）。
#[test]
fn inflight_load_pins_empty_chunk_against_unload() {
    let dir = temp_dir("pin");
    let mut rt = headless_io("pin", dir);
    rt.render_dist = 1; // 缩环降噪：远端请求环不淹没断言
    let pos = ChunkPos::new(0, 0);
    rt.chunks.insert(pos, Arc::new(ChunkHandle::new(pos)));
    // 测试钩子：登记在途读（等价请求环投递后的 io_frames 记账）。
    rt.io_debug_mark_loading(pos);

    // 玩家瞬移远离 → 该块出票环（卸载候选）；在途读未超时不得卸载。
    rt.player.pos = Vec3::new(20.5 * 16.0, 64.0, 8.5);
    for i in 0..299 {
        rt.stream();
        assert!(
            rt.chunks.contains_key(&pos),
            "在途读期间 Empty 驻留失败 @帧 {i}"
        );
    }
    // 第 300 帧超时 → 驻留解除可卸载（防乒乓有界，不永久滞留）。
    rt.stream();
    assert!(
        !rt.chunks.contains_key(&pos),
        "300 帧超时后应可卸载（防乒乓有界）"
    );
    assert_eq!(rt.io_inflight().1, 0, "卸载应同步清在途读记账");
}

/// 异步读回接线：盘上块经独立 IO 线程读回 → advance_to(TerrainReady)，
/// 体素/高度图 = 盘上内容。
#[test]
fn async_load_restores_saved_chunk() {
    let pos = ChunkPos::new(9, -4);
    let dir = presave("roundtrip", pos);
    let mut rt = headless_io("roundtrip", dir);
    rt.render_dist = 1;
    rt.player.pos = Vec3::new(9.0 * 16.0 + 0.5, 64.0, -4.0 * 16.0 + 0.5);

    let loaded = pump(
        &mut rt,
        |rt| {
            rt.chunks
                .get(&pos)
                .is_some_and(|h| (h.stage() as u8) >= (Stage::TerrainReady as u8))
        },
        500,
    );
    assert!(loaded, "投递读应在预算内回填 TerrainReady");
    let h = &rt.chunks[&pos];
    let vg = h.voxels.read().unwrap();
    assert_eq!(vg[vidx(0, -64, 0)], BlockId(1), "体素 = 盘上内容");
    assert_eq!(vg[vidx(0, 100, 0)], BlockId(0));
    drop(vg);
    let hm = h.heightmap.read().unwrap();
    // recompute_heightmap 语义 = 首个非遮光格绝对 y = 遮光顶(-63) + 1。
    assert_eq!(hm[0], -62, "heightmap = 盘上值");
}

/// 回滚开关语义：`new_headless_with_io(_io_async)` 分叉同步/异步两路
/// （生产 `MCV_IO_ASYNC` 的直接解析是纯函数 `io_async_enabled_from`）。
#[test]
fn rollback_switch_semantics() {
    let pos = ChunkPos::new(6, 2);

    // 异步路：第一帧只投递（Empty 在册 + 在途记账 ≥1），不同步加载。
    let dir = presave("sw-on", pos);
    let mut rt = headless_io("sw-on", dir);
    rt.render_dist = 1;
    rt.player.pos = Vec3::new(6.0 * 16.0 + 0.5, 64.0, 2.0 * 16.0 + 0.5);
    rt.stream();
    assert_eq!(
        rt.chunks[&pos].stage(),
        Stage::Empty,
        "异步路：首帧投递后该块应仍为 Empty（在途）"
    );
    assert!(rt.io_inflight().1 >= 1, "异步路：读请求应已记账在途");

    // 同步路（回滚开关）：原 E4 前语义——首帧内同步读盘完成。
    let dir = presave("sw-off", pos);
    let mut rt = GameRuntime::new_headless_with_io(20261010, dir, GameMode::Survival, false);
    rt.render_dist = 1;
    rt.player.pos = Vec3::new(6.0 * 16.0 + 0.5, 64.0, 2.0 * 16.0 + 0.5);
    rt.stream();
    assert!(
        (rt.chunks[&pos].stage() as u8) >= (Stage::TerrainReady as u8),
        "回滚开关：同步路首帧内读盘完成（原语义）"
    );
    assert_eq!(rt.io_inflight(), (0, 0), "同步路无 IO 线程记账");
}

/// `MCV_IO_ASYNC` 纯函数核（env 不触真实环境，对齐 worker_count_from 先例）。
#[test]
fn env_switch_pure_core() {
    assert!(GameRuntime::io_async_enabled_from(None), "未设 → 异步");
    assert!(GameRuntime::io_async_enabled_from(Some("1")), "非 0 → 异步");
    assert!(
        GameRuntime::io_async_enabled_from(Some("garbage")),
        "非法值 → 异步（0 才显式回落）"
    );
    assert!(
        !GameRuntime::io_async_enabled_from(Some("0")),
        "0 → 同步回滚"
    );
}
