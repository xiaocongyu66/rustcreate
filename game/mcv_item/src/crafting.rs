//! Crafting: shaped (bbox-cropped, symmetric/mirror) + shapeless (multiset)
//! matching, cooking types, and the vanilla-recipe table. Engine per
//! NOTES-2 §5 (ShapedRecipePattern/ShapelessRecipe/CraftingInput).
//!
//! 配方表 = 手写表（SHAPED/SHAPELESS/COOKING，#137 历史先行件，数值对账
//! 26.1 数据包 `data/minecraft/recipe/*.json`——仓库外 /root/mc-ref/src-26.1，
//! 不入库；逐条注释引文件名与键名）∪ 生成表（recipes_gen.inc.rs，
//! ci/gen-recipes.py 从同一数据源直译的全量可收子集）。匹配顺序手写优先，
//! 重叠条目同 id 同结果，材料面生成表只会更宽（原版 tag 直译 ⊇ 手写子集）。
//! tag 配方（`#minecraft:planks` /
//! `#minecraft:<种>_logs` / `#minecraft:wooden_tool_materials`）= Ingredient
//! 列表 contains 匹配；只收编本引擎已注册物品（宁缺勿错：原版 tag 里
//! mangrove/cherry/crimson 等未注册成员不进列表）。缺物品挡住的配方登记在
//! ci/recipes-gap.md（后续波次输入）。

use crate::ItemStack;

/// Ingredient: item id list (0 = empty cell in shaped patterns).
/// 单件 = 长度 1；tag = 多件（任一命中即匹配）。
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
    ACACIA_FENCE, ACACIA_FENCE_GATE, ACACIA_LOG, ACACIA_PLANKS, ACACIA_STAIRS, BEEHIVE,
    BIRCH_FENCE, BIRCH_FENCE_GATE, BIRCH_LOG, BIRCH_PLANKS, BIRCH_STAIRS, CHEST, COBBLESTONE,
    CRAFTING_TABLE, DARK_OAK_FENCE, DARK_OAK_FENCE_GATE, DARK_OAK_LOG, DARK_OAK_PLANKS,
    DARK_OAK_STAIRS, DIAMOND_ITEM, DIAMOND_ORE_ITEM, GLASS_BOTTLE, HONEY_BLOCK, HONEY_BOTTLE,
    HONEYCOMB, HONEYCOMB_BLOCK, IRON_INGOT, IRON_ORE_ITEM, JUNGLE_FENCE, JUNGLE_FENCE_GATE,
    JUNGLE_LOG, JUNGLE_PLANKS, JUNGLE_STAIRS, LOG, OAK_FENCE, OAK_FENCE_GATE, OAK_STAIRS, PLANKS,
    SPRUCE_FENCE, SPRUCE_FENCE_GATE, SPRUCE_LOG, SPRUCE_PLANKS, SPRUCE_STAIRS, STICK,
};

/// Grid empty-cell marker（物品 id 0 = STICK 是合法物品，空格只能用哨兵；
/// shaped 图案里的空格是 ' '）。
pub const EMPTY_SLOT: u16 = u16::MAX;

// ---- 基础材料（单件）----
const ING_COBBLE: &[u16] = &[COBBLESTONE];
const ING_IRON: &[u16] = &[IRON_INGOT];
const ING_DIAMOND: &[u16] = &[DIAMOND_ITEM];
const ING_STICK: &[u16] = &[STICK];

// ---- tag 常数（data/minecraft/tags/item/*.json，仅收编已注册成员）----
/// `#minecraft:planks`（stick.json/crafting_table.json/chest.json/beehive.json
/// 的 key；wooden_tool_materials.json 展开亦只含此 tag）。
const ING_PLANKS_TAG: &[u16] = &[
    PLANKS,
    SPRUCE_PLANKS,
    BIRCH_PLANKS,
    JUNGLE_PLANKS,
    ACACIA_PLANKS,
    DARK_OAK_PLANKS,
];
/// `#minecraft:oak_logs`（oak_planks.json ingredients；本引擎 oak=log 旧位）。
const ING_OAK_LOGS: &[u16] = &[LOG];
const ING_SPRUCE_LOGS: &[u16] = &[SPRUCE_LOG];
const ING_BIRCH_LOGS: &[u16] = &[BIRCH_LOG];
const ING_JUNGLE_LOGS: &[u16] = &[JUNGLE_LOG];
const ING_ACACIA_LOGS: &[u16] = &[ACACIA_LOG];
const ING_DARK_OAK_LOGS: &[u16] = &[DARK_OAK_LOG];

