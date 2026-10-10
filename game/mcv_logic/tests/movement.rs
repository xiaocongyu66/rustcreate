//! 玩家移动域 GameRuntime 级回归锁（fix/movement-physics）：
//! 20Hz tick 节拍、固定步 dt 抖动不变性、加载门控、触屏摇杆（恒冲刺/
//! 跳键悬真/模拟量）、双击跳切飞行、饥饿冲刺门、入水减速。
//! Java 依据均为 src-26.1（断言注释附 file:line）。

use std::{path::PathBuf, sync::Arc};

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_logic::game::{GameMode, GamePhase, GameRuntime};

/// 唯一临时存档目录（测试并行安全）。
fn tmp_world(tag: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-movement-{}-{:x}", tag, h.finish()))
}

/// 流式加载直到出生区块地形就绪。
fn wait_terrain(rt: &mut GameRuntime, pos: ChunkPos) {
    for _ in 0..3000 {
        rt.stream();
        if rt
            .chunks
            .get(&pos)
            .is_some_and(|h| (h.stage() as u8) >= (Stage::TerrainReady as u8))
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("地形未在预算内就绪");
}

/// 压平区块 (0,0)：y<70 全石、上方清空（顶面 y=70 可站立）。
fn build_flat(rt: &mut GameRuntime) {
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    {
        let mut v = h.voxels.write().unwrap();
        for y in 0..70usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    v[mcv_core::vidx(x, y as i32, z)] = BlockId(1);
                }
            }
        }
        for y in 70..96usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    v[mcv_core::vidx(x, y as i32, z)] = BlockId(0);
                }
            }
        }
    }
    h.advance_to(Stage::TerrainReady);
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
}

/// 出生区块压实 + 玩家落到 (2.5, 70, 8.5) + 49 块邻域顶到 Uploaded，
/// 步进到游玩态（同 runtime.rs::finish_loading 模式）。
fn enter_playing(rt: &mut GameRuntime) {
    wait_terrain(rt, ChunkPos::new(0, 0));
    build_flat(rt);
    rt.player.pos = Vec3::new(2.5, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            let pos = ChunkPos::new(dx, dz);
            rt.chunks
                .entry(pos)
                .or_insert_with(|| Arc::new(ChunkHandle::new(pos)))
                .advance_to(Stage::Uploaded);
        }
    }
    for _ in 0..60 {
        rt.fixed_step(1.0 / 60.0);
        if rt.phase == GamePhase::Playing {
            return;
        }
    }
    panic!("加载态未在预算步内转游玩态");
}

/// 朝向：yaw → camera.dir = (sin yaw, ·, −cos yaw)（camera.rs:17-23）；
/// yaw=π/2 朝 +X。
fn face_x() -> f32 {
    std::f32::consts::FRAC_PI_2
}

/// app.rs:2000-2009 的固定步驱动循环复刻（真实 dt 累积 → 整数个 1/60 步）。
fn run_frames(rt: &mut GameRuntime, frame_dt: f32, frames: usize) {
    let mut acc = 0.0f32;
    for _ in 0..frames {
        acc = (acc + frame_dt).min(0.2);
        while acc >= 1.0 / 60.0 {
            rt.fixed_step(1.0 / 60.0);
            acc -= 1.0 / 60.0;
        }
    }
}

// ---------------------------------------------------------------------------
// 20Hz tick 节拍（accumulate_ticks 是全仓 tick 语义唯一换算点）
// ---------------------------------------------------------------------------

#[test]
fn fixed_step_accumulates_exactly_20_ticks_per_second() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("ticks"), GameMode::Survival);
    let mut on_tick_steps = 0usize;
    for i in 0..60 {
        rt.fixed_step(1.0 / 60.0);
        if rt.on_tick {
            on_tick_steps += 1;
            // 1/60 步 ×20 = 1/3 tick/步 → 恰好每 3 步一个 tick。
            assert_eq!(i % 3, 2, "第 {i} 步 on_tick 违反 1/3 节拍");
        }
    }
    assert_eq!(on_tick_steps, 20, "60 步必须恰好 20 tick（20 tick/s）");
    assert_eq!(rt.game_ticks, 20);
    assert_eq!(rt.time_ticks, 20, "ServerClockManager 每 tick +1");
}

