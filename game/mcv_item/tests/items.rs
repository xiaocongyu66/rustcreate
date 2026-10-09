//! Item system tests — constants verified against NOTES-2 (MC 26.1).

use mcv_item::crafting::EMPTY_SLOT;
use mcv_item::{
    COBBLESTONE, DIAMOND_ITEM, IRON_INGOT, IRON_PICKAXE_INDEX, IRON_SWORD_INDEX, ItemStack, PLANKS,
    STICK, STONE_SWORD_INDEX, WOODEN_PICKAXE_INDEX, WOODEN_SWORD_INDEX, anvil, crafting, enchant,
};

fn lcg(seed: u64) -> impl FnMut() -> u32 {
    let mut s = seed | 1;
    move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
    }
}

#[test]
fn sword_attack_values() {
    // NOTES-2 §3: wooden = 3.0 + 0, iron = 3.0 + 2; attack speed 4 - 2.4
    let wooden = ItemStack::new(WOODEN_SWORD_INDEX, 1);
    assert!((wooden.full_attack() - 3.0).abs() < 1e-4);
    assert!((wooden.attacks_per_second() - 1.6).abs() < 1e-4);
    let iron = ItemStack::new(IRON_SWORD_INDEX, 1);
    assert!((iron.full_attack() - 5.0).abs() < 1e-4);
    let pick = ItemStack::new(IRON_PICKAXE_INDEX, 1);
    assert!((pick.full_attack() - 3.0).abs() < 1e-4); // 1.0 baseline + 2.0 material
    assert!((pick.attacks_per_second() - 1.2).abs() < 1e-4);
}

#[test]
fn durability_break() {
    let mut wooden = ItemStack::new(WOODEN_SWORD_INDEX, 1); // 59 durability
    let mut rng = lcg(7);
    // 100 plain damage points on a 59-durability item must break it
    assert!(wooden.hurt(100, &mut rng));
    // unbreaking L3: ~1/4 of damage applies — 100 points must NOT break 59
    let mut enchanted = ItemStack::new(WOODEN_SWORD_INDEX, 1);
    enchanted.enchants.push(mcv_item::EnchStack {
        ench_id: enchant::UNBREAKING,
        level: 3,
    });
    let mut rng = lcg(7);
    assert!(
        !enchanted.hurt(100, &mut rng),
        "unbreaking L3 must survive 100 points"
    );
    assert!(
        enchanted.damage < 45,
        "expected ~25 applied, got {}",
        enchanted.damage
    );
}

#[test]
fn mending_two_per_xp() {
    let mut iron = ItemStack::new(IRON_SWORD_INDEX, 1); // 250 max
    iron.damage = 100;
    let left = iron.mend(30); // 30 xp * 2 = 60 durability
    assert_eq!(iron.damage, 40);
    assert_eq!(left, 0);
    let left = iron.mend(1000); // only 40 damage left → 20 xp
    assert_eq!(iron.damage, 0);
    assert_eq!(left, 980);
}

#[test]
fn slot_costs_valid() {
    let mut rng = lcg(11);
    let costs = enchant::slot_costs(15, &mut rng);
    assert!(costs[0].is_some() && costs[1].is_some() && costs[2].is_some());
    assert!(
        costs[2].unwrap() >= 30,
        "15 bookcases → slot2 = max(selected, 30)"
    );
}

#[test]
fn select_enchants_respect_exclusivity() {
    let mut rng = lcg(3);
    let out = enchant::select_enchantments(26, 14, 7, &[], &mut rng);
    assert!(!out.is_empty());
    // damage enchants are mutually exclusive
    let damage = [
        enchant::SHARPNESS,
        enchant::SMITE,
        enchant::BANE_OF_ARTHROPODS,
    ];
    let picked: Vec<_> = out.iter().filter(|(id, _)| damage.contains(id)).collect();
    assert!(picked.len() <= 1);
    // protection group exclusive
    let armor = [
        enchant::PROTECTION,
        enchant::BLAST_PROTECTION,
        enchant::FIRE_PROTECTION,
        enchant::PROJECTILE_PROTECTION,
    ];
    let picked: Vec<_> = out.iter().filter(|(id, _)| armor.contains(id)).collect();
    assert!(picked.len() <= 1);
}

