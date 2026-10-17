//! 敌对生物 AI 状态机（MC 26.1 Goal 体系的等价简化，参数见 mc-ref/NOTES-mobs.md）。
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
//!   bow:   skeleton      ─ Ranged ─(15 格横移半径; >13 接近; <7.5 后撤; 横移;
//!                                    拉弓 20t 满力放箭 + interval(40/20) 冷却,
//!                                    周期 60/40t; 断视线 60t 收弓)
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
use mcv_core::BlockPos;

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
/// Mob.java:494 — igniteForSeconds(8)；Entity.java:630-632 floor(8×20)=160 tick。
pub const BURN_TICKS: u32 = 160;

// ---- M7d 敌对三怪（烈焰人/末影人/嘎吱怪）常量 -------------------------------

/// BlazeAttackGoal.java:202 — 贴脸终止 volley（distanceToSqr < 4 → dist < 2），
/// 近战 doHurtTarget 间隔 attackTime=20（:207-209）。
pub const BLAZE_MELEE_DIST: f32 = 2.0;
pub const BLAZE_MELEE_INTERVAL: u32 = 20;
/// BlazeAttackGoal.java:219-228 — volley 节拍：step1 充能 60t；step2..4 每
/// 6t 发一枚小火球；step≥5 冷却 100t 归零撤充能。
pub const BLAZE_CHARGE_TICKS: u32 = 60;
pub const BLAZE_BURST_TICKS: u32 = 6;
pub const BLAZE_COOLDOWN_TICKS: u32 = 100;
/// BlazeAttackGoal.java:230-234 — 只有 step>1 的各次（=3 发）伴随冲击声。
pub const BLAZE_VOLLEY_SHOTS: u32 = 3;
/// Blaze.java:29-31,:113-124 — 浮空账本：每 100t 换目标高差
/// allowedHeightOffset = triangle(0.5, 6.891)；目标眼高于自身眼+offset →
/// vel.y += (0.3 − vel.y)·0.3（:126-131）。
pub const BLAZE_HOVER_REFRESH: u32 = 100;
pub const BLAZE_HOVER_BASE: f32 = 0.5;
pub const BLAZE_HOVER_DEV: f32 = 6.891;
/// Blaze.java:85-88 — 离地下落空气阻尼：!onGround && vel.y<0 → vel.y ×0.6。
pub const BLAZE_FALL_DRAG: f32 = 0.6;
/// AbstractHurtingProjectile.java:22-24,:102-127 — 小火球初速 =
/// dir·0.1（assignDirectionalMovement(accelerationPower=0.1)），每 tick
/// `v = (v + v̂·0.1)·inertia`（inertia 0.95，Fireball 默认惯性）→ 稳态 1.9。
pub const FIREBALL_INITIAL_SPEED: f32 = 0.1;
pub const FIREBALL_ACCEL: f32 = 0.1;
pub const FIREBALL_INERTIA: f32 = 0.95;
/// SmallFireball.java:38-44 — 命中实体 igniteForSeconds(5)=100t + 5.0 伤害。
pub const FIREBALL_DAMAGE: f32 = 5.0;
pub const FIREBALL_FIRE_TICKS: u32 = 100;

/// EnderMan.java:70-72 — 仇恨移速修正 ADD_VALUE +0.15（0.3 → 0.45）。
pub const ENDERMAN_ANGER_SPEED_ADDED: f32 = 0.15;
/// EndermanLookForPlayerGoal.java:508-509 — 凝视确认延迟 aggroTime=5t。
pub const ENDERMAN_STARE_AGGR_TICKS: u32 = 5;
/// :558-561 — 凝视中 dist²<16（dist<4）→ 每次凝视 tick 瞬移离开。
pub const ENDERMAN_STARED_TP_DIST: f32 = 4.0;
/// :564-566 — dist²>256 且 teleportTime≥30 → teleportTowards（16 格背向）。
pub const ENDERMAN_CHASE_TP_DIST: f32 = 16.0;
pub const ENDERMAN_CHASE_TP_TICKS: u32 = 30;
/// EnderMan.java:245-251 — 白天高原 600t 门（targetChangeTime+600）+
/// burnUndead 同式曝光概率 → 丢目标 + teleport()。
pub const ENDERMAN_SUN_ESCAPE_TICKS: u32 = 600;
/// EnderMan.teleport():256-261 — x/z 各 (rand−0.5)·64（±32），y next(64)−32。
pub const TP_RAND_RANGE_XZ: f32 = 64.0;
pub const TP_RAND_RANGE_Y: u32 = 64;
/// teleportTowards():267-274 — `−dir·16`（dir = this−threat 归一 → 落点
/// **指向 threat** 16 格）+ x/z ±4 抖动、y ±8（teleportTime≥30 且 dist²>256
/// 的"对峙逼近"支）。
pub const TP_TOWARD_DIST: f32 = 16.0;
pub const TP_TOWARD_JITTER: f32 = 8.0;
pub const TP_TOWARD_RANGE_Y: u32 = 16;
/// LivingEntity.randomTeleport:3602-3646 + EnderMan.teleport:277-302 的
/// 落点合法性合成判据（下方 solid+2 净空+不在水）。每次瞬移的尝试上限
/// （原版 hurtServer 弹射分支 :368-372 上限 64 次；本仓任务拍板 8 次/次）。
pub const TELEPORT_ATTEMPTS: u32 = 8;
/// LivingEntity.isLookingAtMe:1723-1742 — 凝视锥体：dot > 1 − cone/dist。
/// enderman cone=0.025（EnderMan.java:210，按距离衰减）；creaking
/// cone=0.5（Creaking.java:466，不衰减 → 恒 dot>0.5）。
pub const ENDERMAN_STARE_CONE: f64 = 0.025;
pub const CREAKING_STARE_CONE: f64 = 0.5;

