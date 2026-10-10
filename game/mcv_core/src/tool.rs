//! 方块 → 掉落所需工具（26.1 数据驱动的最小表，任务板 #90 掉落经济闭环）。
//!
//! 数据来源（本机 `/root/mc-ref/src-26.1`，仓库外，只提取语义不搬代码）：
//!
//! - 掉落门属性 `requiresCorrectToolForDrops`：`world/level/block/Blocks.java`
//!   各 register 的 Properties（含 `Properties.ofLegacyCopy(BASE)` 的继承闭包，
//!   如 `copper_ore`/`deepslate_*_ore` 系）。与 `mcv_core::BLOCKS` 注册表求交。
//! - 层级：`data/minecraft/tags/block/needs_stone_tool.json`（石镐级）、
//!   `needs_iron_tool.json`（铁镐级）、`needs_diamond_tool.json`（钻镐级）；
//!   不在任何 needs tag = 任意镐即可（木/金镐级）。
//! - 例外：`snow`/`snow_block` 走 `mineable/shovel`（铲非镐）；`cobweb` 需要
//!   剑/剪刀的专用语义（本引擎未注册剪刀，宁缺勿错，暂不入门）。
//! - 旧表别名：`cobble`（id 9，blocks_gen 旧名）= 原版 `cobblestone`。
//!
//! 层级序与 `mcv_item::mining::mining_tier` 一致：木/金 1、石 2、铜 3、
//! 铁 4、钻/下界合金 5（ToolMaterial.java:23-31 的 INCORRECT_FOR_* tag 序）。
//!
//! TODO(gen-blocks)：全表并入 ci/gen-blocks.py 生成（含 silk/时运语义），
//! 本手写表为矿石/石族最小可用集（任务板 #90 范围）。

use crate::BlockId;

/// 掉落所需工具种类（26.1 `mineable/pickaxe|shovel` + requiresCorrectTool）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolReq {
    Pickaxe,
    Shovel,
}

/// 一条"掉落需要正确工具"规则：种类 + 最低层级（`mcv_item::mining::mining_tier` 序）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirement {
    pub kind: ToolReq,
    pub tier: u8,
}

