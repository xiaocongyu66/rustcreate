//! Crafting: shaped (bbox-cropped, symmetric/mirror) + shapeless (multiset)
//! matching, cooking types, and a starter recipe table. Engine per
//! NOTES-2 §5 (ShapedRecipePattern/ShapelessRecipe/CraftingInput).

use crate::ItemStack;

/// Ingredient: item id list (0 = empty cell in shaped patterns).
pub type Ingredient = &'static [u16];

pub struct ShapedRecipe {
    pub id: &'static str,
    /// Pattern rows (' ' = empty); equal width rows, ≤3.
    pub pattern: &'static [&'static str],
    /// key char -> ingredient list (' ' reserved).
    pub keys: &'static [(char, Ingredient)],
    pub result: (u16, u8),
}

pub struct ShapelessRecipe {
    pub id: &'static str,
    pub ingredients: &'static [Ingredient],
    pub result: (u16, u8),
}

pub struct CookingRecipe {
    pub id: &'static str,
    pub input: Ingredient,
    pub result: (u16, u8),
    /// ticks: smelting 200, blasting/smoking/campfire 100.
    pub cook_time: u16,
}

use crate::{
    COBBLESTONE, DIAMOND_ITEM, DIAMOND_ORE_ITEM, IRON_INGOT, IRON_ORE_ITEM, LOG, PLANKS, STICK,
};

/// Grid empty-cell marker（物品 id 0 = STICK 是合法物品，空格只能用哨兵；
/// shaped 图案里的空格是 ' '）。
pub const EMPTY_SLOT: u16 = u16::MAX;

// Item ids in the mcv_item registry:
const ING_PLANKS: &[u16] = &[PLANKS];
const ING_COBBLE: &[u16] = &[COBBLESTONE];
const ING_IRON: &[u16] = &[IRON_INGOT];
const ING_DIAMOND: &[u16] = &[DIAMOND_ITEM];
const ING_STICK: &[u16] = &[STICK];

const I_STONE_SWORD: u16 = 8;
const I_IRON_SWORD: u16 = 9;
const I_DIAMOND_SWORD: u16 = 10;
const I_WOOD_SWORD: u16 = 7;
const I_WOOD_PICK: u16 = 13;
const I_STONE_PICK: u16 = 14;
const I_IRON_PICK: u16 = 15;
const I_DIAMOND_PICK: u16 = 16;

/// 26.1 数据包配方对齐（data/minecraft/recipe/*.json）：
/// 镐 = MMM/_S_/_S_；剑 = M/M/S 竖排 1x3（2x2 放不下，必须 3x3 工作台）；
/// 木棍 = 两板竖排 ×4；原木 → 4 板（shapeless）。
pub static SHAPED: [ShapedRecipe; 9] = [
    ShapedRecipe {
        id: "stick",
        pattern: &["M", "M"],
        keys: &[('M', ING_PLANKS)],
        result: (STICK, 4),
    },
    // 剑：材料×2 在上、棍在底（wooden/stone/iron/diamond_sword.json 同型）。
    ShapedRecipe {
        id: "wooden_sword",
        pattern: &["M", "M", "S"],
        keys: &[('M', ING_PLANKS), ('S', ING_STICK)],
        result: (I_WOOD_SWORD, 1),
    },
    ShapedRecipe {
        id: "stone_sword",
        pattern: &["M", "M", "S"],
        keys: &[('M', ING_COBBLE), ('S', ING_STICK)],
        result: (I_STONE_SWORD, 1),
    },
    ShapedRecipe {
        id: "iron_sword",
        pattern: &["M", "M", "S"],
        keys: &[('M', ING_IRON), ('S', ING_STICK)],
        result: (I_IRON_SWORD, 1),
    },
    ShapedRecipe {
        id: "diamond_sword",
        pattern: &["M", "M", "S"],
        keys: &[('M', ING_DIAMOND), ('S', ING_STICK)],
        result: (I_DIAMOND_SWORD, 1),
    },
    // pickaxe: MMM / _S_ / _S_
    ShapedRecipe {
        id: "wooden_pickaxe",
        pattern: &["MMM", " S ", " S "],
        keys: &[('M', ING_PLANKS), ('S', ING_STICK)],
        result: (I_WOOD_PICK, 1),
    },
    ShapedRecipe {
        id: "stone_pickaxe",
        pattern: &["MMM", " S ", " S "],
        keys: &[('M', ING_COBBLE), ('S', ING_STICK)],
        result: (I_STONE_PICK, 1),
    },
    ShapedRecipe {
        id: "iron_pickaxe",
        pattern: &["MMM", " S ", " S "],
        keys: &[('M', ING_IRON), ('S', ING_STICK)],
        result: (I_IRON_PICK, 1),
    },
    ShapedRecipe {
        id: "diamond_pickaxe",
        pattern: &["MMM", " S ", " S "],
        keys: &[('M', ING_DIAMOND), ('S', ING_STICK)],
        result: (I_DIAMOND_PICK, 1),
    },
];

