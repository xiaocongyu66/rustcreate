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
//! - 平滑混合实物 [`net/minecraft/client/multiplayer/ClientLevel.java:876-899`]
//!   `calculateBlockTint`：以目标方块为中心、半径 `biomeBlendRadius`
//!   （Options.java:438-441 默认 **2**，范围 0..7）的 **(2r+1)²盒均值**——
//!   逐采样点解出该列群系色后整数求和除以 count（截断除法）。注意：26.1
//!   的 `Blender.java` 已无 getBlendData/getBlankColor（那是世界生成期
//!   blending；旧「4×4 quart 双线性」语义在 1.17/1.18 群系颜色改为
//!   群系常数 + 盒均值后作废）——本模块按 26.1 实物做盒均值。
//! - 数据通路（#191，替换旧「全局常数基线」）：群系气候表
//!   （`mcv_worldgen::vanilla::biomes::climate`，26.1 biome JSON 实物数值）
//!   → [`ChunkTint`]：网格器按**世界坐标**对
//!   中心区块 ±margin 逐列解析 colormap 色（[`ChunkTint::from_climate`]），
//!   每个**顶点**按其世界坐标所在方块列取 [`ChunkTint::color_at`] 盒均值
//!   → 写进顶点色属性（terrain.wgsl 顶点色 × 贴图，等价原版
//!   「tintSource → BlockColors → 顶点色」链路）。跨区块顶点采样同一张
//!   世界坐标函数（两相邻区块各自独立构造网格，重叠带取色逐列一致）→
//!   无接缝。grass_block_top 顶面层染色、grass_block_side 本体不染色
//!   （侧 overlay 层染色），与原版一致。

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

/// tint 混合半径（Options.java:438-441 `biomeBlendRadius` 默认 2 →
/// (2×2+1)²=25 采样盒均值，ClientLevel.calculateBlockTint:876-899）。
pub const BLEND_RADIUS: i32 = 2;

/// 不染色默认顶点色（乘算单位元）。
pub const TINT_WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];

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

/// 贴图名 → tint 类别（BlockColors.java:20-45 注册表；常量族取色由消费
/// 方用 [`SPRUCE_LEAF_RGB`]/[`BIRCH_LEAF_RGB`]）。
fn kind_of(name: &str) -> u32 {
    if GRASS_TILES.contains(&name) {
        TINT_GRASS
    } else if FOLIAGE_TILES.contains(&name) {
        TINT_FOLIAGE
    } else if name == "spruce_leaves" {
        TINT_SPRUCE
    } else if name == "birch_leaves" {
        TINT_BIRCH
    } else {
        TINT_NONE
    }
}

/// tile 层号 → tint 类别。层号超出 manifest（裂纹等特殊层）一律 [`TINT_NONE`]。
pub fn tint_kind(layer: u16) -> u32 {
    tile_file_names()
        .get(layer as usize)
        .map_or(TINT_NONE, |name| kind_of(name))
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

/// 中心区块网格化 tint 场（#191；构建方 = mcv_logic 网格派发点，消费方 =
/// mcv_mesher 逐顶点取色）。
///
/// 覆盖范围：中心区块 [0,16]²（顶点角坐标）外扩 [`BLEND_RADIUS`] 边距的
/// 逐列色场，坐标一律**世界坐标**（两相邻区块独立构造的网格在重叠带
/// 对同一世界列解出同一群系色 → 边界顶点两侧取色逐位一致，无接缝）。
/// 每列颜色 = 该列群系 (temperature, downfall) 的 colormap 查色（原版
/// 每采样点独立解 biome 再取色，ClientLevel.calculateBlockTint:891）；
/// 顶点色 = [`ChunkTint::color_at`] 盒均值（:885-898 的整数求和/截断除法
/// 同款——先解色后平均，不是先平均气候再查表，两者在 colormap 非线性下
/// 不等价，按实物走前者）。
#[derive(Clone, Debug)]
pub struct ChunkTint {
    /// 网格世界原点（x/z 向列号 = ox + index，y 无关——26.1 tint 无垂直
    /// 维度，getGrassColor/getFoliageColor 均只读群系常数）。
    pub ox: i32,
    pub oz: i32,
    /// 边长（列数，cells = w×w 行主序）。
    pub w: usize,
    grass_col: Vec<[u8; 3]>,
    foliage_col: Vec<[u8; 3]>,
}

impl ChunkTint {
    /// 由逐列气候参数解析色场。`cells` 长度必须为完全平方数（w×w），
    /// colormap 任一缺失返回 None（调用方关闭染色，不伪造颜色——沿用
    /// 旧「素材缺失禁用」红线）。
    #[must_use]
    pub fn from_climate(
        ox: i32,
        oz: i32,
        cells: &[[f64; 2]],
        grass_px: &[u8],
        foliage_px: &[u8],
    ) -> Option<Self> {
        let w = (cells.len() as f64).sqrt().round() as usize;
        if w * w != cells.len() || w == 0 {
            return None;
        }
        let mut grass_col = Vec::with_capacity(cells.len());
        let mut foliage_col = Vec::with_capacity(cells.len());
        for [t, d] in cells {
            let g = colormap_get(grass_px, *t, *d)?;
            let f = colormap_get(foliage_px, *t, *d)?;
            grass_col.push([g[0], g[1], g[2]]);
            foliage_col.push([f[0], f[1], f[2]]);
        }
        Some(Self {
            ox,
            oz,
            w,
            grass_col,
            foliage_col,
        })
    }

    /// 世界方块列 (bx, bz) 的混合色：`calculateBlockTint` 盒均值原式——
    /// `for d in -r..=r`² 求采样色整数和、除以 count（截断）。越出本网格
    /// 的采样列钳到边缘列（生产路径中心区块顶点 + 半径窗恒在网格内，
    /// 钳位只是形状亚格/边界外采样的安全网；原版采样无限世界，钳位
    /// 与真实值的偏差只出现在网格覆盖范围之外，不影响中心区块一致性）。
    #[must_use]
    pub fn color_at(&self, kind: u32, bx: i32, bz: i32) -> [u8; 3] {
        let cols = match kind {
            TINT_GRASS => &self.grass_col[..],
            TINT_FOLIAGE => &self.foliage_col[..],
            _ => return TINT_WHITE,
        };
        let mut sum = [0u64; 3];
        let mut count = 0u64;
        for dz in -BLEND_RADIUS..=BLEND_RADIUS {
            for dx in -BLEND_RADIUS..=BLEND_RADIUS {
                let cx = (bx + dx - self.ox).clamp(0, self.w as i32 - 1) as usize;
                let cz = (bz + dz - self.oz).clamp(0, self.w as i32 - 1) as usize;
                let c = cols[cz * self.w + cx];
                for ch in 0..3 {
                    sum[ch] += u64::from(c[ch]);
                }
                count += 1;
            }
        }
        [
            (sum[0] / count) as u8,
            (sum[1] / count) as u8,
            (sum[2] / count) as u8,
        ]
    }
}
