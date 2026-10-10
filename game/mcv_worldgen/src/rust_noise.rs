//! 纯 Rust 噪声层——`cpp/src/noise.h`（冻结 oracle）的逐位移植。
//!
//! 位一致三原则（与 mesher 移植同纪律，任务板 #78 第一阶段）：
//! 1. 全部浮点保持 f32 单精度，且与 C++ 逐调用点相同的运算序与结合序——
//!    两侧都不做 FMA 收缩（GCC x86-64 基线 ISA 无 FMA 指令，Rust/LLVM 默认
//!    不收缩），IEEE 每步舍入结果由运算序唯一决定；
//! 2. 整数哈希通路全用 wrapping 语义——C++ 无符号回绕是定义行为，Rust
//!    debug 构建溢出 panic，必须显式 `wrapping_*`；
//! 3. 不引入 epsilon：对拍只在「逐位相等」上断言。
//!
//! 位级锚点（splitmix64 已知答案 + f32 位模式向量）锁在本模块 `#[cfg(test)]`；
//! 向量由独立 IEEE 单精度逐运算仿真复算生成（与 C++ 同序），全链路正确性
//! 由 tests/parity.rs 的 C++ oracle 逐字节对拍兜底。出处标注 `noise.h:行号`
//! （引用出处 ≠ 复制表达，docs/porting-conventions.md §1/§3）。

/// splitmix64 终混合（noise.h:22-27）。
#[must_use]
pub fn splitmix64(x: u64) -> u64 {
    let x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// [0, 1) 网格哈希（noise.h:29-34）。i64 分量符号扩展为 u64（C++ 隐式
/// 转换语义），乘法回绕。尾段 `(h >> 40) as f32 * 2^-24`：整数 < 2^24 转
/// f32 精确、乘 2 的幂只调指数，两步均无舍入 → 全平台位一致。
#[must_use]
pub fn hash01(seed: u64, x: i64, y: i64, z: i64) -> f32 {
    let h = splitmix64(
        seed ^ (x as u64).wrapping_mul(0x9E37_79B1)
            ^ (y as u64).wrapping_mul(0x85EB_CA77)
            ^ (z as u64).wrapping_mul(0xC2B2_AE3D),
    );
    ((h >> 40) as f32) * (1.0_f32 / 16_777_216.0)
}

/// 五次平滑插值曲线（noise.h:36-38）。
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0_f32 - 15.0_f32) + 10.0_f32)
}

/// 线性插值，运算序同 noise.h:40（先差、再乘、后加）。
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 2D 值噪声，[0, 1)（noise.h:43-55）。
#[must_use]
pub fn value2(seed: u64, x: f32, y: f32) -> f32 {
    let fx = x.floor();
    let fy = y.floor();
    let tx = fade(x - fx);
    let ty = fade(y - fy);
    let ix = fx as i64;
    let iy = fy as i64;
    let c00 = hash01(seed, ix, iy, 0);
    let c10 = hash01(seed, ix + 1, iy, 0);
    let c01 = hash01(seed, ix, iy + 1, 0);
    let c11 = hash01(seed, ix + 1, iy + 1, 0);
    lerp(lerp(c00, c10, tx), lerp(c01, c11, tx), ty)
}

/// 3D 值噪声，[0, 1)（noise.h:58-79）。
#[must_use]
pub fn value3(seed: u64, x: f32, y: f32, z: f32) -> f32 {
    let fx = x.floor();
    let fy = y.floor();
    let fz = z.floor();
    let tx = fade(x - fx);
    let ty = fade(y - fy);
    let tz = fade(z - fz);
    let ix = fx as i64;
    let iy = fy as i64;
    let iz = fz as i64;
    let c000 = hash01(seed, ix, iy, iz);
    let c100 = hash01(seed, ix + 1, iy, iz);
    let c010 = hash01(seed, ix, iy + 1, iz);
    let c110 = hash01(seed, ix + 1, iy + 1, iz);
    let c001 = hash01(seed, ix, iy, iz + 1);
    let c101 = hash01(seed, ix + 1, iy, iz + 1);
    let c011 = hash01(seed, ix, iy + 1, iz + 1);
    let c111 = hash01(seed, ix + 1, iy + 1, iz + 1);
    let a = lerp(lerp(c000, c100, tx), lerp(c010, c110, tx), ty);
    let b = lerp(lerp(c001, c101, tx), lerp(c011, c111, tx), ty);
    lerp(a, b, tz)
}

/// 每倍频种子步进（noise.h:89/104/125/141：`i * 0x1000'0000`）。
const OCTAVE_SEED_STEP: u64 = 0x1000_0000;

