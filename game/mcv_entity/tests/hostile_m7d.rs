//! M7d 敌对三怪（烈焰人/末影人/嘎吱怪）AI 状态机 / 瞬移落位 / loot 对拍测试。
//! 参照值全部来自 26.1 原版源码（Blaze.java / EnderMan.java / Creaking.java），
//! 行号注于各断言。

use glam::Vec3;
use mcv_core::BlockPos;
use mcv_entity::ai::*;
use mcv_entity::defs::*;
use mcv_entity::drops::death_drops;

fn lcg(seed: u64) -> impl FnMut() -> u32 {
    let mut s = seed | 1;
    move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
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
        has_bow: false,
        hurt: false,
        blocked: false,
        avoid: None,
        shelter: None,
        difficulty: 2,
        waypoint: None,
        plook: Vec3::ZERO,
        vel_y: 0.0,
        on_ground: false,
    }
}

// ---------------------------------------------------------------------------
// 1) 三怪参数表（Blaze.java:54-56 / EnderMan.java:113-120 / Creaking.java:64-69）
// ---------------------------------------------------------------------------

#[test]
fn hostile_m7d_stat_table_matches_26_1() {
    let blaze = MobId::BLAZE.def();
    assert_eq!(blaze.name, "blaze");
    assert!((blaze.health - 20.0).abs() < 1e-6); // Attributes 默认 MAX_HEALTH
    assert!((blaze.attack_damage - 6.0).abs() < 1e-6); // Blaze.java:55
    assert!((blaze.follow_range - 48.0).abs() < 1e-6); // Blaze.java:56
    assert!((blaze.speed_attr - 0.23).abs() < 1e-6);
    assert!((blaze.half_size[0] - 0.3).abs() < 1e-6, "sized(0.6,1.8)/2");
    assert_eq!(blaze.xp, 10); // Blaze.java:40 xpReward=10
    let enderman = MobId::ENDERMAN.def();
    assert!((enderman.health - 40.0).abs() < 1e-6); // EnderMan.java:114
    assert!((enderman.speed_attr - 0.3).abs() < 1e-6); // :116
    assert!((enderman.attack_damage - 7.0).abs() < 1e-6); // :117
    assert!((enderman.follow_range - 64.0).abs() < 1e-6); // :119
    assert!(
        (enderman.half_size[1] - 1.45).abs() < 1e-6,
        "sized(0.6,2.9)/2"
    );
    let creaking = MobId::CREAKING.def();
    assert!((creaking.health - 1.0).abs() < 1e-6); // Creaking.java:66
    assert!((creaking.attack_damage - 3.0).abs() < 1e-6); // :67
    assert!((creaking.follow_range - 32.0).abs() < 1e-6); // :68
    assert!((creaking.speed_attr - 0.4).abs() < 1e-6); // :69
    assert_eq!(creaking.xp, 0); // Creaking.java:92
    // AI 表。
    let tb = ai_table(MobKind::Blaze);
    assert!(tb.water_hurt); // Blaze.java:113-116 isSensitiveToWater
    assert_eq!(tb.melee_interval, BLAZE_MELEE_INTERVAL);
    let te = ai_table(MobKind::EnderMan);
    assert!(te.water_hurt); // EnderMan.java:238-241
    assert!(!te.burn_sun, "末影人白天瞬移逃离不燃烧");
    let tc = ai_table(MobKind::Creaking);
    assert_eq!(tc.melee_interval_override, CREAKING_ATTACK_INTERVAL);
    assert!(!tc.water_hurt, "嘎吱怪心绑免疫环境伤（不可水杀）");
}

// ---------------------------------------------------------------------------
// 2) 烈焰人：volley 节拍（BlazeAttackGoal.java:212-245）+ 浮空 + 水/箭序列
// ---------------------------------------------------------------------------

