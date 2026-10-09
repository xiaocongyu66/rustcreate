//! 被动生物（鸡/牛/羊/猪）的 ECS 组件与系统：随机游荡（`RandomStrollGoal`
//! 等价）+ 有界转身 + 行走/扑翼动画相位 + 受击逃离（`PanicGoal` 等价，
//! 事件驱动：消费 [`MobTicks::hurt_flag`]，由 game.rs `try_attack` 置位）
//! + 羊吃草动画（`EatBlockGoal` 的表现段）。
//!
//! 与 game.rs `mob_ai_system` 的关系（集成点见报告）：
//! - 本模块的系统只处理**持有 [`WanderState`] 的实体**（由
//!   [`spawn_passive_mob`] 装配）；game.rs 的 Brain 路线不插该组件，
//!   两套运行时互不触碰；
//! - 物理固定步复用 `mcv_game::step_entity`（AABB 碰撞），与敌对怪
//!   同一引擎通路；鸡额外做慢落（Chicken.java:121-126 扑翼 ×0.6）；
//! - 羊吃草只播放动画（低头 40 tick），不销毁草方块
//!   ——KNOWN-DIVERGENCE（方块变更需要世界编辑面，留待集成）。
//!
//! 常数出处：`RandomStrollGoal.java:16`（每 tick p=1/120）、
//! `DefaultRandomPos.getPos(mob, 10, 7)`（游走半径）、`PanicGoal.java`
//! （随机点 5,4 内起跑、跑到导航完成）、`Chicken.java:117-129`（扑翼账本）。

use crate::defs::{MobId, MobKind, speed_m_s};
use glam::Vec3;
use mcv_ecs::{Entity, World};
use std::collections::HashMap;
use std::sync::Arc;

use mcv_core::{BlockId, BlockPos, ChunkHandle, ChunkPos, Stage};
use mcv_game::{VoxelAccess, step_entity};

// ---- 参照常量 ---------------------------------------------------------------

/// RandomStrollGoal.java:16 — DEFAULT_INTERVAL（Idle 每 tick p=1/120）。
pub const STROLL_INTERVAL: u32 = 120;
/// PanicGoal 兜底时长（原版跑到导航完成；直线移动下给 5 s 上界）。
pub const PANIC_MAX_TICKS: u32 = 100;
/// 羊吃草触发间隔（EatBlockGoal.java:29 — isBaby ? 50 : 1000）。
pub const EAT_GRASS_INTERVAL: u32 = 1000;
/// 羊吃草动画长度（Sheep.java:112 — eatAnimationTick = 40）。
pub const EAT_GRASS_TICKS: u32 = 40;
/// PanicGoal.java:65 — DefaultRandomPos.getPos(mob, 5, 4)：逃离点半径。
pub const PANIC_RADIUS: f32 = 5.0;

/// 被动怪的一步物理体型（半宽；`step_entity` 的盒高 = half[1]×2）。
pub type HalfSize = [f32; 3];

/// 玩家基础移速 m/s（0.1 速度属性 × 43.17；相位归一化基准，与
/// `player_mesh::update_walk_animation` 的 WALK_SPEED 同值）。
pub const PLAYER_WALK_SPEED: f32 = 4.317;

/// 鸡扑翼账本推进（Chicken.java:117-129，tick 语义；渲染侧等价实现见
/// `mcv_render::mob_mesh::update_wing_animation`——引擎不反向依赖游戏层，
/// 两处各持同一公式，出处行号一致）。
pub fn update_wing(flap: f32, flap_speed: f32, on_ground: bool) -> (f32, f32) {
    let mut fs = flap_speed + (if on_ground { -1.0 } else { 4.0 }) * 0.3;
    fs = fs.clamp(0.0, 1.0);
    let flapping = if !on_ground { 1.0 } else { 0.0 };
    (flap + flapping * 2.0, fs)
}

// ---- 组件 -------------------------------------------------------------------

