//! 生物群系染色（tint）：MC 26.1 原版染色机制的数据化等价实现。
//!
//! 原版依据（反编译源码核对，勿改数值）：
//! - 注册表 [`net/minecraft/client/color/block/BlockColors.java:20-45`]
//!   `createDefault()`：草族（GRASS_BLOCK / FERN / SHORT_GRASS / BUSH /
//!   TALL_GRASS / LARGE_FERN / SUGAR_CANE / PINK_PETALS…）→
//!   [`BlockTintSources.grass()`]；橡木/丛林/金合欢/深色橡木/红树叶与
//!   藤蔓 → `foliage()`；云杉叶 → 常量 `FOLIAGE_EVERGREEN`；白桦叶 →
//!   常量 `FOLIAGE_BIRCH`。樱桃/杜鹃/苍白橡树叶**不注册**（贴图自带
//!   颜色），干草族（short_dry_grass 等）在 26.1 同样不注册。
//! - 查表公式 [`net/minecraft/world/level/ColorMapColorUtil.java:3-9`]
//!   `get(temp, rain)`：`rain *= temp; x = (1-temp)*255; y = (1-rain)*255;
//!   index = y<<8 | x`，256x256 colormap 贴图像素直查（越界才回退默认色，
//!   clamp 后恒在界内）。temp/rain 由 [`Biome.java:212-216`] 在生物群系
//!   侧 clamp 到 0..1。
//! - colormap 素材：`textures/colormap/{grass,foliage}.png`（256x256 RGB，
//!   仓库 assets/ 内与 26.1.jar 逐字节一致；原版 GrassColorReloadListener
//!   运行时加载同一张图）。
//! - 本引擎世界核无生物群系参数（cpp/src/terrain.cpp 单气候全域），基线
//!   取原版 plains：`data/minecraft/worldgen/biome/plains.json`
//!   `temperature 0.8 / downfall 0.4` → 草 #91BD59、叶 #77AB2F（已用
//!   仓内 colormap PNG 实测校验）。
//!
//! 数据通路：[`tint_lut_bytes`] 产出「层号 → 类别」查找表上传 GPU
//! （terrain.wgsl binding 4），片元按 tile 层查 kind 后乘对应生物群系
//! 颜色——等价于原版「模型 tintindex → BlockColors 注册表 → 顶点色」链路
//! （本引擎贴图按层组织，故按层查表；grass_block_top 顶面层染色、侧面
//! grass_block_side 不染色，与原版一致）。

use crate::atlas::tile_file_names;

/// tint 类别常量（LUT kind 通道，terrain.wgsl 按 kind 分支取色）。
pub const TINT_NONE: u32 = 0;
/// 草族：colormap grass.png 查表（BlockTintSources.grass()）。
pub const TINT_GRASS: u32 = 1;
/// 叶族：colormap foliage.png 查表（BlockTintSources.foliage()）。
pub const TINT_FOLIAGE: u32 = 2;
/// 云杉叶：常量色（BlockColors.java:26，FOLIAGE_EVERGREEN）。
pub const TINT_SPRUCE: u32 = 3;
/// 白桦叶：常量色（BlockColors.java:27，FOLIAGE_BIRCH）。
pub const TINT_BIRCH: u32 = 4;

/// 云杉叶常量色 FOLIAGE_EVERGREEN = -10380959 = 0x619961（FoliageColor.java:6）。
pub const SPRUCE_LEAF_RGB: [u8; 3] = [0x61, 0x99, 0x61];
/// 白桦叶常量色 FOLIAGE_BIRCH = -8345771 = 0x80A755（FoliageColor.java:7）。
pub const BIRCH_LEAF_RGB: [u8; 3] = [0x80, 0xA7, 0x55];

/// 世界基线气候 = 原版 plains（26.1 data/minecraft/worldgen/biome/
/// plains.json:200 `"temperature": 0.8`；:10 `"downfall": 0.4`）。
pub const WORLD_TEMP: f64 = 0.8;
pub const WORLD_DOWNFALL: f64 = 0.4;

/// 草族 tile（manifest 贴图名 → grass colormap，BlockColors.java:22-24/41）。
const GRASS_TILES: &[&str] = &[
    "grass_block_top",          // grassBlock()：草方块顶面（:24）
    "grass_block_side_overlay", // 草方块侧 overlay（原版模型 tintindex 同源）
    "fern",                     // grass()（:23）
    "short_grass",
    "bush",
    "tall_grass_bottom", // doubleTallGrass()（:22；高草/大蕨共用底部贴图层）
    "large_fern_bottom",
    "sugar_cane", // sugarCane()（:41）
];

