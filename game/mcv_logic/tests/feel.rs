//! 战斗/手感域回归锁（fix/feel）：攻击冷却 tick 单位、无敌帧差值门、
//! exhaustion 增量表 + 饱和度、昼夜相位与 day_factor 曲线、攻击射线遮挡。
//! Java 依据均为 src-26.1（Minecraft 26.1 反编译树）。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_entity::combat::{attack_strength, cooldown_damage_scale, invulnerable_gate};
use mcv_game::Player;
use mcv_game::physics::{StepInput, step};
use mcv_item::ItemStack;
use mcv_logic::difficulty::Difficulty;
use mcv_logic::game::{
    ENTITY_ATTACK_RANGE, EXHAUSTION_MAX, MAX_AIR_SUPPLY, RAIN_PARTICLE_RADIUS, WorldView,
    air_supply_tick, block_hit_t, food_data_tick, move_exhaustion, pick_attack_target,
    rain_particle_count, ray_aabb_t, swimming_tick,
};
use mcv_render::{day_factor, sun_state};

// ---------------------------------------------------------------------------
// 攻击冷却（CRITICAL：秒/tick 错配）
// ---------------------------------------------------------------------------

#[test]
fn attack_cooldown_ticker_is_ticks_not_seconds() {
    let sword = ItemStack::new(mcv_item::IRON_SWORD_INDEX, 1);
    let speed = sword.attacks_per_second(); // 4.0 + (-2.4) = 1.6/s
    assert!((speed - 1.6).abs() < 1e-6);
    let dmg_scale = |t: f32| cooldown_damage_scale(attack_strength(t, speed));
    // 刚挥出（0 tick）→ 伤害 0.2x 下限。
    assert!((dmg_scale(0.0) - 0.2).abs() < 0.01, "0 tick → ≈0.2x");
    // 满蓄力 delay = 20/attackSpeed = 12.5 tick（Player.java:1793-1795，
    // getAttackStrengthScale :1803-1805 含 +0.5 tick 前置）→ 0.625s 平台。
    assert!((attack_strength(12.5, speed) - 1.0).abs() < 1e-6);
    assert!((dmg_scale(12.5) - 1.0).abs() < 1e-6);
    // 20 tick（=1s，ticker 上限）在平台上；按秒累加的老 bug 会让这里仍是 0.2x。
    assert!((dmg_scale(20.0) - 1.0).abs() < 1e-6, "20 tick=1s 满平台");
    // 中间点：铁剑 10 tick = (10+0.5)/12.5 = 0.84 强度；
    // 慢武器（attackSpeed 1.0，delay 20 tick）10 tick ≈ 0.5 强度点。
    assert!((attack_strength(10.0, 1.6) - 0.84).abs() < 1e-6);
    assert!((attack_strength(10.0, 1.0) - 0.525).abs() < 1e-6);
    // 单位锁死的物理意义：20 tick/s（tick_tests 已锁 accumulate_ticks），
    // 铁剑满蓄 = 12.5 tick = 0.625 s，而非按秒累加的 12.5 s。
    assert!((12.5f32 / 20.0 - 0.625).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// 无敌帧（CRITICAL：20 tick = 1s + 差值门）
// ---------------------------------------------------------------------------

#[test]
fn player_iframes_gate_matches_living_entity() {
    // 首击（invulnerable=0）全额。
    assert_eq!(invulnerable_gate(0, 0.0, 6.0), Some(6.0));
    // 命中后 invulnerable=20（LivingEntity.java:1206）。invulnerableTime>10 时
    // 伤害 ≤ lastHurt 整段忽略、更强只扣差值且不重置无敌帧
    // （LivingEntity.java:1196-1206）——玩家路径（hurt_player）与 mob 路径
    // （apply_hurt）共用本门。
    assert_eq!(invulnerable_gate(19, 6.0, 6.0), None, "等强伤害忽略");
    assert_eq!(invulnerable_gate(11, 6.0, 1.0), None, "更弱伤害忽略");
    assert_eq!(invulnerable_gate(11, 6.0, 10.0), Some(4.0), "只扣差值");
    // 源码实况：守卫窗口是 invulnerableTime>10（=命中后 10 tick），而非
    // 整个 20 tick 硬拒（派单文案「19 tick 内无效」是 1.8 前旧语义，弃用）。
    assert_eq!(
        invulnerable_gate(10, 6.0, 6.0),
        Some(6.0),
        "≤10 tick 全额结算"
    );
    // tick 单位递减（ServerPlayer.java:576-577 每 tick −1；本仓 on_tick 递减）：
    // 20 → 10 tick 后守卫解除，20 tick（1s）后计数归零。
    let mut inv = 20u32;
    for _ in 0..10 {
        inv = inv.saturating_sub(1);
    }
    assert_eq!(inv, 10);
    assert!(invulnerable_gate(inv, 6.0, 6.0).is_some());
    for _ in 0..10 {
        inv = inv.saturating_sub(1);
    }
    assert_eq!(inv, 0, "20 tick = 1s 无敌帧（非 0.167s/0.33s）");
}

// ---------------------------------------------------------------------------
// exhaustion 增量表 + 饱和度（CRITICAL：FoodData/FoodConstants）
// ---------------------------------------------------------------------------

#[test]
fn sprint_costs_exhaustion_per_meter_walk_costs_zero() {
    // 冲刺 0.1/m、走路 0.0/m（FoodConstants.java EXHAUSTION_SPRINT/EXHAUSTION_WALK，
    // ServerPlayer.checkMovementStatistics:1443-1456 只计水平位移）。
    assert!(
        (move_exhaustion(true, 1.0) - 0.1).abs() < 1e-6,
        "冲刺 1m → 0.1"
    );
    assert_eq!(move_exhaustion(false, 1.0), 0.0, "走路不增");
    assert_eq!(move_exhaustion(false, 99.0), 0.0);
}

#[test]
fn exhaustion_spends_saturation_before_hunger() {
    // 4.0 恰好不触发：FoodData.java:35 是 `> 4.0F` 严格大于。
    let (mut ex, mut sat, mut food, mut hp, mut timer) = (4.0f32, 5.0f32, 20.0f32, 20.0f32, 0u32);
    food_data_tick(
        &mut ex,
        &mut sat,
        &mut food,
        &mut hp,
        &mut timer,
        Difficulty::Normal,
    );
    assert!((ex - 4.0).abs() < 1e-6 && (sat - 5.0).abs() < 1e-6);
    assert!((food - 20.0).abs() < 1e-6, "满血满食不应有副作用");
    // >4：先扣 1 饱和，hunger 不动（FoodData.java:35-40）。
    let (mut ex, mut sat, mut food, mut hp, mut timer) = (4.05f32, 5.0f32, 20.0f32, 20.0f32, 0u32);
    food_data_tick(
        &mut ex,
        &mut sat,
        &mut food,
        &mut hp,
        &mut timer,
        Difficulty::Normal,
    );
    assert!((ex - 0.05).abs() < 1e-6);
    assert!((sat - 4.0).abs() < 1e-6, "饱和先扣");
    assert!((food - 20.0).abs() < 1e-6, "饱和未尽饥饿不扣");
    // 饱和见底才扣饥饿。
    let (mut ex, mut sat, mut food, mut hp, mut timer) = (4.05f32, 0.0f32, 20.0f32, 20.0f32, 0u32);
    food_data_tick(
        &mut ex,
        &mut sat,
        &mut food,
        &mut hp,
        &mut timer,
        Difficulty::Normal,
    );
    assert!((food - 19.0).abs() < 1e-6);
}

#[test]
fn slow_regen_costs_six_exhaustion_per_hp() {
    // 慢线：food≥18 且受伤，每 80 tick 回 1 HP、exhaustion+6
    //（FoodData.java:53-59 + FoodConstants.java:20 EXHAUSTION_HEAL=6.0）。
    let (mut ex, mut sat, mut food, mut hp, mut timer) = (0.0f32, 0.0f32, 18.0f32, 10.0f32, 0u32);
    for _ in 0..79 {
        food_data_tick(
            &mut ex,
            &mut sat,
            &mut food,
            &mut hp,
            &mut timer,
            Difficulty::Normal,
        );
    }
    assert!((hp - 10.0).abs() < 1e-6, "79 tick 未回血");
    food_data_tick(
        &mut ex,
        &mut sat,
        &mut food,
        &mut hp,
        &mut timer,
        Difficulty::Normal,
    );
    assert!((hp - 11.0).abs() < 1e-6, "第 80 tick 回 1");
    assert!((ex - 6.0).abs() < 1e-6, "回血代价 exhaustion+6");
}

#[test]
fn saturation_fast_regen_lane() {
    // 快线：饱和>0 且 food≥20 且受伤，每 10 tick 回 min(饱和,6)/6、
    // 代价 exhaustion+min(饱和,6)（FoodData.java:45-52）。
    let (mut ex, mut sat, mut food, mut hp, mut timer) = (0.0f32, 3.0f32, 20.0f32, 10.0f32, 0u32);
    for _ in 0..9 {
        food_data_tick(
            &mut ex,
            &mut sat,
            &mut food,
            &mut hp,
            &mut timer,
            Difficulty::Normal,
        );
    }
    assert!((hp - 10.0).abs() < 1e-6);
    food_data_tick(
        &mut ex,
        &mut sat,
        &mut food,
        &mut hp,
        &mut timer,
        Difficulty::Normal,
    );
    assert!((hp - 10.5).abs() < 1e-6, "回 min(3,6)/6 = 0.5");
    assert!(
        (ex - 3.0).abs() < 1e-6,
        "代价 exhaustion+3（饱和经 exhaustion 链回收）"
    );
}

#[test]
fn exhaustion_caps_at_40() {
    // FoodData.addExhaustion:100-101 上限 40——固定步累加路径同样封顶。
    assert_eq!(EXHAUSTION_MAX, 40.0);
    let mut ex = EXHAUSTION_MAX;
    ex = (ex + 0.1).min(EXHAUSTION_MAX);
    assert_eq!(ex, 40.0);
}

// ---------------------------------------------------------------------------
// 昼夜（CRITICAL：太阳相位 + day_factor 曲线）
// ---------------------------------------------------------------------------

#[test]
fn sun_zenith_at_tick_6000() {
    // day.json visual/sun_angle：tick 6000 = 0°（天顶）；0/12000 地平线、
    // 18000 天底。旧实现 frac*TAU 把天顶放 tick 0，早 1/4 天。
    let (d, _) = sun_state(6_000);
    assert!((d.y - 1.0).abs() < 1e-5, "tick 6000 太阳天顶，got {d}");
    let (d, _) = sun_state(0);
    assert!(d.y.abs() < 1e-5, "tick 0 黎明地平线");
    let (d, _) = sun_state(12_000);
    assert!(d.y.abs() < 1e-5, "tick 12000 日落地平线");
    let (d, _) = sun_state(18_000);
    assert!((d.y + 1.0).abs() < 1e-5, "tick 18000 子夜天底");
    let (_, day) = sun_state(6_000);
    assert!((day - 1.0).abs() < 1e-6);
    // 新世界从 tick 0 黎明起步（26.1 ClockInstance.totalTicks 默认 0，
    // ServerClockManager.java:149；day.json wake_up_from_sleep 标记 0）：
    // day_factor(0) 处于黎明线性段（0.24→1.0），既非正午也非深夜。
    assert!(
        (day_factor(0) - 0.24) > 0.01 && (day_factor(0) - 1.0) < -0.01,
        "黎明段 day_factor(0) 应介于平台之间, got {}",
        day_factor(0)
    );
}

#[test]
fn day_factor_plateaus_per_day_json() {
    // day.json visual/sky_light_factor 线性关键帧（非 cos 形）：白天平台 1.0、
    // 夜间平台 0.24（旧 cos 形夜底 0.03，夜面暗约 3 倍）。
    assert!(
        (day_factor(6_000) - 1.0).abs() < 1e-6,
        "tick 6000 附近 = 1.0"
    );
    assert!(
        (day_factor(18_000) - 0.24).abs() < 1e-6,
        "tick 18000 附近 = 0.24"
    );
    assert!((day_factor(730) - 1.0).abs() < 1e-6);
    assert!((day_factor(11_270) - 1.0).abs() < 1e-6);
    assert!((day_factor(13_140) - 0.24).abs() < 1e-6);
    assert!((day_factor(22_860) - 0.24).abs() < 1e-6);
    // 关键帧间线性：傍晚段中点 = (1.0+0.24)/2。
    assert!((day_factor(12_205) - 0.62).abs() < 1e-3);
    // 跨 0 回绕段连续：tick 729 ≈ 1.0（与 730 平台衔接）。
    assert!((day_factor(729) - 1.0).abs() < 1e-3);
    assert!((day_factor(22_861) - 0.24).abs() < 1e-2);
}

// ---------------------------------------------------------------------------
// 攻击射线遮挡（C2：ray-AABB + 方块 clip，实体距离 3.0）
// ---------------------------------------------------------------------------

fn air_chunk() -> Arc<ChunkHandle> {
    // 单空气区块（体素全 BlockId(0) air），stage 抬到 TerrainReady：
    // fix/stream-collision 后 WorldView 未加载/Empty 一律读空气（原版
    // VOID_AIR 语义，Level.java:361-363），抬 stage 只是保证这里读到的是
    // 本块**真实体素表**（本用例的石头墙必须可命中）而非就位前代理。
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(0, 0)));
    h.advance_to(Stage::TerrainReady);
    h
}

