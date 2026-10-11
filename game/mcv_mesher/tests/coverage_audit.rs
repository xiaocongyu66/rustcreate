//! 全方块覆盖矩阵审计（#147 审计波）——基线回归哨兵，只读三表，零生产改动。
//!
//! 交叉数据源（矩阵口径见 docs/block-coverage.md）：
//! 1. [`mcv_core::BLOCKS`]——blocks_gen.inc.rs 千块表（1171 id：贴图槽/ solid/
//!    opaque/形状派生字段）；
//! 2. `tiles_manifest.json` 层号语义——经 [`mcv_core::atlas`] 常数：层
//!    `0..826` 真实贴图、层 827 = missing 哨兵（运行时 missingno 品红）；
//! 3. [`VANILLA_CROSS_VISUAL`]——26.1 官方 blockstates 默认态「十字/作物/
//!    海草类非整盒视觉」名字清单（审计时点快照，见同目录
//!    `vanilla_cross_models.inc`；blockstates JSON 本身不入库）。
//!
//! 锁定的基线（实测于基座 main=abf83707，2026-10-12）：
//! - 贴图零缺失方块数 ≥ 1165/1171（哨兵唯一用户 = 六个无模型方块）；
//! - 形状分档分布 = [`SHAPE_BASELINE`]（非 Cube 档合计 311）；
//! - 「原版十字视觉但引擎整盒占位」债集 ⊆ [`CROSS_DEBT`]（58 名，只许收缩）；
//! - 「原版十字视觉 ∧ 引擎整盒碰撞（solid）且占位」债集 ⊆
//!   [`SOLID_CROSS_DEBT`]（4 名紫水晶簇/芽，#138 solid 强制债的量化）。
//!
//! ## 基线 bump 协议（后续波必读）
//! 改善波（如逐族补 cross/作物模板）落地后：改了 `shape.rs` 名单 → 按新实测
//! 更新 [`SHAPE_BASELINE`]（各档应单调走向原版应然值）；债集收缩后本测试的
//! ⊆ 断言自动通过，鼓励把收缩后的名单回写进 [`CROSS_DEBT`]/
//! [`SOLID_CROSS_DEBT`]（防回潮）。任何【恶化】（新增整盒占位/贴图缺失/
//! 形状误分类）都会被 ⊆ 与 == 断言当场拦截。
//!
//! 原版对照取「抽查法」（同 docs/block-coverage.md §抽查）：玻璃族全盒视觉
//! = 原版应然（cube_all），逐 1000 方块对 blockstates 全量对照不在本波。

use mcv_core::{BlockDef, Shape, atlas};

include!("vanilla_cross_models.inc");

/// missing 哨兵层号（tiles_manifest.json 第 828 项 = `missing_no_texture`）。
const SENTINEL: u16 = atlas::SENTINEL_LAYER as u16;
/// manifest 区上界（真实层 0..826 + 哨兵 827；裂纹层 828..837 由渲染自建，
/// 生成表不得引用）。
const MANIFEST_MAX: u16 = atlas::MANIFEST_LAYERS as u16;

/// 按注册名查方块（无别名时 panic，测试只依赖生成表）。
fn by_name(name: &str) -> &'static BlockDef {
    mcv_core::BLOCKS
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("BLOCKS 注册表缺少方块: {name}"))
}

/// 旧 14 方块的官方名别名（gen-blocks.py `LEGACY` 同源）：
/// 千块表 id 0..13 沿用旧名（flower_red/poppy 等），对照官方清单时换算。
fn vanilla_alias(van: &str) -> &str {
    match van {
        "poppy" => "flower_red",
        "dandelion" => "flower_yellow",
        other => other,
    }
}