/// 叶族 tile（manifest 贴图名 → foliage colormap，BlockColors.java:29-35）。
const FOLIAGE_TILES: &[&str] = &[
    "oak_leaves",
    "jungle_leaves",
    "acacia_leaves",
    "dark_oak_leaves",
    "mangrove_leaves",
    "vine",
];

/// 贴图名 → tint 类别（BlockColors.java:20-45 注册表）。
fn kind_of(name: &str) -> (u32, [u8; 3]) {
    if GRASS_TILES.contains(&name) {
        (TINT_GRASS, [0, 0, 0])
    } else if FOLIAGE_TILES.contains(&name) {
        (TINT_FOLIAGE, [0, 0, 0])
    } else if name == "spruce_leaves" {
        (TINT_SPRUCE, SPRUCE_LEAF_RGB)
    } else if name == "birch_leaves" {
        (TINT_BIRCH, BIRCH_LEAF_RGB)
    } else {
        (TINT_NONE, [0, 0, 0])
    }
}

/// tile 层号 → tint 类别。层号超出 manifest（裂纹等特殊层）一律 [`TINT_NONE`]。
pub fn tint_kind(layer: u16) -> u32 {
    tile_file_names()
        .get(layer as usize)
        .map_or(TINT_NONE, |name| kind_of(name).0)
}

/// 解码 256x256 colormap PNG → 行主序 RGBA 字节（不足/尺寸不符返回 None）。
pub fn decode_colormap(png: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(png).ok()?;
    let rgba = img.to_rgba8();
    (rgba.width() == 256 && rgba.height() == 256).then(|| rgba.into_raw())
}

/// `ColorMapColorUtil.get`：256x256 RGBA 像素表按 (temp, rain) 查色。
/// 输入沿用原版在生物群系侧 clamp 到 0..1 的约定（Biome.java:212-216）；
/// `rain *= temp`、`(int)` 截断与原版逐句对应。表缺失返回 None。
pub fn colormap_get(pixels: &[u8], temp: f64, rain: f64) -> Option<[u8; 4]> {
    if pixels.len() < 256 * 256 * 4 {
        return None;
    }
    let temp = temp.clamp(0.0, 1.0);
    let rain = (rain * temp).clamp(0.0, 1.0);
    let x = ((1.0 - temp) * 255.0) as usize;
    let y = ((1.0 - rain) * 255.0) as usize;
    let o = (y << 8 | x) * 4;
    Some([pixels[o], pixels[o + 1], pixels[o + 2], pixels[o + 3]])
}

/// plains 基线草/叶色（RGB 0..1）。colormap 任一缺失返回 None（调用方
/// 关闭染色，不伪造颜色）。
pub fn world_grass_foliage_color(
    grass_px: &[u8],
    foliage_px: &[u8],
) -> Option<([f32; 3], [f32; 3])> {
    let g = colormap_get(grass_px, WORLD_TEMP, WORLD_DOWNFALL)?;
    let f = colormap_get(foliage_px, WORLD_TEMP, WORLD_DOWNFALL)?;
    let rgb = |c: [u8; 4]| {
        [
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        ]
    };
    Some((rgb(g), rgb(f)))
}

/// GPU 侧「层号 → tint 类别」查找表：每层 16 字节 = `vec4<u32>(kind, r,
/// g, b)`（WGSL uniform 数组 std140 stride 16）。常量叶色（云杉/白桦）
/// 直接烤进 LUT；colormap 族（草/叶）只给 kind，颜色由 FrameUniforms
/// 按 plains 基线上传。裂纹等特殊层（CRACK_BASE..）保持 0 = 不染色。
pub fn tint_lut_bytes() -> Vec<u8> {
    let names = tile_file_names();
    let mut out = vec![0u8; crate::atlas::LAYERS * 16];
    for (i, name) in names.iter().enumerate() {
        let (kind, rgb) = kind_of(name);
        if kind == TINT_NONE {
            continue;
        }
        let o = i * 16;
        out[o..o + 4].copy_from_slice(&kind.to_le_bytes());
        for (c, v) in rgb.iter().enumerate() {
            out[o + 4 + c * 4..o + 8 + c * 4].copy_from_slice(&(u32::from(*v)).to_le_bytes());
        }
    }
    out
}
