//! 木族+蜂窝配方解析测试（W 波 #137 刀2）。
//!
//! 数值全部对账 26.1 配方数据（仓库外 /root/mc-ref/src-26.1/
//! data/minecraft/recipe/*.json，不入库；注释逐配方引文件名）：
//! - `<种>_planks.json`：shapeless `#minecraft:<种>_logs` → ×4
//! - `<种>_stairs.json`：shaped `#  / ## / ###`（key `#`=`<种>_planks`）→ ×4
//! - `<种>_slab.json`：`###` → ×6
//! - `<种>_fence.json`：`W#W / W#W`（W=板 #=棍）→ ×3
//! - `<种>_fence_gate.json`：`#W# / #W#` → ×1（json result 无 count 键=1）
//! - `<种>_door.json`：`## / ## / ##` → ×3
//! - `<种>_trapdoor.json`：`### / ###` → ×2（26.1 值核对）
//! - `<种>_button.json`：shapeless 1 板 → ×1
//! - `<种>_pressure_plate.json`：`##` → ×1
//! - `stick.json`：`# / #`，key `#`=`#minecraft:planks` tag → ×4
//! - `crafting_table.json`：`## / ##`（planks tag）→ ×1
//! - `chest.json`：`### / # # / ###`（planks tag）→ ×1
//! - `honeycomb_block.json`：`## / ##`（honeycomb）→ ×1
//! - `beehive.json`：`PPP / HHH / PPP`（P=planks tag H=honeycomb）→ ×1
//! - `honey_bottle.json`：shapeless honey_block+glass_bottle×4 → ×4
//! - `honey_block.json`：`## / ##`（honey_bottle）→ ×1
//! - `wooden_axe.json`：`XX / X# /  #`，`X`=`#minecraft:wooden_tool_materials`
//! - `wooden_shovel.json`：`X / # / #`
//!
//! tag 语义（#minecraft:planks / 各 <种>_logs）在本引擎 = Ingredient 列表
//! contains 匹配；仅收编已注册物品（宁缺勿错，见 crafting.rs 注释）。

use mcv_item::crafting::EMPTY_SLOT;
use mcv_item::{
    ACACIA_PLANKS, BIRCH_LOG, BIRCH_PLANKS, CHEST, CRAFTING_TABLE, DARK_OAK_LOG, DARK_OAK_PLANKS,
    GLASS_BOTTLE, HONEY_BLOCK, HONEY_BOTTLE, HONEYCOMB, HONEYCOMB_BLOCK, JUNGLE_PLANKS, LOG,
    PLANKS, SPRUCE_LOG, SPRUCE_PLANKS, STICK,
};

/// 3x3 空网格。
fn g3() -> [u16; 9] {
    [EMPTY_SLOT; 9]
}

/// 2x2 空网格。
fn g2() -> [u16; 4] {
    [EMPTY_SLOT; 4]
}

// ---- shapeless：log→planks×4 同树种链（≥3 例）----

#[test]
fn planks_shapeless_same_species() {
    // oak：旧 log 物品（块 6 "log"=oak_log）→ 旧 PLANKS×4（oak_planks.json
    // 的 `#minecraft:oak_logs` tag，本引擎仅收编已注册 log 位）。
    let mut grid = g3();
    grid[4] = LOG;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((PLANKS, 4)),
        "oak log → 4 oak planks"
    );
    // spruce / dark_oak：同型（spruce_planks.json / dark_oak_planks.json）。
    let mut grid = g3();
    grid[8] = SPRUCE_LOG;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((SPRUCE_PLANKS, 4))
    );
    let mut grid = g3();
    grid[0] = DARK_OAK_LOG;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((DARK_OAK_PLANKS, 4))
    );
    // 2x2 网格同样可出（原版合成格 2x2 即可）。
    let mut grid = g2();
    grid[3] = SPRUCE_LOG;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 2),
        Some((SPRUCE_PLANKS, 4))
    );
}