/// [`vanilla_alias`] 的反方向：引擎注册名 → 官方注册名。
fn engine_alias(ours: &str) -> &str {
    match ours {
        "flower_red" => "poppy",
        "flower_yellow" => "dandelion",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// 轴 1：贴图瓦片覆盖（BLOCKS[].tiles × atlas 哨兵语义）
// ---------------------------------------------------------------------------

/// 贴图零缺失基线：1171 方块中 ≥1165 块六面全部指向真实贴图层（0..826）。
/// 哨兵层的唯一合法用户是「无模型方块」六个（见
/// [`all_sentinel_tiles_are_invisible_class`]），部分缺失（六面混真实/哨兵）
/// 在生成器里应恒为 0——非零即模型解析回归。
#[test]
fn texture_zero_missing_baseline() {
    let mut zero_missing = 0usize;
    let mut partial = Vec::new();
    for b in mcv_core::BLOCKS.iter() {
        let missing = b.tiles.iter().filter(|t| **t >= SENTINEL).count();
        if missing == 0 {
            zero_missing += 1;
        } else if missing < 6 {
            partial.push(b.name);
        }
        // 层号硬不变量：全部落在 manifest 区（< 828）。裂纹层 828..837 是
        // 渲染自建破坏层，生成表引用即越界（运行时越界=品红/panic 面）。
        for t in b.tiles.iter() {
            assert!(
                *t < MANIFEST_MAX,
                "方块 {} 引用层 {} ≥ manifest 上界 {MANIFEST_MAX}",
                b.name,
                t
            );
        }
    }
    assert!(
        zero_missing >= 1165,
        "贴图零缺失方块数 {zero_missing} < 基线 1165（贴图层回退/manifest 漂移？）"
    );
    assert!(
        partial.is_empty(),
        "六面贴图部分缺失（模型解析回归）: {partial:?}"
    );
}

/// 哨兵层用户锁定：六面全哨兵 = 原版无可见模型的方块（light/空气族/barrier/
/// structure_void；网格器按 INVISIBLE_GEOM 出零几何，见 blocks_gen.inc.rs 头注
/// 与 mesher golden geom 锁）。新增全哨兵方块必须是有意登记的不可见类。
#[test]
fn all_sentinel_tiles_are_invisible_class() {
    let mut names: Vec<&str> = mcv_core::BLOCKS
        .iter()
        .filter(|b| b.tiles.iter().all(|t| *t >= SENTINEL))
        .map(|b| b.name)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "air",
            "barrier",
            "cave_air",
            "light",
            "structure_void",
            "void_air"
        ],
        "六面哨兵方块名单漂移——若为新增无模型方块请同步更新本名单与 docs/block-coverage.md"
    );
}

// ---------------------------------------------------------------------------
// 轴 2：形状档分布（BLOCKS[].shape × 26.1 blockstates 视觉，抽查对照）
// ---------------------------------------------------------------------------

/// 形状分档基线（实测 main=abf83707；`Shape::from_u8(BLOCKS[].shape)` 全表
/// 计数）。改善波改动 `shape.rs::shape_of_name` 名单后，按新实测值更新本表
/// （走向原版应然：非 Cube 档应增、Cube 档应减；任意一档回退即红）。
const SHAPE_BASELINE: [(Shape, usize); 16] = [
    (Shape::Cube, 860), // 含 447 整盒占位债（kind=1），见 CROSS_DEBT
    (Shape::Cross, 27), // CROSS_PLANTS 18 + *sapling 9
    (Shape::Torch, 4),
    (Shape::Fence, 13),
    (Shape::Slab, 62),
    (Shape::Stairs, 58),
    (Shape::Carpet, 18),
    (Shape::Trapdoor, 21),
    (Shape::Pane, 17),
    (Shape::Wall, 26),
    (Shape::FenceGate, 12),
    (Shape::Door, 21),
    (Shape::Button, 14),
    (Shape::PressurePlate, 16),
    (Shape::Ladder, 1),
    (Shape::SnowLayer, 1),
];

#[test]
fn shape_template_distribution_baseline() {
    let mut total = 0usize;
    let mut non_full = 0usize;
    for (shape, want) in SHAPE_BASELINE {
        let got = mcv_core::BLOCKS
            .iter()
            .filter(|b| Shape::from_u8(b.shape) == shape)
            .count();
        assert_eq!(
            got, want,
            "形状档 {shape:?} 计数 {got} ≠ 基线 {want}（shape.rs 名单变更后按新实测 bump 本表）"
        );
        total += got;
        if shape != Shape::Cube {
            non_full += got;
        }
    }
    assert_eq!(total, mcv_core::BLOCKS.len(), "形状分档计数必须全覆盖");
    // 任务书「形状非 full 档方块数=N」：N = 311 = 非 Cube 档合计。
    assert_eq!(
        non_full, 311,
        "非 full 档合计漂移（= 各档基线和，联动检查）"
    );
}

