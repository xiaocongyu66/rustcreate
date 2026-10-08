//! 敌对生物 AI 状态机（MC 26.1 Goal 体系的等价简化，参数见 /root/mc-ref/NOTES-mobs.md）。
//!
//! 状态图（zombie/spider 近战怪 / skeleton 弓怪 / creeper）：
//!
//! ```text
//!            受击(hurt)/索敌(dist≤FOLLOW & 视线)
//!   ┌───────────────────────────────────────────┐
//!   v                                           │
//! Idle ──p=1/120/tick──▶ Wander ──到点──▶ Idle   │
//!   │                      │                    │
//!   └─────── seek ◀────────┘◀───────────────────┘
//!                 │ dist≤攻击距离&视线
//!                 ▼
//!   melee: zombie/spider ─ Melee ─(20t 间隔攻击; 出距离→Seek)
//!   bow:   skeleton      ─ Ranged ─(15 格射程; >13 接近; <7.5 后撤; 横移;
//!                                    60t 周期射箭, 断视线 60t 停)
//!   creeper              ─ Swell ─(dist<3&视线起爆, 30t=1.5s;
//!                                   dist>7 或断视线倒数递减→Seek)
//!   creeper 避猫/ocelot(6 格)、skeleton 避狼/白天逃阴影 → Flee
//!   目标丢失: dist > FOLLOW 或 无视线 > 60 tick（TargetGoal.java:24,58-66）
//! ```
//!
//! 与原版寻路差异：原版 A*（GroundPathNavigation/WallClimberNavigation），
//! 此处直线移动 + `blocked` 时跳跃（spider 另有 LeapAtTarget：dist²∈(4,16) 起跳、
//! 贴墙攀爬由主控据 `wall_climb` 标志处理物理）。复杂地形下不等价（见 NOTES §7）。

use crate::defs::{MobDef, MobKind};
use crate::spawner::burn_in_daylight;
use glam::Vec3;

// ---- 参照常量（类名:行号见 NOTES-mobs.md） --------------------------------

/// Mob.java:112 — sqrt(2.04) − 0.6 ≈ 0.828（近战基础触达，另加目标半宽）。
pub const MELEE_REACH_BASE: f32 = 0.828;
/// TargetGoal.java:24 — 无视线记忆 60 tick。
pub const LOSE_SIGHT_TICKS: u32 = 60;
/// RandomStrollGoal.java:16 — DEFAULT_INTERVAL（每 tick p=1/120）。
pub const WANDER_INTERVAL_AVG: u32 = 120;
/// RandomStrollGoal.java:65 — DefaultRandomPos.getPos(mob, 10, 7)。
pub const WANDER_RADIUS: f32 = 10.0;
/// MeleeAttackGoal.java:21 — 近战攻击间隔 20 tick。
pub const MELEE_ATTACK_INTERVAL: u32 = 20;
/// RangedBowAttackGoal 半径（AbstractSkeleton.java:55 r=15.0F）。
pub const BOW_RANGE: f32 = 15.0;
/// RangedBowAttackGoal.java:128 — 拉弓 20 tick 满力。
pub const BOW_DRAW_TICKS: u32 = 20;
/// AbstractSkeleton.java:51-54 — HARD 20 / NORMAL 40 tick。
pub const BOW_INTERVAL_HARD: u32 = 20;
pub const BOW_INTERVAL_NORMAL: u32 = 40;
/// AbstractSkeleton.java:170 — 弹速 1.6。
pub const ARROW_SPEED: f32 = 1.6;
/// AbstractSkeleton.java:170 — inaccuracy = 14 − difficulty_id×4（普通=6）。
pub fn arrow_spread(difficulty: u8) -> f32 {
    14.0 - difficulty.min(3) as f32 * 4.0
}
/// AbstractBowAttackGoal 风筝阈值：sqrt(225×0.75)≈12.99 停追不退、sqrt(225×0.25)=7.5 后撤。
pub const KITE_STOP_DIST: f32 = 12.99;
pub const KITE_RETREAT_DIST: f32 = 7.5;
/// Creeper.java:55 — maxSwell = 30 tick（1.5 s）。
pub const SWELL_TICKS: u32 = 30;
/// SwellGoal.java:20 — dist²<9 → 起爆；:42 — dist²>49 → 中断。
pub const SWELL_START_DIST: f32 = 3.0;
pub const SWELL_BREAK_DIST: f32 = 7.0;
/// Creeper.java:56 — explosionRadius = 3（充能 ×2）。
pub const EXPLOSION_RADIUS: f32 = 3.0;
/// LeapAtTargetGoal.java:29 — dist²∈(4,16) 起跳（spider）。
pub const LEAP_MIN_DIST: f32 = 2.0;
pub const LEAP_MAX_DIST: f32 = 4.0;
/// AvoidEntityGoal 距离（猫/豹猫/狼，Creeper.java:67-68、AbstractSkeleton.java:79）。
pub const AVOID_DIST: f32 = 6.0;
/// Mob.java:497 — igniteForSeconds(8) = 160 tick 火。
pub const BURN_TICKS: u32 = 160;

