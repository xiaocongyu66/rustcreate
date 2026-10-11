//! vanilla 气候通道采样器 — clean-room 对齐 26.1 `DensityFunctions.java` 的
//! ShiftedNoise 编排与 `worldgen/noise/*.json` 的噪声参数表。
//!
//! 编排语义（引用机制 ≠ 搬运代码）：
//! - SHIFT（DEFAULT_SHIFT）：firstOctave -3、amplitudes [1,1,1,0]，2D 采样，
//!   求值坐标 ×0.25 且输出 **×4 乘子**（`DensityFunctions.java:1058-1060`
//!   ShiftNoise.compute = `noise(x·0.25, y·0.25, z·0.25)·4`）；
//! - 通道值 = noise.getValue(blockX·0.25 + shiftX, ·, blockZ·0.25 + shiftZ)
//!   （DensityFunctions.ShiftedNoise.compute，xzScale = 0.25）；
//! - continentalness / erosion / ridge(weirdness) 与 temperature / vegetation
//!   共用同一 SHIFT 偏移（NoiseRouterData:89-97/325-331）。
//!
//! 波长真相（地形保真波 T2 还原，任务卡 #122 / triage ①）：xzScale 0.25 ×
//! firstOctave −9 ⇒ 大陆度/侵蚀波长 2048 格、山脊 512、温度 4096（−10 首层）
//! ——原版生存的大地貌尺度。旧「可步行尺度 ÷8 压缩（XS=2.0）」取舍随世界
//! 升 384 作废：÷8 后盆带每 ~256 格穿越一遍，平原窗占比 0.054 vs 原版口径
//! 0.41（/root/audit/_sim3.py 移植复算）。

use crate::vanilla::noise::{NormalNoise, noise_seed, splitmix64};
use crate::vanilla::spline::Climate;

/// 五参数/温湿度通道采样缩放（26.1 overworld 各通道 json `xz_scale: 0.25`，
/// NoiseRouterData.java:90/93/96/325-328）。
const XS: f64 = 0.25;

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
    /// 气候通道全部为 2D 切片采样（sample() 中 y 恒 0；vanilla
    /// ShiftedNoise/ShiftNoise.compute 亦 y·0.25 归零，NoiseRouterData.java:89-97
    /// 与 DensityFunctions.java ShiftNoise 接口）。底层内核为
    /// [`crate::vanilla::perlin::ImprovedNoise`]（原版机制本体，第四棒换代）：
    /// vanilla `DensityFunctions.shiftedNoise2d` 的 yScale=0 即按 (x,0,z) 求值
    /// （DensityFunctions.java:159-163），ImprovedNoise 直接采样无任何校准。
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
        // SHIFT 通道自身：2D 采样（y = 0 切片）+ ShiftNoise.compute 的 ×4 乘子
        //（DensityFunctions.java:1058-1060）。
        let sx = self.shift_x.get_value(bx * XS, 0.0, bz * XS) * 4.0;
        let sz = self.shift_z.get_value(bx * XS, 0.0, bz * XS) * 4.0;
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
            // RIDGES 即 shiftedNoise2d 原始噪声（NoiseRouterData.java:96-98），
            // 无 squeezed/mapped 折叠：p(r)=−3(|‖r‖−2/3|−1/3) 在 |r|≤1 段上界
            // +1（r=±2/3 处），|r|>1 段随原始噪声无界延伸——以通道哨兵
            // |ridges|≤4.0（振幅和 1+2+1）推导折叠值域 [−9, 1]。
            assert!(
                a.ridges_folded <= 1.0 + 1e-9 && a.ridges_folded >= -9.0 - 1e-9,
                "折叠值域 {a:?}"
            );
        }
    }

    /// 分块尺度上气候轴确有变化（≠ 全平）：±4096 格网格（256 格步距）上
    /// continentalness 跨越 0 两侧。窗口随 T2 波长还原同步扩——原 384 格
    /// 路径只有 0.2 个大陆度波长（2048 格），恢复原版尺度后不具代表性。
    #[test]
    fn continentalness_varies_across_region() {
        let s = ClimateSampler::new(7);
        let mut saw_neg = false;
        let mut saw_pos = false;
        for dz in -16..16 {
            for dx in -16..16 {
                let v = s.sample(dx * 256, dz * 256).continents;
                saw_neg |= v < -0.1;
                saw_pos |= v > 0.1;
            }
        }
        assert!(
            saw_neg && saw_pos,
            "continentalness 退化常数（±4096 网格未跨 0）"
        );
    }
}
