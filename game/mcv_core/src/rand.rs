//! 原版随机源底座（clean-room 转写，语义对照 26.1 实物）：
//!
//! - [`mth_get_seed`]：`Mth.getSeed(int,int,int)` 位打包
//!   （Mth.java:314-318——`x*3129871` 先按 **int** 溢出回绕再符号扩展进
//!   long,`seed*seed*42317861L + seed*11L`,算术 `>>16`）。
//! - [`Lcg`]：`LegacyRandomSource`/`SingleThreadedRandomSource` 同一颗
//!   java.util.Random LCG（LegacyRandomSource.java:39-47、
//!   SingleThreadedRandomSource.java:36-50、BitRandomSource.java 的
//!   `nextInt(bound)`/`nextFloat` 默认实现）。
//!
//! 注：26.1 实物 `RandomSource.create(long)` 返回的是 **LegacyRandomSource**
//! （RandomSource.java:24-26）——`Xoroshiro128PlusPlusRandomSource`/
//! `RandomSharedConstants` 在 26.1 代码树中不存在（`Xoroshiro128PlusPlus`
//! 仅余 levelgen 内部的 nextLong 组件，不挂在 `RandomSource.create` 路径
//! 上）。结构模板 `StructurePlaceSettings.getRandom` 走的正是
//! `RandomSource.create(Mth.getSeed(pos))`,故本模块 LCG 即结构掷骰实物。

/// `Mth.getSeed(x, y, z)`（Mth.java:314-318）。返回值按 Java `long`
/// 位型保存（算术右移后的结果可能为负）。
#[must_use]
pub fn mth_get_seed(x: i32, y: i32, z: i32) -> i64 {
    // x * 3129871 在 Java 是 int 乘法:32 位回绕后再隐式转 long(符号扩展)。
    let a = x.wrapping_mul(3129871) as i64;
    let b = (z as i64).wrapping_mul(116_129_781);
    let mut seed = a ^ b ^ (y as i64);
    seed = seed
        .wrapping_mul(seed)
        .wrapping_mul(42_317_861)
        .wrapping_add(seed.wrapping_mul(11));
    // 算术右移(与 Java `>>` 一致,保持符号)。
    seed >> 16
}

const MASK48: u64 = 281_474_976_710_655; // 2^48 - 1
const MULT: u64 = 25_214_903_917; // 0x5DEECE66D
const MUL_F32: f32 = 5.960_464_5e-8; // BitRandomSource.FLOAT_MULTIPLIER = 2^-24

/// java.util.Random 家族 LCG(LegacyRandomSource.java:12-47;
/// SingleThreadedRandomSource.java:44-50 与之逐位同)。
#[derive(Debug, Clone)]
pub struct Lcg {
    state: u64,
}

impl Lcg {
    /// `setSeed(seed)`:`(seed ^ 0x5DEECE66D) & (2^48-1)`
    /// (LegacyRandomSource.java:33-36)。
    #[must_use]
    pub fn new(seed: i64) -> Self {
        Self {
            state: ((seed as u64) ^ MULT) & MASK48,
        }
    }

    /// `next(bits)`(LegacyRandomSource.java:39-47):先推进,取高位;
    /// Java `(int)(newSeed >> 48 - bits)` 截低 32 位。
    pub fn next(&mut self, bits: u32) -> i32 {
        self.state = self.state.wrapping_mul(MULT).wrapping_add(11) & MASK48;
        ((self.state >> (48 - bits)) & 0xFFFF_FFFF) as u32 as i32
    }