/// 游荡/逃离 AI 运行时（每只被动怪一份，20 Hz 语义）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PassiveMode {
    /// 静立（偶发随机环顾——表现层用 yaw 抖动近似）。
    Idle,
    /// 走向 [`WanderState::target`]。
    Wander,
    /// 受击逃离（远离威胁点直线冲刺）。
    Panic,
    /// 羊低头吃草（原地站立）。
    Eat,
}

#[derive(Clone, Copy, Debug)]
pub struct WanderState {
    pub mode: PassiveMode,
    /// 游走/逃离目标点（世界坐标）。
    pub target: Vec3,
    /// Idle 重新掷点的 tick 冷却（=0 时按 p=1/120 掷）。
    pub cooldown: u32,
    /// 逃离剩余 tick（Panic 用，到点回 Idle）。
    pub panic_ticks: u32,
    /// 吃草剩余 tick（Eat 用，到点回 Idle）。
    pub eat_ticks: u32,
}

/// 动画状态（渲染拼装消费：喂 `mob_mesh::MobPose`）。
#[derive(Clone, Copy, Debug)]
pub struct AnimState {
    /// 行走摆动相位（rad；`player_mesh::update_walk_animation` 同款推进）。
    pub phase: f32,
    /// 行走摆动幅值（0..0.88，静止指数衰减）。
    pub amount: f32,
    /// 鸡扑翼相位 `flap`（Chicken.java：空中每 tick +2）。
    pub wing_phase: f32,
    /// 鸡扑翼速度 `flapSpeed`（0..1，落地收拢）。
    pub wing_speed: f32,
}

impl Default for AnimState {
    fn default() -> Self {
        Self {
            phase: 0.0,
            amount: 0.0,
            wing_phase: 0.0,
            wing_speed: 0.0,
        }
    }
}

impl AnimState {
    /// 羊低头量（getHeadEatPositionScale，a=0；由 eat_ticks 反推账本）。
    pub fn head_drop(&self, eat_ticks: u32) -> f32 {
        if eat_ticks == 0 {
            return 0.0;
        }
        if (4..=36).contains(&eat_ticks) {
            1.0
        } else if eat_ticks < 4 {
            eat_ticks as f32 / 4.0
        } else {
            -(eat_ticks as f32 - 40.0) / 4.0
        }
    }
    /// 羊低头角（getHeadEatAngleScale，a=0，弧度，正=低头 MC 语义）。
    pub fn head_eat_angle(&self, eat_ticks: u32) -> f32 {
        // std 无 FRAC_PI_5 常数（f32::consts 只有 2/3/4/6/8），就地除。
        if eat_ticks == 0 {
            return 0.0;
        }
        if eat_ticks > 4 && eat_ticks <= 36 {
            let scale = (eat_ticks as f32 - 4.0) / 32.0;
            std::f32::consts::PI / 5.0 + 0.219_911_5 * (scale * 28.7).sin()
        } else {
            std::f32::consts::PI / 5.0
        }
    }
}

// ---- 宿主快照资源 -----------------------------------------------------------

/// 被动怪系统的宿主快照（与 game.rs `MobServices` 同构的耦合面）：
/// 每固定步由宿主 `insert` 一份所有权快照；`seed` 由系统内推进（确定性
/// 随机源——固定种子即完全可复现，测试断言位移有界依赖此性质）。
pub struct PassiveServices {
    pub chunks: HashMap<ChunkPos, Arc<ChunkHandle>>,
    pub player_pos: Vec3,
    /// 本固定步是否跨过 20 Hz tick 边界（AI 决策门）。
    pub on_tick: bool,
    /// xorshift64* 状态；宿主给初值，系统每步推进。
    pub seed: u64,
}

/// 单区块只读视图（与 game.rs `WorldView` 同语义：未加载 = 石头保安全）。
struct ChunkWorld<'a> {
    chunks: &'a HashMap<ChunkPos, Arc<ChunkHandle>>,
}

impl VoxelAccess for ChunkWorld<'_> {
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

/// xorshift64*（Marsaglia）：确定性、零依赖；种子 0 视为 1。
fn xorshift(state: &mut u64) -> u64 {
    let mut x = if *state == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        *state
    };
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn rand01(state: &mut u64) -> f32 {
    // 取高 24 位归一化（低 32 位 xorshift 质量较弱）。
    (xorshift(state) >> 40) as f32 / (1u64 << 24) as f32
}

