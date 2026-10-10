//! vanilla density 编排 — clean-room 移植 26.1 `NoiseRouterData.java`
//! （bootstrap / registerTerrainNoises / entrances / spaghetti2D / underground /
//! postProcess / slide）与 `net/minecraft/data/worldgen/TerrainProvider.java`
//! 的样条常数；判定语义：最终 density <= 0 ⇒ 空气（NoiseRouter 式）。
//!
//! 坐标系：本世界 y ∈ [0, 256)、海平面 96（任务拍板）。曲线锚点以 MC 空间
//! 书写（26.1 原生 y ∈ [-64, 320)），换算集中于 `ours_y_to_mc` /
//! `mc_y_to_ours`（勿散落魔法数）。
//!
//! 取舍（验收报告逐条说明）：
//! - interpolated/blendDensity 的单元 8×4 插值不实现——段状高度带由气候
//!   样条本身给出，单元插值只影响洞壁平滑度；
//! - base_3d_noise（BlendedNoise 上下限夹逼混合）以自有双层 fBm 近其
//!   「随机破面」效果（NOTES-terrain §6 的既有取舍）；
//! - noodle / pillars 不在本 v1 管线（任务只点名双洞穴：意面 + 奶酪），
//!   Noodle 通道以常数 64 代入（min 不生效）。
//!
//! 洞穴/山脊噪声通道（2026-10-11 质量门修订）：全部改走 `NormalNoise`
//! fBm + 26.1 noise 参数表（data/minecraft/worldgen/noise/*.json），不再
//! 用裸单倍频 OS2S——裸单倍频的值分布（σ≈0.42）远宽于 26.1 的
//! NormalNoise 归一（σ≈0.14-0.30，按表而定），曾把地下空气占比推到
//! ~18%（quality::cave_air_in_bands 红）。逐通道对位：
//! - cave_entrance.json:1-9（firstOctave -7, [0.4,0.5,1.0]）；
//! - cave_layer.json:1-5（-8, [1.0]）—— vanilla 求值 (xz·1, y·8)
//!   （NoiseRouterData.java:292 `noise(CAVE_LAYER, 8.0)` 单参重载 = yScale），
//!   非旧实现的 (xz·8, y·8)（轴错位，洞穴失去水平层理）；
//! - cave_cheese.json:1-15（-8, [0.5,1,2,1,2,1,0,2,0]）—— vanilla
//!   (xz·1, y·2/3)（NoiseRouterData.java:295），非旧 (xz·2/3, y·1)；
//! - spaghetti_3d_rarity/1/2、spaghetti_2d_modulator、spaghetti_2d、
//!   spaghetti_2d_thickness：各 [1.0]，firstOctave -11/-7/-11（同名 json）；
//! - spaghetti_2d_elevation.json（-8, [1.0]）：vanilla 是 **2D** 通道
//!   （NoiseRouterData.java:273-275 `mappedNoise(…, yScale=0.0, -8, +8)`，
//!   列常量），mapFromUnitTo 值域 [-8,+8]——旧实现按 3D 采样且
//!   span_map(n,0,-8)·8 给出 [-64,0]，层状意面（layerRidged）整体错位；
//!   且 vanilla 的 layerRidged 立方内含 thickness（NoiseRouterData.java:278
//!   `.add(slopedSpaghetti, thickness).cube()`），旧实现漏加；
//! - spaghetti_roughness / spaghetti_roughness_modulator（-5/-8, [1.0]）：
//!   vanilla 是一个共享的 spaghettiRoughnessFunction（NoiseRouterData.java:213-220，
//!   entrances 与 underground 分支同源），modulator 求值 scale (1,1)——
//!   旧实现两处各派一套盐且 modulator 频率 ×2；
//! - jagged.json（-16, [1.0]×16）：山脊锯齿通道同改 NormalNoise
//!   （NoiseRouterData.java:99 `noise(JAGGED, 1500.0, 0.0)`）。

use crate::vanilla::climate::ClimateSampler;
use crate::vanilla::noise::NormalNoise;
use crate::vanilla::spline::{Builder, Coord, Spline};

