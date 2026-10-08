//! 玩家物理常数。
//!
//! 出处：`/root/mc-ref/NOTES.md`（Minecraft 26.1 反编译换算，本地参考文件，
//! 不入库）。MC 逻辑 tick = 20/s，速度单位换算：1 块/tick = 20 m/s。

/// 重力加速度：MC 默认 gravity 0.08 块/tick² × 20² = 32 m/s²。
pub const GRAVITY: f32 = 32.0;

/// 竖直空气阻力系数（仅作用于下落，`vel.y < 0` 时 `vel.y *= exp(-k·dt)`）。
///
/// NOTES.md 两种换算口径：
/// - 按每秒阻尼比：0.98^20 ≈ 0.667/s → `k ≈ 0.4`（NOTES.md 原文 k_air≈0.4），
///   终端速度 ≈ 3.92 块/tick = 78.4 m/s，与 MC 一致；
/// - 按终端 3.92 直接取 `k = g/3.92 ≈ 8.16`：该口径把"块/tick"误作"m/s"，
///   终端仅 3.92 m/s，19 m 自由落体需 ~5 s（300+ 步），
///   无法满足 100 步落地（tests/physics.rs::free_fall_lands），弃用。
pub const AIR_DRAG_K: f32 = 0.4;

/// MC 等效终端下落速度 ≈ 3.92 块/tick = 78.4 m/s（AIR_DRAG_K 的渐近值）。
pub const TERMINAL_FALL_SPEED: f32 = 78.4;

/// 跳跃初速：MC BASE_JUMP_POWER 0.42 块/tick × 20 = 8.4 m/s。
pub const JUMP_SPEED: f32 = 8.4;

/// 行走速度：MC 属性 movement_speed 0.1 → 4.317 m/s。
pub const WALK_SPEED: f32 = 4.317;

/// 冲刺速度：5.612 m/s（StepInput 暂无 sprint 位，预留）。
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
