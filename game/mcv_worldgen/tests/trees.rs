//! T 波树种结构不变量测试（任务板 #135）。
//!
//! 树干高度对拍区间：`TrunkPlacer.getTreeHeight`（trunkplacers/TrunkPlacer
//! .java:58-60）= `base + nextInt(A+1) + nextInt(B+1)`，参数取
//! `data/worldgen/features/TreeFeatures.java`（缩写 TJ）：
//! oak (4,2,0)→4..6（TJ:136）、birch (5,2,0)→5..7（TJ:181）、
//! jungle (4,8,0)→4..12（TJ:189）、spruce (5,2,1)→5..9（TJ:433-441）、
//! acacia (5,2,2)→5..9（TJ:421）、dark oak (6,2,1)→6..9（TJ:139-147）。
//!
//! 叶数区间不是 Java 随机表直读值，而是本文件按各冠层 placer 逐层解析推得
//! 的闭包（每树种测试 doc 注明推导链）；实测值必须整体落在闭包内。

use std::collections::BTreeMap;

use mcv_core::BLOCKS;
use mcv_worldgen::trees::{ALL_KINDS, BlockSetter, Rng, TreeKind, TreesRng, ids, place_tree};

/// 画布：BTreeMap 保序 → 全量快照可直接 `PartialEq`（字节级确定性）。
#[derive(Default, PartialEq, Eq, Debug, Clone)]
struct Canvas {
    blocks: BTreeMap<(i32, i32, i32), u16>,
}

impl BlockSetter for Canvas {
    fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        self.blocks.insert((x, y, z), id);
    }
}

/// 基点 (0, 64, 0) 种一棵并返回画布。
fn grow(seed: u64, kind: TreeKind) -> Canvas {
    let mut rng = TreesRng::new(seed);
    let mut canvas = Canvas::default();
    place_tree(&mut rng, 0, 64, 0, kind, &mut canvas);
    canvas
}

fn count_id(canvas: &Canvas, id: u16) -> usize {
    canvas.blocks.values().filter(|v| **v == id).count()
}

fn top_log_y(canvas: &Canvas, log_id: u16) -> i32 {
    canvas
        .blocks
        .iter()
        .filter(|(_, v)| **v == log_id)
        .map(|((_, y, _), _)| *y)
        .max()
        .expect("tree has logs")
}

fn has_block(canvas: &Canvas, x: i32, y: i32, z: i32, id: u16) -> bool {
    canvas.blocks.get(&(x, y, z)) == Some(&id)
}

/// 方块 id 与生成表锁名：blocks_gen 重排/重生成时先红在这里，防止常数静默
/// 漂移（引注见 trees::ids）。
#[test]
fn block_ids_lock_names() {
    let name = |id: u16| BLOCKS[id as usize].name;
    assert_eq!(name(ids::DIRT), "dirt");
    assert_eq!(name(ids::OAK_LOG), "log");
    assert_eq!(name(ids::OAK_LEAVES), "leaves");
    assert_eq!(name(ids::ACACIA_LOG), "acacia_log");
    assert_eq!(name(ids::ACACIA_LEAVES), "acacia_leaves");
    assert_eq!(name(ids::BIRCH_LOG), "birch_log");
    assert_eq!(name(ids::BIRCH_LEAVES), "birch_leaves");
    assert_eq!(name(ids::DARK_OAK_LOG), "dark_oak_log");
    assert_eq!(name(ids::DARK_OAK_LEAVES), "dark_oak_leaves");
    assert_eq!(name(ids::JUNGLE_LOG), "jungle_log");
    assert_eq!(name(ids::JUNGLE_LEAVES), "jungle_leaves");
    assert_eq!(name(ids::SPRUCE_LOG), "spruce_log");
    assert_eq!(name(ids::SPRUCE_LEAVES), "spruce_leaves");
}

/// 树基 y 契约：基下一格 = dirt（dark oak 2x2 打底含之）、树基格 = 原木、
/// 任何落点不低于基下 1 格。
#[test]
fn tree_base_contract() {
    for kind in ALL_KINDS {
        let c = grow(7, kind);
        let log = kind.log_leaf_ids().0;
        assert_eq!(c.blocks[&(0, 63, 0)], ids::DIRT, "{kind:?} 基下 dirt");
        assert_eq!(c.blocks[&(0, 64, 0)], log, "{kind:?} 树基原木");
        assert!(
            c.blocks.keys().all(|(_, y, _)| *y >= 63),
            "{kind:?} 有低于基下 1 格的落点"
        );
    }
}