// 各树种 planks（单件列表；stairs/slab/door 等按树种各自配方）。
const ING_OAK_PLANKS: &[u16] = &[PLANKS];
const ING_SPRUCE_PLANKS: &[u16] = &[SPRUCE_PLANKS];
const ING_BIRCH_PLANKS: &[u16] = &[BIRCH_PLANKS];
const ING_JUNGLE_PLANKS: &[u16] = &[JUNGLE_PLANKS];
const ING_ACACIA_PLANKS: &[u16] = &[ACACIA_PLANKS];
const ING_DARK_OAK_PLANKS: &[u16] = &[DARK_OAK_PLANKS];

// 蜂窝族单件。
const ING_HONEYCOMB: &[u16] = &[HONEYCOMB];
const ING_HONEY_BOTTLE: &[u16] = &[HONEY_BOTTLE];
const ING_HONEY_BLOCK: &[u16] = &[HONEY_BLOCK];
const ING_GLASS_BOTTLE: &[u16] = &[GLASS_BOTTLE];

const I_STONE_SWORD: u16 = 8;
const I_IRON_SWORD: u16 = 9;
const I_DIAMOND_SWORD: u16 = 10;
const I_WOOD_SWORD: u16 = 7;
const I_WOOD_PICK: u16 = 13;
const I_STONE_PICK: u16 = 14;
const I_IRON_PICK: u16 = 15;
const I_DIAMOND_PICK: u16 = 16;
const I_WOOD_AXE: u16 = 19;
const I_WOOD_SHOVEL: u16 = 25;

/// 同树种七形一刀生成（`data/minecraft/recipe/<种>_<形>.json` 同型）：
/// - stairs: `#  / ## / ###` → ×4（单 key `#`=该种 planks）
/// - slab:   `###` → ×6
/// - door:   `## / ## / ##` → ×3
/// - trapdoor: `### / ###` → ×2
/// - pressure_plate: `##` → ×1
/// - fence: `W#W / W#W` → ×3（W=板 #=棍）
/// - fence_gate: `#W# / #W#` → ×1
macro_rules! wood_shape {
    (stairs, $id:literal, $p:expr, $r:expr) => {
        ShapedRecipe {
            id: $id,
            pattern: &["#  ", "## ", "###"],
            keys: &[('#', $p)],
            result: ($r, 4),
        }
    };
    (slab, $id:literal, $p:expr, $r:expr) => {
        ShapedRecipe {
            id: $id,
            pattern: &["###"],
            keys: &[('#', $p)],
            result: ($r, 6),
        }
    };
    (door, $id:literal, $p:expr, $r:expr) => {
        ShapedRecipe {
            id: $id,
            pattern: &["##", "##", "##"],
            keys: &[('#', $p)],
            result: ($r, 3),
        }
    };
    (trapdoor, $id:literal, $p:expr, $r:expr) => {
        ShapedRecipe {
            id: $id,
            pattern: &["###", "###"],
            keys: &[('#', $p)],
            result: ($r, 2),
        }
    };
    (pressure_plate, $id:literal, $p:expr, $r:expr) => {
        ShapedRecipe {
            id: $id,
            pattern: &["##"],
            keys: &[('#', $p)],
            result: ($r, 1),
        }
    };
    (fence, $id:literal, $p:expr, $r:expr) => {
        ShapedRecipe {
            id: $id,
            pattern: &["W#W", "W#W"],
            keys: &[('W', $p), ('#', ING_STICK)],
            result: ($r, 3),
        }
    };
    (fence_gate, $id:literal, $p:expr, $r:expr) => {
        ShapedRecipe {
            id: $id,
            pattern: &["#W#", "#W#"],
            keys: &[('W', $p), ('#', ING_STICK)],
            result: ($r, 1),
        }
    };
}

