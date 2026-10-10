//! Enchantments: definitions (cost curves, weights, exclusivity) and the
//! table selection algorithm. All constants from NOTES-2 §6
//! (Enchantments.java / EnchantmentHelper.java, MC 26.1).

/// Enchantment ids (registry order is stable).
pub const PROTECTION: u16 = 0;
pub const FEATHER_FALLING: u16 = 1;
pub const BLAST_PROTECTION: u16 = 2;
pub const FIRE_PROTECTION: u16 = 3;
pub const PROJECTILE_PROTECTION: u16 = 4;
pub const SHARPNESS: u16 = 5;
pub const SMITE: u16 = 6;
pub const BANE_OF_ARTHROPODS: u16 = 7;
pub const KNOCKBACK: u16 = 8;
pub const UNBREAKING: u16 = 9;
pub const MENDING: u16 = 10;
pub const EFFICIENCY: u16 = 11;
pub const FORTUNE: u16 = 12;
pub const LOOTING: u16 = 13;

/// Cost curve: cost(level) = base + per_level_above_first * (level - 1)
/// (Enchantment.java:660-672). min_level is always 1 in 26.1.
#[derive(Clone, Copy, Debug)]
pub struct Cost {
    pub base: u32,
    pub per_level_above_first: u32,
}

impl Cost {
    pub const fn at(self, level: u8) -> u32 {
        self.base + self.per_level_above_first * (level.saturating_sub(1) as u32)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EnchantDef {
    pub id: u16,
    pub name: &'static str,
    /// Weight in the table pick (1..=1024 range in MC; see per-ench values).
    pub weight: u32,
    pub max_level: u8,
    pub min_cost: Cost,
    pub max_cost: Cost,
    /// Anvil cost multiplier per level.
    pub anvil_cost: u8,
    /// Applies to tools/weapons (enchantable via table).
    pub table_allowed: bool,
}

/// 互斥组：同组不能共存（NOTES-2 §6 exclusive_set）。
pub fn exclusive_group(id: u16) -> u8 {
    match id {
        PROTECTION
        | FEATHER_FALLING
        | BLAST_PROTECTION
        | FIRE_PROTECTION
        | PROJECTILE_PROTECTION => 1, // ARMOR_EXCLUSIVE
        SHARPNESS | SMITE | BANE_OF_ARTHROPODS => 2, // DAMAGE_EXCLUSIVE
        _ => 0,
    }
}

/// 26.1 实抄值（NOTES-2 §6，Enchantments.java 行号见笔记）。
pub static ENCHANTS: [EnchantDef; 14] = [
    EnchantDef {
        id: PROTECTION,
        name: "protection",
        weight: 10,
        max_level: 4,
        min_cost: Cost {
            base: 1,
            per_level_above_first: 11,
        },
        max_cost: Cost {
            base: 12,
            per_level_above_first: 11,
        },
        anvil_cost: 1,
        table_allowed: true,
    },
    EnchantDef {
        id: FEATHER_FALLING,
        name: "feather_falling",
        weight: 5,
        max_level: 4,
        min_cost: Cost {
            base: 5,
            per_level_above_first: 6,
        },
        max_cost: Cost {
            base: 11,
            per_level_above_first: 6,
        },
        // feather_falling.json: "anvil_cost": 2（旧值 1 过期）
        anvil_cost: 2,
        table_allowed: true,
    },
    EnchantDef {
        id: BLAST_PROTECTION,
        name: "blast_protection",
        weight: 2,
        max_level: 4,
        min_cost: Cost {
            base: 5,
            per_level_above_first: 8,
        },
        // blast_protection.json: max_cost 13+8、anvil_cost 4（旧 17/2 过期）
        max_cost: Cost {
            base: 13,
            per_level_above_first: 8,
        },
        anvil_cost: 4,
        table_allowed: true,
    },
    EnchantDef {
        id: FIRE_PROTECTION,
        name: "fire_protection",
        weight: 5,
        max_level: 4,
        // fire_protection.json: min 10+8 / max 18+8 / anvil 2（旧 5/13/1 全套过期）
        min_cost: Cost {
            base: 10,
            per_level_above_first: 8,
        },
        max_cost: Cost {
            base: 18,
            per_level_above_first: 8,
        },
        anvil_cost: 2,
        table_allowed: true,
    },
    EnchantDef {
        id: PROJECTILE_PROTECTION,
        name: "projectile_protection",
        weight: 5,
        max_level: 4,
        min_cost: Cost {
            base: 3,
            per_level_above_first: 6,
        },
        // projectile_protection.json: max_cost 9+6、anvil_cost 2（旧 15/1 过期）
        max_cost: Cost {
            base: 9,
            per_level_above_first: 6,
        },
        anvil_cost: 2,
        table_allowed: true,
    },
    EnchantDef {
        id: SHARPNESS,
        name: "sharpness",
        weight: 10,
        max_level: 5,
        min_cost: Cost {
            base: 1,
            per_level_above_first: 11,
        },
        max_cost: Cost {
            base: 21,
            per_level_above_first: 11,
        },
        anvil_cost: 1,
        table_allowed: true,
    },
    EnchantDef {
        id: SMITE,
        name: "smite",
        weight: 5,
        max_level: 5,
        min_cost: Cost {
            base: 5,
            per_level_above_first: 8,
        },
        max_cost: Cost {
            base: 25,
            per_level_above_first: 8,
        },
        anvil_cost: 2,
        table_allowed: true,
    },
    EnchantDef {
        id: BANE_OF_ARTHROPODS,
        name: "bane_of_arthropods",
        weight: 5,
        max_level: 5,
        min_cost: Cost {
            base: 5,
            per_level_above_first: 8,
        },
        max_cost: Cost {
            base: 25,
            per_level_above_first: 8,
        },
        anvil_cost: 2,
        table_allowed: true,
    },
    EnchantDef {
        id: KNOCKBACK,
        name: "knockback",
        weight: 5,
        max_level: 2,
        min_cost: Cost {
            base: 5,
            per_level_above_first: 20,
        },
        // knockback.json: max_cost 55+20（旧 25 过期 → table 高档 knockback 永不可出）
        max_cost: Cost {
            base: 55,
            per_level_above_first: 20,
        },
        anvil_cost: 2,
        table_allowed: true,
    },
    EnchantDef {
        id: UNBREAKING,
        name: "unbreaking",
        weight: 5,
        max_level: 3,
        min_cost: Cost {
            base: 5,
            per_level_above_first: 8,
        },
        max_cost: Cost {
            base: 55,
            per_level_above_first: 8,
        },
        anvil_cost: 2,
        table_allowed: true,
    },
    EnchantDef {
        id: MENDING,
        name: "mending",
        weight: 2,
        max_level: 1,
        min_cost: Cost {
            base: 25,
            per_level_above_first: 25,
        },
        max_cost: Cost {
            base: 75,
            per_level_above_first: 25,
        },
        anvil_cost: 4,
        table_allowed: false,
    },
    EnchantDef {
        id: EFFICIENCY,
        name: "efficiency",
        weight: 10,
        max_level: 5,
        min_cost: Cost {
            base: 1,
            per_level_above_first: 10,
        },
        max_cost: Cost {
            base: 51,
            per_level_above_first: 10,
        },
        anvil_cost: 1,
        table_allowed: true,
    },
    EnchantDef {
        id: FORTUNE,
        name: "fortune",
        weight: 2,
        max_level: 3,
        min_cost: Cost {
            base: 15,
            per_level_above_first: 9,
        },
        // fortune.json: max_cost 65+9（旧 61 过期）
        max_cost: Cost {
            base: 65,
            per_level_above_first: 9,
        },
        anvil_cost: 4,
        table_allowed: true,
    },
    EnchantDef {
        id: LOOTING,
        name: "looting",
        weight: 2,
        max_level: 3,
        min_cost: Cost {
            base: 15,
            per_level_above_first: 9,
        },
        // looting.json: max_cost 65+9、anvil_cost 4（旧 61/2 过期）
        max_cost: Cost {
            base: 65,
            per_level_above_first: 9,
        },
        anvil_cost: 4,
        table_allowed: true,
    },
];

pub fn def(id: u16) -> &'static EnchantDef {
    &ENCHANTS[id as usize]
}

/// Table slot costs (EnchantmentHelper.java:554-559):
/// selected = rand(8)+1 + bookcases/2 + rand(bookcases+1)
/// slot0 = max(selected/3, 1); slot1 = selected*2/3 + 1;
/// slot2 = max(selected, bookcases*2); slot invalid if cost < slot+1.
/// 书架几何用经典 5×5 环形近似（26.1 常量未提取，见 NOTES-2 §8 TODO）。
pub fn slot_costs(bookcases: u32, rng: &mut impl FnMut() -> u32) -> [Option<u32>; 3] {
    let selected = 1 + rng() % 8 + bookcases / 2 + rng() % (bookcases + 1);
    let c0 = (selected / 3).max(1);
    let c1 = selected * 2 / 3 + 1;
    let c2 = selected.max(bookcases * 2);
    [
        Some(c0).filter(|c| *c >= 1),
        Some(c1).filter(|c| *c >= 2),
        Some(c2).filter(|c| *c >= 3),
    ]
}

/// selectEnchantment (EnchantmentHelper.java:550-581):
/// cost += 1 + rand(enchantable/4+1) + rand(enchantable/4+1);
/// :560-561 原版扰动为**乘性**——`randomSpan = (nextFloat()+nextFloat()-1)*0.15`
/// （∈[-0.15,+0.15]）后 `cost = clamp(round(cost + cost*randomSpan), 1, MAX)`，
/// 即整体 ×0.85..1.15；旧实现 `cost += span*0.15` 是加性 ±0.15（取整后几乎
/// 恒等），分布偏移，已改为乘性。（注：账本写的 "×0.75..1.0" 与 26.1 实读
/// 不符，按 EnchantmentHelper.java:560-561 实读 ×0.85..1.15 落地。）
/// candidates = defs whose [min_cost(l), max_cost(l)]
/// window contains cost (top level down); weighted pick; loop while
/// rand(50) <= cost, halving cost, removing exclusives.
pub fn select_enchantments(
    cost: u32,
    enchantable: u8,
    seed: u32,
    existing: &[u16],
    rng: &mut impl FnMut() -> u32,
) -> Vec<(u16, u8)> {
    let mut cost = cost as f32
        + 1.0
        + (rng() % (u32::from(enchantable) / 4 + 1)) as f32
        + (rng() % (u32::from(enchantable) / 4 + 1)) as f32;
    // EnchantmentHelper.java:560: span = (r1 + r2 - 1) * 0.15（三角分布）
    let span = (rng() as f32 / u32::MAX as f32 + rng() as f32 / u32::MAX as f32 - 1.0) * 0.15;
    // :561: cost + cost*span，round 后 clamp ≥1（乘性扰动，非加性）
    cost = (cost + cost * span).round().max(1.0);
    let mut cost = cost as u32;

    let mut taken_groups: Vec<u8> = existing.iter().map(|&id| exclusive_group(id)).collect();
    let mut out: Vec<(u16, u8)> = Vec::new();
    let mut first = true;
    loop {
        // candidates: for each table-allowed enchant, top level whose cost
        // window contains `cost`
        let mut candidates: Vec<(u16, u8, u32)> = Vec::new(); // (id, level, weight)
        for d in ENCHANTS.iter() {
            if !d.table_allowed
                || exclusive_group(d.id) != 0 && taken_groups.contains(&exclusive_group(d.id))
            {
                continue;
            }
            let mut level = d.max_level;
            let mut found = None;
            while level >= 1 {
                let lo = d.min_cost.at(level);
                let hi = d.max_cost.at(level);
                if cost >= lo && cost <= hi {
                    found = Some((d.id, level, d.weight));
                    break;
                }
                level -= 1;
            }
            if let Some(c) = found {
                candidates.push(c);
            }
        }
        if candidates.is_empty() {
            break;
        }
        // weighted pick
        let total: u32 = candidates.iter().map(|c| c.2).sum();
        if total == 0 {
            break;
        }
        let pick = if first { seed % total } else { rng() % total };
        let mut acc = 0u32;
        let mut chosen = candidates[0];
        for c in &candidates {
            acc += c.2;
            if pick < acc {
                chosen = *c;
                break;
            }
        }
        out.push((chosen.0, chosen.1));
        let g = exclusive_group(chosen.0);
        if g != 0 {
            taken_groups.push(g);
        }
        first = false;
        if rng() % 50 >= cost {
            break;
        }
        cost /= 2;
    }
    out
}