#[test]
fn planks_tag_discipline_no_cross_species() {
    // birch_log 只在 #minecraft:birch_logs——不得出 oak(planks 旧位)/spruce 板。
    let mut grid = g3();
    grid[4] = BIRCH_LOG;
    let r = mcv_item::crafting::find_result(&grid, 3);
    assert_ne!(r, Some((PLANKS, 4)), "birch log 不得出 oak planks");
    assert_ne!(
        r,
        Some((SPRUCE_PLANKS, 4)),
        "birch log 不得出 spruce planks"
    );
    assert_eq!(
        r,
        Some((mcv_item::BIRCH_PLANKS, 4)),
        "birch log → birch planks"
    );
}

// ---- shaped：stairs/slab（≥3 例/类）----

/// 楼梯三角（左下角原点）：`#  / ## / ###`。
fn stairs_triangle(species_planks: u16) -> [u16; 9] {
    let mut grid = g3();
    grid[0] = species_planks;
    grid[3] = species_planks;
    grid[4] = species_planks;
    grid[6] = species_planks;
    grid[7] = species_planks;
    grid[8] = species_planks;
    grid
}

#[test]
fn stairs_shaped_same_species() {
    // oak_stairs.json：`#  / ## / ###` → ×4。
    assert_eq!(
        mcv_item::crafting::find_result(&stairs_triangle(PLANKS), 3),
        Some((mcv_item::OAK_STAIRS, 4))
    );
    // spruce_stairs.json 同型 → ×4。
    assert_eq!(
        mcv_item::crafting::find_result(&stairs_triangle(SPRUCE_PLANKS), 3),
        Some((mcv_item::SPRUCE_STAIRS, 4))
    );
    // acacia_stairs.json 同型 → ×4。
    assert_eq!(
        mcv_item::crafting::find_result(&stairs_triangle(ACACIA_PLANKS), 3),
        Some((mcv_item::ACACIA_STAIRS, 4))
    );
    // 镜像三角（原版 ShapedRecipePattern 允许镜像）也要出。
    let mut mirrored = g3();
    for v in [2usize, 4, 5, 6, 7, 8] {
        mirrored[v] = PLANKS;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&mirrored, 3),
        Some((mcv_item::OAK_STAIRS, 4)),
        "镜像楼梯出同产物"
    );
}

#[test]
fn stairs_wrong_shape_rejected() {
    // 满排 3x3 全板不是楼梯形（也无该配方）。
    let full = [PLANKS; 9];
    assert_ne!(
        mcv_item::crafting::find_result(&full, 3),
        Some((mcv_item::OAK_STAIRS, 4))
    );
    // 缺一格不成形。
    let mut grid = stairs_triangle(PLANKS);
    grid[8] = EMPTY_SLOT;
    assert_eq!(mcv_item::crafting::find_result(&grid, 3), None);
}

#[test]
fn slab_shaped_three_in_a_row() {
    // oak_slab.json：`###` → ×6。
    let mut grid = g3();
    grid[3] = PLANKS;
    grid[4] = PLANKS;
    grid[5] = PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::OAK_SLAB, 6))
    );
    // birch_slab.json → ×6。
    let mut grid = g3();
    grid[0] = BIRCH_PLANKS;
    grid[1] = BIRCH_PLANKS;
    grid[2] = BIRCH_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::BIRCH_SLAB, 6))
    );
    // dark_oak_slab.json → ×6（第 3 例，跨行位摆放 bbox 裁剪归一）。
    let mut grid = g3();
    grid[6] = DARK_OAK_PLANKS;
    grid[7] = DARK_OAK_PLANKS;
    grid[8] = DARK_OAK_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::DARK_OAK_SLAB, 6))
    );
}

// ---- shaped：fence/fence_gate（≥3 例/类）----

#[test]
fn fence_shaped_wsw() {
    // oak_fence.json：`W#W / W#W`（W=板 #=棍）→ ×3。
    let mut grid = g3();
    grid[0] = PLANKS;
    grid[1] = STICK;
    grid[2] = PLANKS;
    grid[3] = PLANKS;
    grid[4] = STICK;
    grid[5] = PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::OAK_FENCE, 3))
    );
    // jungle_fence.json 同型 → ×3。
    let mut grid = g3();
    for i in [0usize, 2, 3, 5] {
        grid[i] = JUNGLE_PLANKS;
    }
    grid[1] = STICK;
    grid[4] = STICK;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::JUNGLE_FENCE, 3))
    );
    // spruce_fence.json → ×3（第 3 例）。
    let mut grid = g3();
    for i in [0usize, 2, 3, 5] {
        grid[i] = SPRUCE_PLANKS;
    }
    grid[1] = STICK;
    grid[4] = STICK;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::SPRUCE_FENCE, 3))
    );
}

