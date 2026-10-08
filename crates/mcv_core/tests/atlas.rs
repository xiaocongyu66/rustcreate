//! Atlas generation + texture-pack override tests (CPU-only).

use mcv_core::atlas;

#[test]
fn payload_layout() {
    let p = atlas::generate_payload();
    let mip0 = atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4;
    let mip1 = atlas::LAYERS * 8 * 8 * 4;
    assert_eq!(p.len(), mip0 + mip1);
    // layer 0 is debug magenta placeholder; grass_top (1) must be greenish
    let g = p[(atlas::TILE_PX * atlas::TILE_PX + (8 * 16 + 8)) * 4 + 1];
    let r = p[(atlas::TILE_PX * atlas::TILE_PX + (8 * 16 + 8)) * 4];
    assert!(g > r, "grass_top center must be green-dominant");
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
    let stone_off = 4 * atlas::TILE_PX * atlas::TILE_PX * 4; // layer 4 = stone
    assert_eq!(layers[stone_off], 200);
    assert_eq!(layers[stone_off + 1], 10);
    // corner resample covers the whole tile
    assert_eq!(layers[stone_off + (16 * 15 + 15) * 4], 200);
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
    let off = atlas::TILE_PX * atlas::TILE_PX * 4; // layer 1 = grass_top
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
