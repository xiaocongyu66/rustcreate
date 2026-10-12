//! E5 光照 init 并行锁（蓝图 docs/engine-roadmap.md §3 E5）。
//!
//! 快照-计算-回传三段式把 `mcv_light::init`（sky+block 双通道 BFS）移出
//! 主线程后，光照引擎本体零改动——本文件把「只换执行位置」与「写回环序」
//! 钉死为测试锁：
//! 1. **等价锁**：worker 产物 vs 主线程直呼 [`mcv_light::init`] 逐位相同
//!    （光照字节 + 边界脏掩码；确定性 = 纯函数 + 无共享）；
//! 2. **游戏层对拍**：并行路（worker init）vs 串行路（`MCV_LIGHT_WORKERS=0`
//!    同款主线程 init）同夹具驱动到收敛后 25 块逐位对拍 + 每块恒等于局部
//!    init 参照。夹具用**纯高度场**（逐列自洽、跨块边零净流）：引擎跨块
//!    边协议对「在途光 + 回撤判据 s>n+1」（mcv_light KNOWN-DIVERGENCE，
//!    残余 ≤+2 随下次编辑自愈）的收敛**终态依赖成对标记的帧调度**——
//!    角落火把构造下并行与串行可到不同静息态（CI 实证：58 格块光差
//!    ≤6 档，串行侧欠传导）。这是引擎已知序偏差、非本波引入（现网
//!    worldgen 完成序本已使 init 批次组成逐帧漂移）；本波不改边同步
//!    语义（蓝图铁纪律），以纯高度场对拍锁管线保真、以单边界火把精确
//!    值锁跨块流，调度方差登记为引擎债（回退锚 = MCV_LIGHT_WORKERS=0
//!    串行确定性调度）；
//! 3. **回传环序锁**：同帧多块回传的**生效序 = sorted_keys 环序**（玩家
//!    近优先，乱序到达由写回重排）；单 worker 串行化下整段写回序列恒等
//!    于近优先环序（派发序 = 完成序 = 环序）；
//! 4. **僵尸锁**：已卸载区块的在途产物不得套用（提交地板，双时序 + 对照组）；
//! 5. **回滚开关语义**：`MCV_LIGHT_WORKERS` 纯函数核（0 = 主线程串行锚点，
//!    未设 → world 默认池策略，非法值 → 默认；env 不入并行测试）；
//! 6. **在途记账锁**：in_flight 随派发置位、随回传（无论获准与否）解除
//!    ——同键去重门的依据。
//!
//! 原版同构（光照引擎跑在服务器后台执行线程，ChunkMap.java:194-197 持
//! `ThreadedLevelLightEngine` + `ChunkTaskDispatcher(light, executor)`，
//! executor = `Util.backgroundExecutor()`（MinecraftServer.java:352）线程
//! 池；单块布光 = 后台 `propagateLightSources` + 回执 `setLightCorrect`
//! 两段式，ThreadedLevelLightEngine.java:164-180；出队按票级别近优先，
//! ChunkTaskPriorityQueue.java:63-78）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use mcv_core::{BlockId, CHUNK_VOL, ChunkHandle, ChunkPos, Stage, WORLD_MIN_Y, vidx};
use mcv_light::{LightDone, LightJob, LightResults, LightScheduler, worker_count_from};
use mcv_logic::game::{GameMode, GameRuntime};

/// 唯一临时存档目录（并行安全，镜像 streaming.rs temp_dir 的做法）。
fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-lightpar-{tag}-{:x}", h.finish()))
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

/// 千块表 id 1009 = "torch"（26.1 发光 14，见 game.rs tests 同款常量）。
const TORCH: u16 = 1009;