#[test]
fn fence_gate_shaped_sws() {
    // oak_fence_gate.json：`#W# / #W#` → ×1（json 无 count 键）。
    let mut grid = g3();
    grid[0] = STICK;
    grid[1] = PLANKS;
    grid[2] = STICK;
    grid[3] = STICK;
    grid[4] = PLANKS;
    grid[5] = STICK;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::OAK_FENCE_GATE, 1))
    );
    // birch_fence_gate.json → ×1。
    let mut grid = g3();
    for i in [0usize, 2, 3, 5] {
        grid[i] = STICK;
    }
    grid[1] = BIRCH_PLANKS;
    grid[4] = BIRCH_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::BIRCH_FENCE_GATE, 1))
    );
    // dark_oak_fence_gate.json → ×1（第 3 例）。
    let mut grid = g3();
    for i in [0usize, 2, 3, 5] {
        grid[i] = STICK;
    }
    grid[1] = DARK_OAK_PLANKS;
    grid[4] = DARK_OAK_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::DARK_OAK_FENCE_GATE, 1))
    );
}

// ---- shaped：door/trapdoor（26.1 值核对）----

#[test]
fn door_shaped_two_by_three() {
    // oak_door.json：`## / ## / ##` → ×3。
    let mut grid = g3();
    for i in [0usize, 1, 3, 4, 6, 7] {
        grid[i] = PLANKS;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::OAK_DOOR, 3))
    );
    // spruce_door.json → ×3。
    let mut grid = g3();
    for i in [0usize, 1, 3, 4, 6, 7] {
        grid[i] = SPRUCE_PLANKS;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::SPRUCE_DOOR, 3))
    );
    // dark_oak_door.json → ×3（第 3 例）。
    let mut grid = g3();
    for i in [0usize, 1, 3, 4, 6, 7] {
        grid[i] = DARK_OAK_PLANKS;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::DARK_OAK_DOOR, 3))
    );
}

#[test]
fn trapdoor_shaped_three_by_two() {
    // oak_trapdoor.json：`### / ###` → ×2（26.1 值核对）。
    let mut grid = g3();
    for i in [0usize, 1, 2, 3, 4, 5] {
        grid[i] = PLANKS;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::OAK_TRAPDOOR, 2))
    );
    // birch_trapdoor.json → ×2。
    let mut grid = g3();
    for i in [0usize, 1, 2, 3, 4, 5] {
        grid[i] = BIRCH_PLANKS;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::BIRCH_TRAPDOOR, 2))
    );
    // jungle_trapdoor.json → ×2（第 3 例）。
    let mut grid = g3();
    for i in [0usize, 1, 2, 3, 4, 5] {
        grid[i] = JUNGLE_PLANKS;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::JUNGLE_TRAPDOOR, 2))
    );
}

// ---- button/pressure_plate ----

#[test]
fn button_shapeless_one_plank() {
    // oak_button.json：shapeless 1 板 → ×1。
    let mut grid = g3();
    grid[4] = PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::OAK_BUTTON, 1))
    );
    // birch_button.json（2x2 网格角落摆放）。
    let mut grid = g2();
    grid[0] = BIRCH_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 2),
        Some((mcv_item::BIRCH_BUTTON, 1))
    );
    // acacia_button.json（第 3 例）。
    let mut grid = g3();
    grid[8] = ACACIA_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::ACACIA_BUTTON, 1))
    );
}

