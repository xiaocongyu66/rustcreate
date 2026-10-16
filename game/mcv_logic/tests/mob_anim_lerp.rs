//! mob 行走动画 + 渲染位置插值测试（纯 CPU，无 GPU）：
//!
//! 子案①（行走摆动）：spawn_mob 装配的怪（游戏内唯一装配面，敌对/被动
//! 同路）经 `mob_ai_system` 固定步驱动后，AnimState 走摆账本推进
//! （phase/amount > 0），并经 `collect_mob_instances_in` → MobInstance
//! 渲染矩阵落到僵尸双腿（对角步相位互异）——此前 AnimState 仅测试侧
//! passive 路线消费，游戏内恒零相位 → 僵尸走路腿不摆。
//!
//! 子案②（渲染位置瞬移）：渲染帧消费 `pos_old→pos` 的 partialTick 插值
//! （26.1 `lerp(xOld, x, partialTick)`）——收集端 partial=0/0.5/1 的渲染
//! 位置分别是上一步/中点/本步，clamp 兜底越界值；固定 tick 序列下每帧
//! 拿余量插值即消除步边界跳变。
//!
//! 驱动方式与 mobs_runtime 同构：手搓 `mcv_ecs::App` + 单石区块，逐步
//! 注入 `MobServices` 快照跑 `mob_ai_system`（系统与 GameRuntime 之间
//! 只有这一个耦合面）。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_ecs::{App, SysCtx};
use mcv_entity::{MobId, PhysBody, spawn_mob};
use mcv_logic::game::{MobServices, RENDER_DIST, mob_ai_system};
use mcv_logic::mob_render::collect_mob_instances_in;
use mcv_logic::ticket::{ENTITY_TICKING_LEVEL, TicketTable, TicketType};

/// 与 mobs_runtime 镜像的票表（模拟环门槛——环内才 tick）。
fn player_tickets(player: Vec3) -> TicketTable {
    let sim_dist = RENDER_DIST + 1;
    let pc = ChunkPos::new(
        (player.x / 16.0).floor() as i32,
        (player.z / 16.0).floor() as i32,
    );
    let mut t = TicketTable::new();
    t.add_ticket(
        pc,
        TicketType::PlayerSimulation,
        (ENTITY_TICKING_LEVEL - sim_dist).max(0) as u8,
    );
    for dx in -sim_dist..=sim_dist {
        for dz in -sim_dist..=sim_dist {
            t.add_ticket(
                ChunkPos::new(pc.x + dx, pc.z + dz),
                TicketType::PlayerLoading,
                ENTITY_TICKING_LEVEL as u8,
            );
        }
    }
    t
}

/// 单区块世界：y<64 实心石、以上空气。
fn world_chunk() -> HashMap<ChunkPos, Arc<ChunkHandle>> {
    let h = ChunkHandle::new(ChunkPos::new(0, 0));
    {
        let mut vox = h.voxels.write().unwrap();
        for ly in 0..64usize {
            for i in 0..256usize {
                vox.set_flat(mcv_core::vidx(i & 15, ly as i32, i >> 4), BlockId(1));
            }
        }
    }
    {
        let mut hm = h.heightmap.write().unwrap();
        for v in hm.iter_mut() {
            *v = 64;
        }
    }
    h.advance_to(Stage::Uploaded);
    let mut map = HashMap::new();
    map.insert(ChunkPos::new(0, 0), Arc::new(h));
    map
}

fn harness() -> App {
    let mut app = App::new();
    mcv_entity::register_mob_components(&mut app.world);
    app.schedule
        .add(mcv_ecs::Stage::Fixed, "mob_ai", mob_ai_system);
    app
}

/// 驱动一个固定步（on_tick 透传——AI 决策门即断言对象）。
fn step(app: &mut App, chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>, player: Vec3, on_tick: bool) {
    let App {
        world,
        resources,
        events,
        commands,
        schedule,
    } = app;
    resources.insert(MobServices {
        chunks: Arc::new(chunks.clone()),
        player_pos: player,
        on_tick,
        ticks_step: on_tick as u32,
        game_ticks: 0,
        monsters_burn: false,
        sky_darken: 0,
        difficulty: 2, // normal：敌对怪可索敌
        creative: false,
        tickets: Arc::new(player_tickets(player)),
    });
    let mut ctx = SysCtx {
        world,
        resources,
        events,
        commands,
    };
    schedule.run_stage(mcv_ecs::Stage::Fixed, &mut ctx);
}

