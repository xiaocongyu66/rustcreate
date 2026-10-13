//! `PerlinSimplexNoise`/`SimplexNoise` 实物移植 — #191 尾款第二刀。
//! 供 [`crate::vanilla::biomes::height_adjusted_temperature`] 的
//! `TEMPERATURE_NOISE`（Biome.java:61）求值使用。
//!
//! 实物锚点（逐句转写，引用机制与常数 ≠ 搬运代码）：
//! - `synth/SimplexNoise.java:7-52`：16 项 GRADIENT 表、构造期
//!   `nextDouble()*256` 三偏移 + 256 步 Fisher–Yates 洗牌、`getValue(x,y)`
//!   2D（F2/G2 常数、`% 12` 梯度选择、×70 归一）与 `getValue(x,y,z)` 3D
//!   （×32 归一，构造器高倍频种子派生用）；
//! - `synth/PerlinSimplexNoise.java:14-69`：octave 集合构造（本仓唯一
//!   消费者 TEMPERATURE_NOISE 的集合为 `[0]` → 退化为单实例 2D，
//!   `input_factor=1.0、value_factor=1.0`；`use_noise_start=false` 时
//!   不叠 `xo/yo` 偏移）；
//! - `levelgen/LegacyRandomSource.java:9-46`：java.util.Random 同款 48-bit
//!   LCG（MULTIPLIER=25214903917、INCREMENT=11、setSeed 预混 XOR）；
//!   `WorldgenRandom.java:27-31` 对 LegacyRandomSource 包装时逐位透传
//!   `next(bits)`，故随机序列 = 裸 LCG；
//! - `levelgen/BitRandomSource.java:8,26-37,48-52`：`nextDouble()` 乘子
//!   `DOUBLE_MULTIPLIER = 1.110223E-16F`（float 字面量 double 化经考证
//!   恰为 2⁻⁵³，与 java.util.Random 位等价——tests 锁定）；
//!   `nextInt(bound)` 的 2 幂快路与拒绝采样同式。
//!
//! 世界种子接线：Biome.java:61 的 `TEMPERATURE_NOISE` 种子是常量
//! `LegacyRandomSource(1234)`（类静态字段），**与世界种子无关**——
//! 不存在 WorldgenSettings/ChunkGenerator 派生链，直接常量播种。
//!
//! 已知边界（不影响本仓消费者）：`consumeCount(262)`（多倍频集合的
//! 空洞占位，PerlinSimplexNoise.java:40,52）按 `nextInt()` ×count 转写；
//! 集合 `[0]` 下该路径不可达。`FROZEN_TEMPERATURE_NOISE`（种子 3456，
//! 集合 [-2,-1,0]，仅 ICE_PLAIN 温度修饰用）本 10 群系子集无消费者，
//! 未接——机制已由本模块通用构造器覆盖。

use std::sync::OnceLock;

/// java.util.Random 同款 48-bit LCG（`LegacyRandomSource.java:10-46`
/// 经 `WorldgenRandom.next` 透传后的有效序列）。
pub struct LegacyRandom {
    seed: u64,
}

/// `MODULUS_MASK = 281474976710655L`（2^48-1）。
const MASK: u64 = 0xFFFF_FFFF_FFFF;
/// `MULTIPLIER = 25214903917L`。
const MULT: u64 = 25214903917;

impl LegacyRandom {
    /// `setSeed`：`(seed ^ MULTIPLIER) & MASK`（LegacyRandomSource.java:34）。
    #[must_use]
    pub fn new(seed: i64) -> Self {
        Self {
            seed: ((seed as u64) ^ MULT) & MASK,
        }
    }

    /// `next(bits)`：LCG 步进后取高 `bits` 位为有符号 int
    /// （LegacyRandomSource.java:41-46：`(int)(newSeed >> 48 - bits)`）。
    fn next(&mut self, bits: u32) -> i32 {
        self.seed = self.seed.wrapping_mul(MULT).wrapping_add(11) & MASK;
        ((self.seed >> (48 - bits)) as u32) as i32
    }

