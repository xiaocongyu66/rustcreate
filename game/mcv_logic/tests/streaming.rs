//! 流式加载稳定性回归（真机 56adc8a「区块加载/网格崩坏」修复的锁）。
//!
//! 三个症状对应的断言：
//! 1. 抖动——小步移动与区块边界振荡期间，无区块在 2 帧内连续 add/remove；
//!    振荡段（center 在相邻两区块间翻转）零区块进出（卸载迟滞有效）。
//! 2. 收敛——停止移动后 render_dist 环内全部到达 Uploaded，且此后 60 帧
//!    状态（在册集合/stage/MESH 脏位）与光照数组逐字节零变更（无持续重
//!    网格/光乒乓）。
//! 3. 超时放行——30s 超时把玩家放进未就绪世界后，流式补载继续推进，
//!    出生邻域最终全 Uploaded（「方块只剩描边」的入口不卡死）。
//!
//! 无头路径约定：`new_headless` 用 NullMesher（不产出网格、不推进
//! Uploaded），测试用 [`emulate_upload`] 按生产同款门槛（3×3 邻域均
//! ≥LightLocalReady）手工推进状态机，等价真机 CxxMesher+MeshUploader 的
//! 建网格+上传（game.rs remesh 循环注释所述「由测试手工 advance_to」同款）。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkHandle, ChunkPos, Stage, dirty, vidx};
use mcv_game::VoxelAccess;
use mcv_logic::game::{GameMode, GamePhase, GameRuntime, WorldView};

/// 唯一临时存档目录（并行安全，镜像 game.rs 单测 headless_rt 的做法）。
fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-stream-{tag}-{:x}", h.finish()))
}

fn headless(tag: &str) -> GameRuntime {
    GameRuntime::new_headless(20261010, temp_dir(tag), GameMode::Survival)
}

/// 无头拼装一个完成本地布光的区块（经公开 API 复刻 game.rs 单测 lit_chunk）。
fn lit_chunk(x: i32, z: i32, floor_top: usize) -> Arc<ChunkHandle> {
    lit_custom(x, z, floor_top, |_| {})
}

/// [`lit_chunk`] + 装饰回调（在布光前改体素，供边同步用例构造亮暗混合边）。
fn lit_custom(
    x: i32,
    z: i32,
    floor_top: usize,
    decorate: impl FnOnce(&mut [BlockId]),
) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(x, z)));
    {
        let mut vg = h.voxels.write().unwrap();
        for y in 0..=floor_top {
            for zz in 0..16usize {
                for xx in 0..16usize {
                    vg[vidx(xx, y, zz)] = BlockId(1);
                }
            }
        }
        decorate(&mut vg[..]);
    }
    let ids: Vec<u16> = bytemuck::cast_slice(h.voxels.read().unwrap().as_slice()).to_vec();
    *h.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&ids);
    {
        let vg = h.voxels.read().unwrap();
        let mut lg = h.light.write().unwrap();
        let hg = h.heightmap.read().unwrap();
        let voxels: &[u16] = bytemuck::cast_slice(vg.as_slice());
        let mut view = mcv_light::LightChunk {
            voxels,
            light: &mut lg[..],
            heightmap: &hg[..],
        };
        mcv_light::init(&mut view);
    }
    h.advance_to(Stage::LightLocalReady);
    h
}

/// 生产同款网格上传替身：对「中心 ≥LightLocalReady 且 3×3 邻域均
/// ≥LightLocalReady」的区块推进 Uploaded 并消费 MESH 脏位（等价真机上
/// remesh 循环 + MeshUploader 的最终效果；有界预算之外的吞吐差异不影响
/// 收敛语义）。
fn emulate_upload(rt: &mut GameRuntime) {
    let keys: Vec<ChunkPos> = rt.chunks.keys().copied().collect();
    for pos in keys {
        let h = rt.chunks[&pos].clone();
        if (h.stage() as u8) < (Stage::LightLocalReady as u8) {
            continue;
        }
        let mut ready = true;
        for dx in -1i32..=1 {
            for dz in -1i32..=1 {
                match rt.chunks.get(&ChunkPos::new(pos.x + dx, pos.z + dz)) {
                    Some(n) if (n.stage() as u8) >= (Stage::LightLocalReady as u8) => {}
                    _ => ready = false,
                }
            }
        }
        if !ready {
            continue;
        }
        h.advance_to(Stage::Uploaded);
        h.clear_dirty(dirty::MESH);
    }
}