// ---- 装配 ------------------------------------------------------------------

/// 预注册被动怪组件表（GameRuntime 建 World 后调一次；零怪时系统仍会
/// `read`/`write` 这些视图）。
pub fn register_passive_components(world: &mut World) {
    world.register::<WanderState>();
    world.register::<AnimState>();
}

/// 装配一只被动怪：在 [`crate::spawn_mob`] 的组件之上追加
/// [`WanderState`] + [`AnimState`]（渲染拼装与游荡 AI 的输入面）。
pub fn spawn_passive_mob(world: &mut World, id: MobId, pos: Vec3) -> Entity {
    let e = crate::spawn_mob(world, id, pos);
    world.insert(
        e,
        WanderState {
            mode: PassiveMode::Idle,
            target: pos,
            cooldown: 0,
            panic_ticks: 0,
            eat_ticks: 0,
        },
    );
    world.insert(e, AnimState::default());
    e
}

/// PanicGoal 的 speedModifier（registerGoals 各类构造参数）。
pub fn panic_speed_mult(id: MobId) -> f32 {
    match id {
        MobId::COW => 2.0,     // AbstractCow.java:41
        MobId::CHICKEN => 1.4, // Chicken.java:82
        _ => 1.25,             // Sheep.java:69 / Pig.java:74
    }
}

// ---- 系统 ------------------------------------------------------------------