/// 26.1 数据包配方对齐（data/minecraft/recipe/*.json）：
/// 镐 = MMM/_S_/_S_；剑 = M/M/S 竖排 1x3（2x2 放不下，必须 3x3 工作台）；
/// 木棍 = 两板竖排 ×4（stick.json，key=`#minecraft:planks` tag）；原木 →
/// 4 板（shapeless，`<种>_planks.json`）。
pub static SHAPED: [ShapedRecipe; 58] = [
    ShapedRecipe {
        id: "stick",
        pattern: &["#", "#"],
        keys: &[('#', ING_PLANKS_TAG)],
        result: (STICK, 4),
    },
    // 剑：材料×2 在上、棍在底（wooden/stone/iron/diamond_sword.json 同型；
    // wooden 的 X=`#minecraft:wooden_tool_materials`（=planks tag））。
    ShapedRecipe {
        id: "wooden_sword",
        pattern: &["M", "M", "S"],
        keys: &[('M', ING_PLANKS_TAG), ('S', ING_STICK)],
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
        keys: &[('M', ING_PLANKS_TAG), ('S', ING_STICK)],
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
    // axe: XX / X# / _#（wooden_axe.json，X=wooden_tool_materials）
    ShapedRecipe {
        id: "wooden_axe",
        pattern: &["XX", "X#", " #"],
        keys: &[('X', ING_PLANKS_TAG), ('#', ING_STICK)],
        result: (I_WOOD_AXE, 1),
    },
    // shovel: X / # / #（wooden_shovel.json）
    ShapedRecipe {
        id: "wooden_shovel",
        pattern: &["X", "#", "#"],
        keys: &[('X', ING_PLANKS_TAG), ('#', ING_STICK)],
        result: (I_WOOD_SHOVEL, 1),
    },
    // ---- 同树种七形 ×6（<种>_<形>.json，count 键值见 wood_shape! 注释）----
    wood_shape!(stairs, "oak_stairs", ING_OAK_PLANKS, OAK_STAIRS),
    wood_shape!(stairs, "spruce_stairs", ING_SPRUCE_PLANKS, SPRUCE_STAIRS),
    wood_shape!(stairs, "birch_stairs", ING_BIRCH_PLANKS, BIRCH_STAIRS),
    wood_shape!(stairs, "jungle_stairs", ING_JUNGLE_PLANKS, JUNGLE_STAIRS),
    wood_shape!(stairs, "acacia_stairs", ING_ACACIA_PLANKS, ACACIA_STAIRS),
    wood_shape!(
        stairs,
        "dark_oak_stairs",
        ING_DARK_OAK_PLANKS,
        DARK_OAK_STAIRS
    ),
    wood_shape!(slab, "oak_slab", ING_OAK_PLANKS, crate::OAK_SLAB),
    wood_shape!(slab, "spruce_slab", ING_SPRUCE_PLANKS, crate::SPRUCE_SLAB),
    wood_shape!(slab, "birch_slab", ING_BIRCH_PLANKS, crate::BIRCH_SLAB),
    wood_shape!(slab, "jungle_slab", ING_JUNGLE_PLANKS, crate::JUNGLE_SLAB),
    wood_shape!(slab, "acacia_slab", ING_ACACIA_PLANKS, crate::ACACIA_SLAB),
    wood_shape!(
        slab,
        "dark_oak_slab",
        ING_DARK_OAK_PLANKS,
        crate::DARK_OAK_SLAB
    ),
    wood_shape!(fence, "oak_fence", ING_OAK_PLANKS, OAK_FENCE),
    wood_shape!(fence, "spruce_fence", ING_SPRUCE_PLANKS, SPRUCE_FENCE),
    wood_shape!(fence, "birch_fence", ING_BIRCH_PLANKS, BIRCH_FENCE),
    wood_shape!(fence, "jungle_fence", ING_JUNGLE_PLANKS, JUNGLE_FENCE),
    wood_shape!(fence, "acacia_fence", ING_ACACIA_PLANKS, ACACIA_FENCE),
    wood_shape!(fence, "dark_oak_fence", ING_DARK_OAK_PLANKS, DARK_OAK_FENCE),
    wood_shape!(fence_gate, "oak_fence_gate", ING_OAK_PLANKS, OAK_FENCE_GATE),
    wood_shape!(
        fence_gate,
        "spruce_fence_gate",
        ING_SPRUCE_PLANKS,
        SPRUCE_FENCE_GATE
    ),
    wood_shape!(
        fence_gate,
        "birch_fence_gate",
        ING_BIRCH_PLANKS,
        BIRCH_FENCE_GATE
    ),
    wood_shape!(
        fence_gate,
        "jungle_fence_gate",
        ING_JUNGLE_PLANKS,
        JUNGLE_FENCE_GATE
    ),
    wood_shape!(
        fence_gate,
        "acacia_fence_gate",
        ING_ACACIA_PLANKS,
        ACACIA_FENCE_GATE
    ),
    wood_shape!(
        fence_gate,
        "dark_oak_fence_gate",
        ING_DARK_OAK_PLANKS,
        DARK_OAK_FENCE_GATE
    ),
    wood_shape!(door, "oak_door", ING_OAK_PLANKS, crate::OAK_DOOR),
    wood_shape!(door, "spruce_door", ING_SPRUCE_PLANKS, crate::SPRUCE_DOOR),
    wood_shape!(door, "birch_door", ING_BIRCH_PLANKS, crate::BIRCH_DOOR),
    wood_shape!(door, "jungle_door", ING_JUNGLE_PLANKS, crate::JUNGLE_DOOR),
    wood_shape!(door, "acacia_door", ING_ACACIA_PLANKS, crate::ACACIA_DOOR),
    wood_shape!(
        door,
        "dark_oak_door",
        ING_DARK_OAK_PLANKS,
        crate::DARK_OAK_DOOR
    ),
    wood_shape!(
        trapdoor,
        "oak_trapdoor",
        ING_OAK_PLANKS,
        crate::OAK_TRAPDOOR
    ),
    wood_shape!(
        trapdoor,
        "spruce_trapdoor",
        ING_SPRUCE_PLANKS,
        crate::SPRUCE_TRAPDOOR
    ),
    wood_shape!(
        trapdoor,
        "birch_trapdoor",
        ING_BIRCH_PLANKS,
        crate::BIRCH_TRAPDOOR
    ),
    wood_shape!(
        trapdoor,
        "jungle_trapdoor",
        ING_JUNGLE_PLANKS,
        crate::JUNGLE_TRAPDOOR
    ),
    wood_shape!(
        trapdoor,
        "acacia_trapdoor",
        ING_ACACIA_PLANKS,
        crate::ACACIA_TRAPDOOR
    ),
    wood_shape!(
        trapdoor,
        "dark_oak_trapdoor",
        ING_DARK_OAK_PLANKS,
        crate::DARK_OAK_TRAPDOOR
    ),
    wood_shape!(
        pressure_plate,
        "oak_pressure_plate",
        ING_OAK_PLANKS,
        crate::OAK_PRESSURE_PLATE
    ),
    wood_shape!(
        pressure_plate,
        "spruce_pressure_plate",
        ING_SPRUCE_PLANKS,
        crate::SPRUCE_PRESSURE_PLATE
    ),
    wood_shape!(
        pressure_plate,
        "birch_pressure_plate",
        ING_BIRCH_PLANKS,
        crate::BIRCH_PRESSURE_PLATE
    ),
    wood_shape!(
        pressure_plate,
        "jungle_pressure_plate",
        ING_JUNGLE_PLANKS,
        crate::JUNGLE_PRESSURE_PLATE
    ),
    wood_shape!(
        pressure_plate,
        "acacia_pressure_plate",
        ING_ACACIA_PLANKS,
        crate::ACACIA_PRESSURE_PLATE
    ),
    wood_shape!(
        pressure_plate,
        "dark_oak_pressure_plate",
        ING_DARK_OAK_PLANKS,
        crate::DARK_OAK_PRESSURE_PLATE
    ),
    // ---- 功能方块与蜂窝链 ----
    // crafting_table.json：`## / ##`，key=`#minecraft:planks`（无 count=1）。
    ShapedRecipe {
        id: "crafting_table",
        pattern: &["##", "##"],
        keys: &[('#', ING_PLANKS_TAG)],
        result: (CRAFTING_TABLE, 1),
    },
    // chest.json：`### / # # / ###`，key=`#minecraft:planks`（无 count=1）。
    ShapedRecipe {
        id: "chest",
        pattern: &["###", "# #", "###"],
        keys: &[('#', ING_PLANKS_TAG)],
        result: (CHEST, 1),
    },
    // honeycomb_block.json：`## / ##`，key `#`=minecraft:honeycomb。
    ShapedRecipe {
        id: "honeycomb_block",
        pattern: &["##", "##"],
        keys: &[('#', ING_HONEYCOMB)],
        result: (HONEYCOMB_BLOCK, 1),
    },
    // beehive.json：`PPP / HHH / PPP`，P=`#minecraft:planks` H=honeycomb。
    ShapedRecipe {
        id: "beehive",
        pattern: &["PPP", "HHH", "PPP"],
        keys: &[('P', ING_PLANKS_TAG), ('H', ING_HONEYCOMB)],
        result: (BEEHIVE, 1),
    },
    // honey_block.json：`## / ##`，key `#`=minecraft:honey_bottle。
    ShapedRecipe {
        id: "honey_block",
        pattern: &["##", "##"],
        keys: &[('#', ING_HONEY_BOTTLE)],
        result: (HONEY_BLOCK, 1),
    },
];

/// shapeless：log→planks（`<种>_planks.json`）、按钮（`<种>_button.json`）、
/// honey_bottle（`honey_bottle.json`：honey_block+glass_bottle×4 → ×4）。
pub static SHAPELESS: [ShapelessRecipe; 13] = [
    ShapelessRecipe {
        id: "oak_planks",
        ingredients: &[ING_OAK_LOGS],
        result: (PLANKS, 4),
    },
    ShapelessRecipe {
        id: "spruce_planks",
        ingredients: &[ING_SPRUCE_LOGS],
        result: (SPRUCE_PLANKS, 4),
    },
    ShapelessRecipe {
        id: "birch_planks",
        ingredients: &[ING_BIRCH_LOGS],
        result: (BIRCH_PLANKS, 4),
    },
    ShapelessRecipe {
        id: "jungle_planks",
        ingredients: &[ING_JUNGLE_LOGS],
        result: (JUNGLE_PLANKS, 4),
    },
    ShapelessRecipe {
        id: "acacia_planks",
        ingredients: &[ING_ACACIA_LOGS],
        result: (ACACIA_PLANKS, 4),
    },
    ShapelessRecipe {
        id: "dark_oak_planks",
        ingredients: &[ING_DARK_OAK_LOGS],
        result: (DARK_OAK_PLANKS, 4),
    },
    ShapelessRecipe {
        id: "oak_button",
        ingredients: &[ING_OAK_PLANKS],
        result: (crate::OAK_BUTTON, 1),
    },
    ShapelessRecipe {
        id: "spruce_button",
        ingredients: &[ING_SPRUCE_PLANKS],
        result: (crate::SPRUCE_BUTTON, 1),
    },
    ShapelessRecipe {
        id: "birch_button",
        ingredients: &[ING_BIRCH_PLANKS],
        result: (crate::BIRCH_BUTTON, 1),
    },
    ShapelessRecipe {
        id: "jungle_button",
        ingredients: &[ING_JUNGLE_PLANKS],
        result: (crate::JUNGLE_BUTTON, 1),
    },
    ShapelessRecipe {
        id: "acacia_button",
        ingredients: &[ING_ACACIA_PLANKS],
        result: (crate::ACACIA_BUTTON, 1),
    },
    ShapelessRecipe {
        id: "dark_oak_button",
        ingredients: &[ING_DARK_OAK_PLANKS],
        result: (crate::DARK_OAK_BUTTON, 1),
    },
    ShapelessRecipe {
        id: "honey_bottle",
        ingredients: &[
            ING_HONEY_BLOCK,
            ING_GLASS_BOTTLE,
            ING_GLASS_BOTTLE,
            ING_GLASS_BOTTLE,
            ING_GLASS_BOTTLE,
        ],
        result: (HONEY_BOTTLE, 4),
    },
];

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

// ---- 生成表（ci/gen-recipes.py 产物，随源重生成；DO NOT EDIT）----------------
// 与上方手写表按「手写 ∪ 生成」合并匹配（见 find_result/find_cooking）；
// 表内容、重叠与镜像语义的完整引注见该文件头注释与 ci/recipes-gap.md。
include!("recipes_gen.inc.rs");

/// Resolve a pattern char to its ingredient list（tag 配方 = 整张列表，
/// 未知字符 = None → 该格永不匹配；26.1 ShapedRecipePattern 的 key 缺失
/// 即配方反序列化失败，同语义）。
fn char_ingredient(c: char, keys: &[(char, Ingredient)]) -> Option<Ingredient> {
    keys.iter().find(|(k, _)| *k == c).map(|(_, list)| *list)
}

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

fn pattern_rows_fit(
    pattern: &[&str],
    keys: &[(char, Ingredient)],
    cropped: &[u16],
    cw: usize,
) -> bool {
    pattern.iter().enumerate().all(|(y, line)| {
        line.chars().enumerate().all(|(x, c)| {
            let cell = cropped[y * cw + x];
            if c == ' ' {
                cell == EMPTY_SLOT
            } else {
                cell != EMPTY_SLOT
                    && char_ingredient(c, keys).is_some_and(|list| list.contains(&cell))
            }
        })
    })
}

fn matches_shaped(grid: &[u16], w: usize, r: &ShapedRecipe) -> bool {
    let Some((cropped, cw, ch)) = crop(grid, w) else {
        return false;
    };
    let pw = r.pattern[0].chars().count();
    let ph = r.pattern.len();
    if pw != cw || ph != ch {
        return false;
    }
    if pattern_rows_fit(r.pattern, r.keys, &cropped, cw) {
        return true;
    }
    // mirror test for asymmetric patterns（26.1 ShapedRecipePattern 允许镜像：
    // :159-171 symmetrical==false 时 matches(input, true) 同样命中；JSON 数据
    // 只含实际 pattern（Data 记录无镜像字段），镜像全在运行期——手写/生成表
    // 均不展开镜像，逐行反转字符后重比）。
    let mirrored: Vec<String> = r
        .pattern
        .iter()
        .map(|line| line.chars().rev().collect::<String>())
        .collect();
    let mirrored_refs: Vec<&str> = mirrored.iter().map(String::as_str).collect();
    pattern_rows_fit(&mirrored_refs, r.keys, &cropped, cw)
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
/// 遍历「手写表 ∪ 生成表」，手写优先（重叠条目同结果，行为不变）。
pub fn find_result(grid: &[u16], w: usize) -> Option<(u16, u8)> {
    for r in SHAPED.iter().chain(GEN_SHAPED.iter()) {
        if matches_shaped(grid, w, r) {
            return Some(r.result);
        }
    }
    for r in SHAPELESS.iter().chain(GEN_SHAPELESS.iter()) {
        if matches_shapeless(grid, r) {
            return Some(r.result);
        }
    }
    None
}

pub fn find_cooking(input: u16) -> Option<&'static CookingRecipe> {
    COOKING
        .iter()
        .chain(GEN_COOKING.iter())
        .find(|r| r.input.contains(&input))
}

/// Furnace smelt result for one input item（26.1 smelting；燃料值另表待后续波，
/// 见 ci/recipes-gap.md 债节）。纯 API：本引擎暂无 furnace 屏，UI 接线随
/// furnace 物品/方块注册波补（不新建 UI 在本波范围内）。
pub fn smelt_result(item: u16) -> Option<(u16, u8)> {
    find_cooking(item).map(|r| r.result)
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
