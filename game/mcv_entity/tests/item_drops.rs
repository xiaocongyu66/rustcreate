//! 掉落物实体：物理沉降 / 寿命消失 / 确定性初速。系统直接驱动
//! （同 mcv_game 物理测试模式），地面用单 ChunkHandle 假世界。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_entity::items::{
    DESPAWN_AGE, DropWorld, ItemDrop, item_physics_system, register_drop_components,
    spawn_item_drop,
};

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

/// 建 App：注册组件 + 掉落物理系统 + 假世界快照。
fn setup(chunks: HashMap<ChunkPos, Arc<ChunkHandle>>) -> mcv_ecs::App {
    let mut app = mcv_ecs::App::new();
    // f32 累加 1/60 有舍入漂移，tick 率直接取引擎常数保证步数精确。
    app.schedule = mcv_ecs::Schedule::with_fixed_dt(mcv_game::consts::FIXED_DT);
    register_drop_components(&mut app.world);
    app.add_system(mcv_ecs::Stage::Fixed, "item_physics", item_physics_system);
    app.resources.insert(DropWorld { chunks });
    app
}

#[test]
fn drop_settles_on_ground() {
    let mut app = setup(floor_world());
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
        app.resources.insert(DropWorld {
            chunks: floor_world(),
        });
        app.update(mcv_game::consts::FIXED_DT);
    }
    let body = app.world.get_ref::<mcv_entity::PhysBody>(e).unwrap();
    assert!(body.on_ground, "600 tick 后必已落地");
    assert!(
        (body.pos.y - 64.0).abs() < 1e-3,
        "落点贴地 y={}",
        body.pos.y
    );
    assert!(body.vel.length() < 0.01, "静止 vel={:?}", body.vel);
    let d = app.world.get_ref::<ItemDrop>(e).unwrap();
    assert_eq!(d.age, 600, "600 tick 计龄正确且未到期（< DESPAWN_AGE）");
}

#[test]
fn drop_despawns_at_6000_ticks() {
    let mut app = setup(floor_world());
    let mut rng = lcg(7);
    let e = spawn_item_drop(
        &mut app.world,
        Vec3::new(8.5, 65.0, 8.5),
        27,
        1,
        0,
        &mut rng,
    );
    // 6000 tick 前一刻仍存活。
    for _ in 0..DESPAWN_AGE - 1 {
        app.resources.insert(DropWorld {
            chunks: floor_world(),
        });
        app.update(mcv_game::consts::FIXED_DT);
    }
    assert!(app.world.is_alive(e), "5999 tick 仍存活");
    app.resources.insert(DropWorld {
        chunks: floor_world(),
    });
    app.update(mcv_game::consts::FIXED_DT);
    assert!(!app.world.is_alive(e), "6000 tick 到期 despawn");
    assert_eq!(app.world.component_count::<ItemDrop>(), 0);
}

/// 初速确定性：同 seed 同初速（fast_rand 链由调用方驱动，测试可复现）。
#[test]
fn initial_vel_deterministic_per_seed() {
    let mut app = setup(floor_world());
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
    let (va, vb) = (
        app.world.get_ref::<mcv_entity::PhysBody>(a).unwrap().vel,
        app.world.get_ref::<mcv_entity::PhysBody>(b).unwrap().vel,
    );
    assert_eq!(va, vb);
    assert!((va.y - 4.0).abs() < 1e-5, "vy=0.2 块/tick ×20 = 4 m/s");
    assert!(va.x.abs() <= 1.4 + 1e-5 && va.z.abs() <= 1.4 + 1e-5);
    // pickup_delay 原样入组件，系统逐步衰减。
    assert_eq!(app.world.get_ref::<ItemDrop>(a).unwrap().pickup_delay, 10);
    app.resources.insert(DropWorld {
        chunks: floor_world(),
    });
    app.update(mcv_game::consts::FIXED_DT);
    assert_eq!(app.world.get_ref::<ItemDrop>(a).unwrap().pickup_delay, 9);
}

/// 非掉落实体（无 ItemDrop 组件）不被本系统触碰。
#[test]
fn physics_skips_non_drops() {
    let mut app = setup(floor_world());
    let mob = app.world.spawn();
    app.world.insert(
        mob,
        mcv_entity::PhysBody {
            pos: Vec3::new(8.5, 70.0, 8.5),
            vel: Vec3::ZERO,
            on_ground: false,
        },
    );
    app.resources.insert(DropWorld {
        chunks: floor_world(),
    });
    for _ in 0..10 {
        app.update(mcv_game::consts::FIXED_DT);
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
