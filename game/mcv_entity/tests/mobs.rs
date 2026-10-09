//! 敌对生物参数 / 状态机 / 刷怪 / 掉落表驱动测试。
//! 参照值全部来自 mc-ref/NOTES-mobs.md（MC 26.1 反编译行号）。

use glam::Vec3;
use mcv_core::BlockPos;
use mcv_entity::ai::*;
use mcv_entity::combat::knockback_velocity;
use mcv_entity::defs::*;
use mcv_entity::drops::death_drops;
use mcv_entity::spawner::*;

fn lcg(seed: u64) -> impl FnMut() -> u32 {
    let mut s = seed | 1;
    move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
    }
}

fn make_seq(vals: Vec<u32>) -> impl FnMut() -> u32 {
    let mut i = 0usize;
    move || {
        let v = vals[i.min(vals.len() - 1)];
        i += 1;
        v
    }
}

fn percept(pos: Vec3, target: Option<Vec3>) -> Percept {
    Percept {
        pos,
        target,
        target_half_width: 0.3,
        los: true,
        br: 0.0,
        day: false,
        sky_exposed: false,
        in_water: false,
        head_armor: false,
        has_bow: true,
        hurt: false,
        blocked: false,
        avoid: None,
        shelter: None,
        difficulty: 2, // normal
        waypoint: None,
    }
}

// ---------------------------------------------------------------------------
// 1) 四怪参数表（对照 NOTES §1-§4）
// ---------------------------------------------------------------------------

#[test]
fn hostile_stat_table_matches_26_1() {
    // (id, 生命, 移速 attr, 索敌 FOLLOW, 近战伤害, 出处)
    let rows: [(MobId, f32, f32, f32, f32); 4] = [
        // Zombie.java:130-137
        (MobId::ZOMBIE, 20.0, 0.23, 35.0, 3.0),
        // AbstractSkeleton.java:89-91 + Mob.java:162（基础 FOLLOW 16）
        (MobId::SKELETON, 20.0, 0.25, 16.0, 2.0),
        // Creeper.java:77-79（近战无伤害 Creeper.java:182）
        (MobId::CREEPER, 20.0, 0.25, 16.0, 0.0),
        // Spider.java:88-90
        (MobId::SPIDER, 16.0, 0.3, 16.0, 2.0),
    ];
    for (id, hp, speed, follow, dmg) in rows {
        let d = id.def();
        assert_eq!(d.health, hp, "{} health", d.name);
        assert!((d.speed_attr - speed).abs() < 1e-6, "{} speed", d.name);
        assert!((d.follow_range - follow).abs() < 1e-6, "{} follow", d.name);
        assert!((d.attack_damage - dmg).abs() < 1e-6, "{} attack", d.name);
        assert!(d.hostile, "{} hostile", d.name);
        assert_eq!(d.xp, 5, "{} xp (Monster.java:34)", d.name);
    }
    // 线性标定：玩家 0.1 = 4.317 m/s。
    assert!((speed_m_s(0.1) - 4.317).abs() < 1e-3);
}

