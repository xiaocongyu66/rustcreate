//! vanilla 地形噪声层 — clean-room 对齐 26.1 `synth/PerlinNoise.java` 的
//! fBm 递推语义与 `synth/NormalNoise.java` 的双层独立语义；底层格点噪声为
//! [`crate::vanilla::perlin::ImprovedNoise`]（26.1 `synth/ImprovedNoise.java`
//! 的忠实机制移植，地形保真波第四棒换装——见该模块头注的取证链）。
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
//! ## 内核换代史（σ 校准常数的退役）
//! v1：底层为 vendored OpenSimplex2S（`crate::os2s::smooth`，保留在库内，
//! 由 `tests/os2s_golden`/`os2s_parity` 位型锁定），因 OS2S 单倍频 σ 高于
//! ImprovedNoise（0.36002 vs 0.27106，×1.3317），曾以逐倍频乘 σ 比值常数
//! 校准（2D 切片 0.73744 / 3D 体积 0.75291）。第四棒取证证明**σ 相等 ≠
//! 谱相等**：校准只对齐值分布，不还原 ImprovedNoise 的空间相关结构——
//! climate 通道窗内 std（plains_share 0.066<0.10）与 BlendedNoise 相邻差分
//! 谱（p90 0.0550>0.05）两门双红的共同根因。内核换成原版机制本体后，
//! value_factor 归一（NormalNoise.java:63，以 ImprovedNoise 单倍频统计为
//! 基准）恢复设计语义，σ 校准常数随内核一并退役（引用失效即删，避免死码）。
//!
//! ## 种子链换代史（v1 splitmix 域盐 → 原版 fork 链）
//! v1：倍频种子以自有 splitmix 域盐派生（root + i·2^28，与 legacy noise.h
//! 同式），第二层独立种子以一层种子的 splitmix 终混派生——AQUI-1 登记
//! 偏差（首证报告 §3 实测对官方 jar 0/12 同点同值、|d|≈0.23~0.94）。
//! 切链刀（noise-vanilla-fork）后 router 全通道（climate/aquifer/terrain
//! density）改走 [`Self::from_vanilla_name`] 原版 fork 链：python 镜像 ×
//! 官方 26.1 jar `--probe-noise` 探针 **3 seed × 10 通道 × 12 点
//! 360/360 位型同、max|d|=0**（tools/fork-chain-mirror-run.txt）。
//! v1 构造随引用失效删除；唯一残余 = [`crate::vanilla::blended`] 的
//! clean-room 种子（`noise_seed`，BlendedNoise 走 legacy 初始化路径
//! PerlinNoise.createLegacyForBlendedNoise，实物化随地形保真后续刀）。

use crate::vanilla::perlin::ImprovedNoise;

