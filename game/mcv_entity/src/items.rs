//! 掉落物实体（26.1 `ItemEntity` 子集）：`ItemDrop` 组件 + 生成入口 +
//! 物理/寿命/拾取/合并系统。组件与 [`crate::components::PhysBody`] 组合
//! 挂载，步进复用 [`mcv_game::step_entity`]（掉落物重力缩放 0.5，见
//! [`StepInput.gravity_scale`]）。
//!
//! 数值对齐反编译 ItemEntity.java（/root/mc-ref/VERIFY-drops.md）：
//! AABB 0.25³、初速 x/z=±0.1 y=+0.2 块/tick、落地反弹 ×0.5、寿命
//! 6000 tick、pickup_delay 10 tick（死亡掉落 40）、拾取盒 inflate(1,0.5,1)、
//! 合并并给 count 大者且 age 取 min。引擎步 1/60 s，原版 tick 1/20 s，
//! 计数阈值一律 ×3 换算（`STEPS_PER_TICK`）。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkHandle, ChunkPos, Stage};
use mcv_ecs::{Entity, World};

use crate::components::PhysBody;

/// 掉落物碰撞半尺寸（AABB 0.25³，MC Item.java:24 SIZE=0.25）。
pub const ITEM_HALF: [f32; 3] = [0.125; 3];
/// 引擎固定步长 1/60 s，原版 tick = 1/20 s → 1 tick = 3 步。age/pickup_delay
/// 均以**步**计数，阈值 = 原版 tick 值 × 3（审计 C2/C5 的步↔tick 换算）。
pub const STEPS_PER_TICK: u32 = 3;
/// 生成后禁止拾取（MC ItemEntity pickupDelay 默认 10 tick = 0.5 s）。
pub const PICKUP_DELAY: u8 = 10 * 3;
/// 玩家死亡掉落的拾取延迟（MC LivingEntity.drop 40 tick = 2 s）。
pub const DEATH_PICKUP_DELAY: u8 = 40 * 3;
/// 消失年龄（MC ItemEntity LIFETIME 6000 tick = 300 s）。
pub const DESPAWN_AGE: u32 = 6000 * 3;
/// 拾取判定：玩家碰撞盒外扩（MC Player.java:454 `inflate(1.0, 0.5, 1.0)`）。
pub const PICKUP_INFLATE_XZ: f32 = 1.0;
/// 拾取判定竖向外扩（同上，y 轴 0.5）。
pub const PICKUP_INFLATE_Y: f32 = 0.5;
/// 同物品合并水平中心距上限（MC 合并 BB `inflate(0.5, 0, 0.5)` 的近似）。
pub const MERGE_DIST_XZ: f32 = 0.5;
/// 落地反弹冲击下限（m/s）：低于此不反弹。原版无阈值（vy = −vy×0.5 恒
/// 成立），但本引擎落地清零 vy，无阈值会造成贴地毫米级永动微弹。
pub const BOUNCE_MIN_IMPACT: f32 = 1.5;

/// 掉落物组件：与 [`PhysBody`] 成对出现（物理走 PhysBody + step_entity）。
#[derive(Clone, Copy, Debug)]
pub struct ItemDrop {
    /// 物品内核 id（[`mcv_item::ITEMS`] 下标）。
    pub item: u16,
    pub count: u8,
    /// 已存活步数（3 步 = 1 原版 tick）；≥ [`DESPAWN_AGE`] 消失。
    pub age: u32,
    /// 剩余不可拾取步数（MC pickupDelay 语义，每步 −1）。
    pub pickup_delay: u8,
}

/// 掉落系统的单步快照：区块表克隆（物理地形）+ 玩家位姿（拾取判定）。
/// 与 game.rs 的 `MobServices` 同款——Arc 计数级克隆，体素数据共享。
#[derive(Clone)]
pub struct DropWorld {
    pub chunks: HashMap<ChunkPos, Arc<ChunkHandle>>,
    pub player_pos: Vec3,
}