#[test]
fn ai_constants_match_reference() {
    assert_eq!(MELEE_ATTACK_INTERVAL, 20); // MeleeAttackGoal.java:21
    assert_eq!(LOSE_SIGHT_TICKS, 60); // TargetGoal.java:24
    assert_eq!(WANDER_INTERVAL_AVG, 120); // RandomStrollGoal.java:16
    assert!((WANDER_RADIUS - 10.0).abs() < 1e-6); // DefaultRandomPos(10,7)
    assert!((MELEE_REACH_BASE - (2.04_f32.sqrt() - 0.6)).abs() < 1e-3); // Mob.java:112
    assert!((BOW_RANGE - 15.0).abs() < 1e-6); // AbstractSkeleton.java:55
    assert_eq!(BOW_INTERVAL_NORMAL + BOW_DRAW_TICKS, 60); // normal 周期
    assert_eq!(BOW_INTERVAL_HARD + BOW_DRAW_TICKS, 40); // hard 周期
    assert!((ARROW_SPEED - 1.6).abs() < 1e-6); // AbstractSkeleton.java:170
    assert_eq!(arrow_spread(2), 6.0); // 14 − 2×4（normal）
    assert_eq!(arrow_spread(3), 2.0);
    assert_eq!(SWELL_TICKS, 30); // Creeper maxSwell = 1.5s
    assert!((SWELL_START_DIST - 3.0).abs() < 1e-6);
    assert!((SWELL_BREAK_DIST - 7.0).abs() < 1e-6);
    assert!((EXPLOSION_RADIUS - 3.0).abs() < 1e-6);
    assert_eq!(BURN_TICKS, 160); // igniteForSeconds(8)
    // 风筝阈值 = sqrt(225×0.75) / sqrt(225×0.25)
    assert!((KITE_STOP_DIST - (225.0f32 * 0.75).sqrt()).abs() < 0.01);
    assert!((KITE_RETREAT_DIST - (225.0f32 * 0.25).sqrt()).abs() < 1e-6);
    // AI 表
    assert!(ai_table(MobKind::Zombie).burn_sun);
    assert!(ai_table(MobKind::Skeleton).burn_sun);
    assert!(!ai_table(MobKind::Creeper).burn_sun);
    assert!(!ai_table(MobKind::Spider).burn_sun);
    assert!(ai_table(MobKind::Spider).wall_climb);
    assert!(ai_table(MobKind::Spider).leap);
    assert_eq!(ai_table(MobKind::Zombie).stroll_speed, 1.0);
    assert_eq!(ai_table(MobKind::Creeper).stroll_speed, 0.8);
    // 刷怪常数
    assert_eq!(MIN_PLAYER_DIST_SQR, 576.0); // 24²
    assert!((MAX_SPAWN_DIST - 128.0).abs() < 1e-6);
    assert_eq!(MAGIC_NUMBER, 289); // 17²
    assert_eq!(global_cap_total(289, 70), 70);
    assert_eq!(global_cap_total(288, 70), 69);
}

// ---------------------------------------------------------------------------
// 2) 索敌 / 丢目标 / 近战间隔（zombie）
// ---------------------------------------------------------------------------

#[test]
fn zombie_seek_and_lose_target() {
    let def = MobId::ZOMBIE.def();
    let mut brain = Brain::new();
    let mut rng = || 121u32; // 121%120=1：不触发游走；121%32=25
    // 30 格 + 视线 → 索敌（≤35）。
    let p = percept(Vec3::ZERO, Some(Vec3::new(30.0, 0.0, 0.0)));
    brain.tick(def, &p, &mut rng);
    assert!(brain.has_target && matches!(brain.state, MobState::Seek));
    // 拉到 40 格 → 超出 FOLLOW → 丢。
    let p = percept(Vec3::ZERO, Some(Vec3::new(40.0, 0.0, 0.0)));
    brain.tick(def, &p, &mut rng);
    assert!(!brain.has_target, "40 > follow 35 → lost");
    // 断视线：恰好在第 61 个无视线 tick 丢失（>60）。
    let mut brain = Brain::new();
    let near = percept(Vec3::ZERO, Some(Vec3::new(10.0, 0.0, 0.0)));
    brain.tick(def, &near, &mut rng);
    assert!(brain.has_target);
    let mut blind = near;
    blind.los = false;
    for i in 1..=60 {
        brain.tick(def, &blind, &mut rng);
        assert!(brain.has_target, "unseen {i} ≤ 60 保持");
    }
    brain.tick(def, &blind, &mut rng); // unseen = 61
    assert!(!brain.has_target, "unseen 61 > 60 → lost");
    // 受击反击无需视线（HurtByTargetGoal）。
    let mut brain = Brain::new();
    let mut hurt = percept(Vec3::ZERO, Some(Vec3::new(20.0, 0.0, 0.0)));
    hurt.los = false;
    hurt.hurt = true;
    brain.tick(def, &hurt, &mut rng);
    assert!(
        brain.has_target && matches!(brain.state, MobState::Seek),
        "hurt→seek"
    );
    // 无目标无伤：不索敌。
    let mut brain = Brain::new();
    let mut p = percept(Vec3::ZERO, Some(Vec3::new(20.0, 0.0, 0.0)));
    p.los = false;
    brain.tick(def, &p, &mut rng);
    assert!(!brain.has_target);
}

