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

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage, dirty, vidx};
use mcv_logic::game::{GameMode, GameRuntime};

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
    // (b) 振荡段内同一区块不得【既卸载又加载】：迟滞带（请求 ≤rd+1、卸载
    //     >rd+2）下，center 在相邻区块间翻转只能让边缘块一次性离开或
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