/// 26.1 新爆炸伤害曲线（ExplosionDamageCalculator.getEntityDamageAmount）：
/// `R2 = 2R; p = (1 − d/R2)·exposure; dmg = ((p²+p)/2)·7·R2 + 1`。
/// 贴脸 exposure=1 → 7×(2R)+1（R=3 时 43；充能 R=6 时 85）。
pub fn explosion_damage(dist: f32, radius: f32, exposure: f32) -> f32 {
    let r2 = radius * 2.0;
    if dist >= r2 {
        return 0.0;
    }
    let p = ((1.0 - dist / r2) * exposure).clamp(0.0, 1.0);
    ((p * p + p) / 2.0) * 7.0 * r2 + 1.0
}

/// 每种怪的 AI 表（Goal 构造参数，见 NOTES-mobs.md §1-§4）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MobAiTable {
    /// 亡灵，白天直晒燃烧（zombie/skeleton）。
    pub burn_sun: bool,
    /// WaterAvoidingRandomStrollGoal speedModifier。
    pub stroll_speed: f32,
    /// 近战伤害（creeper=0：Creeper.java:182-185 无伤害）。
    pub melee_damage: f32,
    /// 近战间隔 tick（MeleeAttackGoal=20）。
    pub melee_interval: u32,
    /// skeleton 默认持弓（AbstractSkeleton.java:111）。
    pub default_bow: bool,
    /// LeapAtTargetGoal（spider）。
    pub leap: bool,
    /// WallClimberNavigation/onClimbable（spider，Spider.java:70-86）。
    pub wall_climb: bool,
    /// AvoidEntityGoal（creeper 避猫/豹猫；skeleton 避狼）。
    pub avoids: bool,
}

pub const fn ai_table(kind: MobKind) -> MobAiTable {
    match kind {
        MobKind::Zombie => MobAiTable {
            burn_sun: true,
            stroll_speed: 1.0, // Zombie.java:122
            melee_damage: 3.0, // Zombie.java:134（26.1 不随难度缩放）
            melee_interval: MELEE_ATTACK_INTERVAL,
            default_bow: false,
            leap: false,
            wall_climb: false,
            avoids: false,
        },
        MobKind::Skeleton => MobAiTable {
            burn_sun: true,
            stroll_speed: 1.0, // AbstractSkeleton.java:80
            melee_damage: 2.0, // 无弓兜底（Attributes 默认）
            melee_interval: MELEE_ATTACK_INTERVAL,
            default_bow: true,
            leap: false,
            wall_climb: false,
            avoids: true,
        },
        MobKind::Creeper => MobAiTable {
            burn_sun: false,
            stroll_speed: 0.8, // Creeper.java:70
            melee_damage: 0.0,
            melee_interval: MELEE_ATTACK_INTERVAL,
            default_bow: false,
            leap: false,
            wall_climb: false,
            avoids: true,
        },
        MobKind::Spider => MobAiTable {
            burn_sun: false,
            stroll_speed: 0.8, // Spider.java:61
            melee_damage: 2.0,
            melee_interval: MELEE_ATTACK_INTERVAL,
            default_bow: false,
            leap: true, // Spider.java:59
            wall_climb: true,
            avoids: false,
        },
        // 被动怪：只游走、不燃烧、不索敌。
        _ => MobAiTable {
            burn_sun: false,
            stroll_speed: 1.0,
            melee_damage: 0.0,
            melee_interval: MELEE_ATTACK_INTERVAL,
            default_bow: false,
            leap: false,
            wall_climb: false,
            avoids: false,
        },
    }
}

