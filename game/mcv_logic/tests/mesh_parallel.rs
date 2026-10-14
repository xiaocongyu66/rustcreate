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
//!    全栈产物与测试侧直呼 mesher 逐字节对拍；
//! 6. **死账看门狗锁**（worker-accounting DC-1/DC-4）：worker 静默吞产物 →
//!    ≤K+2 帧看门狗回滚三笔账并重派（终态记账一致）；panic 丢失标记 →
//!    即时回滚；连续丢失 M 次 → 断路器降级同步路（防重派风暴）；正常产物
//!    路径零回滚（对照组排除看门狗误伤循环）。注入机制 =
//!    [`MeshResults::install_loss_hook`]（Deliver/Swallow/Panic，语义见
//!    [`mcv_core::pool::LossMode`]）；测试侧按 flush 同款对 drained 产物
//!    调 `mesh_worker_commit`/`mesh_worker_lost` 对账。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use mcv_core::pool::LossMode;
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
        // 并行等价锁测试跑无 tint 基线（顶点色白 = 乘算单位元，两路
        // 产物逐字节同；带色路径一致性由 mcv_core tint 测试锁定）。
        tint: None,
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
/// 到达。确定性构造：先投一笔**重活**钉死单 worker（随机噪声场整块
/// 构建毫秒级 ≫ 三次 submit 的微秒级窗口），A/B/C 必在 worker 忙碌期
/// 入队——出队序 = 优先级序。旧写法用空场景（µs 级构建）与 worker
/// 线程启动/轮询赛跑：runner 空闲时 worker 抢走 A 得 [A,C,B]，runner
/// 满载时三笔全在队里 worker 首取 hi(C) 得 [C,A,B]——后者恰是优先级
/// 正确生效，断言却红（CI run 38303726392 实锤）。跨竞态不变式：
/// hi(C) 先于 A、B（两者在 C 入队时要么在队要么在跑，单 worker 下
/// 完成必晚于 C）；lo 同档 A 先于 B（FIFO）。
#[test]
fn hi_job_jumps_queued_lo_jobs() {
    let scene = empty_scene();
    let heavy = scene_bytes(0xC0_FFEE);
    let (sched, mut results) = MeshScheduler::new(1);
    let blocker = ChunkPos::new(-7, 0);
    let a = ChunkPos::new(0, 0);
    let b = ChunkPos::new(1, 0);
    let c = ChunkPos::new(2, 0);
    sched.submit(job_of(blocker, &heavy), false);
    sched.submit(job_of(a, &scene), false);
    sched.submit(job_of(b, &scene), false);
    sched.submit(job_of(c, &scene), true);

    let got = wait_all(&mut results, 4);
    let seq: Vec<ChunkPos> = got.iter().map(|d| d.pos).collect();
    let idx = |p: ChunkPos| seq.iter().position(|&q| q == p).expect("四笔必有产物");
    assert!(
        idx(c) < idx(a) && idx(a) < idx(b),
        "hi(C) 必须跳过排队中的 lo、lo 同档 FIFO（到达序应 C…A…B，得 {seq:?}）"
    );
    for d in &got {
        assert_eq!(
            d.hi,
            d.pos == c,
            "hi 位必须随产物回传（只有 C 是 hi 通道），得 {seq:?}"
        );
    }
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

    // 5×5 灯就绪夹具：石板地面顶 y=40（streaming.rs lit_chunk 同款手工
    // 推进）。外圈两环必须补齐——预算流邻块（如 (-1,-1)）的 3×3 邻域伸
    // 出 3×3 夹具，缺块即门槛全拒、零 lo 派发。request 环半径 = 2 恰被
    // 覆盖，无世界生成插块干扰。
    for dz in -2i32..=2 {
        for dx in -2i32..=2 {
            let p = ChunkPos::new(dx, dz);
            rt.chunks_mut().insert(p, lit_floor_chunk(p, 40));
        }
    }
    // 参照快照源：中心块 (0,0) 的 3×3 邻域（dz 外层行主序，与
    // snapshot_neighborhood 同布局）。
    let mut handles: Vec<(ChunkPos, Arc<ChunkHandle>)> = Vec::new();
    for dz in -1i32..=1 {
        for dx in -1i32..=1 {
            let p = ChunkPos::new(dx, dz);
            handles.push((p, rt.chunks[&p].clone()));
        }
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
        voxels[i * vol..(i + 1) * vol].copy_from_slice(&h.voxels.read().unwrap().decode_column());
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
                    vg.set_flat(vidx(x, y, z), mcv_core::BlockId(1));
                }
            }
        }
    }
    h.advance_to(Stage::LightLocalReady);
    h
}

// -----------------------------------------------------------------------
// 死账看门狗锁（DC-1/DC-4：派发即记账 × 产物确认配对）
// -----------------------------------------------------------------------