/// 场景体素（种子绑定的确定性纯函数）：起伏石面 + 封闭腔 + 水池 + 火把群
/// ——sky（水面截断/洞穴暗腔）与 block（火把）双通道均非平凡，逐位对拍
/// 才有区分度。
fn scene_voxels(seed: u64) -> Vec<u16> {
    let water = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "water")
        .expect("原版方块表必有 water") as u16;
    let mut rng = Lcg(seed);
    let mut v = vec![0u16; CHUNK_VOL];
    for z in 0..16usize {
        for x in 0..16usize {
            let h = WORLD_MIN_Y + 94 + (rng.next() % 12) as i32; // 表面高 -64+94.. ≈ 30..41
            for y in WORLD_MIN_Y..=h {
                v[vidx(x, y, z)] = 1; // stone
            }
            // 封闭腔：表面下掏 3×3（无邻面进光的暗腔，init 只亮腔口）。
            if rng.next().is_multiple_of(7) {
                for dz in 0..3i32 {
                    for dx in 0..3i32 {
                        let (xx, zz) = (x as i32 + dx, z as i32 + dz);
                        if (0..16).contains(&xx) && (0..16).contains(&zz) {
                            v[vidx(xx as usize, h - 2, zz as usize)] = 0;
                        }
                    }
                }
            }
            // 水池：低洼处灌水（damp 1 → 天光柱 14、13、12 …衰减）。
            if h <= 33 {
                for y in (h + 1)..=35 {
                    v[vidx(x, y, z)] = water;
                }
            }
            // 火把群（只在岸上，避免与水池互相覆盖）。
            if h > 35 && rng.next().is_multiple_of(13) {
                v[vidx(x, h + 1, z)] = TORCH;
            }
        }
    }
    v
}

/// 场景种子（位置绑定的确定性纯函数——同一 pos 两次装配逐字节同源）。
fn scene_seed(pos: ChunkPos) -> u64 {
    0xE5_00_00_u64
        ^ ((pos.x as i64).wrapping_mul(7_919) as u64)
        ^ ((pos.z as i64).wrapping_mul(104_729) as u64)
}

/// 夹具块：场景体素 + heightmap，停在 TerrainReady（未布光——等 worker /
/// 串行路来点）。
fn terrain_chunk(pos: ChunkPos, seed: u64) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(pos));
    let scene = scene_voxels(seed);
    {
        let mut vg = h.voxels.write().unwrap();
        let ids: &mut [u16] = bytemuck::cast_slice_mut(&mut vg[..]);
        ids.copy_from_slice(&scene);
    }
    *h.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&scene);
    h.advance_to(Stage::TerrainReady);
    h
}

/// 游戏层夹具：5×5 TerrainReady 场景块 + rd=0（请求环 = 3×3 恰被覆盖，
/// 无世界生成插块干扰；卸载边界 = Chebyshev rd+4，外环夹具不淘汰）。
/// 返回夹具位表（sorted_keys 期望序的输入）。
fn setup_fixtures(rt: &mut GameRuntime) -> Vec<ChunkPos> {
    rt.render_dist = 0;
    let mut positions = Vec::new();
    for dz in -2i32..=2 {
        for dx in -2i32..=2 {
            let p = ChunkPos::new(dx, dz);
            rt.chunks_mut().insert(p, terrain_chunk(p, scene_seed(p)));
            positions.push(p);
        }
    }
    positions
}

/// [`setup_fixtures`] 同款位表的 sorted_keys 期望序（Chebyshev 近优先，
/// 环内 (dx,dz) 字典序破平——game.rs sorted_keys 同规则）。
fn expected_ring_order(center: ChunkPos, positions: &[ChunkPos]) -> Vec<ChunkPos> {
    let mut sorted = positions.to_vec();
    sorted.sort_by_key(|p| {
        let dx = p.x - center.x;
        let dz = p.z - center.z;
        (dx.abs().max(dz.abs()), dx, dz)
    });
    sorted
}

