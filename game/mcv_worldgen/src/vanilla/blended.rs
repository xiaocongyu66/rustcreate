//! vanilla base_3d_noise — clean-room 移植 26.1 `synth/BlendedNoise.java`
//! 的三层限位混合结构（地形保真波 T4，triage ⑪：旧 fork 自建
//! `NormalNoise(-7,[1,1,1]) @ (xz·2, y·0.75, z·2)` σ≈0.26-0.34、λ 16-64 格，
//! ≈10× 原版地表抖动 = 「坑坑洼洼小土包」的直接生产者）。
//!
//! 机制出处（26.1 反编译树 /root/mc-ref/src-26.1，引用机制与常数 ≠ 搬运代码）：
//! - 注册：`NoiseRouterData.java:87`
//!   `BASE_3D_NOISE_OVERWORLD = BlendedNoise.createUnseeded(0.25, 0.125, 80, 160, 8)`
//!   （old_blended_noise(xz_scale 0.25, y_scale 0.125, xz_factor 80, y_factor 160,
//!   smear_scale_multiplier 8)）；
//! - 乘子：`BlendedNoise.java:64-65` xzMultiplier = 684.412·xzScale = 171.103、
//!   yMultiplier = 684.412·yScale = 85.5515（684.412 = 原版经典噪声尺度常数）；
//! - 三栈倍频：min/max 限位栈 16 倍频（`createLegacyForBlendedNoise`，
//!   `IntStream.rangeClosed(-15, 0)`）、main 选择器栈 8 倍频（rangeClosed(-7, 0)）；
//!   `compute`（:105-148）逐倍频坐标 ×2^-i、权重 2^i（低频主导，最高频倍频
//!   权重 →0），输出 = `clampedLerp(factor, blendMin/512, blendMax/512)/128`；
//!   factor = (mainNoiseValue/10 + 1)/2，factor ≥1 只取 max 栈、≤0 只取 min 栈。
//! - 地表近旁统计（移植复算 /root/audit/_sim3.py）：σ≈0.13、波长 ≥60 格的
//!   光滑起伏、相邻块 |Δ| p90 ≈0.02。
//!
//! 既定偏差（clean-room 取舍，同 [`crate::vanilla::noise`] 的 2D 切片先例）：
//! 5 参 `ImprovedNoise.noise(x,y,z,yScale,yFudge)` 的纵向 smear（y 分数坐标
//! 量化）不移植——OS2S 内核无梯度内插层可注入；该机制只影响 6 个最高频限位
//! 倍频的纵向微结构，水平地表谱不变（复算器对照验证）。

use crate::vanilla::noise::noise_seed;
use crate::vanilla::noise::splitmix64;

/// xzMultiplier = 684.412·0.25（BlendedNoise.java:64）。
const XZ_MULT: f64 = 171.103;
/// yMultiplier = 684.412·0.125（BlendedNoise.java:65）。
const Y_MULT: f64 = 85.5515;
/// xzFactor（base_3d_noise.json / NoiseRouterData.java:87）。
const XZ_FACTOR: f64 = 80.0;
/// yFactor（同上）。
const Y_FACTOR: f64 = 160.0;
/// min/max 限位栈倍频数（rangeClosed(-15, 0)）。
const LIMIT_OCTAVES: usize = 16;
/// main 选择器栈倍频数（rangeClosed(-7, 0)）。
const MAIN_OCTAVES: usize = 8;
/// 倍频种子步进（同 [`crate::vanilla::noise::OCTAVE_SEED_STEP`]）。
const OCTAVE_SEED_STEP: u64 = 0x1000_0000;

/// BlendedNoise：min/max 限位 + main 选择器的三层混合 3D 噪声。
#[derive(Clone, Copy, Debug)]
pub struct BlendedNoise {
    seed_min: u64,
    seed_max: u64,
    seed_main: u64,
}

impl BlendedNoise {
    /// 三栈种子：域盐 + 栈码 fork（vanilla 三次 `createLegacyForBlendedNoise`
    /// 从同一 RandomSource 顺序派生——clean-room 用分栈 splitmix 域盐达成
    /// 统计独立，同 [`crate::vanilla::noise::NormalNoise`] 第二层先例）。
    #[must_use]
    pub fn new(world_seed: u64, salt: u64) -> Self {
        let s = noise_seed(world_seed, salt);
        Self {
            seed_min: splitmix64(s ^ 0x5A11_1A5A),
            seed_max: splitmix64(s ^ 0x5A22_2A5A),
            seed_main: splitmix64(s ^ 0x5A33_3A5A),
        }
    }

