//! vanilla 气候通道采样器 — clean-room 对齐 26.1 `DensityFunctions.java` 的
//! ShiftedNoise 编排与 `worldgen/noise/*.json` 的噪声参数表。
//!
//! 编排语义（引用机制 ≠ 搬运代码）：
//! - SHIFT（DEFAULT_SHIFT）：firstOctave -3、amplitudes [1,1,1,0]，2D 采样；
//! - 通道值 = noise.getValue(blockX·xzScale + shiftX, ·, blockZ·xzScale + shiftZ)
//!   （DensityFunctions.ShiftedNoise.compute）；
//! - continentalness / erosion / ridge(weirdness) 与 temperature / vegetation
//!   共用同一 SHIFT 偏移（NoiseRouterData:89-97/325-331）。
//!
//! 波长取舍（NOTES-terrain §7，防小区域地形全平）：26.1 原生大陆度/侵蚀
//! 波长 2048 格（xzScale 0.25）、山脊 512（0.25:512 保持 4:1）。本世界可
//! 步行尺度上限 256 格：一律 ÷8 压缩且保持通道间相对比例
//! （2048:2048:512 → 256:256:64，xzScale 2.0），同 seed 确定性测试锁定。

use crate::vanilla::noise::{noise_seed, splitmix64, NormalNoise};
use crate::vanilla::spline::Climate;

/// 温度/湿度通道 2D 压缩后的采样缩放（= 0.25 ÷8）。
const XS: f64 = 2.0;

/// 每列气候快照（供样条求值与群系选择）。
#[derive(Clone, Copy, Debug)]
pub struct ClimateSample {
    pub continents: f64,
    pub erosion: f64,
    /// weirdness（未折叠）。
    pub ridges: f64,
    pub ridges_folded: f64,
    pub temperature: f64,
    pub vegetation: f64,
}

impl ClimateSample {
    /// 供 density 样条求值的输入（只含三轴气候）。
    #[must_use]
    pub const fn spline_climate(&self) -> Climate {
        Climate {
            continents: self.continents,
            erosion: self.erosion,
            ridges: self.ridges,
            ridges_folded: self.ridges_folded,
        }
    }
}

pub struct ClimateSampler {
    shift_x: NormalNoise,
    shift_z: NormalNoise,
    continentalness: NormalNoise,
    erosion: NormalNoise,
    ridge: NormalNoise,
    temperature: NormalNoise,
    vegetation: NormalNoise,
}

impl ClimateSampler {
    #[must_use]
    pub fn new(world_seed: u64) -> Self {
        Self {
            shift_x: NormalNoise::new(
                world_seed,
                salt(world_seed, 0x5A11),
                -3,
                &[1.0, 1.0, 1.0, 0.0],
            ),
            shift_z: NormalNoise::new(
                world_seed,
                salt(world_seed, 0x5A22),
                -3,
                &[1.0, 1.0, 1.0, 0.0],
            ),
            continentalness: NormalNoise::new(
                world_seed,
                salt(world_seed, 0x4F11),
                -9,
                &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0],
            ),
            erosion: NormalNoise::new(
                world_seed,
                salt(world_seed, 0x0B55),
                -9,
                &[1.0, 1.0, 0.0, 1.0, 1.0],
            ),
            ridge: NormalNoise::new(world_seed, salt(world_seed, 0x3D11), -7, &[1.0, 2.0, 1.0]),
            temperature: NormalNoise::new(
                world_seed,
                salt(world_seed, 0x7A11),
                -10,
                &[1.5, 0.0, 1.0],
            ),
            vegetation: NormalNoise::new(
                world_seed,
                salt(world_seed, 0x3B33),
                -8,
                &[1.0, 1.0, 0.0],
            ),
        }
    }

    /// 采样一列气候（ShiftedNoise 式：先算 SHIFT 偏移，再查通道值）。
    #[must_use]
    pub fn sample(&self, block_x: i32, block_z: i32) -> ClimateSample {
        let bx = f64::from(block_x);
        let bz = f64::from(block_z);
        // SHIFT 通道自身：2D 采样（y = 0 切片）
        let sx = self.shift_x.get_value(bx * XS, 0.0, bz * XS);
        let sz = self.shift_z.get_value(bx * XS, 0.0, bz * XS);
        let continents = self
            .continentalness
            .get_value(bx * XS + sx, 0.0, bz * XS + sz);
        let erosion = self.erosion.get_value(bx * XS + sx, 0.0, bz * XS + sz);
        let ridges = self.ridge.get_value(bx * XS + sx, 0.0, bz * XS + sz);
        let temperature = self.temperature.get_value(bx * XS + sx, 0.0, bz * XS + sz);
        let vegetation = self.vegetation.get_value(bx * XS + sx, 0.0, bz * XS + sz);
        ClimateSample {
            continents,
            erosion,
            ridges,
            ridges_folded: crate::vanilla::peaks_and_valleys(ridges),
            temperature,
            vegetation,
        }
    }
}

/// 通道域盐：域码 + 全链 splitmix（自有派生，防通道同种子）。
const fn salt(world_seed: u64, domain: u64) -> u64 {
    let seed = noise_seed(world_seed, domain);
    splitmix64(seed ^ domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 采样确定性 + 不同列不同值 + 值域哨兵（归一通道近似 [-1,1]）。
    #[test]
    fn climate_samples_deterministic_and_bounded() {
        let s = ClimateSampler::new(2024);
        for k in 0..32_i32 {
            let a = s.sample(k * 13, k * 7);
            let b = s.sample(k * 13, k * 7);
            assert_eq!(a.continents.to_bits(), b.continents.to_bits());
            assert_eq!(a.ridges_folded.to_bits(), b.ridges_folded.to_bits());
            for v in [
                a.continents,
                a.erosion,
                a.ridges,
                a.temperature,
                a.vegetation,
            ] {
                assert!(v.abs() <= 4.0, "气候通道越界 {v}");
            }
            let folded = crate::vanilla::peaks_and_valleys(a.ridges);
            assert!((a.ridges_folded - folded).abs() < 1e-12);
            assert!(a.ridges_folded.abs() <= 1.0 + 1e-9, "折叠值域 {a:?}");
        }
    }

    /// 分块尺度上气候轴确有变化（≠ 全平）：256 格窗口内 continentalness
    /// 跨越 0 两侧。
    #[test]
    fn continentalness_varies_across_region() {
        let s = ClimateSampler::new(7);
        let mut saw_neg = false;
        let mut saw_pos = false;
        for k in 0..24_i32 {
            let v = s.sample(k * 16, k * 31).continents;
            if v < -0.1 {
                saw_neg = true;
            }
            if v > 0.1 {
                saw_pos = true;
            }
        }
        assert!(saw_neg && saw_pos, "continentalness 退化常数（海陆不可分）");
    }
}