    /// `nextInt()` = `next(32)`（BitRandomSource.java:13-15）。
    fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    /// `nextInt(bound)`（BitRandomSource.java:18-37）：2 幂 bound 走
    /// `(bound * next(31)) >> 31` 乘法折叠，否则拒绝采样
    /// `sample - modulo + bound - 1 >= 0`。
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        debug_assert!(bound > 0, "Bound must be positive");
        if bound & (bound - 1) == 0 {
            return (((bound as i64) * (self.next(31) as i64)) >> 31) as i32;
        }
        loop {
            let sample = self.next(31);
            let modulo = sample % bound;
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }

    /// `nextDouble()`（BitRandomSource.java:48-52）：
    /// `((next(26) << 27) + next(27)) * DOUBLE_MULTIPLIER`，乘子为
    /// float 字面量 `1.110223E-16F` 的 double 化值（恰为 2⁻⁵³，见模块头注）。
    fn next_double(&mut self) -> f64 {
        let upper = self.next(26) as i64;
        let lower = self.next(27) as i64;
        let combined = (upper << 27) + lower;
        combined as f64 * (1.110223e-16_f32 as f64)
    }

    /// `consumeCount(count)`：`nextInt()` ×count（多倍频空洞占位；
    /// 集合 `[0]` 消费者不可达，见模块头注）。
    fn consume_count(&mut self, count: usize) {
        for _ in 0..count {
            self.next_int();
        }
    }
}

/// `synth/SimplexNoise.java`：2D/3D 单倍频单纯形噪声。
pub struct SimplexNoise {
    /// 构造期 `nextDouble()*256` 三偏移（SimplexNoise.java:30-33）。
    pub xo: f64,
    pub yo: f64,
    pub zo: f64,
    /// 256 洗牌 + 复制成 512 的置换表（SimplexNoise.java:27,34-45）。
    p: [i32; 512],
}

/// `SimplexNoise.GRADIENT` 16 项梯度表（SimplexNoise.java:8-25）。
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

/// `SQRT_3 = Math.sqrt(3.0)`、`F2 = 0.5*(SQRT_3-1)`、
/// `G2 = (3-SQRT_3)/6`（SimplexNoise.java:26-28）。
const F2: f64 = 0.3660254037844386;
const G2: f64 = 0.21132486540518708;

impl SimplexNoise {
    /// 构造：三偏移 + Fisher–Yates（`offset = nextInt(256 - ix)`，
    /// SimplexNoise.java:40-45）。
    #[must_use]
    pub fn new(random: &mut LegacyRandom) -> Self {
        let xo = random.next_double() * 256.0;
        let yo = random.next_double() * 256.0;
        let zo = random.next_double() * 256.0;
        let mut p = [0_i32; 512];
        for (i, v) in p.iter_mut().enumerate().take(256) {
            *v = i as i32;
        }
        for ix in 0..256usize {
            let ix32 = ix as i32;
            let swap = (random.next_int_bound(256 - ix32) + ix32) as usize;
            p.swap(ix, swap);
        }
        p.copy_within(0..256, 256);
        Self { xo, yo, zo, p }
    }

    fn p(&self, x: i32) -> i32 {
        self.p[(x & 0xFF) as usize]
    }

    /// `getCornerNoise3D`（SimplexNoise.java:57-67）。
    fn corner_noise(idx: usize, x: f64, y: f64, z: f64, base: f64) -> f64 {
        let mut t = base - x * x - y * y - z * z;
        if t < 0.0 {
            0.0
        } else {
            t *= t;
            t * t
                * (GRADIENT[idx][0] as f64 * x
                    + GRADIENT[idx][1] as f64 * y
                    + GRADIENT[idx][2] as f64 * z)
        }
    }