#[test]
fn blaze_volley_rhythm_charge_burst_cooldown() {
    let def = MobId::BLAZE.def();
    let mut rng = || 121u32; // 121%120≠0 不触发游走
    // 距离 8 格（≤48 且 >2 贴身）齐射段。
    let p = percept(Vec3::ZERO, Some(Vec3::new(8.0, 0.0, 0.0)));
    let mut brain = Brain::new();
    let mut shots = Vec::new();
    let mut charged_at = None;
    let mut uncharged_at = None;
    for t in 1..=260 {
        for a in brain.tick(def, &p, &mut rng) {
            if let AiAction::ShootFireball { dir } = a {
                // 散布三角形抖动已并入 dir，归一化后模长必为 1。
                assert!((dir.length() - 1.0).abs() < 1e-4);
                shots.push(t);
            }
        }
        if brain.charged && charged_at.is_none() {
            charged_at = Some(t);
        }
        if charged_at.is_some() && uncharged_at.is_none() && !brain.charged {
            uncharged_at = Some(t);
        }
    }
    // 节拍（BlazeAttackGoal.java:219-234，attackTime 语义 = 递减到 0 的
    // 边界才换挡）：t1 充能 cd=60 → 每 tick −1 → t61 归零首发、cd=6 →
    // t67 / t73 再各一枚（step>1 共 3 发，:230-241）；step4 后 100t 冷却
    //（t79 归零撤充能、attackStep 归 0）。
    assert_eq!(shots, vec![61, 67, 73], "volley 节拍 60→6→6→100");
    assert_eq!(charged_at, Some(1), "attackStep==1 即充能亮相");
    assert_eq!(uncharged_at, Some(79), "step≥5 冷却归零撤充能");
    // 硬常量复核（Blaze.java:29-31 + AbstractHurtingProjectile.java:22-24）。
    assert_eq!(BLAZE_CHARGE_TICKS, 60);
    assert_eq!(BLAZE_BURST_TICKS, 6);
    assert_eq!(BLAZE_COOLDOWN_TICKS, 100);
    assert_eq!(BLAZE_VOLLEY_SHOTS, 3);
    assert!((FIREBALL_INITIAL_SPEED - 0.1).abs() < 1e-6);
    assert!((FIREBALL_ACCEL - 0.1).abs() < 1e-6);
    assert!((FIREBALL_INERTIA - 0.95).abs() < 1e-6);
    assert!((FIREBALL_DAMAGE - 5.0).abs() < 1e-6); // SmallFireball.java:38-44
    assert_eq!(FIREBALL_FIRE_TICKS, 100); // igniteForSeconds(5)
}

/// Blaze 浮空（Blaze.java:113-124 每 100t 换 triangle(0.5, 6.891) 高差 +
/// :126-131 (0.3−vel.y)·0.3 追升；:85-88 离地下落 ×0.6 落阻）。
#[test]
fn blaze_hover_lift_and_fall_drag() {
    let def = MobId::BLAZE.def();
    let mut rng = || 121u32;
    // 目标站在 +30 高台 → 目标眼 > 自身眼 + 高差（triangle 上界 ≈7.4）。
    let mut p = percept(Vec3::ZERO, Some(Vec3::new(5.0, 30.0, 0.0)));
    p.vel_y = 0.0;
    let mut brain = Brain::new();
    for _ in 0..30 {
        let acts = brain.tick(def, &p, &mut rng);
        let Some(AiAction::Hover { lift, fall_drag }) =
            acts.iter().find(|a| matches!(a, AiAction::Hover { .. }))
        else {
            panic!("blaze 必须每 tick 发 Hover");
        };
        assert!((lift - 0.09).abs() < 1e-5, "vel_y=0 → (0.3−0)·0.3 = 0.09");
        assert!((fall_drag - 1.0).abs() < 1e-6, "vel_y≥0 无落阻");
    }
    // 下落（离地 + vel<0）→ 落阻 0.6（:85-88）。
    p.vel_y = -0.5;
    let mut brain = Brain::new();
    let acts = brain.tick(def, &p, &mut rng);
    let Some(AiAction::Hover { fall_drag, .. }) =
        acts.iter().find(|a| matches!(a, AiAction::Hover { .. }))
    else {
        panic!("missing Hover");
    };
    assert!((fall_drag - BLAZE_FALL_DRAG).abs() < 1e-6);
    assert!((BLAZE_FALL_DRAG - 0.6).abs() < 1e-6);
    // 着地不落阻（:85 !onGround 门）。
    p.on_ground = true;
    let mut brain = Brain::new();
    let acts = brain.tick(def, &p, &mut rng);
    let Some(AiAction::Hover { fall_drag, .. }) =
        acts.iter().find(|a| matches!(a, AiAction::Hover { .. }))
    else {
        panic!("missing Hover on ground");
    };
    assert!((fall_drag - 1.0).abs() < 1e-6, "on_ground 免落阻");
}