/// 近战判定距离：基础触达 + 目标半宽（MC 为 AABB 合成，Mob.java:1317-1323 简化）。
pub fn melee_range(target_half_width: f32) -> f32 {
    MELEE_REACH_BASE + target_half_width
}

/// 状态机状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobState {
    Idle,
    Wander,
    Seek,
    Melee,
    /// skeleton 弓战（含横移/风筝）。
    Ranged,
    /// creeper 引爆倒计时。
    Swell,
    /// 逃离（猫/狼/阴影/猫头鹰……）。
    Flee,
}

/// 状态机单 tick 输出（主控消费：物理/伤害/音效/爆炸实体）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AiAction {
    Nothing,
    /// 水平移动方向（单位向量 × speed_mult，实际速度 = def.speed_m_s × mult）。
    Walk {
        dir: Vec3,
        speed_mult: f32,
        jump: bool,
    },
    /// 视线转向（yaw 目标点）。
    Look(Vec3),
    /// 近战命中（伤害 = ATTACK_DAMAGE 属性）。
    MeleeHit {
        damage: f32,
    },
    /// skeleton 射箭：dir 已含抛物线补偿（y += 水平距×0.2，AbstractSkeleton.java:170），
    /// spread = 14 − 难度×4，base_damage ≈ 2.0（power=1 时）。
    Shoot {
        dir: Vec3,
        speed: f32,
        spread: f32,
        base_damage: f32,
    },
    /// creeper 充能中（fuse: 0..30，渲染嘶嘶/膨胀用）。
    Swelling {
        fuse: u32,
    },
    /// 爆炸事件（调用方结算 explosion_damage 并移除本实体）。
    Detonate {
        radius: f32,
    },
    /// 阳光点燃（ticks=160，Mob.java:497）。
    SetOnFire {
        ticks: u32,
    },
}

/// 每 tick 感知输入（主控/物理层提供）。
#[derive(Clone, Copy, Debug)]
pub struct Percept {
    pub pos: Vec3,
    /// 最近合法目标（玩家）世界坐标。
    pub target: Option<Vec3>,
    pub target_half_width: f32,
    /// 视线（raycast）。
    pub los: bool,
    /// getLightLevelDependentMagicValue（0..1，spawner::magic_light）。
    pub br: f32,
    /// 白天（EnvironmentAttributes.MONSTERS_BURN）。
    pub day: bool,
    /// canSeeSky(眼睛格)。
    pub sky_exposed: bool,
    pub in_water: bool,
    /// 头部有装备可挡阳光（Mob.java:484-496）。
    pub head_armor: bool,
    /// skeleton 是否持弓。
    pub has_bow: bool,
    /// 本 tick 受到伤害（HurtByTargetGoal → 转索敌）。
    pub hurt: bool,
    /// 移动被前方方块阻挡（主控碰撞结果 → 跳跃）。
    pub blocked: bool,
    /// 需躲避的实体（猫/豹猫/狼），6 格内由主控筛选后传入。
    pub avoid: Option<Vec3>,
    /// skeleton 白天的阴影藏身点（FleeSunGoal）。
    pub shelter: Option<Vec3>,
    /// 0=peaceful,1=easy,2=normal,3=hard。
    pub difficulty: u8,
}

impl Percept {
    fn sheltered_from_sun(&self) -> bool {
        self.in_water || self.head_armor
    }
}

