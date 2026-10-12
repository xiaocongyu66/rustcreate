//! 经验球实体（26.1 `ExperienceOrb` 最小闭环，任务 #128-3）：`XpOrb` 组件 +
//! 生成/拆分入口 + 物理/寿命/拾取系统。与掉落物同构：组件与
//! [`crate::components::PhysBody`] 组合、步进复用 [`mcv_game::step_entity`]、
//! 拾取走事件由主控结算（`takeXpDelay` 节流在 GameRuntime 侧）。
//!
//! 数值对齐（mc-ref/src-26.1 实读）：
//! - AABB 0.5³（`EntityType.EXPERIENCE_ORB .sized(0.5F, 0.5F)`，
//!   EntityType.java:467-470）→ 半尺寸 0.25；
//! - 重力 0.03 块/tick²（`getDefaultGravity` ExperienceOrb.java:97-99）；
//! - 初速 xz=(rand×0.2−0.1)×2.0、y=rand×0.2×2.0（块/tick，构造器
//!   ExperienceOrb.java:56-61）；
//! - 落地反弹 vy=+|impact|×0.4（:141-142 竖直碰撞且 fallSpeed<−gravity）；
//! - 寿命 LIFETIME=6000 tick（:32、:145-146 到期 discard）；
//! - 拾取：无球侧 pickupDelay，节流全在玩家侧 takeXpDelay（`playerTouch`
//!   :274-291 的 `takeXpDelay == 0` 门，命中置 2；Player.java:237-238 每
//!   tick −1）——每 tick 至多吸收一只；
//! - 拆分：`award` 循环按 `getExperienceValue` 桶切量（:187-196、:346-369）。
//!
//! 引擎步 1/60 s、原版 tick 1/20 s，计数一律 ×3（`STEPS_PER_TICK` 惯例）。

use glam::Vec3;
use mcv_ecs::{Entity, World};

use crate::components::PhysBody;
use crate::items::{STEPS_PER_TICK, initial_vel, player_pickup_box};

/// 经验球碰撞半尺寸（AABB 0.5³ → 0.25，EntityType.java:469 `.sized(0.5F, 0.5F)`）。
pub const XP_ORB_HALF: [f32; 3] = [0.25; 3];
/// 消失年龄（MC ExperienceOrb LIFETIME 6000 tick = 300 s，ExperienceOrb.java:32）。
pub const XP_ORB_DESPAWN_AGE: u32 = 6000 * STEPS_PER_TICK;
/// 重力缩放：0.03 块/tick²（ExperienceOrb.java:97-99）÷ 实体默认 0.08
/// （mcv_game GRAVITY 32 m/s² = 0.08 块/tick²，consts.rs:11-14）= 0.375。
pub const XP_ORB_GRAVITY_SCALE: f32 = 0.375;
/// 拾取后玩家侧节流（tick）：`player.takeXpDelay = 2`（ExperienceOrb.java:278）。
pub const TAKE_XP_DELAY_TICKS: u32 = 2;

/// 经验球组件：与 [`PhysBody`] 成对出现（物理走 step_entity，重力缩放
/// [`XP_ORB_GRAVITY_SCALE`]）。
#[derive(Clone, Copy, Debug)]
pub struct XpOrb {
    /// 经验值（原版 `DATA_VALUE`；一枚球的值恒为拆分桶值之一）。
    pub value: u16,
    /// 已存活步数（3 步 = 1 原版 tick）；≥ [`XP_ORB_DESPAWN_AGE`] 消失。
    pub age: u32,
}

/// 拾取请求事件：几何判定在系统侧，吸收/节流入账由主控结算
/// （takeXpDelay 是玩家状态，DropWorld 快照不带，语义在 GameRuntime 侧）。
#[derive(Clone, Copy, Debug)]
pub struct XpOrbPickup {
    pub e: Entity,
    pub value: u16,
}

/// 拆分桶（26.1 `getExperienceValue` ExperienceOrb.java:346-369 直译）：
/// 返回 ≤ max_value 的最大桶值，桶 ∈ {2477,1237,617,307,149,73,37,17,7,3,1}。
pub fn get_experience_value(max_value: u32) -> u32 {
    const BUCKETS: [u32; 9] = [2477, 1237, 617, 307, 149, 73, 37, 17, 7];
    for b in BUCKETS {
        if max_value >= b {
            return b;
        }
    }
    if max_value >= 3 { 3 } else { 1 }
}

/// 经验值 → 经验球实体群（26.1 `ExperienceOrb.award` :187-196 直译：循环
/// 取桶切量、逐球生成）。位置 = 方块精确中心（`popExperience` Block.java:
/// 449-453 `Vec3.atCenterOf(pos)`，无 popResource 的 ±0.25 抖动）。
pub fn award(world: &mut World, pos: Vec3, amount: u32, rng: &mut impl FnMut() -> u32) -> u32 {
    let mut rest = amount;
    let mut spawned = 0;
    while rest > 0 {
        let v = get_experience_value(rest);
        rest -= v;
        spawn_xp_orb(world, pos, v as u16, rng);
        spawned += 1;
    }
    spawned
}

/// 预注册经验球组件表。
pub fn register_xp_components(world: &mut World) {
    world.register::<XpOrb>();
    world.register::<PhysBody>();
}

