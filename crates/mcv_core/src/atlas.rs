//! Terrain texture array: layers `0..MANIFEST_LAYERS` 是真实方块贴图
//! （tiles_manifest.json 字典序，运行时从 `<texturepack>/blocks/*.png` 读盘），
//! 特殊程序化层（挖掘裂纹等）排在 `CRACK_BASE..`。真实贴图缺失时回退到
//! 程序化噪声（原始调色板，逐层配方见 LEGACY_RECIPES），保证不崩。

use crate::tiles;
use std::path::Path;

pub const TILE_PX: usize = 16;
/// 真实贴图层数，与 tiles_manifest.json 的 tile_index_to_file 长度一致。
pub const MANIFEST_LAYERS: usize = 827;
/// 挖掘裂纹叠加层数（4 档）。
pub const CRACK_LAYERS: usize = 4;
/// 裂纹 stage s 的数组层号。特殊层统一放在真实贴图之后。
pub const CRACK_BASE: usize = MANIFEST_LAYERS;
/// 图集总层数（= mcv_render::gpu 创建 texture array 的 depth_or_array_layers）。
pub const LAYERS: usize = MANIFEST_LAYERS + CRACK_LAYERS;
pub const MIP_LEVELS: u32 = 2;

const _: () = assert!(MANIFEST_LAYERS + CRACK_LAYERS <= u16::MAX as usize);

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

/// 程序化配方（原始调色板）：贴图名 → 画法。真实贴图缺失时按 manifest 名
/// 落到对应层作回退；名字必须是 tiles_manifest.json 里的原词。
enum Recipe {
    Noise { seed: u64, base: [u8; 3], amp: i32 },
    GrassSide,
    LogSide,
    LogTop,
    Planks,
    Cobble,
    SnowSide,
    Flower { red: bool },
}

/// 旧程序化贴图 → manifest 贴图名（同图异名，回退配方按新层号落位；
/// 旧 "snow" 并入 grass_block_top/grass_block_snow，故 15 项）。
const LEGACY_RECIPES: [(&str, Recipe); 15] = [
    (
        "grass_block_top",
        Recipe::Noise {
            seed: 0xA001,
            base: [104, 168, 62],
            amp: 20,
        },
    ),
    ("grass_block_side", Recipe::GrassSide),
    (
        "dirt",
        Recipe::Noise {
            seed: 0xA005,
            base: [134, 96, 67],
            amp: 20,
        },
    ),
    (
        "stone",
        Recipe::Noise {
            seed: 0xA007,
            base: [127, 127, 127],
            amp: 12,
        },
    ),
    (
        "sand",
        Recipe::Noise {
            seed: 0xA009,
            base: [219, 207, 163],
            amp: 10,
        },
    ),
    (
        "water_still",
        Recipe::Noise {
            seed: 0xA00B,
            base: [56, 108, 214],
            amp: 8,
        },
    ),
    ("oak_log", Recipe::LogSide),
    ("oak_log_top", Recipe::LogTop),
    (
        "oak_leaves",
        Recipe::Noise {
            seed: 0xA00F,
            base: [58, 116, 38],
            amp: 26,
        },
    ),
    ("oak_planks", Recipe::Planks),
    ("cobblestone", Recipe::Cobble),
    (
        "bedrock",
        Recipe::Noise {
            seed: 0xA013,
            base: [64, 64, 64],
            amp: 34,
        },
    ),
    // 官方无独立 "snow" 全层贴图（+Y 用 grass_block_top 的 snowy 状态），
    // SNOW 常量也指 336，配方不再单独占名。
    ("grass_block_snow", Recipe::SnowSide),
    ("poppy", Recipe::Flower { red: true }),
    ("dandelion", Recipe::Flower { red: false }),
];

fn paint_noise(
    p: &mut Painter,
    seed: u64,
    base: [u8; 3],
    amp: i32,
    speck: u64,
    sp_th: f32,
    sp_dark: i32,
) {
    p.fill_noise(seed, base, amp, 255);
    p.speckle(speck, sp_th, sp_dark);
}