/// 每只怪一份的 AI 运行时。
#[derive(Clone, Debug)]
pub struct Brain {
    pub state: MobState,
    /// 是否已有目标（TargetGoal 持有中）。
    pub has_target: bool,
    /// 连续无视线 tick（TargetGoal.unseenTicks）。
    pub unseen: u32,
    /// 连续有视线 tick（RangedBowAttackGoal.seeTime 简化）。
    pub los_ticks: u32,
    /// 游走目标点。
    pub wander_target: Vec3,
    /// creeper 引爆计数（0..SWELL_TICKS）。
    pub fuse: u32,
    /// 攻击冷却剩余（近战 20 / 弓 interval+draw）。
    pub attack_cd: u32,
    /// 横移方向（clockwise 标志，每 20t p=0.3 翻转）。
    pub strafe_clockwise: bool,
    strafe_flip_cd: u32,
    /// 本 tick 生效的 kind 级 leap 标志（Brain::tick 据 ai_table 注入）。
    leap_enabled: bool,
    /// 被外部点燃（打火石；Creeper.isIgnited）。
    pub ignited: bool,
    /// 已爆炸/死亡标志（Detonate 后 true，调用方负责移除实体）。
    pub exploded: bool,
}

impl Default for Brain {
    fn default() -> Self {
        Self::new()
    }
}

impl Brain {
    pub fn new() -> Self {
        Self {
            state: MobState::Idle,
            has_target: false,
            unseen: 0,
            los_ticks: 0,
            wander_target: Vec3::ZERO,
            fuse: 0,
            attack_cd: 0,
            strafe_clockwise: false,
            strafe_flip_cd: 0,
            leap_enabled: false,
            ignited: false,
            exploded: false,
        }
    }

    pub fn ignite(&mut self) {
        self.ignited = true;
    }

    fn clear_target(&mut self) {
        self.has_target = false;
        self.unseen = 0;
        self.los_ticks = 0;
        self.state = MobState::Idle;
    }

    /// 每 tick 推进；返回本 tick 动作列表（燃烧 + 移动/攻击）。
    pub fn tick(
        &mut self,
        def: &MobDef,
        p: &Percept,
        rng: &mut impl FnMut() -> u32,
    ) -> Vec<AiAction> {
        let tbl = ai_table(def.kind);
        self.leap_enabled = tbl.leap;
        let mut acts = Vec::new();

        // 0) 已爆炸：不再行动。
        if self.exploded {
            return acts;
        }

        // 1) 亡灵阳光燃烧（Mob.java:499-513，26.1 曲线版）。
        if tbl.burn_sun && burn_in_daylight(p.sky_exposed, p.sheltered_from_sun(), p.br, p.day, rng)
        {
            acts.push(AiAction::SetOnFire { ticks: BURN_TICKS });
        }

        // 2) 目标获取 / 维持 / 丢失（TargetGoal + NearestAttackableTargetGoal）。
        let dist = p.target.map_or(f32::INFINITY, |t| p.pos.distance(t));
        // spider 只在暗处索敌（Spider.java:225-229）；creeper 不追山羊(忽略)。
        let kind_ok = def.kind != MobKind::Spider || p.br < 0.5;
        if !self.has_target {
            let sees = p.target.is_some() && dist <= def.follow_range && p.los && kind_ok;
            // 受击反击（HurtByTargetGoal，各怪 targetSelector 第 1 位）：无需视线。
            let hurt_back = p.hurt && p.target.is_some() && dist <= def.follow_range;
            if sees || hurt_back {
                self.has_target = true;
                self.unseen = 0;
                self.los_ticks = 0;
                if matches!(self.state, MobState::Idle | MobState::Wander) {
                    self.state = MobState::Seek;
                }
            }
        } else {
            if p.los {
                self.unseen = 0;
                self.los_ticks += 1;
            } else {
                self.unseen += 1;
            }
            // 超跟丢半径 / 断视线 60 tick / spider 亮处 1/100（Spider.java:192-199）。
            let spider_blind =
                def.kind == MobKind::Spider && p.br >= 0.5 && rng().is_multiple_of(100);
            if dist > def.follow_range || self.unseen > LOSE_SIGHT_TICKS || spider_blind {
                self.clear_target();
            }
        }

        // 3) 行为分派。
        match def.kind {
            MobKind::Zombie | MobKind::Spider => {
                self.melee_life(def, &tbl, p, dist, rng, &mut acts);
            }
            MobKind::Skeleton => {
                self.skeleton_life(def, &tbl, p, dist, rng, &mut acts);
            }
            MobKind::Creeper => {
                self.creeper_life(def, &tbl, p, dist, rng, &mut acts);
            }
            _ => {
                self.wander_only(def, &tbl, p, rng, &mut acts);
            }
        }
        acts
    }