/// 分形布朗运动（2D），[0, 1)（noise.h:84-96）。倍频语义：频率 ×2、值权重
/// ×0.5；语句序（累加 → 归一化累加 → 衰减 → 坐标倍频）与 C++ 逐行一致。
#[must_use]
pub fn fbm2(seed: u64, mut x: f32, mut y: f32, octaves: usize) -> f32 {
    let mut sum = 0.0_f32;
    let mut amp = 1.0_f32;
    let mut norm = 0.0_f32;
    for i in 0..octaves {
        sum += value2(
            seed.wrapping_add((i as u64).wrapping_mul(OCTAVE_SEED_STEP)),
            x,
            y,
        ) * amp;
        norm += amp;
        amp *= 0.5_f32;
        x *= 2.0_f32;
        y *= 2.0_f32;
    }
    sum / norm
}

/// 分形布朗运动（3D），[0, 1)（noise.h:99-112）。
#[must_use]
pub fn fbm3(seed: u64, mut x: f32, mut y: f32, mut z: f32, octaves: usize) -> f32 {
    let mut sum = 0.0_f32;
    let mut amp = 1.0_f32;
    let mut norm = 0.0_f32;
    for i in 0..octaves {
        sum += value3(
            seed.wrapping_add((i as u64).wrapping_mul(OCTAVE_SEED_STEP)),
            x,
            y,
            z,
        ) * amp;
        norm += amp;
        amp *= 0.5_f32;
        x *= 2.0_f32;
        y *= 2.0_f32;
        z *= 2.0_f32;
    }
    sum / norm
}

/// 振幅序列版 fbm（2D），[0, 1)（noise.h:118-132）。第 i 倍频权重 =
/// `amps[i] * 0.5^i`；`amps[i] == 0` 的倍频照常推进坐标/种子（与 C++ 相同，
/// 权重 0 不改变 sum/norm 位型）。
#[must_use]
pub fn fbm2_w(seed: u64, mut x: f32, mut y: f32, amps: &[f32]) -> f32 {
    let mut sum = 0.0_f32;
    let mut norm = 0.0_f32;
    let mut amp = 1.0_f32;
    for (i, a) in amps.iter().enumerate() {
        let w = *a * amp;
        sum += value2(
            seed.wrapping_add((i as u64).wrapping_mul(OCTAVE_SEED_STEP)),
            x,
            y,
        ) * w;
        norm += w;
        amp *= 0.5_f32;
        x *= 2.0_f32;
        y *= 2.0_f32;
    }
    sum / norm
}

/// 振幅序列版 fbm（3D），[0, 1)（noise.h:134-149）。
#[must_use]
pub fn fbm3_w(seed: u64, mut x: f32, mut y: f32, mut z: f32, amps: &[f32]) -> f32 {
    let mut sum = 0.0_f32;
    let mut norm = 0.0_f32;
    let mut amp = 1.0_f32;
    for (i, a) in amps.iter().enumerate() {
        let w = *a * amp;
        sum += value3(
            seed.wrapping_add((i as u64).wrapping_mul(OCTAVE_SEED_STEP)),
            x,
            y,
            z,
        ) * w;
        norm += w;
        amp *= 0.5_f32;
        x *= 2.0_f32;
        y *= 2.0_f32;
        z *= 2.0_f32;
    }
    sum / norm
}