#[test]
fn pressure_plate_shaped_two_planks() {
    // oak_pressure_plate.json：`##` → ×1（2x2 网格）。
    let mut grid = g2();
    grid[0] = PLANKS;
    grid[1] = PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 2),
        Some((mcv_item::OAK_PRESSURE_PLATE, 1))
    );
    // spruce_pressure_plate.json（第 2 例，居中列）。
    let mut grid = g3();
    grid[3] = SPRUCE_PLANKS;
    grid[4] = SPRUCE_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::SPRUCE_PRESSURE_PLATE, 1))
    );
    // dark_oak_pressure_plate.json（第 3 例）。
    let mut grid = g2();
    grid[2] = DARK_OAK_PLANKS;
    grid[3] = DARK_OAK_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 2),
        Some((mcv_item::DARK_OAK_PRESSURE_PLATE, 1))
    );
}

// ---- tag：stick / crafting_table / chest ----

#[test]
fn stick_tag_accepts_all_registered_planks() {
    // stick.json：`# / #`，key=`#minecraft:planks`——六树种板全部出棍×4。
    for (planks, label) in [
        (PLANKS, "oak"),
        (SPRUCE_PLANKS, "spruce"),
        (BIRCH_PLANKS, "birch"),
        (JUNGLE_PLANKS, "jungle"),
        (ACACIA_PLANKS, "acacia"),
        (DARK_OAK_PLANKS, "dark_oak"),
    ] {
        let mut grid = g3();
        grid[4] = planks;
        grid[7] = planks;
        assert_eq!(
            mcv_item::crafting::find_result(&grid, 3),
            Some((STICK, 4)),
            "{label} 板竖排 ×2 应出棍×4"
        );
    }
}

#[test]
fn crafting_table_two_by_two_planks_tag() {
    // crafting_table.json：`## / ##`（planks tag）→ ×1。
    let mut grid = g2();
    grid[0] = BIRCH_PLANKS;
    grid[1] = BIRCH_PLANKS;
    grid[2] = SPRUCE_PLANKS;
    grid[3] = DARK_OAK_PLANKS;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 2),
        Some((CRAFTING_TABLE, 1)),
        "混树种 2x2 板（tag）出工作台"
    );
    // 全 oak 也要出（第 2 例）。
    let grid = [PLANKS, PLANKS, PLANKS, PLANKS];
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 2),
        Some((CRAFTING_TABLE, 1))
    );
    // 3x3 里 2x2 角摆放同样出（bbox 裁剪；第 3 例）。
    let mut grid = g3();
    for i in [0usize, 1, 3, 4] {
        grid[i] = ACACIA_PLANKS;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((CRAFTING_TABLE, 1))
    );
}

#[test]
fn chest_eight_planks_with_hole() {
    // chest.json：`### / # # / ###`（planks tag）→ ×1。
    let mut grid = [SPRUCE_PLANKS; 9];
    grid[4] = EMPTY_SLOT;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((CHEST, 1)),
        "8 板中空出箱子"
    );
    // 混树种（tag 语义；第 2 例）。
    let mut grid = g3();
    for (i, p) in [0usize, 1, 2, 3, 5, 6, 7, 8].into_iter().enumerate() {
        grid[p] = if i % 2 == 0 { PLANKS } else { BIRCH_PLANKS };
    }
    grid[4] = EMPTY_SLOT;
    assert_eq!(mcv_item::crafting::find_result(&grid, 3), Some((CHEST, 1)));
    // 中心不放空不算（9 板满格不成 chest；第 3 例负判据）。
    let full = [SPRUCE_PLANKS; 9];
    assert_ne!(mcv_item::crafting::find_result(&full, 3), Some((CHEST, 1)));
}

// ---- 木工具 tag 升级（wooden_tool_materials = planks tag）----

#[test]
fn wooden_tools_accept_species_planks() {
    // wooden_pickaxe.json：`XXX / _#_ / _#_`，X=wooden_tool_materials。
    let mut grid = g3();
    grid[0] = SPRUCE_PLANKS;
    grid[1] = SPRUCE_PLANKS;
    grid[2] = SPRUCE_PLANKS;
    grid[4] = STICK;
    grid[7] = STICK;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::WOODEN_PICKAXE_INDEX, 1)),
        "云杉板（tag）出木镐"
    );
    // wooden_axe.json：`XX / X# / _#` → ×1。
    let mut grid = g3();
    grid[0] = BIRCH_PLANKS;
    grid[1] = BIRCH_PLANKS;
    grid[3] = BIRCH_PLANKS;
    grid[4] = STICK;
    grid[7] = STICK;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::WOODEN_AXE_INDEX, 1))
    );
    // wooden_shovel.json：`X / # / #` → ×1。
    let mut grid = g3();
    grid[1] = DARK_OAK_PLANKS;
    grid[4] = STICK;
    grid[7] = STICK;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::WOODEN_SHOVEL_INDEX, 1))
    );
}