/// Creaking.java:67-68 — 激活半径 dist²<144（12 格）。
pub const CREAKING_ACTIVATE_DIST: f32 = 12.0;
/// Creaking.java:68 — 攻击间隔 40t。
pub const CREAKING_ATTACK_INTERVAL: u32 = 40;
/// Creaking.java:70 — 闲游速度乘子（CreakingAi idle walk balance 0.3）。
pub const CREAKING_IDLE_SPEED_MULT: f32 = 0.3;
/// EndermanTakeBlockGoal.java:590 — canUse nextInt(reducedTickDelay(20))
/// == 0 → 每 tick 1/10（reducedTickDelay = ceil(n/2)）。
pub const TAKE_BLOCK_INTERVAL: u32 = 10;
/// EndermanLeaveBlockGoal.java:446 — nextInt(reducedTickDelay(2000)) == 0
/// → 1/1000。
pub const LEAVE_BLOCK_INTERVAL: u32 = 1000;

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
    /// 对雨/水敏感（Blaze.java:113-116、EnderMan.java:238-241
    /// isSensitiveToWater → 判湿/雨打即受伤，主控接线）。
    pub water_hurt: bool,
    /// 近战间隔覆写（creaking=40，Creaking.java:68 ATTACK_INTERVAL；0 = 用 melee_interval）。
    pub melee_interval_override: u32,
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
            water_hurt: false,
            melee_interval_override: 0,
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
            water_hurt: false,
            melee_interval_override: 0,
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
            water_hurt: false,
            melee_interval_override: 0,
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
            water_hurt: false,
            melee_interval_override: 0,
        },
        // Blaze：WaterAvoidingRandomStrollGoal(1.0)（Blaze.java:47）；不烧
        // （getLightLevelDependentMagicValue 恒 1 仅入火免疫，Blaze.java:80-82）。
        MobKind::Blaze => MobAiTable {
            burn_sun: false,
            stroll_speed: 1.0,
            melee_damage: 6.0, // Attributes ATTACK 6.0（Blaze.java:55）
            melee_interval: BLAZE_MELEE_INTERVAL,
            default_bow: false,
            leap: false,
            wall_climb: false,
            avoids: false,
            water_hurt: true, // Blaze.java:113-116 isSensitiveToWater
            melee_interval_override: 0,
        },
        // EnderMan：MeleeAttackGoal(1.0)（EnderMan.java:96）；不烧（白天走
        // 瞬移逃离而非燃烧，EnderMan.java:245-251）。
        MobKind::EnderMan => MobAiTable {
            burn_sun: false,
            stroll_speed: 1.0, // EnderMan.java:97
            melee_damage: 7.0, // Attributes ATTACK 7.0（EnderMan.java:117）
            melee_interval: MELEE_ATTACK_INTERVAL,
            default_bow: false,
            leap: false,
            wall_climb: false,
            avoids: false,
            water_hurt: true, // EnderMan.java:238-241 isSensitiveToWater
            melee_interval_override: 0,
        },
        // Creaking：攻击间隔 40（Creaking.java:68 ATTACK_INTERVAL）。
        MobKind::Creaking => MobAiTable {
            burn_sun: false,
            stroll_speed: CREAKING_IDLE_SPEED_MULT, // Creaking.java:70
            melee_damage: 3.0,                      // Creaking.java:65/:131
            melee_interval: MELEE_ATTACK_INTERVAL,
            default_bow: false,
            leap: false,
            wall_climb: false,
            avoids: false,
            water_hurt: false, // 不可水杀（hurtServer 心绑免疫环境伤，Creaking.java:152-180）
            melee_interval_override: CREAKING_ATTACK_INTERVAL,
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
            water_hurt: false,
            melee_interval_override: 0,
        },
    }
}