fn paint_recipe(p: &mut Painter, name: &str, r: &Recipe) {
    match r {
        Recipe::Noise { seed, base, amp } => match name {
            "grass_block_top" => paint_noise(p, *seed, *base, *amp, 0xA002, 0.18, 22),
            "dirt" => paint_noise(p, *seed, *base, *amp, 0xA006, 0.14, 18),
            "stone" => paint_noise(p, *seed, *base, *amp, 0xA008, 0.20, 16),
            "sand" => paint_noise(p, *seed, *base, *amp, 0xA00A, 0.10, 12),
            "oak_leaves" => paint_noise(p, *seed, *base, *amp, 0xA010, 0.22, 30),
            _ => p.fill_noise(*seed, *base, *amp, 255), // water_still / bedrock
        },
        Recipe::GrassSide => {
            p.fill_noise(0xA003, [134, 96, 67], 18, 255);
            p.rows(0, 4, 0xA004, [104, 168, 62], 16, true);
        }
        Recipe::LogSide => {
            p.fill_noise(0xA00D, [104, 82, 50], 10, 255);
            for x in 0..TILE_PX as u32 {
                if x % 5 < 2 {
                    for y in 0..TILE_PX as u32 {
                        let off = p.layer * TILE_PX * TILE_PX * 4
                            + ((y as usize * TILE_PX + x as usize) * 4);
                        for c in 0..3 {
                            p.data[off + c] = clamp8(p.data[off + c] as i32 - 18);
                        }
                    }
                }
            }
        }
        Recipe::LogTop => {
            p.fill_noise(0xA00E, [104, 82, 50], 8, 255);
            for y in 0..TILE_PX as u32 {
                for x in 0..TILE_PX as u32 {
                    let d = (x.max(15 - x)).max(y.max(15 - y));
                    if d % 3 == 0 {
                        let off = p.layer * TILE_PX * TILE_PX * 4
                            + ((y as usize * TILE_PX + x as usize) * 4);
                        for c in 0..3 {
                            p.data[off + c] = clamp8(p.data[off + c] as i32 - 24);
                        }
                    }
                }
            }
        }
        Recipe::Planks => {
            p.fill_noise(0xA011, [162, 131, 78], 8, 255);
            for y in 0..TILE_PX as u32 {
                if y % 4 == 3 {
                    for x in 0..TILE_PX as u32 {
                        let off = p.layer * TILE_PX * TILE_PX * 4
                            + ((y as usize * TILE_PX + x as usize) * 4);
                        for c in 0..3 {
                            p.data[off + c] = clamp8(p.data[off + c] as i32 - 30);
                        }
                    }
                }
            }
        }
        Recipe::Cobble => {
            p.fill_noise(0xA012, [112, 112, 112], 14, 255);
            for y in 0..TILE_PX as u32 {
                for x in 0..TILE_PX as u32 {
                    let edge = (x % 5 == 0) || (y % 5 == 0);
                    if edge {
                        let off = p.layer * TILE_PX * TILE_PX * 4
                            + ((y as usize * TILE_PX + x as usize) * 4);
                        for c in 0..3 {
                            p.data[off + c] = clamp8(p.data[off + c] as i32 - 26);
                        }
                    }
                }
            }
        }
        Recipe::SnowSide => {
            p.fill_noise(0xA015, [134, 96, 67], 18, 255);
            p.rows(0, 5, 0xA016, [238, 244, 246], 6, true);
        }
        Recipe::Flower { red } => {
            // 透明背景 + 茎 + 花头（cutout）
            for y in 0..TILE_PX as u32 {
                for x in 0..TILE_PX as u32 {
                    p.set(x, y, 0, 0, 0, 0);
                }
            }
            let petal = if *red { [200, 40, 40] } else { [220, 200, 40] };
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
    }
}

/// 裂纹 stage（0..CRACK_LAYERS）：递增的暗色像素网。
fn paint_crack(p: &mut Painter, stage: usize) {
    for y in 0..TILE_PX as u32 {
        for x in 0..TILE_PX as u32 {
            p.set(x, y, 0, 0, 0, 0);
        }
    }
    let s = stage as u32;
    for i in 0..(6 + s * 8) {
        let x = (hash01(0xA017 + u64::from(s), i * 7, i) * 15.0) as u32;
        let y = (hash01(0xA018 + u64::from(s), i, i * 3) * 15.0) as u32;
        p.set(x, y, 20, 16, 12, 190);
        if x < 15 {
            p.set(x + 1, y, 20, 16, 12, 120);
        }
    }
}

/// Generates mip level 0 for all layers into `data` (LAYERS * 16*16*4 bytes).
/// 全层先铺 debug 品红占位（未命中回退配方的真实层保持品红），再画回退配方
/// 与特殊层；真实贴图由 [`load_real_tiles`] 在调用方覆盖。
pub fn generate_layers(data: &mut [u8]) {
    assert_eq!(data.len(), LAYERS * TILE_PX * TILE_PX * 4);
    let mut p = Painter { data, layer: 0 };

    for layer in 0..LAYERS {
        p.layer = layer;
        p.fill_noise(0xA019 + layer as u64, [180, 40, 180], 10, 255); // debug magenta
    }
    for (name, r) in &LEGACY_RECIPES {
        let Some(idx) = tile_file_names().iter().position(|f| f == name) else {
            continue; // manifest 换名时静默跳过：只是少一个回退配方
        };
        p.layer = idx;
        paint_recipe(&mut p, name, r);
    }
    for s in 0..CRACK_LAYERS {
        p.layer = CRACK_BASE + s;
        paint_crack(&mut p, s);
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

/// Full upload payload: mip0 + mip1 contiguous. 真实贴图从
/// `<pack_dir>/blocks/*.png` 读盘覆盖（目录不存在 = 纯程序化，兼容无素材部署）。
pub fn generate_payload_with_pack(pack_dir: Option<&Path>) -> Vec<u8> {
    let mut mip0 = vec![0u8; LAYERS * TILE_PX * TILE_PX * 4];
    generate_layers(&mut mip0);
    if let Some(dir) = pack_dir {
        let n = load_real_tiles(dir, &mut mip0);
        if n > 0 {
            log::info!(
                "atlas: {n}/{} real tiles from {}",
                MANIFEST_LAYERS,
                dir.display()
            );
        }
    }
    let mut mip1 = vec![0u8; LAYERS * 8 * 8 * 4];
    generate_mip1(&mip0, &mut mip1);
    mip0.extend_from_slice(&mip1);
    mip0
}

/// 无纹理包路径（纯程序化）。带真实贴图用 [`generate_payload_with_pack`]。
pub fn generate_payload() -> Vec<u8> {
    generate_payload_with_pack(None)
}

/// 层数钳制（GLES `MAX_ARRAY_TEXTURE_LAYERS` 常为 256 < 831）：
/// 钳到 `n` 层时返回实际可用的 mip0+mip1 载荷与数组层数。
/// tiles 引用被钳掉的层时 wgpu 在采样器边界内回绕/钳位（贴图上屏，不崩）。
/// 建议 gpu.rs 用 `min(LAYERS, limits.max_texture_layers())` 调用。
pub fn generate_payload_clamped(pack_dir: Option<&Path>, max_layers: usize) -> (Vec<u8>, usize) {
    let n = max_layers.clamp(1, LAYERS);
    let full = generate_payload_with_pack(pack_dir);
    if n == LAYERS {
        return (full, n);
    }
    let mip0_full = LAYERS * TILE_PX * TILE_PX * 4;
    let mut out = Vec::with_capacity(n * (TILE_PX * TILE_PX + 8 * 8) * 4);
    out.extend_from_slice(&full[..n * TILE_PX * TILE_PX * 4]);
    let mip1_off = mip0_full + n * 8 * 8 * 4;
    out.extend_from_slice(&full[mip0_full..mip1_off]);
    (out, n)
}

const MANIFEST_JSON: &str = include_str!("../tiles_manifest.json");
const BLOCKS_SUBDIR: &str = "blocks";
// manifest 键存在性由 tests::parse_manifest_names 运行期锁（str::find 非 const）

/// 从嵌入的 tiles_manifest.json 抠出 `tile_index_to_file` 数组的文件名词表。
/// 手写极简解析（该数组在 JSON 中最靠前，取首个 `[...]` 内的全部引号串；
/// 生成器保证词表内无括号/逗号/引号字符）——零 serde 依赖。
fn parse_manifest_names() -> Vec<&'static str> {
    let key = MANIFEST_JSON
        .find("\"tile_index_to_file\"")
        .expect("manifest: missing tile_index_to_file");
    let open = MANIFEST_JSON[key..]
        .find('[')
        .expect("manifest: missing array open");
    let arr = &MANIFEST_JSON[key + open..];
    let close = arr.find(']').expect("manifest: missing array close");
    let mut names = Vec::with_capacity(MANIFEST_LAYERS);
    // split('"') 后奇数下标段即引号内内容（偶数段是分隔符/前缀）
    for seg in arr[..close].split('"').skip(1).step_by(2) {
        names.push(seg);
    }
    names
}

/// manifest 层号 → 贴图文件名（不含扩展名），下标即层号。
pub fn tile_file_names() -> &'static [&'static str] {
    static NAMES: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    let v = NAMES.get_or_init(parse_manifest_names);
    assert_eq!(
        v.len(),
        MANIFEST_LAYERS,
        "tiles_manifest.json 层数与 MANIFEST_LAYERS 不符"
    );
    v.as_slice()
}

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

/// 解码 PNG → 16x16 RGBA → 写入 mip0 的指定层。失败返回 false（保留现有内容）。
fn decode_into_layer(bytes: &[u8], src_path: &Path, layer: usize, layers: &mut [u8]) -> bool {
    let Ok(img) = image::load_from_memory(bytes) else {
        log::warn!("atlas: failed to decode {}", src_path.display());
        return false;
    };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return false;
    }
    let mut tile = [0u8; TILE_PX * TILE_PX * 4];
    resample_to_tile(rgba.as_raw(), w, h, &mut tile);
    let off = layer * TILE_PX * TILE_PX * 4;
    layers[off..off + tile.len()].copy_from_slice(&tile);
    true
}

