//! Atlas generation + texture-pack override tests (CPU-only).

use mcv_core::atlas;

#[test]
fn manifest_json_wellformed() {
    let s = include_str!("../tiles_manifest.json");
    assert!(s.contains("\"tile_index_to_file\""), "manifest key missing");
}

/// 素材红线（2026-10 任务 #53）：无素材时全部层 = 原版 missing 标记
/// （MissingTextureAtlasSprite 品红/黑棋盘），不再有任何程序化噪声配方。
#[test]
fn missing_marker_when_no_assets() {
    let p = atlas::generate_payload();
    let tile = atlas::TILE_PX * atlas::TILE_PX * 4;
    // 哨兵层与真实层区（此处未喂素材）都应是 missing 棋盘。
    for layer in [0usize, atlas::SENTINEL_LAYER, atlas::CRACK_BASE] {
        let off = layer * tile;
        // 中心 (8,8) 落在右上/左下品红象限（y<8 ^ x<8 = false → 黑？）：
        // y=8 不小于 8，x=8 不小于 8 → false^false → 黑象限。取 (12,4)
        // （右上象限）应为品红。
        let pink = off + (4 * atlas::TILE_PX + 12) * 4;
        let black = off + (4 * atlas::TILE_PX + 4) * 4;
        assert_eq!(
            &p[pink..pink + 4],
            &[248, 0, 248, 255],
            "层 {layer} 品红象限"
        );
        assert_eq!(&p[black..black + 4], &[0, 0, 0, 255], "层 {layer} 黑象限");
    }
}

/// 素材完整性（CPU）：仓库内 827 张真实方块贴图 + 10 档原版裂纹必须全部
/// 命中对应层——真机曾出现 826/827（层 0 被旧 skip(1) 跳过，acacia 门
/// 贴图缺失逼出品红），该断言锁死 827/827。
#[test]
fn real_assets_fill_all_real_layers() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/minecraft");
    let mut mip0 = vec![0u8; atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4];
    let n = atlas::load_real_tiles(&dir, &mut mip0);
    assert_eq!(
        n as usize,
        atlas::REAL_TILE_COUNT,
        "827 张真实贴图应全部加载"
    );
    let c = atlas::load_crack_stages(&dir, &mut mip0);
    assert_eq!(c as usize, atlas::CRACK_LAYERS, "10 档原版裂纹应全部加载");
    // 抽查关键层内容已被真实贴图覆盖（非 missing 品红棋盘）：
    // 草顶为灰度遮罩（r≈g≈b 且非品红）、黄毛 tile 非黑。
    let tile = atlas::TILE_PX * atlas::TILE_PX * 4;
    let center = |layer: usize| (layer * tile) + (8 * atlas::TILE_PX + 8) * 4;
    let grass = &mip0[center(mcv_core::tiles::GRASS_TOP as usize)..][..3];
    assert!(
        !(grass[0] > 150 && grass[2] > 150 && grass[1] + 60 < grass[0]),
        "grass_block_top 层不应是 missing 品红：{grass:?}"
    );
    let yellow = &mip0[center(826)..][..3];
    assert!(
        yellow.iter().any(|&v| v > 0),
        "yellow_wool 层（826，真实贴图区末层）不应全黑"
    );
}

#[test]
fn clamped_payload() {
    // GLES 上限模拟：钳到 64 层 = 64×(16×16+8×8)×4 字节，且 mip0 前缀与全量一致
    let (full, n_full) = atlas::generate_payload_clamped(None, atlas::LAYERS);
    assert_eq!(n_full, atlas::LAYERS);
    let (small, n) = atlas::generate_payload_clamped(None, 64);
    assert_eq!(n, 64);
    assert_eq!(small.len(), 64 * (16 * 16 + 8 * 8) * 4);
    assert_eq!(&small[..64 * 16 * 16 * 4], &full[..64 * 16 * 16 * 4]);
    // 钳制只截尾，不动头部真实贴图区
    const { assert!(atlas::CRACK_BASE > 64) };
}