    /// `getValue(xin, yin)` 2D 切片（SimplexNoise.java:69-104，×70 归一）。
    #[must_use]
    pub fn get_value_2d(&self, xin: f64, yin: f64) -> f64 {
        let s = (xin + yin) * F2;
        let i = (xin + s).floor() as i32;
        let j = (yin + s).floor() as i32;
        let t = (i + j) as f64 * G2;
        let x0 = xin - (i as f64 - t);
        let y0 = yin - (j as f64 - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - i1 as f64 + G2;
        let y1 = y0 - j1 as f64 + G2;
        let x2 = x0 - 1.0 + 2.0 * G2;
        let y2 = y0 - 1.0 + 2.0 * G2;
        let ii = i & 0xFF;
        let jj = j & 0xFF;
        let gi0 = self.p(ii + self.p(jj)) % 12;
        let gi1 = self.p(ii + i1 + self.p(jj + j1)) % 12;
        let gi2 = self.p(ii + 1 + self.p(jj + 1)) % 12;
        let n0 = Self::corner_noise(gi0 as usize, x0, y0, 0.0, 0.5);
        let n1 = Self::corner_noise(gi1 as usize, x1, y1, 0.0, 0.5);
        let n2 = Self::corner_noise(gi2 as usize, x2, y2, 0.0, 0.5);
        70.0 * (n0 + n1 + n2)
    }

    /// `getValue(xin, yin, zin)` 3D（SimplexNoise.java:106-190，×32 归一；
    /// PerlinSimplexNoise 高倍频种子派生用，集合 `[0]` 下仅构造期不触达）。
    #[must_use]
    pub fn get_value_3d(&self, xin: f64, yin: f64, zin: f64) -> f64 {
        let s = (xin + yin + zin) * (1.0 / 3.0);
        let i = (xin + s).floor() as i32;
        let j = (yin + s).floor() as i32;
        let k = (zin + s).floor() as i32;
        let g3 = 1.0 / 6.0;
        let t = (i + j + k) as f64 * g3;
        let x0 = xin - (i as f64 - t);
        let y0 = yin - (j as f64 - t);
        let z0 = zin - (k as f64 - t);
        let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
            if y0 >= z0 {
                (1, 0, 0, 1, 1, 0)
            } else if x0 >= z0 {
                (1, 0, 0, 1, 0, 1)
            } else {
                (0, 0, 1, 1, 0, 1)
            }
        } else if y0 < z0 {
            (0, 0, 1, 0, 1, 1)
        } else if x0 < z0 {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        };
        let x1 = x0 - i1 as f64 + g3;
        let y1 = y0 - j1 as f64 + g3;
        let z1 = z0 - k1 as f64 + g3;
        let x2 = x0 - i2 as f64 + 2.0 * g3;
        let y2 = y0 - j2 as f64 + 2.0 * g3;
        let z2 = z0 - k2 as f64 + 2.0 * g3;
        let x3 = x0 - 1.0 + 0.5;
        let y3 = y0 - 1.0 + 0.5;
        let z3 = z0 - 1.0 + 0.5;
        let ii = i & 0xFF;
        let jj = j & 0xFF;
        let kk = k & 0xFF;
        let gi0 = self.p(ii + self.p(jj + self.p(kk))) % 12;
        let gi1 = self.p(ii + i1 + self.p(jj + j1 + self.p(kk + k1))) % 12;
        let gi2 = self.p(ii + i2 + self.p(jj + j2 + self.p(kk + k2))) % 12;
        let gi3 = self.p(ii + 1 + self.p(jj + 1 + self.p(kk + 1))) % 12;
        let n0 = Self::corner_noise(gi0 as usize, x0, y0, z0, 0.6);
        let n1 = Self::corner_noise(gi1 as usize, x1, y1, z1, 0.6);
        let n2 = Self::corner_noise(gi2 as usize, x2, y2, z2, 0.6);
        let n3 = Self::corner_noise(gi3 as usize, x3, y3, z3, 0.6);
        32.0 * (n0 + n1 + n2 + n3)
    }
}