    // ---- 近战怪（zombie/spider） ------------------------------------------

    fn melee_life(
        &mut self,
        def: &MobDef,
        tbl: &MobAiTable,
        p: &Percept,
        dist: f32,
        rng: &mut impl FnMut() -> u32,
        acts: &mut Vec<AiAction>,
    ) {
        let _ = def;
        if !self.has_target {
            self.wander_only(def, tbl, p, rng, acts);
            return;
        }
        let range = melee_range(p.target_half_width);
        match self.state {
            MobState::Melee => {
                let t = p.target.unwrap_or(p.pos);
                acts.push(AiAction::Look(t));
                if dist > range {
                    self.state = MobState::Seek;
                    return;
                }
                self.attack_cd = self.attack_cd.saturating_sub(1);
                if self.attack_cd == 0 && p.los {
                    // MeleeAttackGoal.java:127-144：命中后重置 20 tick。
                    acts.push(AiAction::MeleeHit {
                        damage: tbl.melee_damage,
                    });
                    self.attack_cd = tbl.melee_interval;
                } else if dist > range * 0.5 {
                    acts.push(self.walk_toward(p, t, 1.0));
                }
            }
            _ => {
                self.state = MobState::Seek;
                let t = p.target.unwrap_or(p.pos);
                if dist <= range {
                    // start() 时 ticksUntilNextAttack=0 → 立刻能打。
                    self.state = MobState::Melee;
                    self.attack_cd = 0;
                } else {
                    acts.push(AiAction::Look(t));
                    acts.push(self.walk_toward(p, t, 1.0));
                }
            }
        }
        // 出目标距离兜底（clear 已在上面，这里保持状态一致）。
        let _ = rng;
    }

    // ---- skeleton（弓 + 白天逃阴影 + 避狼） --------------------------------