// ---------------------------------------------------------------------------
// 3) 末影人：凝视激怒 / 凝视冻结 / 瞬移请求 / 白天高原逃离
// ---------------------------------------------------------------------------

/// 玩家在 pos 期望视线正对 mob（isLookingAtMe 镜像的输入构造）。
fn stared_percept(mob_pos: Vec3, player_pos: Vec3) -> Percept {
    let eye = player_pos + Vec3::Y * mcv_game::Player::EYE;
    let gaze = Vec3::new(
        mob_pos.x - eye.x,
        mob_pos.y + 1.45 * 1.7 - eye.y,
        mob_pos.z - eye.z,
    );
    let mut p = percept(mob_pos, Some(player_pos));
    p.plook = gaze.normalize();
    p
}

#[test]
fn enderman_stare_activate_after_5_ticks() {
    let def = MobId::ENDERMAN.def();
    let mut rng = || 121u32;
    // 距 20 格被凝视（EnderMan.java:203-211 isBeingStaredBy / :508-509 aggroTime）。
    let p = stared_percept(Vec3::ZERO, Vec3::new(0.0, 0.0, 20.0));
    let mut brain = Brain::new();
    for t in 1..=5 {
        brain.tick(def, &p, &mut rng);
        assert!(!brain.has_target, "t{t} 未满 5t 不激怒");
    }
    brain.tick(def, &p, &mut rng);
    assert!(brain.has_target, "凝视 5t 确认 → 仇恨（aggroTime=5）");
    assert_eq!(ENDERMAN_STARE_AGGR_TICKS, 5);
}

#[test]
fn enderman_stare_pending_resets_when_look_away() {
    let def = MobId::ENDERMAN.def();
    let mut rng = || 121u32;
    let p = stared_percept(Vec3::ZERO, Vec3::new(0.0, 0.0, 20.0));
    let mut brain = Brain::new();
    // 3t 凝视 + 移开 1t（plook 归零）→ pending 归零重计。
    brain.tick(def, &p, &mut rng);
    brain.tick(def, &p, &mut rng);
    brain.tick(def, &p, &mut rng);
    assert!(brain.stare_pending > 0);
    let mut away = p;
    away.plook = Vec3::ZERO;
    brain.tick(def, &away, &mut rng);
    assert_eq!(brain.stare_pending, 0, "canContinueToUse 失败 → 归零");
    for _ in 0..4 {
        brain.tick(def, &p, &mut rng);
        assert!(!brain.has_target);
    }
    brain.tick(def, &p, &mut rng);
    assert!(brain.has_target, "重计满 5t 激怒");
}

#[test]
fn enderman_freezes_and_tp_away_when_stared_close() {
    let def = MobId::ENDERMAN.def();
    let mut rng = || 121u32;
    let p = stared_percept(Vec3::ZERO, Vec3::new(0.0, 0.0, 20.0));
    let mut brain = Brain::new();
    for _ in 0..6 {
        brain.tick(def, &p, &mut rng);
    }
    assert!(brain.has_target);
    // 贴脸 3 格被凝视（EnderMan.java:557-561：dist²<16 → teleport() 离开，
    // 立定对视不动——只 Look，瞬移请求由主控重抽落位）。
    let close = stared_percept(Vec3::ZERO, Vec3::new(3.0, 0.0, 0.0));
    brain.tp_cd = 30;
    let acts = brain.tick(def, &close, &mut rng);
    assert!(
        !acts.iter().any(|a| matches!(a, AiAction::Walk { .. })),
        "被凝视立定不动"
    );
    assert!(acts.iter().any(|a| matches!(a, AiAction::Look(..))));
    assert!(
        acts.iter()
            .any(|a| matches!(a, AiAction::Teleport { away_from: None })),
        "dist<4 → 随机瞬移离开"
    );
    assert_eq!(brain.tp_cd, 0, "凝视 tick 清 teleportTime");
    // 凝视中不累计背向瞬移账（tp_cd 停归 0）。
    let acts = brain.tick(def, &close, &mut rng);
    assert!(acts.iter().any(|a| matches!(a, AiAction::Teleport { .. })));
}