    /// `nextInt()` = `next(32)`(BitRandomSource.java:13-15)。
    pub fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    /// `nextInt(bound)`(BitRandomSource.java:17-35):2 的幂走乘法支,
    /// 否则拒绝采样。bound ≤ 0 按原版抛错语义 panic(消费方保证正界)。
    pub fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "Bound must be positive");
        if bound & (bound - 1) == 0 {
            // (int)((long)bound * next(31) >> 31)
            return ((((bound as i64) * self.next(31) as i64) >> 31) & 0xFFFF_FFFF) as u32 as i32;
        }
        loop {
            let sample = self.next(31);
            let modulo = sample % bound;
            if sample - modulo + (bound - 1) >= 0 {
                return modulo;
            }
        }
    }

    /// `nextFloat()` = `next(24) * 2^-24`(BitRandomSource.java:47-49)。
    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * MUL_F32
    }

    /// `nextLong()`(BitRandomSource.java:37-42):`next(32)<<32 + next(32)`
    /// (注意是**加**不是或,与 Java 一致)。
    pub fn next_long(&mut self) -> i64 {
        let upper = self.next(32) as i64;
        let lower = self.next(32) as i64;
        (upper << 32).wrapping_add(lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mth_get_seed_bit_pack() {
        // 期望值由独立实现(Java 语义逐操作的 python 转写)推导,
        // 再与参考实现交叉核对。
        assert_eq!(mth_get_seed(0, 0, 0), 0);
        assert_eq!(mth_get_seed(1, 2, 3), -33_674_130_277_896);
        assert_eq!(mth_get_seed(-7, 100, -999), -31_132_894_333_382);
        assert_eq!(mth_get_seed(-7, 100, -999), mth_get_seed_ref(-7, 100, -999));
        // 大坐标 int 溢出回绕:x=i32::MAX 时 x*3129871 按 32 位回绕。
        assert_eq!(
            mth_get_seed(i32::MAX, i32::MIN, i32::MAX),
            mth_get_seed_ref(i32::MAX, i32::MIN, i32::MAX)
        );
        assert_eq!(
            mth_get_seed(i32::MAX, i32::MIN, i32::MAX),
            71_483_002_586_549
        );
    }

    /// 参考实现:按 Java 表达式逐字直译(u64 位型),与被测实现互相独立
    /// 地走 int 回绕路径。
    fn mth_get_seed_ref(x: i32, y: i32, z: i32) -> i64 {
        let a = (x as u32).wrapping_mul(3_129_871) as i32 as i64;
        let b = (z as i64).wrapping_mul(116_129_781);
        let seed = (a ^ b ^ (y as i64)) as u64;
        let s1 = seed.wrapping_mul(seed);
        let s2 = s1.wrapping_mul(42_317_861);
        let s3 = seed.wrapping_mul(11);
        ((s2.wrapping_add(s3)) as i64) >> 16
    }

    #[test]
    fn lcg_java_random_vectors() {
        // 期望值:python 逐字转写 LegacyRandomSource.java:39-47 推导。
        let mut r = Lcg::new(0);
        assert_eq!(r.next_int(), -1_155_484_576); // seed=0 两次 next(32)
        assert_eq!(r.next_int(), -723_955_400);
        let mut r = Lcg::new(0);
        assert_eq!(r.next(24), 12_263_604);
        // next_float = next(24) × 2^-24(BitRandomSource.java:47-49,首轮同位)。
        let mut r2 = Lcg::new(0);
        let f = r2.next_float();
        assert!(
            (f - 12_263_604.0f32 / 16_777_216.0f32).abs() <= f32::EPSILON,
            "{f}"
        );
        let mut r = Lcg::new(0);
        assert_eq!(r.next_long(), -4_962_768_465_676_381_896);
    }

    #[test]
    fn lcg_next_float_in_unit_interval() {
        let mut r = Lcg::new(12345);
        for _ in 0..1000 {
            let f = r.next_float();
            assert!((0.0..1.0).contains(&f), "{f}");
        }
    }

    #[test]
    fn lcg_next_int_bound_semantics() {
        // bound=1:2 的幂支,(1 * next(31)) >> 31 恒 0,且消耗一次 next(31)。
        let mut r = Lcg::new(7);
        for _ in 0..10 {
            assert_eq!(r.next_int_bounded(1), 0);
        }
        // 非 2 的幂:拒绝采样值域。
        let mut r = Lcg::new(7);
        for _ in 0..1000 {
            let v = r.next_int_bounded(3);
            assert!((0..3).contains(&v), "{v}");
        }
    }
}