    fn skeleton_life(
        &mut self,
        def: &MobDef,
        tbl: &MobAiTable,
        p: &Percept,
        dist: f32,
        rng: &mut impl FnMut() -> u32,
        acts: &mut Vec<AiAction>,
    ) {
        // FleeSunGoal(优先级 3) 高于弓(4)：白天直晒 → 逃向阴影。
        if p.day && p.sky_exposed {
            if let Some(s) = p.shelter {
                self.state = MobState::Flee;
                acts.push(self.walk_toward(p, s, 1.0));
                return;
            }
        }
        // AvoidEntityGoal(Wolf, 6.0)（AbstractSkeleton.java:79）。
        if tbl.avoids {
            if let Some(a) = p.avoid {
                if p.pos.distance(a) < AVOID_DIST {
                    self.state = MobState::Flee;
                    acts.push(self.walk_away(p, a));
                    return;
                }
            }
        }
        if !self.has_target {
            self.wander_only(def, tbl, p, rng, acts);
            return;
        }
        if !p.has_bow {
            // 近战兜底：MeleeAttackGoal(1.2, false)（AbstractSkeleton.java:56）。
            self.melee_life(def, tbl, p, dist, rng, acts);
            return;
        }
        let t = p.target.unwrap_or(p.pos);
        self.state = MobState::Ranged;
        acts.push(AiAction::Look(t));
        // 追击/横移（RangedBowAttackGoal.java:86-113）：
        // 射程外或视线不足 20t → 直线接近；否则横移（<7.5 同时后撤）。
        if dist > KITE_STOP_DIST || self.los_ticks < 20 {
            acts.push(self.walk_toward(p, t, 1.0));
        } else {
            self.strafe_flip_cd = self.strafe_flip_cd.saturating_sub(1);
            if self.strafe_flip_cd == 0 {
                if rng() % 100 < 30 {
                    self.strafe_clockwise = !self.strafe_clockwise;
                }
                self.strafe_flip_cd = 20;
            }
            let mut dir = flat_perp(t - p.pos) * if self.strafe_clockwise { 1.0 } else { -1.0 };
            if dist < KITE_RETREAT_DIST {
                dir += flat_dir(p.pos - t); // 后撤分量
            }
            acts.push(AiAction::Walk {
                dir: normalize_or_zero(dir),
                speed_mult: 0.5, // strafe(±0.5, ±0.5)
                jump: p.blocked,
            });
        }
        // 射击：周期 = interval + 拉弓 20t（完整周期 normal 60 / hard 40）。
        self.attack_cd = self.attack_cd.saturating_sub(1);
        if self.attack_cd == 0 && p.los && dist <= BOW_RANGE {
            let h = flat_dist(p.pos, t);
            let mut dir = t - p.pos;
            dir.y += h * 0.2; // AbstractSkeleton.java:170 抛物线补偿
            acts.push(AiAction::Shoot {
                dir: normalize_or_zero(dir),
                speed: ARROW_SPEED,
                spread: arrow_spread(p.difficulty),
                base_damage: 2.0, // power=1 时 power×2.0（AbstractArrow.java:718-719）
            });
            let interval = if p.difficulty >= 3 {
                BOW_INTERVAL_HARD
            } else {
                BOW_INTERVAL_NORMAL
            };
            self.attack_cd = interval + BOW_DRAW_TICKS;
        }
    }

    // ---- creeper（逼近引爆 / 避猫） ----------------------------------------

    fn creeper_life(
        &mut self,
        def: &MobDef,
        tbl: &MobAiTable,
        p: &Percept,
        dist: f32,
        rng: &mut impl FnMut() -> u32,
        acts: &mut Vec<AiAction>,
    ) {
        // SwellGoal 优先级 2，高于 Avoid(3)：一旦起爆优先结算引信。
        if self.state == MobState::Swell {
            // SwellGoal.java:40-51：7 格内+视线 → +1，否则 −1；外部点燃恒 +1。
            let in_range = self.has_target && dist <= SWELL_BREAK_DIST && p.los;
            if self.ignited || in_range {
                self.fuse += 1;
            } else {
                self.fuse = self.fuse.saturating_sub(1);
            }
            if self.fuse >= SWELL_TICKS {
                self.exploded = true;
                self.clear_target();
                acts.push(AiAction::Detonate {
                    radius: EXPLOSION_RADIUS,
                });
            } else {
                if self.fuse == 0 {
                    self.state = if self.has_target {
                        MobState::Seek
                    } else {
                        MobState::Idle
                    };
                }
                acts.push(AiAction::Swelling { fuse: self.fuse });
            }
            return;
        }
        // 避猫/豹猫 6 格（Creeper.java:67-68；任务描述的"逃跑"即此）。
        if tbl.avoids {
            if let Some(a) = p.avoid {
                if p.pos.distance(a) < AVOID_DIST {
                    self.state = MobState::Flee;
                    acts.push(self.walk_away(p, a));
                    return;
                }
            }
        }
        if !self.has_target {
            if self.ignited {
                // 被点燃但无目标：直接倒数（tick 里 swellDir=+1，Creeper.java:129-131）。
                self.state = MobState::Swell;
                acts.push(AiAction::Swelling { fuse: self.fuse });
                return;
            }
            self.wander_only(def, tbl, p, rng, acts);
            return;
        }
        let t = p.target.unwrap_or(p.pos);
        if dist <= SWELL_START_DIST && p.los {
            self.state = MobState::Swell;
            acts.push(AiAction::Look(t));
            acts.push(AiAction::Swelling { fuse: self.fuse });
        } else {
            self.state = MobState::Seek;
            acts.push(AiAction::Look(t));
            acts.push(self.walk_toward(p, t, 1.0)); // MeleeAttackGoal(1.0) 追击
        }
        let _ = rng;
    }