#[test]
fn enderman_chase_teleport_30t_when_far() {
    let def = MobId::ENDERMAN.def();
    let mut rng = || 121u32;
    let p = stared_percept(Vec3::ZERO, Vec3::new(0.0, 0.0, 20.0));
    let mut brain = Brain::new();
    for _ in 0..6 {
        brain.tick(def, &p, &mut rng);
    }
    // 拉开 30 格 + tp_cd 到点 → teleportTowards 逼近玩家（EnderMan.java:564-566）。
    let far = stared_percept(Vec3::ZERO, Vec3::new(0.0, 0.0, 30.0));
    brain.tp_cd = 30;
    let acts = brain.tick(def, &far, &mut rng);
    let back = acts.iter().find_map(|a| match a {
        AiAction::Teleport { away_from } => *away_from,
        _ => None,
    });
    let back = back.expect("30 格 + 30t → teleportTowards 逼近");
    assert!(
        back.x.abs() < 1e-5 && (back.z - 30.0).abs() < 1e-5,
        "away_from = 玩家位 {back:?}"
    );
    assert_eq!(brain.tp_cd, 0, "发请求即清 teleportTime");
    // tp_cd 未到点 → 不发请求。
    let mut brain = Brain::new();
    for _ in 0..6 {
        brain.tick(def, &p, &mut rng);
    }
    brain.tp_cd = 29;
    let acts = brain.tick(def, &far, &mut rng);
    assert!(
        !acts.iter().any(|a| matches!(a, AiAction::Teleport { .. })),
        "teleportTime < 30 不瞬移"
    );
    // 常量复核。
    assert_eq!(ENDERMAN_CHASE_TP_TICKS, 30);
    assert!((ENDERMAN_CHASE_TP_DIST - 16.0).abs() < 1e-6);
}

#[test]
fn enderman_anger_speed_mult_1_5() {
    // SPEED_MODIFIER_ATTACKING ADD_VALUE +0.15（EnderMan.java:70-72）：
    // 追击 mult = (0.3+0.15)/0.3 = 1.5。
    assert!((ENDERMAN_ANGER_SPEED_ADDED - 0.15).abs() < 1e-6);
    let def = MobId::ENDERMAN.def();
    let mut rng = || 121u32;
    let p = stared_percept(Vec3::ZERO, Vec3::new(0.0, 0.0, 20.0));
    let mut brain = Brain::new();
    for _ in 0..6 {
        brain.tick(def, &p, &mut rng);
    }
    let acts = brain.tick(def, &p, &mut rng);
    assert!(acts.iter().any(
        |a| matches!(a, AiAction::Walk { speed_mult, .. } if (*speed_mult - 1.5).abs() < 1e-5)
    ));
}

#[test]
fn enderman_daylight_escape_at_600_held() {
    let def = MobId::ENDERMAN.def();
    let mut rng = || 0u32; // burn_in_daylight 概率门必过
    let p = stared_percept(Vec3::ZERO, Vec3::new(0.0, 0.0, 20.0));
    let mut brain = Brain::new();
    for _ in 0..6 {
        brain.tick(def, &p, &mut rng);
    }
    let mut day = p;
    day.day = true;
    day.sky_exposed = true;
    day.br = 1.0;
    brain.target_held = 599;
    let acts = brain.tick(def, &day, &mut rng);
    assert!(
        !acts.iter().any(|a| matches!(a, AiAction::Teleport { .. })),
        "targetChangeTime<600 不逃离"
    );
    brain.target_held = 600;
    let acts = brain.tick(def, &day, &mut rng);
    assert!(
        acts.iter()
            .any(|a| matches!(a, AiAction::Teleport { away_from: None })),
        "白天高原 600t → teleport 逃离（EnderMan.java:245-251）"
    );
    assert!(!brain.has_target, "逃离 = 丢目标");
    assert_eq!(ENDERMAN_SUN_ESCAPE_TICKS, 600);
}

