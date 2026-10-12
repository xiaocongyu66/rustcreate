//! E3 网格化并行锁（蓝图 docs/engine-roadmap.md §3 E3）。
//!
//! 三段式（快照-计算-回传）把 `Mesher::build`（opaque+water）移出主线程后，
//! mesher 本体零改动——本文件把「只换执行位置」钉死为测试锁：
//! 1. **等价锁**：同输入下 worker 产物 vs 主线程直呼 [`Mesher::build`] 产物
//!    逐字节相同（确定性 = 纯函数 + 无共享，红即执行位置改变了语义）；
//! 2. **顺序锁**：编辑源块 hi 通道先于排队的 lo 预算流到达（优先队列跳队）；
//! 3. **回滚开关语义**：`MCV_MESHER_WORKERS=0` → 同步路径（0 = 不建池），
//!    未设 → world 默认池策略，非法值 → 默认（对齐 `MCV_TERRAIN_BACKEND`
//!    /`MCV_POOL_WORKERS` 先例的纯函数核测法，env 不入并行测试）；
//! 4. **僵尸/陈旧锁**：已卸载区块的在途产物不得提交（防复活网格条目）、
//!    旧快照不得覆盖新快照（乱序完成序下最终提交 = 更高 seq）；
//! 5. **游戏层派发锁**：编辑当帧 hi 通道派发 + 当帧记账（stage/MESH 脏位），
//!    全栈产物与测试侧直呼 mesher 逐字节对拍。

use std::sync::Arc;
use std::time::{Duration, Instant};

use mcv_core::{CHUNK_VOL, ChunkHandle, ChunkPos, Stage, dirty, vidx};
use mcv_logic::game::{GameMode, GamePhase, GameRuntime};
use mcv_mesher::{
    MESH_OPAQUE, MESH_WATER, MeshJob, MeshResults, MeshScheduler, Mesher, neighborhood_slots,
    worker_count_from,
};

/// 唯一临时存档目录（并行安全，镜像 streaming.rs temp_dir 的做法）。
fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-meshpar-{tag}-{:x}", h.finish()))
}

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

/// 3×3 邻域场景：空气/石/水混合 + 满幅光照字节（water pass 非平凡）。
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

/// 空场景（全空气）：顺序/陈旧锁只关心调度不关心内容，空网格构建在
/// debug 构建下也快，避免多轮 × 全块构建把 CI test job 拖红。
fn empty_scene() -> (Vec<u16>, Vec<u8>) {
    (vec![0u16; 9 * CHUNK_VOL], vec![0u8; 9 * CHUNK_VOL])
}

fn job_of(pos: ChunkPos, scene: &(Vec<u16>, Vec<u8>)) -> MeshJob {
    MeshJob {
        pos,
        origin: [16.0 * pos.x as f32, 0.0, 16.0 * pos.z as f32],
        voxels: scene.0.clone(),
        lights: scene.1.clone(),
    }
}

/// 轮询取回 worker 原始产物（1ms 步进；deadline 兜底防 CI 争用挂死）。
/// 只用于无其他消费者的场景——产物会被本侧取走。
fn wait_all(results: &mut MeshResults, want: usize) -> Vec<mcv_mesher::MeshDone> {
    let t0 = Instant::now();
    let mut got = Vec::new();
    while got.len() < want {
        assert!(
            t0.elapsed() < Duration::from_secs(60),
            "worker 产物超时：{}/{}（池饿死或通道断）",
            got.len(),
            want
        );
        got.extend(results.drain_all());
        std::thread::sleep(Duration::from_millis(1));
    }
    got
}