/// 引擎 Cross 档必须 ⊆ 原版十字视觉清单（防误分类扩散）：任何被
/// `shape.rs::CROSS_PLANTS`/sapling 规则标成 Cross 的名字，都应能在
/// [`VANILLA_CROSS_VISUAL`] 里对上官方 cross/crop/海草视觉（经旧 14 别名）。
/// 已知超集近似（原版是 flowerbed 非 cross）：pink_petals/wildflowers，
/// 登记 docs/block-coverage.md 债表，不在此报红。
#[test]
fn engine_cross_shapes_are_vanilla_cross_superset() {
    let mut cross = 0usize;
    for b in mcv_core::BLOCKS.iter() {
        if Shape::from_u8(b.shape) != Shape::Cross {
            continue;
        }
        cross += 1;
        let van = engine_alias(b.name);
        if van == "pink_petals" || van == "wildflowers" {
            continue; // 已知超集近似（flowerbed 视觉），债表登记
        }
        assert!(
            VANILLA_CROSS_VISUAL.iter().any(|(v, _)| *v == van),
            "引擎 Cross 方块 {} 无原版十字视觉对照（误分类？）",
            b.name
        );
    }
    assert_eq!(cross, 27, "Cross 档基线 27（18 花草 + 9 苗）");
}

// ---------------------------------------------------------------------------
// 轴 3：solid×cross 错位债（#138 遗留「gen solid 强制」的量化）
// ---------------------------------------------------------------------------

/// 已知债集 A——「原版十字视觉 ∧ 引擎整盒占位」58 名（审计时点冻结）。
/// 单调收缩白名单：只许后续波从这里消掉（改 shape.rs 出真形状），不许新增。
const CROSS_DEBT: [&str; 58] = [
    "amethyst_cluster",
    "beetroots",
    "brain_coral",
    "brown_mushroom",
    "bubble_coral",
    "bush",
    "cactus_flower",
    "carrots",
    "cave_vines",
    "cave_vines_plant",
    "closed_eyeblossom",
    "cobweb",
    "crimson_fungus",
    "crimson_roots",
    "dead_brain_coral",
    "dead_bubble_coral",
    "dead_bush",
    "dead_fire_coral",
    "dead_horn_coral",
    "dead_tube_coral",
    "fire_coral",
    "firefly_bush",
    "golden_dandelion",
    "hanging_roots",
    "horn_coral",
    "kelp",
    "kelp_plant",
    "large_amethyst_bud",
    "lilac",
    "medium_amethyst_bud",
    "nether_sprouts",
    "nether_wart",
    "open_eyeblossom",
    "orange_tulip",
    "pale_hanging_moss",
    "peony",
    "pitcher_crop",
    "pink_tulip",
    "potatoes",
    "red_mushroom",
    "red_tulip",
    "seagrass",
    "short_dry_grass",
    "small_amethyst_bud",
    "sugar_cane",
    "sunflower",
    "sweet_berry_bush",
    "tall_dry_grass",
    "tall_seagrass",
    "tube_coral",
    "twisting_vines",
    "twisting_vines_plant",
    "warped_fungus",
    "warped_roots",
    "weeping_vines",
    "weeping_vines_plant",
    "wheat",
    "white_tulip",
];

/// 已知债集 B——「原版十字视觉 ∧ 引擎 solid=true（整盒碰撞）∧ 整盒占位」
/// 4 名：amethyst_cluster + 三档 amethyst_bud。原版十字视觉却带碰撞的方块
/// （AmethystClusterBlock 自带 AABB 碰撞，Blocks.java 5078-5084 无
/// noCollision）——引擎现按整盒碰撞 + 整盒渲染占位，是 #138「gen solid
/// 强制」遗留债的量化名单（白名单锁死：后续波做紫水晶族时收敛）。
const SOLID_CROSS_DEBT: [&str; 4] = [
    "amethyst_cluster",
    "large_amethyst_bud",
    "medium_amethyst_bud",
    "small_amethyst_bud",
];

