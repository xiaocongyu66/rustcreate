//! vanilla ImprovedNoise — clean-room 移植 26.1 `synth/ImprovedNoise.java`
//! 的格点噪声机制（引用机制与常数 ≠ 搬运代码；版权红线：仓库零 Mojang 文件）。
//!
//! ## 为什么替换 OS2S 内核（地形保真波第四棒，run 38154750449 双红取证）
//!
//! T2 的 σ 校准（`OS2S_TO_IMPROVED_SIGMA_*`）只对齐了**单倍频值分布 σ**，
//! 不还原 ImprovedNoise 的**空间相关结构**（核函数谱形/窗内响应/相邻差分
//! 分布）。用忠实移植复算器在 quality.rs 完全同口径（同种子 0x7EA200B1、
//! 同网格、同区域）对照，两门红都是这一个根因：
//! - `plains_share_in_band`：窗（32×32 格）内 std 由 shift 通道 32 格波长
//!   涟漪 + 气候细倍频驱动，OS2S 校准版实测 ok=59/900=0.066（std-fail 194、
//!   land-fail 647），ImprovedNoise 版 **185/900=0.206**；
//! - `base_3d_channel_adjacent_delta_p90`：相邻块 |Δ| 谱由 main 选择器栈的
//!   高频倍频主导，OS2S 校准版实测 **0.0550**>0.05，ImprovedNoise 版
//!   **0.0153/0.0138**（(0,0)/(4096,-4096)）。
//!
//! 即：σ 相等 ≠ 谱相等，修移植 = 内核换成原版机制本体。
//!
//! ## 机制出处（26.1 反编译树 /root/mc-ref/src-26.1）
//! - 构造：`ImprovedNoise.java:14-30` xo/yo/zo = `nextDouble()·256`，
//!   p[byte;256] 恒等填充后 Fisher-Yates（`nextInt(256-i)` 交换 i↔i+offset）；
//! - 求值：`noise`（:37-62）x/y/z 加原点取整到晶格，5 参形在 yScale≠0 时做
//!   **yFudge 纵向量化**（fudgeLimit = yFudge∈[0,yr) ? yFudge : yr；
//!   yrFudge = `floor(fudgeLimit/yScale + SHIFT_UP_EPSILON)·yScale`，
//!   SHIFT_UP_EPSILON = 1.0E-7F（:8，f32 字面量提升 f64）；
//! - 插值：`sampleAndLerp`（:85-104）8 角点 `gradDot`（SimplexNoise.GRADIENT
//!   [hash&15] 点积，SimplexNoise.java:7-24 的 12+4 梯度表）+ fade 五次
//!   smoothstep（Mth.smoothstep:574-576）三线性 `Mth.lerp3`（Mth.java:548-570，
//!   lerp3 = lerp(zA, lerp2(xA,yA,·), lerp2(xA,yA,·))，lerp2 = lerp(yA, lerp(xA,·),
//!   lerp(xA,·))）。**注意 yAlpha 用量化前的 yrOriginal**（:101）——BlendedNoise
//!   高频限位倍频「纵向 smear、水平谱不变」的机制本体；
//! - 消费方：`PerlinNoise`（每倍频一个实例）→ `NormalNoise`（两层独立）与
//!   `BlendedNoise`（min/max 16 + main 8 三栈）。
//!
//! ## 种子偏差（clean-room 既定取舍，同 [`crate::vanilla::noise`] 倍频种子先例）
//! vanilla 的 xo/yo/zo 与排列来自 `RandomState` 域盐链上的
//! XoroshiroRandomSource；本实现用 splitmix64 流给出同分布的 uniform
//! [0,1)/[0,bound) 抽样——种子材料不同即统计独立，机制与统计口径与原版
//! 一致，确定性不依赖具体派生式。单倍频统计（/tmp/improved_noise.py +
//! /tmp/sigma_slice.py 忠实移植复算，12 种子 × 15 万样本）：体积 σ=0.27106、
//! y=0 切片 σ=0.26555——`NormalNoise` 的 value_factor 归一
//!（NormalNoise.java:63）即以该统计为基准，内核一致后归一无残余偏差。

use crate::vanilla::noise::splitmix64;

/// SHIFT_UP_EPSILON（ImprovedNoise.java:8，f32 字面量 1.0E-7F 提升到 f64
/// 的精确值 1.0000000116860974e-07——const 上下文不能用 From<f32>，落成
/// 字面量常数）。
const SHIFT_UP_EPSILON: f64 = 1.000_000_011_686_097_4e-7;

