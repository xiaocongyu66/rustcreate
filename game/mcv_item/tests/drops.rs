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

/// 4c. 族级补全 1-3：石头变体自掉、泥土变体、red_sand/clay/moss/mud
///    （granite/diorite/andesite/tuff/coarse_dirt/podzol/mycelium/dirt_path/
///    red_sand/clay/moss_block/mud.json，均无附魔基数）。
#[test]
fn stone_dirt_surface_families() {
    let table: &[(&str, &[(&str, u8)])] = &[
        // 族 1 石头变体自掉。
        ("granite", &[("granite", 1)]),
        ("diorite", &[("diorite", 1)]),
        ("andesite", &[("andesite", 1)]),
        ("tuff", &[("tuff", 1)]),
        // 族 2 泥土变体：coarse 自掉；podzol/mycelium/dirt_path → dirt。
        ("coarse_dirt", &[("coarse_dirt", 1)]),
        ("podzol", &[("dirt", 1)]),
        ("mycelium", &[("dirt", 1)]),
        ("dirt_path", &[("dirt", 1)]),
        // 族 3 地表：red_sand 自掉；clay → clay_ball×4；moss/mud 自掉。
        ("red_sand", &[("red_sand", 1)]),
        ("clay", &[("clay_ball", 4)]),
        ("moss_block", &[("moss_block", 1)]),
        ("mud", &[("mud", 1)]),
    ];
    for (block, want) in table {
        assert_eq!(
            names(&drops_for_block(bid(block), &mut rng(0))),
            *want,
            "{block} 族 1-3 掉落不符"
        );
    }
}

/// 4d. 族 4 非橡 log 族自掉（birch/spruce/jungle/acacia/dark_oak_log.json，
///    单池 rolls 1，同 oak_log）。
#[test]
fn non_oak_log_family_self_drops() {
    for log in [
        "birch_log",
        "spruce_log",
        "jungle_log",
        "acacia_log",
        "dark_oak_log",
    ] {
        assert_eq!(
            names(&drops_for_block(bid(log), &mut rng(0))),
            [(log, 1)],
            "{log} 应自掉"
        );
    }
}

/// 4e. 族 5 光源/植物产物：glowstone_dust 2-4、prismarine_crystals 2-3、
///    cobweb→string、melon_slice 3-7、pumpkin 自掉、bookshelf→book×3。
#[test]
fn light_plant_product_family() {
    // (方块, 物品, 下界, 上界, 上界掷点)——掷点取 v %（max−min+1）= 宽−1
    // 的定值（u32::MAX 对宽 3/5 整除余 0，不能用）。
    let cases: &[(&str, &str, u8, u8, u32)] = &[
        ("glowstone", "glowstone_dust", 2, 4, 5), // glowstone.json uniform 2..4
        ("sea_lantern", "prismarine_crystals", 2, 3, 1), // sea_lantern.json 2..3
        ("melon", "melon_slice", 3, 7, 4),        // melon.json uniform 3..7
    ];
    for (block, item, lo, hi, roll) in cases {
        assert_eq!(
            names(&drops_for_block(bid(block), &mut rng(0))),
            [(*item, *lo)],
            "{block} 下界 {lo}"
        );
        assert_eq!(
            names(&drops_for_block(bid(block), &mut rng(*roll))),
            [(*item, *hi)],
            "{block} 上界 {hi}"
        );
    }
    assert_eq!(
        names(&drops_for_block(bid("cobweb"), &mut rng(0))),
        [("string", 1)]
    );
    assert_eq!(
        names(&drops_for_block(bid("pumpkin"), &mut rng(0))),
        [("pumpkin", 1)]
    );
    assert_eq!(
        names(&drops_for_block(bid("bookshelf"), &mut rng(0))),
        [("book", 3)],
        "bookshelf.json set_count 3"
    );
}

