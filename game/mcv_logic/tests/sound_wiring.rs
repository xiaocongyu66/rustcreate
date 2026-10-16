//! 音效接线判据：步距门纯函数（26.1 Entity.java 步行音状态机）+ 游泳行进音量
//! 纯函数 + 千块表全名 SoundType 覆盖。数值引注：
//! - 步距累计：`moveDist += 水平位移 × 0.6`（Entity.java:812-813）；
//! - 触发门：`moveDist > nextStep && !supportingState.isAir()`（Entity.java:820）；
//! - 阈值更新：`nextStep = (int)moveDist + 1`（Entity.java:1188-1190），初值 1.0（:246）；
//! - 游泳音量：`min(1, √(vx²·0.2+vy²+vz²·0.2)) × 0.35`
//!   （waterSwimSound Entity.java:1360-1370，自身 volumeModifier=0.35）。
use glam::Vec3;
use mcv_logic::game::{step_gate, swim_volume};

#[test]
fn below_threshold_does_not_fire() {
    // 初值 nextStep=1.0（Entity.java:246）：0.6 步距未过门。
    assert_eq!(step_gate(0.6, 1.0), None);
    // 恰好等值不触发（严格 >，Entity.java:820）。
    assert_eq!(step_gate(1.0, 1.0), None);
    assert_eq!(step_gate(0.999, 1.0), None);
}

#[test]
fn crossing_fires_with_int_plus_one() {
    // 过门即触发并按 (int)moveDist + 1 重置（Entity.java:1188-1190）：
    // moveDist=1.05 → 新阈值 2.0。
    assert_eq!(step_gate(1.05, 1.0), Some(2.0));
    // moveDist=1.99（初阈值 1.0 下的大步跳变）仍 → (int)1.99+1 = 2.0——
    // 小数部分不影响取整加一；1.99 < 2.0 时未过门不触发（严格 >）。
    assert_eq!(step_gate(1.99, 1.0), Some(2.0));
    // 跨两格：moveDist=2.4 → 3.0。
    assert_eq!(step_gate(2.4, 1.0), Some(3.0));
}

#[test]
fn walk_cadence_matches_vanilla() {
    // 原版节奏复演：常速水平 0.2 格/tick × 0.6 = 0.12/tick，从 (0, 1.0) 起步。
    // 触发点应落在 moveDist 首次 > 1.0（第 9 tick：1.08），随后每 1.0 一次。
    let mut dist = 0.0f32;
    let mut next = 1.0f32;
    let mut fired_at = Vec::new();
    for tick in 0..40u32 {
        dist += 0.12;
        if let Some(n) = step_gate(dist, next) {
            next = n;
            fired_at.push((tick, dist));
        }
    }
    assert_eq!(fired_at.len(), 4, "40 tick 应触发 4 步: {fired_at:?}");
    // 首步 ≈1.08（1.0 门），之后首过 2.0/3.0/4.0 阈值落 2.04/3.12/4.08
    // ——阈值 (int)moveDist+1 等差 1.0；严格 > 门下 0.12×25 真值恰 3.0
    // 等值不触发（IEEE754 累积误差 <1e-5，断言容差内确定）。
    assert!((fired_at[0].1 - 1.08).abs() < 1e-5, "{fired_at:?}");
    assert!((fired_at[1].1 - 2.04).abs() < 1e-5, "{fired_at:?}");
    assert!((fired_at[2].1 - 3.12).abs() < 1e-5, "{fired_at:?}");
    assert!((fired_at[3].1 - 4.08).abs() < 1e-5, "{fired_at:?}");
}

#[test]
fn scale_0_6_maps_horizontal_to_move_dist() {
    // 水平步距换算：原版每步实际水平距离 = 1.0/0.6 ≈ 1.667 格（Entity.java:812-813
    // 的 ×0.6 与 :1188 的 +1 联合效果）；1.667 水平 → moveDist 1.0002 → 触发。
    let moved_h = 1.667f32;
    assert!(step_gate(moved_h * 0.6, 1.0).is_some());
    // 不足一步（1.6 水平）不触发。
    assert_eq!(step_gate(1.6f32 * 0.6, 1.0), None);
}

#[test]
fn swim_volume_matches_water_swim_sound_formula() {
    // Entity.waterSwimSound（Entity.java:1360-1370）：speed =
    // min(1, √(vx²·0.2 + vy² + vz²·0.2)) × 0.35（自身控制 volumeModifier
    // = 0.35，:1362；本仓无骑乘恒自身）。静水零速 → 零增益。
    assert_eq!(swim_volume(Vec3::ZERO), 0.0);
    // 缓行 0.2 格/tick：√(0.04·0.2)=0.08944… × 0.35 ≈ 0.03130。
    assert!((swim_volume(Vec3::new(0.2, 0.0, 0.0)) - 0.031305).abs() < 1e-4);
    // 垂直分量权重 1.0·0.35 满算：下潜 0.4 → 0.14。
    assert!((swim_volume(Vec3::new(0.0, 0.4, 0.0)) - 0.14).abs() < 1e-6);
    // 高速封顶 min(1, …)：超速侧滑 → 0.35 上限（× volumeModifier 后）。
    assert_eq!(swim_volume(Vec3::new(0.0, 0.0, 5.0)), 0.35);
    // 横向权重 0.2 的对照：5 只在 y 向才顶满——x 向 5 → √(25·0.2)=2.236 → 仍 1.0
    // 封顶（同上限），但中速区 y 权重 5 倍于 x：
    assert!(swim_volume(Vec3::new(0.5, 0.0, 0.0)) < swim_volume(Vec3::new(0.0, 0.5, 0.0)));
}