#[test]
fn zombie_melee_interval_20t() {
    let def = MobId::ZOMBIE.def();
    let mut brain = Brain::new();
    let mut rng = || 121u32;
    let p = percept(Vec3::ZERO, Some(Vec3::new(1.0, 0.0, 0.0)));
    let mut hits = Vec::new();
    for t in 1..=45 {
        let acts = brain.tick(def, &p, &mut rng);
        for a in acts {
            if let AiAction::MeleeHit { damage } = a {
                assert!((damage - 3.0).abs() < 1e-6); // Zombie.java:134
                hits.push(t);
            }
        }
    }
    // 进入 Melee 后第一击，此后每 20 tick 一击。
    assert_eq!(hits, vec![2, 22, 42], "melee interval = 20 tick");
}

// ---------------------------------------------------------------------------
// 3) skeleton：射程 / 周期 / 风筝
// ---------------------------------------------------------------------------

#[test]
fn skeleton_shoot_cycle_and_kite() {
    let def = MobId::SKELETON.def();
    let mut rng = || 121u32;
    // 周期：普通难度 40+20=60 tick 一箭；hard 40 tick。
    for (diff, period) in [(2u8, 60u32), (3u8, 40u32)] {
        let mut brain = Brain::new();
        let mut p = percept(Vec3::ZERO, Some(Vec3::new(10.0, 0.0, 0.0)));
        p.difficulty = diff;
        let mut shots = Vec::new();
        for t in 1..=(period * 2) {
            let acts = brain.tick(def, &p, &mut rng);
            for a in acts {
                if let AiAction::Shoot {
                    speed,
                    base_damage,
                    spread,
                    ..
                } = a
                {
                    assert!((speed - 1.6).abs() < 1e-6);
                    assert!((base_damage - 2.0).abs() < 1e-6);
                    assert!((spread - (14.0 - diff as f32 * 4.0)).abs() < 1e-6);
                    shots.push(t);
                }
            }
        }
        assert_eq!(shots[1] - shots[0], period, "difficulty {diff} period");
    }
    // 横移：los_ticks≥20 且 dist≤13 → Walk speed_mult=0.5。
    let mut brain = Brain::new();
    let p = percept(Vec3::ZERO, Some(Vec3::new(12.0, 0.0, 0.0)));
    brain.tick(def, &p, &mut rng);
    brain.los_ticks = 19;
    let acts = brain.tick(def, &p, &mut rng);
    assert!(
        acts.iter().any(|a| matches!(a, AiAction::Walk { speed_mult: m, dir, .. } if (*m - 0.5).abs() < 1e-6 && dir.length_squared() > 0.5)),
        "strafe expected: {acts:?}"
    );
    // 距离 <7.5 → 后撤分量（dir 与远离方向同向）。
    let mut brain = Brain::new();
    let p = percept(Vec3::ZERO, Some(Vec3::new(6.0, 0.0, 0.0)));
    brain.tick(def, &p, &mut rng);
    brain.los_ticks = 25;
    let acts = brain.tick(def, &p, &mut rng);
    let away = Vec3::new(-1.0, 0.0, 0.0);
    assert!(
        acts.iter()
            .any(|a| matches!(a, AiAction::Walk { dir, .. } if dir.dot(away) > 0.5)),
        "retreat component expected: {acts:?}"
    );
    // 超射程 13 → 直线接近（1.0×）。
    let mut brain = Brain::new();
    let p = percept(Vec3::ZERO, Some(Vec3::new(14.0, 0.0, 0.0)));
    brain.tick(def, &p, &mut rng);
    brain.los_ticks = 25;
    let acts = brain.tick(def, &p, &mut rng);
    assert!(
        acts.iter().any(|a| matches!(a, AiAction::Walk { speed_mult: m, dir, .. } if (*m - 1.0).abs() < 1e-6 && dir.x > 0.9)),
        "approach expected: {acts:?}"
    );
    // 白天直晒有阴影 → 逃阴影（FleeSun 优先于弓）。
    let mut brain = Brain::new();
    let mut p = percept(Vec3::ZERO, Some(Vec3::new(5.0, 0.0, 0.0)));
    p.day = true;
    p.sky_exposed = true;
    p.shelter = Some(Vec3::new(0.0, 0.0, 5.0));
    let acts = brain.tick(def, &p, &mut rng);
    assert!(matches!(brain.state, MobState::Flee));
    assert!(
        !acts.iter().any(|a| matches!(a, AiAction::Shoot { .. })),
        "fleeing sun takes priority"
    );
}

