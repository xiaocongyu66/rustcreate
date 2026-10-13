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
    gauss_sky(
        &|bx, bz| {
            vanilla::biomes::climate(vanilla::biome_from_sample(&sampler.sample(bx, bz))).sky_color
        },
        cam_x,
        cam_z,
    )
}

/// [`blended_sky_color`] 的纯核（取色函数注入，手推数值用例不需要噪声
/// 世界）：`sky_at` 收方块列坐标（quart 网格 ×4 落位）。
#[must_use]
fn gauss_sky(sky_at: &impl Fn(i32, i32) -> [u8; 3], cam_x: f32, cam_z: f32) -> [f32; 3] {
    // GaussianSampler.sample(position.scale(0.25).subtract(0.5,...))。
    let px = f64::from(cam_x) * 0.25 - 0.5;
    let pz = f64::from(cam_z) * 0.25 - 0.5;
    let (ix, iz) = (px.floor(), pz.floor());
    let (rx, rz) = (px - ix, pz - iz);
    let mut sum = [0.0f64; 3];
    let mut total = 0.0f64;
    // 权重口径 Mth.lerp(rel, K[i+1], K[i]) = K[i+1]·(1−rel) + K[i]·rel
    // （GaussianSampler.java:21/26——rel=0 时峰值落 K[i+1]=6 即
    // sampleX=integralX 本位，方向抄反会整体错一格）。
    for zz in 0..6 {
        let wz = (GAUSS_KERNEL[zz + 1] * (1.0 - rz) + GAUSS_KERNEL[zz] * rz).max(0.0);
        if wz == 0.0 {
            continue;
        }
        // quart → 方块列（ClimateSampler.sample 收方块坐标，内部 ×0.25
        // 即 quart 单位求值，climate.rs XS 注）。
        let bz = ((iz - 2.0) + zz as f64) * 4.0;
        for xx in 0..6 {
            let wx = (GAUSS_KERNEL[xx + 1] * (1.0 - rx) + GAUSS_KERNEL[xx] * rx).max(0.0);
            if wx == 0.0 {
                continue;
            }
            let bx = ((ix - 2.0) + xx as f64) * 4.0;
            let sky = sky_at(bx as i32, bz as i32);
            let w = wx * wz;
            for ch in 0..3 {
                sum[ch] += w * f64::from(sky[ch]);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 核常量与实物逐值一致。
    #[test]
    fn kernel_matches_vanilla() {
        assert_eq!(GAUSS_KERNEL, [0.0, 1.0, 4.0, 6.0, 4.0, 1.0, 0.0]);
    }

    /// 同质世界：任意相机 → 恒等该群系 sky_color（黄金断言取 26.1
    /// plains JSON 抄录值）。
    #[test]
    fn gauss_sky_constant_field_is_identity() {
        let plains_sky = vanilla::biomes::climate(vanilla::biomes::Biome::Plains).sky_color;
        let f = |_: i32, _: i32| plains_sky;
        for (cx, cz) in [(0.0, 0.0), (2.0, 17.0), (-100.5, 3.25)] {
            let got = gauss_sky(&f, cx, cz);
            for ch in 0..3 {
                assert!(
                    (f64::from(got[ch]) - f64::from(plains_sky[ch]) / 255.0).abs() < 1e-5,
                    "ch{ch}: {got:?} vs {plains_sky:?}"
                );
            }
        }
    }

    /// 手推数值：相机落 quart 本位（px 整点，rel=0 → 权重 [1,4,6,4,1,0]），
    /// x<0 黑、x≥0 白。A 权重 = (1+4)×Σz=16 → 80；B = (6+4+1+0)×16 →
    /// 176；总 256 → 期望 176/256×255 = 175.3125（/255 后 ≈0.687500）。
    #[test]
    fn gauss_sky_step_field_hand_computation() {
        let f = |bx: i32, _: i32| {
            if bx < 0 {
                [0u8, 0, 0]
            } else {
                [255u8, 255, 255]
            }
        };
        // cam=2 → px = 2×0.25 − 0.5 = 0（rel=0 本位）。
        let got = gauss_sky(&f, 2.0, 2.0);
        let expect = 176.0f64 / 256.0;
        for ch in 0..3 {
            assert!(
                (f64::from(got[ch]) - expect).abs() < 1e-4,
                "ch{ch}: {} vs {expect}",
                got[ch]
            );
        }
        // 镜像对称：同一世界相机 −2（px=−1 本位，A/B 权重互换）。
        let mirrored = gauss_sky(&f, -2.0, 2.0);
        for ch in 0..3 {
            assert!((f64::from(mirrored[ch]) - (1.0 - expect)).abs() < 1e-4);
        }
    }

    /// 真实噪声世界单群系窗：6×6 窗群系全同 → 天空色 = 该群系表值
    /// （实物公式 + 抄录参数的交叉一致性）。
    #[test]
    fn blended_sky_matches_single_biome_window() {
        let sampler = ClimateSampler::new(2026);
        for &(cx, cz) in &[(8.0, 8.0), (60.0, -40.0), (-250.0, 111.0)] {
            // 窗内群系一致性检查（与 gauss_sky 同坐标口径）。
            let px = f64::from(cx) * 0.25 - 0.5;
            let pz = f64::from(cz) * 0.25 - 0.5;
            let (ix, iz) = (px.floor(), pz.floor());
            let mut biome = None;
            let mut uniform = true;
            for zz in 0..6 {
                for xx in 0..6 {
                    let bx = (((ix - 2.0) + xx as f64) * 4.0) as i32;
                    let bz = (((iz - 2.0) + zz as f64) * 4.0) as i32;
                    let b = vanilla::biome_from_sample(&sampler.sample(bx, bz));
                    match biome {
                        None => biome = Some(b),
                        Some(b0) if b0 == b => {}
                        _ => uniform = false,
                    }
                }
            }
            if uniform {
                let want = vanilla::biomes::climate(biome.unwrap()).sky_color;
                let got = blended_sky_color(2026, cx, cz);
                for ch in 0..3 {
                    assert!(
                        (f64::from(got[ch]) - f64::from(want[ch]) / 255.0).abs() < 1e-4,
                        "单群系窗天空色 ch{ch}: {got:?} vs {want:?} @({cx},{cz})"
                    );
                }
            }
        }
    }
}