    /// 单倍频求值：栈根种子 + i·2^28 派生，坐标已含 2^-i 频率因子。
    /// OS2S 内核输出乘 3D 体积口径常数
    /// [`crate::vanilla::noise::OS2S_TO_IMPROVED_SIGMA_3D`]
    /// 校准回原版 ImprovedNoise 倍频统计（BlendedNoise.java:105-148 的三栈
    /// 在原版均为 ImprovedNoise 且全 3D 采样；σ 基准证据见该常数文档）。
    #[inline]
    fn octave(root: u64, i: usize, x: f64, y: f64, z: f64) -> f64 {
        let seed = root.wrapping_add((i as u64).wrapping_mul(OCTAVE_SEED_STEP));
        crate::vanilla::noise::kernel_octave_3d(crate::os2s::smooth::noise3_ImproveXZ(
            seed as i64,
            wrap(x),
            wrap(y),
            wrap(z),
        ))
    }

    /// BlendedNoise.compute（BlendedNoise.java:105-148）。
    #[must_use]
    pub fn get_value(&self, block_x: f64, block_y: f64, block_z: f64) -> f64 {
        let limit_x = block_x * XZ_MULT;
        let limit_y = block_y * Y_MULT;
        let limit_z = block_z * XZ_MULT;
        let main_x = limit_x / XZ_FACTOR;
        let main_y = limit_y / Y_FACTOR;
        let main_z = limit_z / XZ_FACTOR;
        // main 选择器：8 倍频，坐标 ×2^-i、权重 2^i（/pow）。
        let mut pow = 1.0_f64;
        let mut main_value = 0.0_f64;
        for i in 0..MAIN_OCTAVES {
            main_value +=
                Self::octave(self.seed_main, i, main_x * pow, main_y * pow, main_z * pow) / pow;
            pow *= 0.5;
        }
        let factor = (main_value / 10.0 + 1.0) / 2.0;
        let is_max = factor >= 1.0;
        let is_min = factor <= 0.0;
        // min/max 限位：16 倍频，同权递推；饱和侧整栈跳过（vanilla 同款剪枝）。
        let mut pow = 1.0_f64;
        let mut blend_min = 0.0_f64;
        let mut blend_max = 0.0_f64;
        for i in 0..LIMIT_OCTAVES {
            let wx = wrap(limit_x * pow);
            let wy = wrap(limit_y * pow);
            let wz = wrap(limit_z * pow);
            if !is_max {
                blend_min += Self::octave(self.seed_min, i, wx, wy, wz) / pow;
            }
            if !is_min {
                blend_max += Self::octave(self.seed_max, i, wx, wy, wz) / pow;
            }
            pow *= 0.5;
        }
        // Mth.clampedLerp(factor, blendMin/512, blendMax/512) / 128（:148）。
        let lo = blend_min / 512.0;
        let hi = blend_max / 512.0;
        let v = if factor < 0.0 {
            lo
        } else if factor > 1.0 {
            hi
        } else {
            lo + (hi - lo) * factor
        };
        v / 128.0
    }
}

/// 巨坐标封装（PerlinNoise.wrap:188-190，同 [`crate::vanilla::noise`]）。
#[must_use]
fn wrap(x: f64) -> f64 {
    let d = 3.355_443_2_e7_f64;
    x - (x / d + 0.5).floor() * d
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 确定性：同种子同坐标两次求值位型一致。
    #[test]
    fn blended_noise_deterministic() {
        let n = BlendedNoise::new(42, 0x00B1_3D00);
        let a = n.get_value(1.23, 63.0, -7.89);
        let b = n.get_value(1.23, 63.0, -7.89);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    /// 值域哨兵：clampedLerp(f, min/512, max/512)/128 输出有界（限位栈 16 倍频
    /// 加权和 /65536，全同号极限 ≈ ±(2^16-1)/512/128 ≈ ±0.999）。
    #[test]
    fn blended_noise_stays_bounded() {
        let n = BlendedNoise::new(7, 0x1234);
        for k in 0..256_i32 {
            let v = n.get_value(
                f64::from(k) * 3.7 - 400.0,
                f64::from(k % 128) * 2.3,
                f64::from(k) * -1.9 + 91.0,
            );
            assert!(v.abs() <= 1.0 + 1e-9, "BlendedNoise 越域 {v}");
        }
    }
}