// ---------------------------------------------------------------------------
// 4) creeper：引爆倒计时 / 中断 / 爆炸伤害曲线
// ---------------------------------------------------------------------------

#[test]
fn creeper_fuse_countdown() {
    let def = MobId::CREEPER.def();
    let mut rng = || 7u32;
    let p = percept(Vec3::ZERO, Some(Vec3::new(2.0, 0.0, 0.0)));
    let mut brain = Brain::new();
    // tick1：索敌 + dist<3 → 进入 Swell（fuse 0）。
    brain.tick(def, &p, &mut rng);
    assert!(matches!(brain.state, MobState::Swell), "dist 2 < 3 → swell");
    let mut boom = None;
    let mut fuse_log = vec![brain.fuse];
    for t in 2..=60 {
        let acts = brain.tick(def, &p, &mut rng);
        fuse_log.push(brain.fuse);
        if acts
            .iter()
            .any(|a| matches!(a, AiAction::Detonate { radius } if (*radius - 3.0).abs() < 1e-6))
        {
            boom = Some(t);
            break;
        }
    }
    let t = boom.expect("must detonate");
    // 进入 tick 不计、其后每 tick +1 → fuse 到达 30 tick。
    assert_eq!(t, 31, "1 enter tick + 30 fuse ticks");
    assert_eq!(fuse_log[fuse_log.len() - 2] + 1, SWELL_TICKS);
    assert!(brain.exploded);
    assert_eq!(brain.tick(def, &p, &mut rng).len(), 0, "exploded → 无动作");
}

#[test]
fn creeper_fuse_breaks_at_7_blocks() {
    let def = MobId::CREEPER.def();
    let mut rng = || 7u32;
    let near = percept(Vec3::ZERO, Some(Vec3::new(2.0, 0.0, 0.0)));
    let mut brain = Brain::new();
    for _ in 0..12 {
        brain.tick(def, &near, &mut rng);
    }
    assert!(brain.fuse >= 10 && brain.fuse < SWELL_TICKS);
    // 玩家退到 8 格（>7）→ 引信倒数递减。
    let far = percept(Vec3::ZERO, Some(Vec3::new(8.0, 0.0, 0.0)));
    let start = brain.fuse;
    for _ in 0..start {
        brain.tick(def, &far, &mut rng);
        if brain.fuse < start {
            break;
        }
    }
    assert!(brain.fuse < start, "fuse decays out of 7-block range");
    for _ in 0..40 {
        brain.tick(def, &far, &mut rng);
    }
    assert_eq!(brain.fuse, 0);
    assert!(!brain.exploded);
    assert!(!matches!(brain.state, MobState::Swell));
    // 断视线同样中断（dist 保持 2，los=false）。
    let mut brain = Brain::new();
    for _ in 0..12 {
        brain.tick(def, &near, &mut rng);
    }
    let mut blind = near;
    blind.los = false;
    for _ in 0..40 {
        brain.tick(def, &blind, &mut rng);
    }
    assert_eq!(brain.fuse, 0, "no LOS → fuse decays");
    // 外部点燃恒递增直至爆炸。
    let mut brain = Brain::new();
    brain.ignite();
    let alone = percept(Vec3::ZERO, None);
    let mut boom = false;
    for _ in 0..60 {
        let acts = brain.tick(def, &alone, &mut rng);
        if acts.iter().any(|a| matches!(a, AiAction::Detonate { .. })) {
            boom = true;
            break;
        }
    }
    assert!(boom, "ignited creeper explodes without target");
}

#[test]
fn creeper_explosion_damage_curve_26_1() {
    // dmg = ((p²+p)/2)·7·2R + 1，p=(1−d/2R)·exposure。
    assert!(
        (explosion_damage(0.0, 3.0, 1.0) - 43.0).abs() < 1e-4,
        "贴脸 43"
    );
    assert!(
        (explosion_damage(0.0, 6.0, 1.0) - 85.0).abs() < 1e-4,
        "充能 85"
    );
    // d=3, R=3: p=0.5 → (0.25+0.5)/2×42+1 = 16.75
    assert!((explosion_damage(3.0, 3.0, 1.0) - 16.75).abs() < 1e-4);
    assert!(
        (explosion_damage(6.0, 3.0, 1.0) - 0.0).abs() < 1e-6,
        "范围外 0"
    );
    // 遮挡（exposure=0.5）衰减。
    let full = explosion_damage(2.0, 3.0, 1.0);
    let half = explosion_damage(2.0, 3.0, 0.5);
    assert!(half < full && half > 1.0);
}

