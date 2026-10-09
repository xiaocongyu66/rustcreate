//! Terrain texture array: layers `0..MANIFEST_LAYERS-1` 来自 tiles_manifest.json
//! （字典序；运行时从资源根 `textures/block/*.png` 读盘，827 张真实贴图 +
//! 1 层 missing 哨兵），特殊层（挖掘裂纹）排在 `CRACK_BASE..`。
//!
//! 素材红线（2026-10 任务 #53）：**本 crate 不产生任何程序化假贴图**。
//! 旧版按贴图名逐层画噪声的 LEGACY_RECIPES 回退已全部删除——真实贴图
//! 缺失时该层保留原版 missing 贴图（[`fill_missing_marker`]，与
//! `MissingTextureAtlasSprite.generateMissingImage` 逐像素一致），调用方
//! 以 `log::error!` 显式报错，绝不伪装成真实贴图。裂纹层只读原版
//! `textures/block/destroy_stage_0..9.png`，缺失即为 missing 标记。

use crate::tiles;
use std::path::Path;

pub const TILE_PX: usize = 16;
/// manifest 层数 = 827 张真实方块贴图（层 0..826，字典序）+ 1 层 missing
/// 哨兵（层 [`SENTINEL_LAYER`]，与 tiles_manifest.json 的
/// tile_index_to_file 长度一致）。
pub const MANIFEST_LAYERS: usize = 828;
/// 827 张真实方块贴图（不含哨兵）。完整性断言以此为准。
pub const REAL_TILE_COUNT: usize = MANIFEST_LAYERS - 1;
/// missing 哨兵层号（manifest 最后一项 `missing_no_texture`，无对应文件）。
/// 模型无贴图解析的方块（air/barrier/light/structure_void 等，见
/// ci/gen-blocks.py `tile_id`）指向该层，永远显示原版 missingno 品红标记。
pub const SENTINEL_LAYER: usize = MANIFEST_LAYERS - 1;
/// 哨兵贴图名（与 ci/gen-blocks.py SENTINEL_NAME 一致）。
pub const SENTINEL_TILE: &str = "missing_no_texture";
/// 挖掘裂纹叠加层数（10 档）。原版 26.1 为 destroy_stage_0..9 十张
/// （jar MANIFEST 名录），档位映射 `(int)(destroyProgress * 10)`（
/// MultiPlayerGameMode.java:551）。
pub const CRACK_LAYERS: usize = 10;
/// 裂纹 stage s 的数组层号。特殊层统一放在真实贴图之后。
pub const CRACK_BASE: usize = MANIFEST_LAYERS;
/// 图集总层数（= mcv_render::gpu 创建 texture array 的 depth_or_array_layers）。
pub const LAYERS: usize = MANIFEST_LAYERS + CRACK_LAYERS;
pub const MIP_LEVELS: u32 = 2;

const _: () = assert!(MANIFEST_LAYERS + CRACK_LAYERS <= u16::MAX as usize);

/// 原版 missing 贴图像素（MissingTextureAtlasSprite.java:19 `pink = -524040`
/// = ABGR 0xFFF800F8 → RGB (248, 0, 248)；另一分支 `-16777216` = 不透明黑）。
const MISSING_PINK: [u8; 4] = [248, 0, 248, 255];
const MISSING_BLACK: [u8; 4] = [0, 0, 0, 255];

/// [`MissingTextureAtlasSprite.generateMissingImage`]（:13-32）的逐像素
/// 等价：16x16，四象限棋盘 `y < h/2 ^ x < w/2` → 左下/右上品红、其余黑。
/// 这是原版 missing 贴图的显式「缺失标记」（非程序化假贴图）。
fn paint_missing_marker(data: &mut [u8], layer: usize) {
    let base = layer * TILE_PX * TILE_PX * 4;
    for y in 0..TILE_PX as u32 {
        for x in 0..TILE_PX as u32 {
            let px = if (y < TILE_PX as u32 / 2) ^ (x < TILE_PX as u32 / 2) {
                MISSING_PINK
            } else {
                MISSING_BLACK
            };
            let off = base + ((y as usize * TILE_PX + x as usize) * 4);
            data[off..off + 4].copy_from_slice(&px);
        }
    }
}

