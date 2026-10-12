//! tags_gen.inc.rs 等价锁（#178 刀1 对拍测试）。
//!
//! 计数基准为「两路独立得出」：
//!   路 1 = `python3 ci/gen-tags.py` 的 id 侧解出；
//!   路 2 = 本地对原版 data/minecraft/tags/block/**/*.json 做纯名字集
//!          合并（`#` 递归展开 + 去重）的独立计数，与路 1 全表核对一致。
//! 版权卫生：本文件只写计数与个别成员名（事实），不含原版 JSON 原文。

use mcv_core::tags::ALL_TAGS;
use mcv_registry::TagEntry;

fn tag(path: &str) -> &'static TagEntry {
    ALL_TAGS
        .iter()
        .find(|t| t.key.path == path)
        .unwrap_or_else(|| panic!("tags_gen 缺 tag: {path}"))
}

/// 引擎方块名 → GEN_BLOCKS 行序 id（tag id 域）。
fn block_id(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("BLOCKS 缺方块: {name}")) as u16
}

/// 总数锁：248 个原版 block tag 文件全部入库（含 3 个空表）。
#[test]
fn tag_total_count() {
    assert_eq!(ALL_TAGS.len(), 248);
    assert_eq!(ALL_TAGS.iter().map(|t| t.set.len()).sum::<usize>(), 3998);
}

/// ≥12 个 tag 成员数对拍（两路独立计数，见模块注释）。
#[test]
fn tag_member_counts() {
    let table: &[(&str, usize)] = &[
        ("base_stone_overworld", 6),
        ("features_cannot_replace", 7),
        ("lava_pool_stone_cannot_replace", 62),
        ("replaceable", 29),
        ("mineable/pickaxe", 482),
        ("mineable/axe", 286),
        ("mineable/shovel", 36),
        ("mineable/hoe", 29),
        ("logs", 44),
        ("leaves", 11),
        ("planks", 12),
        ("saplings", 11),
        ("flowers", 31),
        ("walls", 26),
        ("fence_gates", 12),
        ("stone_bricks", 4),
        ("dirt", 3),
        ("air", 3),
        ("underwater_bonemeals", 16),
    ];
    for (path, n) in table {
        assert_eq!(tag(path).set.len(), *n, "tag {path} 成员数漂移");
    }
}

/// `#` 解引用正确性：lava_pool_stone_cannot_replace = 解出
/// `#features_cannot_replace` ∪ `#leaves` ∪ `#logs` 后的并集；
/// 断言其 ⊇ 三个嵌套 tag 全体成员（运行期产物里不许再有引用）。
#[test]
fn hash_ref_dereferenced() {
    let lava = tag("lava_pool_stone_cannot_replace");
    assert!(
        lava.set.contains(block_id("bedrock")),
        "⊇ #features_cannot_replace"
    );
    for t in ["features_cannot_replace", "leaves", "logs"] {
        for id in tag(t).set.iter().copied() {
            assert!(lava.set.contains(id), "#{t} 成员 {id} 未解入 lava_pool");
        }
    }
    // 引擎侧代表成员（oak_leaves→leaves(7)、oak_log→log(6)）。
    assert!(lava.set.contains(block_id("leaves")));
    assert!(lava.set.contains(block_id("log")));
}

/// 个别成员抽查（LEGACY 别名映射 grass_block→grass、cobblestone→cobble 等
/// 经 id 正确落地）。
#[test]
fn individual_members() {
    let bs = tag("base_stone_overworld");
    for n in [
        "stone",
        "granite",
        "diorite",
        "andesite",
        "tuff",
        "deepslate",
    ] {
        assert!(bs.set.contains(block_id(n)), "base_stone_overworld 缺 {n}");
    }
    assert!(tag("replaceable").set.contains(block_id("water")));
    // LEGACY 官方名别名三例：cobblestone→cobble、poppy→flower_red、
    // dandelion→flower_yellow、grass_block→grass。
    assert!(tag("mineable/pickaxe").set.contains(block_id("cobble")));
    assert!(tag("flowers").set.contains(block_id("flower_red")));
    assert!(tag("flowers").set.contains(block_id("flower_yellow")));
    assert!(tag("animals_spawnable_on").set.contains(block_id("grass")));
    assert!(tag("leaves").set.contains(block_id("leaves")));
    assert!(tag("planks").set.contains(block_id("planks")));
    // 原版空表如实入库（optional/无值 JSON）。
    for e in [
        "incorrect_for_diamond_tool",
        "incorrect_for_netherite_tool",
        "supports_frogspawn",
    ] {
        assert!(tag(e).set.is_empty(), "{e} 应为空表");
    }
}

/// 全表 id 升序且无重复（TagSet::contains 二分前置契约）。
#[test]
fn ids_sorted_unique() {
    for t in ALL_TAGS.iter() {
        let ids: Vec<u16> = t.set.iter().copied().collect();
        for w in ids.windows(2) {
            assert!(w[0] < w[1], "tag {} 非严格升序: {w:?}", t.key.path);
        }
        // 键的 registry 命名空间统一 "block"，路径非空。
        assert_eq!(t.key.registry, "block");
        assert!(!t.key.path.is_empty());
    }
}