#[test]
fn wall_between_player_and_mob_blocks_attack() {
    let h = air_chunk();
    {
        // x=3 平面整面石头墙（体素索引 = ly<<8 | lz<<4 | lx）。
        let mut v = h.voxels.write().unwrap();
        for ly in 0..256usize {
            for lz in 0..16usize {
                v[mcv_core::vidx(3, ly as i32, lz)] = BlockId(1);
            }
        }
    }
    let mut chunks: HashMap<ChunkPos, Arc<ChunkHandle>> = HashMap::new();
    chunks.insert(ChunkPos::new(0, 0), h);
    let view = WorldView { chunks: &chunks };

    let eye = Vec3::new(1.5, 1.5, 1.5);
    let dir = Vec3::X;
    // 僵尸 AABB 尺寸（defs.half_size [0.3,0.95,0.3]），墙后 2.7m。
    let mobs = [(Vec3::new(4.5, 0.0, 1.5), [0.3f32, 0.95, 0.3])];

    let bt = block_hit_t(&view, eye, dir, ENTITY_ATTACK_RANGE).expect("wall must be hit");
    assert!((bt - 1.5).abs() < 1e-5, "墙面 x=3 → 距离 1.5");
    // 隔墙落空：实体命中 2.7m > 方块命中 1.5m。
    let direct = ray_aabb_t(eye, dir, Vec3::new(4.2, 0.0, 1.2), Vec3::new(4.8, 1.9, 1.8))
        .expect("ray crosses mob box");
    assert!((direct - 2.7).abs() < 1e-5);
    assert!(
        pick_attack_target(eye, dir, &mobs, Some(bt)).is_none(),
        "方块挡在玩家与 mob 之间 → try_attack 落空（旧锥形近似可隔墙打怪）"
    );
    // 无遮挡时命中（同一条射线）。
    assert_eq!(
        pick_attack_target(eye, dir, &mobs, None).map(|(i, _)| i),
        Some(0)
    );
    // 实体攻击距离 3.0（Player.java:133 DEFAULT_ENTITY_INTERACTION_RANGE，
    // 旧实现 3.5）：盒面近端 >3m 的怪落空。
    let far = [(Vec3::new(5.5, 0.0, 1.5), [0.3f32, 0.95, 0.3])];
    assert!(pick_attack_target(eye, dir, &far, None).is_none());
    // 打偏（瞄准上方）不命中。
    assert!(pick_attack_target(eye, Vec3::new(0.0, 1.0, 0.0), &mobs, None).is_none());
}