// ---------------------------------------------------------------------------
// 5) spider 暗处索敌 + zombie/skeleton 白天燃烧
// ---------------------------------------------------------------------------

#[test]
fn spider_only_hunts_in_dark() {
    let def = MobId::SPIDER.def();
    let mut rng = || 121u32;
    let mut lit = percept(Vec3::ZERO, Some(Vec3::new(10.0, 0.0, 0.0)));
    lit.br = 0.6; // ≥0.5 亮处不索敌（Spider.java:225-229）
    let mut brain = Brain::new();
    brain.tick(def, &lit, &mut rng);
    assert!(!brain.has_target, "bright → no aggro");
    let mut dark = lit;
    dark.br = 0.4;
    brain.tick(def, &dark, &mut rng);
    assert!(brain.has_target, "dark → aggro");
}

#[test]
fn zombie_burns_in_daylight() {
    let def = MobId::ZOMBIE.def();
    let mut rng = || 0u32; // rand=0 → 0 < (br−0.4)·2 必成立
    let mut p = percept(Vec3::ZERO, None);
    p.day = true;
    p.sky_exposed = true;
    p.br = magic_light(15); // 直晒 raw=15 → br=1.0
    assert!((p.br - 1.0).abs() < 1e-6);
    let mut brain = Brain::new();
    let acts = brain.tick(def, &p, &mut rng);
    assert!(
        acts.contains(&AiAction::SetOnFire { ticks: 160 }),
        "白天直晒 → 点燃 8s"
    );
    // 夜晚 / 入水 / 头盔 / 亮度不足 → 不燃。
    let mut p2 = p;
    p2.day = false;
    assert!(
        !brain
            .tick(def, &p2, &mut rng)
            .iter()
            .any(|a| matches!(a, AiAction::SetOnFire { .. })),
        "夜晚"
    );
    let mut p3 = p;
    p3.in_water = true;
    assert!(
        !brain
            .tick(def, &p3, &mut rng)
            .iter()
            .any(|a| matches!(a, AiAction::SetOnFire { .. })),
        "入水"
    );
    let mut p4 = p;
    p4.head_armor = true;
    assert!(
        !brain
            .tick(def, &p4, &mut rng)
            .iter()
            .any(|a| matches!(a, AiAction::SetOnFire { .. })),
        "头盔挡火"
    );
    let mut p5 = p;
    p5.br = magic_light(8); // v=0.533 → br≈0.213 ≤0.5
    assert!(p5.br < 0.5, "26.1 曲线：raw8 → br {br}", br = p5.br);
    assert!(
        !brain
            .tick(def, &p5, &mut rng)
            .iter()
            .any(|a| matches!(a, AiAction::SetOnFire { .. })),
        "亮度不足"
    );
    // skeleton 同燃；creeper/spider 不燃。
    let mut brain = Brain::new();
    let acts = brain.tick(MobId::SKELETON.def(), &p, &mut rng);
    assert!(
        acts.iter().any(|a| matches!(a, AiAction::SetOnFire { .. })),
        "skeleton 亡灵燃烧"
    );
    let mut brain = Brain::new();
    let acts = brain.tick(MobId::CREEPER.def(), &p, &mut rng);
    assert!(
        !acts.iter().any(|a| matches!(a, AiAction::SetOnFire { .. })),
        "creeper 不燃烧"
    );
    // burn_in_daylight 单元：br≤0.5 硬阈值。
    let mut rng = || 0u32;
    assert!(
        !burn_in_daylight(true, false, 0.5, true, &mut rng),
        "br=0.5 不燃（需 >0.5）"
    );
    assert!(burn_in_daylight(true, false, 0.51, true, &mut rng));
}

// ---------------------------------------------------------------------------
// 6) 刷怪：亮度三条件 / 距离带 / 上限 / 和平豁免
// ---------------------------------------------------------------------------

