//! 生物群系染色（mcv_core::tint）测试：查表公式、注册表、ChunkTint
//! 场解色 / 盒均值手推 / 跨区块连续性。

use mcv_core::tint;

fn colormap(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../assets/minecraft/textures/colormap/{name}.png"
    ));
    tint::decode_colormap(&std::fs::read(&path).expect("colormap 素材应在仓库内"))
        .expect("colormap 应为 256x256")
}

/// ColorMapColorUtil.get 公式对照（已用仓内 PNG 离线实测的基准值）：
/// plains（temp 0.8 / rain 0.4）草 #91BD59、叶 #77AB2F；
/// GrassColor.getDefaultColor() = get(0.5, 1.0) = #7CBD6B；
/// 雪原（temp 0 / rain 0.5）草 #80B497。
#[test]
fn colormap_formula_matches_vanilla_values() {
    let grass = colormap("grass");
    let foliage = colormap("foliage");
    // plains 气候 = biomes::climate(Plains)（temp 0.8 / downfall 0.4）。
    let g = tint::colormap_get(&grass, 0.8, 0.4).unwrap();
    assert_eq!(&g[..3], &[0x91, 0xBD, 0x59], "plains 草色应为 #91BD59");
    let f = tint::colormap_get(&foliage, 0.8, 0.4).unwrap();
    assert_eq!(&f[..3], &[0x77, 0xAB, 0x2F], "plains 叶色应为 #77AB2F");
    let d = tint::colormap_get(&grass, 0.5, 1.0).unwrap();
    assert_eq!(&d[..3], &[0x7C, 0xBD, 0x6B], "getDefaultColor 应为 #7CBD6B");
    let s = tint::colormap_get(&grass, 0.0, 0.5).unwrap();
    assert_eq!(&s[..3], &[0x80, 0xB4, 0x97], "雪原草色应为 #80B497");
    // 缺表返回 None（调用方禁色，不伪造）。
    assert!(tint::colormap_get(&[], 0.5, 0.5).is_none());
}

/// 26.1 BlockColors 注册表抽查（BlockColors.java:20-45）：
/// 草族 → GRASS、橡木叶族 → FOLIAGE、云杉/白桦 → 常量、
/// 樱桃/苍白橡叶/干草不注册（贴图自带颜色）。
#[test]
fn tint_kinds_match_blockcolors_registry() {
    let names = mcv_core::atlas::tile_file_names();
    let layer =
        |name: &str| u16::try_from(names.iter().position(|n| *n == name).expect(name)).unwrap();
    assert_eq!(tint::tint_kind(layer("grass_block_top")), tint::TINT_GRASS);
    assert_eq!(tint::tint_kind(layer("short_grass")), tint::TINT_GRASS);
    assert_eq!(tint::tint_kind(layer("fern")), tint::TINT_GRASS);
    assert_eq!(
        tint::tint_kind(layer("tall_grass_bottom")),
        tint::TINT_GRASS
    );
    assert_eq!(
        tint::tint_kind(layer("large_fern_bottom")),
        tint::TINT_GRASS
    );
    assert_eq!(tint::tint_kind(layer("sugar_cane")), tint::TINT_GRASS);
    assert_eq!(tint::tint_kind(layer("oak_leaves")), tint::TINT_FOLIAGE);
    assert_eq!(tint::tint_kind(layer("jungle_leaves")), tint::TINT_FOLIAGE);
    assert_eq!(
        tint::tint_kind(layer("mangrove_leaves")),
        tint::TINT_FOLIAGE
    );
    assert_eq!(tint::tint_kind(layer("vine")), tint::TINT_FOLIAGE);
    assert_eq!(tint::tint_kind(layer("spruce_leaves")), tint::TINT_SPRUCE);
    assert_eq!(tint::tint_kind(layer("birch_leaves")), tint::TINT_BIRCH);
    // 不注册：自带颜色/26.1 未注册族。
    assert_eq!(tint::tint_kind(layer("cherry_leaves")), tint::TINT_NONE);
    assert_eq!(tint::tint_kind(layer("pale_oak_leaves")), tint::TINT_NONE);
    assert_eq!(tint::tint_kind(layer("short_dry_grass")), tint::TINT_NONE);
    assert_eq!(tint::tint_kind(layer("stone")), tint::TINT_NONE);
    assert_eq!(tint::tint_kind(layer("dirt")), tint::TINT_NONE);
    // 特殊层（裂纹/越界）一律不染。
    assert_eq!(
        tint::tint_kind(u16::try_from(mcv_core::atlas::CRACK_BASE).unwrap() + 3),
        tint::TINT_NONE
    );
    assert_eq!(tint::tint_kind(u16::MAX), tint::TINT_NONE);
}