// ---------------------------------------------------------------------------
// 固定步不变性：30/60/240fps 帧序列 → 同样多的 1/60 物理步 → 位移一致
// ---------------------------------------------------------------------------

#[test]
fn dt_jitter_batches_same_motion() {
    let mk = |tag: &str| {
        let mut rt = GameRuntime::new_headless(7, tmp_world(tag), GameMode::Survival);
        enter_playing(&mut rt);
        rt.player.pos = Vec3::new(2.5, 70.0, 8.5);
        rt.player.vel = Vec3::ZERO;
        rt.player.yaw = face_x(); // 朝 +X
        rt.input.forward = true;
        rt
    };
    let mut a = mk("dt30");
    let mut b = mk("dt60");
    let mut c = mk("dt240");
    run_frames(&mut a, 1.0 / 30.0, 30); // 30fps：每帧 2 步
    run_frames(&mut b, 1.0 / 60.0, 60); // 60fps：每帧 1 步
    run_frames(&mut c, 1.0 / 240.0, 240); // 240fps：约每 4 帧 1 步
    for (tag, rt) in [("30fps", &a), ("60fps", &b), ("240fps", &c)] {
        let d = rt.player.pos.x - 2.5;
        assert!(
            d > 3.70 && d < 4.35,
            "{tag} 1s 位移 {d:.4} 不在 (3.70, 4.35)（固定步被帧率扰动）"
        );
        assert!(rt.player.pos.y > 69.9, "{tag} 不应陷地/飞起");
    }
    assert!((a.player.pos.x - b.player.pos.x).abs() < 0.1);
    assert!((a.player.pos.x - c.player.pos.x).abs() < 0.1);
    // tick 计数同样与帧率无关（1s = 20 tick）。
    assert_eq!(a.game_ticks, b.game_ticks);
    assert_eq!(a.game_ticks, c.game_ticks);
}

// ---------------------------------------------------------------------------
// 加载门控：GamePhase::Loading 屏蔽输入（26.1 LevelLoadingScreen 是活动
// Screen，LocalPlayer.java:776 只在 screen 为 null 时处理输入）
// ---------------------------------------------------------------------------

#[test]
fn loading_phase_gates_movement_input() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("gate"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_flat(&mut rt);
    rt.player.pos = Vec3::new(2.5, 70.0, 8.5);
    rt.player.yaw = face_x();
    rt.input.forward = true;
    rt.touch.enabled = true;
    rt.touch.stick_vec = (0.0, -56.0); // 触屏摇杆满偏（进场后走满速）
    // 邻域 49 块顶到 Uploaded（门开条件；同 enter_playing 的就绪路径）。
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            let pos = ChunkPos::new(dx, dz);
            rt.chunks
                .entry(pos)
                .or_insert_with(|| Arc::new(ChunkHandle::new(pos)))
                .advance_to(Stage::Uploaded);
        }
    }
    let mut guard = 0;
    while rt.phase == GamePhase::Loading {
        rt.fixed_step(1.0 / 60.0);
        assert_eq!(rt.player.vel.x, 0.0, "加载态移动输入应被清空");
        assert!(
            (rt.player.pos.x - 2.5).abs() < 1e-4,
            "加载态不得位移：{}",
            rt.player.pos.x
        );
        guard += 1;
        assert!(guard < 600, "加载态未按时转游玩态");
    }
    assert_eq!(rt.phase, GamePhase::Playing);
    // 进场后（输入被门控清过，重新给量）应正常起步。
    rt.input.forward = true;
    for _ in 0..60 {
        rt.fixed_step(1.0 / 60.0);
    }
    assert!(
        rt.player.pos.x > 5.5,
        "Playing 态输入未生效：x={}",
        rt.player.pos.x
    );
}