// ---------------------------------------------------------------------------
// 水与移动还原（fix/water-movement）：空气/溺水 tick 数、步高、潜行防跌落、
// 游泳状态机、雨粒子密度。Java 依据均为 src-26.1。
// ---------------------------------------------------------------------------

/// 测试用：按注册名查方块 id（与 engine tests/physics.rs::id_of 同款）。
fn id_of(name: &str) -> u16 {
    (0..mcv_core::BLOCKS.len() as u16)
        .find(|i| mcv_core::BLOCKS[*i as usize].name == name)
        .unwrap()
}

/// 单区块世界（x/z 0..15）：`floor_max_x` 列以内铺 y≤9 石头地表。
fn floor_chunk(floor_max_x: i32) -> Arc<ChunkHandle> {
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(0, 0)));
    {
        let mut v = h.voxels.write().unwrap();
        for x in 0..floor_max_x {
            for z in 0..16 {
                for y in 0..=9usize {
                    v[mcv_core::vidx(x as usize, y as i32, z)] = BlockId(1);
                }
            }
        }
    }
    h.advance_to(Stage::TerrainReady);
    h
}

fn walk_world(h: Arc<ChunkHandle>) -> HashMap<ChunkPos, Arc<ChunkHandle>> {
    let mut chunks: HashMap<ChunkPos, Arc<ChunkHandle>> = HashMap::new();
    chunks.insert(ChunkPos::new(0, 0), h);
    chunks
}