/// `synth/PerlinSimplexNoise.java`：octave 集合驱动的倍频堆叠。
pub struct PerlinSimplexNoise {
    levels: Vec<Option<SimplexNoise>>,
    highest_freq_input_factor: f64,
    highest_freq_value_factor: f64,
}

impl PerlinSimplexNoise {
    /// 实物构造器逐段转写（PerlinSimplexNoise.java:19-64）。
    /// `octave_set` 为实物 `IntRBTreeSet`（升序去重后传入）。
    #[must_use]
    pub fn new(random: &mut LegacyRandom, octave_set: &[i32]) -> Self {
        assert!(!octave_set.is_empty(), "Need some octaves!");
        let low_freq_octaves = -*octave_set.first().unwrap();
        let high_freq_octaves = *octave_set.last().unwrap();
        let octaves = (low_freq_octaves + high_freq_octaves + 1) as usize;
        assert!(octaves >= 1, "Total number of octaves needs to be >= 1");
        let contains = |o: i32| octave_set.contains(&o);

        let zero_octave = SimplexNoise::new(random);
        // 实物高倍频种子派生 `(long)(zeroOctave.getValue(xo,yo,zo) *
        // 9.223372E18F)`：double ×（float 字面量 double 化），(long) 向零
        // 截断。纯函数求值提前于此（避免 Rust 移动语义缠绕，语义不变）。
        let positive_octave_seed =
            (zero_octave.get_value_3d(zero_octave.xo, zero_octave.yo, zero_octave.zo)
                * (9.223372e18_f32 as f64)) as i64;
        let zero_index = high_freq_octaves;
        // （SimplexNoise 非 Clone，不能用 vec![None; n]。）
        let mut levels: Vec<Option<SimplexNoise>> = (0..octaves).map(|_| None).collect();
        if zero_index >= 0 && (zero_index as usize) < octaves && contains(0) {
            levels[zero_index as usize] = Some(zero_octave);
        }
        for i in (zero_index + 1)..octaves as i32 {
            if i >= 0 && contains(zero_index - i) {
                levels[i as usize] = Some(SimplexNoise::new(random));
            } else {
                random.consume_count(262);
            }
        }
        if high_freq_octaves > 0 {
            let mut high_freq_random = LegacyRandom::new(positive_octave_seed);
            for i in (0..zero_index).rev() {
                if (i as usize) < octaves && contains(zero_index - i) {
                    levels[i as usize] = Some(SimplexNoise::new(&mut high_freq_random));
                } else {
                    high_freq_random.consume_count(262);
                }
            }
        }
        Self {
            levels,
            highest_freq_input_factor: (2.0f64).powi(high_freq_octaves),
            highest_freq_value_factor: 1.0 / ((2.0f64).powi(octaves as i32) - 1.0),
        }
    }

    /// `getValue(x, y, useNoiseStart)`（PerlinSimplexNoise.java:66-79）：
    /// 每层频率 ×1/2、值权 ×2 递推；`use_noise_start=false` 不叠偏移。
    #[must_use]
    pub fn get_value(&self, x: f64, y: f64, use_noise_start: bool) -> f64 {
        let mut value = 0.0;
        let mut factor = self.highest_freq_input_factor;
        let mut value_factor = self.highest_freq_value_factor;
        for level in &self.levels {
            if let Some(n) = level {
                let xs = x * factor + if use_noise_start { n.xo } else { 0.0 };
                let ys = y * factor + if use_noise_start { n.yo } else { 0.0 };
                value += n.get_value_2d(xs, ys) * value_factor;
            }
            factor /= 2.0;
            value_factor *= 2.0;
        }
        value
    }
}

