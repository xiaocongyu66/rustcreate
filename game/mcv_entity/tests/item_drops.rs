//! 掉落物实体：物理沉降 / 寿命消失 / 确定性初速 / 拾取 / 合并守恒。
//! 系统直接驱动（同 mcv_game 物理测试模式），地面用单 ChunkHandle 假世界。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_entity::items::{
    DESPAWN_AGE, DropWorld, ItemDrop, item_merge_system, item_physics_system, item_pickup_system,
    register_drop_components, settle_pickups, spawn_item_drop,
};

/// 远离掉落物的玩家位姿（非拾取测试不被吸附）。
const NO_PLAYER: Vec3 = Vec3::new(1e4, 1e4, 1e4);

/// 单区块假世界：y<64 全实心石，其上空气。
fn floor_world() -> HashMap<ChunkPos, Arc<ChunkHandle>> {
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(0, 0)));
    {
        let mut v = h.voxels.write().unwrap();
        for y in 0..64u32 {
            for z in 0..16u32 {
                for x in 0..16u32 {
                    v[(y << 8 | z << 4 | x) as usize] = BlockId(1);
                }
            }
        }
    }
    h.advance_to(Stage::TerrainReady);
    let mut map = HashMap::new();
    map.insert(ChunkPos::new(0, 0), h);
    map
}

fn lcg(seed: u64) -> impl FnMut() -> u32 {
    let mut s = seed | 1;
    move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
    }
}

/// 建 App：注册组件 + 物理/合并/拾取系统 + 假世界快照。
fn setup(player_pos: Vec3) -> mcv_ecs::App {
    let mut app = mcv_ecs::App::new();
    // f32 累加 1/60 有舍入漂移，tick 率直接取引擎常数保证步数精确。
    app.schedule = mcv_ecs::Schedule::new().with_fixed_dt(mcv_game::consts::FIXED_DT);
    register_drop_components(&mut app.world);
    app.add_system(mcv_ecs::Stage::Fixed, "item_physics", item_physics_system);
    app.add_system(mcv_ecs::Stage::Fixed, "item_merge", item_merge_system);
    app.add_system(mcv_ecs::Stage::Fixed, "item_pickup", item_pickup_system);
    app.resources.insert(DropWorld {
        chunks: floor_world(),
        player_pos,
    });
    app
}

fn tick(app: &mut mcv_ecs::App, player_pos: Vec3) {
    // 复用既有快照的区块表（Arc 计数克隆，1 项 HashMap），仅换玩家位姿
    // ——寿命测试要跑 1.8 万步，每步重建假世界太浪费。
    let chunks = app
        .resources
        .get::<DropWorld>()
        .expect("setup 未注入 DropWorld")
        .chunks
        .clone();
    app.resources.insert(DropWorld { chunks, player_pos });
    // 与 GameRuntime::fixed_step 同款：手动跑 Fixed 阶段。App::update 会
    // 连跑 Variable 阶段并再次翻转事件通道，把 Fixed 期发出的 PickupReq
    // 吞掉（阶段边界 rotate 语义：事件只活一轮）。
    let mcv_ecs::App {
        world,
        resources,
        events,
        commands,
        schedule,
    } = app;
    let mut ctx = mcv_ecs::SysCtx {
        world,
        resources,
        events,
        commands,
    };
    schedule.run_stage(mcv_ecs::Stage::Fixed, &mut ctx);
}

/// 跑 n tick 并结算拾取（GameRuntime 固定步末同款流水线）。
fn tick_with_pickup(app: &mut mcv_ecs::App, hotbar: &mut mcv_item::Hotbar, n: u32) {
    for _ in 0..n {
        tick(app, Vec3::new(8.5, 64.0, 8.5));
        settle_pickups(&mut app.world, &mut app.events, hotbar, 0);
    }
}

#[test]
fn drop_settles_on_ground() {
    let mut app = setup(NO_PLAYER);
    let mut rng = lcg(7);
    let e = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, 70.0, 8.5),
        27,
        1,
        0,
        &mut rng,
    );
    for _ in 0..600 {
        tick(&mut app, NO_PLAYER);
    }
    let body = app.world.get_ref::<mcv_entity::PhysBody>(e).unwrap();
    assert!(body.on_ground, "600 步（含反弹收敛）后必已落地静止");
    assert!(
        (body.pos.y - 64.0).abs() < 1e-3,
        "落点贴地 y={}",
        body.pos.y
    );
    // 静止时贴地冲击 -0.53 m/s 低于反弹下限，落地清零后速度归零。
    assert!(body.vel.length() < 0.01, "静止 vel={:?}", body.vel);
    let d = app.world.get_ref::<ItemDrop>(e).unwrap();
    assert_eq!(d.age, 600, "600 步计龄正确且未到期（< DESPAWN_AGE）");
}