#[test]
fn drown_air_supply_tick_table() {
    // 满气 300（Entity.java:2739-2741 getMaxAirSupply）。
    assert_eq!(MAX_AIR_SUPPLY, 300);
    let mut air = MAX_AIR_SUPPLY;
    // 水下前 300 tick：只耗不伤（air 300→0，LivingEntity.java:424/565-575）。
    for t in 1..=300 {
        assert!(
            !air_supply_tick(&mut air, true, true),
            "tick {t} 不应触发伤害"
        );
    }
    assert_eq!(air, 0);
    // air 0 → −20 再 19 tick 无伤，第 20 tick 触发（shouldTakeDrowningDamage
    // `air ≤ −20`，LivingEntity.java:487-489 + 伤害 :425-428）。
    for t in 1..=19 {
        assert!(!air_supply_tick(&mut air, true, true), "尾段 tick {t}");
    }
    assert_eq!(air, -19);
    assert!(
        air_supply_tick(&mut air, true, true),
        "第 320 tick 溺水伤害"
    );
    assert_eq!(air, 0, "伤害 tick 空气清 0（:426）");
    // 之后每 20 tick 一伤。
    for _ in 0..19 {
        assert!(!air_supply_tick(&mut air, true, true));
    }
    assert!(
        air_supply_tick(&mut air, true, true),
        "第二发（每 20 tick）"
    );
    // 出水恢复 +4/tick（LivingEntity.java:577-579），上限 300。
    let mut air = 0;
    for _ in 0..74 {
        assert!(!air_supply_tick(&mut air, false, true));
    }
    assert_eq!(air, 296, "+4/tick");
    assert!(!air_supply_tick(&mut air, false, true));
    assert_eq!(air, 300, "封顶 300");
    for _ in 0..10 {
        assert!(!air_supply_tick(&mut air, false, true));
    }
    assert_eq!(air, 300, "不越上限");
    // 创造（abilities.invulnerable，LivingEntity.java:422-423）：水下
    // 不耗不伤也不回（原版水下回气只走药水分支 :430-432）。
    let mut air = MAX_AIR_SUPPLY;
    for _ in 0..400 {
        assert!(!air_supply_tick(&mut air, true, false), "创造免溺");
    }
    assert_eq!(air, MAX_AIR_SUPPLY);
}