/// 被动怪固定步系统：AI 决策 20 Hz（`on_tick` 门）→ 写 [`MobIntent`]/
/// [`Yaw`]；物理 60 Hz 每步照上一 tick 意图步进（`mcv_game::step_entity`）；
/// 动画账本每步推进并写 [`AnimState`]。只迭代持有 [`WanderState`] 的实体。
pub fn passive_ai_system(ctx: &mut mcv_ecs::SysCtx) {
    let Some(svc) = ctx.resources.get_mut::<PassiveServices>() else {
        return;
    };
    let world: &World = ctx.world;
    let commands = &mut *ctx.commands;
    let on_tick = svc.on_tick;
    let world_map = &svc.chunks;
    let view = ChunkWorld { chunks: world_map };
    // 确定性随机源：本轮所有实体共享同一 xorshift 流（顺序固定——
    // dense 表迭代次序稳定；单怪测试无歧义，多怪时序仍确定）。
    let mut rng_state = svc.seed;
    if rng_state == 0 {
        rng_state = 0x9E37_79B9_7F4A_7C15;
    }

    let (mut phys, kinds, mut wanders, mut anims, mut yaws, mut intents, mut ticks, mut healths) = (
        world.write::<crate::PhysBody>(),
        world.read::<crate::MobKind>(),
        world.write::<WanderState>(),
        world.write::<AnimState>(),
        world.write::<crate::Yaw>(),
        world.write::<crate::MobIntent>(),
        world.write::<crate::MobTicks>(),
        world.write::<crate::Health>(),
    );

    phys.for_each(|e, body| {
        let Some(wander) = wanders.get_mut(e) else {
            return; // 非被动怪（敌对 Brain 路径），跳过
        };
        let Some(&crate::MobKind(id)) = kinds.get(e) else {
            return;
        };
        let def = *id.def();
        let speed = speed_m_s(def.speed_attr);
        let pos = body.pos;

        // ---- 20 Hz AI（on_tick 门）：游荡 / 逃离 / 吃草 / 转身 ----
        if on_tick {
            // 受击逃离（事件输入）：PanicGoal 优先级最高。
            let hurt = ticks.get_mut(e).is_some_and(|tk| {
                let h = tk.hurt_flag;
                tk.hurt_flag = false;
                h
            });
            if hurt {
                // PanicGoal.findRandomPosition：随机点（5,4）内直线奔逃。
                let ang = rand01(&mut rng_state) * std::f32::consts::TAU;
                let d = 2.0 + rand01(&mut rng_state) * (PANIC_RADIUS - 2.0);
                wander.target = pos + Vec3::new(ang.cos() * d, 0.0, ang.sin() * d);
                wander.mode = PassiveMode::Panic;
                wander.panic_ticks = PANIC_MAX_TICKS;
            }

            match wander.mode {
                PassiveMode::Panic => {
                    wander.panic_ticks = wander.panic_ticks.saturating_sub(1);
                    let reached = horizontal_dist(pos, wander.target) < 0.5;
                    if reached || wander.panic_ticks == 0 {
                        wander.mode = PassiveMode::Idle;
                        wander.cooldown = 0;
                    }
                }
                PassiveMode::Wander => {
                    if horizontal_dist(pos, wander.target) < 0.5 {
                        wander.mode = PassiveMode::Idle;
                        wander.cooldown = 0;
                    }
                }
                PassiveMode::Eat => {
                    wander.eat_ticks = wander.eat_ticks.saturating_sub(1);
                    if wander.eat_ticks == 0 {
                        wander.mode = PassiveMode::Idle;
                    }
                }
                PassiveMode::Idle => {
                    // 羊：站在草方块上按 p=1/1000 低头吃草（动画段）。
                    if def.kind == MobKind::Sheep
                        && body.on_ground
                        && xorshift(&mut rng_state).is_multiple_of(u64::from(EAT_GRASS_INTERVAL))
                    {
                        let below = BlockPos::new(
                            pos.x.floor() as i32,
                            (pos.y - 0.5).floor() as i32,
                            pos.z.floor() as i32,
                        );
                        if below_ground_is_grass(world_map, below) {
                            wander.mode = PassiveMode::Eat;
                            wander.eat_ticks = EAT_GRASS_TICKS;
                        }
                    }
                    // RandomStrollGoal：每 tick p=1/120 起游。
                    if wander.mode == PassiveMode::Idle
                        && xorshift(&mut rng_state).is_multiple_of(u64::from(STROLL_INTERVAL))
                    {
                        let ang = rand01(&mut rng_state) * std::f32::consts::TAU;
                        let d = 2.0 + rand01(&mut rng_state) * (crate::ai::WANDER_RADIUS - 2.0);
                        wander.target = pos + Vec3::new(ang.cos() * d, 0.0, ang.sin() * d);
                        wander.mode = PassiveMode::Wander;
                    }
                }
            }

            // 移动意图 → MobIntent（60 Hz 物理复用）+ 朝向。
            let wish = match wander.mode {
                PassiveMode::Wander | PassiveMode::Panic => {
                    let mult = if wander.mode == PassiveMode::Panic {
                        panic_speed_mult(id)
                    } else {
                        1.0
                    };
                    let mut dir = wander.target - pos;
                    dir.y = 0.0;
                    let len = dir.length();
                    if len < 1e-4 {
                        Vec3::ZERO
                    } else {
                        dir / len * (speed * mult)
                    }
                }
                _ => Vec3::ZERO,
            };
            if let Some(intent) = intents.get_mut(e) {
                // 想走走不动（地面且水平速度远低于预期）→ 跳跃兜底
                //（A* 未实现 N-1 的最小替代，与 game.rs 同款）。
                let flat = (body.vel.x * body.vel.x + body.vel.z * body.vel.z).sqrt();
                let blocked = wish.length_squared() > 1e-4 && body.on_ground && flat < speed * 0.25;
                *intent = crate::MobIntent {
                    wish,
                    jump: blocked,
                };
            }
            if wish.length_squared() > 1e-4
                && let Some(y) = yaws.get_mut(e)
            {
                // 有界转身：每 tick 至多 0.35 rad（≈20°），平滑接近航向。
                let target_yaw = wish.x.atan2(-wish.z);
                let diff = wrap_angle(target_yaw - y.0);
                y.0 = wrap_angle(y.0 + diff.clamp(-0.35, 0.35));
            }
        }

        // ---- 60 Hz 物理：AABB 步进 + 鸡慢落 ----
        let input = mcv_game::StepInput {
            wish_dir: intents.get(e).map(|i| i.wish).unwrap_or(Vec3::ZERO),
            jump: intents.get(e).is_some_and(|i| i.jump) && body.on_ground,
            in_water: false,
            sneak: false,
            sprint: false,
            gravity_scale: 1.0,
        };
        let mut eng = body.body();
        if def.kind == MobKind::Chicken && !body.on_ground && eng.vel.y < 0.0 {
            // Chicken.java:121-123 — 下落中 y 速度 ×0.6（扑翼缓降）。
            eng.vel.y *= 0.6;
        }
        step_entity(&view, &mut eng, def.half_size, &input);
        body.set_body(&eng);

        // ---- 动画账本（每步推进）：行走摆动 + 鸡扑翼 ----
        if let Some(anim) = anims.get_mut(e) {
            const DT: f32 = 1.0 / 60.0;
            let horiz = (eng.vel.x * eng.vel.x + eng.vel.z * eng.vel.z).sqrt();
            // 与 player_mesh::update_walk_animation 同款（玩家 0.1 属性 =
            // 4.317 m/s；被动怪属性不同，直接按比值缩放相位与幅值）。
            let ratio = (horiz / PLAYER_WALK_SPEED).min(1.0);
            let target = ratio * 0.88;
            anim.amount = if horiz > 0.15 {
                anim.amount + (target - anim.amount) * (DT * 10.0).min(1.0)
            } else {
                anim.amount * (-DT * 12.0).exp()
            };
            if horiz <= 0.15 && anim.amount < 0.005 {
                anim.amount = 0.0;
            }
            anim.phase = (anim.phase + ratio * 1.2 * (DT * 20.0)) % std::f32::consts::TAU;
            // 鸡扑翼：Chicken.java:117-119（tick 语义，在 on_tick 内推进）。
            if on_tick {
                let on_ground = body.on_ground;
                let (f, fs) = update_wing(anim.wing_phase, anim.wing_speed, on_ground);
                anim.wing_phase = f;
                anim.wing_speed = fs;
            }
        }

        // 死亡清理（Health≤0 → despawn；掉落由 game.rs 路径处理，
        // 此处与 mob_ai_system 一致只做环境死亡兜底）。
        if healths.get(e).is_some_and(|h| h.0 <= 0.0) {
            commands.despawn(e);
        }
    });

    // 推进后的随机流写回（下一固定步继续）。
    if let Some(svc) = ctx.resources.get_mut::<PassiveServices>() {
        svc.seed = rng_state;
    }
}