/// 轮询等待「已完成、滞留通道」的产物数到位（非消费观测 [`MeshResults::
/// pending_done`]），不取走任何产物。
fn wait_pending(results: &MeshResults, want: usize) {
    let t0 = Instant::now();
    while results.pending_done() < want {
        assert!(
            t0.elapsed() < Duration::from_secs(60),
            "worker 产物超时：pending={}/{}（池饿死或通道断）",
            results.pending_done(),
            want
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 等价锁（蓝图红测试①）：同输入下 worker 产物 vs 主线程直呼
/// [`Mesher::build`] 产物**逐字节相同**——opaque 与 water 两 pass 都锁。
#[test]
fn worker_output_bit_identical_to_direct_build() {
    let scene = scene_bytes(0xE3_5EED);
    let (voxels, lights) = &scene;

    // 主线程参照：同步路径同款装配（neighborhood_slots）+ 直呼 build。
    let slots = neighborhood_slots(voxels, lights);
    let mesher = Mesher::new();
    let ref_opaque = mesher.build(&slots, MESH_OPAQUE).expect("校验过的 kind");
    let ref_water = mesher.build(&slots, MESH_WATER).expect("校验过的 kind");
    assert!(
        !ref_opaque.vertex_data().is_empty(),
        "场景必须产出非平凡 opaque 网格，否则等价锁空转"
    );

    // worker 路径：同一份字节经 MeshJob 投递（hi=false 预算流）。
    let (sched, mut results) = MeshScheduler::new(2);
    sched.submit(job_of(ChunkPos::new(3, -2), &scene), false);
    let done = wait_all(&mut results, 1).pop().expect("恰有一笔");
    assert_eq!(done.pos, ChunkPos::new(3, -2));
    assert!(!done.hi, "预算流派发必须走 lo 通道");
    let opaque = done.opaque.expect("非空场景必有 opaque 产物");
    assert_eq!(opaque.vertex, ref_opaque.vertex_data(), "opaque 顶点漂移");
    assert_eq!(opaque.index, ref_opaque.indices(), "opaque 索引漂移");
    let water = done.water.expect("场景含水必有 water 产物");
    assert_eq!(water.vertex, ref_water.vertex_data(), "water 顶点漂移");
    assert_eq!(water.index, ref_water.indices(), "water 索引漂移");
}

/// 顺序锁（蓝图红测试②）：编辑源块 hi 通道先于**排队中**的 lo 预算流
/// 到达。单 worker 串行化完成序 = 出队序：lo(A) 执行中、队列 [lo(B), hi(C)]
/// 时，A 完成后 worker 必须先取 hi(C)——到达序恒 [A, C, B]。
#[test]
fn hi_job_jumps_queued_lo_jobs() {
    let scene = empty_scene();
    let (sched, mut results) = MeshScheduler::new(1);
    let a = ChunkPos::new(0, 0);
    let b = ChunkPos::new(1, 0);
    let c = ChunkPos::new(2, 0);
    sched.submit(job_of(a, &scene), false);
    sched.submit(job_of(b, &scene), false);
    sched.submit(job_of(c, &scene), true);

    let got = wait_all(&mut results, 3);
    let seq: Vec<ChunkPos> = got.iter().map(|d| d.pos).collect();
    assert_eq!(
        seq,
        vec![a, c, b],
        "hi 通道必须跳过排队中的 lo（到达序 [A,hi(C),B]，得 {seq:?}）"
    );
    assert!(!got[0].hi && got[1].hi && !got[2].hi, "hi 位必须随产物回传");
}

/// 回滚开关语义锁：`MCV_MESHER_WORKERS` 的纯函数核（env 不触真实环境，
/// 对齐 TerrainBackend::from_value / worker_count_for 的测法）。
#[test]
fn worker_count_switch_semantics() {
    let default = 7;
    // 未设 → world 默认池策略（world_worker_count 同源）。
    assert_eq!(worker_count_from(None, default), default);
    // "0" = 回落同步路径（不建池）——回滚开关的锚点值。
    assert_eq!(worker_count_from(Some("0"), default), 0);
    assert_eq!(worker_count_from(Some("3"), default), 3);
    // 非法值回落默认（不 panic、不取 0——0 只能显式要求）。
    assert_eq!(worker_count_from(Some("abc"), default), default);
    assert_eq!(worker_count_from(Some(""), default), default);
    assert_eq!(worker_count_from(Some("-1"), default), default);
    assert_eq!(worker_count_from(Some(" 4"), default), default); // 不 trim，同 Integer.parseInt
}

/// 僵尸锁：区块在产物回传前被卸载（[`MeshResults::kill`] 抬提交地板）后，
/// 其在途产物**不得提交**——否则 build_chunk 会把网格条目复活到已卸载
/// 键上（内存泄漏 + 死批）。两种时序都锁：回传先于/后于卸载记账。
#[test]
fn killed_position_result_never_committed() {
    let scene = empty_scene();
    let pos = ChunkPos::new(7, 7);

    // 时序 A：产物完成入通道后再 kill（回传先于卸载记账）。
    {
        let (sched, mut results) = MeshScheduler::new(1);
        sched.submit(job_of(pos, &scene), false);
        wait_pending(&results, 1);
        results.kill(pos);
        assert!(
            results.drain_committed().is_empty(),
            "已卸载区块的在途产物被提交（僵尸网格，时序 A）"
        );
    }
    // 时序 B：kill 先于产物完成（卸载先于回传——常态卸载窗口）。
    {
        let (sched, mut results) = MeshScheduler::new(1);
        sched.submit(job_of(pos, &scene), false);
        results.kill(pos);
        wait_pending(&results, 1);
        assert!(
            results.drain_committed().is_empty(),
            "已卸载区块的在途产物被提交（僵尸网格，时序 B）"
        );
    }
    // 对照组：无 kill 时同一产物必须获准提交（证明上两段不是空转）。
    {
        let (sched, mut results) = MeshScheduler::new(1);
        sched.submit(job_of(pos, &scene), false);
        wait_pending(&results, 1);
        assert_eq!(
            results.drain_committed().len(),
            1,
            "对照组产物被误拒：僵尸过滤把正常回传也吞了"
        );
    }
}

/// 陈旧锁：同区块两次派发（编辑打断了前一张快照）乱序完成时，旧快照
/// 不得覆盖新快照——最终提交态 = 更高 seq 的产物。完成序操弄：第一笔
/// 投富场景（构建慢）占住一个 worker，第二笔投空场景（快）几乎必然先
/// 完成——到达序 [g2, g1]，g1 必须被提交地板整笔拒收。即便竞争窗口
/// 失效（g1 先到），g2 后到仍以更高 seq 收尾——终态不变量两序一致。
#[test]
fn stale_result_cannot_override_newer_commit() {
    let pos = ChunkPos::new(-4, 9);
    let rich = scene_bytes(0xE3_5EED);
    let empty = empty_scene();
    for round in 0..3u32 {
        let (sched, mut results) = MeshScheduler::new(2);
        sched.submit(job_of(pos, &rich), false); // seq 1（慢）
        sched.submit(job_of(pos, &empty), false); // seq 2（快）
        let committed = wait_quiescent(&mut results);
        assert!(
            !committed.is_empty(),
            "round {round}: 新快照必须获准提交（过滤吞掉一切）"
        );
        assert_eq!(
            committed.last().expect("非空").seq,
            2,
            "round {round}: 最终提交必须是更高 seq 的产物（旧快照覆盖了新快照）"
        );
    }
}

/// 取回**获准提交**的产物并等静默：两笔同块产物均达终态（提交或被拒）
/// 后连续 250ms 无新提交即收口。
fn wait_quiescent(results: &mut MeshResults) -> Vec<mcv_mesher::MeshDone> {
    let t0 = Instant::now();
    let mut got = Vec::new();
    loop {
        assert!(
            t0.elapsed() < Duration::from_secs(60),
            "worker 产物超时（池饿死）"
        );
        let before = got.len();
        got.extend(results.drain_committed());
        if !got.is_empty() && got.len() == before {
            std::thread::sleep(Duration::from_millis(250));
            if results.drain_committed().is_empty() {
                break;
            }
            continue;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    got
}

/// 游戏层派发锁：headless + 真实网格调度器的装配下，编辑（destroy →
/// mesh_priority）当帧派发 hi=true 且当帧记账（stage 推进 + MESH 脏位
/// 随快照清除），预算流邻块派发 hi=false；产物字节 = 测试按同款快照
/// 装配直呼 mesher，逐字节对拍（全栈等价）。
#[test]
fn game_dispatch_records_and_routes_hi_priority() {
    let (mut rt, mut results) =
        GameRuntime::new_headless_with_mesh(20261012, temp_dir("game-hi"), GameMode::Creative, 1);
    rt.phase = GamePhase::Playing;
    rt.render_dist = 2;

    // 3×3 灯就绪夹具：石板地面顶 y=40（streaming.rs lit_chunk 同款手工推进）。
    let mut handles: Vec<(ChunkPos, Arc<ChunkHandle>)> = Vec::new();
    for dz in -1i32..=1 {
        for dx in -1i32..=1 {
            let p = ChunkPos::new(dx, dz);
            handles.push((p, lit_floor_chunk(p, 40)));
        }
    }
    for (p, h) in &handles {
        rt.chunks.insert(*p, h.clone());
    }
    // 玩家立于中心块地面上、准星垂直向下 → interact(false) 命中脚下石块。
    rt.player.pos = glam::Vec3::new(8.5, 41.5, 8.5);
    rt.player.yaw = 0.0;
    rt.player.pitch = -std::f32::consts::FRAC_PI_2;

    rt.interact(false); // 创造秒破 → destroy_block → mesh_priority = (0,0)
    rt.stream(); // remesh：hi 通道（编辑源块）+ 1 格预算余量（邻块首建）

    // 当帧记账：中心块已派发（stage 推进 Uploaded + MESH 脏位随快照清除）。
    let center = rt.chunks[&ChunkPos::new(0, 0)].clone();
    assert_eq!(center.stage(), Stage::Uploaded, "派发即记账：stage 推进");
    assert_eq!(
        center.dirty() & dirty::MESH,
        0,
        "派发即记账：快照与脏位清除同帧同点"
    );

    // 回传产物：中心块 hi=true；另有一笔邻块首建走预算流 hi=false。
    // 注意单 worker 串行：hi 跑完才轮到 lo——必须收齐 hi+lo 两笔再收口。
    let mut got: Vec<mcv_mesher::MeshDone> = Vec::new();
    let t0 = Instant::now();
    while !(got.iter().any(|d| d.hi) && got.iter().any(|d| !d.hi)) {
        assert!(t0.elapsed() < Duration::from_secs(60), "worker 产物超时");
        got.extend(results.drain_all());
        std::thread::sleep(Duration::from_millis(1));
    }
    let center_done = got
        .iter()
        .find(|d| d.pos == ChunkPos::new(0, 0) && d.hi)
        .expect("编辑源块必须走 hi 通道派发");
    let lo_done = got
        .iter()
        .find(|d| !d.hi)
        .expect("预算流邻块必有 hi=false 派发");
    assert_ne!(lo_done.pos, ChunkPos::new(0, 0), "编辑源块不得再走 lo");

    // 全栈等价：测试按同款快照装配（3×3 夹具字节 → neighborhood_slots）
    // 直呼 mesher，与「游戏快照 → worker」的产物逐字节对拍。
    let vol = mcv_core::CHUNK_VOL;
    let mut voxels = vec![0u16; 9 * vol];
    let mut lights = vec![0xF0u8; 9 * vol]; // 与 snapshot_neighborhood 同款初值
    for (i, (_, h)) in handles.iter().enumerate() {
        voxels[i * vol..(i + 1) * vol]
            .copy_from_slice(bytemuck::cast_slice(h.voxels.read().unwrap().as_slice()));
        lights[i * vol..(i + 1) * vol].copy_from_slice(&h.light.read().unwrap()[..]);
    }
    let slots = neighborhood_slots(&voxels, &lights);
    let mesher = Mesher::new();
    let ref_opaque = mesher.build(&slots, MESH_OPAQUE).expect("kind 校验通过");
    let ref_water = mesher.build(&slots, MESH_WATER).expect("kind 校验通过");
    let opaque = center_done.opaque.as_ref().expect("石板场景必有 opaque");
    assert_eq!(opaque.vertex, ref_opaque.vertex_data(), "游戏派发顶点漂移");
    assert_eq!(opaque.index, ref_opaque.indices(), "游戏派发索引漂移");
    let water = center_done.water.as_ref().expect("水 pass 产物随派发回传");
    assert_eq!(water.vertex, ref_water.vertex_data(), "游戏派发水顶点漂移");
    assert_eq!(water.index, ref_water.indices(), "游戏派发水索引漂移");
}

/// 灯就绪夹具块：石板地面 + 手工推进 LightLocalReady（streaming.rs 的
/// lit_chunk 等价物；光照数组保持零值——快照所见即所得）。
fn lit_floor_chunk(pos: ChunkPos, floor_top: i32) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(pos));
    {
        let mut vg = h.voxels.write().unwrap();
        for y in mcv_core::WORLD_MIN_Y..=floor_top {
            for z in 0..16usize {
                for x in 0..16usize {
                    vg[vidx(x, y, z)] = mcv_core::BlockId(1);
                }
            }
        }
    }
    h.advance_to(Stage::LightLocalReady);
    h
}