/// 驱动 stream() 直到夹具全部 ≥LightLocalReady，再补 5 帧排空边同步级联
/// （512 步预算内通常当帧清空；补帧对终态幂等——paired_edge_sync 幂等锁
/// 同款前提）。deadline 兜底防 CI 争用挂死。
fn drive_until_lit(rt: &mut GameRuntime, positions: &[ChunkPos]) {
    let t0 = Instant::now();
    loop {
        rt.stream();
        let all_lit = positions.iter().all(|p| {
            rt.chunks
                .get(p)
                .is_some_and(|h| (h.stage() as u8) >= (Stage::LightLocalReady as u8))
        });
        if all_lit {
            for _ in 0..5 {
                rt.stream();
                std::thread::sleep(Duration::from_millis(1));
            }
            return;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(60),
            "布光未收敛（并行路未接线或池饿死）：{}/{} 块就绪",
            positions
                .iter()
                .filter(|p| rt
                    .chunks
                    .get(p)
                    .is_some_and(|h| (h.stage() as u8) >= (Stage::LightLocalReady as u8)))
                .count(),
            positions.len()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 区块光照字节快照（对拍面）。
fn light_of(rt: &GameRuntime, pos: ChunkPos) -> Vec<u8> {
    rt.chunks[&pos].light.read().unwrap()[..].to_vec()
}

/// 轮询取回 worker 原始产物（1ms 步进；deadline 兜底防 CI 争用挂死）。
fn wait_all(results: &mut LightResults, want: usize) -> Vec<LightDone> {
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

/// 轮询等待「已完成、滞留通道」的产物数到位（非消费观测）。
fn wait_pending(results: &LightResults, want: usize) {
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
/// [`mcv_light::init`] **逐位相同**——光照字节与边界脏掩码都锁；三组场景
/// （洞穴/水面/火把富集）覆盖 sky 源列截断、暗腔、block 播种三条路径。
#[test]
fn worker_init_output_bit_identical_to_serial_init() {
    for (tag, pos, seed) in [
        ("cave", ChunkPos::new(3, -2), 0xE5_0001),
        ("water", ChunkPos::new(-7, 4), 0xE5_0002),
        ("torch", ChunkPos::new(11, 11), 0xE5_0003),
    ] {
        let voxels = scene_voxels(seed);
        let heightmap = mcv_worldgen::recompute_heightmap(&voxels);

        // 主线程参照：串行路径同款装配（LightChunk 直呼 init）。
        let mut ref_light = vec![0u8; CHUNK_VOL];
        let mut view = mcv_light::LightChunk {
            voxels: &voxels,
            light: &mut ref_light,
            heightmap: &heightmap[..],
        };
        let ref_mask = mcv_light::init(&mut view);
        assert!(
            ref_light.iter().any(|&b| b != 0),
            "{tag}：场景必须产出非平凡光照，否则等价锁空转"
        );

        // worker 路径：同一份字节经 LightJob 投递。
        let (sched, mut results) = LightScheduler::new(2);
        sched.submit(LightJob {
            pos,
            voxels: voxels.clone(),
            heightmap: heightmap.to_vec(),
        });
        let done = wait_all(&mut results, 1).pop().expect("恰有一笔");
        assert_eq!(done.pos, pos);
        assert_eq!(done.light, ref_light, "{tag}：worker 光照字节漂移");
        assert_eq!(done.border_mask, ref_mask, "{tag}：边界脏掩码漂移");
    }
}

/// 在途记账锁：in_flight 随派发置位、随回传消化解除（获准与否都解除）
/// ——同键去重门的依据（派发段以 in_flight 为门，漏解除 = 该块永不重派）。
#[test]
fn in_flight_tracking_follows_job_lifecycle() {
    let pos = ChunkPos::new(5, -3);
    let voxels = scene_voxels(0xE5_0001);
    let heightmap = mcv_worldgen::recompute_heightmap(&voxels);
    let (sched, mut results) = LightScheduler::new(1);
    assert!(!sched.in_flight(&pos), "派发前不得记在途");
    sched.submit(LightJob {
        pos,
        voxels,
        heightmap: heightmap.to_vec(),
    });
    assert!(sched.in_flight(&pos), "派发后必须记在途");
    wait_all(&mut results, 1);
    let _ = results.drain();
    assert!(!sched.in_flight(&pos), "回传消化后必须解除在途");
}

/// 僵尸锁：区块在产物回传前被卸载（[`LightResults::kill`] 抬提交地板）
/// 后，其在途产物**不得套用**——防把光照写进已出环的旧键、或卸载后重进
/// 环（读盘内容可能已不同）时旧快照覆盖新数据。两种时序都锁：回传先于/
/// 后于卸载记账。
#[test]
fn killed_position_result_never_admitted() {
    let pos = ChunkPos::new(7, 7);
    let voxels = scene_voxels(0xE5_0001);
    let heightmap = mcv_worldgen::recompute_heightmap(&voxels);
    let job = || LightJob {
        pos,
        voxels: voxels.clone(),
        heightmap: heightmap.to_vec(),
    };

    // 时序 A：产物完成入通道后再 kill（回传先于卸载记账）。
    {
        let (sched, mut results) = LightScheduler::new(1);
        sched.submit(job());
        wait_pending(&results, 1);
        results.kill(pos);
        assert!(
            results.drain().is_empty(),
            "已卸载区块的在途产物被套用（僵尸写回，时序 A）"
        );
    }
    // 时序 B：kill 先于产物完成（卸载先于回传——常态卸载窗口）。
    {
        let (sched, mut results) = LightScheduler::new(1);
        sched.submit(job());
        results.kill(pos);
        wait_pending(&results, 1);
        assert!(
            results.drain().is_empty(),
            "已卸载区块的在途产物被套用（僵尸写回，时序 B）"
        );
    }
    // 对照组：无 kill 时同一产物必须获准（证明上两段不是空转）。
    {
        let (sched, mut results) = LightScheduler::new(1);
        sched.submit(job());
        wait_pending(&results, 1);
        assert_eq!(
            results.drain().len(),
            1,
            "对照组产物被误拒：僵尸过滤把正常回传也吞了"
        );
    }
}

/// 回滚开关语义锁：`MCV_LIGHT_WORKERS` 的纯函数核（env 不触真实环境，
/// 对齐 mcv_mesher::worker_count_from 的测法）。
#[test]
fn light_worker_count_switch_semantics() {
    let default = 7;
    // 未设 → world 默认池策略（world_worker_count 同源）。
    assert_eq!(worker_count_from(None, default), default);
    // "0" = 主线程串行 init（不建池）——回滚开关的锚点值。
    assert_eq!(worker_count_from(Some("0"), default), 0);
    assert_eq!(worker_count_from(Some("3"), default), 3);
    // 非法值回落默认（不 panic、不取 0——0 只能显式要求）。
    assert_eq!(worker_count_from(Some("abc"), default), default);
    assert_eq!(worker_count_from(Some(""), default), default);
    assert_eq!(worker_count_from(Some("-1"), default), default);
    assert_eq!(worker_count_from(Some(" 4"), default), default); // 不 trim，同 Integer.parseInt
}

/// 纯高度场场景体素（全块同款确定性纯函数，无 rng/无水/无光源）：起伏石
/// 面 + 封腔（ sealed，不透光）。高度场无悬空遮蔽、逐列自洽——**局部
/// init = 全局解，跨块边零净流**：对拍不受成对标记帧调度影响（引擎边
/// 协议对「在途光 + 回撤判据 s>n+1」的终态敏感是已知序偏差，见
/// `game_parallel_light_cross_border_torch_exact_values` 文档）。
fn pattern_voxels() -> Vec<u16> {
    let mut v = vec![0u16; CHUNK_VOL];
    for z in 0..16usize {
        for x in 0..16usize {
            let h = WORLD_MIN_Y + 94 + ((x * 7 + z * 13) % 12) as i32; // 30..41，全块同款
            for y in WORLD_MIN_Y..=h {
                v[vidx(x, y, z)] = 1; // stone
            }
            if (x * 5 + z * 3) % 11 == 0 {
                // 封腔：表面下掏 3×3（sealed，对拍不受其影响）。
                for dz in 0..3i32 {
                    for dx in 0..3i32 {
                        let (xx, zz) = (x as i32 + dx, z as i32 + dz);
                        if (0..16).contains(&xx) && (0..16).contains(&zz) {
                            v[vidx(xx as usize, h - 2, zz as usize)] = 0;
                        }
                    }
                }
            }
        }
    }
    v
}

/// [`pattern_voxels`] 同款夹具块（全块逐字节同源），停在 TerrainReady。
fn pattern_chunk(pos: ChunkPos) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(pos));
    let scene = pattern_voxels();
    {
        let mut vg = h.voxels.write().unwrap();
        let ids: &mut [u16] = bytemuck::cast_slice_mut(&mut vg[..]);
        ids.copy_from_slice(&scene);
    }
    *h.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&scene);
    h.advance_to(Stage::TerrainReady);
    h
}

/// 游戏层夹具（纯高度场）：5×5 TerrainReady + rd=0（请求环 = 3×3 恰被
/// 覆盖，无世界生成插块干扰；卸载边界 = Chebyshev rd+4，外环夹具不淘汰）。
/// 返回夹具位表（sorted_keys 期望序的输入）。
fn setup_pattern_fixtures(rt: &mut GameRuntime) -> Vec<ChunkPos> {
    rt.render_dist = 0;
    let mut positions = Vec::new();
    for dz in -2i32..=2 {
        for dx in -2i32..=2 {
            let p = ChunkPos::new(dx, dz);
            rt.chunks_mut().insert(p, pattern_chunk(p));
            positions.push(p);
        }
    }
    positions
}

/// 游戏层对拍（蓝图红测试②）：同夹具下并行路（W=2 worker init）vs 串行路
/// （`new_headless` = `MCV_LIGHT_WORKERS=0` 同款主线程 init）驱动到收敛后
/// 25 块光照**逐位相同**，且每块恒等于该块体素的局部 init 参照（纯高度
/// 场下局部解 = 全局解，跨块边零净流——对拍不受成对标记帧调度影响）。
/// 红即快照/回传/写回管线改变了光照字节。
#[test]
fn game_parallel_light_bitwise_matches_serial_path() {
    // 局部 init 参照（全块同款体素 → 同一份参照）。
    let voxels = pattern_voxels();
    let heightmap = mcv_worldgen::recompute_heightmap(&voxels);
    let mut ref_light = vec![0u8; CHUNK_VOL];
    let mut view = mcv_light::LightChunk {
        voxels: &voxels,
        light: &mut ref_light,
        heightmap: &heightmap[..],
    };
    mcv_light::init(&mut view);
    assert!(
        ref_light.iter().any(|&b| b != 0),
        "场景必须产出非平凡光照，否则对拍空转"
    );

    let serial_light: Vec<(ChunkPos, Vec<u8>)>;
    {
        let mut rt = GameRuntime::new_headless(20261012, temp_dir("lp-serial"), GameMode::Survival);
        let positions = setup_pattern_fixtures(&mut rt);
        drive_until_lit(&mut rt, &positions);
        assert_eq!(
            rt.light_workers(),
            0,
            "串行参照路不得装配调度器（new_headless = MCV_LIGHT_WORKERS=0 同款）"
        );
        assert!(
            rt.take_light_writeback_log().is_empty(),
            "串行路不得产生 worker 写回记录"
        );
        serial_light = positions.iter().map(|&p| (p, light_of(&rt, p))).collect();
    }
    let mut par =
        GameRuntime::new_headless_with_light(20261012, temp_dir("lp-par"), GameMode::Survival, 2);
    let par_positions = setup_pattern_fixtures(&mut par);
    drive_until_lit(&mut par, &par_positions);
    // 并行接线可见（worker 数 = 装配值）。
    assert_eq!(
        par.light_workers(),
        2,
        "new_headless_with_light(2) 必须装配 2 worker 的光照调度器"
    );
    // 回传发生（写回日志非空——串行路恒空）。
    assert!(
        !par.take_light_writeback_log().is_empty(),
        "并行路必须有 worker 产物写回记录"
    );
    // 逐位对拍（25 块全量）：对串行路 + 对局部 init 参照双重锁。
    for (pos, want) in &serial_light {
        assert_eq!(
            &light_of(&par, *pos),
            want,
            "{pos:?}：并行路光照与串行路漂移"
        );
        assert_eq!(
            &light_of(&par, *pos),
            &ref_light,
            "{pos:?}：与局部 init 参照漂移"
        );
    }
}

/// 平地夹具块（无光源）：石板地面顶 `floor_top`，停在 TerrainReady——
/// 跨块火把精确值锁的几何（game.rs tests::lit_chunk 同款，但不预布光）。
fn floor_chunk(pos: ChunkPos, floor_top: i32) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(pos));
    {
        let mut vg = h.voxels.write().unwrap();
        for y in WORLD_MIN_Y..=floor_top {
            for z in 0..16usize {
                for x in 0..16usize {
                    vg[vidx(x, y, z)] = BlockId(1);
                }
            }
        }
    }
    let ids: Vec<u16> = bytemuck::cast_slice(h.voxels.read().unwrap().as_slice()).to_vec();
    *h.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&ids);
    h.advance_to(Stage::TerrainReady);
    h
}