/// 世界高（自研 256）。
pub const SY: i32 = 256;
/// 海平面（自研 96）。
pub const SEA: i32 = 96;
/// GLOBAL_OFFSET（NoiseRouterData.registerTerrainNoises）。
const GLOBAL_OFFSET: f64 = -0.50375;
/// 表面基准偏置：26.1 海平面 63（归一 0.33）线性重标到本基准海平面 96
/// （归一 0.375）；配合内陆子样条档位，平原落点在 SEA+5 ~ SEA+12。
const BACKBONE_BIAS: f64 = 0.2;
/// 顶部开口渐隐带（对应 slide 顶封的简化）。
pub const ENTRANCE_FADE: i32 = 14;
/// noodle 关闭哨兵（rangeChoice 的「未雕刻」常量）。
const NOODLE_OFF: f64 = 64.0;

// ---- 体素 id（镜像 mcv_core legacy 段顺序）----
pub const AIR: u16 = 0;
pub const STONE: u16 = 1;
pub const DIRT: u16 = 2;
pub const GRASS: u16 = 3;
pub const SAND: u16 = 4;
pub const WATER: u16 = 5;
pub const BEDROCK: u16 = 10;
pub const SNOW_GRASS: u16 = 11;

pub fn lerp1(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn slope(y1: f64, y2: f64, x1: f64, x2: f64) -> f64 {
    (y2 - y1) / (x2 - x1)
}

/// mapFromUnitTo（DensityFunctions）：把 [-1,1] 噪声值线性映到 [lo, hi]。
fn span_map(n: f64, lo: f64, hi: f64) -> f64 {
    (lo + hi) * 0.5 + (hi - lo) * 0.5 * n
}

/// y 钳制线性梯度（DensityFunctions.yClampedGradient）。
fn y_grad(y: f64, from: f64, to: f64, from_value: f64, to_value: f64) -> f64 {
    let t = ((y - from) / (to - from)).clamp(0.0, 1.0);
    lerp1(from_value, to_value, t)
}

/// DensityFunctions.yClampedGradient(-64, 320, 1.5, -1.5)。
fn y_clamp(y_mc: f64) -> f64 {
    y_grad(y_mc, -64.0, 320.0, 1.5, -1.5)
}

fn quarter_negative(x: f64) -> f64 {
    if x > 0.0 { x } else { x * 0.25 }
}

fn half_negative(x: f64) -> f64 {
    if x > 0.0 { x } else { x * 0.5 }
}

/// 山脊折叠形（NoiseRouterData.peaksAndValleys）。
pub fn peaks_and_valleys(r: f64) -> f64 {
    -(3.0_f64) * ((r.abs() - 0.6666667_f64).abs() - 0.33333334_f64)
}

// ---------- TerrainProvider 样条常数（clean-room 转写） ----------

const RIDGE_AMPLITUDE: f64 = 0.46082947_f64;
const RIDGE_OFFSET: f64 = 1.17_f64;

/// mountainContinentalness（TerrainProvider:217-225）。
fn mountain_continentalness(ridge: f64, modulation: f64) -> f64 {
    let ridge_slope = 1.0_f64 - (1.0_f64 - modulation) * 0.5_f64;
    let ridge_intersect = 0.5_f64 * (1.0_f64 - modulation);
    let adjusted = (ridge + RIDGE_OFFSET) * RIDGE_AMPLITUDE;
    let c = adjusted * ridge_slope - ridge_intersect;
    if ridge < -0.7_f64 {
        c.max(-0.2222_f64)
    } else {
        c.max(0.0_f64)
    }
}

/// calculateMountainRidgeZeroContinentalnessPoint（TerrainProvider:227-233）。
fn mountain_ridge_zero_point(modulation: f64) -> f64 {
    let ridge_slope = 1.0_f64 - (1.0_f64 - modulation) * 0.5_f64;
    let ridge_intersect = 0.5_f64 * (1.0_f64 - modulation);
    ridge_intersect / (RIDGE_AMPLITUDE * ridge_slope) - RIDGE_OFFSET
}

/// ridgeSpline（TerrainProvider:290-309）：5 段折叠山脊外壳（常量节点）。
fn ridge_shell(valley: f64, low: f64, mid: f64, high: f64, peaks: f64, min_valley: f64) -> Spline {
    let d1 = (0.5_f64 * (low - valley)).max(min_valley);
    let d2 = 5.0_f64 * (mid - low);
    Builder::new(Coord::RidgesFolded)
        .point_d(-1.0, valley, d1)
        .point_d(-0.4, low, d1.min(d2))
        .point_d(0.0, mid, d2)
        .point_d(0.4, high, 2.0 * (high - mid))
        .point_d(1.0, peaks, 0.7 * (peaks - high))
        .build()
}

/// buildMountainRidgeSplineWithPoints（TerrainProvider:177-215）：山脊线外壳。
fn mountain_ridge(modulation: f64, saddle: bool) -> Spline {
    let min_c = mountain_continentalness(-1.0, modulation);
    let max_c = mountain_continentalness(1.0, modulation);
    let zero_point = mountain_ridge_zero_point(modulation);
    let mut b = Builder::new(Coord::RidgesFolded);
    if -0.65_f64 < zero_point && zero_point < 1.0_f64 {
        let after_river = mountain_continentalness(-0.65, modulation);
        let before_river = mountain_continentalness(-0.75, modulation);
        let min_d = slope(min_c, before_river, -1.0, -0.75);
        b.point_d(-1.0, min_c, min_d);
        b.point(-0.75, before_river);
        b.point(-0.65, after_river);
        let zero_c = mountain_continentalness(zero_point, modulation);
        let max_d = slope(zero_c, max_c, zero_point, 1.0);
        b.point(zero_point - 0.01, zero_c);
        b.point_d(zero_point, zero_c, max_d);
        b.point_d(1.0, max_c, max_d);
    } else {
        let simple = slope(min_c, max_c, -1.0, 1.0);
        if saddle {
            b.point(-1.0, 0.2_f64.max(min_c));
            b.point_d(0.0, lerp1(0.5, min_c, max_c), simple);
        } else {
            b.point_d(-1.0, min_c, simple);
        }
        b.point_d(1.0, max_c, simple);
    }
    b.build()
}

/// getErosionFactor（TerrainProvider:128-171）：侵蚀轴子样条。
fn erosion_factor(base_value: f64, shattered: bool) -> Spline {
    let base_spline = Builder::new(Coord::Ridges)
        .point(-0.2, 6.3)
        .point(0.2, base_value)
        .build();
    let mut points = Builder::new(Coord::Erosion);
    points.spline(-0.6, base_spline.clone());
    points.spline(
        -0.5,
        Builder::new(Coord::Ridges)
            .point(-0.05, 6.3)
            .point(0.05, 2.67)
            .build(),
    );
    points.spline(-0.35, base_spline.clone());
    points.spline(-0.25, base_spline.clone());
    points.spline(
        -0.1,
        Builder::new(Coord::Ridges)
            .point(-0.05, 2.67)
            .point(0.05, 6.3)
            .build(),
    );
    points.spline(0.03, base_spline.clone());
    if shattered {
        let weirdness_shattered = Builder::new(Coord::Ridges)
            .point(0.0, base_value)
            .point(0.1, 0.625)
            .build();
        let ridges_shattered = Builder::new(Coord::RidgesFolded)
            .point(-0.9, base_value)
            .spline(-0.69, weirdness_shattered)
            .build();
        points.point(0.35, base_value);
        points.spline(0.45, ridges_shattered.clone());
        points.spline(0.55, ridges_shattered);
        points.point(0.62, base_value);
    } else {
        let extreme_hills = Builder::new(Coord::RidgesFolded)
            .spline(-0.7, base_spline.clone())
            .point(-0.15, 1.37)
            .build();
        let extra_3d = Builder::new(Coord::RidgesFolded)
            .spline(0.45, base_spline)
            .point(0.7, 1.56)
            .build();
        points.spline(0.05, extra_3d.clone());
        points.spline(0.4, extra_3d);
        points.spline(0.45, extreme_hills.clone());
        points.spline(0.55, extreme_hills);
        points.point(0.58, base_value);
    }
    points.build()
}

/// overworldFactor（TerrainProvider:40-51）：大陆度 → 表面「软硬」。
#[must_use]
pub fn overworld_factor_spline() -> Spline {
    let mut b = Builder::new(Coord::Continents);
    b.point(-0.19, 3.95);
    b.spline(-0.15, erosion_factor(6.25, true));
    b.spline(-0.1, erosion_factor(5.47, true));
    b.spline(0.03, erosion_factor(5.08, true));
    b.spline(0.06, erosion_factor(4.69, false));
    b.build()
}

/// buildErosionJaggednessSpline（TerrainProvider:65-88）：侵蚀 → 锯齿外壳。
fn erosion_jaggedness(at_peak0: f64, at_peak1: f64, at_high0: f64, at_high1: f64) -> Spline {
    let ridge0 = ridge_jaggedness(at_peak0, at_high0);
    let ridge1 = ridge_jaggedness(at_peak1, at_high1);
    Builder::new(Coord::Erosion)
        .spline(-1.0, ridge0)
        .spline(-0.78, ridge1.clone())
        .spline(-0.5775, ridge1)
        .point(-0.375, 0.0)
        .build()
}

/// buildRidgeJaggednessSpline（TerrainProvider:90-115）：山脊折叠 → 锯齿。
fn ridge_jaggedness(at_peak: f64, at_high: f64) -> Spline {
    let start = peaks_and_valleys(0.4);
    let end = peaks_and_valleys(0.56666666);
    let middle = (start + end) / 2.0;
    let mut b = Builder::new(Coord::RidgesFolded);
    b.point(start, 0.0);
    if at_high > 0.0 {
        b.spline(middle, weirdness_jaggedness(at_high));
    } else {
        b.point(middle, 0.0);
    }
    if at_peak > 0.0 {
        b.spline(1.0, weirdness_jaggedness(at_peak));
    } else {
        b.point(1.0, 0.0);
    }
    b.build()
}

/// buildWeirdnessJaggednessSpline（TerrainProvider:117-126）。
fn weirdness_jaggedness(jag_factor: f64) -> Spline {
    Builder::new(Coord::Ridges)
        .point(-0.01, 0.63 * jag_factor)
        .point(0.01, 0.3 * jag_factor)
        .build()
}

/// overworldJaggedness（TerrainProvider:53-63）：大陆度 → 锯齿外壳。
#[must_use]
pub fn overworld_jaggedness_spline() -> Spline {
    let mut b = Builder::new(Coord::Continents);
    b.point(-0.11, 0.0);
    b.spline(0.03, erosion_jaggedness(1.0, 0.5, 0.0, 0.0));
    b.spline(0.65, erosion_jaggedness(1.0, 1.0, 1.0, 1.0));
    b.build()
}

/// buildErosionOffsetSpline（TerrainProvider:235-288）：侵蚀 → 山脊/高原/平原。
// 参数集 1:1 对位 26.1 TerrainProvider.buildErosionOffsetSpline:235-246 的
// 标量形参（lowValley/hill/tallHill/mountainFactor/plain/swamp +
// includeExtremeHills/saddle；erosion/ridges 坐标由样条求值器持有，
// offsetTransformer 在 Rust 侧由 Builder 直接收值）——原生即 8 元，
// 不做结构体打包。
#[allow(clippy::too_many_arguments)]
fn erosion_offset_spline(
    low_valley: f64,
    hill: f64,
    tall_hill: f64,
    mf: f64,
    plain: f64,
    swamp: f64,
    include_extreme_hills: bool,
    saddle: bool,
) -> Spline {
    let very_low = mountain_ridge(lerp1(0.6, 1.5, mf), saddle);
    let low_m = mountain_ridge(lerp1(0.6, 1.0, mf), saddle);
    let m = mountain_ridge(mf, saddle);
    let wide = ridge_shell(
        low_valley - 0.15,
        0.5 * mf,
        0.5 * mf,
        0.5 * mf,
        0.6 * mf,
        0.5,
    );
    let narrow = ridge_shell(low_valley, plain * mf, hill * mf, 0.5 * mf, 0.6 * mf, 0.5);
    let plains = ridge_shell(low_valley, plain, plain, hill, tall_hill, 0.5);
    let extreme_hills = Builder::new(Coord::RidgesFolded)
        .point(-1.0, low_valley)
        .spline(-0.4, plains.clone())
        .point(0.0, tall_hill + 0.07)
        .build();
    let swamps = ridge_shell(-0.02, swamp, swamp, hill, tall_hill, 0.0);
    let mut b = Builder::new(Coord::Erosion);
    b.spline(-0.85, very_low);
    b.spline(-0.7, low_m);
    b.spline(-0.4, m);
    b.spline(-0.35, wide);
    b.spline(-0.1, narrow);
    b.spline(0.2, plains.clone());
    if include_extreme_hills {
        b.spline(0.4, plains.clone());
        b.spline(0.45, extreme_hills.clone());
        b.spline(0.55, extreme_hills);
        b.spline(0.58, plains.clone());
    }
    b.spline(0.7, swamps);
    b.build()
}

/// overworldOffset（TerrainProvider:18-38）：大陆度 → 高度盆带外壳。
#[must_use]
pub fn overworld_offset_spline() -> Spline {
    let beach = erosion_offset_spline(-0.15, 0.0, 0.0, 0.1, 0.0, -0.03, false, false);
    let low = erosion_offset_spline(-0.1, 0.03, 0.1, 0.1, 0.01, -0.03, false, false);
    let mid = erosion_offset_spline(-0.1, 0.03, 0.1, 0.7, 0.01, -0.03, true, true);
    let high = erosion_offset_spline(-0.05, 0.03, 0.1, 1.0, 0.01, 0.01, true, true);
    Builder::new(Coord::Continents)
        .point(-1.1, 0.044)
        .point(-1.02, -0.2222)
        .point(-0.51, -0.2222)
        .point(-0.44, -0.12)
        .point(-0.18, -0.12)
        .spline(-0.16, beach.clone())
        .spline(-0.15, beach)
        .spline(-0.1, low)
        .spline(0.25, mid)
        .spline(1.0, high)
        .build()
}

// ---------- 洞穴/山脊噪声参数（26.1 data/minecraft/worldgen/noise/*.json） ----------

const AMPS_1: [f64; 1] = [1.0];
/// cave_entrance.json:1-9。
const AMPS_CAVE_ENTRANCE: [f64; 3] = [0.4, 0.5, 1.0];
/// cave_cheese.json:1-15。
const AMPS_CAVE_CHEESE: [f64; 9] = [0.5, 1.0, 2.0, 1.0, 2.0, 1.0, 0.0, 2.0, 0.0];
/// jagged.json:1-21（16 × 1.0）。
const AMPS_JAGGED: [f64; 16] = [1.0; 16];

/// 意面稀有度量化（NoiseRouterData.QuantizedSpaghettiRarity.getSpaghettiRarity3D）。
fn spaghetti_rarity_3d(rarity_factor: f64) -> f64 {
    if rarity_factor < -0.5 {
        0.75
    } else if rarity_factor < 0.0 {
        1.0
    } else if rarity_factor < 0.5 {
        1.5
    } else {
        2.0
    }
}

/// 层状意面稀有度量化（getSphaghettiRarity2D）。
fn spaghetti_rarity_2d(rarity_factor: f64) -> f64 {
    if rarity_factor < -0.75 {
        0.5
    } else if rarity_factor < -0.5 {
        0.75
    } else if rarity_factor < 0.5 {
        1.0
    } else if rarity_factor < 0.75 {
        2.0
    } else {
        3.0
    }
}

/// 每列编排结果。
#[derive(Clone, Copy, Debug)]
pub struct ColumnState {
    pub offset: f64,
    pub factor: f64,
    pub unscaled_jag: f64,
    /// jaggedness 通道列常量（noise(JAGGED, 1500.0, 0.0)：yScale=0 → 2D，
    /// 采样点 (bx·1500, 0, bz·1500) 与 y 无关，提升出体素循环）。
    pub jag_noise: f64,
    /// SPAGHETTI_2D_ELEVATION 列常量（vanilla yScale=0 的 2D 通道，
    /// NoiseRouterData.java:273-275；mapFromUnitTo → [-8, +8]）。
    pub sp2d_elev: f64,
    pub surface_ours: i32,
    pub ocean: bool,
}

/// density 编排器 — NoiseRouter 式编排（offset/factor/jaggedness 样条 +
/// 各 3D 通道 → 最终 density 判实心）。洞穴/山脊通道全部为 NormalNoise
/// fBm（26.1 noise 参数表；域盐沿用 channel_seed 同款派生：NormalNoise::new
/// 内部 noise_seed(world, salt) = splitmix64(world ^ salt)）。
pub struct Orchestrator {
    climate: ClimateSampler,
    offset_spline: Spline,
    factor_spline: Spline,
    jag_spline: Spline,
    base_3d: NormalNoise,
    // 洞穴/山脊通道（26.1 noise json 表）
    cave_entrance: NormalNoise,
    sp3d_rarity: NormalNoise,
    sp3d_1: NormalNoise,
    sp3d_2: NormalNoise,
    sp3d_thickness: NormalNoise,
    sp_roughness_mod: NormalNoise,
    sp_roughness: NormalNoise,
    sp2d_mod: NormalNoise,
    sp2d: NormalNoise,
    sp2d_elevation: NormalNoise,
    sp2d_thickness: NormalNoise,
    cave_layer: NormalNoise,
    cave_cheese: NormalNoise,
    jagged: NormalNoise,
}

impl Orchestrator {
    #[must_use]
    pub fn new(world_seed: u64) -> Self {
        // 域盐沿用旧通道派生（raw 时代 seed = channel_seed(world, DOMAIN) =
        // splitmix64(world ^ DOMAIN)，与 NormalNoise::new 的 noise_seed 同式）。
        let n = |salt: u64, first_octave: i32, amps: &'static [f64]| {
            NormalNoise::new(world_seed, salt, first_octave, amps)
        };
        Self {
            climate: ClimateSampler::new(world_seed),
            offset_spline: overworld_offset_spline(),
            factor_spline: overworld_factor_spline(),
            jag_spline: overworld_jaggedness_spline(),
            base_3d: n(0x00B1_3D00, -7, &[1.0, 1.0, 1.0]),
            cave_entrance: n(0x0000_CAFE_0001 ^ 0x26, -7, &AMPS_CAVE_ENTRANCE),
            sp3d_rarity: n(0x0000_CAFE_0001 ^ 0x11, -11, &AMPS_1),
            sp3d_1: n(0x0000_CAFE_0001 ^ 0x21, -7, &AMPS_1),
            sp3d_2: n(0x0000_CAFE_0001 ^ 0x22, -7, &AMPS_1),
            sp3d_thickness: n(0x0000_CAFE_0001 ^ 0x23, -8, &AMPS_1),
            sp_roughness_mod: n(0x0B00_5E51, -8, &AMPS_1),
            sp_roughness: n(0x0B00_5E52, -5, &AMPS_1),
            sp2d_mod: n(0x0000_CAFE_0002 ^ 0x31, -11, &AMPS_1),
            sp2d: n(0x0000_CAFE_0002 ^ 0x32, -7, &AMPS_1),
            sp2d_elevation: n(0x0000_CAFE_0002 ^ 0x33, -8, &AMPS_1),
            sp2d_thickness: n(0x0000_CAFE_0002 ^ 0x34, -11, &AMPS_1),
            cave_layer: n(0x0000_CAFE_0003 ^ 0x41, -8, &AMPS_1),
            cave_cheese: n(0x0000_CAFE_0003 ^ 0x42, -8, &AMPS_CAVE_CHEESE),
            jagged: n(0x7A66ED, -16, &AMPS_JAGGED),
        }
    }

    /// 采样并编排一列。
    #[must_use]
    pub fn column_state(&self, block_x: i32, block_z: i32) -> ColumnState {
        let sample = self.climate.sample(block_x, block_z);
        let c = sample.spline_climate();
        let offset = self.offset_spline.eval(&c) + GLOBAL_OFFSET + BACKBONE_BIAS;
        let factor = self.factor_spline.eval(&c);
        let unscaled_jag = self.jag_spline.eval(&c);
        // 地表零点：yClampGradient(-64,320,+1.5,-1.5) 与 offset 抵消处
        //（y_clamp 斜率 -1/128 格/密度单位）
        let surface_mc = (128.0 + 128.0 * offset).clamp(-64.0, 320.0);
        let surface_ours =
            (crate::vanilla::mc_y_to_ours(surface_mc).round() as i32).clamp(4, SY - 10);
        // jaggedness 通道：noise(JAGGED, 1500.0, 0.0)（NoiseRouterData.java:99），
        // yScale=0 → 列常量，提升到列级（每体素省 32 次倍频求值，值逐位不变）。
        let jag_x = f64::from(block_x) * 1500.0;
        let jag_z = f64::from(block_z) * 1500.0;
        let jag_noise = self.jagged.get_value(jag_x, 0.0, jag_z);
        // 层状意面海拔：2D 通道（yScale=0，NoiseRouterData.java:273-275），
        // 列常量；mapFromUnitTo(noise, -8.0, 8.0)。
        let elev_n = self.sp2d_elevation.get_value(f64::from(block_x), 0.0, f64::from(block_z));
        let sp2d_elev = span_map(elev_n, -8.0, 8.0);
        ColumnState {
            offset,
            factor,
            unscaled_jag,
            jag_noise,
            sp2d_elev,
            surface_ours,
            ocean: surface_ours <= SEA,
        }
    }

    /// spaghettiRoughnessFunction（NoiseRouterData.java:213-220）：
    /// mapped(SPAGHETTI_ROUGHNESS_MODULATOR, 0.0, -0.1) · (|noise(SPAGHETTI_ROUGHNESS)| − 0.4)。
    /// vanilla 在 entrances 与 underground 分支共用同一函数（旧实现两处各派
    /// 一套盐、modulator 频率 ×2，均已收回）。
    fn spaghetti_roughness(&self, bx: f64, y_mc: f64, bz: f64) -> f64 {
        let mod_v = span_map(self.sp_roughness_mod.get_value(bx, y_mc, bz), 0.0, -0.1);
        mod_v * (self.sp_roughness.get_value(bx, y_mc, bz).abs() - 0.4)
    }

    /// CaveEntrances 函数（NoiseRouterData.entrances:223-241）：洞口/意面雕刻带。
    /// 返回 entrances 值（< 0 ⇒ 雕空）。
    fn entrances(&self, bx: f64, y_mc: f64, bz: f64) -> f64 {
        // spaghetti3DRarityModulator = noise(SPAGHETTI_3D_RARITY, 2.0, 1.0)
        let rarity_mod = self.sp3d_rarity.get_value(bx * 2.0, y_mc, bz * 2.0);
        let rarity = spaghetti_rarity_3d(rarity_mod);
        // WeirdScaledSampler TYPE1: rarity · |noise_i(bx/rarity, by/rarity, bz/rarity)|
        let w1 = rarity * self.sp3d_1.get_value(bx / rarity, y_mc / rarity, bz / rarity).abs();
        let w2 = rarity * self.sp3d_2.get_value(bx / rarity, y_mc / rarity, bz / rarity).abs();
        // thickness = mapped(SPAGHETTI_3D_THICKNESS, -0.065, -0.088)
        let thick = span_map(self.sp3d_thickness.get_value(bx, y_mc, bz), -0.065, -0.088);
        let sp3d = (w1.max(w2) + thick).clamp(-1.0, 1.0);
        // bigEntrances = noise(CAVE_ENTRANCE, 0.75, 0.5) + 0.37 + yClampedGradient(-10, 30, 0.3, 0)
        let big_noise = self.cave_entrance.get_value(bx * 0.75, y_mc * 0.5, bz * 0.75);
        let big = big_noise + 0.37 + y_grad(y_mc, -10.0, 30.0, 0.3, 0.0);
        big.min(sp3d + self.spaghetti_roughness(bx, y_mc, bz))
    }

    /// spaghetti_2D 函数（NoiseRouterData.spaghetti2D:268-286）：层状意面雕刻带。
    fn spaghetti_2d(&self, cs: &ColumnState, bx: f64, y_mc: f64, bz: f64) -> f64 {
        // spaghetti2DRarityModulator = noise(SPAGHETTI_2D_MODULATOR, 2.0, 1.0)
        let rarity_mod = self.sp2d_mod.get_value(bx * 2.0, y_mc, bz * 2.0);
        let rarity = spaghetti_rarity_2d(rarity_mod);
        // WeirdScaledSampler TYPE2
        let cave = rarity * self.sp2d.get_value(bx / rarity, y_mc / rarity, bz / rarity).abs();
        // thickness2 = mapped(SPAGHETTI_2D_THICKNESS, 2.0, 1.0, -0.6, -1.3)
        let thick2 = span_map(
            self.sp2d_thickness.get_value(bx * 2.0, y_mc, bz * 2.0),
            -0.6,
            -1.3,
        );
        // slopedSpaghetti = |elev(2D 列常量) + yClampedGradient(-64, 320, 8, -40)|
        let sloped_spaghetti = (cs.sp2d_elev + y_grad(y_mc, -64.0, 320.0, 8.0, -40.0)).abs();
        // layerRidged = (slopedSpaghetti + thickness)³（NoiseRouterData.java:278：
        // thickness 在立方内——旧实现漏加）
        let inner = sloped_spaghetti + thick2;
        let layer_ridged = inner * inner * inner;
        let cave_noise = cave + 0.083 * thick2;
        cave_noise.max(layer_ridged).clamp(-1.0, 1.0)
    }

    /// UnderGround 奶酪函数（NoiseRouterData.underground:289-306）：
    /// layerized + clamp(0.27+cheese) + clamp(1.5−0.64·slopedCheese)。
    /// 求值坐标对位 vanilla：cave_layer (xz·1, y·8)（NoiseRouterData.java:293）、
    /// cave_cheese (xz·1, y·2/3)（:295）。
    fn underground(&self, bx: f64, y_mc: f64, bz: f64, sloped_cheese: f64) -> f64 {
        let layer = self.cave_layer.get_value(bx, y_mc * 8.0, bz);
        let layerized = 4.0 * layer * layer;
        let cheese = self.cave_cheese.get_value(bx, y_mc * (2.0 / 3.0), bz);
        let solidified =
            (0.27 + cheese).clamp(-1.0, 1.0) + (1.5 - 0.64 * sloped_cheese).clamp(0.0, 0.5);
        layerized + solidified
    }

    /// 最终 density 场值（> 0 实心）。
    #[must_use]
    pub fn density(&self, cs: &ColumnState, block_x: i32, block_y: i32, block_z: i32) -> f64 {
        let bx = f64::from(block_x);
        let bz = f64::from(block_z);
        let y_mc = crate::vanilla::ours_y_to_mc(f64::from(block_y));
        // jaggedness = unscaled · halfNegative(jaggedNoise @ bx·1500)
        let jag = cs.unscaled_jag * half_negative(cs.jag_noise);
        // initial = 4 · quarterNegative(factor·(depth + jag))
        let depth = y_clamp(y_mc) + cs.offset;
        let initial = 4.0 * quarter_negative(cs.factor * (depth + jag));
        // base_3d_noise：随机破面（自有双层 fBm 近 BlendedNoise）
        let base_3d_v = self.base_3d.get_value(bx * 2.0, y_mc * 0.75, bz * 2.0);
        let sloped = initial + base_3d_v;
        // 双洞穴（sloped_cheese 的 rangeChoice 分界 1.5625，overworld.json
        // final_density 的 range_choice max_exclusive）
        let e = self.entrances(bx, y_mc, bz);
        let caves = if sloped < 1.5625 {
            sloped.min(5.0 * e)
        } else {
            let sp2d = self.spaghetti_2d(cs, bx, y_mc, bz);
            let u = self.underground(bx, y_mc, bz, sloped);
            (u.min(e)).min(sp2d + self.spaghetti_roughness(bx, y_mc, bz))
        };
        // slide（slideOverworld: -64, 384, 80, 64, -0.078125, 0, 24, 0.1171875）
        let top_factor = y_grad(y_mc, 240.0, 256.0, 1.0, 0.0);
        let v = lerp1(-0.078125, caves, top_factor);
        let bottom_factor = y_grad(y_mc, -64.0, -40.0, 0.0, 1.0);
        let v = lerp1(0.1171875, v, bottom_factor);
        // postProcess = squeeze(0.64·slide)，与 noodle(关闭=64) 取 min
        squeeze(0.64 * v).min(NOODLE_OFF)
    }
}