#[test]
fn swimming_state_machine_matches_update_swimming() {
    // 维持：sprinting && isInWater（身体触水即可，Entity.java:1560）。
    assert!(swimming_tick(true, true, true, false, false, false));
    // 起步：sprinting && isUnderWater && 脚下格水（:1561-1563）。
    assert!(swimming_tick(false, true, true, true, true, false));
    assert!(
        !swimming_tick(false, true, true, false, true, false),
        "浅水（眼未没入）不起步"
    );
    assert!(
        !swimming_tick(false, true, true, true, false, false),
        "脚下非水格不起步"
    );
    assert!(
        !swimming_tick(false, false, true, true, true, false),
        "不按冲刺不起步"
    );
    // 松开冲刺 / 出水 → 退出。
    assert!(!swimming_tick(true, false, true, true, true, false));
    assert!(!swimming_tick(true, true, false, true, true, false));
    // 创造飞行恒 false（Player.java:1410-1416）。
    assert!(!swimming_tick(false, true, true, true, true, true));
    assert!(!swimming_tick(true, true, true, true, true, true));
}

#[test]
fn rain_particle_count_formula() {
    // WeatherEffectRenderer.java:232 count = (int)(0.225·(2r+1)²·level²)。
    assert_eq!(
        rain_particle_count(0.0, RAIN_PARTICLE_RADIUS),
        0,
        "无雨不生成"
    );
    assert_eq!(rain_particle_count(1.0, 10), 99, "0.225·441 = 99.225 → 99");
    assert_eq!(rain_particle_count(0.5, 10), 24, "0.225·441·0.25 → 24");
    assert_eq!(RAIN_PARTICLE_RADIUS, 10, "Options.java:178-184 默认 10");
    assert_eq!(rain_particle_count(1.0, 3), 11, "0.225·49 = 11.025 → 11");
}