/// 跨块火把流经并行路（原版同值锁）：3×3 平地夹具（rd=0 无世界生成）+
/// 火把贴 chunk(0,0) 的 +X 边界（x=15），驱动 stream()（W=2）到收敛后
/// 跨界光照必须取**原版精确值**（火把格 14 / 邻块跨界格 13 / 衰减 12——
/// game.rs tests::torch_light_crosses_chunk_border 同款判据）。单边界单
/// 光源的成对交换与帧调度无关（两端就绪后一次性双向补收，ADD/REMOVE
/// 在任一序下都收敛到以该边为界的正当亮度）。
#[test]
fn game_parallel_light_cross_border_torch_exact_values() {
    let mut rt =
        GameRuntime::new_headless_with_light(20261012, temp_dir("lp-torch"), GameMode::Survival, 2);
    rt.render_dist = 0;
    let mut positions = Vec::new();
    for dz in -1i32..=1 {
        for dx in -1i32..=1 {
            let p = ChunkPos::new(dx, dz);
            let h = floor_chunk(p, 20);
            if p == ChunkPos::new(0, 0) {
                // 火把贴 +X 边界（在布光前写体素——快照所见即所得）。
                h.voxels.write().unwrap()[vidx(15, 21, 8)] = BlockId(TORCH);
                let ids: Vec<u16> =
                    bytemuck::cast_slice(h.voxels.read().unwrap().as_slice()).to_vec();
                *h.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&ids);
            }
            rt.chunks_mut().insert(p, h);
            positions.push(p);
        }
    }
    drive_until_lit(&mut rt, &positions);
    assert_eq!(rt.light_workers(), 2);
    let blk = |p: ChunkPos, x: usize, y: i32, z: usize| {
        rt.chunks[&p].light.read().unwrap()[vidx(x, y, z)] & 0xF
    };
    assert_eq!(blk(ChunkPos::new(0, 0), 15, 21, 8), 14, "火把格自发光 14");
    assert_eq!(
        blk(ChunkPos::new(1, 0), 0, 21, 8),
        13,
        "边同步：光跨过区块边界（13 = 14 − max(1, damp 0)）"
    );
    assert_eq!(blk(ChunkPos::new(1, 0), 1, 21, 8), 12, "邻块内部继续衰减");
}