// ---- 蜂窝链 ----

#[test]
fn honeycomb_block_two_by_two() {
    // honeycomb_block.json：`## / ##`（honeycomb）→ ×1。
    let mut grid = g2();
    grid[0] = HONEYCOMB;
    grid[1] = HONEYCOMB;
    grid[2] = HONEYCOMB;
    grid[3] = HONEYCOMB;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 2),
        Some((HONEYCOMB_BLOCK, 1))
    );
    // 3x3 角摆放（bbox 裁剪归一；第 2 例）。
    let mut grid = g3();
    for i in [0usize, 1, 3, 4] {
        grid[i] = HONEYCOMB;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((HONEYCOMB_BLOCK, 1))
    );
    // 3 枚不成（第 3 例负判据）。
    let mut grid = g2();
    grid[0] = HONEYCOMB;
    grid[1] = HONEYCOMB;
    grid[2] = HONEYCOMB;
    assert_eq!(mcv_item::crafting::find_result(&grid, 2), None);
}

#[test]
fn beehive_planks_honeycomb_sandwich() {
    // beehive.json：`PPP / HHH / PPP`，P=planks tag。
    let mut grid = g3();
    for (i, slot) in grid.iter_mut().enumerate() {
        *slot = if i / 3 == 1 { HONEYCOMB } else { BIRCH_PLANKS };
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::BEEHIVE, 1))
    );
    // 混树种板 + 蜂脾（第 2 例）。
    let mut grid = g3();
    for (i, p) in [0usize, 1, 2, 6, 7, 8].into_iter().enumerate() {
        grid[p] = if i % 2 == 0 { PLANKS } else { JUNGLE_PLANKS };
    }
    for i in [3usize, 4, 5] {
        grid[i] = HONEYCOMB;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::BEEHIVE, 1))
    );
    // 蜂脾层错位不成（第 3 例负判据）。
    let mut grid = g3();
    for (i, slot) in grid.iter_mut().enumerate() {
        *slot = if i % 2 == 0 { HONEYCOMB } else { PLANKS };
    }
    assert_ne!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((mcv_item::BEEHIVE, 1))
    );
}

#[test]
fn honey_bottle_from_honey_block_and_four_bottles() {
    // honey_bottle.json：shapeless honey_block + glass_bottle×4 → ×4。
    let mut grid = g3();
    grid[0] = HONEY_BLOCK;
    for i in [1usize, 3, 4, 5] {
        grid[i] = GLASS_BOTTLE;
    }
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 3),
        Some((HONEY_BOTTLE, 4)),
        "蜂蜜块+4 玻璃瓶散放 → 4 蜂蜜瓶"
    );
    // 瓶数不足不成（第 2 例负判据）。
    let mut grid = g3();
    grid[0] = HONEY_BLOCK;
    grid[1] = GLASS_BOTTLE;
    assert_eq!(mcv_item::crafting::find_result(&grid, 3), None);
    // 2x2 也装不下 5 件（第 3 例：小网格容量判据）。
    let mut grid = g2();
    grid[0] = HONEY_BLOCK;
    grid[1] = GLASS_BOTTLE;
    grid[2] = GLASS_BOTTLE;
    grid[3] = GLASS_BOTTLE;
    assert_eq!(mcv_item::crafting::find_result(&grid, 2), None);
}

#[test]
fn honey_block_from_four_bottles() {
    // honey_block.json：`## / ##`（honey_bottle）→ ×1。
    let mut grid = g2();
    grid[0] = HONEY_BOTTLE;
    grid[1] = HONEY_BOTTLE;
    grid[2] = HONEY_BOTTLE;
    grid[3] = HONEY_BOTTLE;
    assert_eq!(
        mcv_item::crafting::find_result(&grid, 2),
        Some((HONEY_BLOCK, 1))
    );
}
