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

/// 草方块侧 overlay 逐面 tint 接线（26.1 grass_block.json 实值表驱动，
/// ci/gen-blocks.py LEGACY_OVERLAY_MODELS 提取）：grass 行 4 侧 overlay =
/// grass_block_side_overlay 层（GRASS 族，tintindex 0 实物），顶/底恒 0；
/// snow_grass 走 grass_block_snow.json（无 overlay 元素）→ 恒 0；base 侧面
/// 贴图（grass_block_side）本身不注册——草裙绿色只来自 overlay 插层。
#[test]
fn grass_side_overlay_wiring_matches_model_json() {
    let names = mcv_core::atlas::tile_file_names();
    let layer =
        |name: &str| u16::try_from(names.iter().position(|n| *n == name).expect(name)).unwrap();
    let overlay = layer("grass_block_side_overlay");
    let side = layer("grass_block_side");
    let mut grass = None;
    let mut snow = None;
    for b in mcv_core::BLOCKS.iter() {
        match b.name {
            "grass" => grass = Some(b),
            "snow_grass" => snow = Some(b),
            _ => {}
        }
    }
    let grass = grass.expect("grass 行");
    let snow = snow.expect("snow_grass 行");
    // FACE_ORDER [+X,-X,+Y,-Y,+Z,-Z]：4 侧插层、顶/底无。
    assert_eq!(
        grass.overlays,
        [overlay, overlay, 0, 0, overlay, overlay],
        "grass 侧面 overlay 应为 grass_block_side_overlay 层"
    );
    assert_eq!(snow.overlays, [0; 6], "覆雪面无草裙（无插层）");
    // 全表唯一插层行 = grass（其它模型即便有多元素 tintindex（叶/cross/
    // 炼药锅）也走 tile 层 kind 或 kind=1 路径，不进插层列）。
    assert!(
        mcv_core::BLOCKS
            .iter()
            .all(|b| b.name == "grass" || b.overlays == [0; 6]),
        "插层列只允许 grass 行非零"
    );
    // tint 消费口径：overlay 层 = GRASS 族，base 侧面层不注册（原版
    // tintindex 只在 overlay 元素上）。
    assert_eq!(tint::tint_kind(overlay), tint::TINT_GRASS);
    assert_eq!(tint::tint_kind(side), tint::TINT_NONE);
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

/// 盒均值手推对拍（ClientLevel.calculateBlockTint 实物语义：先解色后
/// 整数平均）：网格左半 x<5 temp 0.8（plains 草 #91BD59）、右半 temp 0
/// （雪原草 #80B497），取 bx=5 使混合窗（BLEND_RADIUS=2 → 每行
/// x∈[3,7]）内 2 列 plains（x=3,4）+ 3 列 snow（x=5,6,7）→
/// 期望逐通道 (2*plains + 3*snow)/5 整数除（截断，同 ARGB 求和/count）。
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
        ((2 * plains[0] + 3 * snow[0]) / 5) as u8,
        ((2 * plains[1] + 3 * snow[1]) / 5) as u8,
        ((2 * plains[2] + 3 * snow[2]) / 5) as u8,
    ];
    assert_eq!(
        ct.color_at(tint::TINT_GRASS, 5, 4),
        expect,
        "跨群系边界盒均值"
    );
    // 深处不受另一群系影响（窗全 plains / 全 snow；plains/snow 为算式
    // 声明的 u32 域，比对回 u8 语义域）。
    assert_eq!(ct.color_at(tint::TINT_GRASS, 0, 4), plains.map(|v| v as u8));
    assert_eq!(ct.color_at(tint::TINT_GRASS, 8, 4), snow.map(|v| v as u8));
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