fn below_ground_is_grass(chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>, p: BlockPos) -> bool {
    // 草方块判定保持 mcv_entity 不依赖渲染 tiles 表：判“固体非空气”
    //（本仓区块表尚未分层材质编辑，动画触发面等价；报告 KNOWN-DIVERGENCE）。
    let world = ChunkWorld { chunks };
    world.block(p).def().solid
}

fn horizontal_dist(a: Vec3, b: Vec3) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    (dx * dx + dz * dz).sqrt()
}

/// 包裹到 (−π, π]。
fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut x = a % tau;
    if x > std::f32::consts::PI {
        x -= tau;
    } else if x <= -std::f32::consts::PI {
        x += tau;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_is_deterministic() {
        let (mut a, mut b) = (42u64, 42u64);
        for _ in 0..100 {
            assert_eq!(xorshift(&mut a), xorshift(&mut b));
        }
        assert_ne!(a, 42, "状态必须推进");
    }

    #[test]
    fn panic_mult_matches_register_goals() {
        assert_eq!(panic_speed_mult(MobId::COW), 2.0);
        assert_eq!(panic_speed_mult(MobId::CHICKEN), 1.4);
        assert_eq!(panic_speed_mult(MobId::SHEEP), 1.25);
        assert_eq!(panic_speed_mult(MobId::PIG), 1.25);
    }

    #[test]
    fn head_drop_cadence() {
        let anim = AnimState::default();
        assert_eq!(anim.head_drop(0), 0.0);
        assert!((anim.head_drop(2) - 0.5).abs() < 1e-5);
        assert_eq!(anim.head_drop(20), 1.0);
        assert!((anim.head_drop(38) - 0.5).abs() < 1e-5);
        assert!(anim.head_eat_angle(20) > 0.0);
    }
}