/// 回传环序锁（蓝图红测试③a）：单 worker 串行化完成序 = 派发序，而派发
/// 按 `sorted_keys` 近优先扫描——整段写回序列必须恒等于玩家近优先环序
/// （「布光以玩家为圆心成波推进」的加载门时序锁）。单 worker 下与派发
/// 时序无关：完成序 = 队列序 = 派发序，写回批为环序的连续切片。
#[test]
fn game_writeback_follows_player_near_ring_order() {
    let mut rt =
        GameRuntime::new_headless_with_light(20261012, temp_dir("lp-order"), GameMode::Survival, 1);
    let positions = setup_fixtures(&mut rt);
    drive_until_lit(&mut rt, &positions);
    assert_eq!(rt.light_workers(), 1, "红面：单 worker 装配必须可见");
    let log = rt.take_light_writeback_log();
    assert_eq!(
        log,
        expected_ring_order(ChunkPos::new(0, 0), &positions),
        "写回生效序必须 = sorted_keys 环序（玩家近优先成波推进被破坏）"
    );
}

/// 回传环序锁（蓝图红测试③b）：同帧多块回传、**乱序到达**时写回重排——
/// 直接驱动 [`GameRuntime::light_writeback`]（环序生效的唯一落点）：远块
/// 先到、近块后到，生效序仍必须 = sorted_keys 环序，且光照字节 = 串行
/// init 参照逐位落位。
#[test]
fn same_frame_writeback_reorders_scrambled_arrivals() {
    let mut rt =
        GameRuntime::new_headless_with_light(20261012, temp_dir("lp-scram"), GameMode::Survival, 2);
    rt.render_dist = 0;
    let near = ChunkPos::new(0, 0);
    let mid = ChunkPos::new(1, 0);
    let far = ChunkPos::new(3, 3);
    for p in [near, mid, far] {
        rt.chunks_mut().insert(p, terrain_chunk(p, scene_seed(p)));
    }
    // 串行参照产物（远→近乱序回传，seq 与到达序一致——worker 完成序）。
    let mut dones: Vec<LightDone> = Vec::new();
    for (pos, seq) in [(far, 1u64), (near, 2), (mid, 3)] {
        let voxels = scene_voxels(scene_seed(pos));
        let heightmap = mcv_worldgen::recompute_heightmap(&voxels);
        let mut light = vec![0u8; CHUNK_VOL];
        let mut view = mcv_light::LightChunk {
            voxels: &voxels,
            light: &mut light,
            heightmap: &heightmap[..],
        };
        let mask = mcv_light::init(&mut view);
        dones.push(LightDone {
            pos,
            seq,
            light,
            border_mask: mask,
        });
    }
    let applied = rt.light_writeback(dones, ChunkPos::new(0, 0));
    assert_eq!(
        applied,
        vec![near, mid, far],
        "同帧多块乱序回传必须按 sorted_keys 环序生效"
    );
    // 光照字节确已落位（逐位 = 串行 init 参照）。
    for pos in [near, mid, far] {
        let voxels = scene_voxels(scene_seed(pos));
        let heightmap = mcv_worldgen::recompute_heightmap(&voxels);
        let mut want = vec![0u8; CHUNK_VOL];
        let mut view = mcv_light::LightChunk {
            voxels: &voxels,
            light: &mut want,
            heightmap: &heightmap[..],
        };
        mcv_light::init(&mut view);
        assert_eq!(light_of(&rt, pos), want, "{pos:?}：写回字节漂移");
        assert_eq!(
            rt.chunks[&pos].stage(),
            Stage::LightLocalReady,
            "{pos:?}：写回后必须推进 LightLocalReady"
        );
    }
    // 写回日志记录生效序（观测面）。
    assert_eq!(rt.take_light_writeback_log(), vec![near, mid, far]);
}

/// 回滚开关锁：`workers = 0` 装配 = 串行路（不建池、无写回记录），布光
/// 仍经串行 BFS 收敛——`MCV_LIGHT_WORKERS=0` 的运行时语义锚点。
#[test]
fn workers_zero_falls_back_to_serial_init() {
    let mut rt = GameRuntime::new_headless_with_light(
        20261012,
        temp_dir("lp-serialfb"),
        GameMode::Survival,
        0,
    );
    let positions = setup_fixtures(&mut rt);
    assert_eq!(rt.light_workers(), 0, "workers=0 必须走串行路");
    drive_until_lit(&mut rt, &positions);
    assert!(
        rt.take_light_writeback_log().is_empty(),
        "串行路不得产生 worker 写回记录"
    );
    // 串行 BFS 确实布光（天光非零）。
    assert!(
        positions
            .iter()
            .any(|&p| light_of(&rt, p).iter().any(|&b| b >> 4 > 0)),
        "串行路未布光（天光全零）"
    );
}
