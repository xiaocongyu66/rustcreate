//! OpenSimplex2S（vendored 内核）↔ 官方 Java 参考实现逐位对拍——地形 2.0
//! （任务板 #91）第一道硬门禁。
//!
//! 参考向量由开发机上编译运行的官方 Java 类（KdotJPG/OpenSimplex2
//! `java/OpenSimplex2S.java` @ `4cd120d3`）产出并固化在
//! `tests/os2s_golden.rs`（65 KB，5120 样本）。点流与坐标类别由测试侧用
//! 与 Java 参考程序完全相同的整数运算确定性复现（LCG wrapping 乘加、
//! `(state >>> 11) * 2^-53` 精确转 double），因此本文件不含坐标数据、
//! 只有 5120 个 f32 位型断言。
//!
//! 精度路线：全维度 2D/3D/4D 共 10 个公开方法，**f32 输出逐位相等**
//!（无 ULP 容差）。可行原因：两侧都按 IEEE-754 单精度逐步舍入、运算序
//! 逐调用点一致、且都不做 FMA 收缩（JLS 浮点严格语义 + Rust 默认不收缩）。
//! 覆盖：负坐标、单纯形晶格顶点/胞心边界点、超大（1e6..2e6）/特大（1e9）
//! 量级、种子 0 / 全 1 位 / i64::MIN 位型。

#[path = "os2s_golden.rs"]
mod os2s_golden;

use mcv_worldgen::os2s::smooth;

/// 与 Java 参考程序（GenVec.java）完全一致的确定性点流。
struct Stream {
    state: u64,
    k: u64,
}

const LCG_MUL: u64 = 6364136223846793005;
const LCG_ADD: u64 = 1442695040888963407;

impl Stream {
    fn new() -> Self {
        Self { state: 0, k: 0 }
    }

    fn next01(&mut self) -> f64 {
        self.state = self.state.wrapping_mul(LCG_MUL).wrapping_add(LCG_ADD);
        (self.state >> 11) as f64 * (1.0_f64 / (1_u64 << 53) as f64)
    }

    /// 按类别产出坐标分量（k % 6 选类别）。
    fn cat(&mut self) -> f64 {
        const CAT_BASE: [f64; 6] = [0.0, -950.0, 1.0e6, 1.0e9, 0.0, 0.5];
        const CAT_SPAN: [f64; 6] = [16.0, 100.0, 2.0e6, 1.0e6, 0.5, 0.5];
        let c = (self.k % 6) as usize;
        self.k += 1;
        CAT_BASE[c] + (self.next01() - 0.5) * CAT_SPAN[c]
    }

    fn coords4(&mut self) -> (f64, f64, f64, f64) {
        (self.cat(), self.cat(), self.cat(), self.cat())
    }
}

/// 8 个 seed：0、全 1 位（-1）、1、42、0xDEADBEEF、0x0123456789ABCDEF、
/// -0x5DEECE66D、i64::MIN——种子位型两端。
const SEEDS: [i64; 8] = [
    0,
    -1,
    1,
    42,
    0xdead_beef,
    0x0123_4567_89ab_cdef,
    -0x5dee_ce66d,
    i64::MIN,
];

#[test]
fn os2s_bitwise_parity_with_official_java() {
    let mut s = Stream::new();
    for m in 0..10 {
        for si in 0..8 {
            let seed = SEEDS[si];
            for p in 0..64 {
                let want = f32::from_bits(os2s_golden::GOLDEN[m][si * 64 + p]);
                let (x, y, z, w) = s.coords4();
                let got = match m {
                    0 => smooth::noise2(seed, x, y),
                    1 => smooth::noise2_ImproveX(seed, x, y),
                    2 => smooth::noise3_ImproveXY(seed, x, y, z),
                    3 => smooth::noise3_ImproveXZ(seed, x, y, z),
                    4 => smooth::noise3_Fallback(seed, x, y, z),
                    5 => smooth::noise4_ImproveXYZ_ImproveXY(seed, x, y, z, w),
                    6 => smooth::noise4_ImproveXYZ_ImproveXZ(seed, x, y, z, w),
                    7 => smooth::noise4_ImproveXYZ(seed, x, y, z, w),
                    8 => smooth::noise4_ImproveXY_ImproveZW(seed, x, y, z, w),
                    _ => smooth::noise4_Fallback(seed, x, y, z, w),
                };
                assert_eq!(
                    got.to_bits(),
                    want.to_bits(),
                    "方法 {m} seed_idx {si} 点 {p} (x={x}, y={y}, z={z}, w={w}): got {:08x} want {:08x}",
                    got.to_bits(),
                    want.to_bits()
                );
            }
        }
    }
}

/// 输出值域哨兵：OS2S 公开 API 每维输出严格归一到 [-1, 1]。
#[test]
fn os2s_output_range_sentinel() {
    let mut s = Stream::new();
    for _ in 0..200 {
        let (x, y, z, w) = s.coords4();
        for si in [0, 7, 42, i64::MIN] {
            let v2 = smooth::noise2(si, x, y);
            let v3 = smooth::noise3_ImproveXZ(si, x, y, z);
            let v4 = smooth::noise4_Fallback(si, x, y, z, w);
            assert!(v2.abs() <= 1.0 && v3.abs() <= 1.0 && v4.abs() <= 1.0);
        }
    }
}
