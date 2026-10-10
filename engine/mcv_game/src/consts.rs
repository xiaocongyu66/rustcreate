//! 玩家物理常数。
//!
//! 出处：仓库外反编译参照 `src-26.1/`（Minecraft 26.1）；所有行号均为本次
//! 逐行核对（LivingEntity/Player/Entity/Attributes/Abilities/LocalPlayer）。
//! MC 逻辑 tick = 20/s，速度单位换算：1 块/tick = 20 m/s，1 块/tick² = 400 m/s²。
//!
//! 连续化规则（1/60 s 固定步）：逐 tick 乘阻 r/tick → `v *= exp(-k·dt)`，
//! k = −20·ln(r)；「每 tick 先加输入再乘阻」的递推等价于一阶滞后
//! `v += (V_ss − v)·(1 − exp(−k·dt))`，稳态 V_ss = 输入增量 × r/(1−r)。

/// 重力加速度：MC gravity 属性默认 0.08 块/tick²（Attributes.java:45-46，
/// LivingEntity.travelInAir 每 tick `movementY -= getEffectiveGravity()`
/// :2432）× 20² = 32 m/s²。
pub const GRAVITY: f32 = 32.0;

/// 竖直空气阻力系数（仅作用于下落，`vel.y < 0` 时 `vel.y *= exp(-k·dt)`）。
///
/// MC 原值：竖直阻力 0.98/tick（LivingEntity.travelInAir:2442-2443
/// `verticalFriction = 0.98F`，**上升与下落都乘**），连续化
/// k = −20·ln 0.98 = 0.40405/s。现取 **0.4**（差 −1%）。
/// 渐近终端 = g/k = 80 m/s，与 MC 精确终端 78.4 差 2%。
///
/// 已知偏差（保留）：MC 上升段也乘 0.98，本实现不乘——上升峰值不受
/// 终端影响，仅轨迹略高（见 NOTES-physics.md 注 1）。
pub const AIR_DRAG_K: f32 = 0.4;

/// MC 等效终端下落速度：MC 稳态 3.92 块/tick × 20 = 78.4 m/s
/// （= 0.08·0.98/(1−0.98) 的不动点）。本实现的渐近值 g/k = 80 m/s，差 2%。
pub const TERMINAL_FALL_SPEED: f32 = 78.4;

/// 跳跃初速：MC JUMP_STRENGTH 属性默认 **0.42 块/tick**（Attributes.java:48-49
/// ；派单文案「JUMP_STRENGTH=1.0」与源码不符），jumpFromGround 以
/// `vy = max(jumpPower, vy)` 写入（LivingEntity.java:2344-2348）× 20 = 8.4 m/s。
///
/// 峰高对照：MC tick 制离散积分（先位移后 `(v−0.08)·0.98`）= 1.2522 块；
/// 连续解析 v²/2g = 1.1025 m；本实现显式 Euler（本步先以完整初速位移）
/// ≈ 1.173 m。三者均 > 1.0，保证可跳上一格台阶
/// （tests/physics_calib.rs::jump_onto_one_block）。
pub const JUMP_SPEED: f32 = 8.4;

/// 冲刺跳水平增补：0.2 块/tick × 20 = 4.0 m/s，沿**朝向**瞬间加入水平速度
/// （LivingEntity.java:2349-2351 `addDeltaMovement(-sin(yaw)·0.2, 0,
/// cos(yaw)·0.2)`，仅 `isSprinting()` 时）。这是原版冲刺跳比平跑快的
/// 根源（跳跃弧线期间水平速度从 ~9.6 衰减回空中稳态）。
pub const SPRINT_JUMP_BOOST: f32 = 4.0;

/// 行走速度：MC 属性 movement_speed 玩家基值 0.1（Player.createAttributes
/// :205-208；Abilities.walkingSpeed 同 0.1，Abilities.java:20）。
///
/// 原版地面稳态推导：地面每 tick 输入增量 = 0.1×(0.21600002/0.6³)
/// = 0.1 块/tick（getFrictionInfluencedSpeed:2659-2661），地面阻力
/// 0.6×0.91 = 0.546/tick（travelInAir:2424-2425）→ 稳态
/// 0.1/(1−0.546) = 0.2203 块/tick = **4.405 m/s**；
/// 社区实测口径 4.317 m/s（差 2%）。取保守值 **4.317** 保留。
pub const WALK_SPEED: f32 = 4.317;

