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
                vox[(ly << 8) | i] = BlockId(1);
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
    // 僵尸走敌对 Brain 路线，不持有 WanderState → 不进 mob 渲染。
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
    assert_eq!(mobs.len(), 2, "僵尸不参与 mob 渲染");
    let kinds: Vec<u32> = mobs.iter().map(|m| m.kind).collect();
    assert!(kinds.contains(&0), "应有鸡实例");
    assert!(kinds.contains(&3), "应有猪实例");
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
    let near =
        mcv_logic::mob_render::collect_mob_instances_in(&app.world, Some((Vec3::ZERO, 10.0)));
    assert_eq!(near.len(), 1, "远处的牛应被剔除");
    let all = mcv_logic::mob_render::collect_mob_instances(&app.world);
    assert_eq!(all.len(), 2, "不裁剪时全量返回");
}