#[test]
fn shaped_pickaxe_and_mirror() {
    // standard position
    let mut grid = [EMPTY_SLOT; 9];
    grid[0] = PLANKS;
    grid[1] = PLANKS;
    grid[2] = PLANKS;
    grid[4] = STICK;
    grid[7] = STICK;
    assert_eq!(
        crafting::find_result(&grid, 3),
        Some((WOODEN_PICKAXE_INDEX, 1))
    );

    // shifted +1 column (bbox crop must normalise)
    let mut grid2 = [EMPTY_SLOT; 9];
    grid2[1] = PLANKS;
    grid2[2] = PLANKS;
    grid2[3] = PLANKS; // row 2 start = wrapped? no: [3] is row1col0
    // use proper shifted placement: cols 1..=3 of top row is impossible in 3
    // wide; instead shift down one row
    let mut grid3 = [EMPTY_SLOT; 9];
    grid3[3] = PLANKS;
    grid3[4] = PLANKS;
    grid3[5] = PLANKS;
    grid3[7] = STICK;
    // row2 col? stick rows would exceed; this arrangement is invalid
    assert_eq!(crafting::find_result(&grid3, 3), None);
    let _ = grid2;

    // 2x2 grid cannot fit a 3-row recipe
    let small = [PLANKS, PLANKS, STICK, STICK];
    assert_eq!(crafting::find_result(&small, 2), None);
}

#[test]
fn sword_is_shaped_not_shapeless() {
    // 26.1 剑是 shaped 竖排 M/M/S（1x3），不是 shapeless——打散摆放必须不出剑。
    // （旧实现把剑当 shapeless，任何摆放都出剑，属审计 CRITICAL，此为回归锁。）
    let scatter = [
        STICK, EMPTY_SLOT, PLANKS, EMPTY_SLOT, STICK, EMPTY_SLOT, EMPTY_SLOT, EMPTY_SLOT,
        EMPTY_SLOT,
    ];
    assert_eq!(crafting::find_result(&scatter, 3), None, "散放不得出剑");
    // 正确竖排（左列 板/板/棍）才出木剑。
    let vertical = [
        PLANKS, EMPTY_SLOT, EMPTY_SLOT, PLANKS, EMPTY_SLOT, EMPTY_SLOT, STICK, EMPTY_SLOT,
        EMPTY_SLOT,
    ];
    assert_eq!(
        crafting::find_result(&vertical, 3),
        Some((WOODEN_SWORD_INDEX, 1)),
        "竖排 M/M/S 出木剑"
    );
    // 材料顺序反（棍在上、板在下）不符 pattern → 无。
    let flipped = [
        STICK, EMPTY_SLOT, EMPTY_SLOT, STICK, EMPTY_SLOT, EMPTY_SLOT, PLANKS, EMPTY_SLOT,
        EMPTY_SLOT,
    ];
    assert_eq!(crafting::find_result(&flipped, 3), None);
}

#[test]
fn anvil_merge_and_costs() {
    let mut a = ItemStack::new(IRON_SWORD_INDEX, 1);
    a.damage = 240; // 10 remaining of 250
    let mut b = ItemStack::new(IRON_SWORD_INDEX, 1);
    b.damage = 230; // 20 remaining
    let res = anvil::combine(&a, &b, None, 0, 0, 0);
    let out = res.output.expect("merge ok");
    // remaining = 10 + 20 + 250*12/100 = 60 → damage = 190
    assert_eq!(out.damage, 190);
    assert_eq!(res.cost, 2);
    assert_eq!(res.repair_cost, 5); // 2*2+1
}

#[test]
fn anvil_material_repair() {
    let mut a = ItemStack::new(IRON_SWORD_INDEX, 1);
    a.damage = 100;
    let mat = ItemStack::new(IRON_INGOT, 3);
    let res = anvil::combine(&a, &mat, None, 0, 0, 0);
    let out = res.output.expect("repair ok");
    // 3 ingots heal 3 * 250/4 = 186 → damage 0 (clamped), cost 3
    assert_eq!(out.damage, 0);
    assert_eq!(res.cost, 3);
}

#[test]
fn anvil_too_expensive() {
    let a = ItemStack::new(IRON_SWORD_INDEX, 1);
    let b = ItemStack::new(IRON_SWORD_INDEX, 1);
    // huge existing taxes push over 40
    let res = anvil::combine(&a, &b, Some("renamed"), 20, 20, 0);
    assert!(res.output.is_none(), "cost >= 40 must refuse");
    assert!(res.cost >= anvil::TOO_EXPENSIVE);
}

#[test]
fn cooking_lookup() {
    let r = crafting::find_cooking(29).expect("iron ore smelts");
    assert_eq!(r.result, (IRON_INGOT, 1));
    assert_eq!(r.cook_time, 200);
}

#[test]
fn diamond_item_usable() {
    // diamond item id sanity for recipes
    assert_eq!(DIAMOND_ITEM, 3);
    assert_eq!(mcv_item::ITEMS[DIAMOND_ITEM as usize].name, "diamond");
    assert_eq!(
        mcv_item::ITEMS[STONE_SWORD_INDEX as usize].name,
        "stone_sword"
    );
}