/// Generates mip level 0 for all layers into `data` (LAYERS * 16*16*4 bytes).
/// 全层先铺原版 missing 贴图（MissingTextureAtlasSprite 语义）；真实贴图由
/// [`load_real_tiles`] 覆盖，裂纹层由 [`load_crack_stages`] 覆盖。哨兵层
/// [`SENTINEL_LAYER`] 无对应文件，永远保持 missing 标记——与原版对无贴图
/// 模型显示 missingno 一致。
pub fn generate_layers(data: &mut [u8]) {
    assert_eq!(data.len(), LAYERS * TILE_PX * TILE_PX * 4);
    for layer in 0..LAYERS {
        paint_missing_marker(data, layer);
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
/// `<assets_dir>/textures/block/*.png`（资源根 = assets/minecraft）读盘
/// 覆盖；缺失/解码失败的层保留原版 missing 标记并 `log::error!`（素材
/// 红线：不产出程序化假贴图）。挖掘裂纹层读原版 destroy_stage_0..9
/// （[`load_crack_stages`]）。
pub fn generate_payload_with_pack(assets_dir: Option<&Path>) -> Vec<u8> {
    let mut mip0 = vec![0u8; LAYERS * TILE_PX * TILE_PX * 4];
    generate_layers(&mut mip0);
    match assets_dir {
        Some(dir) => {
            let n = load_real_tiles(dir, &mut mip0);
            if n < REAL_TILE_COUNT as u32 {
                log::error!(
                    "atlas: {n}/{} real tiles from {}——缺失层将显示原版 missing 标记（无程序化回退）",
                    REAL_TILE_COUNT,
                    dir.display()
                );
            } else {
                log::info!(
                    "atlas: {n}/{} real tiles from {}",
                    REAL_TILE_COUNT,
                    dir.display()
                );
            }
            let c = load_crack_stages(dir, &mut mip0);
            if c < CRACK_LAYERS as u32 {
                log::error!(
                    "atlas: {c}/{} vanilla destroy stages——缺失档位显示 missing 标记",
                    CRACK_LAYERS
                );
            } else {
                log::info!("atlas: {c}/{} vanilla destroy stages", CRACK_LAYERS);
            }
        }
        None => {
            log::warn!("atlas: 无资源根——全部层为原版 missing 标记（仅无头测试模式）");
        }
    }
    let mut mip1 = vec![0u8; LAYERS * 8 * 8 * 4];
    generate_mip1(&mip0, &mut mip1);
    mip0.extend_from_slice(&mip1);
    mip0
}

/// 无资源路径（全部层 = 原版 missing 标记；仅供无头测试）。带真实贴图用
/// [`generate_payload_with_pack`]。
pub fn generate_payload() -> Vec<u8> {
    generate_payload_with_pack(None)
}

/// 层数钳制（GLES `MAX_ARRAY_TEXTURE_LAYERS` 常为 256 < 838）：
/// 钳到 `n` 层时返回实际可用的 mip0+mip1 载荷与数组层数。
/// tiles 引用被钳掉的层时 wgpu 在采样器边界内回绕/钳位（贴图上屏，不崩）。
/// 建议 gpu.rs 用 `min(LAYERS, limits.max_texture_layers())` 调用。
pub fn generate_payload_clamped(assets_dir: Option<&Path>, max_layers: usize) -> (Vec<u8>, usize) {
    let n = max_layers.clamp(1, LAYERS);
    let full = generate_payload_with_pack(assets_dir);
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
const BLOCKS_SUBDIR: &str = "textures/block";
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

/// 从 `<dir>/textures/block/<name>.png`（单一资源根的原版路径）加载
/// 827 张真实方块贴图覆盖层 0..826。缺文件/解码失败该层保留原版
/// missing 标记并计数告警（调用方 `log::error!`，无程序化回退）；
/// missing 哨兵层（[`SENTINEL_LAYER`]）无对应文件，恒跳过。返回覆盖层数。
pub fn load_real_tiles(dir: &Path, layers: &mut [u8]) -> u32 {
    assert!(layers.len() >= MANIFEST_LAYERS * TILE_PX * TILE_PX * 4);
    let blocks = dir.join(BLOCKS_SUBDIR);
    if !blocks.is_dir() {
        return 0;
    }
    let mut count = 0u32;
    for (idx, name) in tile_file_names().iter().enumerate() {
        if idx == SENTINEL_LAYER {
            debug_assert_eq!(*name, SENTINEL_TILE);
            continue; // 哨兵层：无对应文件，保持 missing 标记
        }
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

/// 加载原版挖掘裂纹 10 档到 `CRACK_BASE + s` 层。原版 jar 内路径即
/// `textures/block/destroy_stage_0..9.png`（16x16，黑色裂纹 + alpha），
/// 档位公式 `(int)(destroyProgress * 10)`（MultiPlayerGameMode.java:551）。
/// 缺档该层保留 missing 标记（调用方 `log::error!`，无程序化回退）。
/// 返回覆盖档数。
pub fn load_crack_stages(dir: &Path, layers: &mut [u8]) -> u32 {
    assert!(layers.len() >= LAYERS * TILE_PX * TILE_PX * 4);
    let blocks = dir.join(BLOCKS_SUBDIR);
    if !blocks.is_dir() {
        return 0;
    }
    let mut count = 0u32;
    for s in 0..CRACK_LAYERS {
        let path = blocks.join(format!("destroy_stage_{s}.png"));
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if decode_into_layer(&bytes, &path, CRACK_BASE + s, layers) {
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
/// files replace the real content of exactly that layer.
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
