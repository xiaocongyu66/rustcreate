//! 生成配方表判据（#142 配方全表波）。
//!
//! 生成器 ci/gen-recipes.py 从 26.1 数据包 `data/minecraft/recipe/*.json`
//! （仓库外 /root/mc-ref/src-26.1，路径经 MCV_REF_SRC 注入，不入库）直译
//! 「凡输入输出都在本仓 ITEMS」的配方到 game/mcv_item/src/recipes_gen.inc.rs，
//! crafting.rs 匹配走「手写表 ∪ 生成表」（手写优先，重叠条目同结果）。
//! 判据三层：
//! 1. 生成器可复现锁：重跑 == 入库产物（同 mcv_app/tests/lang_items.rs 先例，
//!    OnceLock 单次重跑防并发假红；MCV_REF_SRC 缺席时跳过——CI 无参考树，
//!    本地表内容由第 2/3 层判据常驻锁住）；
//! 2. 表内容抽验：代表配方逐值对 JSON（注释引文件名；反例含 stone→cobble）；
//! 3. 手写表 ∪ 生成表一致性：重叠条目结果/图案/材料集逐格等价（不削 #137）。
//!
//! 镜像语义引注：26.1 JSON 只含实际 pattern（ShapedRecipePattern.Data 无镜像
//! 字段），镜像在运行期匹配（ShapedRecipePattern.java:159-171，
//! symmetrical==false 时 matches(input, true) 同样命中）——生成表不需要也不
//! 应展开镜像（bow.json 等不对称图案由 matches_shaped 的 xFlip 分支覆盖）。

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use mcv_item::crafting::{self, EMPTY_SLOT, GEN_COOKING, GEN_SHAPED, GEN_SHAPELESS};
use mcv_item::{
    ANDESITE, BOW, COARSE_DIRT, COBBLED_DEEPSLATE, COBBLESTONE, DIAMOND_ITEM, DIAMOND_ORE_ITEM,
    DIORITE, GOLD_INGOT, GOLD_NUGGET, GRANITE, IRON_INGOT, IRON_ORE_ITEM, LOG, QUARTZ, RAW_GOLD,
    RAW_IRON, STICK, STONE_ITEM,
};

/// 注册名 → id（无手写常数的小众物品经此取；注册序锁在 tests/items.rs）。
fn id(name: &str) -> u16 {
    mcv_item::item_by_name(name).unwrap_or_else(|| panic!("ITEMS 缺 {name}"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("mcv_item 位于 game/ 下一层")
        .to_path_buf()
}

/// w 宽网格，`at` = (下标, 物品)。
fn placed(w: usize, at: &[(usize, u16)]) -> Vec<u16> {
    let mut grid = vec![EMPTY_SLOT; w * 3];
    for &(i, it) in at {
        grid[i] = it;
    }
    grid
}

/// 按图案串填网格（rows ≤3；keys = (字符, 物品)；' ' 空）——直接以配方 JSON
/// 的 pattern/key 书写测试摆放，逐值对账直观。
fn from_pattern(rows: &[&str], keys: &[(char, u16)]) -> Vec<u16> {
    let w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);
    let mut grid = vec![EMPTY_SLOT; w * rows.len()];
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == ' ' {
                continue;
            }
            let item = keys
                .iter()
                .find(|(k, _)| *k == c)
                .map(|(_, it)| *it)
                .unwrap_or_else(|| panic!("测试 keys 缺字符 {c}"));
            grid[y * w + x] = item;
        }
    }
    grid
}

// ---- 生成表形状/确定性锁（CI 常驻） -----------------------------------------

