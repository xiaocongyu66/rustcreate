//! mob 运行时接线测试（审计 CRITICAL 的端到端断言，mcv_logic 侧）：
//! tick 单位（C-1）、消散（C-3）、Brain AI 接线（C-2：creeper 引爆 /
//! skeleton 弓战 / LOS）、燃烧扣血、箭矢命中。
//!
//! 不构造 GameRuntime（需要 GPU 注入）：直接手搓 `mcv_ecs::App` +
//! 一个空气区块，逐步驱动 `mob_ai_system` / `arrow_system`——系统与
//! GameRuntime 之间只有 `MobServices` 快照这一个耦合面。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_ecs::{App, SysCtx};
use mcv_entity::{MobArrow, MobId, MobTicks, spawn_mob};
use mcv_logic::game::{
    MobArrowHit, MobExplosionHit, MobMeleeHit, MobServices, arrow_system, mob_ai_system,
};

/// 单区块世界：x,z ∈ [0,16)，y<64 实心石、64 以上空气。
/// `lit=true` 时光照表 sky=15/block=0（露天白天代理）；否则全 0（室内代理）。
fn world_chunk(lit: bool) -> HashMap<ChunkPos, Arc<ChunkHandle>> {
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
    if lit {
        let mut light = h.light.write().unwrap();
        for v in light.iter_mut() {
            *v = 0xF0; // sky=15, block=0
        }
    }
    h.advance_to(Stage::Uploaded);
    let mut map = HashMap::new();
    map.insert(ChunkPos::new(0, 0), Arc::new(h));
    map
}

fn air_chunk_map() -> HashMap<ChunkPos, Arc<ChunkHandle>> {
    world_chunk(false)
}

fn harness() -> App {
    let mut app = App::new();
    mcv_entity::register_mob_components(&mut app.world);
    app.schedule
        .add(mcv_ecs::Stage::Fixed, "mob_ai", mob_ai_system);
    app.schedule
        .add(mcv_ecs::Stage::Fixed, "mob_arrows", arrow_system);
    app
}

/// 驱动一个固定步（`on_tick` 直接透传——tick 边界行为就是要断言的对象）。
fn step(
    app: &mut App,
    chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>,
    player: Vec3,
    on_tick: bool,
    monsters_burn: bool,
) {
    let App {
        world,
        resources,
        events,
        commands,
        schedule,
    } = app;
    resources.insert(MobServices {
        chunks: chunks.clone(),
        player_pos: player,
        on_tick,
        // 与 on_tick 同步的单步 tick 数（测试无 burst，等价 0/1）。
        ticks_step: on_tick as u32,
        game_ticks: 0,
        monsters_burn,
        // 空气/单区块测试不涉昼夜：darken=0（白天代理，与 monsters_burn
        // 入参语义一致）。
        sky_darken: 0,
    });
    let mut ctx = SysCtx {
        world,
        resources,
        events,
        commands,
    };
    schedule.run_stage(mcv_ecs::Stage::Fixed, &mut ctx);
}

fn drain_melee(app: &mut App) -> Vec<MobMeleeHit> {
    app.events.channel::<MobMeleeHit>().take()
}

// ---------------------------------------------------------------------------
// 1) tick 单位（审计 C-1）：近战重击间隔 20 tick，而非 20 固定步
// ---------------------------------------------------------------------------

#[test]
fn melee_restrike_takes_20_ticks_not_20_steps() {
    let mut app = harness();
    let chunks = air_chunk_map();
    let player = Vec3::new(8.0, 64.0, 8.0);
    spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(8.0, 64.0, 9.0));
    // 20 个**非 tick** 固定步（1/3 s）：tick 计时器不许走 → 无命中。
    for _ in 0..20 {
        step(&mut app, &chunks, player, false, false);
        assert!(
            drain_melee(&mut app).is_empty(),
            "非 tick 步不得推进攻击冷却"
        );
    }
    // 40 个 tick 步：命中节拍 20 tick（MeleeAttackGoal.java:136）——第 2 tick
    // 首次命中（第 1 tick 索敌转 Melee），第 22 tick 第二次。
    let mut hits = Vec::new();
    for t in 1..=40u32 {
        step(&mut app, &chunks, player, true, false);
        if !drain_melee(&mut app).is_empty() {
            hits.push(t);
        }
    }
    assert_eq!(hits.len(), 2, "40 tick 内恰好 2 击，实际 {:?} ", hits);
    assert_eq!(hits[1] - hits[0], 20, "两次命中相隔 20 tick");
}