/// 冲刺速度：步行 ×1.3（26.1 SPRINTING 速度修饰 `0.3F
/// ADD_MULTIPLIED_TOTAL`，LivingEntity.java:156-157）= 5.612 m/s。
/// 由 [`crate::physics::StepInput::sprint`] 接线（GameRuntime 侧另有
/// food>6 饥饿门，FoodConstants.java SPRINT_LEVEL=6）。
pub const SPRINT_SPEED: f32 = 5.612;

/// 潜行速度：SNEAKING_SPEED 属性默认 0.3（Attributes.java:79-80，客户端
/// modifyInput 对输入向量整体 ×0.3，LocalPlayer.java:714）= 行走 ×0.3。
pub const SNEAK_SPEED: f32 = WALK_SPEED * 0.3;

/// 地面水平加速一阶滞后系数：k = −20·ln(0.6×0.91) = 12.10/s。
///
/// 原版地面递推「v += 0.1·dir 后 v ×= 0.546」（LivingEntity.java:2616+2443）
/// 等价于朝目标速度（walk/sprint/sneak）的指数收敛；本系数同时决定
/// 起步加速与松手减速的响应（MC 松手约 1 tick 内减半）。
pub const GROUND_CONVERGE_K: f32 = 12.10;

/// 空气中水平控制稳态（非冲刺）：0.02 块/tick 输入 × 0.91/(1−0.91)
/// = 0.2022 块/tick = **4.044 m/s**（Player.java:1955 非飞行空中
/// getFlyingSpeed = 0.02；LivingEntity.java:2660 空中加速取该值）。
/// 注意空中稳态略低于地面 4.317——原版空中控速本就更弱。
pub const AIR_TERMINAL: f32 = 4.044;

/// 空气中水平控制稳态（冲刺）：0.026 块/tick（Player.java:1955
/// `isSprinting() ? 0.025999999F : 0.02F`）→ 5.258 m/s。
pub const AIR_TERMINAL_SPRINT: f32 = 5.258;

/// 空中水平收敛系数：k = −20·ln(0.91) = 1.886/s（离地时 blockFriction=1.0
/// → friction = 0.91/tick，LivingEntity.java:2424-2425）。响应比地面慢
/// ~6 倍 = 原版空中操控的「惯性」手感。
pub const AIR_CONVERGE_K: f32 = 1.886;

/// 飞行水平速度：创造飞行 moveRelative 输入 0.05 块/tick
/// （Abilities.java:19 flyingSpeed 默认 0.05；Player.java:1951-1953 飞行时
/// getFlyingSpeed 取该值）× 阻力 0.91 → 稳态 0.05×0.91/0.09 = 0.5056 块/tick
/// = **10.111 m/s**。
pub const FLY_SPEED: f32 = 10.111;

/// 飞行冲刺倍率：飞行中 isSprinting → getFlyingSpeed ×2
/// （Player.java:1953）→ 稳态 20.22 m/s。
pub const SPRINT_FLY_MULTIPLIER: f32 = 2.0;

/// 飞行竖直速度：按住跳/潜行每 tick 直接给 vy ±0.15 块
/// （LocalPlayer.java:878 `inputYa × flyingSpeed × 3.0`），Player.travel
/// 飞行分支把 y 回写为进入值的 ×0.6（Player.java:1395-1397）→ 稳态
/// 0.15×0.6/0.4 = 0.225 块/tick = **4.5 m/s**（升降同速）。
pub const FLY_VERT_SPEED: f32 = 4.5;

/// 飞行水平收敛系数：k = −20·ln(0.91) = 1.886/s（travelInAir 统一阻力
/// 0.91/tick，LivingEntity.java:2443；原版飞行漂移感即此）。
pub const FLY_DRAG_H: f32 = AIR_CONVERGE_K;

/// 飞行竖直收敛系数：k = −20·ln(0.6) = 10.22/s（Player.java:1397 的
/// ×0.6 回写；松开升降键 ~0.2s 内刹停，无漂浮）。
pub const FLY_DRAG_V: f32 = 10.22;