struct FlatWorld;
impl SpawnWorld for FlatWorld {
    fn sky_light(&self, _p: BlockPos) -> u8 {
        0
    }
    fn block_light(&self, _p: BlockPos) -> u8 {
        0
    }
    fn can_see_sky(&self, _p: BlockPos) -> bool {
        true
    }
    fn find_spawn_pos(&self, x: i32, z: i32) -> Option<BlockPos> {
        Some(BlockPos::new(x, 64, z))
    }
}

struct LitWorld;
impl SpawnWorld for LitWorld {
    fn sky_light(&self, _p: BlockPos) -> u8 {
        0
    }
    fn block_light(&self, _p: BlockPos) -> u8 {
        5 // > block_light_limit 0
    }
    fn can_see_sky(&self, _p: BlockPos) -> bool {
        false
    }
    fn find_spawn_pos(&self, x: i32, z: i32) -> Option<BlockPos> {
        Some(BlockPos::new(x, 64, z))
    }
}

#[test]
fn spawn_light_three_conditions() {
    let cfg = SpawnConfig::default();
    // 洞穴全黑：任意 rng 都允许。
    for v in [0u32, 1, 7, 15, 31, 100] {
        let mut rng = make_seq(vec![v, v]);
        assert!(
            is_dark_enough(0, 0, 0, &cfg, &mut rng),
            "sky0 blk0 bright0 必可刷 (rng={v})"
        );
    }
    // block light > 0 → 拒（limit=0）。
    for v in 0..64u32 {
        let mut rng = make_seq(vec![v, 7]);
        assert!(
            !is_dark_enough(0, 5, 5, &cfg, &mut rng),
            "block 5 > limit 0"
        );
    }
    // sky > rand(32) → 拒。
    let mut rng = make_seq(vec![0, 7]); // sky 1 > 0 → 拒
    assert!(!is_dark_enough(1, 0, 1, &cfg, &mut rng), "sky1 > rand32=0");
    let mut rng = make_seq(vec![15, 1]); // sky 1 ≤ 15；bright 1 ≤ 1 → 过
    assert!(is_dark_enough(1, 0, 1, &cfg, &mut rng));
    // bright > rand(8) → 拒（地表 sky15 bright15 永不可刷）。
    for v in 0..64u32 {
        let mut rng = make_seq(vec![31, v % 8]); // sky15 ≤ 31; bright 15 ≤ ≤7 false
        assert!(!is_dark_enough(15, 0, 15, &cfg, &mut rng), "地表亮处不可刷");
    }
    // 自定义维度 limit=7（下界风格）：block5 可过。
    let cfg2 = SpawnConfig {
        block_light_limit: 7,
        ..Default::default()
    };
    let mut rng = make_seq(vec![15, 0]);
    assert!(
        is_dark_enough(0, 5, 0, &cfg2, &mut rng),
        "limit 7 时 block5 可刷"
    );
    // 兼容旧接口。
    let any = (0..200).any(|_| light_allows_hostile(0, 0, &mut lcg(3)));
    assert!(any);
    assert!(!light_allows_hostile(0, 5, &mut lcg(3)));
}

