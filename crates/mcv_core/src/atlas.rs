//! Procedural 16x16 texture array generation (original palette; MC textures
//! are proprietary so every tile is painted from noise, not copied).

use crate::tiles;

pub const TILE_PX: usize = 16;
pub const LAYERS: usize = 32;
pub const MIP_LEVELS: u32 = 2;

fn hash01(seed: u64, x: u32, y: u32) -> f32 {
    let mut h =
        seed ^ (u64::from(x).wrapping_mul(0x9E3779B1)) ^ (u64::from(y).wrapping_mul(0x85EBCA77));
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 31;
    (h >> 40) as f32 / 16_777_216.0
}

struct Painter<'a> {
    data: &'a mut [u8],
    layer: usize,
}

impl Painter<'_> {
    fn set(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        let off = self.layer * TILE_PX * TILE_PX * 4 + ((y as usize * TILE_PX + x as usize) * 4);
        self.data[off] = r;
        self.data[off + 1] = g;
        self.data[off + 2] = b;
        self.data[off + 3] = a;
    }

    fn fill_noise(&mut self, seed: u64, base: [u8; 3], amp: i32, alpha: u8) {
        for y in 0..TILE_PX as u32 {
            for x in 0..TILE_PX as u32 {
                let n = (hash01(seed, x, y) * 2.0 - 1.0) * amp as f32;
                self.set(
                    x,
                    y,
                    clamp8(base[0] as i32 + n as i32),
                    clamp8(base[1] as i32 + n as i32),
                    clamp8(base[2] as i32 + n as i32),
                    alpha,
                );
            }
        }
    }

    fn speckle(&mut self, seed: u64, threshold: f32, dark: i32) {
        for y in 0..TILE_PX as u32 {
            for x in 0..TILE_PX as u32 {
                if hash01(seed ^ 0x51, x, y) < threshold {
                    let off = self.layer * TILE_PX * TILE_PX * 4
                        + ((y as usize * TILE_PX + x as usize) * 4);
                    for c in 0..3 {
                        self.data[off + c] = clamp8(self.data[off + c] as i32 - dark);
                    }
                }
            }
        }
    }

    fn rows(&mut self, from: u32, to: u32, seed: u64, base: [u8; 3], amp: i32, ragged: bool) {
        for y in from..to {
            for x in 0..TILE_PX as u32 {
                let extra = if ragged && hash01(seed ^ 0x77, x, y) < 0.35 {
                    1
                } else {
                    0
                };
                let n = (hash01(seed, x, y) * 2.0 - 1.0) * amp as f32;
                self.set(
                    x,
                    y.min(TILE_PX as u32 - 1 + extra),
                    clamp8(base[0] as i32 + n as i32),
                    clamp8(base[1] as i32 + n as i32),
                    clamp8(base[2] as i32 + n as i32),
                    255,
                );
            }
        }
    }
}

fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Generates mip level 0 for all layers into `data` (layers * 16*16*4 bytes).
pub fn generate_layers(data: &mut [u8]) {
    assert_eq!(data.len(), LAYERS * TILE_PX * TILE_PX * 4);
    let mut p = Painter { data, layer: 0 };
    let _ = &mut p;

    for layer in 0..LAYERS {
        p.layer = layer;
        match layer as u16 {
            tiles::GRASS_TOP => {
                p.fill_noise(0xA001, [104, 168, 62], 20, 255);
                p.speckle(0xA002, 0.18, 22);
            }
            tiles::GRASS_SIDE => {
                p.fill_noise(0xA003, [134, 96, 67], 18, 255);
                p.rows(0, 4, 0xA004, [104, 168, 62], 16, true);
            }
            tiles::DIRT => {
                p.fill_noise(0xA005, [134, 96, 67], 20, 255);
                p.speckle(0xA006, 0.14, 18);
            }
            tiles::STONE => {
                p.fill_noise(0xA007, [127, 127, 127], 12, 255);
                p.speckle(0xA008, 0.20, 16);
            }
            tiles::SAND => {
                p.fill_noise(0xA009, [219, 207, 163], 10, 255);
                p.speckle(0xA00A, 0.10, 12);
            }
            tiles::WATER => {
                p.fill_noise(0xA00B, [56, 108, 214], 8, 255);
                // lighter wave rows
                for y in 0..TILE_PX as u32 {
                    if (y + 2) % 7 < 2 {
                        for x in 0..TILE_PX as u32 {
                            let n = hash01(0xA00C, x, y) * 24.0;
                            let off = layer * TILE_PX * TILE_PX * 4
                                + ((y as usize * TILE_PX + x as usize) * 4);
                            p.data[off] = clamp8(56 + 24 + n as i32);
                            p.data[off + 1] = clamp8(108 + 26 + n as i32);
                            p.data[off + 2] = clamp8(214 + 20 + n as i32);
                        }
                    }
                }
            }
            tiles::LOG_SIDE => {
                p.fill_noise(0xA00D, [104, 82, 50], 10, 255);
                for x in 0..TILE_PX as u32 {
                    if x % 5 < 2 {
                        for y in 0..TILE_PX as u32 {
                            let off = layer * TILE_PX * TILE_PX * 4
                                + ((y as usize * TILE_PX + x as usize) * 4);
                            for c in 0..3 {
                                p.data[off + c] = clamp8(p.data[off + c] as i32 - 18);
                            }
                        }
                    }
                }
            }
            tiles::LOG_TOP => {
                p.fill_noise(0xA00E, [104, 82, 50], 8, 255);
                for y in 0..TILE_PX as u32 {
                    for x in 0..TILE_PX as u32 {
                        let d = (x.max(15 - x)).max(y.max(15 - y));
                        if d % 3 == 0 {
                            let off = layer * TILE_PX * TILE_PX * 4
                                + ((y as usize * TILE_PX + x as usize) * 4);
                            for c in 0..3 {
                                p.data[off + c] = clamp8(p.data[off + c] as i32 - 24);
                            }
                        }
                    }
                }
            }
            tiles::LEAVES => {
                p.fill_noise(0xA00F, [58, 116, 38], 26, 255);
                p.speckle(0xA010, 0.22, 30);
            }
            tiles::PLANKS => {
                p.fill_noise(0xA011, [162, 131, 78], 8, 255);
                for y in 0..TILE_PX as u32 {
                    if y % 4 == 3 {
                        for x in 0..TILE_PX as u32 {
                            let off = layer * TILE_PX * TILE_PX * 4
                                + ((y as usize * TILE_PX + x as usize) * 4);
                            for c in 0..3 {
                                p.data[off + c] = clamp8(p.data[off + c] as i32 - 30);
                            }
                        }
                    }
                }
            }
            tiles::COBBLE => {
                p.fill_noise(0xA012, [112, 112, 112], 14, 255);
                // blob cells
                for y in 0..TILE_PX as u32 {
                    for x in 0..TILE_PX as u32 {
                        let cx = x / 5;
                        let cy = y / 5;
                        let edge = (x % 5 == 0) || (y % 5 == 0);
                        if edge {
                            let off = layer * TILE_PX * TILE_PX * 4
                                + ((y as usize * TILE_PX + x as usize) * 4);
                            for c in 0..3 {
                                p.data[off + c] = clamp8(p.data[off + c] as i32 - 26);
                            }
                        }
                        let _ = (cx, cy);
                    }
                }
            }
            tiles::BEDROCK => {
                p.fill_noise(0xA013, [64, 64, 64], 34, 255);
            }
            tiles::SNOW => {
                p.fill_noise(0xA014, [238, 244, 246], 6, 255);
            }
            tiles::SNOW_SIDE => {
                p.fill_noise(0xA015, [134, 96, 67], 18, 255);
                p.rows(0, 5, 0xA016, [238, 244, 246], 6, true);
            }
            tiles::FLOWER_RED | tiles::FLOWER_YELLOW => {
                // transparent background + stem + petal head (cutout)
                for y in 0..TILE_PX as u32 {
                    for x in 0..TILE_PX as u32 {
                        p.set(x, y, 0, 0, 0, 0);
                    }
                }
                let petal = if layer as u16 == tiles::FLOWER_RED {
                    [200, 40, 40]
                } else {
                    [220, 200, 40]
                };
                for y in 8..15 {
                    p.set(7, y, 40, 120, 40, 255);
                    p.set(8, y, 50, 130, 45, 255);
                }
                let head = [
                    (6u32, 3u32),
                    (7, 2),
                    (8, 2),
                    (9, 3),
                    (6, 4),
                    (7, 3),
                    (8, 3),
                    (9, 4),
                    (7, 4),
                    (8, 4),
                ];
                for (x, y) in head {
                    p.set(x, y, petal[0], petal[1], petal[2], 255);
                }
                p.set(7, 5, 240, 220, 120, 255);
                p.set(8, 5, 240, 220, 120, 255);
            }
            20..=23 => {
                // mining crack overlay: increasing dark pixel web
                for y in 0..TILE_PX as u32 {
                    for x in 0..TILE_PX as u32 {
                        p.set(x, y, 0, 0, 0, 0);
                    }
                }
                let stage = (layer - 20) as u32;
                for i in 0..(6 + stage * 8) {
                    let x = (hash01(0xA017 + u64::from(stage), i * 7, i) * 15.0) as u32;
                    let y = (hash01(0xA018 + u64::from(stage), i, i * 3) * 15.0) as u32;
                    p.set(x, y, 20, 16, 12, 190);
                    if x < 15 {
                        p.set(x + 1, y, 20, 16, 12, 120);
                    }
                }
            }
            _ => {
                p.fill_noise(0xA019 + layer as u64, [180, 40, 180], 10, 255); // debug magenta
            }
        }
    }
}

