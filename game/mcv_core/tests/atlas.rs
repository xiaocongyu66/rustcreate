//! Atlas generation + texture-pack override tests (CPU-only).

use mcv_core::atlas;

#[test]
fn manifest_json_wellformed() {
    let s = include_str!("../tiles_manifest.json");
    assert!(s.contains("\"tile_index_to_file\""), "manifest key missing");
}

#[test]
fn payload_layout() {
    let p = atlas::generate_payload();
    let mip0 = atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4;
    let mip1 = atlas::LAYERS * 8 * 8 * 4;
    assert_eq!(p.len(), mip0 + mip1);
    // layer 0 is debug magenta placeholder; grass_top fallback recipe must be
    // painted at its manifest layer
    let off = (mcv_core::tiles::GRASS_TOP as usize) * atlas::TILE_PX * atlas::TILE_PX * 4
        + (8 * 16 + 8) * 4;
    assert!(
        p[off + 1] > p[off],
        "grass_top center must be green-dominant"
    );
}

#[test]
fn split_layer_counts_shapes() {
    // 上限 ≥ 837（lavapipe 3907 / 桌面 Vulkan）：单数组，零拆分零回归
    assert_eq!(
        atlas::split_layer_counts(atlas::LAYERS),
        vec![atlas::LAYERS]
    );
    assert_eq!(atlas::split_layer_counts(3907), vec![atlas::LAYERS]);
    // 刚好少一层 → 拆 2 份均分
    assert_eq!(atlas::split_layer_counts(atlas::LAYERS - 1), vec![419, 418]);
    // GLES 保底 256 → 4 个数组均分装满 837
    assert_eq!(atlas::split_layer_counts(256), vec![210, 210, 210, 207]);
    assert_eq!(atlas::split_layer_counts(512), vec![419, 418]);
    // 连 GLES 下限都没给的异常 adapter：不越上限，截断兜底（总和 < 837）
    assert_eq!(atlas::split_layer_counts(100), vec![100, 100, 100, 100]);
    assert_eq!(atlas::split_layer_counts(0), vec![1, 1, 1, 1]);
}

#[test]
fn remap_layer_boundaries() {
    let c4 = atlas::split_layer_counts(256);
    assert_eq!(c4, vec![210, 210, 210, 207]);
    // 数组首/尾边界全覆盖：GRASS_TOP=336 落数组 1，CRACK_BASE=827 落数组 3
    let cases = [
        (0usize, (0usize, 0usize)),
        (209, (0, 209)),
        (210, (1, 0)),
        (336, (1, 126)),
        (419, (1, 209)),
        (420, (2, 0)),
        (629, (2, 209)),
        (630, (3, 0)),
        (827, (3, 197)),
        (836, (3, 206)),
    ];
    for (layer, expect) in cases {
        assert_eq!(atlas::remap_layer(layer, &c4), expect, "layer={layer}");
    }
    // 单数组全走 (0, l)
    let c1 = vec![atlas::LAYERS];
    for layer in [0usize, 1, 336, 827, 836] {
        assert_eq!(atlas::remap_layer(layer, &c1), (0, layer));
    }
    // 越界钳到末数组末层（防御语义）
    assert_eq!(atlas::remap_layer(usize::MAX, &c4), (3, 206));
    // round-trip：每个（数组, 局部层）唯一对应一个全局层号
    let mut acc = 0usize;
    for (i, &cnt) in c4.iter().enumerate() {
        for local in 0..cnt {
            assert_eq!(atlas::remap_layer(acc + local, &c4), (i, local));
        }
        acc += cnt;
    }
    assert_eq!(acc, atlas::LAYERS);
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