fn squeeze(c: f64) -> f64 {
    let c = c.clamp(-1.0, 1.0);
    c / 2.0 - c * c * c / 24.0
}

/// 统一入口：生成一个区块（pass 1 柱层序）。
///
/// # Errors
/// 恒返回 `Ok(())`；保留 `Result` 以与 FFI 路径同构。
pub fn generate(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    out_voxels: &mut [u16],
    out_heightmap: &mut [u8],
) -> Result<(), i32> {
    debug_assert!(out_voxels.len() >= mcv_core::CHUNK_VOL);
    debug_assert!(out_heightmap.len() >= 256);
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    let orch = Orchestrator::new(seed);

    for cz in 0..16_i32 {
        for cx in 0..16_i32 {
            let wx = base_x.wrapping_add(cx);
            let wz = base_z.wrapping_add(cz);
            let cs = orch.column_state(wx, wz);
            let surface = cs.surface_ours;
            let beach = surface <= SEA + 1;
            let snowy = surface > 142;
            let top = surface.max(SEA);

            for y in 0..=top {
                let field = orch.density(&cs, wx, y, wz);
                let id = if y == 0 || (y <= 2 && bedrock_noise(seed, wx, y, wz)) {
                    BEDROCK
                } else if field <= 0.0 {
                    if y <= SEA { WATER } else { AIR }
                } else if y == surface {
                    if beach {
                        SAND
                    } else if snowy {
                        SNOW_GRASS
                    } else {
                        GRASS
                    }
                } else if y >= surface - 3 {
                    if beach { SAND } else { DIRT }
                } else {
                    STONE
                };
                let vx = cx as usize;
                let vy = y as usize;
                let vz = cz as usize;
                out_voxels[(vy << 8) | (vz << 4) | vx] = id;
            }
        }
    }
    // 高度图与 cpp kernel pass 3 / 存档重算（crate::recompute_heightmap）
    // 同一谓词（damp!=0 且非流体）——直接复用，三方由构造保证逐块一致。
    // （此前 vanilla 路径漏写高度图，恒 0 提交：出生列搜索全部 reject。）
    let hm = crate::recompute_heightmap(out_voxels);
    out_heightmap[..256].copy_from_slice(&hm[..]);
    Ok(())
}

fn bedrock_noise(seed: u64, wx: i32, y: i32, wz: i32) -> bool {
    let h = crate::vanilla::hash01(
        crate::vanilla::channel_seed(seed, 0x0D00_D00D),
        i64::from(wx),
        i64::from(y),
        i64::from(wz),
    );
    h < 0.5
}