// ---------------------------------------------------------------------------
// 2) 消散（审计 C-3）：>128 一格一步即移除
// ---------------------------------------------------------------------------

#[test]
fn far_mob_despawned_in_one_tick() {
    let mut app = harness();
    let chunks = air_chunk_map();
    spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(8.0, 64.0, 8.0));
    // 玩家 192 格外（>128²）：一个 tick 步即排队 despawn。
    step(&mut app, &chunks, Vec3::new(8.0, 64.0, 200.0), true, false);
    assert_eq!(app.world.component_count::<mcv_entity::MobKind>(), 0);
}

#[test]
fn near_mob_survives_despawn_band() {
    let mut app = harness();
    let chunks = air_chunk_map();
    spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(8.0, 64.0, 8.0));
    // 32..128 带内、idle 从 0 起：600 tick 内不得随机移除（idle>600 才有门）。
    for _ in 0..300 {
        step(&mut app, &chunks, Vec3::new(8.0, 64.0, 58.0), true, false);
    }
    assert_eq!(app.world.component_count::<mcv_entity::MobKind>(), 1);
    // 且 <32² 时 idle 账本被清零（Mob.java:673）：拉回近处后账本为 0。
    step(&mut app, &chunks, Vec3::new(8.0, 64.0, 8.5), true, false);
    let mut found = 0u64;
    for (_, tk) in app.world.read::<MobTicks>().iter() {
        found = tk.idle_ticks;
    }
    assert!(found <= 1, "<32² 每 tick 清零，实际 idle={found}");
}

// ---------------------------------------------------------------------------
// 3) creeper 引爆（审计 C-2）：30 tick 起爆 → 事件 + 本体移除
// ---------------------------------------------------------------------------

#[test]
fn creeper_detonates_after_30_ticks() {
    let mut app = harness();
    let chunks = air_chunk_map();
    let player = Vec3::new(8.0, 64.0, 8.0);
    spawn_mob(&mut app.world, MobId::CREEPER, Vec3::new(8.0, 64.0, 9.5));
    let mut detonated_at = None;
    for t in 1..=60u32 {
        step(&mut app, &chunks, player, true, false);
        let blasts: Vec<MobExplosionHit> = app.events.channel::<MobExplosionHit>().take();
        if !blasts.is_empty() {
            assert_eq!(blasts[0].radius, 3.0, "Creeper.java:56 explosionRadius");
            detonated_at = Some(t);
            break;
        }
    }
    let t = detonated_at.expect("贴脸 LOS 下 30 tick 引信必须起爆（Creeper.java:51）");
    // 第 1 tick 索敌进入 Swell，其后每 tick fuse+1（SwellGoal.java:44）→
    // 第 31~32 tick 引爆；不早于 30。
    assert!((30..=33).contains(&t), "fuse 30 tick，实际第 {t} tick 起爆");
    // 引爆后本体移除（Creeper.java:144-149 explode → discard）。
    step(&mut app, &chunks, player, true, false);
    assert_eq!(app.world.component_count::<mcv_entity::MobKind>(), 0);
}

// ---------------------------------------------------------------------------
// 4) skeleton 弓战（审计 C-2）：远距离有 LOS → 远程分支（箭矢实体），非贴脸近战
// ---------------------------------------------------------------------------