/// 躯干（部位 1）矩阵平移列 = 渲染位置 + 模型 pivot 偏移（与
/// mob_render/part 部件约定一致，pivot y 分量按 kind 固定）。
fn torso(model: &[[[f32; 4]; 4]; 12]) -> Vec3 {
    let t = model[1];
    Vec3::new(t[3][0], t[3][1], t[3][2])
}

/// 子案②：partialTick 插值面——渲染位置 = lerp(pos_old, pos, partial)。
#[test]
fn render_pos_lerps_between_prev_and_current_step() {
    let mut app = App::new();
    mcv_entity::register_mob_components(&mut app.world);
    // 手搓一只僵尸：pos_old = 上一步位置、pos = 本步位置（每固定步开头
    // mob_ai_system 会重拍 pos_old；这里直接摆值免跑系统，插值语义隔离）。
    let e = app.world.spawn();
    app.world
        .insert(e, mcv_entity::MobKind(mcv_entity::MobId::ZOMBIE));
    app.world.insert(
        e,
        PhysBody {
            pos: Vec3::new(1.0, 64.0, 0.0),
            vel: Vec3::ZERO,
            on_ground: true,
            pos_old: Vec3::new(0.0, 64.0, 0.0),
        },
    );

    // 上一步位置（t=0）。
    let t0 = collect_mob_instances_in(&app.world, None, 0.0);
    assert_eq!(t0.len(), 1);
    let p0 = torso(&t0[0].models);
    assert!(
        (p0.x - 0.0).abs() < 1e-5,
        "partial=0 应取 pos_old.x，得 {p0}"
    );
    // 中点：跨步边界不跳变的核心断言（旧实现恒取 body.pos → 恒 x=1）。
    let mid = collect_mob_instances_in(&app.world, None, 0.5);
    let pm = torso(&mid[0].models);
    assert!((pm.x - 0.5).abs() < 1e-5, "partial=0.5 应插值中点，得 {pm}");
    // 本步位置（t=1）。
    let cur = collect_mob_instances_in(&app.world, None, 1.0);
    let p1 = torso(&cur[0].models);
    assert!((p1.x - 1.0).abs() < 1e-5, "partial=1 应取 pos.x，得 {p1}");
    // 越界 clamp：>1 封顶到本步位置（渲染帧久未喂步时不外推）。
    let over = collect_mob_instances_in(&app.world, None, 7.0);
    let po = torso(&over[0].models);
    assert!(
        (po.x - 1.0).abs() < 1e-5,
        "partial>1 应 clamp 到 pos.x，得 {po}"
    );
    for m in [&t0, &mid, &cur, &over] {
        for mm in &m[0].models {
            for col in mm {
                for v in col {
                    assert!(v.is_finite(), "矩阵必须有限");
                }
            }
        }
    }
}

