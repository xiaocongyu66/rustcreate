//! 水生生物运行时无头测试（无 GPU）：固定种子驱动 `aquatic_ai_system` +
//! `mcv_game` AABB 物理。覆盖三绿验收面：
//! 1. dry-out 计时（WaterAnimal.handleAirSupply 300+20 窒息节拍 → 死亡掉落）；
//! 2. 变体映射（AquaticVariant 持久 + salmon 权重/缩放 + tropical pack 往返）；
//! 3. 掉落对拍（生鱼 ×1 + 骨粉 5%，variant 透传）。
//! 另锁水内随机游（RandomSwimmingGoal 等价）、避玩家（AvoidEntityGoal）、
//! 离水扑腾（AbstractFish.aiStep jump 通道）。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_ecs::{App, SysCtx};
use mcv_entity::aquatic::{self, AquaticBehavior, AquaticServices};
use mcv_entity::defs::MobKind;
use mcv_entity::{
    MobId, WaterState, aquatic_ai_system, death_drops_variant, register_aquatic_components,
    spawn_aquatic_mob,
};

/// 石地世界：x,z ∈ [0,16)，y<64 实心石，64 以上空气（干涸/扑腾用）。
fn stone_chunk() -> HashMap<ChunkPos, Arc<ChunkHandle>> {
    let h = ChunkHandle::new(ChunkPos::new(0, 0));
    {
        let mut vox = h.voxels.write().unwrap();
        for ly in 0..64usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    vox.set_flat(mcv_core::vidx(x, ly as i32, z), BlockId(1)); // stone
                }
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

/// 静水世界：y<60 石、60..64 水（海平面带内）、64+ 空气。
fn water_chunk() -> HashMap<ChunkPos, Arc<ChunkHandle>> {
    let h = ChunkHandle::new(ChunkPos::new(0, 0));
    {
        let mut vox = h.voxels.write().unwrap();
        for y in 0..64i32 {
            for z in 0..16usize {
                for x in 0..16usize {
                    let id = if y < 60 { BlockId(1) } else { BlockId(5) }; // stone / water
                    vox.set_flat(mcv_core::vidx(x, y, z), id);
                }
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
    // 掉落物组件（环境死亡掉落经 spawn_item_drop → ItemDrop 表）。
    mcv_entity::register_drop_components(&mut app.world);
    register_aquatic_components(&mut app.world);
    app.schedule
        .add(mcv_ecs::Stage::Fixed, "aquatic", aquatic_ai_system);
    app.resources.insert(AquaticServices {
        chunks: Arc::new(chunks.clone()),
        player_pos: player,
        on_tick: true,
        seed,
        // 单区块测试世界：模拟环 = 该区块全量（生产面由 GameRuntime 逐 chunk
        // is_entity_ticking 快照）。
        sim_cells: Arc::new([ChunkPos::new(0, 0)].into_iter().collect()),
    });
    app
}

/// 驱动一个固定步（1/60 s = 1 tick，on_tick 恒真——决策+物理一步到位）。
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

fn count_item_drops(app: &App, name: &str) -> usize {
    let id = mcv_item::item_by_name(name).unwrap();
    app.world
        .read::<mcv_entity::ItemDrop>()
        .iter()
        .filter(|(_, d)| d.item == id)
        .count()
}

fn lcg(mut s: u64) -> impl FnMut() -> u32 {
    move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
    }
}

// ---------------------------------------------------------------------------
// 1) dry-out 计时：离水鳕鱼 air 300 → 第 320 tick 首次 2.0 dmg（air ≤ −20
//    = LivingEntity.shouldTakeDrowningDamage 门），Health 3.0 → 第 340 tick
//    死亡 despawn 并掉 1 生鳕鱼（LootTable entity/cod.json）。
// ---------------------------------------------------------------------------

#[test]
fn dry_out_cadence_death_and_drop() {
    let chunks = stone_chunk();
    // 玩家 24 格外（>avoid 8²，<32² 清消散账 → 无随机消散干扰）。
    let mut app = harness(&chunks, Vec3::new(8.0, 64.0, 32.0), 20261017);
    let e = spawn_aquatic_mob(&mut app.world, MobId::COD, Vec3::new(8.0, 64.0, 8.0), 0);
    let mut first_hit = None;
    for t in 1..=340u32 {
        step(&mut app);
        let Some(hp) = app.world.read::<mcv_entity::Health>().get(e).map(|h| h.0) else {
            break; // 已 despawn（第二窒息跳）
        };
        if hp < 3.0 && first_hit.is_none() {
            first_hit = Some(t);
        }
        if t < 320 {
            assert_eq!(hp, 3.0, "tick {t}：空气账未耗尽不得掉血");
        }
        assert!(
            !app.world.read::<WaterState>().get(e).unwrap().in_water,
            "石地恒离水"
        );
    }
    assert_eq!(first_hit, Some(320), "首次窒息伤害 = 300 空气 + 20 宽限");
    assert_eq!(
        app.world.component_count::<mcv_entity::MobKind>(),
        0,
        "第二跳（340 tick）后 Health≤0 必须 despawn（ItemDrop 不持 MobKind）"
    );
    // 环境掉落：恰 1 生鳕鱼（bone_meal 5% 可能件不影响主件）。
    assert_eq!(count_item_drops(&app, "cod"), 1, "cod 环境死亡掉 1 生鳕鱼");
}

// ---------------------------------------------------------------------------
// 2) 入水回充：耗损 200 tick 后换水世界，air 必须回满 300 且不掉血。
// ---------------------------------------------------------------------------

#[test]
fn water_refills_air_ledger() {
    let land = stone_chunk();
    let mut app = harness(&land, Vec3::new(8.0, 64.0, 32.0), 777);
    let e = spawn_aquatic_mob(&mut app.world, MobId::SALMON, Vec3::new(8.0, 64.0, 8.0), 0);
    for _ in 0..200 {
        step(&mut app);
    }
    assert!(app.world.read::<WaterState>().get(e).unwrap().air < 300);
    // 换水世界（实体放海平面带内）。
    let water = water_chunk();
    let mut app_w = harness(&water, Vec3::new(8.0, 96.0, 32.0), 777);
    let e_w = spawn_aquatic_mob(
        &mut app_w.world,
        MobId::SALMON,
        Vec3::new(8.0, 63.0, 8.0),
        0,
    );
    for _ in 0..400 {
        step(&mut app_w);
        let waters = app_w.world.read::<WaterState>();
        let w = waters.get(e_w).unwrap();
        assert!(w.in_water, "浸没态判定");
        assert_eq!(w.air, 300, "入水即回满（handleAirSupply else 分支）");
    }
    assert_eq!(
        app_w.world.read::<mcv_entity::Health>().get(e_w).unwrap().0,
        3.0,
        "水中不结算窒息"
    );
}

// ---------------------------------------------------------------------------
// 3) 水内随机游 + 避玩家：固定种子 600 tick 必然起游且位移有粗界；玩家
//    8 格内必进 Flee（AvoidEntityGoal:93）。
// ---------------------------------------------------------------------------

#[test]
fn swim_wander_and_avoid_player() {
    let water = water_chunk();
    let mut app = harness(&water, Vec3::new(8.0, 96.0, 32.0), 42);
    spawn_aquatic_mob(
        &mut app.world,
        MobId::TROPICAL_FISH,
        Vec3::new(8.0, 63.0, 8.0),
        aquatic::tropical::DEFAULT,
    );
    let origin = Vec3::new(8.0, 63.0, 8.0);
    let mut moved = 0.0f32;
    let mut wandered = false;
    for _ in 0..600 {
        step(&mut app);
        let bodies = app.world.read::<mcv_entity::PhysBody>();
        let modes = app.world.read::<mcv_entity::AquaticMode>();
        for (e, b) in bodies.iter() {
            moved = moved.max((b.pos - origin).length());
            assert!(
                b.pos.y > 59.7 && b.pos.y < 64.0,
                "不得沉入石层/跳出水面（y={}）",
                b.pos.y
            );
            if let Some(m) = modes.get(e) {
                assert!(m.wander_ticks <= 100, "游走预算上界");
                if m.behavior == AquaticBehavior::Wander {
                    wandered = true;
                }
            }
        }
    }
    assert!(moved > 0.3, "固定种子 600 tick 必然起游（moved {moved}）");
    assert!(moved < 32.0, "水内随机游漂移粗界（moved {moved}）");
    assert!(wandered, "600 tick 内必然进入 Wander");
    // 避逃：玩家贴近到 ~1.5 格 → Flee。
    let mut app2 = harness(&water, Vec3::new(9.5, 63.8, 9.5), 13);
    let e2 = spawn_aquatic_mob(&mut app2.world, MobId::COD, Vec3::new(8.0, 63.0, 8.0), 0);
    let mut fled = false;
    for _ in 0..20 {
        step(&mut app2);
        if let Some(m) = app2.world.read::<mcv_entity::AquaticMode>().get(e2)
            && m.behavior == AquaticBehavior::Flee
        {
            fled = true;
        }
    }
    assert!(fled, "玩家 8 格内必触发 AvoidEntityGoal 等价 Flee");
}

// ---------------------------------------------------------------------------
// 4) 离水扑腾：石地鳕鱼持续 jump 意图（AbstractFish.aiStep +0.4 竖直 ⟶
//    引擎 jump 通道），不离地、水平抖动有界。
// ---------------------------------------------------------------------------

#[test]
fn fish_flops_when_stranded() {
    let chunks = stone_chunk();
    let mut app = harness(&chunks, Vec3::new(8.0, 64.0, 32.0), 7);
    let e = spawn_aquatic_mob(
        &mut app.world,
        MobId::TROPICAL_FISH,
        Vec3::new(8.0, 64.0, 8.0),
        0,
    );
    let origin = Vec3::new(8.0, 64.0, 8.0);
    let mut hops = 0;
    // 跳跃滞空 ≈ 2×8.4/32 s ≈ 32 物理步 ≈ 11 tick → 600 tick ≈ 19±跳。
    for _ in 0..600 {
        step(&mut app);
        let bodies = app.world.read::<mcv_entity::PhysBody>();
        let intents = app.world.read::<mcv_entity::MobIntent>();
        let b = bodies.get(e).unwrap();
        let i = intents.get(e).unwrap();
        if i.jump {
            hops += 1;
        }
        assert!(b.pos.y >= 63.9, "扑腾不离地（y={}）", b.pos.y);
        assert!(
            (b.pos - origin).length() < 32.0,
            "扑腾水平随机游走粗界（d={}）",
            (b.pos - origin).length()
        );
    }
    assert!(hops >= 5, "离水扑腾按 jump 通道每次落地前重掷（hops {hops}）");
}

// ---------------------------------------------------------------------------
// 5) 掉落对拍：cod/salmon/tropical_fish = 生鱼 ×1 + 5% 骨粉（LootTable 经典
//    面）；tropical variant 全额透传；axolotl 恒空。
// ---------------------------------------------------------------------------

#[test]
fn drops_parity() {
    let pos = Vec3::ZERO;
    let mut bone_meal_seen = false;
    for seed in 1..300u64 {
        let mut rng = lcg(seed);
        let cod = death_drops_variant(MobKind::Cod, 0, false, pos, &mut rng);
        assert_eq!(cod[0].item, "cod");
        assert_eq!(cod[0].count, 1);
        assert_eq!(cod[0].variant, 0);
        assert!(cod.len() <= 2);
        let mut rng = lcg(seed);
        let sal = death_drops_variant(MobKind::Salmon, 0, false, pos, &mut rng);
        assert_eq!(sal[0].item, "salmon");
        assert_eq!(sal[0].count, 1);
        assert_eq!(sal[0].variant, 0);
        // tropical：packed variant 全额透传。
        let packed = aquatic::tropical::pack(
            3,
            true,
            aquatic::tropical::DYE_CYAN,
            aquatic::tropical::DYE_PINK,
        );
        let mut rng = lcg(seed);
        let tf = death_drops_variant(MobKind::TropicalFish, packed, false, pos, &mut rng);
        assert_eq!(tf[0].item, "tropical_fish");
        assert_eq!(tf[0].variant, packed, "掉落必须携带 packed variant");
        // 骨粉（5%）：至多 1 份且不与主件同槽。
        if cod.len() > 1 {
            assert_eq!(cod[1].item, "bone_meal");
            assert_eq!(cod[1].count, 1);
            bone_meal_seen = true;
        }
        // 美西螈：无 loot 表（vanilla axolotl.json 缺失面）。
        let mut rng = lcg(seed);
        let ax = death_drops_variant(MobKind::Axolotl, 0, false, pos, &mut rng);
        assert!(ax.is_empty(), "美西螈掉落恒空");
    }
    assert!(bone_meal_seen, "300 种子至少掷中一次 5% 骨粉");
}

// ---------------------------------------------------------------------------
// 6) 变体映射（runtime 侧）：salmon 权重边界 + 美西螈 common 池 + tropical
//    pack 往返 + AquaticVariant 持久。
// ---------------------------------------------------------------------------

#[test]
fn variant_tables_runtime() {
    // salmon 0..30 → SMALL / 30..80 → MEDIUM / 80..95 → LARGE（:129-136）。
    let seq = |start: u32| {
        let mut i = start;
        move || {
            let v = i;
            i = i.wrapping_add(1);
            v
        }
    };
    assert_eq!(
        aquatic::salmon::finalize_variant(&mut seq(0)),
        aquatic::salmon::SMALL
    );
    assert_eq!(
        aquatic::salmon::finalize_variant(&mut seq(30)),
        aquatic::salmon::MEDIUM
    );
    assert_eq!(
        aquatic::salmon::finalize_variant(&mut seq(80)),
        aquatic::salmon::LARGE
    );
    assert_eq!(aquatic::salmon::scale(aquatic::salmon::SMALL), 0.5);
    assert_eq!(aquatic::salmon::scale(aquatic::salmon::LARGE), 1.5);
    // 美西螈：spawn 只出 common 四型（BLUE 稀有只走繁殖门）。
    let mut rng = lcg(9);
    let mut seen = HashSet::new();
    for _ in 0..50 {
        let v = aquatic::axolotl::spawn_variant(&mut rng);
        assert!(v <= aquatic::axolotl::CYAN);
        seen.insert(v);
    }
    assert!(seen.len() >= 2, "均匀池 50 抽至少出 2 型");
    // 热带鱼 pack/unpack 往返（TropicalFish.java:59-72）。
    let packed = aquatic::tropical::pack(
        5,
        false,
        aquatic::tropical::DYE_PINK,
        aquatic::tropical::DYE_LIME,
    );
    assert_eq!(aquatic::tropical::unpack_pattern(packed), (false, 5));
    assert_eq!(
        aquatic::tropical::base_color(packed),
        aquatic::tropical::DYE_PINK
    );
    assert_eq!(
        aquatic::tropical::pattern_color(packed),
        aquatic::tropical::DYE_LIME
    );
    assert_eq!(aquatic::tropical::COMMON.len(), 22);
    assert_eq!(aquatic::tropical::COMMON_PACKED.len(), 22);
    // spawn 装配后变体持久。
    let chunks = water_chunk();
    let mut app = harness(&chunks, Vec3::new(8.0, 96.0, 32.0), 3);
    let e = spawn_aquatic_mob(
        &mut app.world,
        MobId::TROPICAL_FISH,
        Vec3::new(8.0, 63.0, 8.0),
        packed,
    );
    assert_eq!(
        app.world
            .read::<mcv_entity::AquaticVariant>()
            .get(e)
            .unwrap()
            .0,
        packed
    );
}
