//! vanilla 地形噪声层 — clean-room 对齐 26.1 `synth/PerlinNoise.java` 的
//! fBm 递推语义与 `synth/NormalNoise.java` 的双层独立语义，底层格点噪声
//! 换成 vendored 的 OpenSimplex2S（`crate::os2s::smooth`）。
//!
//! 机制出处（引用机制与常数 ≠ 搬运代码）：
//! - 每倍频频率 ×2.0、值权重 ÷2（PerlinNoise.getValue:140-162 递推）；首倍频
//!   权重 = 2^(octaves-1)/(2^octaves-1)，相对权重 ∝ amplitude[i]/2^i；
//! - NormalNoise = **两个独立** PerlinNoise 实例相加（NormalNoise.java:51-53：
//!   `first`/`second` 是两次 `PerlinNoise.create(random, …)`，各自
//!   `forkPositional()`；XoroshiroRandomSource.java:38-40 每次 fork 消耗生成器
//!   状态 → 两层倍频种子不同），第二层坐标再 ×1.0181268882175227；
//!   输出 × value_factor = 0.16666666666666666 / expectedDeviation；
//! - expectedDeviation 以**非零倍频的下标跨度**计：
//!   0.1·(1 + 1/(span+1))，span = maxOctave − minOctave（NormalNoise.java:59-70
//!   扫非零 amplitude，:97-103 expectedDeviation）——非 amplitudes.len()；
//! - 巨坐标 wrap（PerlinNoise.wrap:188-190，防 f32 晶格退化）。
//!
//! 与 26.1 的差异（v1 子集，验收报告说明）：26.1 的 2D 通道用 2D 格点噪声；
//! 这里统一用 `noise3_ImproveXZ`，2D 通道取 y=0 平面切片（输出同为归一
//! [-1,1] 量级，晶格方向一致），倍频种子以自有 splitmix 域盐派生；第二层
//! 独立种子以一层种子的 splitmix 终混派生（等价 26.1 的第二次 fork：
//! 种子材料不同即统计独立，确定性不依赖具体派生式）。内核四阶 falloff 的
//! 单倍频 σ 高于 ImprovedNoise，逐倍频乘 σ 校准常数（2D 切片/3D 体积双口径，
//! [`OS2S_TO_IMPROVED_SIGMA_2D`]/[`OS2S_TO_IMPROVED_SIGMA_3D`]）回原版统计
//! （地形保真波 T2 主修，证据链见两常数文档）。