#[test]
fn drop_despawns_at_6000_ticks() {
    let mut app = setup(NO_PLAYER);
    let mut rng = lcg(7);
    let e = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, 65.0, 8.5),
        27,
        1,
        0,
        &mut rng,
    );
    // 寿命（6000 tick = 18000 步）前一刻仍存活。
    for _ in 0..DESPAWN_AGE - 1 {
        tick(&mut app, NO_PLAYER);
    }
    assert!(app.world.is_alive(e), "17999 步仍存活");
    tick(&mut app, NO_PLAYER);
    assert!(!app.world.is_alive(e), "18000 步（6000 tick）到期 despawn");
    assert_eq!(app.world.component_count::<ItemDrop>(), 0);
}

/// 掉进虚空（y < -64）当 tick 销毁（MC Entity 出界移除，不等 DESPAWN_AGE）。
#[test]
fn drop_despawns_below_void() {
    let mut app = setup(NO_PLAYER);
    let mut rng = lcg(17);
    let e = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, -70.0, 8.5),
        27,
        1,
        0,
        &mut rng,
    );
    tick(&mut app, NO_PLAYER);
    assert!(!app.world.is_alive(e), "y<-64 的掉落物当 tick 即销毁");
    assert_eq!(app.world.component_count::<ItemDrop>(), 0);
}

/// 初速确定性：同 seed 同初速（rng 由调用方驱动，测试可复现）。
#[test]
fn initial_vel_deterministic_per_seed() {
    let mut app = setup(NO_PLAYER);
    let mut r1 = lcg(42);
    let mut r2 = lcg(42);
    let a = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, 70.0, 8.5),
        27,
        1,
        10,
        &mut r1,
    );
    let b = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, 70.0, 8.5),
        27,
        1,
        10,
        &mut r2,
    );
    let va = app.world.get_ref::<mcv_entity::PhysBody>(a).unwrap().vel;
    let vb = app.world.get_ref::<mcv_entity::PhysBody>(b).unwrap().vel;
    assert_eq!(va, vb);
    assert!((va.y - 4.0).abs() < 1e-5, "vy=0.2 块/tick ×20 = 4 m/s");
    assert!(va.x.abs() <= 2.0 + 1e-5 && va.z.abs() <= 2.0 + 1e-5);
    // pickup_delay 原样入组件，系统逐步衰减。
    assert_eq!(app.world.get_ref::<ItemDrop>(a).unwrap().pickup_delay, 10);
    tick(&mut app, NO_PLAYER);
    assert_eq!(app.world.get_ref::<ItemDrop>(a).unwrap().pickup_delay, 9);
}

/// 非掉落实体（无 ItemDrop 组件）不被掉落系统触碰。
#[test]
fn physics_skips_non_drops() {
    let mut app = setup(NO_PLAYER);
    let mob = app.world.spawn();
    app.world.insert(
        mob,
        mcv_entity::PhysBody {
            pos: Vec3::new(8.5, 70.0, 8.5),
            vel: Vec3::ZERO,
            on_ground: false,
        },
    );
    for _ in 0..10 {
        tick(&mut app, NO_PLAYER);
    }
    assert_eq!(
        app.world
            .get_ref::<mcv_entity::PhysBody>(mob)
            .unwrap()
            .pos
            .y,
        70.0,
        "无 ItemDrop 的 PhysBody（怪物）不应被掉落系统步进"
    );
}

// ---------------------------------------------------------------------------
// 拾取 + 合并
// ---------------------------------------------------------------------------

/// 玩家贴身站立：pickup_delay（默认 10 tick = 30 步）归零后数步内入栏
/// 并 despawn。
#[test]
fn adjacent_player_collects_drop() {
    let mut app = setup(Vec3::new(8.5, 64.0, 8.5));
    let mut hotbar = mcv_item::Hotbar::empty();
    let mut rng = lcg(3);
    let e = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, 64.2, 8.5),
        27,
        3,
        mcv_entity::PICKUP_DELAY,
        &mut rng,
    );
    // 29 步（delay 未到 0）：不收。
    tick_with_pickup(&mut app, &mut hotbar, 29);
    assert!(app.world.is_alive(e), "pickup_delay 未到不拾取");
    assert!(hotbar.slots.iter().all(|s| s.is_empty()));
    // 再 5 步：delay 归零，贴身必被收走。
    tick_with_pickup(&mut app, &mut hotbar, 5);
    assert!(!app.world.is_alive(e), "delay 归零后贴身必拾取");
    let s = &hotbar.slots[0];
    assert_eq!((s.item, s.count), (27, 3));
}