// ---------------------------------------------------------------------------
// 4) 嘎吱怪：凝视激活 12 格 / 定格 / 节拍 40t 近战（Creaking.java:68/69/466）
// ---------------------------------------------------------------------------

/// 嘎吱怪凝视构造：cone 0.5 不衰减（Creaking.java:466），gaze 高 = 眼位。
fn creaking_stared(mob_pos: Vec3, player_pos: Vec3) -> Percept {
    let eye = player_pos + Vec3::Y * mcv_game::Player::EYE;
    let gaze = Vec3::new(
        mob_pos.x - eye.x,
        mob_pos.y + 1.35 * 1.7 - eye.y,
        mob_pos.z - eye.z,
    );
    let mut p = percept(mob_pos, Some(player_pos));
    p.plook = gaze.normalize();
    p
}

#[test]
fn creaking_activates_only_when_stared_in_12() {
    let def = MobId::CREAKING.def();
    let mut rng = || 121u32;
    let mut brain = Brain::new();
    // 8 格被凝视 → 激活（checkCanMove activate / Creaking.java:69 半径 144²）。
    let p = creaking_stared(Vec3::ZERO, Vec3::new(8.0, 0.0, 0.0));
    brain.tick(def, &p, &mut rng);
    assert!(brain.has_target, "8<12 ∧ stared → activate");
    // 15 格（>12.4）被凝视 → 休眠游走。
    let mut brain = Brain::new();
    let far = creaking_stared(Vec3::ZERO, Vec3::new(15.0, 0.0, 0.0));
    let acts = brain.tick(def, &far, &mut rng);
    assert!(!brain.has_target, "15>12 不激活");
    assert!(acts.iter().any(|a| matches!(a, AiAction::Walk { .. })));
    // 未凝视 → 休眠（游走速度乘 0.3 = CREAKING_IDLE_SPEED_MULT）。
    let mut brain = Brain::new();
    let mut p = percept(Vec3::ZERO, Some(Vec3::new(5.0, 0.0, 0.0)));
    p.plook = Vec3::Y; // 看别处（dot < 0.5）
    let acts = brain.tick(def, &p, &mut rng);
    assert!(!brain.has_target);
    assert!(acts.iter().any(|a| matches!(a, AiAction::Walk { speed_mult, .. } if (*speed_mult - CREAKING_IDLE_SPEED_MULT).abs() < 1e-5)));
}