/// 水中水平稳态（非冲刺）：0.02 块/tick 输入（travelInWater
/// LivingEntity.java:2461）× 阻力 0.8（getWaterSlowDown :2366-2368）→
/// 稳态 0.08 块/tick = **1.6 m/s**（原版涉水/游泳显著减速）。
pub const SWIM_SPEED: f32 = 1.6;

/// 冲刺游泳水平稳态：sprinting 时阻力 0.9（LivingEntity.java:2460）→
/// 0.18 块/tick = **3.6 m/s**。
pub const SWIM_SPRINT_SPEED: f32 = 3.6;

/// 水中收敛系数（非冲刺水平与竖直同阻 0.8/tick）：k = −20·ln 0.8 =
/// 4.463/s（LivingEntity.java:2483 `multiply(slowDown, 0.8F, slowDown)`）。
pub const SWIM_DRAG_K: f32 = 4.463;

/// 冲刺游泳水平收敛系数：k = −20·ln 0.9 = 2.107/s。
pub const SWIM_SPRINT_DRAG_K: f32 = 2.107;

/// 游泳上浮稳态：按住跳每 tick +0.04（jumpInLiquid，LivingEntity.java:2362
/// -2364；深水走 aiStep 流体分支 :3032-3046）×0.8 阻力 − 重力 0.005/tick
/// （getFluidFallingAdjustedMovement :2633，g/16）→ (0.032−0.005)/0.2 =
/// 0.135 块/tick = **2.7 m/s**。水中**没有** jumpFromGround。
pub const SWIM_UP_SPEED: f32 = 2.7;

/// 下潜稳态：按住潜行每 tick −0.04（goDownInWater，LocalPlayer.java:855
/// -857）→ (−0.032−0.005)/0.2 = −0.185 块/tick = **−3.7 m/s**。
pub const SWIM_DOWN_SPEED: f32 = 3.7;

/// 中性沉浮稳态：−0.005/0.2 = −0.025 块/tick = **−0.5 m/s**（缓沉，
/// 不按任何键时）。
pub const SWIM_SINK_SPEED: f32 = 0.5;

/// 冲刺游泳视线转向系数：isSwimming 时 y 朝视线收敛（Player.travel
/// Player.java:1383-1392 `(lookY − v.y)×0.06/tick`，×0.8 阻力且冲刺免
/// 重力）→ 合成稳态 0.1935·lookY 块/tick = **3.87·lookY m/s**，
/// 合成收敛 k = −20·ln(0.8×0.94) = 5.71/s。俯冲/上仰跟随视角。
pub const SWIM_LOOK_GAIN: f32 = 3.87;
pub const SWIM_LOOK_STEER_K: f32 = 5.71;

/// 贴水面/撞墙出水：jumpOutOfFluid 置 vy = 0.3 块/tick = **6 m/s**
/// （LivingEntity.java:2506-2511；travelInWater 每 tick 结算：水平碰撞
/// 且抬升 0.6 后无碰撞 → 置位。水下按墙攀爬与跳出水面均源于此）。
pub const SWIM_EXIT_SPEED: f32 = 6.0;

/// 自动上台阶高度（maxUpStep）：STEP_HEIGHT 属性默认 **0.6** 块
/// （Attributes.java:85-86 `RangedAttribute("step_height", 0.6, 0.0, 10.0)`；
/// LivingEntity.maxUpStep :3911-3913 玩家无骑乘修正时直取该值）。
/// 半砖（0.5）可步行登上，整块（1.0）不可。
///
/// 派单说法「我们 0.5」与源码实况不符：本仓此前**没有**任何自动上台阶
/// 机制（0.5 是 `physics::MAX_SUBSTEP` 扫掠子步上限，语义无关）；本次
/// 按原版 `Entity.collide` 候选台阶高度重试（Entity.java:1080-1106 +
/// collectCandidateStepUpHeights :1111-1136）补上，取原版 0.6。
pub const STEP_HEIGHT: f32 = 0.6;

/// 固定物理步长：1/60 s。
pub const FIXED_DT: f32 = 1.0 / 60.0;