/// 看门狗夹具：rd=1（请求环 Chebyshev 2 恰被 5×5 夹具覆盖，零世界生成
/// 插块）+ 5×5 灯就绪石板 + 玩家立中心、准星向下（interact(false) 秒破
/// → mesh_priority = (0,0)）。返回 (rt, results, 中心块)。
fn watchdog_fixture(tag: &str) -> (GameRuntime, MeshResults, ChunkPos) {
    let (mut rt, results) =
        GameRuntime::new_headless_with_mesh(20261014, temp_dir(tag), GameMode::Creative, 1);
    rt.phase = GamePhase::Playing;
    rt.render_dist = 1;
    for dz in -2i32..=2 {
        for dx in -2i32..=2 {
            let p = ChunkPos::new(dx, dz);
            rt.chunks_mut().insert(p, lit_floor_chunk(p, 40));
        }
    }
    rt.player.pos = glam::Vec3::new(8.5, 41.5, 8.5);
    rt.player.yaw = 0.0;
    rt.player.pitch = -std::f32::consts::FRAC_PI_2;
    (rt, results, ChunkPos::new(0, 0))
}

/// 对账镜像（生产 = RustMesher::flush 的事件流）：取走全部原始产物，
/// 逐笔按「获准提交/丢失标记」分账回灌游戏层。
fn mirror_flush(rt: &mut GameRuntime, results: &mut MeshResults) {
    for done in results.drain_all() {
        if done.opaque.is_some() {
            rt.mesh_worker_commit(done.pos, done.seq);
        } else {
            rt.mesh_worker_lost(done.pos, done.seq);
        }
    }
}

