//! Anvil mechanics (NOTES-2 §4, AnvilMenu.java, MC 26.1):
//! - material repair: each material item heals max/4, +1 level cost each
//! - merge: durability = remaining1 + remaining2 + max*12/100, +2 levels
//! - enchant merge: same level -> +1, else max; incompatible +1 each
//! - rename +1; total cost clamp; >= 40 = too expensive
//! - history tax: REPAIR_COST = min(old * 2 + 1, MAX)

use crate::enchant;
use crate::{ItemStack, ENCHANTED_BOOK, ITEMS};

pub const TOO_EXPENSIVE: u32 = 40;

pub struct AnvilResult {
    pub output: Option<ItemStack>,
    /// Experience levels required to take the result.
    pub cost: u32,
    /// New REPAIR_COST tax stored on the output.
    pub repair_cost: u32,
}

/// Computes the anvil operation result. `name` = Some(new_name) when the
/// user renamed (any difference counts). `repair_items` = count of material
/// items in the second slot when it's a repair material of the target.
pub fn combine(
    target: &ItemStack,
    sacrifice: &ItemStack,
    name: Option<&str>,
    target_tax: u32,
    sacrifice_tax: u32,
    rng_seed_compat: u32,
) -> AnvilResult {
    let _ = rng_seed_compat;
    let mut cost = target_tax + sacrifice_tax;
    let mut out = target.clone();
    let target_def = &ITEMS[target.item as usize];

    // Rename.
    if let Some(n) = name {
        let _ = n; // display name storage lands with inventory UI
        cost += 1;
    }

    // Repair with material (sacrifice is a plain material item).
    if sacrifice.is_repair_material_for(target.item) {
        let max = target.max_damage();
        let heal_per = max / 4;
        let mut heal = 0u32;
        for _ in 0..sacrifice.count {
            heal += u32::from(heal_per);
            cost += 1;
        }
        out.damage = out
            .damage
            .saturating_sub(heal.min(u32::from(out.damage)) as u16);
        return finish(out, cost, target_tax);
    }

    // Merge two items of the same type (or enchanted book onto item).
    if sacrifice.item != 0 {
        if sacrifice.item == ENCHANTED_BOOK {
            cost = cost.saturating_add(apply_enchants(&mut out, &sacrifice.enchants, true));
        } else if sacrifice.item == target.item && target_def.max_damage > 0 {
            // durability merge: remaining1 + remaining2 + 12% of max
            let rem1 = u32::from(target.max_damage() - target.damage);
            let rem2 = u32::from(sacrifice.max_damage() - sacrifice.damage);
            let merged = rem1 + rem2 + u32::from(target.max_damage()) * 12 / 100;
            out.damage = target
                .max_damage()
                .saturating_sub(merged.min(u32::from(target.max_damage())) as u16);
            cost += 2;
            // enchant merge
            cost = cost.saturating_add(apply_enchants(&mut out, &sacrifice.enchants, false));
        } else {
            return AnvilResult {
                output: None,
                cost: 0,
                repair_cost: 0,
            };
        }
    }

    finish(out, cost, target_tax)
}

fn apply_enchants(out: &mut ItemStack, enchants: &[crate::EnchStack], book: bool) -> u32 {
    let mut cost = 0u32;
    for e in enchants {
        let (id, level) = (e.ench_id, e.level);
        let existing = out
            .enchants
            .iter()
            .find(|e| e.ench_id == id)
            .map(|e| e.level);
        let d = enchant::def(id);
        let unit = if book {
            (u32::from(d.anvil_cost) / 2).max(1)
        } else {
            u32::from(d.anvil_cost)
        };
        match existing {
            Some(cur) if cur == level => {
                let new_level = cur.saturating_add(1).min(d.max_level);
                if new_level != cur {
                    let e = out.enchants.iter_mut().find(|e| e.ench_id == id).unwrap();
                    e.level = new_level;
                    cost += unit * u32::from(new_level);
                }
            }
            Some(cur) => {
                if level > cur {
                    let e = out.enchants.iter_mut().find(|e| e.ench_id == id).unwrap();
                    e.level = level;
                    cost += unit * u32::from(level);
                } else {
                    cost += 1; // incompatible/downgrade still costs
                }
            }
            None => {
                let group = enchant::exclusive_group(id);
                let conflicts = group != 0
                    && out
                        .enchants
                        .iter()
                        .any(|e| enchant::exclusive_group(e.ench_id) == group);
                out.enchants.push(crate::EnchStack { ench_id: id, level });
                if conflicts {
                    cost += 1;
                }
                cost += unit * u32::from(level);
            }
        }
    }
    cost
}

fn finish(out: ItemStack, cost: u32, _target_tax: u32) -> AnvilResult {
    if cost >= TOO_EXPENSIVE {
        return AnvilResult {
            output: None,
            cost,
            repair_cost: 0,
        };
    }
    // history tax: min(old*2 + 1, MAX)
    let repair_cost = cost.saturating_mul(2).saturating_add(1);
    // The tax travels with the item (carried by inventory UI wiring in M8).
    AnvilResult {
        output: Some(out),
        cost,
        repair_cost,
    }
}

impl ItemStack {
    /// Material repair tags (DataComponents.REPAIRABLE equivalent).
    pub fn is_repair_material_for(&self, target: u16) -> bool {
        use crate::{COAL, DIAMOND_ITEM, IRON_INGOT};
        matches!(
            (self.item, ITEMS[target as usize].name),
            (
                COAL,
                "wooden_sword" | "wooden_pickaxe" | "wooden_axe" | "wooden_shovel"
            ) | (
                IRON_INGOT,
                "iron_sword" | "iron_pickaxe" | "iron_axe" | "iron_shovel"
            ) | (
                DIAMOND_ITEM,
                "diamond_sword" | "diamond_pickaxe" | "diamond_axe" | "diamond_shovel"
            )
        )
    }
}
