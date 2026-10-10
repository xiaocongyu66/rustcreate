//! 掉落经济闭环（任务板 #90）表驱动测试：掉落表族 × 26.1 loot 语义、
//! 掉落门裁决（徒手 vs 石镐挖铁矿）、玻璃族明确不掉。
//!
//! 对账基准（仓库外 /root/mc-ref/src-26.1）：
//! - `data/minecraft/loot_table/blocks/*.json`（各 family 基数/概率）
//! - `ServerPlayerGameMode.java:295-299`：`canDestroy =
//!   hasCorrectToolForDrops(state)`，`if (changed && canDestroy)
//!   block.playerDestroy(...)`——**原版徒手挖铁矿不掉 raw_iron**
//!   （iron_ore 注册带 requiresCorrectToolForDrops，Blocks.java:270-274）。

use mcv_core::BlockId;
use mcv_item::mining::has_correct_tool;
use mcv_item::{ItemStack, STONE_PICKAXE_INDEX, WOODEN_PICKAXE_INDEX, drops_for_block};

/// 按注册名取方块 id（BLOCKS 字典序生成，name 即主键）。
fn bid(name: &str) -> BlockId {
    let i = mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("方块 {name} 未注册"));
    BlockId(i as u16)
}

/// 确定性 rng：每次返回 `v`（drops_for_block 的概率/数量掷点都吃它）。
fn rng(v: u32) -> impl FnMut() -> u32 {
    move || v
}

/// 期望值写法：`[(物品名, 数量)]`（同物品数量随机时用最小掷点 0 → min）。
fn names(drops: &[ItemStack]) -> Vec<(&'static str, u8)> {
    drops.iter().map(|d| (d.def().name, d.count)).collect()
}

/// 1. 掉落表族：每行一条用例，期望 = 26.1 loot json 的无附魔基数语义。
///    必掉池不消耗 rng；数量区间在 min 掷点取下界。
#[test]
fn drop_families_table() {
    // (方块, 期望掉落 [(物品, 数量)])
    let table: &[(&str, &[(&str, u8)])] = &[
        ("stone", &[("cobblestone", 1)]),
        ("cobble", &[("cobblestone", 1)]),
        ("deepslate", &[("cobbled_deepslate", 1)]),
        ("dirt", &[("dirt", 1)]),
        ("grass", &[("dirt", 1)]),
        ("snow_grass", &[("dirt", 1)]),
        ("farmland", &[("dirt", 1)]),
        ("sand", &[("sand", 1)]),
        ("coal_ore", &[("coal", 1)]),
        ("deepslate_coal_ore", &[("coal", 1)]),
        ("iron_ore", &[("raw_iron", 1)]),
        ("deepslate_iron_ore", &[("raw_iron", 1)]),
        // 铜矿基数 2-5（copper_ore.json set_count uniform 2..5）。
        ("copper_ore", &[("raw_copper", 2)]),
        ("deepslate_copper_ore", &[("raw_copper", 2)]),
        ("gold_ore", &[("raw_gold", 1)]),
        ("deepslate_gold_ore", &[("raw_gold", 1)]),
        // 红石 4-5、青金 4-9（各自 json uniform 基数）。
        ("redstone_ore", &[("redstone", 4)]),
        ("deepslate_redstone_ore", &[("redstone", 4)]),
        ("lapis_ore", &[("lapis", 4)]),
        ("deepslate_lapis_ore", &[("lapis", 4)]),
        ("diamond_ore", &[("diamond", 1)]),
        ("deepslate_diamond_ore", &[("diamond", 1)]),
        ("emerald_ore", &[("emerald", 1)]),
        ("deepslate_emerald_ore", &[("emerald", 1)]),
        ("log", &[("log", 1)]),
        ("planks", &[("planks", 1)]),
    ];
    for (block, want) in table {
        let got = names(&drops_for_block(bid(block), &mut rng(0)));
        assert_eq!(got, *want, "{block} 掉落族不符");
    }
    // 铜矿计数上界：掷点最大 → 5（u8 区间 max−min+1 = 4 取模）。
    assert_eq!(
        names(&drops_for_block(bid("copper_ore"), &mut rng(u32::MAX))),
        [("raw_copper", 5)]
    );
    // 青金计数上界 9（uniform 4..9；掷点 5 % 6 = 5 → 4+5）。
    assert_eq!(
        names(&drops_for_block(bid("lapis_ore"), &mut rng(5))),
        [("lapis", 9)]
    );
}

/// 2. 砂砾 alternatives：燧石 10%，未触发回退掉自身（gravel.json）。
#[test]
fn gravel_flint_or_self() {
    // 掷点 0 < 100‰ → flint。
    assert_eq!(
        names(&drops_for_block(bid("gravel"), &mut rng(0))),
        [("flint", 1)]
    );
    // 掷点 ≥ 100‰ → 回退 gravel。
    assert_eq!(
        names(&drops_for_block(bid("gravel"), &mut rng(500))),
        [("gravel", 1)]
    );
}

