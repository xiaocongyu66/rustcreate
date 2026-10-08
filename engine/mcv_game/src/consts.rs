//! 玩家物理常数。
//!
//! 出处：仓库外反编译参照 `/root/mc-ref/src-26.1/`（Minecraft 26.1）与
//! 换算笔记 `/root/mc-ref/NOTES-physics.md`、`/root/mc-ref/NOTES.md`。
//! MC 逻辑 tick = 20/s，速度单位换算：1 块/tick = 20 m/s，1 块/tick² = 400 m/s²。

/// 重力加速度：MC gravity 属性默认 0.08 块/tick²（Entity.java:166、
/// Attributes.java:45）× 20² = 32 m/s²。
pub const GRAVITY: f32 = 32.0;

/// 竖直空气阻力系数（仅作用于下落，`vel.y < 0` 时 `vel.y *= exp(-k·dt)`）。
///
/// MC 原值：竖直阻力 0.98/tick（LivingEntity.travelInAir:2442，上升下落都乘），
/// 连续化 k = −20·ln 0.98 = 0.40402/s（0.98^20 ≈ 0.667/s）。
/// 现取 **0.4**，与精确值差 −1%（<15%，保留，见 NOTES-physics.md 常数对照表）。
/// 渐近终端 = g/k = 80 m/s，与 MC 精确终端 78.4 差 2%。
///
/// （NOTES.md 旧口径"按终端 3.92 直接取 k=g/3.92≈8.16"把块/tick 误作 m/s，
/// 终端仅 3.92 m/s，19 m 自由落体需 ~5 s，无法满足 100 步落地，弃用。）
pub const AIR_DRAG_K: f32 = 0.4;

/// MC 等效终端下落速度：MC 稳态 3.92 块/tick × 20 = 78.4 m/s
/// （= 0.08·0.98/(1−0.98) 的不动点）。本实现的渐近值 g/k = 80 m/s，差 2%，保留。
pub const TERMINAL_FALL_SPEED: f32 = 78.4;

/// 跳跃初速：MC BASE_JUMP_POWER / JUMP_STRENGTH 属性默认 0.42 块/tick × 20 = 8.4 m/s。
///
/// 峰高对照：MC tick 制离散积分（先位移后 `(v−0.08)·0.98`）= 1.2522 块；
/// 连续解析 v²/2g = 1.1025 m；本实现显式 Euler（本步先以完整初速位移）≈ 1.173 m，
/// 仍 > 1.0 可上一格台阶（tests/physics_calib.rs::jump_onto_one_block）。
pub const JUMP_SPEED: f32 = 8.4;

/// 行走速度：MC 属性 movement_speed 玩家值 0.1（Player.createAttributes:208）。
///
/// 26.1 反编译稳态推导：地面加速度 0.1·(0.216/0.6³)=0.1 块/tick，
/// 地面阻力 0.6·0.91=0.546/tick → 稳态 0.1/(1−0.546) = 0.2203 块/tick = 4.405 m/s；
/// 社区/旧版口径 4.317 m/s（差 2%，<15%），两者取保守值 **4.317** 保留。
pub const WALK_SPEED: f32 = 4.317;

/// 冲刺速度：步行 ×1.3（MC sprint 速度修饰 +30%）= 5.612 m/s（StepInput 暂无
/// sprint 位，预留）。
pub const SPRINT_SPEED: f32 = 5.612;

/// 潜行速度：MC 潜行 ≈ 1.3 m/s ≈ 行走的 0.3 倍。
pub const SNEAK_SPEED: f32 = WALK_SPEED * 0.3;

/// 飞行速度：10 m/s。
pub const FLY_SPEED: f32 = 10.0;

/// 飞行阻尼系数：`vel *= exp(-8·dt)` 并朝目标收敛。
pub const FLY_DAMP_K: f32 = 8.0;

/// 飞行竖直控制速度：jump 上升 / sneak 下降，±6 m/s。
pub const FLY_VERT_SPEED: f32 = 6.0;

/// 地面水平加速：60 m/s²，朝 wish 方向收敛。
pub const GROUND_ACCEL: f32 = 60.0;

/// 空中水平加速：8 m/s²，朝 wish 方向收敛。
pub const AIR_ACCEL: f32 = 8.0;

/// 水中重力：4 m/s²。
pub const WATER_GRAVITY: f32 = 4.0;

/// 水中下沉终端速度：2 m/s。
pub const WATER_SINK_SPEED: f32 = 2.0;

/// 水中跳跃上浮速度：3 m/s。
pub const WATER_RISE_SPEED: f32 = 3.0;

/// 固定物理步长：1/60 s。
pub const FIXED_DT: f32 = 1.0 / 60.0;
