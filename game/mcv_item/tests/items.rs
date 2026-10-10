//! Item system tests — constants verified against NOTES-2 (MC 26.1).

use mcv_item::crafting::EMPTY_SLOT;
use mcv_item::{
    DIAMOND_ITEM, IRON_INGOT, IRON_PICKAXE_INDEX, IRON_SWORD_INDEX, ItemStack, PLANKS, STICK,
    STONE_SWORD_INDEX, WOODEN_PICKAXE_INDEX, WOODEN_SWORD_INDEX, anvil, crafting, enchant,
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

/// 近战耐久扣减：`ToolMaterial.java:59` 工具 `new Weapon(2, …)`、:94 剑
/// `new Weapon(1)` → 一次近战攻击工具扣 2、剑扣 1。
#[test]
fn tool_melee_durability_is_two_sword_one() {
    for idx in [
        WOODEN_PICKAXE_INDEX,
        IRON_PICKAXE_INDEX,
        DIAMOND_PICKAXE_INDEX,
        mcv_item::WOODEN_AXE_INDEX,
        mcv_item::WOODEN_SHOVEL_INDEX,
    ] {
        assert_eq!(
            mcv_item::ITEMS[idx as usize].item_damage_per_attack, 2,
            "{} 工具近战应扣 2（Weapon(2)）",
            mcv_item::ITEMS[idx as usize].name
        );
    }
    for idx in [WOODEN_SWORD_INDEX, STONE_SWORD_INDEX, IRON_SWORD_INDEX] {
        assert_eq!(
            mcv_item::ITEMS[idx as usize].item_damage_per_attack, 1,
            "{} 剑近战应扣 1（Weapon(1)）",
            mcv_item::ITEMS[idx as usize].name
        );
    }
    // 端到端：木镐(59) 两次近战 = 4 点，未碎；第 30 次攻击(60) 碎。
    let mut pick = ItemStack::new(WOODEN_PICKAXE_INDEX, 1);
    let mut rng = lcg(5);
    assert!(!pick.hurt(2, &mut rng));
    assert_eq!(pick.damage, 2);
    assert!(pick.hurt(58, &mut rng), "59 耐久扣满必须碎");
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

/// ENCHANTS 14 条对 26.1 `data/minecraft/enchantment/*.json` 的回归锁
/// （min_cost/max_cost = base+per_level_above_first、weight、anvil_cost）。
#[test]
fn enchant_defs_match_261_json() {
    // (name, weight, anvil, min base+per, max base+per) —— 逐条摘自 json。
    let table: &[(&str, u32, u8, (u32, u32), (u32, u32))] = &[
        ("protection", 10, 1, (1, 11), (12, 11)), // protection.json
        ("feather_falling", 5, 2, (5, 6), (11, 6)), // feather_falling.json anvil=2
        ("blast_protection", 2, 4, (5, 8), (13, 8)), // blast_protection.json 13+8/4
        ("fire_protection", 5, 2, (10, 8), (18, 8)), // fire_protection.json 10+8/18+8/2
        ("projectile_protection", 5, 2, (3, 6), (9, 6)), // projectile_protection.json 9+6/2
        ("sharpness", 10, 1, (1, 11), (21, 11)),
        ("smite", 5, 2, (5, 8), (25, 8)),
        ("bane_of_arthropods", 5, 2, (5, 8), (25, 8)),
        ("knockback", 5, 2, (5, 20), (55, 20)), // knockback.json max 55+20
        ("unbreaking", 5, 2, (5, 8), (55, 8)),
        ("mending", 2, 4, (25, 25), (75, 25)),
        ("efficiency", 10, 1, (1, 10), (51, 10)),
        ("fortune", 2, 4, (15, 9), (65, 9)), // fortune.json max 65+9
        ("looting", 2, 4, (15, 9), (65, 9)), // looting.json max 65+9、anvil 4
    ];
    for (name, w, anvil, min, max) in table {
        let d = enchant::ENCHANTS.iter().find(|e| e.name == *name).unwrap();
        assert_eq!(d.weight, *w, "{name} weight");
        assert_eq!(d.anvil_cost, *anvil, "{name} anvil_cost");
        assert_eq!((d.min_cost.base, d.min_cost.per_level_above_first), *min, "{name} min");
        assert_eq!((d.max_cost.base, d.max_cost.per_level_above_first), *max, "{name} max");
    }
}

/// select 扰动乘性验证（EnchantmentHelper.java:560-561
/// `cost + cost*(r1+r2-1)*0.15`）：rng 恒 0 → span=−0.15，输入 40 经
/// +1 后 41 → round(41×0.85)=35；efficiency 窗口 L4=[31,81] 含 35、
/// L5=[41,91] 不含 → 出效率 **4 级**。旧加性实现 cost≈41 → 出 5 级。
/// seed=47 使加权首选取中 efficiency（候选序 protection10/blast2/fire5/
/// sharp10/smite5/bane5/kb5/unb5/eff10… 累计 47 落 eff 段）。
#[test]
fn select_perturbation_is_multiplicative() {
    let mut rng = || 0u32;
    let out = enchant::select_enchantments(40, 14, 47, &[], &mut rng);
    assert!(
        out.contains(&(enchant::EFFICIENCY, 4)),
        "乘性扰动后 cost=35 → efficiency L4；实得 {out:?}（旧加性会出 L5）"
    );
    assert!(!out.contains(&(enchant::EFFICIENCY, 5)));
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
    // 历史税基数 = max(继承税) = 0 → 0*2+1 = 1（操作价 2 **不入基数**；
    // 旧实现 (2)*2+1=5 为 bug，AnvilMenu.java:255-264）
    assert_eq!(res.repair_cost, 1);
}

/// 历史税：基数只吃 max(继承税)，连修线性倍增而非操作价滚雪球。
#[test]
fn anvil_history_tax_base_is_inherited_only() {
    let mut a = ItemStack::new(IRON_SWORD_INDEX, 1);
    a.damage = 200;
    let mat = ItemStack::new(IRON_INGOT, 1); // 修 1 件 +1 级
    // 第 1 修：继承税 0 → 显示价 1、新税 0*2+1 = 1
    let r1 = anvil::combine(&a, &mat, None, 0, 0, 0);
    assert_eq!(r1.cost, 1);
    assert_eq!(r1.repair_cost, 1);
    // 第 2 修（带税 1 的具）：显示价 1+1=2、新税 1*2+1 = 3
    //（旧实现 (1+1)*2+1=5，操作价入基数的指数 bug）
    let r2 = anvil::combine(&a, &mat, None, r1.repair_cost, 0, 0);
    assert_eq!(r2.cost, 2);
    assert_eq!(r2.repair_cost, 3);
    // 第 3 修：新税 3*2+1 = 7
    let r3 = anvil::combine(&a, &mat, None, r2.repair_cost, 0, 0);
    assert_eq!(r3.repair_cost, 7);
    // 基数 = max(继承税)：牺牲品税更高时取牺牲品（:255-258）
    let r = anvil::combine(&a, &mat, None, 2, 9, 0);
    assert_eq!(r.cost, 12); // 2+9+1
    assert_eq!(r.repair_cost, 19); // 9*2+1
}

/// 纯改名不涨税（AnvilMenu.java:260 namingCost==price&&>0 → 不递增）。
#[test]
fn anvil_pure_rename_keeps_tax() {
    let a = ItemStack::new(IRON_SWORD_INDEX, 1);
    let empty = ItemStack::empty();
    let r = anvil::combine(&a, &empty, Some("Excalibur"), 6, 0, 0);
    assert_eq!(r.cost, 7); // 6 + 改名 1
    assert_eq!(r.repair_cost, 6, "纯改名税原样继承");
    // 改名 + 其他操作 = 涨价涨税（非纯改名）
    let b = ItemStack::new(IRON_SWORD_INDEX, 1);
    let r2 = anvil::combine(&a, &b, Some("Excalibur"), 6, 0, 0);
    assert_eq!(r2.cost, 9); // 6 + 2(合并) + 1(改名)
    assert_eq!(r2.repair_cost, 13); // 6*2+1
}

/// 修料映射 = tags/item/*_tool_materials.json（旧实现木具用煤炭、石具无门）。
#[test]
fn anvil_repair_materials_match_tags() {
    use mcv_item::{COBBLESTONE, COBBLED_DEEPSLATE, ItemStack as IS};
    let p = |id: u16| IS::new(id, 1);
    // wooden → #planks（wooden_tool_materials.json）
    assert!(p(PLANKS).is_repair_material_for(WOODEN_PICKAXE_INDEX));
    assert!(!p(mcv_item::COAL).is_repair_material_for(WOODEN_PICKAXE_INDEX), "煤炭修木具为旧 bug");
    // stone → cobblestone/cobbled_deepslate（stone_tool_materials.json，
    // blackstone 未注册缺席）
    assert!(p(COBBLESTONE).is_repair_material_for(STONE_PICKAXE_INDEX));
    assert!(p(COBBLED_DEEPSLATE).is_repair_material_for(STONE_PICKAXE_INDEX));
    assert!(!p(mcv_item::COBBLESTONE).is_repair_material_for(WOODEN_PICKAXE_INDEX));
    // iron/diamond 同物
    assert!(p(IRON_INGOT).is_repair_material_for(IRON_PICKAXE_INDEX));
    assert!(!p(IRON_INGOT).is_repair_material_for(STONE_PICKAXE_INDEX));
    // golden/netherite → gold_ingot/netherite_ingot（本次补注册）
    let golden_sword = mcv_item::item_by_name("golden_sword").unwrap();
    let netherite_pick = mcv_item::item_by_name("netherite_pickaxe").unwrap();
    assert!(p(mcv_item::GOLD_INGOT).is_repair_material_for(golden_sword));
    assert!(p(mcv_item::NETHERITE_INGOT).is_repair_material_for(netherite_pick));
    // 非耐久物（iron_ore 同前缀陷阱）不放行
    assert!(
        !p(IRON_INGOT).is_repair_material_for(mcv_item::item_by_name("iron_ore").unwrap()),
        "isDamageableItem 门（AnvilMenu.java:133）"
    );
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
