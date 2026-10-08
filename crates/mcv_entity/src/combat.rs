//! Combat math: Player.attack pipeline + LivingEntity.hurt + CombatRules.
//! All formulas transcribed from NOTES-2 §3 (26.1 line refs).

use mcv_item::ItemStack;

/// Attack cooldown scale (Player.attackStrengthScale:1710-1722):
/// (ticker + 0.5) / (20 / attackSpeed), clamped 0..1. `ticker` counts ticks
/// (20/s); full recovery takes 20/attackSpeed ticks (1/attackSpeed s).
pub fn attack_strength(ticker: f32, attack_speed: f32) -> f32 {
    ((ticker + 0.5) / (20.0 / attack_speed)).clamp(0.0, 1.0)
}

/// Damage scaling with cooldown (Player.java:936): 0.2 + scale² × 0.8.
pub fn cooldown_damage_scale(scale: f32) -> f32 {
    0.2 + scale * scale * 0.8
}

/// Critical hit conditions (Player.canCriticalAttack:1008-1017):
/// fallDistance > 0, not on ground, not climbing, not in water, not
/// sprinting. Damage ×1.5 (:950-953).
pub fn is_critical(fall_distance: f32, on_ground: bool, in_water: bool, sprinting: bool) -> bool {
    fall_distance > 0.0 && !on_ground && !in_water && !sprinting
}

/// Sweep attack (doSweepAttack:1124-1147): 1 + SWEEPING_DAMAGE_RATIO × base
/// damage to targets within bbox inflate(1.0, 0.25, 1.0) and dist² < 9.
pub fn sweep_damage(base_damage: f32, sweeping_ratio: f32) -> f32 {
    1.0 + sweeping_ratio * base_damage
}

/// Armor reduction (CombatRules.java:16-32, 26.1 verbatim):
/// toughness = 2 + armorToughness / 4
/// realArmor = clamp(armor - damage / toughness, armor * 0.2, 20)
/// damage *= 1 - realArmor / 25
pub fn damage_after_armor(damage: f32, armor: f32, armor_toughness: f32) -> f32 {
    let toughness = 2.0 + armor_toughness / 4.0;
    let real = (armor - damage / toughness).clamp(armor * 0.2, 20.0);
    damage * (1.0 - real / 25.0)
}

/// Protection enchant reduction (CombatRules.java:34-37 + LivingEntity
/// :1926-1935): total protection clamped 0..20, damage × (1 - total/25).
pub fn damage_after_protection(damage: f32, total_protection: u32) -> f32 {
    damage * (1.0 - total_protection.min(20) as f32 / 25.0)
}

/// Invulnerability frames (LivingEntity:1207-1214):
/// during invulnerableTime=20, if new damage <= lastHurt → no damage;
/// otherwise only (new - lastHurt) applies.
pub fn invulnerable_gate(invulnerable_ticks: u32, last_hurt: f32, incoming: f32) -> Option<f32> {
    if invulnerable_ticks > 10 {
        if incoming <= last_hurt {
            None
        } else {
            Some(incoming - last_hurt)
        }
    } else {
        Some(incoming)
    }
}

/// Armor durability loss on hit (LivingEntity:1874-1886): max(1, damage/4).
pub fn armor_durability_loss(damage: f32) -> u16 {
    (damage / 4.0).max(1.0) as u16
}

/// Full melee attack resolution (attacker → target), per Player.attack.
pub struct AttackContext {
    pub attacker_pos_eye: glam::Vec3,
    pub weapon: Option<ItemStack>,
    /// Current attack cooldown ticker (seconds accumulated).
    pub cooldown_ticker: f32,
    pub fall_distance: f32,
    pub on_ground: bool,
    pub in_water: bool,
    pub sprinting: bool,
}

pub struct AttackOutcome {
    pub damage: f32,
    pub critical: bool,
    pub sweep: bool,
    /// Weapon durability to consume (1 sword, 2 tools).
    pub weapon_damage: u16,
}

pub fn resolve_attack(ctx: &AttackContext) -> AttackOutcome {
    let weapon = ctx.weapon.clone().unwrap_or_else(ItemStack::empty);
    let scale = attack_strength(ctx.cooldown_ticker, weapon.attacks_per_second());
    let base = 1.0 + weapon.full_attack(); // player 1.0 + item attribute
    let mut damage = base * cooldown_damage_scale(scale);
    let crit = scale > 0.9
        && is_critical(
            ctx.fall_distance,
            ctx.on_ground,
            ctx.in_water,
            ctx.sprinting,
        );
    if crit {
        damage *= 1.5;
    }
    let sweep = scale > 0.9 && !crit && ctx.on_ground;
    if sweep {
        damage += 0.0; // sweep is extra damage to *other* targets, see sweep_damage
    }
    AttackOutcome {
        damage,
        critical: crit,
        sweep,
        weapon_damage: u16::from(weapon.def().item_damage_per_attack).max(if weapon.is_empty() {
            0
        } else {
            1
        }),
    }
}

/// Applies hurt to a target mob (armor + protection + i-frames).
pub fn apply_hurt(mob: &mut super::Mob, incoming: f32, total_protection: u32) -> Option<f32> {
    let Some(dmg) = invulnerable_gate(mob.invulnerable, mob.last_hurt, incoming) else {
        return None;
    };
    let after_armor = damage_after_armor(dmg, mob.def().armor, 0.0);
    let after_prot = damage_after_protection(after_armor, total_protection);
    mob.health -= after_prot;
    mob.invulnerable = 20;
    mob.last_hurt = incoming;
    Some(after_prot)
}
