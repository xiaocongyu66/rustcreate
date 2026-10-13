//! 原版 RNG 种子链——26.1 `Xoroshiro128PlusPlus`/`XoroshiroRandomSource`/
//! `RandomSupport`/`Mth.getSeed` 的机制转写（#192 RNG-1 销账刀）。
//!
//! 用途：surface_rule `vertical_gradient`（bedrock_floor/deepslate）与
//! SurfaceSystem 表层噪声的位置随机，原版链 =
//! `NoiseGeneratorSettings.getRandomSource()`（主世界默认 XOROSHIRO，
//! WorldgenRandom.java:75-88；overworld.json 无 `random_source` 字段=取
//! 默认，NoiseGeneratorSettings.java:73）`newInstance(worldSeed)
//! .forkPositional()`（RandomState.java:37）→
//! `getOrCreateRandomFactory(name) = random.fromHashOf(name)
//! .forkPositional()`（RandomState.java:113-115）→
//! `at(x,y,z)` 逐位 `nextFloat()`（SurfaceRules.java:764-775）。
//! 本模块把这条链做到与原版逐位一致（同 seed 同坐标同随机数），
//! KNOWN-DIVERGENCE RNG-1 随之销账。
//!
//! 机制出处（/root/mc-ref/src-26.1，引用机制与常数 ≠ 搬运代码）：
//! - `Xoroshiro128PlusPlus.java:24-36`：`result = rotl(s0+s1,17)+s0`，
//!   `s1 ^= s0; s0 = rotl(s0,49) ^ s1 ^ (s1<<21); s1 = rotl(s1,28)`；
//!   全零态回退 (GOLDEN, SILVER)（:18-21）。
//! - `RandomSupport.java:14-15,17-22,24-31,36-42`：`mixStafford13`、
//!   `upgradeSeedTo128bit`（lo = seed ^ SILVER、hi = lo + GOLDEN，再各自
//!   Stafford 终混）、`seedFromHashOf = MD5-128`（前 8 字节 BE = lo、后 8
//!   = hi）。
//! - `XoroshiroRandomSource.java:22-24`（long 种子构造 = upgradeSeedTo128bit
//!   直入）、:38-41（forkPositional = 消耗两次 nextLong）、:96-99
//!   （nextFloat = nextBits(24)·5.9604645E-8F——**float 域乘法**）、
//!   :92-94（nextDouble = nextBits(53)·1.110223E-16F——常量池为 float
//!   字面量，long×float 提升到 float，**乘积 f32 域舍入**后拓宽；常量
//!   值恰 = 2⁻⁵³，f32 舍入 quirk 可返回恰 1.0）、:48-56
//!   （nextInt(bound) = Lemire 无偏归约 + 拒绝循环）。
//! - `XoroshiroPositionalRandomFactory.at = Xoroshiro128PlusPlus(
//!   Mth.getSeed(x,y,z) ^ seedLo, seedHi)`（:124-128）；`fromHashOf =
//!   XoroshiroRandomSource(md5(name) xor (seedLo, seedHi))`（:130-134，
//!   Seed128bit 构造**不再混合**，仅零态守卫）。
//! - `Mth.getSeed(int,int,int)`（Mth.java:314-318）：`x·3129871` 为
//!   **int 域溢出乘**后升 long，`z·116129781L` long 域；`seed²·42317861 +
//!   seed·11 >> 16`（算术右移）。
//!
//! 逐位对拍金标准（tests 内锁死）由 Java 语义的独立复算生成（python
//! hashlib.md5 + 按上述公式复算，seed=0x7EA200B1）。

/// GOLDEN_RATIO_64（Java 字面量 -7046029254386353131）。
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
/// SILVER_RATIO_64（7640891576956012809）。
const SILVER: u64 = 0x6A09_E667_F3BC_C909;

/// 64 位 long 语义封装（Java 溢出行为 = Rust wrapping 语义）。
type Long = i64;

/// Mth.getSeed(x, y, z)（Mth.java:314-318）。注意 `x * 3129871` 在 Java 里
/// 是 int 乘法（溢出回绕）后才升 long，`z * 116129781L` 是 long 乘法。
#[must_use]
pub fn mth_get_seed(x: i32, y: i32, z: i32) -> Long {
    let a = x.wrapping_mul(3_129_871) as Long;
    let mut seed = a ^ (Long::from(z).wrapping_mul(116_129_781)) ^ Long::from(y);
    seed = seed
        .wrapping_mul(seed)
        .wrapping_mul(42_317_861)
        .wrapping_add(seed.wrapping_mul(11));
    seed >> 16 // Java >> = 算术右移
}

