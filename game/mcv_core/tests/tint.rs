//! 生物群系染色表（mcv_core::tint）测试：查表公式、注册表、LUT 结构。

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
    let g = tint::colormap_get(&grass, tint::WORLD_TEMP, tint::WORLD_DOWNFALL).unwrap();
    assert_eq!(&g[..3], &[0x91, 0xBD, 0x59], "plains 草色应为 #91BD59");
    let f = tint::colormap_get(&foliage, tint::WORLD_TEMP, tint::WORLD_DOWNFALL).unwrap();
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

/// LUT 结构：每层 16B = vec4<u32>(kind, r, g, b)；常量叶色烤入 LUT，
/// 草/叶 colormap 族 kind 正确；裂纹层与哨兵层为 0。
#[test]
fn tint_lut_layout() {
    let lut = tint::tint_lut_bytes();
    let layers = mcv_core::atlas::LAYERS;
    assert_eq!(lut.len(), layers * 16);
    let entry = |l: usize| -> [u32; 4] {
        (0..4)
            .map(|c| {
                u32::from_le_bytes(lut[l * 16 + c * 4..l * 16 + c * 4 + 4].try_into().unwrap())
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    };
    let names = mcv_core::atlas::tile_file_names();
    let grass_l = names.iter().position(|n| *n == "grass_block_top").unwrap();
    assert_eq!(entry(grass_l)[0], tint::TINT_GRASS);
    let spruce_l = names.iter().position(|n| *n == "spruce_leaves").unwrap();
    let e = entry(spruce_l);
    assert_eq!(
        (e[0], e[1], e[2], e[3]),
        (tint::TINT_SPRUCE, 0x61, 0x99, 0x61)
    );
    let birch_l = names.iter().position(|n| *n == "birch_leaves").unwrap();
    let e = entry(birch_l);
    assert_eq!(
        (e[0], e[1], e[2], e[3]),
        (tint::TINT_BIRCH, 0x80, 0xA7, 0x55)
    );
    // 裂纹层（CRACK_BASE..）恒 0。
    let crack0 = entry(mcv_core::atlas::CRACK_BASE);
    assert_eq!(crack0, [0; 4]);
}

/// plains 基线封装：世界草/叶色来自真实 colormap 查表（0..1 f32）。
#[test]
fn world_colors_are_plains_baseline() {
    let (g, f) = tint::world_grass_foliage_color(&colormap("grass"), &colormap("foliage"))
        .expect("仓内 colormap 应产出基线色");
    assert_eq!(
        g,
        [
            0x91 as f32 / 255.0,
            0xBD as f32 / 255.0,
            0x59 as f32 / 255.0
        ]
    );
    assert_eq!(
        f,
        [
            0x77 as f32 / 255.0,
            0xAB as f32 / 255.0,
            0x2F as f32 / 255.0
        ]
    );
}