/// 山脊/河谷折叠（noise.h:154-156）：输入 [-1,1] → 输出 [-1,1]，|r| = 2/3
/// 处出 +1 脊线。逐系数与 C++ 一致（2/3、1/3 均为 f32 常量折叠）。
#[must_use]
pub fn peaks_valleys(r: f32) -> f32 {
    -3.0_f32 * ((r.abs() - (2.0_f32 / 3.0_f32)).abs() - (1.0_f32 / 3.0_f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 锚点 seed：任意取定，向量一经生成就冻结。
    const SEED: u64 = 0x1234_5678_9ABC_DEF0;

    /// splitmix64 已知答案向量（纯整数运算，与 f32 无关）。
    #[test]
    fn splitmix64_known_answers() {
        assert_eq!(splitmix64(0), 0xe220_a839_7b1d_cdaf);
        assert_eq!(splitmix64(1), 0x910a_2dec_8902_5cc1);
        assert_eq!(splitmix64(0xDEAD_BEEF), 0x4adf_b90f_68c9_eb9b);
        assert_eq!(splitmix64(0x7EE5_7EE5_7EE5_7EE5), 0x76e9_1ee8_817e_4720);
    }

    /// hash01 位锚点：i64 负分量覆盖符号扩展路径。
    #[test]
    fn hash01_bit_anchors() {
        assert_eq!(hash01(SEED, 5, -3, 12).to_bits(), 0x3f79_2fa0);
        assert_eq!(hash01(SEED, 0, 0, 0).to_bits(), 0x3db0_c910);
        assert_eq!(hash01(SEED, 2, 1, 7).to_bits(), 0x3d42_8f60);
        assert_eq!(hash01(SEED, -1, 0, 1).to_bits(), 0x3d1c_b0d0);
    }

    /// value2/fbm2(3 倍频) 位锚点。
    #[test]
    fn value2_fbm2_bit_anchors() {
        assert_eq!(value2(SEED, 3.7, -11.25).to_bits(), 0x3f07_670c);
        assert_eq!(value2(SEED, 0.031_25, 7.5).to_bits(), 0x3efb_fbd1);
        assert_eq!(value2(SEED, -190.5, 88.125).to_bits(), 0x3eaa_35e3);
        assert_eq!(value2(SEED, 1.0, 1.0).to_bits(), 0x3e3b_7c90);
        assert_eq!(fbm2(SEED, 3.7, -11.25, 3).to_bits(), 0x3ed1_46ab);
        assert_eq!(fbm2(SEED, 0.031_25, 7.5, 3).to_bits(), 0x3f26_625d);
        assert_eq!(fbm2(SEED, -190.5, 88.125, 3).to_bits(), 0x3f01_ea0b);
        assert_eq!(fbm2(SEED, 1.0, 1.0, 3).to_bits(), 0x3f06_e0cb);
    }

    /// fbm2_w 位锚点：terrain.cpp 四个振幅序列各一例。
    #[test]
    fn fbm2_w_bit_anchors() {
        let cont = [1.0_f32, 1.0, 2.0, 2.0, 2.0, 1.0];
        let erosion = [1.0_f32, 1.0, 0.0, 1.0, 1.0];
        let ridge = [1.0_f32, 2.0, 1.0];
        let cheese = [0.5_f32, 1.0, 2.0, 1.0, 2.0];
        assert_eq!(fbm2_w(SEED, 3.7, -11.25, &cont).to_bits(), 0x3ed6_c974);
        assert_eq!(fbm2_w(SEED, 0.031_25, 7.5, &erosion).to_bits(), 0x3f16_3b6e);
        assert_eq!(fbm2_w(SEED, -190.5, 88.125, &ridge).to_bits(), 0x3f0d_c3c9);
        assert_eq!(fbm2_w(SEED, 1.0, 1.0, &cheese).to_bits(), 0x3f2e_f241);
    }

    /// value3/fbm3(2 倍频)/fbm3_w 位锚点。
    #[test]
    fn value3_fbm3_bit_anchors() {
        let cheese = [0.5_f32, 1.0, 2.0, 1.0, 2.0];
        assert_eq!(value3(SEED, 0.6, -3.1, 44.0).to_bits(), 0x3eb4_5aca);
        assert_eq!(value3(SEED, 7.9, 2.2, -1.5).to_bits(), 0x3efc_19dd);
        assert_eq!(value3(SEED, -13.0, 0.5, 3.25).to_bits(), 0x3f04_f072);
        assert_eq!(fbm3(SEED, 0.6, -3.1, 44.0, 2).to_bits(), 0x3f04_6789);
        assert_eq!(fbm3(SEED, 7.9, 2.2, -1.5, 2).to_bits(), 0x3f04_7a1d);
        assert_eq!(fbm3(SEED, -13.0, 0.5, 3.25, 2).to_bits(), 0x3ee5_adc8);
        assert_eq!(
            fbm3_w(SEED, 0.6, -3.1, 44.0, &cheese).to_bits(),
            0x3f17_2620
        );
        assert_eq!(fbm3_w(SEED, 7.9, 2.2, -1.5, &cheese).to_bits(), 0x3ee4_285e);
        assert_eq!(
            fbm3_w(SEED, -13.0, 0.5, 3.25, &cheese).to_bits(),
            0x3f13_c191
        );
    }

    /// peaks_valleys 位锚点：脊线 2/3 两侧、0 点、折叠平台。
    #[test]
    fn peaks_valleys_bit_anchors() {
        assert_eq!(peaks_valleys(0.0).to_bits(), 0xbf80_0000);
        assert_eq!(peaks_valleys(0.5).to_bits(), 0x3eff_ffff);
        assert_eq!(peaks_valleys(-0.9).to_bits(), 0x3e99_999f);
        assert_eq!(peaks_valleys(2.0_f32 / 3.0_f32).to_bits(), 0x3f80_0000);
        assert_eq!(peaks_valleys(-2.0_f32 / 3.0_f32).to_bits(), 0x3f80_0000);
        assert_eq!(peaks_valleys(1.0).to_bits(), 0x33c0_0000);
    }

    /// 值域不变量（对锚点的补充哨兵，不作对拍替代）。peaks_valleys 只对
    /// 定义域 [-1,1] 保证值域（terrain 侧输入是 fbm2_w*2-1 ∈ [-1,1]），故
    /// 此处以 r=v/64 ∈ [-1,1] 喂入；value2/value3 对任意输入都有界。
    #[test]
    fn ranges_hold() {
        for i in -64..=64 {
            let v = f32::from(i16::try_from(i).unwrap());
            let x = v * 0.371;
            let y = v * -1.13;
            let z = v * 2.717;
            let h = hash01(SEED, i64::from(i), i64::from(i * 3), i64::from(i * -7));
            assert!((0.0..1.0).contains(&h));
            assert!((0.0..1.0).contains(&value2(SEED, x, y)));
            assert!((0.0..1.0).contains(&value3(SEED, x, y, z)));
            assert!((-1.0..=1.0).contains(&peaks_valleys(v / 64.0)));
        }
    }
}