/// 4f. 族 7 下界：六块自掉 + gold_nugget 2-6 + quartz 1 + ancient_debris 自掉
///    （netherrack/soul_sand/soul_soil/obsidian/magma_block/basalt/
///    ancient_debris.json 自掉；nether_gold_ore.json uniform 2..6；
///    nether_quartz_ore.json ore_drops 基数 1）。
#[test]
fn nether_family_drops() {
    for block in [
        "netherrack",
        "soul_sand",
        "soul_soil",
        "obsidian",
        "magma_block",
        "basalt",
    ] {
        assert_eq!(
            names(&drops_for_block(bid(block), &mut rng(0))),
            [(block, 1)],
            "{block} 应自掉"
        );
    }
    assert_eq!(
        names(&drops_for_block(bid("nether_gold_ore"), &mut rng(0))),
        [("gold_nugget", 2)]
    );
    assert_eq!(
        names(&drops_for_block(bid("nether_gold_ore"), &mut rng(9))),
        [("gold_nugget", 6)],
        "2 + 9 % 5 = 6"
    );
    assert_eq!(
        names(&drops_for_block(bid("nether_quartz_ore"), &mut rng(0))),
        [("quartz", 1)]
    );
    assert_eq!(
        names(&drops_for_block(bid("ancient_debris"), &mut rng(0))),
        [("ancient_debris", 1)]
    );
}

/// 4g. NO_DROP 扩名单：ice 族 + 16 色玻璃（36 名）全部在注册表且在名单内
///    （注册校验在 glass_family_drops_nothing 循环中同批执行）。
#[test]
fn no_drop_list_covers_ice_and_16_glass() {
    let list = mcv_item::inventory::NO_DROP;
    for name in ["ice", "packed_ice", "blue_ice"] {
        assert!(list.contains(&name), "{name} 应在 NO_DROP");
    }
    let stained = list.iter().filter(|n| n.ends_with("stained_glass")).count();
    let stained_pane = list
        .iter()
        .filter(|n| n.ends_with("stained_glass_pane"))
        .count();
    assert_eq!(stained, 16, "16 色染色玻璃");
    assert_eq!(stained_pane, 16, "16 色染色玻璃板");
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

/// 8. 方块经验掉落表（任务 #128-3）：矿石族区间逐一锚定 26.1 注册行——
///    `DropExperienceBlock` 的 `xpRange`（DropExperienceBlock.java:24-37
///    spawnAfterBreak → tryDropExperience，Block.java:619-624）：
///    - iron/gold/copper 恒 0（ConstantInt.of(0)，Blocks.java:262-277、:5208-5211）
///    - coal Uniform(0,2)（:282,:287）、nether_gold Uniform(0,1)（:292）
///    - lapis Uniform(2,5)（:520,:525）、quartz Uniform(2,5)（:2376）
///    - diamond/emerald Uniform(3,7)（:1024-1029、:2177-2182）
///    - redstone Uniform(1,5)（RedStoneOreBlock.java:94-98）
///    非矿石方块无经验（无 DropExperienceBlock 语义）。
#[test]
fn block_experience_ore_table_matches_vanilla() {
    // LCG 确定性采样（同 drops 用例 rng 风格，区间两界都要掷到）。
    let mut state = 0x9E37_79B9_u32;
    let mut rng = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state
    };
    for (name, lo, hi) in [
        ("coal_ore", 0, 2),
        ("deepslate_coal_ore", 0, 2),
        ("nether_gold_ore", 0, 1),
        ("lapis_ore", 2, 5),
        ("deepslate_lapis_ore", 2, 5),
        ("diamond_ore", 3, 7),
        ("deepslate_diamond_ore", 3, 7),
        ("emerald_ore", 3, 7),
        ("deepslate_emerald_ore", 3, 7),
        ("nether_quartz_ore", 2, 5),
        ("redstone_ore", 1, 5),
        ("deepslate_redstone_ore", 1, 5),
        ("copper_ore", 0, 0),
        ("deepslate_copper_ore", 0, 0),
        ("iron_ore", 0, 0),
        ("deepslate_iron_ore", 0, 0),
        ("gold_ore", 0, 0),
        ("deepslate_gold_ore", 0, 0),
    ] {
        for i in 0..64 {
            let v = mcv_item::block_experience(bid(name), &mut rng);
            assert!(
                (lo..=hi).contains(&v),
                "{name} 第 {i} 次采样 {v} 越界 [{lo},{hi}]（26.1 xpRange）"
            );
        }
    }
    // 非矿石：石头/泥土/圆石无经验掉落语义。
    for name in ["stone", "dirt", "cobble"] {
        assert_eq!(
            mcv_item::block_experience(bid(name), &mut rng),
            0,
            "{name} 不得产经验（非 DropExperienceBlock）"
        );
    }
}