/// 任意镐即可正确掉落（26.1 requiresCorrectToolForDrops 且不在任何 needs_*_tool tag）。
/// 含旧表别名 `cobble`（= 原版 cobblestone）与 ofLegacyCopy 继承族。
static TIER_ANY_PICKAXE: &[&str] = &[
    "amethyst_block",
    "andesite",
    "andesite_slab",
    "andesite_wall",
    "anvil",
    "basalt",
    "black_concrete",
    "black_glazed_terracotta",
    "black_terracotta",
    "blackstone",
    "blackstone_slab",
    "blackstone_wall",
    "blast_furnace",
    "blue_concrete",
    "blue_glazed_terracotta",
    "blue_terracotta",
    "bone_block",
    "brain_coral_block",
    "brick_slab",
    "brick_wall",
    "bricks",
    "brown_concrete",
    "brown_glazed_terracotta",
    "brown_terracotta",
    "bubble_coral_block",
    "budding_amethyst",
    "calcite",
    "cauldron",
    "chain_command_block",
    "chipped_anvil",
    "chiseled_deepslate",
    "chiseled_nether_bricks",
    "chiseled_polished_blackstone",
    "chiseled_quartz_block",
    "chiseled_red_sandstone",
    "chiseled_resin_bricks",
    "chiseled_sandstone",
    "chiseled_stone_bricks",
    "chiseled_tuff",
    "chiseled_tuff_bricks",
    "coal_block",
    "coal_ore",
    "cobble",
    "cobbled_deepslate",
    "cobbled_deepslate_slab",
    "cobbled_deepslate_wall",
    "cobblestone_slab",
    "cobblestone_wall",
    "command_block",
    "cracked_deepslate_bricks",
    "cracked_deepslate_tiles",
    "cracked_nether_bricks",
    "cracked_polished_blackstone_bricks",
    "cracked_stone_bricks",
    "crimson_nylium",
    "cut_red_sandstone",
    "cut_red_sandstone_slab",
    "cut_sandstone",
    "cut_sandstone_slab",
    "cyan_concrete",
    "cyan_glazed_terracotta",
    "cyan_terracotta",
    "damaged_anvil",
    "dark_prismarine",
    "dark_prismarine_slab",
    "dead_brain_coral",
    "dead_brain_coral_block",
    "dead_brain_coral_fan",
    "dead_brain_coral_wall_fan",
    "dead_bubble_coral",
    "dead_bubble_coral_block",
    "dead_bubble_coral_fan",
    "dead_bubble_coral_wall_fan",
    "dead_fire_coral",
    "dead_fire_coral_block",
    "dead_fire_coral_fan",
    "dead_fire_coral_wall_fan",
    "dead_horn_coral",
    "dead_horn_coral_block",
    "dead_horn_coral_fan",
    "dead_horn_coral_wall_fan",
    "dead_tube_coral",
    "dead_tube_coral_block",
    "dead_tube_coral_fan",
    "dead_tube_coral_wall_fan",
    "deepslate",
    "deepslate_brick_slab",
    "deepslate_brick_wall",
    "deepslate_bricks",
    "deepslate_coal_ore",
    "deepslate_tile_slab",
    "deepslate_tile_wall",
    "deepslate_tiles",
    "diorite",
    "diorite_slab",
    "diorite_wall",
    "dispenser",
    "dripstone_block",
    "dropper",
    "enchanting_table",
    "end_stone",
    "end_stone_brick_slab",
    "end_stone_brick_wall",
    "end_stone_bricks",
    "fire_coral_block",
    "furnace",
    "gilded_blackstone",
    "granite",
    "granite_slab",
    "granite_wall",
    "gray_concrete",
    "gray_glazed_terracotta",
    "gray_terracotta",
    "green_concrete",
    "green_glazed_terracotta",
    "green_terracotta",
    "grindstone",
    "hopper",
    "horn_coral_block",
    "iron_bars",
    "iron_chain",
    "iron_trapdoor",
    "jigsaw",
    "lava_cauldron",
    "light_blue_concrete",
    "light_blue_glazed_terracotta",
    "light_blue_terracotta",
    "light_gray_concrete",
    "light_gray_glazed_terracotta",
    "light_gray_terracotta",
    "lime_concrete",
    "lime_glazed_terracotta",
    "lime_terracotta",
    "lodestone",
    "magenta_concrete",
    "magenta_glazed_terracotta",
    "magenta_terracotta",
    "magma_block",
    "mossy_cobblestone",
    "mossy_cobblestone_slab",
    "mossy_cobblestone_wall",
    "mossy_stone_brick_slab",
    "mossy_stone_brick_wall",
    "mossy_stone_bricks",
    "mud_brick_slab",
    "mud_brick_wall",
    "mud_bricks",
    "nether_brick_fence",
    "nether_brick_slab",
    "nether_brick_wall",
    "nether_bricks",
    "nether_gold_ore",
    "nether_quartz_ore",
    "netherrack",
    "observer",
    "orange_concrete",
    "orange_glazed_terracotta",
    "orange_terracotta",
    "petrified_oak_slab",
    "pink_concrete",
    "pink_glazed_terracotta",
    "pink_terracotta",
    "polished_andesite",
    "polished_andesite_slab",
    "polished_basalt",
    "polished_blackstone",
    "polished_blackstone_brick_slab",
    "polished_blackstone_brick_wall",
    "polished_blackstone_bricks",
    "polished_blackstone_slab",
    "polished_blackstone_wall",
    "polished_deepslate",
    "polished_deepslate_slab",
    "polished_deepslate_wall",
    "polished_diorite",
    "polished_diorite_slab",
    "polished_granite",
    "polished_granite_slab",
    "polished_tuff",
    "polished_tuff_slab",
    "polished_tuff_stairs",
    "polished_tuff_wall",
    "powder_snow_cauldron",
    "prismarine",
    "prismarine_brick_slab",
    "prismarine_bricks",
    "prismarine_slab",
    "prismarine_wall",
    "purple_concrete",
    "purple_glazed_terracotta",
    "purple_terracotta",
    "purpur_block",
    "purpur_pillar",
    "purpur_slab",
    "quartz_block",
    "quartz_bricks",
    "quartz_pillar",
    "quartz_slab",
    "red_concrete",
    "red_glazed_terracotta",
    "red_nether_brick_slab",
    "red_nether_brick_wall",
    "red_nether_bricks",
    "red_sandstone",
    "red_sandstone_slab",
    "red_sandstone_wall",
    "red_terracotta",
    "redstone_block",
    "repeating_command_block",
    "resin_brick_slab",
    "resin_brick_wall",
    "resin_bricks",
    "sandstone",
    "sandstone_slab",
    "sandstone_wall",
    "smoker",
    "smooth_basalt",
    "smooth_quartz",
    "smooth_quartz_slab",
    "smooth_red_sandstone",
    "smooth_red_sandstone_slab",
    "smooth_sandstone",
    "smooth_sandstone_slab",
    "smooth_stone",
    "smooth_stone_slab",
    "spawner",
    "stone",
    "stone_brick_slab",
    "stone_brick_wall",
    "stone_bricks",
    "stone_slab",
    "stonecutter",
    "structure_block",
    "terracotta",
    "tube_coral_block",
    "tuff",
    "tuff_brick_slab",
    "tuff_brick_stairs",
    "tuff_brick_wall",
    "tuff_bricks",
    "tuff_slab",
    "tuff_stairs",
    "tuff_wall",
    "warped_nylium",
    "water_cauldron",
    "white_concrete",
    "white_glazed_terracotta",
    "white_terracotta",
    "yellow_concrete",
    "yellow_glazed_terracotta",
    "yellow_terracotta",
];