/// 看门狗锁（DC-4 主锁）：worker 静默吞掉首笔产物（无任何回执——回执
/// 通道丢失的极端面）后，≤K+2 帧内看门狗必须回滚三笔账（meshed 删账 +
/// MESH 置脏 + stage 回退）并重派，重派产物照常提交，终态记账与正常
/// 路径一致（meshed 在账 / stage Uploaded / 脏位清除 / 在途账销账），
/// 且不再有第三次派发（无回滚循环）。判定面全部取**主线程同步态**
/// （在途账 seq / meshed / stage / 脏位）——丢失钩子计数在 worker 线程
/// 递增，只作静默后的总量复核，不作时序判定（防竞态假红）。
#[test]
fn watchdog_redispatches_after_silent_worker_loss() {
    const K: u64 = 10; // game.rs MESH_WATCHDOG_FRAMES 镜像
    let (mut rt, mut results, center) = watchdog_fixture("wd-recover");

    let dispatches = Arc::new(AtomicU32::new(0));
    {
        let dispatches = Arc::clone(&dispatches);
        let swallowed = Arc::new(AtomicU32::new(0));
        results.install_loss_hook(Box::new(move |p| {
            if p != center {
                return LossMode::Deliver;
            }
            dispatches.fetch_add(1, Ordering::Relaxed);
            // 首笔静默蒸发，之后一切正常（看门狗单轮兜底即可自愈）。
            if swallowed.fetch_add(1, Ordering::Relaxed) == 0 {
                LossMode::Swallow
            } else {
                LossMode::Deliver
            }
        }));
    }

    rt.interact(false);
    rt.stream(); // 派发 #1（被吞）+ 邻块 lo 预算
    // 派发即记账 = 主线程同步事实（钩子计数是 worker 侧异步量，不在此断）。
    let (frame0, seq0) = rt
        .mesh_watchdog_pending(center)
        .expect("派发即登记看门狗在途账");
    assert!(rt.meshed_contains(center), "派发即记 meshed 账");
    mirror_flush(&mut rt, &mut results);

    // 泵至重派（窗口硬顶 = K+2 帧迭代，每迭代一帧——机制给出的恢复期
    // 上界即 K+1 派发帧 + 1 提交帧；跑满未 break 即死账未自愈）。重派的
    // 同步判定 = 在途账被看门狗回滚后由 worker 路重登新 seq（> seq0；
    // 同步路不登账，故这同时证明重派走了 worker 池）。
    let mut redispatched = false;
    for _ in 0..=(K as usize + 2) {
        mirror_flush(&mut rt, &mut results);
        rt.stream();
        mirror_flush(&mut rt, &mut results);
        if rt
            .mesh_watchdog_pending(center)
            .is_some_and(|(f, s)| s > seq0 && f > frame0)
        {
            redispatched = true;
            break;
        }
    }
    assert!(redispatched, "≤K+2 帧内必须重派（看门狗死账兜底失效）");

    // 终态：重派产物获准提交 → 记账回到正常路径形态（liveness 泵，deadline
    // 兜底防 CI 争用挂死）。
    let mut settled = false;
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(30) {
        mirror_flush(&mut rt, &mut results);
        rt.stream();
        mirror_flush(&mut rt, &mut results);
        if rt.mesh_watchdog_pending(center).is_none()
            && rt.meshed_contains(center)
            && rt.chunks[&center].stage() == Stage::Uploaded
            && rt.chunks[&center].dirty() & dirty::MESH == 0
        {
            settled = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        settled,
        "重派提交后终态记账必须一致（meshed 在账/Uploaded/脏位清除/在途销账）"
    );
    assert_eq!(
        dispatches.load(Ordering::Relaxed),
        2,
        "恰好两次派发：被吞 + 重派（第三次 = 看门狗误伤循环）"
    );
    assert_eq!(
        rt.mesh_watchdog_streak(center),
        0,
        "提交确认清零连续丢失计数"
    );
}

/// 丢失标记锁（DC-3 即时回滚路）：worker panic → opaque=None 丢失标记 →
/// **不等看门狗窗**即时回滚三笔账（meshed 删账/MESH 置脏/stage 回退），
/// worker_lost 计数响亮累计；连续丢失 3 次 → 断路器降级同步路（派发次数
/// 硬顶 = M，不许无限重派）。回滚发生在 mirror_flush 的 `mesh_worker_lost`
/// 回灌点（主线程同步），判定面 = meshed/stage/脏位同步态。
#[test]
fn loss_marker_rolls_back_immediately_and_degrades_after_repeat() {
    let (mut rt, mut results, center) = watchdog_fixture("wd-loss-marker");

    let dispatches = Arc::new(AtomicU32::new(0));
    {
        let dispatches = Arc::clone(&dispatches);
        results.install_loss_hook(Box::new(move |p| {
            if p != center {
                return LossMode::Deliver;
            }
            dispatches.fetch_add(1, Ordering::Relaxed);
            LossMode::Panic // 每笔都 panic（确定性丢失源）
        }));
    }

    rt.interact(false);
    rt.stream(); // 派发 #1

    // 第一轮：丢失标记回流 → 即时回滚三笔账（不等看门狗窗）。
    let t0 = Instant::now();
    loop {
        mirror_flush(&mut rt, &mut results);
        if !rt.meshed_contains(center) {
            break; // 回滚已发生（主线程同步态）
        }
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "丢失标记必须即时回滚（meshed 删账）"
        );
        rt.stream();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        rt.mesh_stats().worker_lost >= 1,
        "丢失事件必须响亮记账（render-stats worker_lost）"
    );
    assert_eq!(
        rt.chunks[&center].stage(),
        Stage::LightLocalReady,
        "回滚必须回退 stage（撤回「已上传」记账）"
    );
    assert_ne!(
        rt.chunks[&center].dirty() & dirty::MESH,
        0,
        "回滚必须置 MESH 脏（预算流重派资格）"
    );

    // 再丢两轮 → 断路器：降级名单在册 + 派发硬顶 3 次 + 不再有 worker 派发。
    let t0 = Instant::now();
    loop {
        mirror_flush(&mut rt, &mut results);
        rt.stream();
        mirror_flush(&mut rt, &mut results);
        if rt.mesh_watchdog_fallback(center) {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "连续丢失 3 次必须降级同步路（防重派风暴）"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        dispatches.load(Ordering::Relaxed),
        3,
        "worker 派发硬顶 M=3（断路器后不再进池）"
    );
    assert_eq!(rt.mesh_watchdog_streak(center), 0, "降级时清零丢失计数");
    assert!(
        !rt.mesh_watchdog_fallback(ChunkPos::new(1, 0)),
        "断路器必须按块隔离（邻块不连坐）"
    );
}

/// 对照组锁：正常产物路径零回滚——派发一次、提交确认一次、终态记账一致
/// （排除看门狗误伤循环；K 帧静置不触发任何回滚/重派）。
#[test]
fn normal_products_never_trigger_watchdog() {
    let (mut rt, mut results, center) = watchdog_fixture("wd-control");
    results.install_loss_hook(Box::new(|_| LossMode::Deliver));

    rt.interact(false);
    rt.stream(); // 派发 #1（hi）+ 邻块 lo 预算
    assert!(
        rt.mesh_watchdog_pending(center).is_some(),
        "派发即登记看门狗在途账"
    );

    // 泵至提交确认（在途账销账 = 主线程同步态；deadline 兜底）。
    let t0 = Instant::now();
    loop {
        mirror_flush(&mut rt, &mut results);
        if rt.mesh_watchdog_pending(center).is_none() {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "正常产物必须提交确认（在途账销账）"
        );
        rt.stream();
        mirror_flush(&mut rt, &mut results);
        std::thread::sleep(Duration::from_millis(1));
    }

    // 静置 K+3 帧：无编辑 → 无重派发、无回滚、无丢失计数。
    for _ in 0..(K_WATCHDOG as usize + 3) {
        rt.stream();
        mirror_flush(&mut rt, &mut results);
    }
    assert!(rt.meshed_contains(center));
    assert_eq!(rt.chunks[&center].stage(), Stage::Uploaded);
    assert_eq!(rt.chunks[&center].dirty() & dirty::MESH, 0);
    assert_eq!(rt.mesh_watchdog_streak(center), 0);
    assert_eq!(rt.mesh_stats().worker_lost, 0, "正常路径零丢失事件");
    assert_eq!(rt.mesh_stats().dispatch_gap, 0, "正常路径零超窗在途");
}

/// game.rs MESH_WATCHDOG_FRAMES 镜像（对照锁的静置窗长度）。
const K_WATCHDOG: u64 = 10;