#[test]
fn gen_tables_shape_and_determinism_lock() {
    // 表规模（ci/gen-recipes.py 输出摘要；改表须同步此锁）。#144 物品地基波
    // 注册全量后：shaped 708/708、shapeless 305/322（余 17 = components 债）、
    // smelting 73/73 —— 缺物品挡住配方清零。
    assert_eq!(GEN_SHAPED.len(), 708, "GEN_SHAPED 规模");
    assert_eq!(GEN_SHAPELESS.len(), 305, "GEN_SHAPELESS 规模");
    assert_eq!(GEN_COOKING.len(), 73, "GEN_COOKING 规模");
    // 配方 id 严格升序（文件名字典序 = 生成确定性；26.1 RecipeMap 插入序）。
    let sorted = |ids: Vec<&str>| ids.windows(2).all(|w| w[0] < w[1]);
    assert!(
        sorted(GEN_SHAPED.iter().map(|r| r.id).collect()),
        "GEN_SHAPED id 序错乱"
    );
    assert!(
        sorted(GEN_SHAPELESS.iter().map(|r| r.id).collect()),
        "GEN_SHAPELESS id 序错乱"
    );
    assert!(
        sorted(GEN_COOKING.iter().map(|r| r.id).collect()),
        "GEN_COOKING id 序错乱"
    );
    // 全部产物 id 落在 ITEMS 注册表内。
    for r in GEN_SHAPED.iter() {
        assert!(
            (r.result.0 as usize) < mcv_item::ITEMS.len(),
            "{} 产物 id 越界",
            r.id
        );
    }
    for r in GEN_SHAPELESS.iter() {
        assert!(
            (r.result.0 as usize) < mcv_item::ITEMS.len(),
            "{} 产物 id 越界",
            r.id
        );
    }
    for r in GEN_COOKING.iter() {
        assert!(
            (r.result.0 as usize) < mcv_item::ITEMS.len(),
            "{} 产物 id 越界",
            r.id
        );
        assert_eq!(r.cook_time, 200, "{} cookingtime（26.1 全表 200）", r.id);
    }
    // shaped 图案 ≤3x3、行宽对齐、字符都有 key（26.1 MAX_SIZE=3，缺 key 反序列化失败）。
    for r in GEN_SHAPED.iter() {
        let pw = r.pattern[0].chars().count();
        assert!(pw <= 3 && r.pattern.len() <= 3, "{} 图案超 3x3", r.id);
        for row in r.pattern {
            assert_eq!(row.chars().count(), pw, "{} 行宽不齐", r.id);
            for c in row.chars() {
                assert!(
                    c == ' ' || r.keys.iter().any(|(k, _)| *k == c),
                    "{} 图案字符 {c} 无 key",
                    r.id
                );
            }
        }
    }
}

// ---- 抽验：代表配方逐值对 JSON（注释引 recipe/*.json）------------------------

#[test]
fn diorite_two_by_two_checker() {
    // recipe/diorite.json：`CQ / QC`，C=cobblestone Q=quartz → ×2。
    let grid = from_pattern(&["CQ", "QC"], &[('C', COBBLESTONE), ('Q', QUARTZ)]);
    assert_eq!(crafting::find_result(&grid, 2), Some((DIORITE, 2)));
    // 3x3 角摆放（bbox 裁剪归一；第 2 例）。
    let grid = from_pattern(&["CQ ", "QC ", "   "], &[('C', COBBLESTONE), ('Q', QUARTZ)]);
    assert_eq!(crafting::find_result(&grid, 3), Some((DIORITE, 2)));
    // 同字符对角错位不成（第 3 例负判据）。
    let grid = from_pattern(&["QC", "QC"], &[('C', COBBLESTONE), ('Q', QUARTZ)]);
    assert_eq!(crafting::find_result(&grid, 2), None);
}

#[test]
fn andesite_granite_shapeless() {
    // recipe/andesite.json：shapeless diorite+cobblestone → ×2。
    let grid = placed(2, &[(0, DIORITE), (3, COBBLESTONE)]);
    assert_eq!(crafting::find_result(&grid, 2), Some((ANDESITE, 2)));
    // recipe/granite.json：shapeless diorite+quartz → ×1（json 无 count 键）。
    let grid = placed(3, &[(4, DIORITE), (7, QUARTZ)]);
    assert_eq!(crafting::find_result(&grid, 3), Some((GRANITE, 1)));
    // cobblestone 换成 stone 不出 andesite（ingredient 精确性负判据）。
    let grid = placed(2, &[(0, DIORITE), (3, STONE_ITEM)]);
    assert_eq!(crafting::find_result(&grid, 2), None);
}

#[test]
fn coarse_dirt_dirt_gravel_checker() {
    // recipe/coarse_dirt.json：`DG / GD`，D=dirt G=gravel → ×4。
    let grid = from_pattern(
        &["DG", "GD"],
        &[('D', mcv_item::DIRT_ITEM), ('G', mcv_item::GRAVEL_ITEM)],
    );
    assert_eq!(crafting::find_result(&grid, 2), Some((COARSE_DIRT, 4)));
    // 同侧摆放不成（第 2 例负判据）。
    let grid = from_pattern(
        &["DG", "DG"],
        &[('D', mcv_item::DIRT_ITEM), ('G', mcv_item::GRAVEL_ITEM)],
    );
    assert_eq!(crafting::find_result(&grid, 2), None);
}