/// 3. 树叶族三独立池（oak_leaves.json）：树苗 5%o + 苹果 0.5%o + 木棍 2%o
///    ×1-2，可同 tick 多掉；全部未触发则空手而归。
#[test]
fn oak_leaves_three_independent_pools() {
    // 全中：树苗 + 苹果 + 木棍（木棍数量掷点 0 → 1）。
    assert_eq!(
        names(&drops_for_block(bid("leaves"), &mut rng(0))),
        [("oak_sapling", 1), ("apple", 1), ("stick", 1)]
    );
    // 全不中（600‰ > 全部池）：空。
    assert!(drops_for_block(bid("leaves"), &mut rng(600)).is_empty());
    // 木棍数量上界 2：掷点序列 (100,100,0,1) → 树苗不中（100 ≥ 50）、
    // 苹果不中、木棍中（0 < 20）、数量掷点 1 % 2 = 1 → 1+1 = 2。
    let mut seq = {
        let rolls = [100u32, 100, 0, 1];
        let mut i = 0usize;
        move || {
            let v = rolls[i.min(3)];
            i += 1;
            v
        }
    };
    assert_eq!(
        names(&drops_for_block(bid("leaves"), &mut seq)),
        [("stick", 2)]
    );
}

/// 4. 各树种树叶全池（各 *_leaves.json）：树苗（5%o、丛林 2.5%o）+ 木棍
///    （六树叶各有 2%o ×1-2）+ 苹果（oak_leaves.json 与 dark_oak_leaves.json
///    各有 0.5%o 池——"苹果只橡树"为旧表误记）。
#[test]
fn tree_leaves_sapling_families() {
    let table: &[(&str, &[(&str, u8)])] = &[
        ("birch_leaves", &[("birch_sapling", 1), ("stick", 1)]),
        ("spruce_leaves", &[("spruce_sapling", 1), ("stick", 1)]),
        ("acacia_leaves", &[("acacia_sapling", 1), ("stick", 1)]),
        (
            "dark_oak_leaves",
            &[("dark_oak_sapling", 1), ("apple", 1), ("stick", 1)],
        ),
        ("jungle_leaves", &[("jungle_sapling", 1), ("stick", 1)]),
    ];
    for (leaves, want) in table {
        let got = names(&drops_for_block(bid(leaves), &mut rng(0)));
        assert_eq!(got, *want, "{leaves} 全池（rng 0 全命中）");
        // 全不中（600‰ > 全部池）。
        assert!(drops_for_block(bid(leaves), &mut rng(600)).is_empty());
    }
}

/// 4b. 雪族掉落：snow.json 按 layers 1..8 各 set_count=layers 掉雪球
///    （本引擎无方块状态、雪按单层建模 → 1）；snow_block.json 无 silk
///    分支 set_count 4（silk 掉自身；本引擎无 silk 附魔恒走 4 分支）。
#[test]
fn snow_family_drops_snowballs() {
    assert_eq!(
        names(&drops_for_block(bid("snow"), &mut rng(0))),
        [("snowball", 1)],
        "雪层 = 1 层 → 1 雪球"
    );
    assert_eq!(
        names(&drops_for_block(bid("snow_block"), &mut rng(0))),
        [("snowball", 4)],
        "雪块无 silk → 4 雪球"
    );
}

/// 5. 掉落门裁决用例：**徒手挖铁矿无掉落、石镐挖铁矿掉 raw_iron**。
///    26.1 裁决：destroyBlock 的 playerDestroy（掉落）被
///    hasCorrectToolForDrops 前置门控（ServerPlayerGameMode.java:295-299）；
///    iron_ore 带 requiresCorrectToolForDrops（Blocks.java:270-274）。
///    引擎侧同构：drops_for_block 只管"掉什么"，门由 has_correct_tool 把守，
///    与 game.rs destroy_block 的调用序一致。
#[test]
fn drop_gate_ruling_bare_hand_vs_stone_pickaxe() {
    let ore = bid("iron_ore");
    // 表本身会掉 raw_iron（表驱动能力）。
    assert_eq!(names(&drops_for_block(ore, &mut rng(0))), [("raw_iron", 1)]);
    // 但徒手（None）过不了门 → 实际不产出（26.1 语义，非"原版徒手掉铁"）。
    assert!(
        !has_correct_tool(ore, None),
        "26.1 裁决：铁矿 requiresCorrectToolForDrops，徒手非正确工具"
    );
    // 石镐（tier 2）过门 → 产出。
    let pick = ItemStack::new(STONE_PICKAXE_INDEX, 1);
    assert!(has_correct_tool(ore, Some(&pick)));
    // 木镐（tier 1 < 2）也过不了门：能磨掉但不掉。
    let wooden = ItemStack::new(WOODEN_PICKAXE_INDEX, 1);
    assert!(!has_correct_tool(ore, Some(&wooden)));
}

/// 6. 玻璃族明确不掉（glass.json 池只放行 silk touch；引擎无 silk）。
#[test]
fn glass_family_drops_nothing() {
    for name in mcv_item::inventory::NO_DROP {
        assert!(
            drops_for_block(bid(name), &mut rng(0)).is_empty(),
            "{name} 必须不掉"
        );
    }
    // NO_DROP 名单本身可解析为已注册方块（防注册名漂移后用例空转）。
    for name in mcv_item::inventory::NO_DROP {
        assert!(
            mcv_core::BLOCKS.iter().any(|b| b.name == *name),
            "{name} 未注册"
        );
    }
}

/// 7. 表完整性：所有"必掉"族测试已覆盖 DROP_TABLE 全部行（族用例即行用例）；
///    未注册/未列名方块一律空表（宁缺勿错）。
#[test]
fn unlisted_blocks_drop_nothing() {
    // 花与未进表的方块不掉（宁缺勿错）。
    for name in ["flower_red", "flower_yellow", "amethyst_cluster", "spawner"] {
        assert!(
            drops_for_block(bid(name), &mut rng(0)).is_empty(),
            "{name} 应空"
        );
    }
}