/// 生成一只经验球：PhysBody(pos, 随机初速) + XpOrb（初速同掉落物入口的
/// 随机链约定；原版构造器随机幅度见模块注释）。
pub fn spawn_xp_orb(
    world: &mut World,
    pos: Vec3,
    value: u16,
    rng: &mut impl FnMut() -> u32,
) -> Entity {
    let e = world.spawn();
    world.insert(
        e,
        PhysBody {
            pos,
            vel: initial_vel(rng),
            on_ground: false,
        },
    );
    world.insert(e, XpOrb { value, age: 0 });
    e
}

/// 固定步系统：物理积分（重力缩放 0.375）+ 落地小弹（vy=+|impact|×0.4，
/// ExperienceOrb.java:141-142）+ age 步进 + 到期/虚空销毁（延迟命令）。
/// 经验球无合并（原版 scanForMerges/tryMergeToExisting 同 id%40 桶合并是
/// 大量掉落场景的记账优化，登记债——矿石单次 ≤3 球，量级无害）。
pub fn xp_orb_physics_system(ctx: &mut mcv_ecs::SysCtx) {
    let dw = ctx
        .resources
        .get::<crate::items::DropWorld>()
        .expect("xp_orb_physics：DropWorld 快照未注入 Resources");
    let world: &mcv_ecs::World = ctx.world;
    let commands = &mut *ctx.commands;
    let (mut phys, mut orbs) = (world.write::<PhysBody>(), world.write::<XpOrb>());
    let input = mcv_game::StepInput {
        gravity_scale: XP_ORB_GRAVITY_SCALE,
        ..mcv_game::StepInput::default()
    };
    phys.for_each(|e, body| {
        let Some(o) = orbs.get_mut(e) else { return };
        let mut eng = body.body();
        let impact = eng.vel.y;
        mcv_game::step_entity(&VoxelView(dw), &mut eng, XP_ORB_HALF, &input);
        // 落地小弹（ExperienceOrb.java:141-142：verticalCollisionBelow 且
        // fallSpeed < −gravity → vy = −fallSpeed×0.4；fallSpeed 阈值
        // 0.03 块/tick = 0.6 m/s，低于不弹——贴地微动不算碰撞回落）。
        if eng.on_ground && impact < -0.6 {
            eng.vel.y = -impact * 0.4;
        }
        body.set_body(&eng);
        // 虚空：y < minY−64 → discard（Entity.checkBelowWorld:579-583 同款，
        // 与掉落物同纪律）。
        if body.pos.y < -64.0 {
            commands.despawn(e);
            return;
        }
        o.age += 1;
        if o.age >= XP_ORB_DESPAWN_AGE {
            commands.despawn(e);
        }
    });
}

/// `step_entity` 的体素视图：未载区块/界外按实心（物理安全垫），与
/// items.rs `ChunkVoxels` 同口径（资源快照借引用，不复制表）。
struct VoxelView<'a>(&'a crate::items::DropWorld);

impl mcv_game::VoxelAccess for VoxelView<'_> {
    fn block(&self, p: mcv_core::BlockPos) -> mcv_core::BlockId {
        use mcv_core::{Stage, WORLD_MAX_Y, WORLD_MIN_Y};
        let Some(chunk) = self.0.chunks.get(&p.chunk()) else {
            return mcv_core::BlockId(1);
        };
        if chunk.stage() == Stage::Empty {
            return mcv_core::BlockId(1);
        }
        if !(WORLD_MIN_Y..WORLD_MAX_Y).contains(&p.y) {
            return mcv_core::BlockId(1);
        }
        let [lx, _, lz] = p.local();
        chunk.voxels.read().unwrap()[mcv_core::vidx(lx, p.y, lz)]
    }
    fn light(&self, _p: mcv_core::BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, c: mcv_core::ChunkPos) -> bool {
        self.0.chunks.contains_key(&c)
    }
}

/// 拾取系统：球 AABB 与外扩 (1.0, 0.5, 1.0) 玩家盒相交 → 发
/// [`XpOrbPickup`]（几何判定同 item_pickup_system；节流在主控结算侧）。
pub fn xp_orb_pickup_system(ctx: &mut mcv_ecs::SysCtx) {
    let dw = ctx
        .resources
        .get::<crate::items::DropWorld>()
        .expect("xp_orb_pickup：DropWorld 快照未注入 Resources");
    let (pmin, pmax) = crate::items::player_pickup_box(dw.player_pos);
    let world: &mcv_ecs::World = ctx.world;
    let events = &mut *ctx.events;
    let (phys, orbs) = (world.read::<PhysBody>(), world.read::<XpOrb>());
    for (e, o) in orbs.iter() {
        let Some(b) = phys.get(e) else { continue };
        let (cx, cy, cz) = (b.pos.x, b.pos.y + XP_ORB_HALF[1], b.pos.z);
        let hit = (cx - XP_ORB_HALF[0] < pmax.x)
            && (cx + XP_ORB_HALF[0] > pmin.x)
            && (cy - XP_ORB_HALF[1] < pmax.y)
            && (cy + XP_ORB_HALF[1] > pmin.y)
            && (cz - XP_ORB_HALF[2] < pmax.z)
            && (cz + XP_ORB_HALF[2] > pmin.z);
        if hit {
            events
                .channel::<XpOrbPickup>()
                .send(XpOrbPickup { e, value: o.value });
        }
    }
}