static TIER_STONE_PICKAXE: &[&str] = &[
    "chiseled_copper",
    "copper_block",
    "copper_bulb",
    "copper_grate",
    "copper_ore",
    "copper_trapdoor",
    "crafter",
    "cut_copper",
    "cut_copper_slab",
    "cut_copper_stairs",
    "deepslate_copper_ore",
    "deepslate_iron_ore",
    "deepslate_lapis_ore",
    "exposed_chiseled_copper",
    "exposed_copper",
    "exposed_copper_bulb",
    "exposed_copper_grate",
    "exposed_copper_trapdoor",
    "exposed_cut_copper",
    "exposed_cut_copper_slab",
    "exposed_cut_copper_stairs",
    "iron_block",
    "iron_ore",
    "lapis_block",
    "lapis_ore",
    "oxidized_chiseled_copper",
    "oxidized_copper",
    "oxidized_copper_bulb",
    "oxidized_copper_grate",
    "oxidized_copper_trapdoor",
    "oxidized_cut_copper",
    "oxidized_cut_copper_slab",
    "oxidized_cut_copper_stairs",
    "raw_copper_block",
    "raw_iron_block",
    "waxed_chiseled_copper",
    "waxed_copper_block",
    "waxed_copper_bulb",
    "waxed_copper_grate",
    "waxed_copper_trapdoor",
    "waxed_cut_copper",
    "waxed_cut_copper_slab",
    "waxed_cut_copper_stairs",
    "waxed_exposed_chiseled_copper",
    "waxed_exposed_copper",
    "waxed_exposed_copper_bulb",
    "waxed_exposed_copper_grate",
    "waxed_exposed_copper_trapdoor",
    "waxed_exposed_cut_copper",
    "waxed_exposed_cut_copper_slab",
    "waxed_exposed_cut_copper_stairs",
    "waxed_oxidized_chiseled_copper",
    "waxed_oxidized_copper",
    "waxed_oxidized_copper_bulb",
    "waxed_oxidized_copper_grate",
    "waxed_oxidized_copper_trapdoor",
    "waxed_oxidized_cut_copper",
    "waxed_oxidized_cut_copper_slab",
    "waxed_oxidized_cut_copper_stairs",
    "waxed_weathered_chiseled_copper",
    "waxed_weathered_copper",
    "waxed_weathered_copper_bulb",
    "waxed_weathered_copper_grate",
    "waxed_weathered_copper_trapdoor",
    "waxed_weathered_cut_copper",
    "waxed_weathered_cut_copper_slab",
    "waxed_weathered_cut_copper_stairs",
    "weathered_chiseled_copper",
    "weathered_copper",
    "weathered_copper_bulb",
    "weathered_copper_grate",
    "weathered_copper_trapdoor",
    "weathered_cut_copper",
    "weathered_cut_copper_slab",
    "weathered_cut_copper_stairs",
    // #copper_chests / #lightning_rods tag 成员（needs_stone_tool 引用展开）。
    "copper_chest",
    "exposed_copper_chest",
    "weathered_copper_chest",
    "oxidized_copper_chest",
    "waxed_copper_chest",
    "waxed_exposed_copper_chest",
    "waxed_weathered_copper_chest",
    "waxed_oxidized_copper_chest",
    "lightning_rod",
    "exposed_lightning_rod",
    "weathered_lightning_rod",
    "oxidized_lightning_rod",
    "waxed_lightning_rod",
    "waxed_exposed_lightning_rod",
    "waxed_weathered_lightning_rod",
    "waxed_oxidized_lightning_rod",
];

static TIER_IRON_PICKAXE: &[&str] = &[
    "deepslate_diamond_ore",
    "deepslate_emerald_ore",
    "deepslate_gold_ore",
    "deepslate_redstone_ore",
    "diamond_block",
    "diamond_ore",
    "emerald_block",
    "emerald_ore",
    "gold_block",
    "gold_ore",
    "raw_gold_block",
    "redstone_ore",
];