#[test]
fn bow_pattern_and_mirror() {
    // recipe/bow.json：` #X / # X /  #X`（#=stick X=string）→ ×1。
    let grid = from_pattern(&[" #X", "# X", " #X"], &[('#', STICK), ('X', id("string"))]);
    assert_eq!(crafting::find_result(&grid, 3), Some((BOW, 1)));
    // 镜像摆放（xFlip）同出——原版在运行期展开（ShapedRecipePattern.java:162），
    // JSON 只含实际 pattern，本表不展开镜像（生成表语义引注见文件头）。
    let grid = from_pattern(&["X# ", "X #", "X# "], &[('#', STICK), ('X', id("string"))]);
    assert_eq!(crafting::find_result(&grid, 3), Some((BOW, 1)));
    // 高度不足（2x2 放不下 3 行）不成（第 3 例负判据）。
    let grid = from_pattern(&["#X", " #"], &[('#', STICK), ('X', id("string"))]);
    assert_eq!(crafting::find_result(&grid, 2), None);
}

#[test]
fn golden_tools_from_gold_ingot_tag() {
    // recipe/golden_pickaxe.json：`XXX / _#_ / _#_`，X=`#minecraft:gold_tool_materials`
    // （26.1 tags/item/gold_tool_materials.json = [gold_ingot]）→ ×1。
    let grid = from_pattern(&["XXX", " # ", " # "], &[('X', GOLD_INGOT), ('#', STICK)]);
    assert_eq!(
        crafting::find_result(&grid, 3),
        Some((id("golden_pickaxe"), 1))
    );
    // recipe/golden_sword.json：`X / X / #` → ×1（手写表只有木/石/铁/钻石剑）。
    let grid = from_pattern(&["X  ", "X  ", "#  "], &[('X', GOLD_INGOT), ('#', STICK)]);
    assert_eq!(
        crafting::find_result(&grid, 3),
        Some((id("golden_sword"), 1))
    );
    // 镐身错位（材料不在顶排）不成（负判据）。
    let grid = from_pattern(&[" # ", "XXX", " # "], &[('X', GOLD_INGOT), ('#', STICK)]);
    assert_eq!(crafting::find_result(&grid, 3), None);
}

#[test]
fn stone_axe_accepts_both_registered_stone_materials() {
    // recipe/stone_axe.json：`XX / X# / _#`，X=`#minecraft:stone_tool_materials`
    // （tags/item/stone_tool_materials.json = cobblestone+blackstone+cobbled_deepslate；
    // blackstone 未注册被窄化——ci/recipes-gap.md 窄化节）。
    let cob = from_pattern(&["XX ", "X# ", " # "], &[('X', COBBLESTONE), ('#', STICK)]);
    assert_eq!(crafting::find_result(&cob, 3), Some((id("stone_axe"), 1)));
    // cobbled_deepslate 同 tag 命中（手写表只有 cobblestone——生成表扩展）。
    let cds = from_pattern(
        &["XX ", "X# ", " # "],
        &[('X', COBBLED_DEEPSLATE), ('#', STICK)],
    );
    assert_eq!(crafting::find_result(&cds, 3), Some((id("stone_axe"), 1)));
    // 混材质（tag 语义）同出（第 3 例；保持在 2 宽 bbox 内）。
    let mut grid = cob;
    grid[3] = COBBLED_DEEPSLATE;
    assert_eq!(crafting::find_result(&grid, 3), Some((id("stone_axe"), 1)));
}

#[test]
fn iron_and_diamond_tool_completion() {
    // recipe/iron_axe.json：`XX / X# / _#` → ×1。
    let grid = from_pattern(&["XX ", "X# ", " # "], &[('X', IRON_INGOT), ('#', STICK)]);
    assert_eq!(crafting::find_result(&grid, 3), Some((id("iron_axe"), 1)));
    // recipe/iron_shovel.json：`X / # / #` → ×1。
    let grid = from_pattern(&["X  ", "#  ", "#  "], &[('X', IRON_INGOT), ('#', STICK)]);
    assert_eq!(
        crafting::find_result(&grid, 3),
        Some((id("iron_shovel"), 1))
    );
    // recipe/diamond_axe.json → ×1。
    let grid = from_pattern(&["XX ", "X# ", " # "], &[('X', DIAMOND_ITEM), ('#', STICK)]);
    assert_eq!(
        crafting::find_result(&grid, 3),
        Some((id("diamond_axe"), 1))
    );
    // recipe/diamond_shovel.json → ×1。
    let grid = from_pattern(&["X  ", "#  ", "#  "], &[('X', DIAMOND_ITEM), ('#', STICK)]);
    assert_eq!(
        crafting::find_result(&grid, 3),
        Some((id("diamond_shovel"), 1))
    );
}

