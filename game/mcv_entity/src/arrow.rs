//! 箭伤害曲线（26.1 `AbstractArrow` + `RangedAttackGoal`，审计后遗留项）。
//! 旧实现的箭伤害是发射时定值（power×2.0），丢了命中时**按当前速度缩放**
//! 的公式 → 近失箭伤害虚低、远射箭不衰减。全函数纯可测。
//!
//! 源码依据（类名:行号均为 26.1 反编译实读）：
//! - **命中量**：`AbstractArrow.onHitEntity`（AbstractArrow.java:421-431）
//!   `damage = ceil(clamp(速度模长 × baseDamage, 0, i32::MAX))`——速度取
//!   **命中瞬间**的 deltaMovement（弹道随重力/空气衰减 → 远射伤害降）。
//! - **怪射基础伤害**：`setBaseDamageFromMob`（AbstractArrow.java:718-720）
//!   `base = power × 2.0 + triangle(难度id × 0.11, 0.57425)`。
//! - **power**：`RangedAttackGoal.tick`（RangedAttackGoal.java:92-93）
//!   `power = clamp(dist / attackRadius, 0.1, 1.0)`（skeleton attackRadius
//!   = 15，AbstractSkeleton.java:55）。
//! - **triangle 噪声**：`RandomSource.triangle`（RandomSource.java:59-61）
//!   `mean + spread × (r1 − r2)`（三角分布）。
//! - **暴击**：`onHitEntity`（AbstractArrow.java:434-437）
//!   `damage += random.nextInt(damage / 2 + 2)`（满蓄力弓 pow==1 时 isCrit，
//!   BowItem.java:41）。

use glam::Vec3;

/// `AbstractSkeleton.java:55` — RangedBowAttackGoal attackRadius = 15.0F。
pub const SKELETON_ATTACK_RADIUS: f32 = 15.0;

/// `BowItem.java:38` — pow < 0.1 直接取消不放箭。
pub const BOW_MIN_POWER: f32 = 0.1;

/// 命中伤害（AbstractArrow.java:421-431）：`ceil(clamp(v×base, 0, i32MAX))`。
/// 返回整数点数（原版即 int 伤害）。v 为命中瞬间速度（格/tick）。
pub fn hit_damage(vel: Vec3, base_damage: f32) -> u32 {
    let pow = vel.length();
    let d = (pow * base_damage).clamp(0.0, i32::MAX as f32);
    d.ceil() as u32
}

/// 暴击加成（AbstractArrow.java:434-437）：`+ rand(damage/2 + 2)`。
pub fn crit_bonus(damage: u32, rng: &mut impl FnMut() -> u32) -> u32 {
    damage + rng() % (damage / 2 + 2)
}

/// `RangedAttackGoal.java:92-93` — `power = clamp(dist/radius, 0.1, 1.0)`。
pub fn ranged_power(dist: f32, attack_radius: f32) -> f32 {
    (dist / attack_radius).clamp(0.1, 1.0)
}

/// `RandomSource.triangle`（RandomSource.java:59-61）：
/// `mean + spread × (r1 − r2)`（rng 产 u32 均匀分布）。
pub fn triangle(mean: f32, spread: f32, rng: &mut impl FnMut() -> u32) -> f32 {
    let u = |r: u32| r as f32 / u32::MAX as f32;
    mean + spread * (u(rng()) - u(rng()))
}

/// `setBaseDamageFromMob`（AbstractArrow.java:718-720）：怪射箭的
/// `base = power×2.0 + triangle(难度id×0.11, 0.57425)`。
pub fn mob_base_damage(power: f32, difficulty_id: u8, rng: &mut impl FnMut() -> u32) -> f32 {
    power * 2.0 + triangle(difficulty_id.min(3) as f32 * 0.11, 0.57425, rng)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(seed: u64) -> impl FnMut() -> u32 {
        let mut s = seed | 1;
        move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as u32
        }
    }

    /// AbstractArrow.java:421-431 — ceil(v×base)：满速 skeleton 箭
    /// （v=1.6、base=2.0）= 4；速度衰减到一半 → ceil(1.6) = 2。
    #[test]
    fn hit_damage_scales_with_velocity() {
        let v = Vec3::new(1.6, 0.0, 0.0);
        assert_eq!(hit_damage(v, 2.0), 4);
        assert_eq!(hit_damage(v * 0.5, 2.0), 2);
        // base≤0 → ceil(0)=0。
        assert_eq!(hit_damage(v, 0.0), 0);
    }

    /// RangedAttackGoal.java:92-93 — clamp(dist/15, 0.1, 1.0)。
    #[test]
    fn ranged_power_clamps() {
        assert_eq!(ranged_power(1.5, SKELETON_ATTACK_RADIUS), 0.1);
        assert_eq!(ranged_power(7.5, SKELETON_ATTACK_RADIUS), 0.5);
        assert_eq!(ranged_power(30.0, SKELETON_ATTACK_RADIUS), 1.0);
    }

    /// AbstractArrow.java:718-720 — base = power×2 + triangle(diff×0.11, .57425)：
    /// 均值 ≈ power×2 + diff×0.11；三角分布界 ±0.57425。
    #[test]
    fn mob_base_damage_mean_and_bounds() {
        for seed in 1..40u64 {
            let mut r = det(seed * 7919);
            let b = mob_base_damage(1.0, 2, &mut r);
            // 2 + 0.22 ± 0.57425 → (1.65, 2.80)。
            assert!((1.6..=2.8).contains(&b), "base {b}");
        }
        // 和平(0)比困难(3)均值低 0.33。
        let mut r = det(42);
        let peaceful = mob_base_damage(1.0, 0, &mut r);
        let mut r = det(42);
        let hard = mob_base_damage(1.0, 3, &mut r);
        assert!(hard > peaceful, "难度越高均值越高 {peaceful} < {hard}");
    }

    /// AbstractArrow.java:434-437 — 暴击 = damage + rand(damage/2+2)。
    #[test]
    fn crit_bonus_in_range() {
        let mut r = det(7);
        for _ in 0..50 {
            let b = crit_bonus(4, &mut r);
            assert!((4..=7).contains(&b), "crit {b}"); // rand(4) ∈ 0..3
        }
    }

    /// RandomSource.java:59-61 — triangle 均值落在 mean 附近。
    #[test]
    fn triangle_centered() {
        let mut r = det(11);
        let mut acc = 0.0f32;
        for _ in 0..400 {
            acc += triangle(0.33, 0.57425, &mut r);
        }
        let mean = acc / 400.0;
        assert!((0.23..=0.43).contains(&mean), "mean {mean}");
    }
}