/// ChunkTint：plains 全域时草/叶色来自真实 colormap（#91BD59/#77AB2F），
/// 混合窗内同质 → 恒等；box 均值语义（calculateBlockTint 实物：先解色
/// 后整数平均）。
#[test]
fn chunk_tint_resolves_climate_to_colormap() {
    let grass = colormap("grass");
    let foliage = colormap("foliage");
    // 5×5 全域 plains（temp 0.8 / rain 0.4）。
    let cells = [[0.8, 0.4]; 25];
    let ct = tint::ChunkTint::from_climate(-2, -2, &cells, &grass, &foliage).unwrap();
    assert_eq!(
        &ct.color_at(tint::TINT_GRASS, 0, 0)[..],
        &[0x91, 0xBD, 0x59],
        "plains 草"
    );
    assert_eq!(
        &ct.color_at(tint::TINT_FOLIAGE, 0, 0)[..],
        &[0x77, 0xAB, 0x2F],
        "plains 叶"
    );
    // 非 colormap 族（常量/不染色）不查网格。
    assert_eq!(ct.color_at(tint::TINT_NONE, 0, 0), tint::TINT_WHITE);
    // 缺 colormap → None（调用方禁用染色，不伪造颜色）。
    assert!(tint::ChunkTint::from_climate(-2, -2, &cells, &[], &foliage).is_none());
}

/// 盒均值手推对拍（Blender/ClientLevel 语义）：网格左半 temp 0.8
/// （plains 草 #91BD59）、右半 temp 0（雪原草 #80B497），沿 x 边界处
/// 取 (bx=右半第 3 列) 使混合窗内 3 列左 + 2 列右（BLEND_RADIUS=2 →
/// 每行 x∈[bx-2,bx+2]，前 3 列 plains 后 2 列 snow）→
/// 期望逐通道 (3*plains + 2*snow)/5 整数除。
#[test]
fn chunk_tint_box_average_matches_hand_computation() {
    let grass = colormap("grass");
    let foliage = colormap("foliage");
    let w = 9usize;
    let mut cells = [[0.0; 2]; 81];
    for (i, c) in cells.iter_mut().enumerate() {
        let x = (i % w) as i32;
        // x<5 → plains(0.8,0.4)；x>=5 → snow(0.0,0.5)。
        *c = if x < 5 { [0.8, 0.4] } else { [0.0, 0.5] };
    }
    let ct = tint::ChunkTint::from_climate(0, 0, &cells, &grass, &foliage).unwrap();
    let plains = [0x91u32, 0xBD, 0x59];
    let snow = [0x80u32, 0xB4, 0x97];
    // bx=5：窗 x∈[3,7] → x=3,4 plains；x=5,6,7 snow（列同侧混合，
    // 各行一致，共 5 行）。
    let expect = [
        ((3 * plains[0] + 2 * snow[0]) / 5) as u8,
        ((3 * plains[1] + 2 * snow[1]) / 5) as u8,
        ((3 * plains[2] + 2 * snow[2]) / 5) as u8,
    ];
    assert_eq!(
        ct.color_at(tint::TINT_GRASS, 5, 4),
        expect,
        "跨群系边界盒均值"
    );
    // 深处不受另一群系影响（窗全 plains / 全 snow）。
    assert_eq!(&ct.color_at(tint::TINT_GRASS, 0, 4)[..], &plains[..]);
    assert_eq!(&ct.color_at(tint::TINT_GRASS, 8, 4)[..], &snow[..]);
}

/// 跨区块边界连续：两块用同一气候场（仅原点不同），边界世界列两侧取色
/// 差 = 0（同一批采样列、同式整数平均 → 逐位相等，无硬切）。
#[test]
fn chunk_tint_continuous_across_block_boundary() {
    let grass = colormap("grass");
    let foliage = colormap("foliage");
    // 世界场：x<16 → plains，x>=16 → snow（边界落世界 x=16）。
    let climate_at = |wx: i32| -> [f64; 2] { if wx < 16 { [0.8, 0.4] } else { [0.0, 0.5] } };
    // 左侧块（原点 0，覆盖 [0,16)）：tint 窗 [-2,18] 共 w=21。
    let w = 21usize;
    let left: Vec<[f64; 2]> = (0..w * w)
        .map(|i| climate_at(-2 + (i % w) as i32))
        .collect();
    let left = tint::ChunkTint::from_climate(-2, -2, &left, &grass, &foliage).unwrap();
    // 右侧块（原点 16，覆盖 [16,32)）：窗 [14,34]。
    let right: Vec<[f64; 2]> = (0..w * w)
        .map(|i| climate_at(14 + (i % w) as i32))
        .collect();
    let right = tint::ChunkTint::from_climate(14, -2, &right, &grass, &foliage).unwrap();
    // 边界世界列 x=16（左块右缘 bx=16，右块左缘 bx=16）同色。
    assert_eq!(
        left.color_at(tint::TINT_GRASS, 16, 10),
        right.color_at(tint::TINT_GRASS, 16, 10),
        "世界 x=16 两侧取色必须逐位相等"
    );
    // 边界两侧各 1 列（x=15/17）也应连续（窗重叠大半，差值 < 阈值）。
    let a = left.color_at(tint::TINT_GRASS, 15, 10);
    let b = left.color_at(tint::TINT_GRASS, 16, 10);
    for ch in 0..3 {
        assert!(
            (i16::from(a[ch]) - i16::from(b[ch])).abs() <= 60,
            "相邻列跳变过陡 ch{ch}: {}→{}",
            a[ch],
            b[ch]
        );
    }
}