// ---------------------------------------------------------------------------
// 触屏摇杆（engine/mcv_platform/touch.rs → apply_touch_input → fixed_step）
// ---------------------------------------------------------------------------

#[test]
fn touch_stick_half_deflection_walks_not_sprints() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("stick"), GameMode::Survival);
    enter_playing(&mut rt);
    rt.player.pos = Vec3::new(2.5, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    rt.player.yaw = face_x();
    rt.touch.enabled = true;
    // 40px 偏移（> 死区 8，< 满冲刺门 0.85×56=47.6）：行走而非冲刺。
    // 旧实现把归一化单位向量再求模（恒 1.0）比较 0.85 → 轻推恒冲刺。
    rt.touch.stick_vec = (0.0, -40.0);
    for _ in 0..120 {
        rt.fixed_step(1.0 / 60.0);
    }
    let v = rt.player.vel.x;
    assert!(
        v > 2.0 && v < 4.0,
        "半杆速度 {v:.3} 不在 (2.0, 4.0)：恒冲刺 bug 未修（冲刺应 ≈5.6）"
    );
    // 满偏 56px：冲刺 + 全速（60 步足够从行走收敛到冲刺；120 步会走出区块）。
    rt.touch.stick_vec = (0.0, -56.0);
    for _ in 0..60 {
        rt.fixed_step(1.0 / 60.0);
    }
    assert!(
        (rt.player.vel.x - mcv_game::consts::SPRINT_SPEED).abs() < 0.2,
        "满杆应达冲刺稳态：{}",
        rt.player.vel.x
    );
}

#[test]
fn touch_jump_release_does_not_stick() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("tjump"), GameMode::Survival);
    enter_playing(&mut rt);
    rt.player.pos = Vec3::new(2.5, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    rt.touch.enabled = true;
    // 按住跳跃 4 步（起跳+上升），随后松开。
    rt.touch.jump_held = true;
    for _ in 0..4 {
        rt.fixed_step(1.0 / 60.0);
    }
    assert!(rt.player.vel.y > 0.0 || rt.player.pos.y > 70.0, "应已起跳");
    rt.touch.release_jump();
    for _ in 0..120 {
        rt.fixed_step(1.0 / 60.0);
    }
    // 旧实现松开不清 jump → 落地瞬间再起跳，永不落回。
    assert!(
        (rt.player.pos.y - 70.0).abs() < 0.01 && rt.player.on_ground,
        "松开跳跃后应落回地面：y={}, on_ground={}",
        rt.player.pos.y,
        rt.player.on_ground
    );
}

// ---------------------------------------------------------------------------
// 双击跳 = 切换创造飞行（LocalPlayer.java:827-848，窗口 7 tick）
// ---------------------------------------------------------------------------

#[test]
fn double_tap_jump_toggles_creative_flight() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("fly"), GameMode::Creative);
    enter_playing(&mut rt);
    rt.player.pos = Vec3::new(2.5, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    assert!(!rt.player.flying);
    // 一按一放，3 步内再按（≈1 tick，窗口 7 tick）。
    rt.input.jump = true;
    rt.fixed_step(1.0 / 60.0);
    rt.input.jump = false;
    rt.fixed_step(1.0 / 60.0);
    rt.fixed_step(1.0 / 60.0);
    rt.input.jump = true;
    rt.fixed_step(1.0 / 60.0);
    assert!(rt.player.flying, "双击跳应起飞");
    // 再来一次完整双击（原版语义：toggle 后 trigger 归零，须重新首按置
    // 窗口、再按才切换——LocalPlayer.java:835-846）→ 落回步行。
    rt.input.jump = false;
    rt.fixed_step(1.0 / 60.0);
    rt.input.jump = true;
    rt.fixed_step(1.0 / 60.0); // 首按沿：置 7 tick 窗口
    rt.input.jump = false;
    rt.fixed_step(1.0 / 60.0);
    rt.input.jump = true;
    rt.fixed_step(1.0 / 60.0); // 窗口内再按沿：切换
    assert!(!rt.player.flying, "再次双击应退出飞行");
    // 单按不切换（普通跳）。
    rt.input.jump = false;
    rt.fixed_step(1.0 / 60.0);
    rt.input.jump = true;
    rt.fixed_step(1.0 / 60.0);
    assert!(!rt.player.flying, "单按跳不得切飞行");
}