/// 近战触达判定（Mob.isWithinMeleeAttackRange，Mob.java:1317-1328 +
/// getAttackBoundingBox :1333-1352）：自身 AABB inflate(√2.04−0.6, 0, ·)
/// 与目标 hitbox 水平相交 ⟺ 中心水平距 < 基础触达 + **攻击者半宽** + 目标
/// 半宽。垂直向原版还要 AABB 相交（同格站立近似恒真，简化，见模块头）。
pub fn melee_range(own_half_width: f32, target_half_width: f32) -> f32 {
    MELEE_REACH_BASE + own_half_width + target_half_width
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
    /// 烈焰人火球齐射（BlazeAttackGoal，60t 充能 → 3×6t 射击 → 100t 冷却）。
    Volley,
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
    /// spread = 14 − 难度×4，base_damage = 2.0×power（RangedAttackGoal.java:92-93）。
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
    /// 阳光点燃（ticks=160，Mob.java:494 → Entity.java:630-632）。
    SetOnFire {
        ticks: u32,
    },
    /// 小火球（BlazeAttackGoal.java:236-241 → SmallFireball）：dir = 归一化
    /// 指向（含 triangle 横向散布，抖动已在 Brain 内叠加）；速度/加速度/
    /// 惯性/命中伤害都是常量表（FIREBALL_*），生成与步进由主控做。
    ShootFireball {
        dir: Vec3,
    },
    /// 瞬移请求（EnderMan.teleport / teleportTowards）：away_from = Some
    /// 时按 teleportTowards 语义背向该点；None = 原地随机 ±32。候选点由
    /// 主控重抽并做落点合法性扫描（[`try_teleport`]，8 次/次）。
    Teleport {
        away_from: Option<Vec3>,
    },
    /// 拿方块（EndermanTakeBlockGoal）：pos = 自身边 ±2/y+0..+3 内一个
    /// 候选格；合法性（ENDERMAN_HOLDABLE tag + 可达）由主控校验后落笔
    /// （（成功）回写 Brain.carrying）。
    TakeBlock {
        pos: BlockPos,
    },
    /// 放下方块（EndermanLeaveBlockGoal）：pos = 自身边 ±1/y+0..+2 候选；
    /// 主控校验空气+下方实心后 setBlock 并清 Brain.carrying。
    PlaceBlock {
        pos: BlockPos,
    },
    /// 浮空/下落阻尼（Blaze.customServerAiStep:119-134 + aiStep:85-88）：
    /// lift 已按 Percept.vel_y 算好增量（(0.3−vel.y)·0.3 或 0）；fall_drag
    /// = 离地下落时 vel.y×0.6（非烈焰恒 1.0 不动作）。由主控在物理步进前
    /// 消费（写 PhysBody.vel）。
    Hover {
        lift: f32,
        fall_drag: f32,
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
    /// A* 路径下一航点（审计 N-1；主控 `mob_ai_system` 维护
    /// `MobPath` 账本）。有值时 `walk_toward` 走航点而非直线冲目标
    /// （GroundPathNavigation.moveTo 等价）；None 回落直线（旧行为，
    /// 寻路失败/不可达降级，MeleeAttackGoal.java:121-123 +15 节奏）。
    pub waypoint: Option<Vec3>,
    /// 玩家视线方向（getViewVector(1.0) 归一化）。凝视判定
    /// [`looking_at`]（LivingEntity.isLookingAtMe:1723-1742）的输入；
    /// 零向量 = 无方向（永不凝视，安全退化）。
    pub plook: Vec3,
    /// 本实体竖直速度（Blaze 浮空增量式 (0.3−vel.y)·0.3 的输入，
    /// Blaze.java:126-131）。
    pub vel_y: f32,
    /// 本实体是否着地（Blaze 下落阻尼仅在离地时生效，Blaze.java:85-88）。
    pub on_ground: bool,
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
    /// skeleton 拉弓进度（0=未拉弓；≥BOW_DRAW_TICKS 且有视线即放箭，
    /// RangedBowAttackGoal.java:123-136）。
    pub draw_ticks: u32,
    /// 横移方向（clockwise 标志，每 20t p=0.3 翻转）。
    pub strafe_clockwise: bool,
    strafe_flip_cd: u32,
    /// 本 tick 生效的 kind 级 leap 标志（Brain::tick 据 ai_table 注入）。
    leap_enabled: bool,
    /// 被外部点燃（打火石；Creeper.isIgnited）。
    pub ignited: bool,
    /// 已爆炸/死亡标志（Detonate 后 true，调用方负责移除实体）。
    pub exploded: bool,
    // ---- M7d 三怪运行时 -----------------------------------------------------
    /// 烈焰人 volley 段位（BlazeAttackGoal.attackStep，0..5 循环）。
    pub volley_step: u32,
    /// 烈焰人充能中（DATA_FLAGS_ID bit0，渲染火层/粒子面）。
    pub charged: bool,
    /// 烈焰人浮空目标高差（Blaze.allowedHeightOffset，每 100t 换
    /// triangle(0.5, 6.891)）。
    pub hover_offset: f32,
    /// 烈焰人高差刷新倒计时（nextHeightOffsetChangeTick）。
    pub hover_cd: u32,
    /// 末影人凝视确认倒计时（EndermanLookForPlayerGoal.aggroTime，5t）。
    pub stare_pending: u32,
    /// 末影人 teleportTime（背向瞬移节流 30t，EnderMan.java:563-566）。
    pub tp_cd: u32,
    /// 索敌持续 tick（targetChangeTime 差的白昼逃离门，≥600 才判曝光）。
    pub target_held: u32,
    /// 是否携带方块（DATA_CARRY_STATE 非 None 的 bool 化；校验与回写在
    /// 主控 TakeBlock/PlaceBlock 分支）。
    pub carrying: bool,
    /// 携带方块 id（0 = 空；主控校验成功时落笔，渲染/放置取同一 id）。
    pub carried: u16,
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
            draw_ticks: 0,
            strafe_clockwise: false,
            strafe_flip_cd: 0,
            leap_enabled: false,
            ignited: false,
            exploded: false,
            volley_step: 0,
            charged: false,
            hover_offset: 0.5,
            hover_cd: 0,
            stare_pending: 0,
            tp_cd: 0,
            target_held: 0,
            carrying: false,
            carried: 0,
        }
    }

    pub fn ignite(&mut self) {
        self.ignited = true;
    }

    fn clear_target(&mut self) {
        self.has_target = false;
        self.unseen = 0;
        self.los_ticks = 0;
        self.draw_ticks = 0; // 目标丢失 = goal.stop() → stopUsingItem（:56-62）
        self.stare_pending = 0;
        self.tp_cd = 0;
        self.target_held = 0;
        self.volley_step = 0;
        self.charged = false;
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

        // 0.5) 烈焰人浮空/落阻（Blaze.customServerAiStep + aiStep：高差
        // 追升只在有目标时；落阻离地恒发）。主控消费 Hover 写 vel。
        if def.kind == MobKind::Blaze {
            let lift = if self.has_target
                && let Some(t) = p.target
            {
                // 目标眼位 > 自身眼位 + allowedHeightOffset → 追升。
                let eye_self = p.pos.y + def.half_size[1] * 1.7;
                let target_eye = t.y + p.target_half_width * 1.7;
                let cd = self.hover_cd.saturating_sub(1);
                let (cd, off) = if cd == 0 {
                    (
                        BLAZE_HOVER_REFRESH,
                        triangle(BLAZE_HOVER_BASE, BLAZE_HOVER_DEV, rng),
                    )
                } else {
                    (cd, self.hover_offset)
                };
                self.hover_cd = cd;
                self.hover_offset = off;
                if target_eye > eye_self + off {
                    (0.3 - p.vel_y) * 0.3
                } else {
                    0.0
                }
            } else {
                self.hover_cd = self.hover_cd.saturating_sub(1);
                0.0
            };
            let drag = if !p.on_ground && p.vel_y < 0.0 {
                BLAZE_FALL_DRAG
            } else {
                1.0
            };
            acts.push(AiAction::Hover {
                lift,
                fall_drag: drag,
            });
        }

        // 1) 亡灵阳光燃烧（Mob.java:499-513，26.1 曲线版）。末影人白天
        // 不烧——走目标持有 600t 后的瞬移逃离（EnderMan.java:245-251，
        // 在 enderman_life 内判）。
        if tbl.burn_sun && burn_in_daylight(p.sky_exposed, p.sheltered_from_sun(), p.br, p.day, rng)
        {
            acts.push(AiAction::SetOnFire { ticks: BURN_TICKS });
        }

        // 2) 目标获取 / 维持 / 丢失（TargetGoal + NearestAttackableTargetGoal）。
        // enderman/creaking 索敌凝视门控（EndermanLookForPlayerGoal /
        // Creaking.checkCanMove：靠近不索敌，须被凝视）——见到/受击反击
        // 分支在下方拆分。
        let dist = p.target.map_or(f32::INFINITY, |t| p.pos.distance(t));
        // spider 只在暗处索敌（Spider.java:225-229）；creeper 不追山羊(忽略)。
        let kind_ok = def.kind != MobKind::Spider || p.br < 0.5;
        // 凝视门控怪：HurtByTargetGoal 仍即时反索（targetSelector(1)/:2）。
        let gaze_locked = matches!(def.kind, MobKind::EnderMan | MobKind::Creaking);
        let hurt_back = p.hurt && p.target.is_some() && dist <= def.follow_range;
        if !self.has_target {
            let sees = p.target.is_some() && dist <= def.follow_range && p.los && kind_ok;
            if (!gaze_locked && sees) || hurt_back {
                self.has_target = true;
                self.unseen = 0;
                self.los_ticks = 0;
                self.target_held = 0;
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
            self.target_held = self.target_held.saturating_add(1);
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
            MobKind::Blaze => {
                self.blaze_life(def, &tbl, p, dist, rng, &mut acts);
            }
            MobKind::EnderMan => {
                self.enderman_life(def, &tbl, p, dist, rng, &mut acts);
            }
            MobKind::Creaking => {
                self.creaking_life(def, &tbl, p, dist, rng, &mut acts);
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
        if !self.has_target {
            self.wander_only(def, tbl, p, rng, acts);
            return;
        }
        let range = melee_range(def.half_size[0], p.target_half_width);
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
                    // MeleeAttackGoal.java:127-144：命中后重置 20 tick
                    //（creaking 用 ATTACK_INTERVAL=40 覆写，Creaking.java:68）。
                    acts.push(AiAction::MeleeHit {
                        damage: tbl.melee_damage,
                    });
                    self.attack_cd = if tbl.melee_interval_override > 0 {
                        tbl.melee_interval_override
                    } else {
                        tbl.melee_interval
                    };
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
        if p.day
            && p.sky_exposed
            && let Some(s) = p.shelter
        {
            self.draw_ticks = 0; // 目标切换 = stopUsingItem（FleeSunGoal 抢占）
            self.state = MobState::Flee;
            acts.push(self.walk_toward(p, s, 1.0));
            return;
        }
        // AvoidEntityGoal(Wolf, 6.0)（AbstractSkeleton.java:79）。
        if tbl.avoids
            && let Some(a) = p.avoid
            && p.pos.distance(a) < AVOID_DIST
        {
            self.draw_ticks = 0;
            self.state = MobState::Flee;
            acts.push(self.walk_away(p, a));
            return;
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
        // 拉弓/放箭（RangedBowAttackGoal.java:123-136，逐态镜像）：
        // - 拉弓中：pullTime ≥ 20 且本 tick 有视线 → 放箭（:126-132；
        //   release **无射程门**——15² 半径只门横移停走 :86-92）；
        // - 未拉弓：attackTime 归零 → startUsingItem（:134-136）；
        // - 放箭后 attackTime = attackIntervalMin（:131，难度门
        //   AbstractSkeleton.java:138-141/151-157：hard 20 / 其余 40）。
        if self.draw_ticks > 0 {
            if p.los && self.draw_ticks >= BOW_DRAW_TICKS {
                let h = flat_dist(p.pos, t);
                let mut dir = t - p.pos;
                dir.y += h * 0.2; // AbstractSkeleton.java:164-170 抛物线补偿
                acts.push(AiAction::Shoot {
                    dir: normalize_or_zero(dir),
                    speed: ARROW_SPEED,
                    spread: arrow_spread(p.difficulty),
                    // power = BowItem.getPowerForTime(pullTime=20) = 1.0
                    // （BowItem.java:74-82）；base = power×2.0
                    // （AbstractArrow.java:718-720，难度噪声由主控叠加）。
                    base_damage: 2.0,
                });
                self.draw_ticks = 0;
                self.attack_cd = if p.difficulty >= 3 {
                    BOW_INTERVAL_HARD
                } else {
                    BOW_INTERVAL_NORMAL
                };
            } else if !p.los && self.unseen > LOSE_SIGHT_TICKS {
                // 断视线 60t → 收弓（:124-125 seeTime < −60 stopUsingItem）。
                self.draw_ticks = 0;
            } else {
                self.draw_ticks += 1;
            }
        } else {
            self.attack_cd = self.attack_cd.saturating_sub(1);
            if self.attack_cd == 0 {
                // 冷却归零即起拉（本 tick = startUsingItem；pullTime 从
                // 本 tick 起算，满 20 后下一 tick 放箭）。
                self.draw_ticks = 1;
            }
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
        if tbl.avoids
            && let Some(a) = p.avoid
            && p.pos.distance(a) < AVOID_DIST
        {
            self.state = MobState::Flee;
            acts.push(self.walk_away(p, a));
            return;
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

    // ---- 烈焰人（BlazeAttackGoal：贴身近战 / volley 齐射 / 浮空） ------------

    fn blaze_life(
        &mut self,
        def: &MobDef,
        tbl: &MobAiTable,
        p: &Percept,
        dist: f32,
        rng: &mut impl FnMut() -> u32,
        acts: &mut Vec<AiAction>,
    ) {
        if !self.has_target {
            self.wander_only(def, tbl, p, rng, acts);
            return;
        }
        let t = p.target.unwrap_or(p.pos);
        if dist < BLAZE_MELEE_DIST {
            // BlazeAttackGoal.java:202-212 — dist²<4 且有视线 → 20t 近战 +
            // setWantedPosition 贴身（移动门在 los）。
            self.state = MobState::Melee;
            self.volley_step = 0;
            self.charged = false;
            acts.push(AiAction::Look(t));
            if p.los {
                self.attack_cd = self.attack_cd.saturating_sub(1);
                if self.attack_cd == 0 {
                    acts.push(AiAction::MeleeHit {
                        damage: tbl.melee_damage,
                    });
                    self.attack_cd = tbl.melee_interval;
                }
                acts.push(self.walk_toward(p, t, 1.0));
            }
            return;
        }
        // :213 — `distanceToSqr < 48²`（FollowDistance 平方门；本仓 dist 是
        // 真距，约掉平方 → dist < 48 = follow_range）。
        if dist < def.follow_range && p.los {
            // :213-245 — 齐射段：节拍由 attackTime（attack_cd）驱动。
            self.state = MobState::Volley;
            acts.push(AiAction::Look(t));
            self.attack_cd = self.attack_cd.saturating_sub(1);
            if self.attack_cd == 0 {
                self.volley_step += 1;
                if self.volley_step == 1 {
                    // 充能亮相（DATA_FLAGS bit1——渲染火层）。
                    self.attack_cd = BLAZE_CHARGE_TICKS;
                    self.charged = true;
                } else if self.volley_step <= 1 + BLAZE_VOLLEY_SHOTS {
                    self.attack_cd = BLAZE_BURST_TICKS;
                    // :230-241 — `if (this.attackStep > 1)`：发射与充能亮相
                    // 同门包裹（step 2/3/4 各一枚 = BLAZE_VOLLEY_SHOTS 发）。
                    if self.volley_step > 1 {
                        let yd = (t.y + def.half_size[1]) - (p.pos.y + def.half_size[1]);
                        let mut dir = Vec3::new(t.x - p.pos.x, yd, t.z - p.pos.z);
                        // :231-237 — 三角散布：dev = 2.297·√dist ×0.5（原版
                        // distance 是平方距，两层开方 = √真距；本仓 dist 已
                        // 是真距，一层 sqrt 对齐）。
                        let sqd = dist.sqrt() * 0.5;
                        let dev = 2.297 * sqd;
                        dir.x = triangle(dir.x, dev, rng);
                        dir.z = triangle(dir.z, dev, rng);
                        acts.push(AiAction::ShootFireball {
                            dir: normalize_or_zero(dir),
                        });
                    }
                } else {
                    self.attack_cd = BLAZE_COOLDOWN_TICKS;
                    self.volley_step = 0;
                    self.charged = false;
                }
            }
        } else if self.unseen < 5 {
            // :246-248 — 刚失去视线（lastSeen<5）仍朝目标移动。
            acts.push(self.walk_toward(p, t, 1.0));
        }
    }

    // ---- 末影人（凝视激怒 + 瞬移 + 拿/放方块） -------------------------------

    fn enderman_life(
        &mut self,
        def: &MobDef,
        tbl: &MobAiTable,
        p: &Percept,
        dist: f32,
        rng: &mut impl FnMut() -> u32,
        acts: &mut Vec<AiAction>,
    ) {
        // 白天高原逃离（EnderMan.java:244-254）：目标持有 ≥600t 且曝光 →
        // 丢目标 + teleport()（burnUndead 同式概率）。
        if self.has_target
            && self.target_held >= ENDERMAN_SUN_ESCAPE_TICKS
            && p.day
            && p.sky_exposed
            && burn_in_daylight(p.sky_exposed, false, p.br, p.day, rng)
        {
            self.clear_target();
            acts.push(AiAction::Teleport { away_from: None });
            return;
        }
        let Some(t) = p.target else {
            self.wander_only(def, tbl, p, rng, acts);
            return;
        };
        // 凝视判定（EnderMan.java:209-211 isBeingStaredBy → cone 0.025/dist，
        // 冻结 Goal canUse 限 dist²≤256）：gaze 高 = 自身眼位，观测源 = 玩家眼。
        let stared = dist * dist <= 256.0
            && looking_at(
                p.plook,
                t + Vec3::Y * mcv_game::Player::EYE,
                p.pos,
                p.pos.y + def.half_size[1] * 1.7,
                p.los,
                ENDERMAN_STARE_CONE,
                true,
            );
        if !self.has_target {
            // canUse：被凝视即挂 pendingTarget，5t 确认激怒
            //（EndermanLookForPlayerGoal.start:508-512）。
            if stared {
                self.stare_pending += 1;
                if self.stare_pending >= ENDERMAN_STARE_AGGR_TICKS {
                    self.stare_pending = 0;
                    self.has_target = true;
                    self.state = MobState::Seek;
                }
            } else {
                self.stare_pending = 0; // canContinueToUse 失败 → 归零重计。
            }
            // 拿方块（EndermanTakeBlockGoal：p=1/10（reducedTickDelay(20)=10），
            // 不持物才可）。合法性与回写在主控校验（ENDERMAN_HOLDABLE 近似
            // + 落笔 world）——Brain 只发请求（乐观置位会与校验失败形成
            // 双写，见 game.rs TakeBlock 分支）。
            if !self.carrying
                && rng().is_multiple_of(TAKE_BLOCK_INTERVAL)
                && let Some(pos) = take_block_candidate(p.pos, rng)
            {
                acts.push(AiAction::TakeBlock { pos });
            }
            // 放下方块（EndermanLeaveBlockGoal：p=1/1000，持物才可）；放点
            // 空气+支撑校验在主控。
            if self.carrying
                && rng().is_multiple_of(LEAVE_BLOCK_INTERVAL)
                && let Some(pos) = leave_block_candidate(p.pos, rng)
            {
                acts.push(AiAction::PlaceBlock { pos });
            }
            self.wander_only(def, tbl, p, rng, acts);
            return;
        }
        if stared {
            // :557-568 — 被凝视（tick 分支）：立定对视（FreezeWhenLookedAt
            // 停寻路 + lookAt）；4 格内瞬移离开并清 teleportTime。
            acts.push(AiAction::Look(t));
            if dist < ENDERMAN_STARED_TP_DIST {
                acts.push(AiAction::Teleport { away_from: None });
            }
            self.tp_cd = 0;
            // 凝视中不动（无 walk 动作 = 意图 IDLE）。
            return;
        }
        // 未被凝视teleportTime 累计（:563-566 每非凝视 tick ++）。
        self.tp_cd += 1;
        // :564-566 — 拉开 16 格以上且 30t 到点 → teleportTowards（背向目标）。
        if dist > ENDERMAN_CHASE_TP_DIST && self.tp_cd >= ENDERMAN_CHASE_TP_TICKS {
            self.tp_cd = 0;
            acts.push(AiAction::Teleport { away_from: Some(t) });
        }
        // MeleeAttackGoal(1.0) 追击近战（EnderMan.java:96）——仇恨移速
        // 修正 SPEED_MODIFIER_ATTACKING ADD_VALUE +0.15（:70-72）：
        // mult = (0.3+0.15)/0.3 = 1.5。
        let range = melee_range(def.half_size[0], p.target_half_width);
        if dist <= range {
            self.state = MobState::Melee;
            acts.push(AiAction::Look(t));
            self.attack_cd = self.attack_cd.saturating_sub(1);
            if self.attack_cd == 0 && p.los {
                acts.push(AiAction::MeleeHit {
                    damage: tbl.melee_damage,
                });
                self.attack_cd = tbl.melee_interval;
            }
        } else {
            if self.state == MobState::Melee {
                self.state = MobState::Seek;
            }
            acts.push(AiAction::Look(t));
            acts.push(self.walk_toward(
                p,
                t,
                (def.speed_attr + ENDERMAN_ANGER_SPEED_ADDED) / def.speed_attr,
            ));
        }
    }

    // ---- 嘎吱怪（凝视激活/冻结 + 节拍 40 近战） ------------------------------

    fn creaking_life(
        &mut self,
        def: &MobDef,
        tbl: &MobAiTable,
        p: &Percept,
        dist: f32,
        rng: &mut impl FnMut() -> u32,
        acts: &mut Vec<AiAction>,
    ) {
        // 冻结判定（Creaking.checkCanMove:454-481 → aiStep CAN_MOVE）：
        // 被凝视（cone 0.5 不衰减，gaze 三点近似取眼位，Creaking.java:466、
        // LivingEntity.isLookingAtMe:1723-1742）即定格——无移动意图、无跳跃，
        // 渲染沿上步姿态；对视仍 Look。
        let stared = p.target.is_some()
            && looking_at(
                p.plook,
                p.target.unwrap() + Vec3::Y * mcv_game::Player::EYE,
                p.pos,
                p.pos.y + def.half_size[1] * 1.7,
                p.los,
                CREAKING_STARE_CONE,
                false,
            );
        if !self.has_target {
            // 激活门（checkCanMove activate 分支）：12 格内被凝视 → 挂
            // 攻击目标（activate :490-496）；否则休眠（canMove=true 游走）。
            if stared && dist < CREAKING_ACTIVATE_DIST {
                self.has_target = true;
                self.state = MobState::Seek;
                self.unseen = 0;
                self.los_ticks = 0;
            } else {
                self.wander_only(def, tbl, p, rng, acts);
                return;
            }
        }
        let t = p.target.unwrap_or(p.pos);
        if stared {
            acts.push(AiAction::Look(t));
            return;
        }
        let range = melee_range(def.half_size[0], p.target_half_width);
        if dist <= range {
            self.state = MobState::Melee;
            acts.push(AiAction::Look(t));
            self.attack_cd = self.attack_cd.saturating_sub(1);
            if self.attack_cd == 0 && p.los {
                acts.push(AiAction::MeleeHit {
                    damage: tbl.melee_damage,
                });
                self.attack_cd = tbl.melee_interval_override.max(tbl.melee_interval);
            }
        } else {
            if self.state == MobState::Melee {
                self.state = MobState::Seek;
            }
            acts.push(AiAction::Look(t));
            acts.push(self.walk_toward(p, t, 1.0));
        }
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
            MobState::Seek | MobState::Melee | MobState::Ranged | MobState::Volley
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
        // 追击方向优先取 A* 航点（GroundPathNavigation.moveTo 等价）；
        // 无航点（寻路失败/未索敌游走）→ 直线，旧行为兜底。
        let goal = p.waypoint.unwrap_or(to);
        let jump = p.blocked || self.should_leap(p, to);
        AiAction::Walk {
            dir: flat_dir(goal - p.pos),
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

// ---- M7d 三怪公共工具（凝视锥 / 方块候选 / 瞬移落点） -----------------------

/// u32 均匀流 → [0,1) 双精度（原版 nextDouble 近似）。
fn rnd01(rng: &mut impl FnMut() -> u32) -> f64 {
    rng() as f64 / (u32::MAX as f64 + 1.0)
}

/// `LivingEntity.isLookingAtMe`（LivingEntity.java:1723-1742）镜像：
/// `dot(玩家视线, 玩家眼→gaze 点) > 1 − cone/(adjust ? dist : 1)`。
/// LOS 由调用方以 eye-to-eye 视线近似（原版逐 gaze 高度 clip，KNOWN 简化）。
#[allow(clippy::too_many_arguments)]
pub fn looking_at(
    look: Vec3,
    player_eye: Vec3,
    mob_pos: Vec3,
    gaze_y: f32,
    los: bool,
    cone: f64,
    adjust_for_dist: bool,
) -> bool {
    if !los || look == Vec3::ZERO {
        return false;
    }
    let dir = Vec3::new(
        mob_pos.x - player_eye.x,
        gaze_y - player_eye.y,
        mob_pos.z - player_eye.z,
    );
    let d = dir.length();
    if d < 1e-4 {
        return true; // 重合点视为被盯。
    }
    let dot = look.dot(dir / d) as f64;
    dot > 1.0 - cone / if adjust_for_dist { d as f64 } else { 1.0 }
}

/// EndermanTakeBlockGoal.tick（EnderMan.java:595-600）候选格：
/// x/z 各 `-2+r·4` 的 floor（覆盖 ±2）、y `+r·3` 的 floor。
fn take_block_candidate(pos: Vec3, rng: &mut impl FnMut() -> u32) -> Option<BlockPos> {
    let bx = (pos.x - 2.0 + rnd01(rng) * 4.0).floor() as i32;
    let by = (pos.y + rnd01(rng) * 3.0).floor() as i32;
    let bz = (pos.z - 2.0 + rnd01(rng) * 4.0).floor() as i32;
    Some(BlockPos::new(bx, by, bz))
}

/// EndermanLeaveBlockGoal.tick（EnderMan.java:454-456）候选格：
/// x/z 各 `-1+r·2` 的 floor、y `+r·2` 的 floor。
fn leave_block_candidate(pos: Vec3, rng: &mut impl FnMut() -> u32) -> Option<BlockPos> {
    let bx = (pos.x - 1.0 + rnd01(rng) * 2.0).floor() as i32;
    let by = (pos.y + rnd01(rng) * 2.0).floor() as i32;
    let bz = (pos.z - 1.0 + rnd01(rng) * 2.0).floor() as i32;
    Some(BlockPos::new(bx, by, bz))
}

/// EnderMan.teleport():256-261 — 随机候选：x/z 各 (r−0.5)·64、y next(64)−32。
pub fn teleport_random_target(pos: Vec3, rng: &mut impl FnMut() -> u32) -> Vec3 {
    let x = pos.x + (rnd01(rng) - 0.5) * TP_RAND_RANGE_XZ;
    let y = pos.y + (rng() % TP_RAND_RANGE_Y) as f32 - (TP_RAND_RANGE_Y / 2) as f32;
    let z = pos.z + (rnd01(rng) - 0.5) * TP_RAND_RANGE_XZ;
    Vec3::new(x, y, z)
}

/// EnderMan.teleportTowards():267-274 — 背向 threat 16 格 + x/z ±4 抖动、
/// y ±8（dir 含竖直分量：threat 用眼位的差向量归一）。
pub fn teleport_away_target(pos: Vec3, threat: Vec3, rng: &mut impl FnMut() -> u32) -> Vec3 {
    let dir = (pos - threat).normalize_or_zero();
    let x = pos.x + (rnd01(rng) - 0.5) * TP_TOWARD_JITTER - dir.x * TP_TOWARD_DIST;
    let y = pos.y + (rng() % TP_TOWARD_RANGE_Y) as f32
        - (TP_TOWARD_RANGE_Y / 2) as f32
        - dir.y * TP_TOWARD_DIST;
    let z = pos.z + (rnd01(rng) - 0.5) * TP_TOWARD_JITTER - dir.z * TP_TOWARD_DIST;
    Vec3::new(x, y, z)
}

/// 瞬移落点扫描的世界查询面（主控以 WorldView 实现）。
pub trait TeleportGround {
    /// 方块是否 blocksMotion（固体；含含水固岩）。
    fn solid(&self, p: BlockPos) -> bool;
    /// 方块是否含流体（水/岩浆）——落身判 wet 拒。
    fn liquid(&self, p: BlockPos) -> bool;
}

/// 一次瞬移落点尝试（EnderMan.teleport():277-302 + LivingEntity.randomTeleport
/// :3602-3646 合成）：从候选列向下找支撑（blocksMotion），支撑存在且
/// 落身格（支撑上方 ceil(height) 格）无碰撞无流体 → 落点 = 支撑上方
/// 底面中心。失败（无支撑/入水/卡身）返回 None，由调用方重抽（每瞬移
/// [`TELEPORT_ATTEMPTS`] 次）。
pub fn try_teleport(world: &dyn TeleportGround, candidate: Vec3, height: f32) -> Option<Vec3> {
    let bx = candidate.x.floor() as i32;
    let bz = candidate.z.floor() as i32;
    // 向下找支撑（:280-282 while !blocksMotion → DOWN；底界 = 本仓
    // WORLD_MIN_Y 0，穿底无支撑即失败）。
    let mut sy = candidate.y.floor() as i32;
    while sy >= 0 && !world.solid(BlockPos::new(bx, sy, bz)) {
        sy -= 1;
    }
    if sy < 0 {
        return None;
    }
    // 落身格 = 支撑上方 ceil(height) 格（enderman 2.9 → 3 格，blaze 2 格）；
    // randomTeleport 的 noCollision + containsAnyLiquid 双门（:3626 +
    // teleport 的 isWet :285-287）合成 = 这些格既非固体也非流体。
    let span = (height.ceil() as i32).max(1);
    for dy in 1..=span {
        let p = BlockPos::new(bx, sy + dy, bz);
        if world.solid(p) || world.liquid(p) {
            return None;
        }
    }
    Some(Vec3::new(bx as f32 + 0.5, (sy + 1) as f32, bz as f32 + 0.5))
}

/// 每次瞬移的完整尝试封装：重抽候选 ≤[`TELEPORT_ATTEMPTS`] 次（vanilla
/// hurtServer 弹射分支 :367-372 循环 telep 的重试语义，任务拍板 8 次/次）。
pub fn teleport_landing(
    world: &dyn TeleportGround,
    height: f32,
    mut draw: impl FnMut() -> Vec3,
) -> Option<Vec3> {
    for _ in 0..TELEPORT_ATTEMPTS {
        if let Some(land) = try_teleport(world, draw(), height) {
            return Some(land);
        }
    }
    None
}

// ---- 朝向（body yaw）推导 ---------------------------------------------------

/// 有界转身步长（rad / AI tick）。与 `passive.rs` 有界转身同值（≈20°/tick、
/// 平滑逼近航向）；原版 MoveControl.rotlerp 上限 90°/tick（MoveControl.java
/// :96，近乎瞬转），本仓两端共用较缓的步长作收敛上界——只是下界语义，不
/// 改判定值。
pub const BODY_TURN_STEP: f32 = 0.35;

/// 包裹到 (−π, π]（与 passive.rs 同式；集中到 ai.rs 供两路共用）。
pub fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut x = a % tau;
    if x > std::f32::consts::PI {
        x -= tau;
    } else if x <= -std::f32::consts::PI {
        x += tau;
    }
    x
}

/// 移动 body 朝向推导（26.1 与移动方向同面，LivingEntity.java:2734-2746：
/// 实际位移 sideDist>0.0025 → `walkDirection = atan2(zd,xd)·180/π − 90°`；
/// BodyRotationControl.clientTick:19-20：移动中 `yBodyRot = getYRot()`）。
/// 引擎朝向角约定 `f(yaw) = (sin yaw, 0, −cos yaw)`（与 `Camera::dir` 同型），
/// vanilla 的 atan2(zd,xd)−90° 在引擎空间镜像为 `wish.x.atan2(−wish.z)`（与
/// passive.rs 有界转身同式，yaw=0 即 −z 正面）。`wish` 水平足迹近零（站立/
/// 纯竖直位移）返回 None——不转（原版站立时 body 缓跟头 rot，RenderSide
/// 单 yaw 面不建模）。
/// 返回有界步进后的新 yaw（每 tick 至多 [`BODY_TURN_STEP`]，环回最短角）。
pub fn walk_body_yaw(current: f32, wish: Vec3, max_step: f32) -> Option<f32> {
    // 航向只看水平投影（vanilla sideDist 只含 xd/zd，LivingEntity.java:2734-2736
    // ——重力诱导的纯竖直位移不转头，水平足迹近零即视作站立不吃步长）。
    if wish.x * wish.x + wish.z * wish.z < 1e-8 {
        return None;
    }
    let target = wish.x.atan2(-wish.z);
    let diff = wrap_angle(target - current);
    Some(wrap_angle(current + diff.clamp(-max_step, max_step)))
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
    if len < 1e-6 { Vec3::ZERO } else { v / len }
}

fn flat_dist(a: Vec3, b: Vec3) -> f32 {
    flat_dist2(a, b).sqrt()
}

fn flat_dist2(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    dx * dx + dz * dz
}

/// 游走目标点（WaterAvoidingRandomStrollGoal.getPosition，任务 #131 缺口 3）：
/// 陆地分支 `LandRandomPos.getPos(mob, 10, 7)`（WaterAvoidingRandomStrollGoal
/// .java:27）→ `RandomPos.generateRandomDirection`（RandomPos.java:18-23）：
/// x/z 各 `next(2·10+1)−10` ∈ [−10,10]、y `next(2·7+1)−7` ∈ [−7,7] 的**方块
/// 偏移**，落点 = 偏移方块底面中心（atBottomCenterOf，RandomPos.java:111）。
/// LandRandomPos 的水面/障碍/稳定性筛选与 10 次择优（LandRandomPos.java:66-78、
/// RandomPos.java:96-112）依赖寻路 malus 账本——本仓无该数据面，采样后直接
/// 采用（KNOWN-DIVERGENCE：游走点可能落水/落障碍，由移动物理自行兜底）。
pub fn random_point_around(pos: Vec3, rng: &mut impl FnMut() -> u32) -> Vec3 {
    const H: u32 = 10; // 水平偏移半径（generateRandomDirection 第 1/3 参）
    const V: u32 = 7; // 垂直偏移半径（第 2 参）
    let xt = (rng() % (2 * H + 1)) as f32 - H as f32;
    let yt = (rng() % (2 * V + 1)) as f32 - V as f32;
    let zt = (rng() % (2 * H + 1)) as f32 - H as f32;
    // generateRandomPosTowardDirection（RandomPos.java:114-133）：偏移加到
    // 实体坐标后取整成方块（BlockPos.containing = floor），返回底面中心。
    let bx = (pos.x + xt).floor() as i32;
    let by = (pos.y + yt).floor() as i32;
    let bz = (pos.z + zt).floor() as i32;
    Vec3::new(bx as f32 + 0.5, by as f32, bz as f32 + 0.5)
}

#[cfg(test)]
mod walk_yaw_tests {
    use super::*;

    /// 引擎朝向约定 f(yaw)=(sin yaw, 0, −cos yaw)：+x 航向 → 目标角 π/2。
    /// vanilla 镜像核对：MoveControl.java:93 `yRotD = atan2(zd,xd)·180/π−90°`
    /// 在引擎空间（yaw 与 MC yaw 互反、z 取反后拍型同 Camera::dir）等价于
    /// `wish.x.atan2(−wish.z)`——与 passive.rs 旧式逐字一致。
    #[test]
    fn yaw_target_matches_engine_convention() {
        let east = walk_body_yaw(0.0, Vec3::new(1.0, 0.0, 0.0), 10.0).unwrap();
        assert!((east - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{east}");
        let south = walk_body_yaw(0.0, Vec3::new(0.0, 0.0, 1.0), 10.0).unwrap();
        assert!((south - std::f32::consts::PI).abs() < 1e-5, "{south}");
        // 与 f(yaw) 反推互证：目标角处单位航向 = dir。
        let dir = Vec3::new(east.sin(), 0.0, -east.cos());
        assert!((dir - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5, "{dir}");
    }

    /// 站立（wish 近零）不转——BodyRotationControl.java:24-37 站立分支走
    /// head-stable 账本（body 缓跟头），单 yaw 面不建模，显式跳过。
    #[test]
    fn zero_wish_does_not_turn() {
        for w in [Vec3::ZERO, Vec3::new(1e-5, 0.0, 1e-5)] {
            assert!(walk_body_yaw(0.7, w, BODY_TURN_STEP).is_none());
        }
        // y 分量（重力/俯仰抖动）不参与水平航向。
        assert!(walk_body_yaw(0.7, Vec3::new(0.0, -9.8, 0.0), BODY_TURN_STEP).is_none());
    }

    /// 有界逼近 + 收敛：每 tick 步长 ≤ BODY_TURN_STEP，逐步贴住目标且不
    /// 越冲（walk 的 wish 恒为单位向量，收敛终点 = 精确航向）。
    #[test]
    fn bounded_turn_converges() {
        let target_dir = Vec3::new(1.0, 0.0, 0.0); // 目标 π/2，从 0 起转
        let mut yaw = 0.0f32;
        for _ in 0..60 {
            yaw = walk_body_yaw(yaw, target_dir, BODY_TURN_STEP).unwrap();
        }
        assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{yaw}");
    }
}