#[test]
fn gold_ingot_nugget_roundtrip() {
    // recipe/gold_ingot_from_nuggets.json：`### / ### / ###`（gold_nugget）→ ×1。
    let grid = placed(3, &[(0, GOLD_NUGGET), (4, GOLD_NUGGET), (8, GOLD_NUGGET)]);
    // 3 枚不成（第 1 例负判据先行）。
    assert_eq!(crafting::find_result(&grid, 3), None);
    let full = [GOLD_NUGGET; 9];
    assert_eq!(crafting::find_result(&full, 3), Some((GOLD_INGOT, 1)));
    // recipe/gold_nugget.json：shapeless gold_ingot → ×9。
    let grid = placed(2, &[(3, GOLD_INGOT)]);
    assert_eq!(crafting::find_result(&grid, 2), Some((GOLD_NUGGET, 9)));
}

// ---- smelting 入口（smelt_result；furnace UI 待物品注册波，债见缺口报告）-----

#[test]
fn smelting_raw_metals_and_cobble() {
    // recipe/iron_ingot_from_smelting_raw_iron.json：raw_iron → iron_ingot ×1。
    assert_eq!(crafting::smelt_result(RAW_IRON), Some((IRON_INGOT, 1)));
    // recipe/gold_ingot_from_smelting_raw_gold.json：raw_gold → gold_ingot ×1。
    assert_eq!(crafting::smelt_result(RAW_GOLD), Some((GOLD_INGOT, 1)));
    // recipe/stone.json：cobblestone → stone ×1（cookingtime 200）。
    assert_eq!(crafting::smelt_result(COBBLESTONE), Some((STONE_ITEM, 1)));
    // find_cooking 拿得到时长（与 smelt_result 同一条）。
    let r = crafting::find_cooking(COBBLESTONE).expect("cobblestone smelting");
    assert_eq!(r.id, "stone");
    assert_eq!(r.cook_time, 200);
}

#[test]
fn smelting_ore_entries_from_handwritten_table() {
    // 手写表先中（重叠条目手写优先，行为不变）：
    // COOKING[0] iron_ore → iron_ingot（= recipe/iron_ingot_from_smelting_iron_ore.json）。
    assert_eq!(crafting::smelt_result(IRON_ORE_ITEM), Some((IRON_INGOT, 1)));
    // COOKING[1] diamond_ore → diamond（= recipe/diamond_from_smelting_diamond_ore.json）。
    assert_eq!(
        crafting::smelt_result(DIAMOND_ORE_ITEM),
        Some((DIAMOND_ITEM, 1))
    );
}

#[test]
fn smelting_negatives_known_gaps() {
    // stone → smooth_stone（smooth_stone.json；#144 前产物未注册时为负例）。
    assert_eq!(
        crafting::smelt_result(STONE_ITEM),
        Some((id("smooth_stone"), 1))
    );
    // 反例：钻石成品无熔炼配方（26.1 全表无 input=diamond 的 smelting）。
    assert_eq!(crafting::smelt_result(DIAMOND_ITEM), None);
    // #144 物品地基波转正后，旧负例全部变正（产物已注册）：
    // log → charcoal（charcoal.json）；raw_copper → copper_ingot（缺口报告
    // 旧 top1）；sand → glass（glass.json）。
    assert_eq!(
        crafting::smelt_result(mcv_item::LOG),
        Some((id("charcoal"), 1))
    );
    assert_eq!(
        crafting::smelt_result(mcv_item::RAW_COPPER),
        Some((id("copper_ingot"), 1))
    );
    assert_eq!(
        crafting::smelt_result(mcv_item::SAND_ITEM),
        Some((id("glass"), 1))
    );
}

// ---- 手写表 ∪ 生成表一致性（#137 不回归的强锁）-------------------------------

