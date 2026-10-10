//! Combat math tests — formulas verified against NOTES-2 §3 (26.1 source).

use glam::Vec3;
use mcv_entity::MobId;
use mcv_entity::combat::*;
use mcv_item::ItemStack;

#[test]
fn cooldown_scaling() {
    // full recovery of an iron sword (1.6/s) takes 20/1.6 = 12.5 ticks
    assert!((attack_strength(12.0, 1.6) - 1.0).abs() < 1e-4);
    // right after a swing: (0+0.5)/12.5 = 0.04
    assert!((attack_strength(0.0, 1.6) - 0.04).abs() < 1e-4);
    // damage scaling: full = 1.0x, minimum 0.2x
    assert!((cooldown_damage_scale(1.0) - 1.0).abs() < 1e-4);
    assert!((cooldown_damage_scale(0.0) - 0.2).abs() < 1e-4);
}

#[test]
fn armor_formula_exact() {
    // CombatRules: toughness = 2 + t/4; realArmor = clamp(armor - dmg/t, 0.2a, 20)
    // full iron (armor 15, toughness 0): realArmor = clamp(15 - d/2, 3, 20)
    let d = damage_after_armor(10.0, 15.0, 0.0);
    // real = clamp(15-5, 3, 20) = 10 → 10 * 0.6 = 6.0
    assert!((d - 6.0).abs() < 1e-4);
    let d = damage_after_armor(2.0, 15.0, 0.0);
    // real = clamp(15-1, 3, 20) = 14 → 2 * (1-14/25) = 0.88
    assert!((d - 0.88).abs() < 1e-3);
    // floor: armor*0.2
    let d = damage_after_armor(100.0, 15.0, 0.0);
    // real = clamp(15-50, 3, 20) = 3 → 100 * (1-3/25) = 88
    assert!((d - 88.0).abs() < 1e-3);
}

#[test]
fn protection_clamps() {
    assert!((damage_after_protection(10.0, 20) - 2.0).abs() < 1e-4);
    assert!(
        (damage_after_protection(10.0, 100) - 2.0).abs() < 1e-4,
        "clamp 0..20"
    );
    assert!((damage_after_protection(10.0, 0) - 10.0).abs() < 1e-4);
}

#[test]
fn critical_conditions() {
    assert!(is_critical(0.5, false, false, false));
    assert!(!is_critical(0.5, true, false, false), "on ground");
    assert!(!is_critical(0.5, false, false, true), "sprinting");
    assert!(!is_critical(0.5, false, true, false), "in water");
    assert!(!is_critical(0.0, false, false, false), "no fall");
}

#[test]
fn iron_sword_full_hit_on_zombie() {
    // player 1.0 + iron sword 5.0 attribute, full cooldown
    let weapon = ItemStack::new(mcv_item::IRON_SWORD_INDEX, 1);
    let ctx = AttackContext {
        attacker_pos_eye: Vec3::ZERO,
        weapon: Some(weapon),
        cooldown_ticker: 20.0,
        fall_distance: 0.0,
        on_ground: true,
        in_water: false,
        sprinting: false,
        attack_damage_bonus: 0.0,
        attack_speed_mult: 1.0,
    };
    let out = resolve_attack(&ctx);
    assert!((out.damage - 6.0).abs() < 1e-4, "iron sword full hit = 6");
    assert!(!out.critical);

    // 力量 I（+3 ADD_VALUE）、急迫 II（攻速 ×1.2，MobEffects.java:41-45/:29-33）：
    // 加值在冷却缩放之前并入基础伤害（Player.attack:945-950 属性值即含效果修饰）。
    let buffed = AttackContext {
        attack_damage_bonus: 3.0,
        attack_speed_mult: 1.2,
        ..ctx
    };
    let out = resolve_attack(&buffed);
    assert!((out.damage - 9.0).abs() < 1e-4, "6 + 3 = 9");
    // 虚弱 I（−4，MobEffects.java:71-75）按序并入。
    let weak = AttackContext {
        attack_damage_bonus: -4.0,
        ..buffed
    };
    let out = resolve_attack(&weak);
    assert!((out.damage - 5.0).abs() < 1e-4, "9 − 4 = 5");

    // crit: falling → 9.0
    let ctx = AttackContext {
        fall_distance: 1.0,
        on_ground: false,
        ..ctx
    };
    let out = resolve_attack(&ctx);
    assert!(out.critical);
    assert!((out.damage - 9.0).abs() < 1e-4, "crit = 6 × 1.5");
}

#[test]
fn zombie_hurt_iframes() {
    // ECS 形状：受击状态 = 散点可变引用（Health/MobTicks::invulnerable/LastHurt）。
    let def = MobId::ZOMBIE.def();
    let mut health = def.health;
    let mut invulnerable = 0u32;
    let mut last_hurt = 0.0f32;
    let d1 = apply_hurt(
        &mut health,
        &mut invulnerable,
        &mut last_hurt,
        def.armor,
        6.0,
        0,
    )
    .expect("first hit lands");
    // CombatRules: toughness=2, real = clamp(2 - 6/2, 2*0.2, 20) = 0.4 (floor)
    assert!(
        (d1 - 6.0 * (1.0 - 0.4 / 25.0)).abs() < 1e-4,
        "zombie armor 2 with floor"
    );
    // within i-frames (>10 ticks), weaker hit ignored
    assert!(
        apply_hurt(
            &mut health,
            &mut invulnerable,
            &mut last_hurt,
            def.armor,
            1.0,
            0
        )
        .is_none()
    );
    // stronger hit only applies the difference
    let d2 = apply_hurt(
        &mut health,
        &mut invulnerable,
        &mut last_hurt,
        def.armor,
        10.0,
        0,
    )
    .expect("difference lands");
    // damage 4: real = clamp(2 - 4/2, 0.4, 20) = 0.4 → 4 * (1-0.4/25)
    assert!((d2 - 4.0 * (1.0 - 0.4 / 25.0)).abs() < 1e-4);
    // 结算写回：血量扣两次伤害、无敌帧 20、last_hurt 记最大一刀。
    assert!((health - (20.0 - d1 - d2)).abs() < 1e-4);
    assert_eq!(invulnerable, 20);
    assert!((last_hurt - 10.0).abs() < 1e-6);
}

#[test]
fn armor_durability() {
    assert_eq!(armor_durability_loss(10.0), 2);
    assert_eq!(armor_durability_loss(1.0), 1, "min 1");
}

#[test]
fn spawn_light_rules() {
    let mut rng = lcg(5);
    // surface at night: sky 15 passes rand(32) sometimes, but overall rand(8)
    // fails → deep night surface spawning is rare but possible; cave: sky 0
    for _ in 0..200 {
        assert!(
            !mcv_entity::spawner::light_allows_hostile(0, 5, &mut rng),
            "block light > 0 rejects"
        );
    }
    let any = (0..500).any(|_| mcv_entity::spawner::light_allows_hostile(0, 0, &mut rng));
    assert!(any, "cave darkness must allow monsters");
}

fn lcg(seed: u64) -> impl FnMut() -> u32 {
    let mut s = seed | 1;
    move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
    }
}