/// SimplexNoise.GRADIENT（SimplexNoise.java:7-24）：12 个轴向 ±1 组合 + 4 个
/// 重复项，索引 = hash & 15。
const GRADIENT: [[i32; 3]; 16] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
    [1, 1, 0],
    [0, -1, 1],
    [-1, 1, 0],
    [0, -1, -1],
];

/// gradDot（ImprovedNoise.java:77-79 = SimplexNoise.dot:55-57）。
#[inline]
fn grad_dot(hash: usize, x: f64, y: f64, z: f64) -> f64 {
    let g = GRADIENT[hash & 15];
    f64::from(g[0]) * x + f64::from(g[1]) * y + f64::from(g[2]) * z
}

/// Mth.smoothstep（Mth.java:574-576）。
#[inline]
fn smoothstep(x: f64) -> f64 {
    x * x * x * (x * (6.0 * x - 15.0) + 10.0)
}

/// Mth.lerp（线性）+ lerp2/lerp3（Mth.java:544-570）：三线性，先 x 后 y 再 z。
#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

#[inline]
fn lerp2(a1: f64, a2: f64, x00: f64, x10: f64, x01: f64, x11: f64) -> f64 {
    lerp(a2, lerp(a1, x00, x10), lerp(a1, x01, x11))
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn lerp3(
    a1: f64,
    a2: f64,
    a3: f64,
    x000: f64,
    x100: f64,
    x010: f64,
    x110: f64,
    x001: f64,
    x101: f64,
    x011: f64,
    x111: f64,
) -> f64 {
    lerp(
        a3,
        lerp2(a1, a2, x000, x100, x010, x110),
        lerp2(a1, a2, x001, x101, x011, x111),
    )
}

/// splitmix64 确定性流（clean-room 种子源，见模块头注「种子偏差」）。
struct SplitmixStream(u64);

impl SplitmixStream {
    fn next_u64(&mut self) -> u64 {
        self.0 = splitmix64(self.0);
        self.0
    }

    /// uniform [0,1)（vanilla nextDouble 的同分布替身）。
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0_f64 / (1_u64 << 53) as f64)
    }

    /// uniform [0, bound)（vanilla nextInt(bound) 的同分布替身）。
    fn next_below(&mut self, bound: usize) -> usize {
        ((self.next_u64() >> 32) as usize) % bound
    }
}

/// 原版 ImprovedNoise：原点平移 + 256 元排列 + 8 角点梯度点积 + fade 插值。
#[derive(Clone, Debug)]
pub struct ImprovedNoise {
    xo: f64,
    yo: f64,
    zo: f64,
    p: [u8; 256],
}

impl ImprovedNoise {
    /// ImprovedNoise.java:14-30。
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut rng = SplitmixStream(seed);
        let xo = rng.next_f64() * 256.0;
        let yo = rng.next_f64() * 256.0;
        let zo = rng.next_f64() * 256.0;
        let mut p = [0_u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        for i in 0..256_usize {
            let offset = rng.next_below(256 - i);
            p.swap(i, i + offset);
        }
        Self { xo, yo, zo, p }
    }