/// 子案①：固定 tick 序列驱动僵尸追击（真实 AI 通路），行走账本推进并
/// 经渲染矩阵落到双腿（两腿相位互异 = 摆动非零相）。
#[test]
fn zombie_walk_ledger_advances_and_swings_legs() {
    let mut app = harness();
    let chunks = world_chunk();
    // 玩家南侧 8 格：MeleeAttackGoal 索敌 → Walk 意图（Brain 路线），
    // 僵尸朝玩家真实走（与 mobs_runtime 同一 movement 通路）。
    let player = Vec3::new(8.0, 64.0, 8.0);
    let z = spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(8.0, 64.0, 14.0));

    let start = app.world.read::<PhysBody>().get(z).unwrap().pos;
    for _ in 0..60 {
        step(&mut app, &chunks, player, true);
    }
    let body = *app.world.read::<PhysBody>().get(z).unwrap();
    let moved = (body.pos - start).length();
    assert!(moved > 1.0, "60 步内僵尸应向玩家位移，实际 {moved:.3}");

    // 账本面：horiz > 0.15 m/s 的步数充分 → amount 收敛上行、phase 推进。
    let anims = app.world.read::<mcv_entity::AnimState>();
    let anim = anims.get(z).expect("spawn_mob 应装配 AnimState");
    assert!(
        anim.amount > 0.1,
        "行走幅值应上行（原版满速 0.88），实际 {}",
        anim.amount
    );
    assert!(anim.phase > 0.1, "行走相位应推进，实际 {}", anim.phase);

    // 渲染消费面：摆动写入 MobInstance 矩阵。测试期固定账本值做确定性
    // 断言（AI 跑出的 phase 可能落在 cos→0 的退火点，幅值改动偶发不可见；
    // phase=1.1 → cos≈0.45 非零）：基线 = 同位置（含 collect 的 partial=0.5
    // 插值位置！否则躯干平移差会虚构通过）/朝向/相位 + 零幅值——双腿
    // WalkLeg 矩阵应只被非零 amount 改写。
    *app.world
        .write::<mcv_entity::AnimState>()
        .get_mut(z)
        .unwrap() = mcv_entity::AnimState {
        phase: 1.1,
        amount: 0.88,
        ..Default::default()
    };
    let walk_yaw = app
        .world
        .read::<mcv_entity::Yaw>()
        .get(z)
        .map(|y| y.0)
        .unwrap_or(0.0);
    let render_pos = body.pos_old + (body.pos - body.pos_old) * 0.5;
    let baseline = mcv_render::mob_mesh::mob_model_matrices(
        mcv_render::mob_mesh::MobModelKind::Zombie,
        &mcv_render::mob_mesh::MobPose {
            pos: render_pos,
            yaw: walk_yaw,
            phase: 1.1,
            amount: 0.0,
            ..Default::default()
        },
    );
    let mobs = collect_mob_instances_in(&app.world, None, 0.5);
    assert_eq!(mobs.len(), 1);
    for idx in [4usize, 5usize] {
        assert_ne!(
            mobs[0].models[idx],
            baseline[idx].to_cols_array_2d(),
            "腿 {idx} 矩阵应偏离零幅值基线（WalkLeg 被 amount 改写）"
        );
    }
    for part in &mobs[0].models {
        for col in part {
            for v in col {
                assert!(v.is_finite(), "矩阵必须有限");
            }
        }
    }
}

/// 子案②端到端：固定 tick 序列内多帧收集，partial=0/1 渲染位置分别落在
/// 上一步/本步的步进端点（相机帧率 ≠ 步率时由插值缝合，不再瞬移）。
#[test]
fn fixed_tick_sequence_frame_positions_track_steps() {
    let mut app = harness();
    let chunks = world_chunk();
    let player = Vec3::new(8.0, 64.0, 8.0);
    let z = spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(8.0, 64.0, 14.0));
    for _ in 0..10 {
        step(&mut app, &chunks, player, true);
    }

    let body = *app.world.read::<PhysBody>().get(z).unwrap();
    // 内部账本与渲染端点一致：partial=1 → 本步 pos；partial=0 → 上一步
    // pos_old（矩阵平移列减 hub 偏移直接比对 x/z；y 含 pivot 与落地差，
    // x/z 是行走位移的主轴）。
    let at1 = collect_mob_instances_in(&app.world, None, 1.0);
    let p1 = torso(&at1[0].models);
    let at0 = collect_mob_instances_in(&app.world, None, 0.0);
    let p0 = torso(&at0[0].models);
    assert!(
        (p1.x - body.pos.x).abs() < 1e-4 && (p1.z - body.pos.z).abs() < 1e-4,
        "partial=1 应等于本步位置 {p1} vs {body:?}"
    );
    assert!(
        (p0.x - body.pos_old.x).abs() < 1e-4 && (p0.z - body.pos_old.z).abs() < 1e-4,
        "partial=0 应等于上一步位置 {p0} vs pos_old {:?}",
        body.pos_old
    );
    // 中点不越出两端点包围盒（顺序无关的插值收敛性）。
    let mid = collect_mob_instances_in(&app.world, None, 0.5);
    let pm = torso(&mid[0].models);
    let (lo_x, hi_x) = (p0.x.min(p1.x), p0.x.max(p1.x));
    assert!(
        pm.x >= lo_x - 1e-4 && pm.x <= hi_x + 1e-4,
        "中点应落在步进端点之间：{pm} ∉ [{lo_x}, {hi_x}]"
    );
}