#[test]
fn creaking_freezes_when_stared_and_40t_interval() {
    let def = MobId::CREAKING.def();
    let mut rng = || 121u32;
    let mut brain = Brain::new();
    let p = creaking_stared(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0));
    brain.tick(def, &p, &mut rng); // 激活（1<12）
    assert!(brain.has_target);
    // 被凝视 = 定格：仅 Look，无移动无攻击（Creaking.java:454-481 CAN_MOVE）。
    let acts = brain.tick(def, &p, &mut rng);
    assert!(!acts.iter().any(|a| matches!(a, AiAction::Walk { .. })));
    assert!(acts.iter().all(|a| matches!(a, AiAction::Look(..))));
    // 移开视线（dot<0.5）→ 近战节拍 40t（ATTACK_INTERVAL，Creaking.java:68）。
    let mut p2 = creaking_stared(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0));
    p2.plook = Vec3::Y;
    let mut hits = Vec::new();
    for t in 1..=90 {
        let acts = brain.tick(def, &p2, &mut rng);
        for a in acts {
            if let AiAction::MeleeHit { damage } = a {
                assert!((damage - 3.0).abs() < 1e-6);
                hits.push(t);
            }
        }
    }
    assert_eq!(hits[0], 1, "attack_cd 初始 0 → 进入即第一击");
    assert_eq!(hits[1], hits[0] + CREAKING_ATTACK_INTERVAL as i32);
    assert_eq!(hits[2], hits[1] + CREAKING_ATTACK_INTERVAL as i32);
    assert_eq!(CREAKING_ATTACK_INTERVAL, 40);
    assert_eq!(CREAKING_ACTIVATE_DIST, 12.0); // Creaking.java:69 dist²<144
    assert!((CREAKING_STARE_CONE - 0.5).abs() < 1e-9);
    assert!((ENDERMAN_STARE_CONE - 0.025).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// 5) 凝视锥（LivingEntity.isLookingAtMe:1723-1742 镜像单元）
// ---------------------------------------------------------------------------

#[test]
fn looking_at_cone_math() {
    let eye = Vec3::new(0.0, 1.62, 10.0);
    let mob = Vec3::ZERO;
    let dir = (Vec3::new(0.0, 0.845, -10.0)).normalize(); // 直指
    assert!(looking_at(dir, eye, mob, 0.845, true, 0.025, true));
    // 视线偏 30°（dot≈0.866）< 阈值 1−0.025/d → 不凝视。
    let off = Vec3::new(
        10.0_f32.to_radians().sin(),
        0.0,
        -10.0_f32.to_radians().cos(),
    )
    .normalize();
    assert!(!looking_at(off, eye, mob, 0.845, true, 0.025, true));
    // 嘎吱怪 cone 0.5 不衰减：dot 0.866 > 1−0.5 = 0.5 → 凝视。
    assert!(looking_at(off, eye, mob, 0.845, true, 0.5, false));
    // 断视线恒否。
    assert!(!looking_at(dir, eye, mob, 0.845, false, 0.025, true));
}

// ---------------------------------------------------------------------------
// 6) 瞬移落位（EnderMan.teleport:277-302 + LivingEntity.randomTeleport 合成）
// ---------------------------------------------------------------------------

/// 落位扫描世界：y<floor → 空；[floor−1] 支撑面（solid 或 water）；above = 净空格。
struct TpWorld {
    floor: i32,
    support_liquid: bool,
    above_gap: Option<i32>, // Some(y) = y 格为固体（卡身用例）
}
impl TeleportGround for TpWorld {
    fn solid(&self, p: BlockPos) -> bool {
        if self.support_liquid {
            return false;
        }
        p.y == self.floor - 1 || self.above_gap == Some(p.y)
    }
    fn liquid(&self, p: BlockPos) -> bool {
        self.support_liquid && p.y == self.floor - 1
    }
}

#[test]
fn teleport_landing_finds_support_and_clearance() {
    let world = TpWorld {
        floor: 64,
        support_liquid: false,
        above_gap: None,
    };
    let land =
        try_teleport(&world, Vec3::new(3.4, 70.0, 5.6), 2.9).expect("实心面上方 3 净空 → 可落");
    assert!(
        (land.x - 3.5).abs() < 1e-4 && (land.z - 5.5).abs() < 1e-4,
        "方块底心 {land:?}"
    );
    assert!(
        (land.y - 64.0).abs() < 1e-5,
        "支撑上方一格 y=64，得 {land:?}"
    );
    // 顶部封 1 格（64 或 65 实心）→ 卡身拒。
    let capped = TpWorld {
        floor: 64,
        support_liquid: false,
        above_gap: Some(65),
    };
    assert!(try_teleport(&capped, Vec3::new(3.4, 70.0, 5.6), 2.9).is_none());
    // 支撑面是水（isWet）→ 拒（EnderMan.teleport:285-287）。
    let wet = TpWorld {
        floor: 64,
        support_liquid: true,
        above_gap: None,
    };
    assert!(try_teleport(&wet, Vec3::new(3.4, 70.0, 5.6), 2.9).is_none());
    // 无支撑（虚空柱直落到底界）→ None。
    let void = VoidWorld;
    assert!(try_teleport(&void, Vec3::new(3.4, 70.0, 5.6), 2.9).is_none());
}

struct VoidWorld;
impl TeleportGround for VoidWorld {
    fn solid(&self, _p: BlockPos) -> bool {
        false
    }
    fn liquid(&self, _p: BlockPos) -> bool {
        false
    }
}

#[test]
fn teleport_landing_eight_attempts_cap() {
    // 无支撑世界：每次瞬移重抽上界 = TELEPORT_ATTEMPTS=8 次尝试（任务拍板；
    // 原版 hurtServer 弹支 64 次语义近似）。
    let mut draws = 0u32;
    let out = teleport_landing(&VoidWorld, 2.9, || {
        draws += 1;
        Vec3::new(1.0, 70.0, 1.0)
    });
    assert!(out.is_none());
    assert_eq!(draws, TELEPORT_ATTEMPTS);
    assert_eq!(TELEPORT_ATTEMPTS, 8);
}

#[test]
fn teleport_candidate_ranges_match_vanilla() {
    let origin = Vec3::new(100.0, 64.0, 100.0);
    let mut rng = lcg(7);
    for _ in 0..400 {
        let c = teleport_random_target(origin, &mut rng);
        // x/z = (rand−0.5)·64 → ±32（EnderMan.java:256-261）。
        assert!((c.x - 100.0).abs() <= 32.0 && (c.z - 100.0).abs() <= 32.0);
        // y = next(64)−32 → ±32。
        assert!((c.y - 64.0).abs() <= 32.0, "y {c:?}");
    }
    // teleportTowards：`−dir·16` = 落点向 threat 逼近 16 格 + x/z ±4 抖动
    //（:267-274，dir = this−threat 归一 → −dir 指向 threat）。
    let threat = Vec3::new(90.0, 64.0, 100.0); // threat 在 −x 侧
    for _ in 0..400 {
        let c = teleport_away_target(origin, threat, &mut rng);
        let dx = c.x - origin.x;
        assert!((-20.001..=-11.999).contains(&dx), "逼近 16±4 抖动，得 {dx}");
    }
    assert!((TP_RAND_RANGE_XZ - 64.0).abs() < 1e-6);
    assert_eq!(TP_RAND_RANGE_Y, 64);
    assert!((TP_TOWARD_DIST - 16.0).abs() < 1e-6);
    assert!((TP_TOWARD_JITTER - 8.0).abs() < 1e-6);
    assert_eq!(TP_TOWARD_RANGE_Y, 16);
}

// ---------------------------------------------------------------------------
// 7) loot 对拍（26.1 data/loot_table/entity/*.json）
// ---------------------------------------------------------------------------

#[test]
fn m7d_loot_matches_vanilla_tables() {
    let pos = Vec3::new(1.5, 64.0, 2.5);
    // entity/blaze.json：blaze_rod uniform 0–1 且 killed_by_player 条件
    //（count==0 被 push 过滤 = 空事件，两端覆盖以 seen_rod/seen_empty 表达）。
    let mut seen_rod = false;
    let mut seen_empty = false;
    for seed in 0..300u64 {
        let mut rng = lcg(seed + 17);
        let no_kill = death_drops(MobKind::Blaze, false, pos, &mut rng);
        assert!(
            no_kill.is_empty(),
            "非玩家击杀不掉烈焰棒（killed_by_player）"
        );
        let mut rng = lcg(seed + 17);
        let z = death_drops(MobKind::Blaze, true, pos, &mut rng);
        assert!(z.len() <= 1);
        if z.is_empty() {
            seen_empty = true;
        }
        for d in &z {
            assert_eq!(d.item, "blaze_rod");
            assert_eq!(d.count, 1, "uniform 0–1 → 非零即 1");
            seen_rod = true;
        }
    }
    assert!(seen_rod && seen_empty, "0–1 两端必须都覆盖（300 seed）");
    // entity/enderman.json：ender_pearl uniform 0–1，无 killed_by_player 条件。
    let mut seen_pearl = false;
    for seed in 0..300u64 {
        let mut rng = lcg(seed + 991);
        let z = death_drops(MobKind::EnderMan, false, pos, &mut rng);
        assert!(z.len() <= 1);
        for d in &z {
            assert_eq!(d.item, "ender_pearl");
            assert!((0..=1).contains(&d.count));
            seen_pearl = true;
        }
    }
    assert!(seen_pearl);
    // entity/creaking.json：无 pools（零掉落；xpReward=0 亦在参数表测试）。
    for seed in 0..64u64 {
        let mut rng = lcg(seed + 1);
        assert!(death_drops(MobKind::Creaking, true, pos, &mut rng).is_empty());
    }
}