/// splitmix64 终混合（复用 legacy 内核同源常数）。
#[must_use]
pub const fn splitmix64(x: u64) -> u64 {
    let x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// clean-room 种子派生：世界种子 ^ 域盐 → 终混合。现仅 [`crate::vanilla::
/// blended`]（BlendedNoise v1 种子，头注「种子链换代史」残余项）使用；
/// router NormalNoise 通道已切原版 fork 链（[`Self::from_vanilla_name`]），
/// 不再经过本函数。
#[must_use]
pub const fn noise_seed(world_seed: u64, salt: u64) -> u64 {
    splitmix64(world_seed ^ salt)
}

/// NormalNoise：六参数 density/气候通道用的双层独立 fBm 噪声。
///
/// 构造走原版 fork 链 [`Self::from_vanilla_name`]：`name` 为原版注册名
/// （Noises.java createKey 实名，含 `minecraft:` 前缀）；`first_octave` 为
/// 26.1 noise json 的 firstOctave（最低倍频波长 = 2^(-firstOctave) 格）；
/// `amplitudes` 为 amplitude 表（**逐字抄 json 含尾部零填充**——表长进
/// PerlinNoise.lowestFreqValueFactor = 2^(n−1)/(2^n−1) 归一）。`x/y/z` 为
/// 已乘通道 xz/y 缩放的 density 坐标（f64）。
///
/// 2D 通道（气候）：vanilla `DensityFunctions.shiftedNoise2d` 的 yScale=0
/// 使求值坐标恒为 y=0（DensityFunctions.java:159-163 + ShiftedNoise.compute）——
/// ImprovedNoise 直接按 (x, 0, z) 采样即原版语义，无需任何切片校准。
#[derive(Clone, Debug)]
pub struct NormalNoise {
    /// 第一层/第二层的逐倍频实例（PerlinNoise 每倍频持有一个 ImprovedNoise；
    /// 零振幅倍频为不参与求值的占位实例，PerlinNoise.java:90-95 同跳过）。
    first: Vec<ImprovedNoise>,
    second: Vec<ImprovedNoise>,
    first_octave: i32,
    amplitudes: &'static [f64],
    value_factor: f64,
    max_value: f64,
}

impl NormalNoise {
    /// **原版种子链**构造——本仓 router 全通道（climate/aquifer/terrain
    /// density）的构造路径（切链刀，原 v1 `NormalNoise::new` splitmix 域盐
    /// 构造已删，见头注「种子链换代史」）：
    /// `RandomState.getOrCreateNoise(key)` = `Noises.instantiate` =
    /// `NormalNoise.create(root.fromHashOf("minecraft:{name}"), params)`
    /// （Noises.java:78-83；`root` = `root_positional_factory(worldSeed)`
    /// = RandomState.java:36）。逐 fork 对位：
    /// - NormalNoise.java:51-53：`first`/`second` = 同一随机源先后两次
    ///   `PerlinNoise.create`（每次开首 `random.forkPositional()` 消耗 2
    ///   个 nextLong——PerlinNoise.java:87）；
    /// - PerlinNoise.java:88-96：每个**非零** amplitude 的倍频 =
    ///   `ImprovedNoise(positional.fromHashOf("octave_" + octave))`，
    ///   零 amplitude 倍频**不消耗**随机数（无 else 分支）；
    /// - ImprovedNoise 构造流见 [`ImprovedNoise::from_xoroshiro`]。
    ///   同 world_seed 下排列/原点平移与原版同位型（内核求值 = 既有
    ///   ImprovedNoise 机制转写，口径与其头注一致）。
    #[must_use]
    pub fn from_vanilla_name(
        root: &crate::vanilla::xoroshiro::PositionalRandomFactory,
        name: &str,
        first_octave: i32,
        amplitudes: &'static [f64],
    ) -> Self {
        let mut source = root.random_from_hash(name);
        let build = |source: &mut crate::vanilla::xoroshiro::Xoroshiro| {
            let positional = source.fork_positional();
            amplitudes
                .iter()
                .enumerate()
                .map(|(i, &a)| {
                    if a != 0.0 {
                        let octave = first_octave + i as i32;
                        ImprovedNoise::from_xoroshiro(
                            &mut positional.random_from_hash(&format!("octave_{octave}")),
                        )
                    } else {
                        // 零 amplitude：原版不建实例也不消耗随机数；本结构
                        // 按索引 zip 求值恒跳过，占位值不参与任何输出。
                        ImprovedNoise::new(0)
                    }
                })
                .collect::<Vec<_>>()
        };
        let first = build(&mut source);
        let second = build(&mut source);
        let (value_factor, max_value) = factors(amplitudes);
        Self {
            first,
            second,
            first_octave,
            amplitudes,
            value_factor,
            max_value,
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
        let first = self.one_perlin(&self.first, x, y, z);
        let second = self.one_perlin(&self.second, x * d, y * d, z * d);
        (first + second) * self.value_factor
    }

    #[must_use]
    fn one_perlin(&self, octaves: &[ImprovedNoise], x: f64, y: f64, z: f64) -> f64 {
        let mut value = 0.0_f64;
        let mut factor = 2.0_f64.powi(self.first_octave);
        let mut value_factor = 2.0_f64.powi(self.amplitudes.len() as i32 - 1)
            / (2.0_f64.powi(self.amplitudes.len() as i32) - 1.0);
        for (amp, inst) in self.amplitudes.iter().zip(octaves) {
            if *amp != 0.0 {
                let nx = wrap(x * factor);
                let ny = wrap(y * factor);
                let nz = wrap(z * factor);
                value += amp * inst.noise(nx, ny, nz) * value_factor;
            }
            factor *= 2.0;
            value_factor /= 2.0;
        }
        value
    }
}

/// value_factor / edgeValue 归一（from_vanilla_name 用）：
/// expectedDeviation 按「非零倍频的下标跨度」计（NormalNoise.java:59-70 扫
/// 非零 amplitude 求 min/maxOctave，:97-103 expectedDeviation(span)），不是
/// amplitudes.len()——temperature [1.5,0,1] 这类含零表的通道若按 len 计会
/// 系统性抬高 σ。全零表不在 26.1 数据集中，防御按 0。
fn factors(amplitudes: &[f64]) -> (f64, f64) {
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
    let octaves = amplitudes.len();
    let mut edge = 0.0_f64;
    let mut vf = 2.0_f64.powi(octaves as i32 - 1) / (2.0_f64.powi(octaves as i32) - 1.0);
    let mut i = 0;
    while i < octaves {
        edge += amplitudes[i] * 2.0 * vf;
        vf /= 2.0;
        i += 1;
    }
    (value_factor, edge)
}

/// 巨坐标封装（PerlinNoise.wrap:188-190）。
#[must_use]
fn wrap(x: f64) -> f64 {
    let d = 3.355_443_2_e7_f64;
    x - (x / d + 0.5).floor() * d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vanilla::xoroshiro::root_positional_factory;

    /// 通道构造助手（root 工厂对位 RandomState.java:36；fromHashOf 无状态，
    /// 每测试各建一份根工厂与原版每 RandomState 一份同值）。
    fn chan(world_seed: i64, name: &str, fo: i32, amps: &'static [f64]) -> NormalNoise {
        NormalNoise::from_vanilla_name(&root_positional_factory(world_seed), name, fo, amps)
    }

    /// 值域哨兵：fBm 输出在 ±max_value 内，且不同名的通道互不相关
    ///（同坐标抽查均值不为 1）。
    #[test]
    fn normal_noise_stays_in_domain() {
        let a = chan(
            7,
            "minecraft:probe_a",
            -9,
            &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0],
        );
        let b = chan(7, "minecraft:probe_b", -10, &[1.5, 0.0, 1.0, 0.0, 0.0, 0.0]);
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
        assert!(same < 8, "不同名通道疑似同种子：{same}/64 全同");
    }

    /// 确定性：同 seed 同坐标两次求值位型一致（含重建一遍）。
    #[test]
    fn normal_noise_deterministic() {
        let n = chan(
            42,
            "minecraft:probe_det",
            -9,
            &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0],
        );
        let m = chan(
            42,
            "minecraft:probe_det",
            -9,
            &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0],
        );
        let a = n.get_value(1.23, 4.56, 7.89);
        let b = n.get_value(1.23, 4.56, 7.89);
        assert_eq!(a.to_bits(), b.to_bits());
        assert_eq!(a.to_bits(), m.get_value(1.23, 4.56, 7.89).to_bits());
    }

    /// 第二层必须是独立实例（NormalNoise.java:51-53 两次 create 各自
    /// forkPositional；XoroshiroRandomSource.java:38-40 每次 fork 消耗状态
    /// → 工厂种子不同）：根种子与逐倍频种子流都不得与第一层重合。
    /// 旧实现两层共用 self.seed，第二层 = 同一场 1.018× 重采样（近原点
    /// ρ≈0.95），通道 σ 系统性抬高约 1.4×（曾把 River 群系占比压到带外）。
    #[test]
    fn second_layer_uses_independent_root_seed() {
        let n = chan(2024, "minecraft:probe_layers", -7, &[1.0, 1.0, 1.0]);
        // 两层实例由不同 fork 工厂派生（NormalNoise.java:43-44 两次 create）：
        // 逐倍频求值位型必须两两不同（同种子会逐位重合）。
        for i in 0..n.amplitudes.len() {
            let a = n.first[i].noise(1.0, 2.0, 3.0);
            let b = n.second[i].noise(1.0, 2.0, 3.0);
            assert_ne!(a.to_bits(), b.to_bits(), "第 {i} 倍频两层实例疑似同种子");
        }
    }

    /// expectedDeviation 以非零倍频的下标跨度计（NormalNoise.java:59-70 扫
    /// 非零 amplitude，:97-103 expectedDeviation(maxOctave−minOctave)），
    /// 非 amplitudes.len()：[1,1,0] 的 span=1 → 0.1·(1+1/2)≈0.15
    /// （value_factor ≈ 1.111；若误用 len=3 → 0.125 → 1.333，必被此断言识破）。
    #[test]
    fn expected_deviation_uses_nonzero_octave_span() {
        let n = chan(7, "minecraft:probe_span", -8, &[1.0, 1.0, 0.0]);
        let want = 0.166_666_666_666_666_66_f64 / 0.15_f64;
        let got = n.value_factor;
        assert!(
            (got - want).abs() < 1e-12,
            "value_factor 偏离 span 语义期望 {want}（实得 {got}）"
        );
    }

    /// 原版 fork 链位型金标准——python 镜像（tools/fork-chain-mirror.py，
    /// 复刻本文件 from_vanilla_name 派生链）× java-harness 官方 26.1 jar
    /// `--probe-noise` 探针（Main.java:117-131 getOrCreateNoise 实物链）：
    /// 3 seed × 10 通道 × 12 点 **360/360 位型同、max|d|=0**
    /// （tools/fork-chain-mirror-run.txt）。此处锁 3 个代表点位，防种子链
    /// 回归（向量生成口径同 xoroshiro.rs 金标准：python 独立复算）。
    #[test]
    fn fork_chain_matches_official_jar_probe_bits() {
        // (通道名, 世界种子, firstOctave, 振幅表, 探针点 (x,y,z), 期望位型)
        let cases: [(&str, i64, i32, &[f64], (f64, f64, f64), u64); 3] = [
            (
                "minecraft:temperature",
                1,
                -10,
                &[1.5, 0.0, 1.0, 0.0, 0.0, 0.0],
                (17.0, -32.0, 25.0),
                0x3f4c_d93c_5322_c100,
            ),
            (
                "minecraft:ridge",
                123,
                -7,
                &[1.0, 2.0, 1.0, 0.0, 0.0, 0.0],
                (32.0, 3.0, 50.0),
                0x3fa0_e705_8a46_51ec,
            ),
            (
                "minecraft:aquifer_barrier",
                2026_0114,
                -3,
                &[1.0],
                (50.0, 45.0, 80.0),
                0xbfd6_5a5a_e129_4f1b,
            ),
        ];
        for (name, seed, fo, amps, (x, y, z), want) in cases {
            let v = chan(seed, name, fo, amps).get_value(x, y, z);
            assert_eq!(
                v.to_bits(),
                want,
                "{name} seed={seed} 位型漂移（got 0x{:016x} decimal={v}）",
                v.to_bits()
            );
        }
    }
}
