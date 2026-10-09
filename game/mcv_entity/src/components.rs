//! Mob 的 ECS 组件拆分（结构迁移自原单体 `Mob`，数值/AI 逻辑不变）：
//! 每只怪 = `MobKind` + `PhysBody` + `Health` + `Yaw` + `MobTicks` + `LastHurt`
//! + `MobBrain`（20 Hz 状态机，[`crate::ai::Brain`]）+ `MobIntent`（上一 tick
//! 的移动指令，供 60 Hz 物理步进复用）。箭矢 = `MobArrow`（简化投射物）。

use crate::ai::Brain;
use crate::defs::MobId;
use glam::Vec3;
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
/// [`crate::ai::AiAction::Shoot`] 经命令队列生成，`arrow_system` 每 tick
/// 以 0.2 子步推进（防高速隧穿），撞方块 / 命中玩家 / ttl 耗尽即移除。
/// 原版 `gravity=0.05/tick²`、`INERTIA=0.99`（AbstractArrow.java:59）。
#[derive(Clone, Copy, Debug)]
pub struct MobArrow {
    pub pos: Vec3,
    pub vel: Vec3,
    /// 剩余寿命（tick），到点移除（原版 livedAfterGround 兜底）。
    pub ttl_ticks: u32,
    /// 命中玩家造成的基础伤害（base=power×2.0=2.0，AbstractArrow.java:719；
    /// 原版还乘当前速度 ceil(v×base)（:423-432），此处取定值 → 近似）。
    pub damage: f32,
}

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
    world.register::<MobArrow>();
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
    e
}
