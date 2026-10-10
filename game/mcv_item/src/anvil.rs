//! Anvil mechanics (NOTES-2 §4, AnvilMenu.java, MC 26.1):
//! - material repair: each material item heals max/4, +1 level cost each
//! - merge: durability = remaining1 + remaining2 + max*12/100, +2 levels
//! - enchant merge: same level -> +1, else max; incompatible +1 each
//! - rename +1; total cost clamp; >= 40 = too expensive
//! - history tax: REPAIR_COST = min(max(继承税) * 2 + 1, MAX)——操作价
//!   不入基数、纯改名不涨税（AnvilMenu.java:129/255-264/276-278）

use crate::enchant;
use crate::{ENCHANTED_BOOK, ITEMS, ItemStack};

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
    // op_price = 原版 `price`（AnvilMenu.createResult 的局部 price）：
    // 仅操作价。历史税基数只用继承税 max（:255-262），操作价**不入基数**
    // ——旧实现把含操作价的总 cost 送进 ×2+1，连修指数爆炸。
    let mut op_price = 0u32;
    let mut out = target.clone();
    let target_def = &ITEMS[target.item as usize];

    // Rename.
    if let Some(n) = name {
        let _ = n; // display name storage lands with inventory UI
        op_price += 1; // :226 namingCost = 1
    }

    // Repair with material (sacrifice is a plain material item).
    if sacrifice.is_repair_material_for(target.item) {
        let max = target.max_damage();
        let heal_per = max / 4;
        let mut heal = 0u32;
        for _ in 0..sacrifice.count {
            heal += u32::from(heal_per);
            op_price += 1; // :145 price++ 每件材料 +1
        }
        out.damage = out
            .damage
            .saturating_sub(heal.min(u32::from(out.damage)) as u16);
        return finish(out, op_price, target_tax, sacrifice_tax, name);
    }

    // Merge two items of the same type (or enchanted book onto item).
    if sacrifice.item != 0 {
        if sacrifice.item == ENCHANTED_BOOK {
            op_price = op_price.saturating_add(apply_enchants(&mut out, &sacrifice.enchants, true));
        } else if sacrifice.item == target.item && target_def.max_damage > 0 {
            // durability merge: remaining1 + remaining2 + 12% of max
            let rem1 = u32::from(target.max_damage() - target.damage);
            let rem2 = u32::from(sacrifice.max_damage() - sacrifice.damage);
            let merged = rem1 + rem2 + u32::from(target.max_damage()) * 12 / 100;
            out.damage = target
                .max_damage()
                .saturating_sub(merged.min(u32::from(target.max_damage())) as u16);
            op_price += 2; // :169 price += 2
            // enchant merge
            op_price =
                op_price.saturating_add(apply_enchants(&mut out, &sacrifice.enchants, false));
        } else {
            return AnvilResult {
                output: None,
                cost: 0,
                repair_cost: 0,
            };
        }
    }

    finish(out, op_price, target_tax, sacrifice_tax, name)
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

/// 原版 `AnvilMenu.calculateIncreasedRepairCost`（AnvilMenu.java:276-278）：
/// `min(base * 2 + 1, 2147483647)`。
///
/// 注：账本/任务写作 `min(…, 40)`，26.1 实读组件上限是 int MAX——
/// 40 是「too expensive」操作门（:250 `cost >= 40 → 拒`），不是 REPAIR_COST
/// 组件钳位（原版 NBT 里 repair_cost 可 >40，两件高税具合并可滚出 61/123）。
/// 按 AnvilMenu.java:277 实读落地 saturating（u32 域内等价 int MAX）。
pub fn calculate_increased_repair_cost(base: u32) -> u32 {
    base.saturating_mul(2).saturating_add(1)
}

/// `op_price` = 操作价（不计税基）；显示价 = 继承税之和 + 操作价
/// （AnvilMenu.java:129 `tax += input税+addition税`、:236 `clamp(tax+price)`）。
/// 输出税基 = max(继承税)（:255-258），纯改名不涨税
/// （:260 `namingCost != price || namingCost == 0` 才递增）。
fn finish(
    out: ItemStack,
    op_price: u32,
    target_tax: u32,
    sacrifice_tax: u32,
    name: Option<&str>,
) -> AnvilResult {
    let cost = target_tax
        .saturating_add(sacrifice_tax)
        .saturating_add(op_price);
    if cost >= TOO_EXPENSIVE {
        return AnvilResult {
            output: None,
            cost,
            repair_cost: 0,
        };
    }
    // history tax: min(max(继承税) * 2 + 1, MAX)，纯改名（操作价仅改名 1 级）
    // 时税原样继承。
    let pure_rename = name.is_some() && op_price == 1;
    let repair_cost = if pure_rename {
        target_tax.max(sacrifice_tax)
    } else {
        calculate_increased_repair_cost(target_tax.max(sacrifice_tax))
    };
    // The tax travels with the item (carried by inventory UI wiring in M8).
    AnvilResult {
        output: Some(out),
        cost,
        repair_cost,
    }
}

impl ItemStack {
    /// Material repair tags（`ToolMaterial.java:23-31` 第七参 `repairItems`
    /// = ItemTags.*_TOOL_MATERIALS，`Item.Properties.repairable(tag)`；
    /// 成员实证 data/minecraft/tags/item/*_tool_materials.json）：
    /// - wooden → `#minecraft:planks`（木具用**木板**修，旧实现误用 COAL）
    /// - stone → cobblestone / blackstone / cobbled_deepslate（石具原本
    ///   无任何材料，挖而无门；blackstone 未注册物品，按「宁缺勿错」缺席）
    /// - iron / diamond → 同物材料
    /// - gold → gold_ingot、netherite → netherite_ingot（本次补注册）
    /// - copper → copper_ingot（本引擎无铜工具，物品亦未注册，天然缺席）
    pub fn is_repair_material_for(&self, target: u16) -> bool {
        use crate::{
            COBBLED_DEEPSLATE, COBBLESTONE, DIAMOND_ITEM, GOLD_INGOT, IRON_INGOT, NETHERITE_INGOT,
            PLANKS,
        };
        // 原版语义：仅 damageable（带 durability 组件）物品有 repairable
        // 门（AnvilMenu.java:133 `result.isDamageableItem() &&
        // input.isValidRepairItem`），"iron_ore"/"diamond_ore" 等非工具
        // 同名前缀不得放行。
        if ITEMS[target as usize].max_damage == 0 {
            return false;
        }
        let mat: &[u16] = match ITEMS[target as usize].name {
            n if n.starts_with("wooden_") => &[PLANKS],
            n if n.starts_with("stone_") => &[COBBLESTONE, COBBLED_DEEPSLATE],
            n if n.starts_with("iron_") => &[IRON_INGOT],
            n if n.starts_with("diamond_") => &[DIAMOND_ITEM],
            n if n.starts_with("golden_") => &[GOLD_INGOT],
            n if n.starts_with("netherite_") => &[NETHERITE_INGOT],
            _ => return false,
        };
        mat.contains(&self.item)
    }
}
