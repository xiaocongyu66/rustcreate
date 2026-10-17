//! Mob 的 ECS 组件拆分（结构迁移自原单体 `Mob`，数值/AI 逻辑不变）：
//! 每只怪 = `MobKind` + `PhysBody` + `Health` + `Yaw` + `MobTicks` +
//! `LastHurt`，再加 `MobBrain`（20 Hz 状态机，[`crate::ai::Brain`]）与
//! `MobIntent`（上一 tick 的移动指令，供 60 Hz 物理步进复用）。
//! 箭矢 = `MobArrow`（简化投射物）。

use crate::ai::Brain;
use crate::defs::MobId;
use crate::passive::AnimState;
use glam::Vec3;
use mcv_core::BlockPos;
use mcv_ecs::{Entity, World};

/// 怪物种类（查 [`MobId::def`] 取数值表）。
#[derive(Clone, Copy, Debug)]
pub struct MobKind(pub MobId);

/// 物理刚体：对应 `mcv_game::Entity{pos,vel,on_ground}`（引擎类型非 Copy，
/// 游戏层包一层；进出用 [`PhysBody::body`] / [`PhysBody::into_body`]）。
#[derive(Clone, Copy, Debug)]
pub struct PhysBody {
    pub pos: Vec3,
    pub vel: Vec3,
    pub on_ground: bool,
    /// 上一固定步的位置（渲染插值基准，26.1 `xOld/yOld/zOld` 等价）：每个
    /// 60 Hz 物理步开头由系统拍快照，渲染帧用 `pos_old→pos` 按 partialTick
    /// 残差插值（26.1 LivingEntity render pos = lerp(xOld, x, partialTick)）。
    /// 生成时与 `pos` 同值（首步零差值，不跳）。
    pub pos_old: Vec3,
}

impl PhysBody {
    /// 借出引擎物理体型（喂 [`mcv_game::step_entity`]）。
    pub fn body(&self) -> mcv_game::Entity {
        mcv_game::Entity {
            pos: self.pos,
            vel: self.vel,
            on_ground: self.on_ground,
        }
    }

    /// 物理步进结果写回。
    pub fn set_body(&mut self, b: &mcv_game::Entity) {
        self.pos = b.pos;
        self.vel = b.vel;
        self.on_ground = b.on_ground;
    }
}

/// 生命值（≤0 由 AI tick 末尾统一 despawn，语义同原 retain 清理）。
#[derive(Clone, Copy, Debug)]
pub struct Health(pub f32);

/// 朝向（游走/追击航向角，弧度）。
#[derive(Clone, Copy, Debug)]
pub struct Yaw(pub f32);

/// 计时器打包（**全部 20 tick/s 语义**，只在 on_tick 边界推进，禁止按
/// 60 Hz 步计数——审计 C-1）。原版对应三套独立时钟，此前近战冷却与受击
/// i-frame 共用 `invulnerable` 一字段（审计 M-12），现已拆开：攻击节拍归
/// [`crate::ai::Brain::attack_cd`]。
#[derive(Clone, Copy, Debug)]
pub struct MobTicks {
    /// 受击无敌帧（LivingEntity.java:1217 `invulnerableTime=20`，1 tick 减 1）。
    pub invulnerable: u32,
    /// 无行动 tick（Mob.java:683 `noActionTime++`；Monster.java:51-54 亮处
    /// +2；Mob.java:673 距玩家 <32² 清零）——消散账本。
    pub idle_ticks: u64,
    /// 本 tick 被玩家命中（HurtByTargetGoal 索敌输入，AI tick 消费后清除）。
    pub hurt_flag: bool,
    /// 燃烧剩余 tick（Entity.java:253 `remainingFireTicks`；点燃 8s=160t，
    /// Entity.java:538-540 每 20 tick 扣 1 血）。
    pub fire_ticks: u32,
}

/// 上次受击伤害（i-frame 差值结算基准，见 [`crate::combat::apply_hurt`]）。
#[derive(Clone, Copy, Debug)]
pub struct LastHurt(pub f32);

/// 每只怪一份的 AI 状态机（[`crate::ai::Brain`]）：creeper 引信、skeleton
/// 弓战冷却、目标记忆 60 tick 等**按 20 tick/s 推进**的运行时全在这里。
/// 系统只在 `on_tick` 为真时调 [`Brain::tick`]（一次 tick 一步），非 tick
/// 边界沿用上一 tick 的 [`MobIntent`]。
#[derive(Clone, Debug)]
pub struct MobBrain(pub Brain);

/// 上一 AI tick 产出的移动意图，60 Hz 物理每步照此步进（AI 决策 20 Hz、
/// 物理 60 Hz 的换算面）。`wish` 已含速度倍率（m/s）。
#[derive(Clone, Copy, Debug)]
pub struct MobIntent {
    pub wish: Vec3,
    pub jump: bool,
}

impl MobIntent {
    /// 静止（生成后的初始意图）。
    pub const IDLE: MobIntent = MobIntent {
        wish: Vec3::ZERO,
        jump: false,
    };
}

