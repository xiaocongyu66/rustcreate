//! #191 群系 tint 装配（游戏层，任务板 #191 消费面接线）。
//!
//! 两条独立实物通路（26.1 反编译树只读实测，file:line 注）：
//!
//! 1. **方块顶点色**（[`BiomeTintSource::field_for`]）：原版灰度遮罩贴图
//!    × 群系色，逐列色 = colormap 查色（`Biome.getGrassColor/
//!    getFoliageColor`，Biome.java:202-226，温度×湿度 0..1 钳位查
//!    256×256 colormap），逐顶点色 = `ClientLevel.calculateBlockTint`
//!    盒均值（ClientLevel.java:876-899，`biomeBlendRadius` 默认 2 →
//!    5×5 先解色后整数平均）。混合窗按**世界坐标**取列——相邻区块独立
//!    构网取同一批世界列，跨区块边界零接缝（26.1 的
//!    `client/color/block/Blender.java` 不存在；4×4 quart 双线性是
//!    ≤1.18 旧语义，随 1.18 群系常数化作废，本模块按 26.1 实物）。
//! 2. **天空底色**（[`blended_sky_color`]）：26.1 天穹/雾 SKY 分量 =
//!    相机处 `EnvironmentAttributes.SKY_COLOR`
//!    （EnvironmentAttributeProbe.java:30-34）——空间插值 =
//!    `GaussianSampler`（GaussianSampler.java:8-37）：位置 ×1/4（quart
//!    间距）−0.5，6×6×6 窗，核 {0,1,4,6,4,1,0} 亚格线性插值加权。本仓
//!    气候通道无垂直维度（2D 切片，climate.rs 头注），y 维权重恒等
//!    归一抵消 → 6×6 x/z 窗等价。属性值 = 26.1 biome JSON
//!    `attributes."minecraft:visual/sky_color"` 实测
//!    （`mcv_worldgen::vanilla::biomes::climate` 表）。

use std::sync::Arc;

use mcv_core::tint::{self, ChunkTint};
use mcv_worldgen::vanilla::{self, climate::ClimateSampler};

/// colormap 已解码色场持有者（256×256 RGBA，[`mcv_core::tint::
/// decode_colormap`] 产物；装载方 = mcv_app（素材归壳层），注入 =
/// [`crate::game::GameRuntime::set_biome_tint`]）。
///
/// 世界种子随注入封存（网格采样需要），采样本身零状态可并发。
#[derive(Clone)]
pub struct BiomeTintSource {
    seed: u64,
    grass_px: Arc<[u8]>,
    foliage_px: Arc<[u8]>,
}

impl BiomeTintSource {
    /// `*_px` = 解码后的 256×256 RGBA 字节（任一缺失/尺寸不符 → None，
    /// 调用方保持染色禁用，不伪造颜色）。
    #[must_use]
    pub fn new(seed: u64, grass_px: Vec<u8>, foliage_px: Vec<u8>) -> Option<Self> {
        let ok = |px: &Vec<u8>| px.len() == 256 * 256 * 4;
        if !ok(&grass_px) || !ok(&foliage_px) {
            return None;
        }
        Some(Self {
            seed,
            grass_px: Arc::from(grass_px.into_boxed_slice()),
            foliage_px: Arc::from(foliage_px.into_boxed_slice()),
        })
    }

    /// 中心区块（chunk 坐标）的顶点色场：网格覆盖世界列
    /// `[origin−R, origin+16+R]`（含两端，w = 16+2R+1；R =
    /// [`tint::BLEND_RADIUS`]），场原点 = `origin−R`——与
    /// [`ChunkTint`] / [`mcv_mesher::TintInput`] 的契约同源（顶点角
    /// 0..=16 的全宽混合窗恒在网格内）。
    #[must_use]
    pub fn field_for(&self, chunk_x: i32, chunk_z: i32) -> Option<ChunkTint> {
        let r = tint::BLEND_RADIUS;
        let w = (16 + 2 * r + 1) as usize;
        let ox = chunk_x * 16 - r;
        let oz = chunk_z * 16 - r;
        let cells = vanilla::biome_climate_grid(self.seed, ox, oz, w);
        ChunkTint::from_climate(ox, oz, &cells, &self.grass_px, &self.foliage_px)
    }
}

/// GaussianSampler 核（GaussianSampler.java:10 `{0,1,4,6,4,1,0}`，
/// 6 抽样位取相邻核值按亚格 lerp）。
const GAUSS_KERNEL: [f64; 7] = [0.0, 1.0, 4.0, 6.0, 4.0, 1.0, 0.0];

/// 相机处群系天空底色（sRGB 0..1）：`GaussianSampler` 语义 6×6 quart
/// 窗加权平均（见模块头注；y 维在 2D 气候下归一抵消）。
///
/// 加权平均口径：`SpatialAttributeInterpolator` 按权重链式 lerp
/// （SpatialAttributeInterpolator.java:33-50），ARGB 通道的 lerp 为逐
/// 通道线性 → 链式加权 lerp ≡ 归一加权平均（与遍历序无关）。
#[must_use]
pub fn blended_sky_color(world_seed: u64, cam_x: f32, cam_z: f32) -> [f32; 3] {
    let sampler = ClimateSampler::new(world_seed);
    // GaussianSampler.sample(position.scale(0.25).subtract(0.5,...))。
    let px = f64::from(cam_x) * 0.25 - 0.5;
    let pz = f64::from(cam_z) * 0.25 - 0.5;
    let (ix, iz) = (px.floor(), pz.floor());
    let (rx, rz) = (px - ix, pz - iz);
    let mut sum = [0.0f64; 3];
    let mut total = 0.0f64;
    for zz in 0..6 {
        let wz = (GAUSS_KERNEL[zz] * (1.0 - rz) + GAUSS_KERNEL[zz + 1] * rz).max(0.0);
        if wz == 0.0 {
            continue;
        }
        // quart → 方块列（ClimateSampler.sample 收方块坐标，内部 ×0.25
        // 即 quart 单位求值，climate.rs XS 注）。
        let bz = ((iz - 2.0) + zz as f64) * 4.0;
        for xx in 0..6 {
            let wx = (GAUSS_KERNEL[xx] * (1.0 - rx) + GAUSS_KERNEL[xx + 1] * rx).max(0.0);
            if wx == 0.0 {
                continue;
            }
            let bx = ((ix - 2.0) + xx as f64) * 4.0;
            let b = vanilla::biome_from_sample(&sampler.sample(bx as i32, bz as i32));
            let w = wx * wz;
            for ch in 0..3 {
                sum[ch] += w * f64::from(vanilla::biomes::climate(b).sky_color[ch]);
            }
            total += w;
        }
    }
    if total == 0.0 {
        // 核权重全 0 只在 NaN 相机坐标出现——回退 overworld 默认天底色
        // （26.1 SKY_COLOR 属性默认值 0x78a7ff）。
        return [0x78 as f32 / 255.0, 0xa7 as f32 / 255.0, 1.0];
    }
    [
        (sum[0] / total) as f32 / 255.0,
        (sum[1] / total) as f32 / 255.0,
        (sum[2] / total) as f32 / 255.0,
    ]
}