/// 树干高度 ∈ 原版区间（推导见模块 doc）。
/// 直杆种实测 = 原木总数（直杆 placer 每层恰 1 根）；dark oak 2x2 立柱顶与
/// acacia 主杆顶都落在 基+高−1 层（枝干不超过该层），故实测取最高原木层。
#[test]
fn trunk_height_in_vanilla_range() {
    for kind in ALL_KINDS {
        let (lo, hi) = kind.trunk_height_range();
        let log = kind.log_leaf_ids().0;
        for seed in 0..64u64 {
            let c = grow(seed, kind);
            let h = match kind {
                TreeKind::Oak | TreeKind::Birch | TreeKind::Jungle | TreeKind::Spruce => {
                    count_id(&c, log) as i32
                }
                TreeKind::Acacia | TreeKind::DarkOak => top_log_y(&c, log) - 64 + 1,
            };
            assert!(
                (lo..=hi).contains(&h),
                "{kind:?} seed{seed} 树干高 {h} 出原版区间 [{lo},{hi}]"
            );
        }
    }
}

/// 叶数 ∈ 逐层推导闭包（红刀：冠层未移植时实测恒 0）。
///
/// 推导（各冠层 placer 的每层格数闭包，层间 y 互异 → 无跨层重叠）：
/// - oak/birch/jungle 膨冠（BlobFoliagePlacer.java:44-53，r=2, fh=3）：
///   yo=0 r=1 恒切角 → 5；yo=−1 r=1 切角随机、中心=顶原木 → 4..8；
///   yo=−2/−3 r=2 → 21..25 各扣中心原木 1 → 20..24。
///   合计 [5+4+20+20, 5+8+24+24] = [49, 61]。
/// - spruce（SpruceFoliagePlacer.java:38-56，r=U(2,3), off=U(0,2),
///   fh=max(4, h−U(1,2))，h≤9 ⇒ fh≤8, 层数≤11）：下位轮生 r≥1 层
///   (2r+1)²−4−1（切角+树干柱），r=0 层 0；上位（yo≥0）不扣柱。
///   最小 cr0=0、lr=2、fh=4、off=0：1+4+0+4+20=29；
///   最大 cr0=1、lr=3、fh=8、off=2：11+160=171。
/// - acacia（AcaciaFoliagePlacer.java:33-54，r=2, fh=0）：主冠
///   (−1,r3)=45−4角−1顶柱=44、(0,r1)=9、(0,r2) 十字=13 → 66；
///   枝冠 (−1,r2)=21−1自身顶柱=20、(0,r1)∪(0,r1)=9 → ≤29。
///   重叠/枝原木让位 ⇒ 实测 [43, 95]，闭包取 [40, 100]。
/// - dark oak（DarkOakFoliagePlacer.java:33-52，r=0）：主冠
///   (−1,r2)=36−4柱=32、(0,r3)=64−9极角−4柱=51、(+1,r2)=36−12
///   （minDx+minDz>2 的 3 组合×4）=24、50% 顶盖 (+2,r0)=0..4
///   → 107..111，扣 ≤12 枝顶原木让位 ⇒ ≥95；枝冠 ≤12×29=348
///   ⇒ 上界 111+348=459，闭包取 [90, 465]。
#[test]
fn leaf_count_in_derived_band() {
    for kind in ALL_KINDS {
        let leaf = kind.log_leaf_ids().1;
        let (lo, hi) = match kind {
            TreeKind::Oak | TreeKind::Birch | TreeKind::Jungle => (49, 61),
            TreeKind::Spruce => (29, 171),
            TreeKind::Acacia => (40, 100),
            TreeKind::DarkOak => (90, 465),
        };
        for seed in 0..64u64 {
            let n = count_id(&grow(seed, kind), leaf);
            assert!(
                (lo..=hi).contains(&n),
                "{kind:?} seed{seed} 叶数 {n} 出推导闭包 [{lo},{hi}]"
            );
        }
    }
}

/// 某层原木位置集合（用于定位倾斜杆/2x2 柱顶）。
fn logs_at_y(canvas: &Canvas, y: i32, log_id: u16) -> Vec<(i32, i32)> {
    canvas
        .blocks
        .iter()
        .filter(|((_, yy, _), v)| *yy == y && **v == log_id)
        .map(|((x, _, z), _)| (*x, *z))
        .collect()
}