#[test]
fn skeleton_shoots_ranged_instead_of_melee() {
    let mut app = harness();
    let chunks = air_chunk_map();
    let player = Vec3::new(4.0, 64.0, 8.0);
    spawn_mob(&mut app.world, MobId::SKELETON, Vec3::new(13.0, 64.0, 8.0)); // dist 9 ∈ (7.5,15]
    let mut melee = 0usize;
    for _ in 0..5 {
        step(&mut app, &chunks, player, true, false);
        melee += drain_melee(&mut app).len();
    }
    assert_eq!(melee, 0, "拉弓区间不走近战门");
    assert!(
        app.world.component_count::<MobArrow>() > 0,
        "有 LOS 且 dist≤15 必须产出箭矢（RangedBowAttackGoal）"
    );
}

#[test]
fn arrow_hits_player_and_sends_event() {
    let mut app = harness();
    let chunks = air_chunk_map();
    let player = Vec3::new(8.0, 64.0, 8.0);
    // 直接放置一支朝玩家飞的箭（arrow_system 的命中结算单测）。
    let e = app.world.spawn();
    app.world.insert(
        e,
        MobArrow {
            pos: Vec3::new(8.0, 64.6, 10.0),
            vel: Vec3::new(0.0, 0.0, -2.0),
            ttl_ticks: 100,
            damage: 2.0,
        },
    );
    let mut got = 0.0f32;
    for _ in 0..5 {
        step(&mut app, &chunks, player, true, false);
        let hits: Vec<MobArrowHit> = app.events.channel::<MobArrowHit>().take();
        for h in hits {
            got = h.damage;
        }
    }
    assert_eq!(got, 2.0, "AbstractArrow.java:718 power×2.0");
    assert_eq!(app.world.component_count::<MobArrow>(), 0, "命中后移除");
}

#[test]
fn arrow_does_not_move_between_ticks() {
    // tick 语义：非 tick 固定步箭矢不推进（与 AI 同一 20 Hz 门）。
    let mut app = harness();
    let chunks = air_chunk_map();
    let e = app.world.spawn();
    app.world.insert(
        e,
        MobArrow {
            pos: Vec3::new(2.0, 200.0, 2.0),
            vel: Vec3::new(1.0, 0.0, 0.0),
            ttl_ticks: 100,
            damage: 2.0,
        },
    );
    for _ in 0..3 {
        step(&mut app, &chunks, Vec3::new(8.0, 64.0, 8.0), false, false);
    }
    let pos = app.world.read::<MobArrow>().get(e).expect("箭矢应存活").pos;
    assert_eq!(pos, Vec3::new(2.0, 200.0, 2.0), "非 tick 步不推进弹道");
}

// ---------------------------------------------------------------------------
// 5) 燃烧扣血（审计 C-2/N-4）：remainingFireTicks 每 20 tick 扣 1
// ---------------------------------------------------------------------------

#[test]
fn burning_damage_cadence_is_20_ticks() {
    let mut app = harness();
    let chunks = air_chunk_map();
    let player = Vec3::new(8.0, 64.0, 100.0); // 远但 <128：不消散
    let e = spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(8.0, 64.0, 60.0));
    app.world.write::<MobTicks>().get_mut(e).unwrap().fire_ticks = 160;
    let hp = |app: &App| app.world.read::<mcv_entity::Health>().get(e).unwrap().0;
    // Entity.java:538-540：160 % 20 == 0 → 第 1 个 tick 即扣 1。
    step(&mut app, &chunks, player, true, false);
    assert!(
        (hp(&app) - 19.0).abs() < 1e-3,
        "点燃首 tick 扣 1，实际 {}",
        hp(&app)
    );
    // 其后 19 tick 不再扣（20 tick 一次）。
    for _ in 0..19 {
        step(&mut app, &chunks, player, true, false);
    }
    assert!((hp(&app) - 19.0).abs() < 1e-3);
    step(&mut app, &chunks, player, true, false);
    assert!((hp(&app) - 18.0).abs() < 1e-3, "第 21 tick 再扣 1");
}

