//! 掉落物实体（26.1 `ItemEntity` 子集）：`ItemDrop` 组件 + 生成入口 +
//! 物理/寿命系统。组件与 [`crate::components::PhysBody`] 组合挂载，步进
//! 复用 [`mcv_game::step_entity`]（重力/阻尼/落地与怪物同一套代码）。
//!
//! 拾取与合并走 [`pickup_system`] / 合并趟（increment 2），本文件先落
//! 实体形态：AABB 0.25³（half 0.125，MC Item.java SIZE=0.25）、初速
//! 随机散布、age 6000 tick（5 分钟）消失（MC ItemEntity DESPAWN_TIME）。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkHandle, ChunkPos, Stage};
use mcv_ecs::{Entity, World};

use crate::components::PhysBody;

/// 掉落物碰撞半尺寸（AABB 0.25³，MC Item.java:24 SIZE=0.25）。
pub const ITEM_HALF: [f32; 3] = [0.125; 3];
/// 生成后禁止拾取的 tick 数（MC ItemEntity pickupDelay=10；矿掉落传 0）。
pub const PICKUP_DELAY: u8 = 10;
/// 消失年龄（MC ItemEntity.java DESPAWN_TIME=6000 tick = 5 min）。
pub const DESPAWN_AGE: u32 = 6000;
/// 拾取判定：玩家 AABB 外扩半径（MC 近似：实体碰撞盒 + 0.5 吸附余量）。
pub const PICKUP_INFLATE: f32 = 0.5;
/// 同物品合并半径（米，MC ItemEntity 合并距离近似）。
pub const MERGE_DIST: f32 = 0.5;

/// 掉落物组件：与 [`PhysBody`] 成对出现（物理走 PhysBody + step_entity）。
#[derive(Clone, Copy, Debug)]
pub struct ItemDrop {
    /// 物品内核 id（[`mcv_item::ITEMS`] 下标）。
    pub item: u16,
    pub count: u8,
    /// 已存活 tick；≥ [`DESPAWN_AGE`] 消失。
    pub age: u32,
    /// 剩余不可拾取 tick（MC pickupDelay 语义）。
    pub pickup_delay: u8,
}

/// 掉落系统的单步快照：区块表克隆（物理地形）。与 game.rs 的
/// `MobServices` 同款——Arc 计数级克隆，体素数据共享。
#[derive(Clone)]
pub struct DropWorld {
    pub chunks: HashMap<ChunkPos, Arc<ChunkHandle>>,
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

/// MC 初速：水平 (rand−0.5)×0.14、竖直 +0.2（块/tick，ItemEntity.java
/// 构造器）→ 引擎 m/s 需 ×20（块/tick × tick/s）。rng 复用 game.rs 的
/// 确定性 fast_rand 链，测试可控。
fn initial_vel(rng: &mut impl FnMut() -> u32) -> Vec3 {
    let r = |rng: &mut dyn FnMut() -> u32| rng() as f32 / u32::MAX as f32;
    Vec3::new(
        (r(rng) - 0.5) * 0.14 * 20.0,
        0.2 * 20.0,
        (r(rng) - 0.5) * 0.14 * 20.0,
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

/// 固定步系统：物理积分（step_entity 通用重力/阻尼/落地）+ age++ +
/// 到期 despawn（延迟命令，同 mob_ai 语义）。
pub fn item_physics_system(ctx: &mut mcv_ecs::SysCtx) {
    let dw = ctx
        .resources
        .get::<DropWorld>()
        .expect("item_physics：DropWorld 快照未注入 Resources");
    let world: &mcv_ecs::World = ctx.world;
    let commands = &mut *ctx.commands;
    let (mut phys, mut drops) = (world.write::<PhysBody>(), world.write::<ItemDrop>());
    let view = ChunkVoxels { chunks: &dw.chunks };
    phys.for_each(|e, body| {
        let Some(d) = drops.get_mut(e) else { return };
        let mut eng = body.body();
        mcv_game::step_entity(&view, &mut eng, ITEM_HALF, &mcv_game::StepInput::default());
        body.set_body(&eng);
        d.age += 1;
        d.pickup_delay = d.pickup_delay.saturating_sub(1);
        if d.age >= DESPAWN_AGE {
            commands.despawn(e);
        }
    });
}