/// keys 里字符 → 材料 id 集（排序后比较；tag 窄化两侧同源应一致）。
fn ing_set(keys: &[(char, crafting::Ingredient)], c: char) -> Vec<u16> {
    let mut v = keys
        .iter()
        .find(|(k, _)| *k == c)
        .map(|(_, l)| l.to_vec())
        .unwrap_or_default();
    v.sort_unstable();
    v
}

#[test]
fn overlap_with_handwritten_is_superset() {
    // 手写优先合并且生成表为 JSON 直译超集 ⇒ 生成表对重叠条目的材料面
    // 只会【更宽】不会更窄（如 stone_tool_materials 生成 [cobblestone,
    // cobbled_deepslate] ⊇ 手写 [cobblestone]；窄化成员差异见缺口报告）。
    for h in crafting::SHAPED.iter() {
        let Some(g) = GEN_SHAPED.iter().find(|g| g.id == h.id) else {
            continue; // 手写独有条目——不属重叠，跳过
        };
        assert_eq!(g.result, h.result, "重叠条目 {} 结果不一致", h.id);
        assert_eq!(g.pattern.len(), h.pattern.len(), "{} 行数不一致", h.id);
        let pw = g.pattern[0].chars().count();
        assert_eq!(h.pattern[0].chars().count(), pw, "{} 行宽不一致", h.id);
        for y in 0..g.pattern.len() {
            for x in 0..pw {
                let gc = g.pattern[y].chars().nth(x).unwrap();
                let hc = h.pattern[y].chars().nth(x).unwrap();
                assert_eq!(gc == ' ', hc == ' ', "{} ({x},{y}) 空占不一致", h.id);
                if gc != ' ' {
                    let hs = ing_set(h.keys, hc);
                    let gs = ing_set(g.keys, gc);
                    assert!(
                        hs.iter().all(|i| gs.contains(i)),
                        "{} ({x},{y}) 生成材料集 {gs:?} 未包含手写 {hs:?}",
                        h.id
                    );
                }
            }
        }
    }
    for h in crafting::SHAPELESS.iter() {
        if let Some(g) = GEN_SHAPELESS.iter().find(|g| g.id == h.id) {
            assert_eq!(g.result, h.result, "重叠条目 {} 结果不一致", h.id);
            assert_eq!(
                g.ingredients.len(),
                h.ingredients.len(),
                "{} 件数不一致",
                h.id
            );
            // 无序件匹配：手写每一件的材料集都要被生成侧某件覆盖。
            for hi in h.ingredients.iter() {
                assert!(
                    g.ingredients
                        .iter()
                        .any(|gi| hi.iter().all(|i| gi.contains(i))),
                    "{} 生成材料未包含手写件 {hi:?}",
                    h.id
                );
            }
        }
    }
    for h in crafting::COOKING.iter() {
        // 手写 smelting 与生成表按「同一输入集」对号（id 命名不同：
        // 手写 "iron_ingot" vs JSON "iron_ingot_from_smelting_iron_ore"）。
        let mut hs = h.input.to_vec();
        hs.sort_unstable();
        if let Some(g) = GEN_COOKING.iter().find(|g| {
            let mut gs = g.input.to_vec();
            gs.sort_unstable();
            gs == hs
        }) {
            assert_eq!(g.result, h.result, "smelting {} 产物不一致", h.id);
            assert_eq!(g.cook_time, h.cook_time, "{} cook_time 不一致", h.id);
        }
    }
}

#[test]
fn handwritten_priority_unchanged_for_overlapped_grids() {
    // 手写优先合并不改 #137 行为：满 2x2 板仍先中 crafting_table。
    let grid = [
        mcv_item::PLANKS,
        mcv_item::SPRUCE_PLANKS,
        mcv_item::BIRCH_PLANKS,
        mcv_item::JUNGLE_PLANKS,
    ];
    assert_eq!(
        crafting::find_result(&grid, 2),
        Some((mcv_item::CRAFTING_TABLE, 1))
    );
    // 楼梯三角仍出 oak_stairs（生成表里同 id 条目在后）。
    let mut grid = vec![EMPTY_SLOT; 9];
    for i in [0usize, 3, 4, 6, 7, 8] {
        grid[i] = mcv_item::PLANKS;
    }
    assert_eq!(
        crafting::find_result(&grid, 3),
        Some((mcv_item::OAK_STAIRS, 4))
    );
}