/// 每帧状态签名：在册区块 (x, z, stage, MESH 脏位) 全表 + render_chunks
/// 长度。SAVE 脏位不入签名（commit_terrain 落盘记账，与画面崩坏无关）。
fn signature(rt: &GameRuntime) -> Vec<(i32, i32, u32, u32)> {
    let mut sig: Vec<(i32, i32, u32, u32)> = rt
        .chunks
        .iter()
        .map(|(p, h)| (p.x, p.z, h.stage() as u32, (h.dirty() & dirty::MESH) as u32))
        .collect();
    sig.push((i32::MAX, i32::MAX, rt.render_chunks().len() as u32, 0));
    sig.sort_unstable();
    sig
}

/// 全部在册区块的光照数组快照（乒乓/漂移的逐字节证据）。
fn snapshot_light(rt: &GameRuntime) -> Vec<(i32, i32, Vec<u8>)> {
    let mut out: Vec<(i32, i32, Vec<u8>)> = rt
        .chunks
        .iter()
        .map(|(p, h)| (p.x, p.z, h.light.read().unwrap().to_vec()))
        .collect();
    out.sort_unstable_by_key(|(x, z, _)| (*x, *z));
    out
}

fn player_chunk(rt: &GameRuntime) -> ChunkPos {
    ChunkPos::new(
        (rt.player.pos.x / 16.0).floor() as i32,
        (rt.player.pos.z / 16.0).floor() as i32,
    )
}

/// 环内（Chebyshev ≤ render_dist）全部区块都到 Uploaded。
fn ring_ready(rt: &GameRuntime, center: ChunkPos) -> bool {
    let r = rt.render_dist;
    for dx in -r..=r {
        for dz in -r..=r {
            match rt.chunks.get(&ChunkPos::new(center.x + dx, center.z + dz)) {
                Some(h) if (h.stage() as u8) >= (Stage::Uploaded as u8) => {}
                _ => return false,
            }
        }
    }
    true
}