/// CPU box-downsample mip level 1 (16 -> 8 px per side, per layer).
pub fn generate_mip1(mip0: &[u8], mip1: &mut [u8]) {
    assert_eq!(mip0.len(), LAYERS * TILE_PX * TILE_PX * 4);
    assert_eq!(mip1.len(), LAYERS * 8 * 8 * 4);
    for layer in 0..LAYERS {
        for y in 0..8u32 {
            for x in 0..8u32 {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for dy in 0..2u32 {
                        for dx in 0..2u32 {
                            let sx = x * 2 + dx;
                            let sy = y * 2 + dy;
                            let off = layer * TILE_PX * TILE_PX * 4
                                + ((sy * TILE_PX as u32 + sx) * 4 + c) as usize;
                            sum += u32::from(mip0[off]);
                        }
                    }
                    mip1[layer * 8 * 8 * 4 + ((y * 8 + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
    }
}

/// Full upload payload: mip0 + mip1 contiguous.
pub fn generate_payload() -> Vec<u8> {
    let mut mip0 = vec![0u8; LAYERS * TILE_PX * TILE_PX * 4];
    generate_layers(&mut mip0);
    let mut mip1 = vec![0u8; LAYERS * 8 * 8 * 4];
    generate_mip1(&mip0, &mut mip1);
    mip0.extend_from_slice(&mip1);
    mip0
}

/// Tile file names loadable from a texture pack directory.
pub const PACK_TILE_NAMES: [&str; 16] = [
    "grass_top",
    "grass_side",
    "dirt",
    "stone",
    "sand",
    "water",
    "log_side",
    "log_top",
    "leaves",
    "planks",
    "cobble",
    "bedrock",
    "snow",
    "snow_side",
    "flower_red",
    "flower_yellow",
];

/// Nearest-neighbour resample of an RGBA image to TILE_PX x TILE_PX.
fn resample_to_tile(src: &[u8], sw: u32, sh: u32, dst: &mut [u8]) {
    for y in 0..TILE_PX as u32 {
        for x in 0..TILE_PX as u32 {
            let sx = (u64::from(x) * u64::from(sw) / u64::from(TILE_PX as u32)) as u32;
            let sy = (u64::from(y) * u64::from(sh) / u64::from(TILE_PX as u32)) as u32;
            for c in 0..4 {
                let d = ((y * TILE_PX as u32 + x) * 4 + c) as usize;
                let s = ((sy * sw + sx) * 4 + c) as usize;
                dst[d] = src[s];
            }
        }
    }
}

/// Overrides procedural layers with PNGs from `dir` (files named after
/// [`PACK_TILE_NAMES`], e.g. `stone.png`). Images of any size are accepted
/// (nearest-resampled to 16x16); RGBA or RGB. This keeps the engine free of
/// third-party art: users supply their own packs at runtime.
///
/// Returns the number of layers overridden.
pub fn load_pack_over(dir: &std::path::Path, layers: &mut [u8]) -> u32 {
    /// Engine tile -> candidate file names inside a standard MC resource pack
    /// (`assets/minecraft/textures/block/`). Flat packs just use the engine
    /// name directly, e.g. `stone.png`.
    const MC_NAMES: [(&str, &str); 16] = [
        ("grass_top", "grass_block_top"),
        ("grass_side", "grass_block_side"),
        ("dirt", "dirt"),
        ("stone", "stone"),
        ("sand", "sand"),
        ("water", "water_still"),
        ("log_side", "oak_log"),
        ("log_top", "oak_log_top"),
        ("leaves", "oak_leaves"),
        ("planks", "oak_planks"),
        ("cobble", "cobblestone"),
        ("bedrock", "bedrock"),
        ("snow", "snow"),
        ("snow_side", "grass_block_snow"),
        ("flower_red", "poppy"),
        ("flower_yellow", "dandelion"),
    ];
    const MC_DIRS: [&str; 2] = ["assets/minecraft/textures/block", "textures/block"];

    let mut count = 0u32;
    for (idx, name) in PACK_TILE_NAMES.iter().enumerate() {
        // Candidate order: flat engine name, MC-style names in the standard
        // pack tree. First hit wins.
        let candidates: Vec<std::path::PathBuf> = {
            let mut v = vec![dir.join(format!("{name}.png"))];
            if let Some((_, mc)) = MC_NAMES.iter().find(|(e, _)| *e == *name) {
                for d in MC_DIRS {
                    v.push(dir.join(d).join(format!("{mc}.png")));
                }
            }
            v
        };
        let (bytes, src_path) = candidates
            .iter()
            .find_map(|p| std::fs::read(p).ok().map(|b| (b, p.clone())))
            .unwrap_or((Vec::new(), std::path::PathBuf::new()));
        if bytes.is_empty() {
            continue;
        }
        let Ok(img) = image::load_from_memory(&bytes) else {
            log::warn!("texture pack: failed to decode {}", src_path.display());
            continue;
        };
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        if w == 0 || h == 0 {
            continue;
        }
        let layer = idx + 1; // PACK_TILE_NAMES[0] is layer 1 (grass_top)
        let layer_off = layer * TILE_PX * TILE_PX * 4;
        let mut tile = [0u8; TILE_PX * TILE_PX * 4];
        resample_to_tile(rgba.as_raw(), w, h, &mut tile);
        layers[layer_off..layer_off + tile.len()].copy_from_slice(&tile);
        count += 1;
    }
    count
}