/// splitmix64 终混合（复用 legacy 内核同源常数）。
#[must_use]
pub const fn splitmix64(x: u64) -> u64 {
    let x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// density 通道种子：世界种子 ^ 域盐 → 终混合（逐通道互不相同）。
#[must_use]
pub const fn noise_seed(world_seed: u64, salt: u64) -> u64 {
    splitmix64(world_seed ^ salt)
}

/// 倍频种子步进（与 legacy noise.h 同款：每倍频 +i·2^28）。
const OCTAVE_SEED_STEP: u64 = 0x1000_0000;

/// NormalNoise：六参数 density/气候通道用的双层独立 fBm 噪声。
///
/// `salt` 为本通道域盐；`first_octave` 为 26.1 noise json 的 firstOctave
/// （最低倍频波长 = 2^(-firstOctave) 格）；`amplitudes` 为 amplitude 表。
/// `x/y/z` 为已乘通道 xz/y 缩放的 density 坐标（f64）。
#[derive(Clone, Copy, Debug)]
pub struct NormalNoise {
    seed: u64,
    /// 第二层独立实例的根种子（NormalNoise.java:51-53 两次 create 各自
    /// fork → 工厂种子不同；clean-room 用一层种子的 splitmix 终混派生）。
    seed2: u64,
    first_octave: i32,
    amplitudes: &'static [f64],
    value_factor: f64,
    max_value: f64,
    /// 2D 切片采样标记（气候通道 y=0）：选 [`OS2S_TO_IMPROVED_SIGMA_2D`]，
    /// 否则 [`OS2S_TO_IMPROVED_SIGMA_3D`]（见两常数文档的口径差异）。
    slice_2d: bool,
}

impl NormalNoise {
    #[must_use]
    pub fn new(world_seed: u64, salt: u64, first_octave: i32, amplitudes: &'static [f64]) -> Self {
        Self::build(world_seed, salt, first_octave, amplitudes, false)
    }

    /// 2D 切片通道构造器：气候全通道按 y=0 平面切片采样（climate.rs
    /// ShiftedNoise/ShiftNoise 语义），单倍频 σ 校准用**切片口径**
    /// [`OS2S_TO_IMPROVED_SIGMA_2D`]（3D 通道走 [`Self::new`]）。
    #[must_use]
    pub fn new_slice_2d(
        world_seed: u64,
        salt: u64,
        first_octave: i32,
        amplitudes: &'static [f64],
    ) -> Self {
        Self::build(world_seed, salt, first_octave, amplitudes, true)
    }

    fn build(
        world_seed: u64,
        salt: u64,
        first_octave: i32,
        amplitudes: &'static [f64],
        slice_2d: bool,
    ) -> Self {
        let seed = noise_seed(world_seed, salt);
        let seed2 = splitmix64(seed);
        let octaves = amplitudes.len();
        // expectedDeviation 按「非零倍频的下标跨度」计（NormalNoise.java:59-70
        // 扫非零 amplitude 求 min/maxOctave，:97-103 expectedDeviation(span)），
        // 不是 amplitudes.len()——temperature [1.5,0,1] 这类含零表的通道
        // 若按 len 计会系统性抬高 σ。全零表不在 26.1 数据集中，防御按 0。
        let mut min_oct = usize::MAX;
        let mut max_oct = 0_usize;
        for (i, &a) in amplitudes.iter().enumerate() {
            if a != 0.0 {
                min_oct = min_oct.min(i);
                max_oct = max_oct.max(i);
            }
        }
        let span = if min_oct == usize::MAX {
            0
        } else {
            max_oct - min_oct
        };
        let expected_deviation = 0.1_f64 * (1.0 + 1.0 / (span as f64 + 1.0));
        let value_factor = 0.166_666_666_666_666_66_f64 / expected_deviation;
        // edgeValue(2.0)（PerlinNoise.edgeValue:168-182）：全倍频和哨兵值
        let mut edge = 0.0_f64;
        let mut vf = 2.0_f64.powi(octaves as i32 - 1) / (2.0_f64.powi(octaves as i32) - 1.0);
        let mut i = 0;
        while i < octaves {
            edge += amplitudes[i] * 2.0 * vf;
            vf /= 2.0;
            i += 1;
        }
        Self {
            seed,
            seed2,
            first_octave,
            amplitudes,
            value_factor,
            max_value: edge,
            slice_2d,
        }
    }

    /// 值域哨兵（全倍频同号极限）。
    #[must_use]
    pub const fn max_value(&self) -> f64 {
        self.max_value
    }

    #[must_use]
    pub fn get_value(&self, x: f64, y: f64, z: f64) -> f64 {
        let d = 1.018_126_888_217_522_7_f64;
        let first = self.one_perlin(self.seed, x, y, z);
        let second = self.one_perlin(self.seed2, x * d, y * d, z * d);
        (first + second) * self.value_factor
    }

    #[must_use]
    fn one_perlin(&self, root: u64, x: f64, y: f64, z: f64) -> f64 {
        let mut value = 0.0_f64;
        let mut factor = 2.0_f64.powi(self.first_octave);
        let mut value_factor = 2.0_f64.powi(self.amplitudes.len() as i32 - 1)
            / (2.0_f64.powi(self.amplitudes.len() as i32) - 1.0);
        for (i, &amp) in self.amplitudes.iter().enumerate() {
            if amp != 0.0 {
                let seed = root.wrapping_add((i as u64).wrapping_mul(OCTAVE_SEED_STEP));
                let nx = wrap(x * factor);
                let ny = wrap(y * factor);
                let nz = wrap(z * factor);
                value +=
                    amp * if self.slice_2d {
                        kernel_octave_2d(crate::os2s::smooth::noise3_ImproveXZ(
                            seed as i64,
                            nx,
                            ny,
                            nz,
                        ))
                    } else {
                        kernel_octave_3d(crate::os2s::smooth::noise3_ImproveXZ(
                            seed as i64,
                            nx,
                            ny,
                            nz,
                        ))
                    } * value_factor;
            }
            factor *= 2.0;
            value_factor /= 2.0;
        }
        value
    }
}

/// 巨坐标封装（PerlinNoise.wrap:188-190）。
#[must_use]
fn wrap(x: f64) -> f64 {
    let d = 3.355_443_2_e7_f64;
    x - (x / d + 0.5).floor() * d
}

/// OS2S 内核 → 原版 ImprovedNoise 的 σ 校准因子（地形保真波 T2 主修）。
///
/// NormalNoise 的 value_factor 归一（NormalNoise.java:63
/// `0.16666666666666666 / expectedDeviation(...)`，:71-72
/// `0.1·(1+1/(span+1))`）以**原版 ImprovedNoise 倍频统计**为基准标定
/// （气候通道表 NoiseData.java:80-81：continentalness/erosion firstOctave -9，
/// 经 ShiftedNoise xz_scale 0.25 采样，NoiseRouterData.java:89-96）。内核替换为
/// vendored OpenSimplex2S（`noise3_ImproveXZ`，四阶 falloff）后单倍频 σ 系统性
/// 偏高（原 0.26991 vs 0.35946 的 ×1.3317：Ocean 占比 35.71% 越带、平原窗
/// 0.024<0.10 的真实环节），按 σ 比值把内核输出校准回原版倍频统计，使
/// value_factor 归一恢复设计语义。
///
/// **双口径（地形保真波 T2 续刀，2026-10-11 主线程实测）**：内核在「体积均匀
/// 采样」与「y=0 平面切片采样」下的 σ 并不相同（两内核 y 方向结构不同），
/// 12 种子 × 15 万样本/口径（`/tmp/sigma_slice.py` + `/tmp/improved_noise.py`
/// 移植复算）：
/// - 体积口径：ImprovedNoise 0.27106 / OS2S 0.36002 → 比 **0.75291**（3D）；
/// - y=0 切片口径：0.26555 / 0.36010 → 比 **0.73744**（2D）。
///
/// 气候全通道是 2D 切片采样（climate.rs y=0；vanilla ShiftedNoise/
/// ShiftNoise.compute 亦 y·0.25 归零，NoiseRouterData.java:89-97），走
/// [`Self::new_slice_2d`] 选切片常数；3D 通道（洞穴/山脊/base_3d）走体积
/// 常数。BlendedNoise 倍频（vanilla 同为 ImprovedNoise 栈，3D 采样）同用
/// 3D 常数（[`crate::vanilla::blended`]）。
pub const OS2S_TO_IMPROVED_SIGMA_2D: f64 = 0.737_44;
/// 体积（3D）口径 σ 比值 = 0.27106/0.36002（见 [`OS2S_TO_IMPROVED_SIGMA_2D`]）。
pub const OS2S_TO_IMPROVED_SIGMA_3D: f64 = 0.752_91;

/// 内核单倍频输出 → f64（2D 切片通道 σ 校准；见 [`OS2S_TO_IMPROVED_SIGMA_2D`]）。
#[inline]
#[must_use]
pub(crate) fn kernel_octave_2d(n: f32) -> f64 {
    f64::from(n) * OS2S_TO_IMPROVED_SIGMA_2D
}

/// 内核单倍频输出 → f64（3D 体积通道 σ 校准；见 [`OS2S_TO_IMPROVED_SIGMA_3D`]）。
#[inline]
#[must_use]
pub(crate) fn kernel_octave_3d(n: f32) -> f64 {
    f64::from(n) * OS2S_TO_IMPROVED_SIGMA_3D
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 值域哨兵：fBm 输出在 ±max_value 内，且不同盐的通道互不相关
    ///（同坐标抽查均值不为 1）。
    #[test]
    fn normal_noise_stays_in_domain() {
        let a = NormalNoise::new(
            7,
            0x1111,
            -9,
            &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0],
        );
        let b = NormalNoise::new(7, 0x2222, -10, &[1.5, 0.0, 1.0]);
        let mut same = 0;
        for k in 0..64_i64 {
            let (x, y, z) = (k as f64 * 1.7, k as f64 * 0.3, k as f64 * 2.1);
            for (n, mx) in [(&a, a.max_value()), (&b, b.max_value())] {
                let v = n.get_value(x, y, z);
                assert!(v.abs() <= n.max_value() + 1e-9, "fBm 越界 {v} > {mx}");
            }
            if a.get_value(x, y, z) == b.get_value(x, y, z) {
                same += 1;
            }
        }
        assert!(same < 8, "不同域盐通道疑似同种子：{same}/64 全同");
    }

    /// 确定性：同 seed 同坐标两次求值位型一致。
    #[test]
    fn normal_noise_deterministic() {
        let n = NormalNoise::new(
            42,
            0xABCD,
            -9,
            &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0],
        );
        let a = n.get_value(1.23, 4.56, 7.89);
        let b = n.get_value(1.23, 4.56, 7.89);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    /// 第二层必须是独立实例（NormalNoise.java:51-53 两次 create 各自
    /// forkPositional；XoroshiroRandomSource.java:38-40 每次 fork 消耗状态
    /// → 工厂种子不同）：根种子与逐倍频种子流都不得与第一层重合。
    /// 旧实现两层共用 self.seed，第二层 = 同一场 1.018× 重采样（近原点
    /// ρ≈0.95），通道 σ 系统性抬高约 1.4×（曾把 River 群系占比压到带外）。
    #[test]
    fn second_layer_uses_independent_root_seed() {
        let n = NormalNoise::new(2024, 0x1234, -7, &[1.0, 1.0, 1.0]);
        assert_ne!(n.seed, n.seed2, "第二层根种子与第一层相同（未独立派生）");
        for i in 0..n.amplitudes.len() {
            let s1 = n
                .seed
                .wrapping_add((i as u64).wrapping_mul(OCTAVE_SEED_STEP));
            let s2 = n
                .seed2
                .wrapping_add((i as u64).wrapping_mul(OCTAVE_SEED_STEP));
            assert_ne!(s1, s2, "第 {i} 倍频两层种子重合");
        }
    }

    /// expectedDeviation 以非零倍频的下标跨度计（NormalNoise.java:59-70 扫
    /// 非零 amplitude，:97-103 expectedDeviation(maxOctave−minOctave)），
    /// 非 amplitudes.len()：[1,1,0] 的 span=1 → 0.1·(1+1/2)≈0.15
    /// （value_factor ≈ 1.111；若误用 len=3 → 0.125 → 1.333，必被此断言识破）。
    #[test]
    fn expected_deviation_uses_nonzero_octave_span() {
        let n = NormalNoise::new(7, 0xAB, -8, &[1.0, 1.0, 0.0]);
        let want = 0.166_666_666_666_666_66_f64 / 0.15_f64;
        let got = n.value_factor;
        assert!(
            (got - want).abs() < 1e-12,
            "value_factor 偏离 span 语义期望 {want}（实得 {got}）"
        );
    }
}