#[test]
fn step_height_is_vanilla_06() {
    // Attributes.java:85-86 STEP_HEIGHT 默认 0.6（派单「我们 0.5」为误，
    // 本仓此前无步高机制——见 consts.rs 注释）。
    assert_eq!(mcv_game::consts::STEP_HEIGHT, 0.6);
}

#[test]
fn walk_up_half_slab_without_jumping() {
    // 自动上台阶（Entity.collide:1089-1106）：候选 = 脚底上方 ≤0.6 的
    // 碰撞面。下半砖 0.5 ≤ 0.6 → 平地走路（不按跳）应登上。
    let h = floor_chunk(16);
    {
        let slab = id_of("oak_slab");
        let mut v = h.voxels.write().unwrap();
        for z in 0..16usize {
            v[mcv_core::vidx(5, 10, z)] = BlockId(slab); // state 0 = 下半砖
        }
    }
    let chunks = walk_world(h);
    let view = WorldView { chunks: &chunks };
    let mut p = Player {
        pos: Vec3::new(3.0, 10.0, 8.0),
        ..Default::default()
    };
    let input = StepInput {
        wish_dir: Vec3::X,
        sprint: true,
        ..Default::default()
    };
    // 单列半砖走过去还会走下来，所以验「600 步内存在踏上砖面的一瞬」
    // （原版行为：踏上 → 走过 → 走下）。
    let mut climbed = false;
    for _ in 0..600 {
        step(&view, &mut p, &input);
        if p.pos.y > 10.45 && p.on_ground {
            climbed = true;
            break;
        }
    }
    assert!(climbed, "600 步内应不跳登上半砖（STEP_HEIGHT 0.6 ≥ 0.5）");
    assert!(
        p.pos.y > 10.45 && p.pos.y < 10.6,
        "半砖顶 10.5，got {}",
        p.pos.y
    );
    assert!(p.pos.x > 4.6, "登临点在半砖列前缘，got {}", p.pos.x);
}

#[test]
fn full_block_still_blocks_walking() {
    // 1.0 > STEP_HEIGHT 0.6 → 无候选高度，走路撞墙不登（对比跳上一格台阶
    // 既有测试 physics_calib::jump_onto_one_block）。
    let h = floor_chunk(16);
    {
        let mut v = h.voxels.write().unwrap();
        for z in 0..16usize {
            v[mcv_core::vidx(5, 10, z)] = BlockId(1); // 整块石头
        }
    }
    let chunks = walk_world(h);
    let view = WorldView { chunks: &chunks };
    let mut p = Player {
        pos: Vec3::new(3.0, 10.0, 8.0),
        ..Default::default()
    };
    let input = StepInput {
        wish_dir: Vec3::X,
        sprint: true,
        ..Default::default()
    };
    for _ in 0..600 {
        step(&view, &mut p, &input);
    }
    assert!(p.pos.y < 10.05, "不登整块，got {}", p.pos.y);
    assert!(p.pos.x < 4.8, "停在墙前，got {}", p.pos.x);
}

#[test]
fn sneak_edge_guard_blocks_falling() {
    // 潜行防跌落（Player.maybeBackOffFromEdge，Player.java:880-933）：
    // 地表止于 x=8（体素 0..=7），潜行走向边缘停在沿口、不坠；对照组
    // 不潜行则走出边缘下落。
    let mk = |sneak: bool| {
        let chunks = walk_world(floor_chunk(8));
        let view = WorldView { chunks: &chunks };
        let mut p = Player {
            pos: Vec3::new(5.0, 10.0, 8.0),
            ..Default::default()
        };
        let input = StepInput {
            wish_dir: Vec3::X,
            sneak,
            sprint: true,
            ..Default::default()
        };
        for _ in 0..600 {
            step(&view, &mut p, &input);
        }
        (p.pos.x, p.pos.y)
    };
    let (xs, ys) = mk(true);
    assert!(ys > 9.9, "潜行不掉下去，got y={ys}");
    assert!(xs > 7.0, "应走到近沿口，got x={xs}");
    // AABB 后缘不能完全离开支撑面（canFallAtLeast 盒内缩 1e-7）：
    // 中心 ≤ 8.3 − ε（半宽 0.3）。
    assert!(xs < 8.35, "停在沿口内，got x={xs}");
    let (xn, yn) = mk(false);
    assert!(yn < 9.5, "不潜行走出边缘应下坠，got y={yn}");
    assert!(xn > xs, "对照组走得更远");
}
