//! mob 渲染拼装测试（纯 CPU）：Kind→MobInstance 映射、矩阵有限、裁剪语义。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_ecs::App;
use mcv_entity::{MobId, PassiveServices, spawn_passive_mob};

fn stone_chunk() -> HashMap<ChunkPos, Arc<ChunkHandle>> {
    let h = ChunkHandle::new(ChunkPos::new(0, 0));
    {
        let mut vox = h.voxels.write().unwrap();
        for ly in 0..64usize {
            for i in 0..256usize {
                vox[mcv_core::vidx(i & 15, ly as i32, i >> 4)] = BlockId(1);
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

#[test]
fn collect_instances_maps_kinds() {
    let chunks = stone_chunk();
    let mut app = App::new();
    mcv_entity::register_mob_components(&mut app.world);
    mcv_entity::register_passive_components(&mut app.world);
    app.resources.insert(PassiveServices {
        chunks: chunks.clone(),
        player_pos: Vec3::new(60.0, 64.0, 60.0),
        on_tick: true,
        seed: 9,
    });
    spawn_passive_mob(&mut app.world, MobId::CHICKEN, Vec3::new(4.0, 64.0, 4.0));
    spawn_passive_mob(&mut app.world, MobId::PIG, Vec3::new(12.0, 64.0, 12.0));
    // 敌对怪（Brain 路线，无 WanderState/AnimState）同样进 mob 渲染：
    // 收集器对缺组件实体走零相位静止姿态，不 panic（真实游戏世界即此形态）。
    let z = app.world.spawn();
    app.world
        .insert(z, mcv_entity::MobKind(mcv_entity::MobId::ZOMBIE));
    app.world.insert(
        z,
        mcv_entity::PhysBody {
            pos: Vec3::new(1.0, 64.0, 1.0),
            vel: Vec3::ZERO,
            on_ground: false,
        },
    );

    let mobs = mcv_logic::mob_render::collect_mob_instances(&app.world);
    assert_eq!(mobs.len(), 3, "被动 2 + 敌对 1");
    let kinds: Vec<u32> = mobs.iter().map(|m| m.kind).collect();
    assert!(kinds.contains(&0), "应有鸡实例");
    assert!(kinds.contains(&3), "应有猪实例");
    assert!(kinds.contains(&4), "应有僵尸实例（MobModelKind::Zombie=4）");
    for m in &mobs {
        for mm in &m.models {
            for col in mm {
                for v in col {
                    assert!(v.is_finite(), "矩阵必须有限");
                }
            }
        }
    }
}

#[test]
fn hostile_spawn_collects_two_instances_with_positions() {
    // 敌对通道端到端：spawn_mob（自然生成同款装配）塞 2 只 →
    // collect 出 2 实例、kind 与位置一一对应。
    let mut app = App::new();
    mcv_entity::register_mob_components(&mut app.world);
    mcv_entity::register_passive_components(&mut app.world);
    let a = Vec3::new(3.5, 65.0, -8.25);
    let b = Vec3::new(-12.0, 70.0, 40.0);
    mcv_entity::spawn_mob(&mut app.world, MobId::CREEPER, a);
    mcv_entity::spawn_mob(&mut app.world, MobId::SKELETON, b);

    let mobs = mcv_logic::mob_render::collect_mob_instances(&app.world);
    assert_eq!(mobs.len(), 2, "两只敌对怪应各出一个实例");
    let by_kind: HashMap<u32, Vec3> = mobs
        .iter()
        .map(|m| {
            // 取躯干（部位 1）平移分量作为实体位置（矩阵末列）。
            let t = m.models[1];
            (m.kind, Vec3::new(t[3][0], t[3][1], t[3][2]))
        })
        .collect();
    // MobModelKind::Creeper=6 / Skeleton=5；位置应精确等于出生点
    // （苦力怕躯干 pivot(0,6,0) → 平移 = pos + (0,18px,0)）。
    let ca = by_kind.get(&6).expect("应有苦力怕实例");
    assert!((ca.x - a.x).abs() < 1e-5 && (ca.z - a.z).abs() < 1e-5);
    assert!((ca.y - (a.y + 18.0 / 16.0)).abs() < 1e-5, "creeper {ca}");
    let sk = by_kind.get(&5).expect("应有骷髅实例");
    assert!((sk.x - b.x).abs() < 1e-5 && (sk.z - b.z).abs() < 1e-5);
}

#[test]
fn radius_clip_drops_far_mobs() {
    let chunks = stone_chunk();
    let mut app = App::new();
    mcv_entity::register_mob_components(&mut app.world);
    mcv_entity::register_passive_components(&mut app.world);
    app.resources.insert(PassiveServices {
        chunks,
        player_pos: Vec3::new(0.0, 64.0, 0.0),
        on_tick: true,
        seed: 1,
    });
    spawn_passive_mob(&mut app.world, MobId::SHEEP, Vec3::new(1.0, 64.0, 1.0));
    spawn_passive_mob(&mut app.world, MobId::COW, Vec3::new(50.0, 64.0, 50.0));
    // 观察眼与生物同层（y=64）：裁剪按 3D 距离，别把同层生物误判 64m 外。
    let near = mcv_logic::mob_render::collect_mob_instances_in(
        &app.world,
        Some((Vec3::new(0.0, 64.0, 0.0), 10.0)),
    );
    assert_eq!(near.len(), 1, "远处的牛应被剔除");
    let all = mcv_logic::mob_render::collect_mob_instances(&app.world);
    assert_eq!(all.len(), 2, "不裁剪时全量返回");
}