#[test]
fn double_tap_jump_ignored_in_survival() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("flysur"), GameMode::Survival);
    enter_playing(&mut rt);
    rt.player.pos = Vec3::new(2.5, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    rt.input.jump = true;
    rt.fixed_step(1.0 / 60.0);
    rt.input.jump = false;
    rt.fixed_step(1.0 / 60.0);
    rt.fixed_step(1.0 / 60.0);
    rt.input.jump = true;
    rt.fixed_step(1.0 / 60.0);
    assert!(!rt.player.flying, "生存无 mayfly（Player.java:827 门）");
}

// ---------------------------------------------------------------------------
// 饥饿冲刺门（Player.java:1569-1571 food>6；FoodConstants SPRINT_LEVEL=6）
// ---------------------------------------------------------------------------

#[test]
fn sprint_gate_blocks_low_hunger() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("hunger"), GameMode::Survival);
    enter_playing(&mut rt);
    rt.player.pos = Vec3::new(2.5, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    rt.player.yaw = face_x();
    rt.player.hunger = 5.0;
    rt.input.sprint = true;
    rt.input.forward = true;
    for _ in 0..120 {
        rt.fixed_step(1.0 / 60.0);
    }
    let v = rt.player.vel.x;
    assert!(
        (v - mcv_game::consts::WALK_SPEED).abs() < 0.15,
        "饥饿 ≤6 不得冲刺：{v:.3}（应 ≈4.317）"
    );
}

// ---------------------------------------------------------------------------
// 入水减速（wasTouchingWater = AABB 任一重叠，Entity.java:1566-1580；
// travelInWater 稳态 1.6 m/s，LivingEntity.java:2461+2366-2368）
// ---------------------------------------------------------------------------

#[test]
fn water_pool_slows_to_swim_speed() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("pool"), GameMode::Survival);
    enter_playing(&mut rt);
    // 平台上开一池 1 格深水（x 4..12, z 4..14），池底仍是石头。池要够长：
    // 行走速度入水减速需要数米，3 格池 0.35s 就冲出去了。
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    {
        let mut v = h.voxels.write().unwrap();
        for z in 4..14usize {
            for x in 4..12usize {
                v[mcv_core::vidx(x, 70, z)] = BlockId(5); // water（liquid=true）
            }
        }
    }
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
    rt.player.pos = Vec3::new(7.5, 70.0, 8.5); // 站在水里
    rt.player.vel = Vec3::ZERO;
    rt.player.yaw = std::f32::consts::PI; // 朝 +Z（沿池长边）
    rt.input.forward = true;
    for _ in 0..60 {
        rt.fixed_step(1.0 / 60.0);
    }
    let v = Vec3::new(rt.player.vel.x, 0.0, rt.player.vel.z).length();
    assert!(
        v > 0.9 && v < 2.5,
        "水中速度 {v:.3} 不在 (0.9, 2.5)：旧实现以走路速度涉水（≈4.3）"
    );
    // 仍在池内（未因速度异常被弹出）。
    assert!(rt.player.pos.z < 14.4, "不应冲出池：z={}", rt.player.pos.z);
    assert!(
        (rt.player.pos.y - 70.0).abs() < 0.02,
        "1 格深水中应站池底：y={}",
        rt.player.pos.y
    );
}