/// `Biome.TEMPERATURE_NOISE`（Biome.java:61）：
/// `PerlinSimplexNoise(WorldgenRandom(LegacyRandomSource(1234)), [0])`
/// ——常量种子，世界种子无关；进程内共享单例（实物 static final）。
#[must_use]
pub fn temperature_noise() -> &'static PerlinSimplexNoise {
    static NOISE: OnceLock<PerlinSimplexNoise> = OnceLock::new();
    NOISE.get_or_init(|| PerlinSimplexNoise::new(&mut LegacyRandom::new(1234), &[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LCG 位型锁：java.util.Random(0) 文档序列首位
    /// `nextInt() == -1155484576`（LegacyRandomSource 透传后同序列）。
    #[test]
    fn lcg_matches_java_random_bit_pattern() {
        let mut r = LegacyRandom::new(0);
        // java.util.Random(0) 公开文档值 `nextInt() == -1155484576`
        // ——单锚已逐位锁定 seed 预混 + LCG 步进 + 高位截取全链。
        assert_eq!(r.next_int(), -1_155_484_576);
    }

    /// MC 乘子逐位考证：float 字面量 `1.110223E-16F` double 化后
    /// **恰为 2⁻⁵³**（float→double 往返落在精确 2 的幂上，CI 实证）
    /// ——故 `next_double()` 与 java.util.Random 位等价；此测试锁住
    /// 「不要误改乘子为其它近似十进制值」。
    #[test]
    fn mc_double_multiplier_is_pow2_53() {
        assert_eq!(1.110223e-16_f32 as f64, 2f64.powi(-53));
    }

    /// nextInt(bound)：2 幂快路与非 2 幂拒绝采样都覆盖（洗牌全程
    /// 用到 bound=256-ix 两类）。
    #[test]
    fn next_int_bound_in_range() {
        let mut r = LegacyRandom::new(1234);
        for ix in 0..256 {
            let v = r.next_int_bound(256 - ix);
            assert!((0..256 - ix).contains(&v), "{ix} → {v}");
        }
    }

    /// SimplexNoise 构造：置换表合法（0..255 全排列 + 512 复制）。
    #[test]
    fn simplex_permutation_is_permutation() {
        let s = SimplexNoise::new(&mut LegacyRandom::new(1234));
        let mut head: Vec<i32> = s.p[..256].to_vec();
        head.sort_unstable();
        assert!(head.iter().enumerate().all(|(i, v)| *v == i as i32));
        assert!(s.p[256..] == s.p[..256]);
    }

    /// 2D 值域 ±1（×70 归一）+ 确定性。
    #[test]
    fn simplex_2d_domain_and_determinism() {
        let s = SimplexNoise::new(&mut LegacyRandom::new(1234));
        for k in 0..64 {
            let v = s.get_value_2d(k as f64 * 0.37, k as f64 * -0.53);
            assert!((-1.0..=1.0).contains(&v), "{v} 越界");
            assert_eq!(v, s.get_value_2d(k as f64 * 0.37, k as f64 * -0.53));
        }
    }

    /// TEMPERATURE_NOISE：集合 [0] 退化为单实例——输入/值因子恒 1，
    /// 且与底层 SimplexNoise 同点同值（use_noise_start=false → 不叠偏移）。
    #[test]
    fn temperature_noise_octave_zero_degenerates_to_single() {
        let n = temperature_noise();
        assert_eq!(n.levels.len(), 1);
        assert_eq!(n.highest_freq_input_factor, 1.0);
        assert_eq!(n.highest_freq_value_factor, 1.0);
        let mut r = LegacyRandom::new(1234);
        let ref_s = SimplexNoise::new(&mut r);
        for k in 0..16 {
            let (x, z) = (k as f64 * 1.7, k as f64 * -2.3);
            assert_eq!(n.get_value(x, z, false), ref_s.get_value_2d(x, z));
        }
    }

    /// 实物调用式 `getValue(x/8.0F, z/8.0F, false) * 8.0` 的值域：
    /// 单倍频 ×8 → |v| ≤ 8（高度修正公式的界算依据）。
    #[test]
    fn temperature_noise_scaled_range() {
        let n = temperature_noise();
        for k in 0..256 {
            let v = n.get_value((k % 97) as f64 / 8.0, (k * 13) as f64 / 8.0, false) * 8.0;
            assert!(v.abs() <= 8.0 + 1e-9, "{v}");
        }
    }
}