// ---- 快照：缺口报告读取 + 生成器复现锁共用一次初始化 --------------------------
//
// OnceLock 单次执行（同 mcv_app/tests/lang_items.rs run 38156491631 教训）：
// 多测试并发跑同一 binary，若各自读文件/重跑生成器，脚本 write_text 截断
// 瞬间另一线程读空会假红。全部文件读取走这份快照；MCV_REF_SRC 在场时顺带
// 重跑生成器做「重跑 == 入库」同步断言（CI 无参考树 → 只锁入库产物本身）。
struct Snapshot {
    /// 入库缺口报告文本。
    gap: String,
    /// 同步失配信息（Some = 重跑产物 != 入库产物 / 生成器失败）。
    mismatch: Option<String>,
}

fn snapshot() -> &'static Snapshot {
    static ONCE: OnceLock<Snapshot> = OnceLock::new();
    ONCE.get_or_init(|| {
        let root = repo_root();
        let inc_path = root.join("game/mcv_item/src/recipes_gen.inc.rs");
        let gap_path = root.join("ci/recipes-gap.md");
        let gap = std::fs::read_to_string(&gap_path)
            .expect("ci/recipes-gap.md 缺失（ci/gen-recipes.py 产物须入库）");
        let mut mismatch = None;
        match std::env::var("MCV_REF_SRC") {
            Err(_) => eprintln!("跳过复现锁：本机未设 MCV_REF_SRC（CI 无 26.1 参考树）"),
            Ok(ref_src) => {
                let committed_inc = std::fs::read_to_string(&inc_path).unwrap_or_default();
                let out = Command::new("python3")
                    .arg(root.join("ci/gen-recipes.py"))
                    .env("MCV_REF_SRC", ref_src)
                    .current_dir(&root)
                    .output()
                    .expect("运行 ci/gen-recipes.py");
                if !out.status.success() {
                    mismatch = Some(format!(
                        "gen-recipes.py 失败: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ));
                } else {
                    let regen_inc = std::fs::read_to_string(&inc_path).unwrap_or_default();
                    let regen_gap = std::fs::read_to_string(&gap_path).unwrap_or_default();
                    if committed_inc != regen_inc {
                        mismatch = Some(String::from(
                            "recipes_gen.inc.rs 与生成器输出不同步：改 ITEMS/配方后须重跑 ci/gen-recipes.py 再提交",
                        ));
                    } else if gap != regen_gap {
                        mismatch = Some(String::from(
                            "recipes-gap.md 与生成器输出不同步：须重跑 ci/gen-recipes.py 再提交",
                        ));
                    }
                }
            }
        }
        Snapshot { gap, mismatch }
    })
}

// ---- 缺口报告（ci/recipes-gap.md，入库；CI 常驻）-----------------------------

#[test]
fn gap_report_exists_and_names_known_gaps() {
    let gap = &snapshot().gap;
    // #144 物品地基波把 ITEMS 扩到 1381 后，缺物品挡路配方清零：
    // 任务书点名的抽验例（旧缺口 top：torch/furnace/bread/charcoal/wheat/
    // copper_ingot）全部转正，挡路物品表必须为空表头。
    for item in [
        "torch",
        "furnace",
        "bread",
        "charcoal",
        "wheat",
        "copper_ingot",
    ] {
        assert!(
            !gap.contains(&format!("| {item} | ")),
            "{item} 已注册，不得再出现在挡路物品表"
        );
    }
    assert!(
        gap.contains("| crafting_shaped | 708 | 708 | 0 |"),
        "shaped 缺物品跳过必须为 0"
    );
    assert!(
        gap.contains("| crafting_shapeless | 322 | 305 | 0 |"),
        "shapeless 缺物品跳过必须为 0（余 17 = components 债）"
    );
    assert!(gap.contains("| smelting | 73 | 73 | 0 |"), "smelting 全收");
    // 债机制登记（blasting/campfire 本波不接线；stonecutting 275 条）。
    assert!(gap.contains("blasting"), "缺口报告缺 blasting 债");
    assert!(gap.contains("campfire_cooking"), "缺口报告缺 campfire 债");
    assert!(
        gap.contains("| stonecutting | 275 |"),
        "缺口报告 stonecutting 计数漂移（26.1 全表 275）"
    );
}

// ---- 生成器可复现锁（重跑 == 入库；MCV_REF_SRC 缺席时跳过）--------------------

#[test]
fn generator_output_matches_committed_artifacts() {
    if let Some(m) = &snapshot().mismatch {
        panic!("{m}");
    }
}