/// upgradeSeedTo128bit（RandomSupport.java:24-31）：lo = seed ^ SILVER、
/// hi = lo + GOLDEN，再各自 mixStafford13。
#[must_use]
pub fn upgrade_seed_to_128bit(seed: Long) -> (u64, u64) {
    let lo = (seed as u64) ^ SILVER;
    let hi = lo.wrapping_add(GOLDEN);
    (mix_stafford13(lo), mix_stafford13(hi))
}

/// Stafford13 终混（RandomSupport.mixStafford13，z^(z≫30)·C1 → z^(z≫27)
/// ·C2 → z^(z≫31)）。
#[must_use]
pub const fn mix_stafford13(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// MD5-128（RFC 1321）→ (lo = 前 8 字节 BE, hi = 后 8 字节 BE)，
/// 对齐 RandomSupport.seedFromHashOf（:36-42，Guava `Longs.fromBytes`
/// = 大端）。
#[must_use]
pub fn seed_from_hash_of(name: &str) -> (u64, u64) {
    let digest = md5(name.as_bytes());
    let lo = u64::from_be_bytes(digest[0..8].try_into().unwrap());
    let hi = u64::from_be_bytes(digest[8..16].try_into().unwrap());
    (lo, hi)
}

/// Xoroshiro128++ 生成器（Xoroshiro128PlusPlus.java:24-36）。
#[derive(Clone, Copy, Debug)]
pub struct Xoroshiro {
    lo: u64,
    hi: u64,
}

impl Xoroshiro {
    /// Seed128bit 直入构造（含全零守卫，Xoroshiro128PlusPlus.java:18-21）。
    #[must_use]
    pub const fn from_seed128(lo: u64, hi: u64) -> Self {
        if lo | hi == 0 {
            Self {
                lo: GOLDEN,
                hi: SILVER,
            }
        } else {
            Self { lo, hi }
        }
    }

    /// `new XoroshiroRandomSource(long seed)` = upgradeSeedTo128bit 直入
    /// （XoroshiroRandomSource.java:22-24）。
    #[must_use]
    pub fn from_world_seed(seed: Long) -> Self {
        let (lo, hi) = upgrade_seed_to_128bit(seed);
        Self::from_seed128(lo, hi)
    }

    #[must_use]
    pub fn next_long(&mut self) -> Long {
        let s0 = self.lo;
        let mut s1 = self.hi;
        let result = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
        s1 ^= s0;
        self.lo = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
        self.hi = s1.rotate_left(28);
        // Java long 域回绕 = u64 wrapping 位型恒等，出口位精确转译。
        result as Long
    }

    /// nextBits(bits) = nextLong() 逻辑右移（XoroshiroRandomSource:159-161）。
    #[must_use]
    fn next_bits(&mut self, bits: u32) -> u64 {
        (self.next_long() as u64) >> (64 - bits)
    }

    /// (float)nextBits(24) · 5.9604645E-8F——**float 域**乘法后按 f32
    /// 舍入（Java 表达式两侧皆 float；调用方与 double 概率比较时才拓宽）。
    #[must_use]
    pub fn next_float(&mut self) -> f32 {
        (self.next_bits(24) as f32) * 5.960_464_5e-8_f32
    }

    /// `nextBits(53) * 1.110223E-16F`（XoroshiroRandomSource.java:93）——
    /// 常量池操作数是 **float** 字面量（反编译带 F 后缀；:12 的 double
    /// 字段声明是 javac 内联假象）：Java 二元数值提升 long×float→float，
    /// 乘积先在 **f32 域**舍入再拓宽返回。f32 quirk 照抄：bits(53) 上端
    /// 舍入到 2⁵³ 时返回恰 1.0（原版同病）；常量值本身恰 = 2⁻⁵³。
    #[must_use]
    pub fn next_double(&mut self) -> f64 {
        ((self.next_bits(53) as f32) * 1.110_223e-16_f32) as f64
    }

    #[must_use]
    pub fn next_int(&mut self) -> i32 {
        self.next_long() as i32
    }

    /// nextInt(bound)：Lemire 无偏归约 + 拒绝循环，逐分支对位
    /// XoroshiroRandomSource.java:48-70（`Integer.remainderUnsigned(~bound+1,
    /// bound)` = `(0u32.wrapping_sub(bound)) % bound`）。
    #[must_use]
    pub fn next_int_bounded(&mut self, bound: u32) -> u32 {
        debug_assert!(bound > 0, "Bound must be positive");
        let mut random_bits = self.next_int() as u32 as u64;
        let mut multiplied = random_bits.wrapping_mul(u64::from(bound));
        let mut frac = multiplied & 0xFFFF_FFFF;
        if frac < u64::from(bound) {
            let unbiased_start = u32::wrapping_sub(0, bound) % bound;
            while frac < u64::from(unbiased_start) {
                random_bits = self.next_int() as u32 as u64;
                multiplied = random_bits.wrapping_mul(u64::from(bound));
                frac = multiplied & 0xFFFF_FFFF;
            }
        }
        (multiplied >> 32) as u32
    }

    /// forkPositional：消耗两次 nextLong 作工厂 (seedLo, seedHi)
    /// （XoroshiroRandomSource.java:38-41）。
    #[must_use]
    pub fn fork_positional(&mut self) -> PositionalRandomFactory {
        PositionalRandomFactory {
            lo: self.next_long() as u64,
            hi: self.next_long() as u64,
        }
    }
}

/// XoroshiroPositionalRandomFactory（XoroshiroRandomSource.java:113-155）。
#[derive(Clone, Copy, Debug)]
pub struct PositionalRandomFactory {
    lo: u64,
    hi: u64,
}

impl PositionalRandomFactory {
    /// at(x,y,z)：Xoroshiro128PlusPlus(Mth.getSeed ^ seedLo, seedHi)。
    #[must_use]
    pub fn at(&self, x: i32, y: i32, z: i32) -> Xoroshiro {
        let pos = mth_get_seed(x, y, z) as u64;
        Xoroshiro::from_seed128(pos ^ self.lo, self.hi)
    }

    /// fromHashOf(name).forkPositional()（RandomState.getOrCreateRandomFactory
    /// 的同型两步）：md5 名种 ⊕ 工厂态 → 生成器 → fork。
    #[must_use]
    pub fn named_factory(&self, name: &str) -> PositionalRandomFactory {
        let (nlo, nhi) = seed_from_hash_of(name);
        let mut g = Xoroshiro::from_seed128(nlo ^ self.lo, nhi ^ self.hi);
        g.fork_positional()
    }

    /// fromHashOf(name) 一步版（XoroshiroPositionalRandomFactory.java:130-134
    /// = Noises.instantiate / PerlinNoise 的 octaves 取随机源用）：返回
    /// 生成器本身（不 fork），Seed128bit 直入仅零态守卫。
    #[must_use]
    pub fn random_from_hash(&self, name: &str) -> Xoroshiro {
        let (nlo, nhi) = seed_from_hash_of(name);
        Xoroshiro::from_seed128(nlo ^ self.lo, nhi ^ self.hi)
    }

    /// 直接以本工厂位置随机 nextDouble（SurfaceSystem.getSurfaceDepth 的
    /// jitter 用法：`noiseRandom.at(x, 0, z).nextDouble()`）。
    #[must_use]
    pub fn double_at(&self, x: i32, y: i32, z: i32) -> f64 {
        self.at(x, y, z).next_double()
    }
}

/// 世界种子 → 根位置随机工厂：`settings.getRandomSource().newInstance(seed)
/// .forkPositional()`（RandomState.java:37；主世界 XOROSHIRO 默认）。
#[must_use]
pub fn root_positional_factory(world_seed: Long) -> PositionalRandomFactory {
    Xoroshiro::from_world_seed(world_seed).fork_positional()
}

/// RFC 1321 MD5（16 B 摘要）。仅需哈希短名字串（"minecraft:xxx"），
/// 无抗碰撞诉求，纯查表实现。
fn md5(msg: &[u8]) -> [u8; 16] {
    // 每左移位数与常量表 K[i] = floor(|sin(i+1)|·2^32)。
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76a_a478,
        0xe8c7_b756,
        0x2420_70db,
        0xc1bd_ceee,
        0xf57c_0faf,
        0x4787_c62a,
        0xa830_4613,
        0xfd46_9501,
        0x6980_98d8,
        0x8b44_f7af,
        0xffff_5bb1,
        0x895c_d7be,
        0x6b90_1122,
        0xfd98_7193,
        0xa679_438e,
        0x49b4_0821,
        0xf61e_2562,
        0xc040_b340,
        0x265e_5a51,
        0xe9b6_c7aa,
        0xd62f_105d,
        0x0244_1453,
        0xd8a1_e681,
        0xe7d3_fbc8,
        0x21e1_cde6,
        0xc337_07d6,
        0xf4d5_0d87,
        0x455a_14ed,
        0xa9e3_e905,
        0xfcef_a3f8,
        0x676f_02d9,
        0x8d2a_4c8a,
        0xfffa_3942,
        0x8771_f681,
        0x6d9d_6122,
        0xfde5_380c,
        0xa4be_ea44,
        0x4bde_cfa9,
        0xf6bb_4b60,
        0xbebf_bc70,
        0x289b_7ec6,
        0xeaa1_27fa,
        0xd4ef_3085,
        0x0488_1d05,
        0xd9d4_d039,
        0xe6db_99e5,
        0x1fa2_7cf8,
        0xc4ac_5665,
        0xf429_2244,
        0x432a_ff97,
        0xab94_23a7,
        0xfc93_a039,
        0x655b_59c3,
        0x8f0c_cc92,
        0xffef_f47d,
        0x8584_5dd1,
        0x6fa8_7e4f,
        0xfe2c_e6e0,
        0xa301_4314,
        0x4e08_11a1,
        0xf753_7e82,
        0xbd3a_f235,
        0x2ad7_d2bb,
        0xeb86_d391,
    ];
    let mut a0: u32 = 0x6745_2301;
    let mut b0: u32 = 0xefcd_ab89;
    let mut c0: u32 = 0x98ba_dcfe;
    let mut d0: u32 = 0x1032_5476;

    // 填充：0x80 + 零 + u64 LE 位长，512 bit 块。
    let bit_len = (msg.len() as u64) << 3;
    let mut padded = msg.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_le_bytes());

    for block in padded.chunks(64) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let tmp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = tmp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MD5 对拍 RFC 1321 官方向量 + Guava 大端取词方向。
    #[test]
    fn md5_matches_rfc_vectors() {
        // md5("hello") = 5d41402abc4b2a76b9719d911017c592
        let (lo, hi) = seed_from_hash_of("hello");
        assert_eq!(lo, 0x5d41_402a_bc4b_2a76);
        assert_eq!(hi, 0xb971_9d91_1017_c592);
        // md5("") = d41d8cd98f00b204e9800998ecf8427e
        let (lo, hi) = seed_from_hash_of("");
        assert_eq!(lo, 0xd41d_8cd9_8f00_b204);
        assert_eq!(hi, 0xe980_0998_ecf8_427e);
    }

    /// 全链金标准（seed=0x7EA200B1，python hashlib.md5 + Java 公式独立
    /// 复算生成）：根工厂 → named 工厂 → at(x,y,z).nextFloat 逐级锁位型。
    #[test]
    fn rng_chain_golden_vectors() {
        let root = root_positional_factory(0x7EA2_00B1_i64);
        assert_eq!(
            (root.lo, root.hi),
            (912_564_408_094_999_842, 18_274_158_727_979_056_502)
        );
        let bedrock = root.named_factory("minecraft:bedrock_floor");
        assert_eq!(
            (bedrock.lo, bedrock.hi),
            (1_310_856_785_949_554_927, 16_309_835_061_261_666_599)
        );
        let deepslate = root.named_factory("minecraft:deepslate");
        assert_eq!(
            (deepslate.lo, deepslate.hi),
            (3_415_402_734_886_139_659, 2_839_082_823_218_483_738)
        );
        let surf = root.named_factory("minecraft:surface");
        assert_eq!(
            (surf.lo, surf.hi),
            (14_330_676_068_613_473_762, 16_635_822_912_546_565_766)
        );
        // at 位值（bedrock_floor）
        let cases: [((i32, i32, i32), u32); 3] = [
            // #192 二修：前两条旧期望值与实物公式（本文件头注链 =
            // Mth.java:314-318 + Xoroshiro128PlusPlus.java:24-36 +
            // XoroshiroRandomSource.java:87-89）的 python 独立复算不符
            // （生成侧 bug，与工厂级 lo/hi 期望互不自洽）；按复算锁位。
            ((3, -62, 7), 0x3f31_5a41),
            ((100, 4, -55), 0x3f38_90f2),
            ((0, 7, 0), 0x3f1d_f1ec),
        ];
        for ((x, y, z), want_bits) in cases {
            let got = bedrock.at(x, y, z).next_float();
            assert_eq!(got.to_bits(), want_bits, "at({x},{y},{z}) 位型漂移");
        }
    }

    /// 输出域哨兵：float ∈ [0,1)、double ∈ [0,1]（原版 f32 舍入 quirk 可
    /// 返回恰 1.0，见 next_double 注）、bounded int < bound 且
    /// 小 bound（256−i 形态，ImprovedNoise 洗牌消费面）全覆盖。
    #[test]
    fn outputs_in_domain() {
        let mut g = Xoroshiro::from_world_seed(12345);
        for _ in 0..256 {
            assert!((0.0..1.0).contains(&g.next_float()));
            let d = g.next_double();
            assert!((0.0..=1.0).contains(&d), "next_double 出 [0,1] 域: {d}");
        }
        let mut g = Xoroshiro::from_world_seed(999);
        for bound in [1u32, 2, 7, 256, 1000] {
            for _ in 0..64 {
                assert!(g.next_int_bounded(bound) < bound);
            }
        }
    }

    /// 全零态守卫（Xoroshiro128PlusPlus.java:18-21）不得输出恒零。
    #[test]
    fn zero_state_guard() {
        let mut g = Xoroshiro::from_seed128(0, 0);
        assert_ne!(g.next_long(), 0);
    }
}