#[test]
fn spawn_round_distance_and_caps() {
    let kinds = [
        MobId::ZOMBIE,
        MobId::SKELETON,
        MobId::CREEPER,
        MobId::SPIDER,
    ];
    let cfg = SpawnConfig::default();
    let player = Vec3::new(0.0, 64.0, 0.0);
    let mut total = 0usize;
    for seed in 1..=40u64 {
        let mut rng = lcg(seed * 7919);
        let out = spawn_round(&FlatWorld, &[player], 0, 289, &kinds, &cfg, &mut rng);
        assert!(out.len() <= 3 * 4, "每轮 ≤ 3 组 × 4 walk");
        for sp in &out {
            let c = Vec3::new(
                sp.pos.x as f32 + 0.5,
                sp.pos.y as f32,
                sp.pos.z as f32 + 0.5,
            );
            let d2 = player.distance_squared(c);
            assert!(d2 > 576.0 && d2 <= 16384.0, "距离带 (24,128]，得 {d2}");
            assert!(kinds.contains(&sp.kind));
        }
        total += out.len();
    }
    assert!(total > 0, "黑暗平坦世界必须能刷出怪");
    // 全局上限：alive ≥ 70×289/289 → 空。
    let mut rng = lcg(9);
    assert!(
        spawn_round(&FlatWorld, &[player], 70, 289, &kinds, &cfg, &mut rng).is_empty(),
        "cap 70"
    );
    // 和平难度 / 敌对生成关闭 → 空。
    let peace = SpawnConfig {
        peaceful: true,
        ..Default::default()
    };
    let mut rng = lcg(9);
    assert!(
        spawn_round(&FlatWorld, &[player], 0, 289, &kinds, &peace, &mut rng).is_empty(),
        "和平不刷"
    );
    let noenemy = SpawnConfig {
        spawn_enemies: false,
        ..Default::default()
    };
    let mut rng = lcg(9);
    assert!(
        spawn_round(&FlatWorld, &[player], 0, 289, &kinds, &noenemy, &mut rng).is_empty(),
        "豁免开关"
    );
    // 亮世界（blockLight 5 > limit 0）→ 空。
    let mut any_attempt_had_light = false;
    for seed in 1..=40u64 {
        let mut rng = lcg(seed * 13);
        let out = spawn_round(&LitWorld, &[player], 0, 289, &kinds, &cfg, &mut rng);
        assert!(out.is_empty(), "亮处不可刷");
        any_attempt_had_light = true;
    }
    assert!(any_attempt_had_light);
    // 出生点 24 格豁免：min_dist 放宽到 0 时仍拒出生点附近。
    let cfg3 = SpawnConfig {
        min_dist_sqr: 0.0,
        respawn_center: Some(player),
        ..Default::default()
    };
    for seed in 1..=40u64 {
        let mut rng = lcg(seed * 23);
        let out = spawn_round(&FlatWorld, &[player], 0, 289, &kinds, &cfg3, &mut rng);
        for sp in &out {
            let c = Vec3::new(
                sp.pos.x as f32 + 0.5,
                sp.pos.y as f32,
                sp.pos.z as f32 + 0.5,
            );
            assert!(player.distance_squared(c) > 576.0, "出生点 24 格豁免");
        }
    }
}

// ---------------------------------------------------------------------------
// 7) 死亡掉落（DropEvent）
// ---------------------------------------------------------------------------

#[test]
fn death_drops_ranges() {
    let pos = Vec3::new(1.5, 64.0, 2.5);
    let mut seen_eye = false;
    let mut seen_empty_string = false;
    for seed in 0..300u64 {
        let mut rng = lcg(seed + 1);
        let z = death_drops(MobKind::Zombie, true, pos, &mut rng);
        assert!(z.len() <= 1);
        for d in &z {
            assert_eq!(d.item, "rotten_flesh");
            assert!((1..=2).contains(&d.count));
            assert_eq!(d.pos, pos);
        }
        let s = death_drops(MobKind::Skeleton, true, pos, &mut rng);
        for d in &s {
            assert!(d.item == "bone" || d.item == "arrow", "{}", d.item);
            assert!((1..=2).contains(&d.count));
        }
        let c = death_drops(MobKind::Creeper, true, pos, &mut rng);
        for d in &c {
            assert_eq!(d.item, "gunpowder");
            assert!((1..=2).contains(&d.count));
        }
        let sp = death_drops(MobKind::Spider, true, pos, &mut rng);
        for d in &sp {
            assert!(d.item == "string" || d.item == "spider_eye");
            assert!((1..=2).contains(&d.count));
            if d.item == "spider_eye" {
                seen_eye = true;
            }
            if d.item == "string" && d.count == 0 {
                seen_empty_string = true;
            }
        }
        // 非玩家击杀不掉蜘蛛眼。
        let sp2 = death_drops(MobKind::Spider, false, pos, &mut rng);
        assert!(!sp2.iter().any(|d| d.item == "spider_eye"));
    }
    assert!(seen_eye, "300 次至少一次掉蜘蛛眼");
    let _ = seen_empty_string;
}

// ---------------------------------------------------------------------------
// 8) 受击击退（LivingEntity.knockback 26.1）
// ---------------------------------------------------------------------------

