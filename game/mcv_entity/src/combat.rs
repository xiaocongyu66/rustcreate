//! Combat math: Player.attack pipeline + LivingEntity.hurt + CombatRules.
//! All formulas transcribed from NOTES-2 §3 (26.1 line refs).

use mcv_item::ItemStack;

/// Attack cooldown scale (26.1 Player.getAttackStrengthScale:1803-1805,
/// delay = 20/attackSpeed via getCurrentItemAttackStrengthDelay:1793-1795):
/// (ticker + 0.5) / (20 / attackSpeed), clamped 0..1. `ticker` counts ticks
/// (20/s); full recovery takes 20/attackSpeed ticks (1/attackSpeed s).
pub fn attack_strength(ticker: f32, attack_speed: f32) -> f32 {
    ((ticker + 0.5) / (20.0 / attack_speed)).clamp(0.0, 1.0)
}

/// Damage scaling with cooldown (Player.baseDamageScaleFactor:1185-1188):
/// 0.2 + scale² × 0.8.
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

/// Invulnerability frames (LivingEntity.java:1196-1206):
/// during invulnerableTime > 10, if new damage <= lastHurt → no damage;
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

/// 受击击退向量（LivingEntity.knockback:1612-1630，26.1 verbatim）：
/// `p = power × (1 − knockback_resistance)`；
/// `vel' = (vel.x/2 − d.x·p,  on_ground ? min(0.4, vel.y/2 + p) : vel.y,
///           vel.z/2 − d.z·p)`，d = 攻击方向水平单位向量（指向目标）。
/// 玩家全力冲刺命中 power = 0.5（Player.java:966,987 causeExtraKnockback；
/// ATTACK_KNOCKBACK 属性另加）。方向退化为零向量时原版用随机数，这里保持不动。
pub fn knockback_velocity(
    cur: glam::Vec3,
    on_ground: bool,
    knockback_resist: f32,
    power: f32,
    push_dir_xz: glam::Vec3,
) -> glam::Vec3 {
    let power = power * (1.0 - knockback_resist.clamp(0.0, 1.0));
    if power <= 0.0 {
        return cur;
    }
    let mut d = push_dir_xz;
    d.y = 0.0;
    let len = d.length();
    if len < 1e-6 {
        return cur;
    }
    let v = d / len * power;
    glam::Vec3::new(
        cur.x / 2.0 - v.x,
        if on_ground {
            (cur.y / 2.0 + power).min(0.4)
        } else {
            cur.y
        },
        cur.z / 2.0 - v.z,
    )
}

/// Full melee attack resolution (attacker → target), per Player.attack.
pub struct AttackContext {
    pub attacker_pos_eye: glam::Vec3,
    pub weapon: Option<ItemStack>,
    /// Current attack cooldown ticker — **ticks** (20/s)，对应 26.1
    /// `Player.attackStrengthTicker`（Player.java:267 每 tick +1；
    /// 满蓄力 delay = 20/attackSpeed tick，Player.java:1793-1795）。
    /// 生产侧（GameRuntime）必须按 on_tick 累加，绝不按秒或 60 Hz 步。
    pub cooldown_ticker: f32,
    pub fall_distance: f32,
    pub on_ground: bool,
    pub in_water: bool,
    pub sprinting: bool,
    /// ATTACK_DAMAGE 的加值修饰 Σ（力量 +3×(amp+1)、虚弱 −4×(amp+1)，
    /// MobEffects.java:41-45/:71-75 ADD_VALUE）——在冷却缩放**之前**并入基础
    /// 伤害（Player.attack:945-950 属性值即含效果修饰）。
    pub attack_damage_bonus: f32,
    /// ATTACK_SPEED 乘子 Π(1+amount)（急迫 +0.1×(amp+1)、挖掘疲劳 −0.1×
    /// (amp+1)，MobEffects.java:29-40 ADD_MULTIPLIED_TOTAL）——作用于冷却
    /// delay = 20/attackSpeed 的分母。
    pub attack_speed_mult: f32,
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
    let scale = attack_strength(
        ctx.cooldown_ticker,
        weapon.attacks_per_second() * ctx.attack_speed_mult,
    );
    let base = 1.0 + weapon.full_attack() + ctx.attack_damage_bonus; // player 1.0 + item attribute + 效果加值
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
/// ECS 化后收散点可变引用（对应 `Health`/`MobTicks::invulnerable`/`LastHurt`
/// 组件）+ `MobDef::armor`；结算数学与原 `&mut Mob` 版逐行等价。
///
/// 无敌帧回写分岔（26.1 LivingEntity.java:1196-1206）：只有全额分支
/// （invulnerableTime ≤ 10）才置 `invulnerableTime = 20`（:1206）；差值分支
/// （:1200-1202 扣 damage−lastHurt）**不重置**无敌帧——旧实现无条件置 20，
/// 连续强击会把 i 帧无限续期。
pub fn apply_hurt(
    health: &mut f32,
    invulnerable: &mut u32,
    last_hurt: &mut f32,
    armor: f32,
    incoming: f32,
    total_protection: u32,
) -> Option<f32> {
    let full_branch = *invulnerable <= 10;
    invulnerable_gate(*invulnerable, *last_hurt, incoming).map(|dmg| {
        let after_armor = damage_after_armor(dmg, armor, 0.0);
        let after_prot = damage_after_protection(after_armor, total_protection);
        *health -= after_prot;
        if full_branch {
            *invulnerable = 20;
        }
        *last_hurt = incoming;
        after_prot
    })
}