static TIER_DIAMOND_PICKAXE: &[&str] = &[
    "ancient_debris",
    "crying_obsidian",
    "netherite_block",
    "obsidian",
    "respawn_anchor",
];

/// 铲级掉落门（26.1 snow/snow_block：mineable/shovel + requiresCorrectTool）。
static SNOW_SHOVEL: &[&str] = &["snow", "snow_block"];

/// 查询方块的掉落工具要求（按 blocks_gen 注册名）。
///
/// 返回 `None` = 不需工具（泥土/沙/砂砾/木/叶/玻璃等，26.1 无
/// requiresCorrectToolForDrops 属性）——空手即"正确工具"。
/// `id.def().name` 现查；未注册名一律 `None`（宁缺勿错）。
pub fn requirement(name: &str) -> Option<Requirement> {
    // needs tag 优先级：钻 > 铁 > 石 > 任意镐（tag 集合互斥，序只影响效率）。
    let kind = ToolReq::Pickaxe;
    if TIER_DIAMOND_PICKAXE.contains(&name) {
        Some(Requirement { kind, tier: 5 })
    } else if TIER_IRON_PICKAXE.contains(&name) {
        Some(Requirement { kind, tier: 4 })
    } else if TIER_STONE_PICKAXE.contains(&name) {
        Some(Requirement { kind, tier: 2 })
    } else if TIER_ANY_PICKAXE.contains(&name) {
        Some(Requirement { kind, tier: 1 })
    } else if SNOW_SHOVEL.contains(&name) {
        Some(Requirement {
            kind: ToolReq::Shovel,
            tier: 1,
        })
    } else {
        None
    }
}

/// 按 [`BlockId`] 查（等价 `requirement(id.def().name)`，供无 mcv_item 依赖的
/// engine 侧挖掘公式使用）。
pub fn requirement_by_id(id: BlockId) -> Option<Requirement> {
    requirement(&crate::BLOCKS[id.id() as usize].name)
}

#[cfg(test)]
mod tests {
    use super::{ToolReq, requirement};

    fn t(name: &str) -> Option<(ToolReq, u8)> {
        requirement(name).map(|r| (r.kind, r.tier))
    }

    /// 矿石/深板岩矿石族的层级门（26.1 needs_*_tool tag + Blocks.java 继承）。
    #[test]
    fn ore_families_gate_by_tier() {
        // 任意镐：煤矿族。
        assert_eq!(t("coal_ore"), Some((ToolReq::Pickaxe, 1)));
        assert_eq!(t("deepslate_coal_ore"), Some((ToolReq::Pickaxe, 1)));
        // 石镐级（needs_stone_tool）：铁/铜/青金 + 深板岩变体。
        for name in [
            "iron_ore",
            "deepslate_iron_ore",
            "copper_ore",
            "deepslate_copper_ore",
            "lapis_ore",
            "deepslate_lapis_ore",
        ] {
            assert_eq!(t(name), Some((ToolReq::Pickaxe, 2)), "{name} 石镐级");
        }
        // 铁镐级（needs_iron_tool）：金/红石/钻石/绿宝石 + 深板岩变体。
        for name in [
            "gold_ore",
            "deepslate_gold_ore",
            "redstone_ore",
            "deepslate_redstone_ore",
            "diamond_ore",
            "deepslate_diamond_ore",
            "emerald_ore",
            "deepslate_emerald_ore",
        ] {
            assert_eq!(t(name), Some((ToolReq::Pickaxe, 4)), "{name} 铁镐级");
        }
        // 钻镐级（needs_diamond_tool）：黑曜石族/远古残骸。
        for name in ["obsidian", "crying_obsidian", "ancient_debris"] {
            assert_eq!(t(name), Some((ToolReq::Pickaxe, 5)), "{name} 钻镐级");
        }
    }

    /// 石族任意镐；泥土/沙/木/叶/玻璃/砂砾/耕地不需工具；雪走铲。
    #[test]
    fn stone_family_and_tool_free_blocks() {
        for name in [
            "stone",
            "cobble",
            "deepslate",
            "cobbled_deepslate",
            "netherrack",
        ] {
            assert_eq!(t(name), Some((ToolReq::Pickaxe, 1)), "{name} 任意镐");
        }
        for name in [
            "dirt", "grass", "sand", "gravel", "glass", "farmland", "leaves", "log", "planks",
        ] {
            assert_eq!(t(name), None, "{name} 不需工具");
        }
        assert_eq!(t("snow"), Some((ToolReq::Shovel, 1)));
        assert_eq!(t("snow_block"), Some((ToolReq::Shovel, 1)));
    }
}