/// 冠层结构不变量（红刀全败）。
#[test]
fn canopy_shape_invariants() {
    for kind in ALL_KINDS {
        let c = grow(11, kind);
        let (log, leaf) = kind.log_leaf_ids();
        let top = top_log_y(&c, log);
        match kind {
            // 直杆膨冠：冠顶中心=叶（附着 yo=0 行中心），四角恒切
            // （BlobFoliagePlacer.java:56-61 y==0 恒切）。直杆无倾斜，冠心
            // 即树基列。
            TreeKind::Oak | TreeKind::Birch | TreeKind::Jungle => {
                assert!(has_block(&c, 0, top + 1, 0, leaf), "{kind:?} 冠顶中心叶");
                assert!(
                    !has_block(&c, 1, top + 1, 1, leaf),
                    "{kind:?} 冠顶角不应有叶"
                );
            }
            // spruce：附着点之下必有贴干裙摆轮生（r≥1 层的 (±1,0) 位）。
            // 直杆无倾斜，冠心即树基列。
            TreeKind::Spruce => {
                assert!(
                    (64..top).any(|y| has_block(&c, 1, y, 0, leaf)),
                    "spruce 干侧裙摆缺失"
                );
            }
            // acacia：平顶层（最高原木+1，附着 yo=0 行）绕每个顶层原木必有
            // ±2 十字臂（r=2 行 (±2,0)/(0,±2) 位不切，AcaciaFoliagePlacer
            // .java:48-50 y==0 只切 |dx|>1 且 |dz|>1 的角）。
            TreeKind::Acacia => {
                let flat = top + 1;
                let arms = logs_at_y(&c, top, log).iter().any(|(x, z)| {
                    [(-2, 0), (2, 0), (0, -2), (0, 2)]
                        .iter()
                        .any(|(dx, dz)| has_block(&c, x + dx, flat, z + dz, leaf))
                });
                assert!(arms, "acacia 平顶十字臂缺失");
            }
            // dark oak：主冠三层齐放（DarkOakFoliagePlacer.java:38-42）——
            // 柱顶 2x2 之上（y=+1 行，r=2）中心位不切恒有叶；柱顶层
            // （y=-1 行）6x6 除 2x2 立柱外全叶。
            TreeKind::DarkOak => {
                let crown = logs_at_y(&c, top, log)
                    .first()
                    .copied()
                    .expect("dark oak 柱顶 2x2");
                assert!(
                    has_block(&c, crown.0, top + 1, crown.1, leaf),
                    "dark oak 冠上层缺失"
                );
                assert!(
                    c.blocks
                        .iter()
                        .any(|((_, y, _), v)| *y == top - 1 && *v == leaf),
                    "dark oak 冠下层缺失"
                );
            }
        }
    }
}

/// 同 seed 字节级确定：两次独立生成逐字节相等。
#[test]
fn same_seed_is_byte_identical() {
    for kind in ALL_KINDS {
        for seed in 0..24u64 {
            assert_eq!(grow(seed, kind), grow(seed, kind), "{kind:?} seed{seed}");
        }
    }
}

/// 不同 seed 应产生不同树（8 苗全同概率≈0：树干高度分布+冠层随机收缩）。
#[test]
fn different_seeds_diverge() {
    for kind in ALL_KINDS {
        let first = grow(0, kind);
        assert!(
            (1..8).any(|s| grow(s, kind) != first),
            "{kind:?} 8 个 seed 全同——随机面失效"
        );
    }
}

/// TreesRng 契约面：bound=1 恒 0 且必消耗；小界覆盖均匀。
#[test]
fn trees_rng_contract() {
    let mut r = TreesRng::new(42);
    for _ in 0..100 {
        assert_eq!(r.next_int(1), 0);
    }
    let mut r = TreesRng::new(7);
    let mut seen = [false; 3];
    for _ in 0..300 {
        seen[r.next_int(3) as usize] = true;
    }
    assert!(seen.iter().all(|s| *s), "next_int(3) 300 抽未覆盖全值域");
    let mut r = TreesRng::new(9);
    let mut both = (false, false);
    for _ in 0..200 {
        if r.next_bool() {
            both.0 = true;
        } else {
            both.1 = true;
        }
    }
    assert!(both.0 && both.1, "next_bool 200 抽未覆盖两侧");
}