pub static SHAPELESS: [ShapelessRecipe; 1] = [ShapelessRecipe {
    id: "planks",
    ingredients: &[&[LOG]],
    result: (PLANKS, 4),
}];

pub static COOKING: [CookingRecipe; 2] = [
    CookingRecipe {
        id: "iron_ingot",
        input: &[IRON_ORE_ITEM],
        result: (IRON_INGOT, 1),
        cook_time: 200,
    },
    CookingRecipe {
        id: "diamond", // ore smelts to diamond via any cooking type
        input: &[DIAMOND_ORE_ITEM],
        result: (DIAMOND_ITEM, 1),
        cook_time: 200,
    },
];

/// Crop the 3x3 (or 2x2) grid to its occupied bounding box, then compare
/// against the recipe pattern (shrink, ShapedRecipePattern.java:101-134).
/// `grid` is row-major, width `w` (2 or 3), item ids (0 = empty).
fn crop(grid: &[u16], w: usize) -> Option<(Vec<u16>, usize, usize)> {
    let h = grid.len() / w;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0usize, 0usize);
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if grid[y * w + x] != EMPTY_SLOT {
                any = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if !any {
        return None;
    }
    let cw = max_x - min_x + 1;
    let ch = max_y - min_y + 1;
    let mut cropped = Vec::with_capacity(cw * ch);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            cropped.push(grid[y * w + x]);
        }
    }
    Some((cropped, cw, ch))
}

fn char_item(c: char, keys: &[(char, Ingredient)]) -> u16 {
    for (k, list) in keys {
        if *k == c {
            // single-item keys only in the starter table; tags land later
            return list.first().copied().unwrap_or(0);
        }
    }
    0
}

fn pattern_grid(r: &ShapedRecipe) -> Vec<Vec<u16>> {
    let mut rows = Vec::new();
    for line in r.pattern {
        let mut row = Vec::new();
        for c in line.chars() {
            row.push(if c == ' ' {
                EMPTY_SLOT
            } else {
                char_item(c, r.keys)
            });
        }
        rows.push(row);
    }
    rows
}

fn matches_shaped(grid: &[u16], w: usize, r: &ShapedRecipe) -> bool {
    let Some((cropped, cw, ch)) = crop(grid, w) else {
        return false;
    };
    let pat = pattern_grid(r);
    let pw = pat[0].len();
    let ph = pat.len();
    if pw != cw || ph != ch {
        return false;
    }
    let eq = pat
        .iter()
        .flatten()
        .zip(cropped.iter())
        .all(|(a, b)| a == b);
    if eq {
        return true;
    }
    // mirror test for asymmetric patterns
    let mut mirrored: Vec<u16> = Vec::with_capacity(cw * ch);
    for row in &pat {
        mirrored.extend(row.iter().rev().copied());
    }
    mirrored.iter().zip(cropped.iter()).all(|(a, b)| a == b)
}

fn matches_shapeless(grid: &[u16], r: &ShapelessRecipe) -> bool {
    let items: Vec<u16> = grid.iter().copied().filter(|&i| i != EMPTY_SLOT).collect();
    if items.len() != r.ingredients.len() {
        return false;
    }
    let mut remaining: Vec<Ingredient> = r.ingredients.to_vec();
    'outer: for &it in &items {
        for (i, ing) in remaining.iter().enumerate() {
            if ing.contains(&it) {
                remaining.swap_remove(i);
                continue 'outer;
            }
        }
        return false;
    }
    true
}

/// Finds a crafting result for a grid (w = 2 or 3). Shaped first (26.1 order:
/// shaped/shapeless are distinct types; RecipeMap keeps insertion order).
pub fn find_result(grid: &[u16], w: usize) -> Option<(u16, u8)> {
    for r in SHAPED.iter() {
        if matches_shaped(grid, w, r) {
            return Some(r.result);
        }
    }
    for r in SHAPELESS.iter() {
        if matches_shapeless(grid, r) {
            return Some(r.result);
        }
    }
    None
}

pub fn find_cooking(input: u16) -> Option<&'static CookingRecipe> {
    COOKING.iter().find(|r| r.input.contains(&input))
}

/// Consumes one of each occupied grid cell into `inv` (caller applies
/// stack decrements; kept here so 2x2 and 3x3 share semantics).
pub fn consume_grid(grid: &mut [ItemStack]) {
    for s in grid.iter_mut() {
        if s.count > 0 {
            s.count -= 1;
            if s.count == 0 {
                *s = ItemStack::empty();
            }
        }
    }
}