    // ---- 公共：游走/待机 ----------------------------------------------------

    /// Idle →(每 tick 1/120)→ Wander(目标点 ≤10 格) →(到点)→ Idle。
    fn wander_only(
        &mut self,
        _def: &MobDef,
        tbl: &MobAiTable,
        p: &Percept,
        rng: &mut impl FnMut() -> u32,
        acts: &mut Vec<AiAction>,
    ) {
        if matches!(
            self.state,
            MobState::Seek | MobState::Melee | MobState::Ranged
        ) {
            // 目标刚丢失：回到待机。
            self.state = MobState::Idle;
        }
        match self.state {
            MobState::Wander => {
                let d = flat_dist(p.pos, self.wander_target);
                if d < 0.5 {
                    self.state = MobState::Idle;
                    acts.push(AiAction::Nothing);
                } else {
                    acts.push(self.walk_toward(p, self.wander_target, tbl.stroll_speed));
                }
            }
            MobState::Flee => {
                // flee 来源消失 → 待机。
                self.state = MobState::Idle;
            }
            _ => {
                self.state = MobState::Idle;
                if rng().is_multiple_of(WANDER_INTERVAL_AVG) {
                    self.wander_target = random_point_around(p.pos, rng);
                    self.state = MobState::Wander;
                }
                acts.push(AiAction::Nothing);
            }
        }
    }

    fn walk_toward(&self, p: &Percept, to: Vec3, speed_mult: f32) -> AiAction {
        let jump = p.blocked || self.should_leap(p, to);
        AiAction::Walk {
            dir: flat_dir(to - p.pos),
            speed_mult,
            jump,
        }
    }

    fn walk_away(&self, p: &Percept, from: Vec3) -> AiAction {
        AiAction::Walk {
            dir: flat_dir(p.pos - from),
            speed_mult: 1.0,
            jump: p.blocked,
        }
    }

    /// spider LeapAtTargetGoal(0.4)：地面且 dist²∈(4,16) 起跳
    /// （LeapAtTargetGoal.java:29；原版附加 vel×0.2 前冲，主控处理）。
    fn should_leap(&self, p: &Percept, to: Vec3) -> bool {
        self.leap_enabled && p.target.is_some() && {
            let d2 = flat_dist2(p.pos, to);
            d2 > LEAP_MIN_DIST * LEAP_MIN_DIST && d2 < LEAP_MAX_DIST * LEAP_MAX_DIST
        }
    }
}

// ---- 小工具 -----------------------------------------------------------------

fn flat_dir(v: Vec3) -> Vec3 {
    let mut f = v;
    f.y = 0.0;
    normalize_or_zero(f)
}

fn flat_perp(v: Vec3) -> Vec3 {
    flat_dir(Vec3::new(-v.z, 0.0, v.x))
}

fn normalize_or_zero(v: Vec3) -> Vec3 {
    let len = v.length();
    if len < 1e-6 {
        Vec3::ZERO
    } else {
        v / len
    }
}

fn flat_dist(a: Vec3, b: Vec3) -> f32 {
    flat_dist2(a, b).sqrt()
}

fn flat_dist2(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    dx * dx + dz * dz
}

/// 游走目标点：水平 2..10 格（DefaultRandomPos 排除贴身处）、垂直 ±2（简化，原版 ±7）。
fn random_point_around(pos: Vec3, rng: &mut impl FnMut() -> u32) -> Vec3 {
    let r1 = rng() as f32 / u32::MAX as f32;
    let r2 = rng() as f32 / u32::MAX as f32;
    let r3 = rng() as f32 / u32::MAX as f32;
    let ang = r1 * std::f32::consts::TAU;
    let d = 2.0 + r2 * (WANDER_RADIUS - 2.0);
    pos + Vec3::new(ang.cos() * d, (r3 * 2.0 - 1.0) * 2.0, ang.sin() * d)
}