#[test]
fn small_step_movement_no_flap_and_converges() {
    let mut rt = headless("flap");
    rt.render_dist = 4;

    // 等出生投放（spawn 投放块按 heightmap 落地；本测试不改该块逻辑，
    // 只等它发生——投放后 player.pos 离开 ZERO，center 才稳定可控）。
    for _ in 0..600 {
        rt.stream();
        if rt.player.pos != Vec3::ZERO {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_ne!(rt.player.pos, Vec3::ZERO, "出生投放未发生（地形未就绪）");

    // —— 移动段：100 帧小步直线前进（每帧 0.8 格，跨 5 个区块边界），
    //    再 100 帧贴着「直线段结束时的实际 center 边界」±0.2 振荡（center
    //    在相邻区块间翻转；base 不能写死坐标——spawn 版投放的出生列不
    //    固定，写死会让振荡段起点偏离实际位置）。
    let mut prev: HashSet<ChunkPos> = rt.chunks.keys().copied().collect();
    let mut events: Vec<(u32, ChunkPos, bool)> = Vec::new(); // (帧, 区块, add?)
    let boundary = ((player_chunk(&rt).x + 1) * 16) as f32; // 当前 center 的 +X 边界
    for i in 0..200u32 {
        if i < 100 {
            rt.player.pos.x += 0.8;
        } else {
            rt.player.pos.x = boundary + if i % 2 == 0 { -0.2 } else { 0.2 };
        }
        rt.stream();
        let now: HashSet<ChunkPos> = rt.chunks.keys().copied().collect();
        for p in &now {
            if !prev.contains(p) {
                events.push((i + 1, *p, true));
            }
        }
        for p in &prev {
            if !now.contains(p) {
                events.push((i + 1, *p, false));
            }
        }
        prev = now;
        std::thread::sleep(Duration::from_millis(1));
    }

    // (a) 全程无 2 帧内同区块 add/remove 翻动（「动一下就换一批块」的病根）。
    for w in events.windows(2) {
        let same = w[0].1 == w[1].1;
        let near = w[1].0.saturating_sub(w[0].0) <= 2;
        assert!(
            !(same && near),
            "区块 {:?} 在 {}→{} 帧内连续 add/remove 抖动：{events:?}",
            w[0].1,
            w[0].0,
            w[1].0
        );
    }
    // (b) 振荡段内同一区块不得【既卸载又加载】：迟滞带（请求 ≤rd+1=sim_dist、
    //     卸载 >rd+4=unload_dist，fix/stream-collision 把带宽从 1 环加到
    //     3 环）下，center 在相邻区块间翻转只能让边缘块一次性离开或
    //     一次性补进，不会往复——「离开又回到半径」即迟滞失效（嫌疑 D
    //     的防御性锁；真机取证已裁决 D 非主因，本断言防回归）。振荡段
    //     开局的一次性卸载（骑跨起点重定位）与补载 backlog 的合法插入
    //     均不在本断言内。
    let mut removed: HashSet<ChunkPos> = HashSet::new();
    let mut flap: Vec<(u32, ChunkPos, bool)> = Vec::new();
    for (f, p, add) in events.iter().filter(|(f, _, _)| *f > 100) {
        if *add {
            if removed.contains(p) {
                flap.push((*f, *p, *add));
            }
        } else {
            removed.insert(*p);
        }
    }
    assert!(
        flap.is_empty(),
        "边界振荡期出现区块离开又回到半径（卸载迟滞失效）：{flap:?}"
    );

    // —— 收敛段：停止移动。静止判定 = 环内全 Uploaded 且在册区块全部完成
    //    本地布光（成对边同步的全部输入就绪，尾波走完的前提）；此后 30 帧
    //    余量 + 60 帧零变更窗。
    let center = player_chunk(&rt);
    let mut quiescent_at: Option<u32> = None;
    let mut settled = 0u32;
    let mut frame = 0u32;
    let mut window: Vec<Vec<(i32, i32, u32, u32)>> = Vec::new();
    let mut light_at_window_start: Option<Vec<(i32, i32, Vec<u8>)>> = None;
    for _ in 0..1500u32 {
        frame += 1;
        rt.stream();
        let all_lit = rt
            .chunks
            .values()
            .all(|h| (h.stage() as u8) >= (Stage::LightLocalReady as u8));
        if ring_ready(&rt, center) && all_lit {
            if quiescent_at.is_none() {
                quiescent_at = Some(frame);
            }
            settled += 1;
            if settled > 30 {
                if window.is_empty() {
                    light_at_window_start = Some(snapshot_light(&rt));
                }
                window.push(signature(&rt));
                if window.len() == 60 {
                    break;
                }
            }
        } else {
            quiescent_at = None;
            settled = 0;
            window.clear();
            light_at_window_start = None;
        }
        emulate_upload(&mut rt);
        std::thread::sleep(Duration::from_millis(1));
    }
    let at = quiescent_at.expect("停止移动后环内未全部到达 Uploaded（未收敛）");
    assert!(at <= 600, "环内收敛过慢：静止后 {at} 帧才就绪");
    assert_eq!(window.len(), 60, "未凑齐 60 帧零变更窗（未收敛）");
    let first = &window[0];
    assert!(
        window.iter().all(|s| s == first),
        "收敛后状态仍漂移（持续重网格/反复标脏）"
    );
    // 光照逐字节零变化：第二轮起边同步必须幂等（乒乓即在此暴露）。
    let light_at_end = snapshot_light(&rt);
    assert_eq!(
        light_at_window_start.as_ref().unwrap(),
        &light_at_end,
        "收敛后光照数组仍变化（边同步不幂等/乒乓）"
    );
}

#[test]
fn timeout_release_keeps_streaming_catchup() {
    let mut rt = headless("timeout");
    rt.render_dist = 4;
    // 不等就绪：把 20Hz tick 拨过 30s 等待截止（LevelLoadTracker.java:152-156
    // 超时放行），玩家被放进一个尚未准备好的世界——「方块只剩描边」入口。
    rt.game_ticks = 601;
    rt.fixed_step(1.0 / 60.0);
    assert_eq!(
        mcv_logic::game::GamePhase::Playing,
        rt.phase,
        "超时后必须放行"
    );
    // 放行不是停摆：流式补载继续推进，出生点 7×7 邻域（半径 3，26.1
    // EXPECTED_PLAYER_CHUNKS）最终全部 Uploaded。
    let mut ready = false;
    for _ in 0..1200u32 {
        rt.stream();
        emulate_upload(&mut rt);
        let (done, total) = rt.loading_progress_parts();
        if done >= total && total > 0 {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(ready, "超时放行后出生邻域未在 1200 帧内补载就绪");
}

/// 提取 `from` 的 `side` 边快照（mcv_light 边协议，0=+X 1=-X 2=+Z 3=-Z）。
fn extract_edge(from: &ChunkHandle, side: u8) -> [u8; 4096] {
    let vg = from.voxels.read().unwrap();
    let mut lg = from.light.write().unwrap();
    let hg = from.heightmap.read().unwrap();
    let voxels: &[u16] = bytemuck::cast_slice(vg.as_slice());
    let view = mcv_light::LightChunk {
        voxels,
        light: &mut lg[..],
        heightmap: &hg[..],
    };
    mcv_light::extract_edge(&view, side)
}

/// 把边快照按 REMOVE(1) 先 ADD(0) 后两相施加到 `to` 的 `side` 侧，
/// 返回两相的脏掩码（镜像 game.rs `sync_light_edges` 的两相次序）。
fn apply_edge_pair(to: &ChunkHandle, edge: &[u8; 4096], side: u8) -> (u8, u8) {
    let mut out = (0u8, 0u8);
    for (i, op) in [1u8, 0u8].into_iter().enumerate() {
        let vg = to.voxels.read().unwrap();
        let mut lg = to.light.write().unwrap();
        let hg = to.heightmap.read().unwrap();
        let voxels: &[u16] = bytemuck::cast_slice(vg.as_slice());
        let mut view = mcv_light::LightChunk {
            voxels,
            light: &mut lg[..],
            heightmap: &hg[..],
        };
        let d = mcv_light::apply_edge(&mut view, edge, side, op);
        if i == 0 {
            out.0 = d;
        } else {
            out.1 = d;
        }
    }
    out
}

#[test]
fn paired_edge_sync_is_idempotent() {
    // 直接检验边协议的数学幂等性（嫌疑 A：REMOVE+ADD 两相乒乓）：
    // 同一条边快照重复施加第二轮，光照必须零变化、级联脏掩码必须为 0。
    // 场景：B 在 y=50 架一块全尺寸顶板——y 11..49 成封闭暗腔（init 只播
    // 天板之上的直天光），唯一进光口是 A 侧的亮边（A 平地边缘 y≥21=15）。
    // 首轮 A→B 的 ADD 相把 B 西缘抬到 14 并向腔内传播；第二轮同边同相
    // `s < n−dec` 不再触发、REMOVE 相 `s > n+1` 也不触发——固定点。
    // （不用「A 暗边撤 B 亮边」的构造：撤回光会被 B 的直天光柱经
    // spread_target 的直落规则立刻原值重新证成，两轮间本就零净变化，
    // 区分不出幂等与空转。）
    let a = lit_chunk(0, 0, 20);
    let b = lit_custom(1, 0, 10, |vg| {
        // y=50 全尺寸顶板：其下腔体对外封闭（±Z/+X 无邻块可进光）。
        for z in 0..16usize {
            for x in 0..16usize {
                vg[vidx(x, 50, z)] = BlockId(1);
            }
        }
    });

    let light_of = |h: &ChunkHandle| h.light.read().unwrap().to_vec();
    let b_before = light_of(&b);

    // 首轮：A→B 与成对的 B→A（sync_light_edges 的成对双向语义）。
    let _ = apply_edge_pair(&b, &extract_edge(&a, 0), 1);
    let b_once = light_of(&b);
    let _ = apply_edge_pair(&a, &extract_edge(&b, 1), 0);
    let a_once = light_of(&a);
    let b_after_back = light_of(&b);

    // 首轮必须真的做过事：A 的亮边把光喂进 B 的暗腔。
    assert_ne!(b_before, b_once, "首轮 A→B 未改写 B 的光照——用例空转");

    // 第二轮：同一条边、同样的 REMOVE+ADD——必须零级联脏、零光照变化。
    let (d2r, d2a) = apply_edge_pair(&b, &extract_edge(&a, 0), 1);
    assert_eq!((d2r, d2a), (0, 0), "第二轮 A→B 边同步仍报级联脏（不幂等）");
    assert_eq!(b_after_back, light_of(&b), "第二轮 A→B 光照变化");
    let (e2r, e2a) = apply_edge_pair(&a, &extract_edge(&b, 1), 0);
    assert_eq!((e2r, e2a), (0, 0), "第二轮 B→A 边同步仍报级联脏（不幂等）");
    assert_eq!(a_once, light_of(&a), "第二轮 B→A 光照变化");
    assert_eq!(b_once, light_of(&b), "反向同步扰动首轮结果（乒乓）");
}

// ---------------------------------------------------------------------------
// fix/stream-collision：模拟区不变量 / 卸载迟滞 / 在途驻留 / 复活缓存 /
// 无 unloaded-石代理。原版对照：玩家区块恒持 PLAYER_SIMULATION ticket
// （DistanceManager.java:110-117），界外列 = VOID_AIR（Level.java:361-363）。
// ---------------------------------------------------------------------------

/// 廉价就位块：TerrainReady + y<5 石地板（够玩家站立），不做光照/体素工程。
fn floor_chunk(x: i32, z: i32) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(x, z)));
    {
        let mut v = h.voxels.write().unwrap();
        for y in 0..5usize {
            for zz in 0..16usize {
                for xx in 0..16usize {
                    v[vidx(xx, y, zz)] = BlockId(1);
                }
            }
        }
    }
    h.advance_to(Stage::TerrainReady);
    h
}

/// WorldView 未加载/Empty 列 = 空气（隐形墙代理已删）；就位后读真体素。
#[test]
fn unloaded_columns_read_as_air_not_stone() {
    let chunks: HashMap<ChunkPos, Arc<ChunkHandle>> = HashMap::new();
    let view = WorldView { chunks: &chunks };
    assert_eq!(
        view.block(BlockPos::new(100, 70, -37)),
        BlockId(0),
        "缺区块列必须是空气：26.1 Level.java:361-363 VOID_AIR，旧石安全垫=隐形墙根因"
    );
    // Empty 态（已入册、体素未 commit）：即便数组里有数据也按空气读。
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(0, 0)));
    h.voxels.write().unwrap()[vidx(3, 4, 5)] = BlockId(1);
    chunks.into_iter().for_each(drop); // 借用检查占位（下方重新构造）
    let mut chunks2: HashMap<ChunkPos, Arc<ChunkHandle>> = HashMap::new();
    chunks2.insert(ChunkPos::new(0, 0), h.clone());
    let view2 = WorldView { chunks: &chunks2 };
    assert_eq!(
        view2.block(BlockPos::new(3, 4, 5)),
        BlockId(0),
        "Empty 体素未就位，读空气"
    );
    h.advance_to(Stage::TerrainReady);
    assert_eq!(
        view2.block(BlockPos::new(3, 4, 5)),
        BlockId(1),
        "就位后读真体素"
    );
    assert_eq!(
        view2.block(BlockPos::new(3, 5, 5)),
        BlockId(0),
        "就位后空气列是真空气"
    );
}