/// 拾取请求事件：系统只做几何判定，物品入栏由主控 `settle_pickups`
/// 走 [`mcv_item::Hotbar::add`]（与破坏直落同一 give 路径）。
#[derive(Clone, Copy, Debug)]
pub struct PickupReq {
    pub e: Entity,
    pub item: u16,
    pub count: u8,
}

/// WorldView 的 mcv_entity 侧等价：未载区块按实心石（物理安全垫）。
struct ChunkVoxels<'a> {
    chunks: &'a HashMap<ChunkPos, Arc<ChunkHandle>>,
}

impl mcv_game::VoxelAccess for ChunkVoxels<'_> {
    fn block(&self, p: BlockPos) -> BlockId {
        let Some(chunk) = self.chunks.get(&p.chunk()) else {
            return BlockId(1);
        };
        if chunk.stage() == Stage::Empty {
            return BlockId(1);
        }
        let [lx, ly, lz] = p.local();
        chunk.voxels.read().unwrap()[ly << 8 | lz << 4 | lx]
    }
    fn light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, c: ChunkPos) -> bool {
        self.chunks.contains_key(&c)
    }
}

/// MC 初速：水平 rand×0.2−0.1、竖直 +0.2（块/tick，ItemEntity.java:66
/// 构造器）→ 引擎 m/s 需 ×20（块/tick × tick/s）。rng 复用 game.rs 的
/// 确定性 fast_rand 链，测试可控。
fn initial_vel(rng: &mut impl FnMut() -> u32) -> Vec3 {
    let r = |rng: &mut dyn FnMut() -> u32| rng() as f32 / u32::MAX as f32;
    Vec3::new(
        (r(rng) - 0.5) * 0.2 * 20.0,
        0.2 * 20.0,
        (r(rng) - 0.5) * 0.2 * 20.0,
    )
}

/// 预注册掉落物组件表（零掉落时系统仍要取视图）。
pub fn register_drop_components(world: &mut World) {
    world.register::<ItemDrop>();
    world.register::<PhysBody>();
}

/// 生成一只掉落物：PhysBody(pos, 随机初速) + ItemDrop(pickup_delay)。
pub fn spawn_item_drop(
    world: &mut World,
    pos: Vec3,
    item: u16,
    count: u8,
    pickup_delay: u8,
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
    world.insert(
        e,
        ItemDrop {
            item,
            count,
            age: 0,
            pickup_delay,
        },
    );
    e
}

/// 固定步系统：物理积分（step_entity 通用重力/阻尼/落地；掉落物重力
/// 缩放 0.5 = 原版 ItemEntity.getDefaultGravity 0.04 块/tick²）+ 落地
/// 反弹 + age/pickup_delay 步进 + 到期 despawn（延迟命令，同 mob_ai）。
pub fn item_physics_system(ctx: &mut mcv_ecs::SysCtx) {
    let dw = ctx
        .resources
        .get::<DropWorld>()
        .expect("item_physics：DropWorld 快照未注入 Resources");
    let world: &mcv_ecs::World = ctx.world;
    let commands = &mut *ctx.commands;
    let (mut phys, mut drops) = (world.write::<PhysBody>(), world.write::<ItemDrop>());
    let view = ChunkVoxels { chunks: &dw.chunks };
    let input = mcv_game::StepInput {
        gravity_scale: 0.5,
        ..mcv_game::StepInput::default()
    };
    phys.for_each(|e, body| {
        let Some(d) = drops.get_mut(e) else { return };
        let mut eng = body.body();
        let impact = eng.vel.y; // 落地前竖直速度 = 冲击速度
        mcv_game::step_entity(&view, &mut eng, ITEM_HALF, &input);
        // 落地反弹（MC ItemEntity.java:159-164：onGround 且 vy<0 →
        // vy = −vy×0.5）；低于冲击下限不弹（见 BOUNCE_MIN_IMPACT 注释）。
        if eng.on_ground && impact < -BOUNCE_MIN_IMPACT {
            eng.vel.y = -impact * 0.5;
        }
        body.set_body(&eng);
        // 掉进虚空：y < -64 立即销毁（MC Entity.checkBelowWorld：y <
        // minY−64 → discard，不等 LIFETIME；BlockPos::local 对负 y 取模
        // 安全，读到的别名数据无碍）。
        if body.pos.y < -64.0 {
            commands.despawn(e);
            return;
        }
        d.age += 1;
        d.pickup_delay = d.pickup_delay.saturating_sub(1);
        if d.age >= DESPAWN_AGE {
            commands.despawn(e);
        }
    });
}

