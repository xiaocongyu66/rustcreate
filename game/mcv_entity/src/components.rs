//! Mob 的 ECS 组件拆分（结构迁移自原单体 `Mob`，数值/AI 逻辑不变）：
//! 每只怪 = `MobKind` + `PhysBody` + `Health` + `Yaw` + `MobTicks` + `LastHurt`。
//! 原 `burning`（全仓无读取方）与 `brain`（game 层 AI 为内联 chase/wander，
//! [`crate::ai::Brain`] 无运行时消费方）不进 ECS。

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

/// 计时器打包：受击无敌帧（近战冷却复用，20 tick = 1s）+ 闲置 tick（游走抖动）。
#[derive(Clone, Copy, Debug)]
pub struct MobTicks {
    /// Ticks since last attacked (invulnerability frames, 20 = 1s)。
    pub invulnerable: u32,
    /// Ticks idle (despawn accounting)。
    pub idle_ticks: u64,
}

/// 上次受击伤害（i-frame 差值结算基准，见 [`crate::combat::apply_hurt`]）。
#[derive(Clone, Copy, Debug)]
pub struct LastHurt(pub f32);

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
        },
    );
    world.insert(e, LastHurt(0.0));
    e
}