/// 模拟区门：玩家本块未就位 → 物理整步冻结（不走、不坠、不查询未加载列）。
#[test]
fn physics_frozen_until_own_chunk_terrain_ready() {
    let mut rt = headless("freeze");
    rt.phase = GamePhase::Playing;
    rt.player.pos = Vec3::new(8.5, 5.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    rt.player.yaw = std::f32::consts::FRAC_PI_2; // 朝 +X
    rt.input.forward = true;
    let before = rt.player.pos;
    for _ in 0..60 {
        rt.fixed_step(1.0 / 60.0);
    }
    assert_eq!(
        rt.player.pos, before,
        "本块未就位必须冻结（旧行为：撞隐形石墙或坠虚空）"
    );
    // 就位后物理恢复。注意 step 的次序（physics.rs:389-408）是「先按当前
    // 速度位移、后积分输入速度」——从静止起步第 1 步只建立速度（vel.x>0）
    // 而零位移，位移从第 2 步开始，故恢复断言须走多步。
    rt.chunks.insert(ChunkPos::new(0, 0), floor_chunk(0, 0));
    for _ in 0..10 {
        rt.fixed_step(1.0 / 60.0);
    }
    assert_ne!(rt.player.pos, before, "就位后物理恢复");
    assert!(rt.player.pos.x > 8.5, "恢复后应能前进");
}

/// 边缘钳制（r_safe=1：仅 3×3 就位）：玩家贴边行走被钉回本区块内侧，
/// 永不越进未就位列（AABB 触达 ≤ ring1 ≤ 已就位环）。
#[test]
fn edge_clamp_pins_player_inside_ready_ring() {
    let mut rt = headless("clamp1");
    rt.render_dist = 4;
    for dx in -1..=1 {
        for dz in -1..=1 {
            rt.chunks.insert(ChunkPos::new(dx, dz), floor_chunk(dx, dz));
        }
    }
    rt.phase = GamePhase::Playing;
    rt.player.pos = Vec3::new(8.5, 5.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    rt.player.yaw = std::f32::consts::FRAC_PI_2; // 朝 +X
    rt.input.forward = true;
    for _ in 0..600 {
        rt.fixed_step(1.0 / 60.0);
        assert!(
            rt.player.pos.x < 16.0,
            "钳制失效，玩家越进未就位区块：x={}",
            rt.player.pos.x
        );
        assert!(
            rt.player.pos.z >= 0.0 && rt.player.pos.z < 16.0,
            "Z 同理：z={}",
            rt.player.pos.z
        );
    }
    assert!(
        rt.player.pos.x > 15.0,
        "应被推逼到钳制边界附近：x={}",
        rt.player.pos.x
    );
    assert_eq!(rt.chunks.len(), 9, "物理路径不得偷偷加载新区块");
    // 模拟区不变量：玩家 AABB 触达列（pos±1 格取整）恒 ≥TerrainReady。
    for dx in -1..=1 {
        for dz in -1..=1 {
            let c = ChunkPos::new(
                ((rt.player.pos.x + dx as f32) / 16.0).floor() as i32,
                ((rt.player.pos.z + dz as f32) / 16.0).floor() as i32,
            );
            assert!(
                rt.chunks
                    .get(&c)
                    .is_some_and(|h| (h.stage() as u8) >= (Stage::TerrainReady as u8)),
                "玩家邻域列 {c:?} 未就位——物理查询会命中未加载列"
            );
        }
    }
}

/// 边缘钳制（r_safe=2：±2 环就位）：可活动区扩到 pc±1 区块，钳制位 =
/// (pc+2)*16 − pad；同样永不越界。
#[test]
fn edge_clamp_allows_full_ready_margin() {
    let mut rt = headless("clamp2");
    rt.render_dist = 4;
    for dx in -2..=2 {
        for dz in -2..=2 {
            rt.chunks.insert(ChunkPos::new(dx, dz), floor_chunk(dx, dz));
        }
    }
    rt.phase = GamePhase::Playing;
    rt.player.pos = Vec3::new(8.5, 5.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    rt.player.yaw = std::f32::consts::FRAC_PI_2;
    rt.input.forward = true;
    for _ in 0..600 {
        rt.fixed_step(1.0 / 60.0);
    }
    // r_safe=2 → 矩形 = pc±1 区块：x ≤ 32 − pad（pad = 0.32）。
    assert!(
        rt.player.pos.x < 32.0,
        "越进未就位区块：x={}",
        rt.player.pos.x
    );
    assert!(
        rt.player.pos.x > 31.0,
        "应被推逼到钳制边界附近：x={}",
        rt.player.pos.x
    );
}

/// 卸载迟滞 ≥2 环（fix/stream-collision）：旧阈值 rd+2 会把 rd+3 环逐帧
/// 撤回重请求；新阈值 rd+4 下 rd+2..rd+4 滞留环保留。
#[test]
fn unload_hysteresis_keeps_stay_ring() {
    let mut rt = headless("hyst");
    rt.render_dist = 2; // sim=3，unload=6，旧阈值=4
    for dx in -6..=6 {
        for dz in -6..=6 {
            rt.chunks.insert(ChunkPos::new(dx, dz), floor_chunk(dx, dz));
        }
    }
    rt.player.pos = Vec3::new(8.5, 5.0, 8.5);
    for _ in 0..5 {
        rt.stream();
    }
    assert!(
        rt.chunks.contains_key(&ChunkPos::new(5, 0)),
        "rd+3 滞留环被误卸（迟滞失效）"
    );
    assert!(
        rt.chunks.contains_key(&ChunkPos::new(6, 0)),
        "rd+4 滞留环被误卸"
    );
    assert_eq!(rt.sim_dist(), 3);
    assert_eq!(rt.unload_dist(), 6);
    assert!(rt.unload_dist() - rt.sim_dist() >= 2, "迟滞带宽要求 ≥2 环");
}

/// 卸载预算 + pending_unloads 复活缓存（原版 ChunkMap.java:388-392）：
/// 单帧淘汰 ≤4；玩家走远再回来，区块以同一 Arc 复活（不重 IO、不重生成、
/// stage 保留），全程无「卸载后重新请求生成」的空窗。
#[test]
fn unload_budget_and_pending_revival() {
    let mut rt = headless("pending");
    rt.render_dist = 2;
    for dx in -6..=6 {
        for dz in -6..=6 {
            rt.chunks.insert(ChunkPos::new(dx, dz), floor_chunk(dx, dz));
        }
    }
    rt.player.pos = Vec3::new(8.5, 5.0, 8.5);
    rt.stream();
    let home = rt.chunks[&ChunkPos::new(0, 0)].clone();
    let stage_before = home.stage();

    // 走远：单帧淘汰必须 ≤4（原版 processUnloads 时间片）。
    rt.player.pos = Vec3::new(150.5, 5.0, 8.5); // 新中心 (9,0)，(0,0) 距 9
    let prev: HashSet<ChunkPos> = rt.chunks.keys().copied().collect();
    rt.stream();
    let now: HashSet<ChunkPos> = rt.chunks.keys().copied().collect();
    let evicted = prev.iter().filter(|c| !now.contains(c)).count();
    assert!(evicted <= 4, "单帧淘汰 {evicted} 块，超过卸载预算 4");

    // 逐帧走到 (0,0) 真的被摘出活动表（时间片排队：它前面恒有近端候选，
    // 必然出现在某帧 ≤4 的淘汰批次里；≤40 帧兜底失败暴露迟滞/预算异常）。
    let mut gone_frames = 0usize;
    while rt.chunks.contains_key(&ChunkPos::new(0, 0)) {
        rt.stream();
        gone_frames += 1;
        assert!(gone_frames < 40, "(0,0) 始终未被淘汰，用例装配失效");
    }

    // 走回来：请求环 miss 先命中复活缓存 → 同一 Arc、stage 原样。
    rt.player.pos = Vec3::new(8.5, 5.0, 8.5);
    rt.stream();
    let Some(revived) = rt.chunks.get(&ChunkPos::new(0, 0)).cloned() else {
        panic!("回环后出生区块未复活/未重载");
    };
    assert!(
        Arc::ptr_eq(&revived, &home),
        "复活必须命中 pending_unloads（同一 Arc），实际走了重新加载/生成路径"
    );
    assert_eq!(
        revived.stage(),
        stage_before,
        "复活保留 stage（数据从未丢失）"
    );
}

/// 在途请求驻留：已发 worker、体素未回（Empty）的区块不逐帧撤回——
/// 走远后留在册（等待结果），杜绝「请求→撤回→重请求→再生成」乒乓。
/// 确定性构造：等 worker 的**唯一**结果到达并由测试自己从 channel 吞掉
/// ——此后 stream() 的 drain 永远等不到东西，(3,0) 恒为 Empty，驻留断言
/// 不再与 worker 速度赛跑（结果若被 stream 消费，提交后的卸载是正确
/// 行为，测试会误报）。
#[test]
fn inflight_request_survives_walkaway() {
    let mut rt = headless("inflight");
    rt.render_dist = 2; // sim=3：请求环 ≤3
    for dx in -6..=6 {
        for dz in -6..=6 {
            if dx == 3 && dz == 0 {
                continue; // 故意缺席：让 stream 真实发出 worker 请求
            }
            rt.chunks.insert(ChunkPos::new(dx, dz), floor_chunk(dx, dz));
        }
    }
    rt.player.pos = Vec3::new(8.5, 5.0, 8.5);
    rt.stream(); // 螺旋到 r=3 时请求 (3,0) → 入册为 Empty（在途）
    let inflight = rt.chunks.get(&ChunkPos::new(3, 0));
    match inflight {
        // 存档命中路径（try_load_saved）不可能：temp 目录全新。
        None => panic!("(3,0) 未被请求入册，用例装配失效"),
        Some(h) => assert_eq!(
            h.stage(),
            Stage::Empty,
            "(3,0) 已被 stream 内联 drain 提交（worker 快于入册帧，环境异常）"
        ),
    }

    // 吞掉 worker 结果：结果不再可能被 stream 提交，Empty 状态恒定。
    let mut absorbed = false;
    for _ in 0..5000 {
        if let Ok(r) = rt.scheduler.results().try_recv() {
            match r {
                mcv_worldgen::GenResult::Terrain(Ok(out)) => {
                    assert_eq!(out.pos, ChunkPos::new(3, 0), "只应存在一个请求");
                    absorbed = true;
                }
                mcv_worldgen::GenResult::Terrain(Err((pos, rc))) => {
                    panic!("terrain gen failed at {pos:?}: {rc}");
                }
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(absorbed, "worker 未在预算内产出 (3,0) 结果");

    // 走远（(3,0) 距新中心 > unload_dist）：在途块驻留不撤回。
    rt.player.pos = Vec3::new(200.5, 5.0, 8.5);
    for _ in 0..10 {
        rt.stream();
    }
    let Some(held) = rt.chunks.get(&ChunkPos::new(3, 0)) else {
        panic!("在途（Empty）请求被逐帧撤回——回环后将重发请求再生成（症状 5 乒乓）");
    };
    assert_eq!(held.stage(), Stage::Empty, "驻留期间状态不许漂移");
}

/// y 越界不绕回（coords 审计 P2 + 任务板 #60）：越界列读空气而非绕回
/// 同列另一端；heightmap 全柱遮光时饱和 255 而非 u8 溢出绕 0。
#[test]
fn y_out_of_bounds_never_wraps() {
    let h = floor_chunk(0, 0);
    {
        let mut v = h.voxels.write().unwrap();
        for y in 0..256usize {
            v[vidx(4, y, 4)] = BlockId(1); // 全柱石头，顶格 = y255
        }
    }
    let mut chunks: HashMap<ChunkPos, Arc<ChunkHandle>> = HashMap::new();
    chunks.insert(ChunkPos::new(0, 0), h.clone());
    let view = WorldView { chunks: &chunks };
    assert_eq!(
        view.block(BlockPos::new(4, 255, 4)),
        BlockId(1),
        "界内顶格照旧"
    );
    assert_eq!(
        view.block(BlockPos::new(4, 256, 4)),
        BlockId(0),
        "y≥256 必须空气（旧行为 rem_euclid 绕回 y=0 读到实心=隐形地板/幽灵块）"
    );
    assert_eq!(
        view.block(BlockPos::new(4, -1, 4)),
        BlockId(0),
        "y<0 必须空气（旧行为绕回 y=255）"
    );
    // heightmap 饱和：全柱遮光（含 y255）→ 255，不得绕回 0。
    let ids: Vec<u16> = bytemuck::cast_slice(h.voxels.read().unwrap().as_slice()).to_vec();
    let hm = mcv_worldgen::recompute_heightmap(&ids);
    assert_eq!(
        hm[(4 << 4) | 4],
        255,
        "顶盖遮光 heightmap 饱和 255（u8 绕 0 = 出生/碰撞错位）"
    );
}