/// 满栏（9×64 异种物品）：拾取被拒，实体存活且数量分毫不少。
#[test]
fn full_hotbar_keeps_drop_alive() {
    let mut app = setup(Vec3::new(8.5, 64.0, 8.5));
    // 全栏填满与掉落物（27 cobble）不同种的钻石（id 3，64 堆）。
    let mut hotbar = mcv_item::Hotbar::empty();
    for slot in hotbar.slots.iter_mut() {
        *slot = mcv_item::ItemStack::new(3, 64);
    }
    let mut rng = lcg(5);
    let e = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, 64.2, 8.5),
        27,
        5,
        0,
        &mut rng,
    );
    tick_with_pickup(&mut app, &mut hotbar, 30);
    assert!(app.world.is_alive(e), "满栏时掉落物必须留在地上");
    assert_eq!(
        app.world.get_ref::<ItemDrop>(e).unwrap().count,
        5,
        "拾取被拒不吞数量"
    );
    assert!(
        hotbar.slots.iter().all(|s| s.item == 3 && s.count == 64),
        "满栏不被改动"
    );
}

/// 合并守恒：总数量不变、单堆不超 max_stack、age 取 min（保年轻者）。
/// 三只用同 seed 独立 rng → 初速完全一致，同点位锁步漂移不散开。
#[test]
fn merge_conserves_count() {
    let mut app = setup(NO_PLAYER);
    // 同点三只 30 个圆石（max 64）→ 合并为 64 + 26。
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(spawn_item_drop(
            &mut app.world,
            Vec3::new(8.5, 64.0, 8.5),
            27,
            30,
            250,
            &mut lcg(11),
        ));
    }
    for _ in 0..101 {
        tick(&mut app, NO_PLAYER);
    }
    let mut total = 0u32;
    let mut max_age = 0u32;
    for &e in &ids {
        if let Some(d) = app.world.get_ref::<ItemDrop>(e) {
            total += d.count as u32;
            max_age = max_age.max(d.age);
            assert!(
                d.count <= mcv_item::Hotbar::max_stack(d.item),
                "合并后单堆超上限"
            );
        }
    }
    assert_eq!(total, 90, "合并守恒");
    // 被并入者 despawn：只剩两只（64+26）。
    assert_eq!(app.world.component_count::<ItemDrop>(), 2);
    // age 取 min：三只同龄，合并后仍为生成时年龄（随步数累加到 101）。
    assert_eq!(max_age, 101);
    // 再跑 200 步（delay 已过但玩家不在场）：不消失不增数。
    for _ in 0..200 {
        tick(&mut app, NO_PLAYER);
    }
    let t2: u32 = ids
        .iter()
        .filter_map(|&e| app.world.get_ref::<ItemDrop>(e))
        .map(|d| d.count as u32)
        .sum();
    assert_eq!(t2, 90);
}

/// 合并半径：≥ 0.5 距离不合。
#[test]
fn merge_respects_distance() {
    let mut app = setup(NO_PLAYER);
    let mut rng = lcg(13);
    let a = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, 64.0, 8.5),
        27,
        10,
        250,
        &mut rng,
    );
    // 1.3 格外：落地后地面阻尼（60 m/s²）数步内清零初速，漂移 <0.1/只，
    // 仍 > 合并水平半径 0.5。
    let b = spawn_item_drop(
        &mut app.world,
        Vec3::new(9.8, 64.0, 8.5),
        27,
        10,
        250,
        &mut rng,
    );
    for _ in 0..5 {
        tick(&mut app, NO_PLAYER);
    }
    assert_eq!(
        app.world.get_ref::<ItemDrop>(a).unwrap().count
            + app.world.get_ref::<ItemDrop>(b).unwrap().count,
        20
    );
    assert_eq!(app.world.component_count::<ItemDrop>(), 2, "1 格外不合");
    assert_eq!(app.world.get_ref::<ItemDrop>(a).unwrap().count, 10);
}