#[test]
fn daylight_burn_wires_through_brain() {
    // 白天 + 直晒（天光 15）+ sky_exposed：Brain 有概率点燃（概率式
    // Mob.java:498-505，固定 RNG 在 mcv_entity 层单测已断言公式）。这里
    // 断言**接线面**：白天直晒下多 tick 后最终 fire_ticks>0 或已被点燃
    // 扣过血；夜晚（monsters_burn=false）永不自燃。
    // 僵尸与玩家都必须落在唯一已加载区块（x/z<16）内：缺区块读回
    // 全黑（chunk_light 保守值），摆放区外会让本测试永远点不着。
    let player = Vec3::new(8.0, 64.0, 13.0);
    let mut burned = false;
    // 全局 fast_rand 无法注入：跑足量 tick 让 4% 概率门命中。
    let mut app = harness();
    // 天光 15 = 露天代理（Mob.java:504 canSeeSky(眼) 的判据源，
    // BlockAndLightGetter.java:18-20 sky≥15）。
    let chunks = world_chunk(true);
    spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(8.0, 64.0, 3.0));
    for _ in 0..400 {
        step(&mut app, &chunks, player, true, true);
        if app
            .world
            .read::<MobTicks>()
            .iter()
            .any(|(_, tk)| tk.fire_ticks > 0)
        {
            burned = true;
            break;
        }
    }
    assert!(
        burned,
        "白天直晒僵尸必须被 Brain 点燃（burn_in_daylight 接线）"
    );
    // 夜：800 tick 不点燃。
    let mut app = harness();
    spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(8.0, 64.0, 3.0));
    for _ in 0..800 {
        step(&mut app, &chunks, player, true, false);
        assert!(
            app.world
                .read::<MobTicks>()
                .iter()
                .all(|(_, tk)| tk.fire_ticks == 0),
            "MONSTERS_BURN=false 不得自燃"
        );
    }
}

#[test]
fn indoor_mob_never_ignites() {
    // 光照全 0（区块 light 默认 0=室内）：白天直晒门（sky==15 代理
    // canSeeSky）不成立 → 永不点燃。
    let mut app = harness();
    let chunks = air_chunk_map();
    let player = Vec3::new(8.0, 64.0, 13.0);
    spawn_mob(&mut app.world, MobId::SKELETON, Vec3::new(8.0, 64.0, 3.0));
    for _ in 0..800 {
        step(&mut app, &chunks, player, true, true);
        assert!(
            app.world
                .read::<MobTicks>()
                .iter()
                .all(|(_, tk)| tk.fire_ticks == 0),
            "低天光（室内）白天不得燃烧：canSeeSky 门必须参与判定"
        );
    }
}

// ---------------------------------------------------------------------------
// 6) LOS 截断：中间立一堵石墙 → 僵尸不索敌（不打、不追）
// ---------------------------------------------------------------------------

#[test]
fn wall_blocks_target_acquisition() {
    let mut app = harness();
    let chunks = air_chunk_map();
    {
        let h = chunks.get(&ChunkPos::new(0, 0)).unwrap();
        let mut vox = h.voxels.write().unwrap();
        // x=10 整面石墙（y 64..80），切断 (8.5→13.5) 视线。
        for y in 64..80usize {
            for z in 0..16usize {
                vox[(y << 8) | (z << 4) | 10] = BlockId(1);
            }
        }
    }
    let player = Vec3::new(8.5, 64.0, 8.0);
    spawn_mob(&mut app.world, MobId::ZOMBIE, Vec3::new(13.5, 64.0, 8.0));
    for _ in 0..40 {
        step(&mut app, &chunks, player, true, false);
        assert!(drain_melee(&mut app).is_empty(), "隔墙不得命中");
    }
    // 直接断言 LOS 门的语义（不用位置断言——游走方向由全局 RNG 决定，
    // 位置区间会 flaky）：初次索敌要求视线（TargetGoal canUse →
    // TargetGoal.java:57-71 的 seen 判定；Brain 等价物见 ai.rs 目标获取），
    // 隔墙 40 tick 内 Brain 必须从未获得过目标。
    for (_, brain) in app.world.read::<mcv_entity::MobBrain>().iter() {
        assert!(!brain.0.has_target, "隔墙不得索敌（LOS 门失效）");
    }
}