/// 从 `<dir>/blocks/<name>.png` 加载 827 张真实方块贴图覆盖层 0..827。
/// 缺文件/解码失败保留程序化回退（开发期桌面与真机 texturepack 路径差异
/// 下不崩）；目录整体不存在时零次读盘尝试。返回覆盖层数。
pub fn load_real_tiles(dir: &Path, layers: &mut [u8]) -> u32 {
    assert!(layers.len() >= MANIFEST_LAYERS * TILE_PX * TILE_PX * 4);
    let blocks = dir.join(BLOCKS_SUBDIR);
    if !blocks.is_dir() {
        return 0;
    }
    let mut count = 0u32;
    for (idx, name) in tile_file_names().iter().enumerate().skip(1) {
        // 层 0 是生成器占位层，无对应文件
        let path = blocks.join(format!("{name}.png"));
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if decode_into_layer(&bytes, &path, idx, layers) {
            count += 1;
        }
    }
    count
}

/// 标准 MC 资源包树（贴图名 1.13+ 起即 manifest 原名，无需翻译表）。
const MC_DIRS: [&str; 2] = ["assets/minecraft/textures/block", "textures/block"];

/// 纹理包旧图引擎名（历史 API，值即 manifest 贴图名，15 项）。
pub const PACK_TILE_NAMES: [&str; 15] = [
    "grass_block_top",
    "grass_block_side",
    "dirt",
    "stone",
    "sand",
    "water_still",
    "oak_log",
    "oak_log_top",
    "oak_leaves",
    "oak_planks",
    "cobblestone",
    "bedrock",
    "grass_block_snow",
    "poppy",
    "dandelion",
];

