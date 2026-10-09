//! 难度（26.1 `net.minecraft.world.Difficulty`，审计 N-5 的数据面 + 伤害
//! 曲线接线）。旧实现难度恒普通（game.rs Percept 里写死 2）且伤害结算
//! 不分难度。
//!
//! 源码依据（类名:行号均为 26.1 反编译实读）：
//! - **枚举**：`Difficulty.java:8-13` — PEACEFUL(0)/EASY(1)/NORMAL(2)/
//!   HARD(3)，`getId`（:28-30）。
//! - **玩家受伤缩放**：`Player.hurtServer`（Player.java:692-706）——仅
//!   `source.scalesWithDifficulty()` 的伤害源参与：
//!   `WHEN_CAUSED_BY_LIVING_NON_PLAYER` = 造成者是 LivingEntity 且非玩家
//!   （DamageSource.java:92-97）→ 本仓 mob 近战/箭/爆炸（Some(src)）适用，
//!   摔落/虚空（无来源实体）不适用。倍率：
//!   - PEACEFUL → 0；
//!   - EASY → `min(d/2 + 1, d)`；
//!   - NORMAL → 不变；
//!   - HARD → `d × 3/2`。
//!   **不存在** `Mob.getAttackDamageScale`（全树 grep 无此符号，派单说法
//!   有误）——难度对 mob 伤害的影响就集中在 Player.hurtServer 这一处。
//! - **和平清怪**：`Mob.checkDespawn`（Mob.java:656-658）— PEACEFUL 且
//!   非 isAllowedInPeaceful → 立即 discard。
//! - **和平不索敌**：`LivingEntity.canBeSeenAsEnemy`（LivingEntity.java:928）
//!   — PEACEFUL 时玩家不可为敌人。
//! - **和平不刷怪**：`Monster.checkMonsterSpawnRules`（Monster.java:104）
//!   — spawner 库层已有 `SpawnConfig.peaceful` 门（spawner.rs:287），运行时
//!   try_natural_spawn 接线归主控（spawn 区不归本任务）。
//! - **饥饿侧**：`FoodData.tick`（FoodData.java:38-51）——和平不因
//!   exhaustion 掉饥饿；饥饿 0 掉血门 `health>10 || HARD ||
//!   (health>1 && NORMAL)`（EASY 即 >10，PEACEFUL 不掉）。
//! - **寻路侧**：`Mob.getMaxFallDistance`（Mob.java:806）难度收紧坠落
//!   意愿——见 mcv_entity::pathfinding::max_fall_distance。

/// 难度（Difficulty.java:8-13）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    /// Difficulty.java:28-30。
    pub fn id(self) -> u8 {
        match self {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        }
    }

    /// `Difficulty.byId`（Difficulty.java:35-37，越界 WRAP——本仓 clamp）。
    pub fn by_id(id: u8) -> Self {
        match id.min(3) {
            0 => Difficulty::Peaceful,
            1 => Difficulty::Easy,
            2 => Difficulty::Normal,
            _ => Difficulty::Hard,
        }
    }

    /// PEACEFUL？（Mob.java:656 和平清怪 / Monster.java:104 和平不刷）。
    pub fn is_peaceful(self) -> bool {
        self == Difficulty::Peaceful
    }
}

/// `Player.hurtServer`（Player.java:692-706）难度缩放，仅
/// scalesWithDifficulty 伤害源适用（DamageSource.java:92-97：造成者
/// LivingEntity 且非玩家）。
pub fn scale_entity_damage(amount: f32, difficulty: Difficulty) -> f32 {
    match difficulty {
        Difficulty::Peaceful => 0.0,
        Difficulty::Easy => (amount / 2.0 + 1.0).min(amount),
        Difficulty::Normal => amount,
        Difficulty::Hard => amount * 3.0 / 2.0,
    }
}

/// `FoodData.tick`（FoodData.java:48-51）饥饿 0 掉血门：
/// `health>10 || HARD || (health>1 && NORMAL)`。
pub fn starve_can_hurt(health: f32, difficulty: Difficulty) -> bool {
    match difficulty {
        Difficulty::Hard => true,
        Difficulty::Normal => health > 1.0,
        // EASY/PEACEFUL：>10 才掉；PEACEFUL 走不到（和平不消耗饥饿）。
        _ => health > 10.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Difficulty.java:8-30 — id 双射。
    #[test]
    fn ids_roundtrip() {
        for i in 0..=3u8 {
            assert_eq!(Difficulty::by_id(i).id(), i);
        }
        assert_eq!(Difficulty::by_id(9), Difficulty::Hard); // clamp
    }

    /// Player.java:692-706 — 逐难度倍率。
    #[test]
    fn damage_scaling() {
        let d = |x: f32, diff: Difficulty| scale_entity_damage(x, diff);
        assert_eq!(d(3.0, Difficulty::Peaceful), 0.0);
        // EASY: min(d/2+1, d) —— d=3 → 2.5；d=1 → 1（抬升到下限 1 的语义）
        assert_eq!(d(3.0, Difficulty::Easy), 2.5);
        assert_eq!(d(1.0, Difficulty::Easy), 1.0);
        assert_eq!(d(4.0, Difficulty::Easy), 3.0);
        assert_eq!(d(3.0, Difficulty::Normal), 3.0);
        assert_eq!(d(3.0, Difficulty::Hard), 4.5);
    }

    /// FoodData.java:48-51 — 饥饿掉血难度门。
    #[test]
    fn starve_gate() {
        assert!(!starve_can_hurt(10.0, Difficulty::Peaceful));
        assert!(!starve_can_hurt(10.0, Difficulty::Easy));
        assert!(starve_can_hurt(10.01, Difficulty::Easy));
        assert!(!starve_can_hurt(1.0, Difficulty::Normal));
        assert!(starve_can_hurt(1.01, Difficulty::Normal));
        assert!(starve_can_hurt(0.5, Difficulty::Hard));
    }
}
