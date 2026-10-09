//! 被动生物运行时无头测试（无 GPU）：固定随机种子 → 驱动 `passive_ai_system`
//! + `mcv_game` AABB 物理步进，断言位移有界、不穿墙、受击逃离、动画账本。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_ecs::{App, SysCtx};
use mcv_entity::{MobId, PassiveMode, PassiveServices, passive_ai_system, spawn_passive_mob};

/// 石地世界：x,z ∈ [0,16)，y<64 实心石，64 以上空气。
fn stone_chunk() -> HashMap<ChunkPos, Arc<ChunkHandle>> {
    let h = ChunkHandle::new(ChunkPos::new(0, 0));
    {
        let mut vox = h.voxels.write().unwrap();
        for ly in 0..64usize {
            for i in 0..256usize {
                vox[(ly << 8) | i] = BlockId(1); // stone
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

fn harness(chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>, player: Vec3, seed: u64) -> App {
    let mut app = App::new();
    mcv_entity::register_mob_components(&mut app.world);
    mcv_entity::register_passive_components(&mut app.world);
    app.schedule
        .add(mcv_ecs::Stage::Fixed, "passive_ai", passive_ai_system);
    // 注入宿主快照（与运行时 fixed_step 同款耦合面）。
    app.resources.insert(PassiveServices {
        chunks: chunks.clone(),
        player_pos: player,
        on_tick: true,
        seed,
    });
    app
}

/// 驱动一个固定步（1/60 s；每步都是 tick 边界，决策+物理一步到位）。
fn step(app: &mut App) {
    let App {
        world,
        resources,
        events,
        commands,
        schedule,
    } = app;
    let mut ctx = SysCtx {
        world,
        resources,
        events,
        commands,
    };
    schedule.run_stage(mcv_ecs::Stage::Fixed, &mut ctx);
}

// ---------------------------------------------------------------------------
// 1) 游走位移有界：固定种子跑 3600 tick（60 s）。两层断言：
//    a) 机制界——Wander 中到目标点距离 ≤ 游走半径（DefaultRandomPos
//       getPos(mob,10,7) 在意图点 10 格内掷点，行进中只减不增）；
//    b) 漂移兜底——随机游走逐腿累积漂移（本种子实测 ~32.5），设
//       6×半径 的粗界防飞出/NaN，非机制紧界。
// ---------------------------------------------------------------------------

#[test]
fn wander_displacement_is_bounded() {
    let chunks = stone_chunk();
    let mut app = harness(&chunks, Vec3::new(60.0, 64.0, 60.0), 20260101);
    spawn_passive_mob(&mut app.world, MobId::SHEEP, Vec3::new(8.0, 64.0, 8.0));
    let origin = Vec3::new(8.0, 64.0, 8.0);
    let wander_radius = mcv_entity::ai::WANDER_RADIUS; // 10.0（ai.rs:40）
    let mut moved = 0.0f32;
    for _ in 0..3600 {
        step(&mut app);
        let bodies = app.world.read::<mcv_entity::PhysBody>();
        let wanders = app.world.read::<mcv_entity::WanderState>();
        for (e, b) in bodies.iter() {
            moved = moved.max((b.pos - origin).length());
            assert!(b.pos.y >= 63.9, "羊不得陷入地下（y={}）", b.pos.y);
            if let Some(w) = wanders.get(e)
                && w.mode == PassiveMode::Wander
            {
                let d = (b.pos - w.target).length();
                assert!(
                    d <= wander_radius + 1.0,
                    "游走目标必须在起游点半径 {wander_radius} 内，实际 {d}"
                );
            }
        }
    }
    // DefaultRandomPos 半径 10：60 s 必然起游且漂移有粗界。
    assert!(moved > 0.5, "固定种子下 60 s 必然起游（最大位移 {moved}）");
    assert!(
        moved <= 6.0 * wander_radius,
        "位移必须有界（随机游走漂移兜底），实际 {moved}"
    );
}

// ---------------------------------------------------------------------------
// 2) 不穿墙：围一个 8x8 石室（墙在 x=2/x=13），跑 1200 tick 仍在室内
// ---------------------------------------------------------------------------

#[test]
fn wander_does_not_clip_walls() {
    let chunks = stone_chunk();
    {
        let h = chunks.get(&ChunkPos::new(0, 0)).unwrap();
        let mut vox = h.voxels.write().unwrap();
        // 两堵墙：x=2 与 x=13（y 64..80 全高），羊被夹在中间走廊。
        for y in 64..80usize {
            for z in 0..16usize {
                vox[(y << 8) | (z << 4) | 2] = BlockId(1);
                vox[(y << 8) | (z << 4) | 13] = BlockId(1);
            }
        }
    }
    let mut app = harness(&chunks, Vec3::new(60.0, 64.0, 60.0), 777);
    spawn_passive_mob(&mut app.world, MobId::COW, Vec3::new(8.0, 64.0, 8.0));
    for _ in 0..1200 {
        step(&mut app);
        let bodies = app.world.read::<mcv_entity::PhysBody>();
        for (_, b) in bodies.iter() {
            assert!(
                b.pos.x > 3.4 && b.pos.x < 12.6,
                "牛不得穿墙（x={}）",
                b.pos.x
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3) 受击逃离：hurt_flag 置位后进入 Panic、速度显著高于游走（牛 ×2.0）
// ---------------------------------------------------------------------------

#[test]
fn hurt_triggers_panic_sprint() {
    let chunks = stone_chunk();
    let mut app = harness(&chunks, Vec3::new(60.0, 64.0, 60.0), 42);
    let e = spawn_passive_mob(&mut app.world, MobId::COW, Vec3::new(8.0, 64.0, 8.0));
    // 直接触发事件输入（等价 try_attack 置位）。
    app.world
        .write::<mcv_entity::MobTicks>()
        .get_mut(e)
        .unwrap()
        .hurt_flag = true;
    let mut max_speed = 0.0f32;
    let mut panicked = false;
    for _ in 0..20 {
        step(&mut app);
        if let Some(w) = app.world.read::<mcv_entity::WanderState>().get(e) {
            if w.mode == PassiveMode::Panic {
                panicked = true;
            }
        }
        let bodies = app.world.read::<mcv_entity::PhysBody>();
        for (_, b) in bodies.iter() {
            let v = (b.vel.x * b.vel.x + b.vel.z * b.vel.z).sqrt();
            max_speed = max_speed.max(v);
        }
    }
    assert!(panicked, "受击必须进入 Panic 模式");
    // 牛 Panic 速度 = 0.2 属性 ×43.17×2.0 ≈ 17.3 m/s（加速到达前 >8）
    assert!(
        max_speed > 8.0,
        "逃离冲刺速度应显著高于游走，实际 {max_speed}"
    );
}

// ---------------------------------------------------------------------------
// 4) 行走动画账本：移动时 amount 抬升，静止后衰减归零
// ---------------------------------------------------------------------------

#[test]
fn walk_animation_builds_and_rests() {
    let chunks = stone_chunk();
    let mut app = harness(&chunks, Vec3::new(60.0, 64.0, 60.0), 7);
    let e = spawn_passive_mob(&mut app.world, MobId::PIG, Vec3::new(8.0, 64.0, 8.0));
    // 强制持续游走：把目标放远处并反复重掷。
    let mut peak = 0.0f32;
    for t in 0..240 {
        if t % 60 == 0 {
            if let Some(w) = app.world.write::<mcv_entity::WanderState>().get_mut(e) {
                w.mode = PassiveMode::Wander;
                w.target = Vec3::new(8.0 + (t % 7) as f32, 64.0, 8.0 - (t % 5) as f32);
            }
        }
        step(&mut app);
        if let Some(a) = app.world.read::<mcv_entity::AnimState>().get(e) {
            peak = peak.max(a.amount);
        }
    }
    assert!(peak > 0.3, "行走时摆幅必须建立，实际 {peak}");
    // 静止 180 tick（3 s）后归零。
    for _ in 0..180 {
        if let Some(w) = app.world.write::<mcv_entity::WanderState>().get_mut(e) {
            w.mode = PassiveMode::Eat;
            w.eat_ticks = 3;
        }
        step(&mut app);
    }
    let anims = app.world.read::<mcv_entity::AnimState>();
    let a = anims.get(e).unwrap();
    assert_eq!(a.amount, 0.0, "静止后摆幅必须归零");
}

// ---------------------------------------------------------------------------
// 5) 鸡慢落：同高度同 tick 数下落距离小于牛
// ---------------------------------------------------------------------------

#[test]
fn chicken_falls_slower_than_cow() {
    let chunks = stone_chunk();
    let mut cow_app = harness(&chunks, Vec3::new(60.0, 64.0, 60.0), 5);
    let mut chicken_app = harness(&chunks, Vec3::new(60.0, 64.0, 60.0), 5);
    spawn_passive_mob(&mut cow_app.world, MobId::COW, Vec3::new(8.0, 90.0, 8.0));
    spawn_passive_mob(
        &mut chicken_app.world,
        MobId::CHICKEN,
        Vec3::new(8.0, 90.0, 8.0),
    );
    for _ in 0..40 {
        step(&mut cow_app);
        step(&mut chicken_app);
    }
    let dy = |app: &App| {
        app.world
            .read::<mcv_entity::PhysBody>()
            .iter()
            .map(|(_, b)| b.pos.y)
            .next()
            .unwrap()
    };
    let cow_y = dy(&cow_app);
    let chicken_y = dy(&chicken_app);
    assert!(
        chicken_y > cow_y,
        "鸡应缓降：chicken {chicken_y} vs cow {cow_y}"
    );
}