/// Overrides atlas layers with user texture-pack PNGs for the classic 15
/// tiles (files named after manifest texture names, e.g. `stone.png`, or the
/// standard MC pack tree). Each name resolves to its manifest layer, so pack
/// files replace the real/程序化 content of exactly that layer.
///
/// Returns the number of layers overridden.
pub fn load_pack_over(dir: &std::path::Path, mip0: &mut [u8]) -> u32 {
    // 调用方（gpu.rs）传 mip0 子切片，只要求覆盖真实层区
    assert!(mip0.len() >= MANIFEST_LAYERS * TILE_PX * TILE_PX * 4);
    let names = tile_file_names();
    let mut count = 0u32;
    for name in PACK_TILE_NAMES {
        // 候选顺序：扁平引擎名 → 标准 MC 包树。首个命中生效。
        let candidates: Vec<std::path::PathBuf> = {
            let mut v = vec![dir.join(format!("{name}.png"))];
            for d in MC_DIRS {
                v.push(dir.join(d).join(format!("{name}.png")));
            }
            v
        };
        let Some((bytes, src_path)) = candidates
            .iter()
            .find_map(|p| std::fs::read(p).ok().map(|b| (b, p.clone())))
        else {
            continue;
        };
        let Some(idx) = names.iter().position(|f| *f == name) else {
            continue;
        };
        if decode_into_layer(&bytes, &src_path, idx, mip0) {
            count += 1;
        }
    }
    count
}

/// 编译期一致性：tiles:: 常量必须是 manifest 里对应贴图的层号。
const _: () = {
    // 解析在 const 里做不了（需要 OnceLock），退化为范围断言 + 运行期
    // tests::gen_table_layer_indices_in_real_region 双重锁。
    assert!(tiles::GRASS_TOP as usize > 0 && (tiles::GRASS_TOP as usize) < MANIFEST_LAYERS);
    assert!((tiles::STONE as usize) < MANIFEST_LAYERS);
    assert!((tiles::FLOWER_YELLOW as usize) < MANIFEST_LAYERS);
};