    /// 同机制的**原版种子流**构造（#192 RNG-1 家族）：xo/yo/zo =
    /// nextDouble()·256 + Fisher-Yates `nextInt(256−i)`，随机源换成
    /// Xoroshiro128++（vanilla ImprovedNoise 构造即此流，
    /// XoroshiroRandomSource.java:48-70 的 Lemire 归约含 bound=1 也消耗
    /// 一次 long——next_int_bounded 同构）。给 `NormalNoise::
    /// from_vanilla_name`（SurfaceSystem 表层噪声族）用；与 new() 的
    /// splitmix 流互不互通（既有通道种子不动，无历史锚可破）。
    #[must_use]
    pub fn from_xoroshiro(rng: &mut crate::vanilla::xoroshiro::Xoroshiro) -> Self {
        let xo = rng.next_double() * 256.0;
        let yo = rng.next_double() * 256.0;
        let zo = rng.next_double() * 256.0;
        let mut p = [0_u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        for i in 0..256_usize {
            let offset = rng.next_int_bounded((256 - i) as u32) as usize;
            p.swap(i, i + offset);
        }
        Self { xo, yo, zo, p }
    }

    /// 同机制的 **legacy 随机源流**构造（flora 花域噪声链专用）：
    /// 随机源 = 48-bit LCG（`simplex::LegacyRandom`，LegacyRandomSource
    /// 透传 BitRandomSource——nextDouble = 浮点域乘、nextInt(bound) =
    /// 2 幂折叠/拒绝采样），与 `from_xoroshiro` 同一消耗序
    /// （ImprovedNoise.java:14-30：xo/yo/zo = nextDouble()·256 +
    /// Fisher–Yates `nextInt(256−i)`）。给 flora 花域
    /// `NormalNoise.create(LegacyRandomSource)` 路径（NoiseBasedState
    /// Provider.java:31）供体；与 new()/from_xoroshiro() 三流互不互通。
    #[must_use]
    pub fn from_legacy(rng: &mut crate::vanilla::simplex::LegacyRandom) -> Self {
        let xo = rng.next_double() * 256.0;
        let yo = rng.next_double() * 256.0;
        let zo = rng.next_double() * 256.0;
        let mut p = [0_u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        for i in 0..256_usize {
            let offset = rng.next_int_bound((256 - i) as i32) as usize;
            p.swap(i, i + offset);
        }
        Self { xo, yo, zo, p }
    }

    /// 排列查询（ImprovedNoise.java:81-83：`p[x & 0xFF] & 0xFF`）。
    #[inline]
    fn p_at(&self, x: i64) -> usize {
        usize::from(self.p[(x & 0xFF) as usize])
    }

    /// 3 参求值 = 5 参 yScale=0（ImprovedNoise.java:32-34）。
    #[must_use]
    pub fn noise(&self, x: f64, y: f64, z: f64) -> f64 {
        self.noise_fudged(x, y, z, 0.0, 0.0)
    }

    /// 5 参求值（ImprovedNoise.java:37-62）：yScale≠0 时 y 分数坐标按 yFudge
    /// 量化——BlendedNoise.compute 对三栈全部走 5 参（BlendedNoise.java:109
    /// mainSmear/mainY 与 :130/:137 yScalePow/limitY）。
    #[must_use]
    pub fn noise_fudged(&self, _x: f64, _y: f64, _z: f64, y_scale: f64, y_fudge: f64) -> f64 {
        let x = _x + self.xo;
        let y = _y + self.yo;
        let z = _z + self.zo;
        let xf = x.floor() as i64;
        let yf = y.floor() as i64;
        let zf = z.floor() as i64;
        let xr = x - xf as f64;
        let yr = y - yf as f64;
        let zr = z - zf as f64;
        let yr_fudge = if y_scale != 0.0 {
            // fudgeLimit = yFudge∈[0,yr) ? yFudge : yr（:50-54）
            let fudge_limit = if y_fudge >= 0.0 && y_fudge < yr {
                y_fudge
            } else {
                yr
            };
            (fudge_limit / y_scale + SHIFT_UP_EPSILON).floor() * y_scale
        } else {
            0.0
        };
        self.sample_and_lerp(xf, yf, zf, xr, yr - yr_fudge, zr, yr)
    }

    /// sampleAndLerp（ImprovedNoise.java:85-104）。**yAlpha 用量化前的
    /// yrOriginal**（:101）。
    #[allow(clippy::too_many_arguments)]
    fn sample_and_lerp(
        &self,
        x: i64,
        y: i64,
        z: i64,
        xr: f64,
        yr: f64,
        zr: f64,
        yr_original: f64,
    ) -> f64 {
        let x0 = self.p_at(x);
        let x1 = self.p_at(x + 1);
        let xy00 = self.p_at(x0 as i64 + y);
        let xy01 = self.p_at(x0 as i64 + y + 1);
        let xy10 = self.p_at(x1 as i64 + y);
        let xy11 = self.p_at(x1 as i64 + y + 1);
        let d000 = grad_dot(self.p_at(xy00 as i64 + z), xr, yr, zr);
        let d100 = grad_dot(self.p_at(xy10 as i64 + z), xr - 1.0, yr, zr);
        let d010 = grad_dot(self.p_at(xy01 as i64 + z), xr, yr - 1.0, zr);
        let d110 = grad_dot(self.p_at(xy11 as i64 + z), xr - 1.0, yr - 1.0, zr);
        let d001 = grad_dot(self.p_at(xy00 as i64 + z + 1), xr, yr, zr - 1.0);
        let d101 = grad_dot(self.p_at(xy10 as i64 + z + 1), xr - 1.0, yr, zr - 1.0);
        let d011 = grad_dot(self.p_at(xy01 as i64 + z + 1), xr, yr - 1.0, zr - 1.0);
        let d111 = grad_dot(self.p_at(xy11 as i64 + z + 1), xr - 1.0, yr - 1.0, zr - 1.0);
        let x_alpha = smoothstep(xr);
        let y_alpha = smoothstep(yr_original);
        let z_alpha = smoothstep(zr);
        lerp3(
            x_alpha, y_alpha, z_alpha, d000, d100, d010, d110, d001, d101, d011, d111,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 确定性：同种子同坐标位型一致；异种子不同值。
    #[test]
    fn improved_noise_deterministic() {
        let a = ImprovedNoise::new(42);
        let b = ImprovedNoise::new(42);
        let c = ImprovedNoise::new(43);
        let (x, y, z) = (1.23_f64, 4.56, -7.89);
        assert_eq!(a.noise(x, y, z).to_bits(), b.noise(x, y, z).to_bits());
        assert_ne!(a.noise(x, y, z).to_bits(), c.noise(x, y, z).to_bits());
    }

    /// 单倍频统计哨兵：体积/切片 σ 与忠实移植复算一致（模块头注：
    /// 体积 0.27106、切片 0.26555；带宽 ±0.03 防回归，12 种子实测极差
    /// <0.01）。这是「OS2S σ 校准常数」退役后的原版统计基准锁。
    #[test]
    fn single_octave_sigma_matches_vanilla_reference() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = || {
            state = splitmix64(state);
            (state >> 11) as f64 * (1.0_f64 / (1_u64 << 53) as f64)
        };
        let inst = ImprovedNoise::new(7);
        let n = 60_000_usize;
        let mut sum = 0.0_f64;
        let mut sum2 = 0.0_f64;
        for _ in 0..n {
            let v = inst.noise(next() * 256.0, next() * 256.0, next() * 256.0);
            sum += v;
            sum2 += v * v;
        }
        let mean = sum / n as f64;
        let sigma = (sum2 / n as f64 - mean * mean).sqrt();
        assert!(
            (0.24..=0.30).contains(&sigma),
            "ImprovedNoise 体积单倍频 σ {sigma:.4} 越带 [0.24,0.30]（原版参考 0.27106）"
        );
        let mut sum = 0.0_f64;
        let mut sum2 = 0.0_f64;
        for _ in 0..n {
            let v = inst.noise(next() * 256.0, 0.0, next() * 256.0);
            sum += v;
            sum2 += v * v;
        }
        let mean = sum / n as f64;
        let sigma = (sum2 / n as f64 - mean * mean).sqrt();
        assert!(
            (0.235..=0.295).contains(&sigma),
            "ImprovedNoise y=0 切片单倍频 σ {sigma:.4} 越带 [0.235,0.295]（原版参考 0.26555）"
        );
    }

    /// yFudge 纵向量化机制锁（ImprovedNoise.java:48-61）：
    /// 1. yScale=0 ⇒ yrFudge=0 ⇒ 5 参退化为 3 参逐位一致（:32-34 与 :57-59）；
    /// 2. yScale≠0 ⇒ 量化改变角点权重（yr' = yr − floor(fudgeLimit/yScale)·yScale
    ///    进入 gradDot，而 yAlpha 仍用量化前 yrOriginal，:101）——足够密的 y
    ///    采样中必有位型不同的点（frac(y+yo) ≥ yScale 的部分）。
    #[test]
    fn y_fudge_quantizes_y_fraction() {
        let inst = ImprovedNoise::new(11);
        let (x, z) = (3.7_f64, -9.2);
        for y in [0.1_f64, 0.55, 1.7, -3.4] {
            let three_arg = inst.noise(x, y, z).to_bits();
            let no_scale = inst.noise_fudged(x, y, z, 0.0, 0.0).to_bits();
            assert_eq!(
                three_arg, no_scale,
                "yScale=0 应逐位退化为 3 参语义（y={y}）"
            );
        }
        let any_diff = (0..50).any(|k| {
            let y = f64::from(k) * 0.037 + 0.11;
            inst.noise_fudged(x, y, z, 0.25, 1.0).to_bits() != inst.noise(x, y, z).to_bits()
        });
        assert!(
            any_diff,
            "yScale≠0 未改变任何采样点 = 纵向量化失效（ImprovedNoise.java:48-56）"
        );
    }
}