#[test]
fn vanilla_cross_placeholder_debt_whitelist() {
    let debt: Vec<&str> = VANILLA_CROSS_VISUAL
        .iter()
        .map(|(v, _)| *v)
        .filter(|v| Shape::from_u8(by_name(vanilla_alias(v)).shape) == Shape::Cube)
        .collect();
    for d in &debt {
        assert!(
            CROSS_DEBT.contains(d),
            "方块 {d} 原版十字视觉但引擎整盒占位，且不在已知债集——先查 shape.rs 回归，新债须同步扩债单与 docs/block-coverage.md"
        );
    }
    assert!(
        debt.len() <= CROSS_DEBT.len(),
        "整盒占位十字债 {} 只许单调收缩（基线 58）",
        debt.len()
    );
    // 债集 B ⊆ 债集 A（有碰撞的十字占位必属十字占位全集）。
    for d in SOLID_CROSS_DEBT {
        assert!(
            debt.contains(&d),
            "债集 B 成员 {d} 应仍在债集 A 中（若已收敛请同步删 SOLID_CROSS_DEBT）"
        );
    }
}

#[test]
fn solid_fullbox_vanilla_cross_debt_whitelist() {
    let debt: Vec<&str> = VANILLA_CROSS_VISUAL
        .iter()
        .map(|(v, _)| *v)
        .filter(|v| {
            let b = by_name(vanilla_alias(v));
            b.solid && Shape::from_u8(b.shape) == Shape::Cube
        })
        .collect();
    for d in &debt {
        assert!(
            SOLID_CROSS_DEBT.contains(d),
            "方块 {d} 原版十字视觉但引擎整盒碰撞（solid 强制债新成员）——须同步 SOLID_CROSS_DEBT 与 KNOWN-DIVERGENCE"
        );
    }
    assert!(
        debt.len() <= SOLID_CROSS_DEBT.len(),
        "solid×cross 错位债只许单调收缩（基线 4）",
    );
    // 基线非空自检：债集 B 必须仍然非空（否则说明 4 名已被收敛，应删白名单）。
    assert!(
        !debt.is_empty(),
        "solid×cross 债已清零——请更新白名单与本注释"
    );
}

// ---------------------------------------------------------------------------
// 轴 4：玻璃族抽查（原版 cube_all 全盒视觉应然 = Cube + 有碰撞）
// ---------------------------------------------------------------------------

/// 玻璃族抽查（任务书「glass 有碰撞✓」）：名字含 glass 的 35 块——
/// 全盒玻璃（glass/tinted_glass/16 色 stained_glass）应然=Cube 且 solid=true
/// （原版 cube_all 视觉 + 有碰撞）；玻璃板（glass_pane/16 色 stained_pane）
/// 应然=Pane 档且 solid=true。逐 1000 方块对 blockstates 全量对照不在本波。
#[test]
fn glass_family_spot_check() {
    let mut seen = 0usize;
    for b in mcv_core::BLOCKS.iter() {
        if !b.name.contains("glass") {
            continue;
        }
        seen += 1;
        let is_pane = b.name.contains("pane");
        let want = if is_pane { Shape::Pane } else { Shape::Cube };
        assert_eq!(
            Shape::from_u8(b.shape),
            want,
            "玻璃族 {} 形状档应 {:?}",
            b.name,
            want
        );
        assert!(b.solid, "玻璃族 {} 必须有碰撞（solid=true）", b.name);
        assert!(
            !b.opaque,
            "玻璃族 {} 应透光（opaque=false，noOcclusion/透明像素）",
            b.name
        );
    }
    assert_eq!(seen, 35, "玻璃族抽查应覆盖 35 块（2 全盒 + 33 板/染色）");
}

// ---------------------------------------------------------------------------
// 汇总打印（test 输出即矩阵摘要，CI 日志可查）
// ---------------------------------------------------------------------------

#[test]
fn coverage_matrix_summary() {
    let tex_ok = mcv_core::BLOCKS
        .iter()
        .filter(|b| b.tiles.iter().all(|t| *t < SENTINEL))
        .count();
    let non_full = mcv_core::BLOCKS
        .iter()
        .filter(|b| Shape::from_u8(b.shape) != Shape::Cube)
        .count();
    let cross_debt = VANILLA_CROSS_VISUAL
        .iter()
        .filter(|(v, _)| Shape::from_u8(by_name(vanilla_alias(v)).shape) == Shape::Cube)
        .count();
    eprintln!(
        "COVERAGE-AUDIT 基线摘要: 方块 {} | 贴图零缺失 {} | 非 full 档 {} | 十字占位债 {cross_debt}/58 | solid×cross 债 4/4",
        mcv_core::BLOCKS.len(),
        tex_ok,
        non_full
    );
}