/// 简化箭矢（AbstractArrow 直线 + 重力近似）：由 skeleton 的
/// [`crate::ai::AiAction::Shoot`] 与玩家弓（BowItem.java:41 pow×3.0）生成，
/// `arrow_system` 每 tick 以 0.2 子步推进（防高速隧穿），撞方块 / 命中
/// 玩家 / ttl 耗尽即移除。原版 `gravity=0.05/tick²`、`INERTIA=0.99`
/// （AbstractArrow.java:59）。
#[derive(Clone, Copy, Debug)]
pub struct MobArrow {
    pub pos: Vec3,
    pub vel: Vec3,
    /// 剩余寿命（tick），到点移除（原版 livedAfterGround 兜底）。
    pub ttl_ticks: u32,
    /// baseDamage（AbstractArrow.java:622 默认 2.0；怪射 = power×2.0 +
    /// triangle(难度×0.11, 0.57425)，AbstractArrow.java:718-720）。命中量
    /// = ceil(命中瞬间速度模长 × base)（:421-431）。
    pub base_damage: f32,
    /// 满蓄力弓箭（pow==1，BowItem.java:41）→ 暴击加成
    /// `+ rand(damage/2+2)`（AbstractArrow.java:434-437）。
    pub crit: bool,
    /// true = 玩家射出（命中 mob 并结算；false = 怪射，命中玩家）。
    pub player_owned: bool,
}

/// 小火球（SmallFireball；源 = BlazeAttackGoal 发射，M7d）：无重力的
/// 加速投射物（AbstractHurtingProjectile.java:22-24,:102-127）——
/// `v += v̂·0.1; v ×= 0.95`（Fireball 默认惯性，稳态 1.9 格/tick），
/// 撞方块/命中玩家/寿命耗尽即移除。命中：5.0 伤害 + 点燃 5s
/// （SmallFireball.java:38-44，主控接线伤害与玩家 fire_ticks）。
/// TTL 兜底 100t（原版不老化，主控防泄漏面）。
#[derive(Clone, Copy, Debug)]
pub struct MobFireball {
    pub pos: Vec3,
    pub vel: Vec3,
    pub ttl_ticks: u32,
}

/// A* 路径账本（审计 N-1 接线）：`nodes` 为起→终航点、`idx` 游标、
/// `recalc_cd` 重算倒计时（MeleeAttackGoal.java:100-124 节奏）、
/// `pathed_target` 上次寻路的目标点（移动 ≥1 格才触发重算，:104-107）。
#[derive(Clone, Debug)]
pub struct MobPath {
    pub nodes: Vec<BlockPos>,
    pub idx: usize,
    pub recalc_cd: u32,
    pub pathed_target: Vec3,
}

impl Default for MobPath {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            idx: 0,
            recalc_cd: 0,
            pathed_target: Vec3::ZERO,
        }
    }
}

/// 活动状态效果账本（药水/信标等效果源的落点；本仓暂无施加方，
/// 组件先行占位——效果机制见 [`crate::effect::EffectBook`]）。
#[derive(Default)]
pub struct Effects(pub crate::effect::EffectBook);

/// 预注册全部 mob 组件表：`GameRuntime` 建 World 后调一次——零怪时
/// AI tick 仍会 `read::<MobKind>()` 等视图，未注册类型 read 会 panic。
pub fn register_mob_components(world: &mut World) {
    world.register::<MobKind>();
    world.register::<PhysBody>();
    world.register::<Health>();
    world.register::<Yaw>();
    world.register::<MobTicks>();
    world.register::<LastHurt>();
    world.register::<MobBrain>();
    world.register::<MobIntent>();
    world.register::<MobPath>();
    world.register::<MobArrow>();
    world.register::<MobFireball>();
    world.register::<Effects>();
    // 行走动画账本（渲染拼装消费 AnimState.phase/amount → mob_mesh 腿摆）。
    // register 幂等（ensure_store 只建空表）：register_passive_components 的
    // 二次注册无冲突。此前敌对怪不插不推进此组件 → mob_render 走零相位
    // 静止路径（僵尸走路腿不摆的根因，随 spawn_mob 统一装配解除）。
    world.register::<AnimState>();
}

/// 集中装配一只怪（原 `Mob::new` 的组件化等价），避免散点漏插。
pub fn spawn_mob(world: &mut World, id: MobId, pos: Vec3) -> Entity {
    let e = world.spawn();
    world.insert(e, MobKind(id));
    world.insert(
        e,
        PhysBody {
            pos,
            vel: Vec3::ZERO,
            on_ground: false,
            pos_old: pos,
        },
    );
    world.insert(e, Health(id.def().health));
    world.insert(e, Yaw(0.0));
    world.insert(
        e,
        MobTicks {
            invulnerable: 0,
            idle_ticks: 0,
            hurt_flag: false,
            fire_ticks: 0,
        },
    );
    world.insert(e, LastHurt(0.0));
    world.insert(e, MobBrain(Brain::new()));
    world.insert(e, MobIntent::IDLE);
    world.insert(e, MobPath::default());
    world.insert(e, Effects::default());
    // 行走动画账本随装配统一给出（敌对/被动同面）：mob_ai_system 每个 60 Hz
    // 物理步推进（advance_walk_anim），渲染拼装 collect_mob_instances_in 消费。
    world.insert(e, AnimState::default());
    e
}