/// 玩家碰撞盒外扩（x/z [`PICKUP_INFLATE_XZ`]、y [`PICKUP_INFLATE_Y`]，
/// MC Player.java:454 `inflate(1.0, 0.5, 1.0)`）后的盒（pos = 脚底中心）。
fn player_box(pos: Vec3) -> (Vec3, Vec3) {
    let h = mcv_game::Player::HALF;
    (
        Vec3::new(
            pos.x - h[0] - PICKUP_INFLATE_XZ,
            pos.y - PICKUP_INFLATE_Y,
            pos.z - h[2] - PICKUP_INFLATE_XZ,
        ),
        Vec3::new(
            pos.x + h[0] + PICKUP_INFLATE_XZ,
            pos.y + h[1] * 2.0 + PICKUP_INFLATE_Y,
            pos.z + h[2] + PICKUP_INFLATE_XZ,
        ),
    )
}

/// 拾取系统：pickup_delay 归零后，掉落物 AABB 与外扩 (1.0, 0.5, 1.0) 的
/// 玩家 AABB 相交 → 发 [`PickupReq`] 事件（剩余入栏逻辑在主控
/// settle_pickups，满栏剩余自然留在地上——下一步再相交再请求）。
pub fn item_pickup_system(ctx: &mut mcv_ecs::SysCtx) {
    let dw = ctx
        .resources
        .get::<DropWorld>()
        .expect("item_pickup：DropWorld 快照未注入 Resources");
    let (pmin, pmax) = player_box(dw.player_pos);
    let world: &mcv_ecs::World = ctx.world;
    let events = &mut *ctx.events;
    let (phys, drops) = (world.read::<PhysBody>(), world.read::<ItemDrop>());
    for (e, d) in drops.iter() {
        if d.pickup_delay > 0 {
            continue;
        }
        let Some(b) = phys.get(e) else { continue };
        // 掉落物 AABB 中心（pos 为脚底）。
        let (cx, cy, cz) = (b.pos.x, b.pos.y + ITEM_HALF[1], b.pos.z);
        let hit = (cx - ITEM_HALF[0] < pmax.x)
            && (cx + ITEM_HALF[0] > pmin.x)
            && (cy - ITEM_HALF[1] < pmax.y)
            && (cy + ITEM_HALF[1] > pmin.y)
            && (cz - ITEM_HALF[2] < pmax.z)
            && (cz + ITEM_HALF[2] > pmin.z);
        if hit {
            events.channel::<PickupReq>().send(PickupReq {
                e,
                item: d.item,
                count: d.count,
            });
        }
    }
}