#[test]
fn pack_override() {
    // Build a 4x4 solid red PNG in memory and override stone.
    let mut img = image::RgbaImage::new(4, 4);
    for px in img.pixels_mut() {
        *px = image::Rgba([200, 10, 10, 255]);
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let dir = std::env::temp_dir().join("mcv_pack_test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stone.png"), &png).unwrap();

    let mut layers = vec![0u8; atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4];
    let n = atlas::load_pack_over(&dir, &mut layers);
    assert_eq!(n, 1, "exactly one tile overridden");
    let stone_off = (mcv_core::tiles::STONE as usize) * atlas::TILE_PX * atlas::TILE_PX * 4;
    assert_eq!(layers[stone_off], 200);
    assert_eq!(layers[stone_off + 1], 10);
    // corner resample covers the whole tile
    assert_eq!(layers[stone_off + (16 * 15 + 15) * 4], 200);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mc_resource_pack_layout() {
    // Standard MC pack tree: assets/minecraft/textures/block/<mc_name>.png
    let mut img = image::RgbaImage::new(32, 32);
    for px in img.pixels_mut() {
        *px = image::Rgba([10, 200, 10, 255]);
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let dir = std::env::temp_dir().join("mcv_pack_mc_test");
    let block_dir = dir.join("assets/minecraft/textures/block");
    std::fs::create_dir_all(&block_dir).unwrap();
    std::fs::write(block_dir.join("grass_block_top.png"), &png).unwrap();

    let mut layers = vec![0u8; atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4];
    let n = atlas::load_pack_over(&dir, &mut layers);
    assert_eq!(n, 1, "grass_top picked from the MC pack tree");
    let off = (mcv_core::tiles::GRASS_TOP as usize) * atlas::TILE_PX * atlas::TILE_PX * 4;
    assert_eq!(layers[off + 1], 200);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mip_downsample() {
    let mut mip0 = vec![0u8; atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4];
    atlas::generate_layers(&mut mip0);
    let mut mip1 = vec![0u8; atlas::LAYERS * 8 * 8 * 4];
    atlas::generate_mip1(&mip0, &mut mip1);
    // per-channel average of the four top-left pixels of layer 1
    let src = atlas::TILE_PX * atlas::TILE_PX * 4;
    let dst = 8 * 8 * 4; // layer 1 base in the downsampled buffer
    for c in 0..4 {
        // 2x2 block: (0,0),(1,0),(0,1),(1,1) — row stride is TILE_PX*4 bytes
        let offs = [0usize, 4, atlas::TILE_PX * 4, atlas::TILE_PX * 4 + 4];
        let exp: u32 = offs
            .iter()
            .map(|&o| u32::from(mip0[src + o + c]))
            .sum::<u32>()
            / 4;
        assert_eq!(mip1[dst + c] as u32, exp, "channel {c}");
    }
}

#[test]
fn vanilla_crack_stages_load() {
    // 仓库内已提交的原版 destroy_stage_0..9（26.1 jar MANIFEST 名录）必须
    // 全部命中裂纹层 CRACK_BASE+s；原版裂纹为黑裂纹 + alpha，覆盖面逐档
    // 单调不减（MultiPlayerGameMode.java:551 十档映射）。
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/minecraft");
    let mut layers = vec![0u8; atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4];
    let n = atlas::load_crack_stages(&dir, &mut layers);
    assert_eq!(n, atlas::CRACK_LAYERS as u32, "10 档原版裂纹应全部加载");
    let tile = atlas::TILE_PX * atlas::TILE_PX * 4;
    let crack_alpha = |s: usize, i: usize| layers[(atlas::CRACK_BASE + s) * tile + i * 4 + 3];
    // stage 0 有不透明裂纹像素（原版贴图 alpha>0），且逐档裂纹像素数不减。
    let count = |s: usize| (0..256).filter(|&i| crack_alpha(s, i) > 0).count();
    assert!(count(0) > 0, "stage0 裂纹层不应为空");
    assert!(
        (0..9).all(|s| count(s + 1) >= count(s)),
        "裂纹覆盖应随档位单调不减: {:?}",
        (0..10).map(count).collect::<Vec<_>>()
    );
}