#[test]
fn knockback_formula() {
    // 站立、无抗性、power 0.5、方向 +X → vel' = (−0.5, min(0.4, 0.5), 0)。
    let v = knockback_velocity(Vec3::ZERO, true, 0.0, 0.5, Vec3::X);
    assert!((v.x + 0.5).abs() < 1e-6, "vel.x/2 − p = −0.5");
    assert!((v.y - 0.4).abs() < 1e-6, "站立上跳封顶 0.4");
    assert!(v.z.abs() < 1e-6);
    // 抗性 1 → 完全免疫。
    let v = knockback_velocity(Vec3::new(1.0, 2.0, 3.0), true, 1.0, 0.5, Vec3::X);
    assert_eq!(v, Vec3::new(1.0, 2.0, 3.0));
    // 抗性 0.5、power 0.5 → p=0.25；y=min(0.4,0.25)=0.25。
    let v = knockback_velocity(Vec3::ZERO, true, 0.5, 0.5, Vec3::X);
    assert!((v.x + 0.25).abs() < 1e-6 && (v.y - 0.25).abs() < 1e-6);
    // 空中：y 不动。
    let v = knockback_velocity(Vec3::new(0.0, -1.0, 0.0), false, 0.0, 0.5, Vec3::X);
    assert!((v.y + 1.0).abs() < 1e-6, "空中保持 y");
    // 零方向：原样返回（原版用随机数，逻辑层不动）。
    let v = knockback_velocity(Vec3::new(1.0, 0.0, 0.0), true, 0.0, 0.5, Vec3::ZERO);
    assert_eq!(v, Vec3::new(1.0, 0.0, 0.0));
}

// ---------------------------------------------------------------------------
// 9) 消散 / noActionTime 账本 / raw 亮度（Mob.java:655-683、Monster.java:50-54、
//    MobCategory.java:21、LevelReader.java:167-175）
// ---------------------------------------------------------------------------

#[test]
fn despawn_bands_match_mob_java_655_678() {
    let mut never = make_seq(vec![7]); // 7 % 800 != 0 → 随机门永不命中
    // >128² 立即移除（despawnDistance=128，MobCategory.java:7）。
    assert!(should_despawn(
        129.0 * 129.0,
        0,
        DESPAWN_DIST,
        NO_DESPAWN_DIST,
        &mut never
    ));
    // 32..128 带：idle ≤ 600 不移除。
    assert!(!should_despawn(
        50.0 * 50.0,
        600,
        DESPAWN_DIST,
        NO_DESPAWN_DIST,
        &mut never
    ));
    // idle > 600 且 rng 命中 1/800 → 移除。
    let mut hit = make_seq(vec![0]);
    assert!(should_despawn(
        50.0 * 50.0,
        601,
        DESPAWN_DIST,
        NO_DESPAWN_DIST,
        &mut hit
    ));
    // noDespawn=32（源码实况，派单"24"有误）：32² 内即使 idle 超限也不移。
    assert!(!should_despawn(
        31.0 * 31.0,
        10_000,
        DESPAWN_DIST,
        NO_DESPAWN_DIST,
        &mut hit
    ));
    // 31..32 之间无豁免（>32² 才进随机带）。
    assert!(should_despawn(
        33.0 * 33.0,
        601,
        DESPAWN_DIST,
        NO_DESPAWN_DIST,
        &mut hit
    ));
}

#[test]
fn no_action_time_increment_matches_monster_java() {
    // Mob.java:683 每 tick +1；Monster.java:51-54 敌对亮处（br>0.5）+2。
    assert_eq!(no_action_inc(true, 0.5), 1);
    assert_eq!(no_action_inc(false, 1.0), 1);
    assert_eq!(no_action_inc(true, 0.51), 3);
    assert_eq!(
        no_action_inc(true, magic_light(15)),
        3,
        "直晒 raw=15 → br=1.0"
    );
    assert_eq!(no_action_inc(true, magic_light(7)), 1);
}

#[test]
fn raw_brightness_subtracts_sky_darken() {
    // LevelReader.java:167-175 getRawBrightness(pos, darken)。
    assert_eq!(raw_brightness(15, 0, 11), 4, "夜晚直晒：15−11");
    assert_eq!(raw_brightness(15, 0, 0), 15, "白天直晒");
    assert_eq!(raw_brightness(3, 0, 11), 0, "天光扣到负封 0");
    assert_eq!(raw_brightness(0, 6, 11), 6, "方块光不受 skyDarken 影响");
}