/// 合并趟（26.1 ItemEntity.merge 近似）：同物品、双方未满堆、未待删
/// （age < [`DESPAWN_AGE`]，审计 M10）→ 数量小的并给数量大的（同量取
/// 稠密表序先到者）；单次封顶 max_stack；合并后目标 **age 取 min（保
/// 年轻者）、pickup_delay 取 max**（原版 ItemEntity.java:260-267）。
/// 形状：水平 XZ 中心距 < [`MERGE_DIST_XZ`] 且 y 盒重叠（原版 BB
/// `inflate(0.5, 0, 0.5)` 的近似）。TODO(审计 M4)：合并触发节奏每 2/40
/// tick，本实现每步都跑（掉落物量级几十，O(n²) 可接受）。
pub fn item_merge_system(ctx: &mut mcv_ecs::SysCtx) {
    let world: &mcv_ecs::World = ctx.world;
    let commands = &mut *ctx.commands;
    // 快照（合并中会改 count，先收集避免迭代借用冲突）。
    let mut list: Vec<(Entity, Vec3, u16, u8, u32, u8)> = Vec::new();
    {
        let (phys, drops) = (world.read::<PhysBody>(), world.read::<ItemDrop>());
        for (e, d) in drops.iter() {
            if let Some(b) = phys.get(e) {
                list.push((e, b.pos, d.item, d.count, d.age, d.pickup_delay));
            }
        }
    }
    for i in 0..list.len() {
        let (ei, pi, item_i, mut count_i, mut age_i, mut delay_i) = list[i];
        let max = mcv_item::Hotbar::max_stack(item_i);
        // 已被吸收光（count=0，稍后 despawn）或已满堆：不作合并目标。
        if count_i == 0 || count_i >= max {
            continue;
        }
        // 可变切片迭代（clippy needless_range_loop：j 仅用于索引 list）。
        for slot in list[i + 1..].iter_mut() {
            let (ej, pj, item_j, count_j, age_j, delay_j) = *slot;
            if item_j != item_i || count_j == 0 || count_j >= max {
                continue;
            }
            if (pi.x - pj.x).hypot(pi.z - pj.z) >= MERGE_DIST_XZ
                || (pi.y - pj.y).abs() >= ITEM_HALF[1] * 2.0
            {
                continue;
            }
            // 并给 count 大者（原版 ItemEntity.java:233-239）；同量并入先到者。
            if count_i >= count_j {
                let mv = (max - count_i).min(count_j);
                if mv == 0 {
                    break; // 目标已满，后续更远者不必再看
                }
                count_i += mv;
                age_i = age_i.min(age_j);
                delay_i = delay_i.max(delay_j);
                slot.3 -= mv;
                if slot.3 == 0 {
                    commands.despawn(ej);
                } else {
                    commands.push(move |w| {
                        if let Some(d) = w.write::<ItemDrop>().get_mut(ej) {
                            d.count -= mv;
                        }
                    });
                }
            } else {
                let mv = (max - count_j).min(count_i);
                if mv == 0 {
                    continue;
                }
                slot.3 += mv;
                slot.4 = age_j.min(age_i);
                slot.5 = delay_j.max(delay_i);
                commands.push(move |w| {
                    if let Some(d) = w.write::<ItemDrop>().get_mut(ej) {
                        d.count += mv;
                        d.age = age_j.min(age_i);
                        d.pickup_delay = delay_j.max(delay_i);
                    }
                });
                count_i -= mv;
                if count_i == 0 {
                    commands.despawn(ei);
                    break;
                }
            }
        }
        let (final_count, final_age, final_delay) = (count_i, age_i, delay_i);
        commands.push(move |w| {
            if let Some(d) = w.write::<ItemDrop>().get_mut(ei) {
                d.count = final_count;
                d.age = final_age;
                d.pickup_delay = final_delay;
            }
        });
    }
}

/// 主控结算 [`PickupReq`]：走 [`mcv_item::Hotbar::add`]（与破坏直落同一
/// give 路径），全收 → despawn；有剩余（满栏）→ 组件 count 改为剩余量，
/// 实体留在地上。GameRuntime 固定步末与集成测试共用本函数。
pub fn settle_pickups(
    world: &mut mcv_ecs::World,
    events: &mut mcv_ecs::EventBus,
    hotbar: &mut mcv_item::Hotbar,
    sel: usize,
) {
    for req in events.channel::<PickupReq>().take() {
        // 句柄带代校验：本 tick 内已被合并/到期的实体直接跳过。
        let Some(mut d) = world.get_ref::<ItemDrop>(req.e).map(|d| *d) else {
            continue;
        };
        if d.count == 0 {
            continue;
        }
        let rem = hotbar.add(sel, mcv_item::ItemStack::new(req.item, d.count));
        match rem {
            None => {
                world.despawn(req.e);
            }
            Some(rest) => {
                d.count = rest.count;
                if let Some(slot) = world.write::<ItemDrop>().get_mut(req.e) {
                    *slot = d;
                }
            }
        }
    }
}
