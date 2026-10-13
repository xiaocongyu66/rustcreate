//! vanilla density 编排 — clean-room 移植 26.1 `NoiseRouterData.java`
//! （bootstrap / registerTerrainNoises / entrances / spaghetti2D / underground /
//! postProcess / slide）与 `net/minecraft/data/worldgen/TerrainProvider.java`
//! 的样条常数；判定语义：最终 density <= 0 ⇒ 空气（NoiseRouter 式）。
//!
//! 坐标系（v6）：本世界 y ∈ [-64, 320)、海平面 63 —— 与 26.1 原生同域，
//! 曲线锚点即世界 y（`ours_y_to_mc`/`mc_y_to_ours` 恒等，见 vanilla/mod.rs；
//! 换算壳保留，勿散落魔法数）。
//!
//! 取舍（验收报告逐条说明）：
//! - interpolated/blendDensity 的单元 8×4 插值不实现——段状高度带由气候
//!   样条本身给出，单元插值只影响洞壁平滑度；
//! - base_3d_noise 自 T4 起为真移植 BlendedNoise（vanilla/blended.rs：
//!   NoiseRouterData.java:86 `createUnseeded(0.25,0.125,80,160,8)`，上下限
//!   夹逼混合；此前「双层 fBm 近其破面」取舍作废）；
//! - noodle / pillars 自 #192 洞穴改进波起为实物移植
//!   （`density_function/overworld/caves/{noodle,pillars}.json` 逐节点对位，
//!   见 [`Orchestrator::noodle`] / [`Orchestrator::pillars`] 注）；
//!   noodle 各通道的 `minecraft:interpolated` 外壳按本模块既定口径跳过
//!   （与 final_density 外层 interpolated/blend_density 同一取舍——只影响
//!   8×8×4 单元级管壁平滑度，不改变掩膜/雕刻判定语义）。
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

use crate::vanilla::biomes::Biome;
use crate::vanilla::blended::BlendedNoise;
use crate::vanilla::climate::ClimateSampler;
use crate::vanilla::noise::NormalNoise;
use crate::vanilla::spline::{Builder, Coord, Spline};

/// 世界高（v6 = 384）。
pub const SY: i32 = crate::vanilla::SY;
/// 海平面（v6 = 原生 63）。
pub const SEA: i32 = crate::vanilla::SEA;
/// 世界竖直下界（v6 = -64）。
pub const MIN_Y: i32 = crate::vanilla::MIN_Y;
/// GLOBAL_OFFSET（NoiseRouterData.registerTerrainNoises）。
const GLOBAL_OFFSET: f64 = -0.50375;
/// 顶部开口渐隐带（对应 slide 顶封的简化）。
pub const ENTRANCE_FADE: i32 = 14;
/// noodle 关闭哨兵（noodle.json 外层 range_choice
/// `when_in_range[-10⁶,0) → 64`：掩膜噪声 < 0 处不参与 min）。
const NOODLE_OFF: f64 = 64.0;
/// pillars 门（final_density out_of_range 支
/// `range_choice(pillars, [-10⁶, 0.03) → −10⁶, 否则 pillars)`）。
const PILLARS_GATE_MIN: f64 = 0.03;
/// pillars/noodle 门哨兵（range_choice 越界常量，语义=「不抬升」）。
const CARVE_OFF: f64 = -1.0e6;
/// noodle_ridge_a/b 求值缩放（noodle.json `2.6666666666666665`）。
const NOODLE_RIDGE_SCALE: f64 = 2.666_666_666_666_666_5_f64;

/// Noises.SURFACE 注册名（Noises.java:78-83 instantiate 用
/// `factory.fromHashOf(name.identifier())` 的字符串形态）与 surface.json
/// 参数表（firstOctave −6、amplitudes [1,1,1]）——#192 SURF-1 ② 销账，
/// 消费侧 [`Orchestrator::surface_depth`]。
const SURFACE_NOISE_NAME: &str = "minecraft:surface";
const AMPS_SURFACE: [f64; 3] = [1.0, 1.0, 1.0];

/// 真实实顶扫描上限余量：过零面上方的「软空气」半支（quarterNegative 负
/// 支斜率 ÷4 ⇒ 每格仅 −factor/128 ≈ −0.03..−0.05）叠加 base_3d 极值尾
/// （|v| ≤ ~0.6，BlendedNoise 混合界），20 格 ≥ 0.6·128/3.95（最低
/// factor = overworldFactor 3.95）保证扫描窗罩住一切实心——窗过矮会把
/// 峰顶齐窗削平（旧实现锚估计面即此病）。
pub const REAL_TOP_SCAN_MARGIN: i32 = 20;

/// abovePreliminarySurface 门深（SurfaceRules.java:264
/// HOW_FAR_BELOW_PRELIMINARY_SURFACE_LEVEL_TO_BUILD_SURFACE = 8；
/// SurfaceRuleData.java:278 全部表层规则包在该门内——带下（深洞底）
/// 回落 STONE，洞口带内照常表层化）。
pub const PRELIM_SURFACE_BAND: i32 = 8;

// ---- 体素 id（镜像 mcv_core legacy 段顺序）----
pub const AIR: u16 = 0;
pub const STONE: u16 = 1;
pub const DIRT: u16 = 2;
pub const GRASS: u16 = 3;
pub const SAND: u16 = 4;
pub const WATER: u16 = 5;
/// 岩浆（#175 含水层岩浆带）：blocks_gen.inc.rs:537（liquid、光 15、
/// 六面贴图 402）。渲染为贴图整盒，流动动画/水面下沉面登记 #148 流体波债。
pub const LAVA: u16 = 516;
pub const BEDROCK: u16 = 10;
pub const SNOW_GRASS: u16 = 11;
/// 深板岩（#192 深板岩层）：blocks_gen.inc.rs:363（id 342，
/// mcv_item/src/lib.rs:2149 `Block(BlockId(342))` 交叉印证）。
pub const DEEPSLATE: u16 = 342;

/// vertical_gradient 条件位置随机的实物 random_name（overworld.json
/// surface_rule 树原文；#192 RNG-1 销账：种子链 = vanilla/xoroshiro.rs
/// 的 fromHashOf(name).forkPositional() 实物链，同 seed 同坐标与原版
/// 同位型）。
const BEDROCK_FLOOR_RANDOM: &str = "minecraft:bedrock_floor";
const DEEPSLATE_RANDOM: &str = "minecraft:deepslate";

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

pub(crate) fn half_negative(x: f64) -> f64 {
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
    // 0.65 点第 4 标量 = jaggednessFactorAtHighRidgeAndErosionIndex1 = 0.0
    //（TerrainProvider.java:61）——高侵蚀带（erosion ∈ [-0.78,-0.5775] 段）
    // 山脊中段 point(middle, 0.0)，高原顶部无锯齿；旧码漂移 1.0 使
    // weirdness_jaggedness(1.0)≈0.465 混入（T3 红测试锁）。
    b.spline(0.65, erosion_jaggedness(1.0, 1.0, 1.0, 0.0));
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
/// pillar.json:1-7（firstOctave -7, [1,1]）。
const AMPS_PILLAR: [f64; 2] = [1.0, 1.0];
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
    /// 样条估计面 = round(128+128·offset)——树门/群系窗/质量统计的估计
    /// 口径（quality::plains_share、geometry_locks 材质分带）。#140 起不再
    /// 决定 pass 1 表层材质：表层锚密度场真实过零面（见 generate 的逐列
    /// 实顶扫描）。
    pub surface_ours: i32,
    pub ocean: bool,
    /// erosion 气候通道原值（含水层 deep dark 门用，
    /// OverworldBiomeBuilder.java:1008-1010）。
    pub erosion: f64,
}

/// density 编排器 — NoiseRouter 式编排（offset/factor/jaggedness 样条 +
/// 各 3D 通道 → 最终 density 判实心）。洞穴/山脊通道全部为 NormalNoise
/// fBm（26.1 noise 参数表；域盐沿用 channel_seed 同款派生：NormalNoise::new
/// 内部 noise_seed(world, salt) = splitmix64(world ^ salt)）。
pub struct Orchestrator {
    /// vertical_gradient 条件位置随机工厂（#192 RNG-1 销账）：
    /// `root_positional_factory(worldSeed).named_factory(random_name)` =
    /// RandomState.java:37+113-115 实物链（xoroshiro.rs 头注），同 seed
    /// 同坐标与原版同位型。
    bedrock_random: crate::vanilla::xoroshiro::PositionalRandomFactory,
    deepslate_random: crate::vanilla::xoroshiro::PositionalRandomFactory,
    /// SurfaceSystem 根噪声随机工厂（= RandomState.java:37 `this.random`
    /// 直传，SurfaceSystem.java:50 的 noiseRandom 槽位）——
    /// [`Self::surface_depth`] 抖动用（#192 SURF-1 ② 销账）。
    noise_random: crate::vanilla::xoroshiro::PositionalRandomFactory,
    /// Noises.SURFACE（surface.json -6 [1,1,1]），原版种子链
    /// （`NormalNoise::from_vanilla_name`）。
    surface_noise: NormalNoise,
    climate: ClimateSampler,
    offset_spline: Spline,
    factor_spline: Spline,
    jag_spline: Spline,
    /// base_3d_noise：vanilla BlendedNoise（NoiseRouterData.java:86
    /// `BlendedNoise.createUnseeded(0.25, 0.125, 80, 160, 8)`；种子注入
    /// RandomState.java:69-72 fromHashOf("terrain")，clean-room 用域盐派生）。
    base_3d: BlendedNoise,
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
    // #192 洞穴改进波：pillars / noodle 通道（同名 26.1 noise json 表，
    // pillar.json -7 [1,1]；pillar_rareness/pillar_thickness/noodle/
    // noodle_thickness 均 -8 [1]；noodle_ridge_a/b 均 -7 [1]）。
    pillar: NormalNoise,
    pillar_rareness: NormalNoise,
    pillar_thickness: NormalNoise,
    noodle_noise: NormalNoise,
    noodle_thickness: NormalNoise,
    noodle_ridge_a: NormalNoise,
    noodle_ridge_b: NormalNoise,
}

impl Orchestrator {
    #[must_use]
    pub fn new(world_seed: u64) -> Self {
        // 域盐沿用旧通道派生（raw 时代 seed = channel_seed(world, DOMAIN) =
        // splitmix64(world ^ DOMAIN)，与 NormalNoise::new 的 noise_seed 同式）。
        let n = |salt: u64, first_octave: i32, amps: &'static [f64]| {
            NormalNoise::new(world_seed, salt, first_octave, amps)
        };
        let root = crate::vanilla::xoroshiro::root_positional_factory(world_seed as i64);
        Self {
            bedrock_random: root.named_factory(BEDROCK_FLOOR_RANDOM),
            deepslate_random: root.named_factory(DEEPSLATE_RANDOM),
            noise_random: root,
            surface_noise: NormalNoise::from_vanilla_name(
                &root,
                SURFACE_NOISE_NAME,
                -6,
                &AMPS_SURFACE,
            ),
            climate: ClimateSampler::new(world_seed),
            offset_spline: overworld_offset_spline(),
            factor_spline: overworld_factor_spline(),
            jag_spline: overworld_jaggedness_spline(),
            base_3d: BlendedNoise::new(world_seed, 0x00B1_3D00),
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
            pillar: n(0x0000_CA11_5E00 ^ 0x51, -7, &AMPS_PILLAR),
            pillar_rareness: n(0x0000_CA11_5E00 ^ 0x52, -8, &AMPS_1),
            pillar_thickness: n(0x0000_CA11_5E00 ^ 0x53, -8, &AMPS_1),
            noodle_noise: n(0x0000_0DE0_1E00 ^ 0x61, -8, &AMPS_1),
            noodle_thickness: n(0x0000_0DE0_1E00 ^ 0x62, -8, &AMPS_1),
            noodle_ridge_a: n(0x0000_0DE0_1E00 ^ 0x63, -7, &AMPS_1),
            noodle_ridge_b: n(0x0000_0DE0_1E00 ^ 0x64, -7, &AMPS_1),
        }
    }

    /// 一列表层材质对 (顶格, 亚表带)。#140 表层波表驱动，自 `generate`
    /// 原样抽出（判定语义逐字节不变，`column_real_top` 与柱层循环共用，
    /// 杜绝两套表层规则漂移）。
    ///
    /// 滩/雪位置带沿用 `surface_ours` 估计面口径（geometry_locks
    /// vanilla_surface_material_bands 的契约面：「材质由 offset 估计面
    /// 分带」，tests/geometry_locks.rs:100-121）。26.1 表层是逐方块条件
    /// 规则表（SurfaceRuleData.overworldLike），顶格按群系与水情取块——
    /// 内陆默认 grassOrDirtIfUnderwater（SurfaceRuleData.java:71,165：
    /// aboveWater→GRASS_BLOCK，否则 DIRT）、BEACH/WARM_OCEAN/DESERT 沙
    /// （:74-87 sandOrSandstoneIfCeiling）、SNOWY_PLAINS 草+雪层（雪草
    /// 单块表示，本子集无 frozen peaks/badlands/mushroom/mangrove 例外
    /// 群系，缺 id 债见 KNOWN-DIVERGENCE SURF-1 ①）。
    #[must_use]
    pub fn column_surface_ids(&self, block_x: i32, block_z: i32, cs: &ColumnState) -> (u16, u16) {
        let surface = cs.surface_ours;
        let beach = surface <= SEA + 1;
        let snowy = surface > SEA + 46; // 旧 256 基准 y>142 = 海平面上 46
        if beach {
            (SAND, SAND)
        } else if snowy {
            (SNOW_GRASS, DIRT)
        } else {
            match self.biome_at(block_x, block_z) {
                Biome::Desert | Biome::Beach | Biome::Ocean | Biome::River => (SAND, SAND),
                Biome::SnowyPlains => (SNOW_GRASS, DIRT),
                Biome::Plains
                | Biome::Forest
                | Biome::BirchForest
                | Biome::Savanna
                | Biome::Taiga => (GRASS, DIRT),
            }
        }
    }

    /// 实心格材质（#175 抽公共：pass-1 实心分支与含水层 Rock 封口袋分支、
    /// `column_real_top` 三处共用，杜绝规则漂移）。语义原样：带下 STONE 兜底
    /// （abovePreliminarySurface 门）、depth_above==1 顶格、≤1+surface_depth
    /// 亚表带（#192 SURF-1 ② 销账：`surface_depth` = 实物
    /// [`Self::surface_depth`] 逐列值，替旧常数 3——原版 UNDER_FLOOR
    /// `stoneDepth ≤ 1 + surfaceDepth`，SurfaceRules.java:676-684）；
    /// #192：两个 STONE 出口统一过 [`Self::stone_replacement`]
    /// （vertical_gradient bedrock/deepslate 实物规则替换——表层草/土/沙位
    /// 不受影响）。
    #[must_use]
    #[allow(clippy::too_many_arguments)] // 柱层上下文天然多参（位坐标+门深+计数+双材质对，blockshapes.rs:293 先例）
    pub fn solid_material(
        &self,
        wx: i32,
        y: i32,
        wz: i32,
        min_surface: i32,
        depth_above: i32,
        surface_id: u16,
        subsoil_id: u16,
        surface_depth: i32,
    ) -> u16 {
        if y < min_surface {
            self.stone_replacement(wx, y, wz)
        } else if depth_above == 1 {
            surface_id
        } else if depth_above <= 1 + surface_depth {
            subsoil_id
        } else {
            self.stone_replacement(wx, y, wz)
        }
    }

    /// SurfaceSystem.getSurfaceDepth（SurfaceSystem.java:165-168）实物：
    /// `int(surfaceNoise.getValue(bx, 0, bz) · 2.75 + 3.0 +
    /// noiseRandom.at(bx, 0, bz).nextDouble() · 0.25)`——
    /// - surfaceNoise = `Noises.SURFACE`（surface.json firstOctave −6、
    ///   amplitudes [1,1,1]），种子链 = getOrCreateNoise 实物
    ///   （[`NormalNoise::from_vanilla_name`]，同 seed 与原版同位型）；
    /// - noiseRandom = RandomState 根位置随机工厂本身（SurfaceSystem.java:50
    ///   `new SurfaceSystem(this, …, this.random)` 直传，非 named fork）；
    /// - (int) 截断 = Java 向零取整，Rust `as i32` 同语义。
    ///   消费侧 = UNDER_FLOOR 亚表带 `stoneDepth ≤ 1 + surfaceDepth`
    ///   （SurfaceRules.java:676-684 `addSurfaceDepth`），典型 0..5、均值 ≈3。
    #[must_use]
    pub fn surface_depth(&self, block_x: i32, block_z: i32) -> i32 {
        let n = self
            .surface_noise
            .get_value(f64::from(block_x), 0.0, f64::from(block_z));
        let jitter = self.noise_random.double_at(block_x, 0, block_z);
        (n * 2.75 + 3.0 + jitter * 0.25) as i32
    }

    /// pass-1 体素缓冲中该列的**实顶**（自顶向下第一个非空气格）+1 = 树基
    /// 候选，附带实顶格材质——不读本列体素、纯世界坐标函数，任意区块可对
    /// 任意 (x,z) 重放（fix/tree-chunk-clip：邻块树在本块重放时，树基判定
    /// 必须与树干所属块的缓冲扫描逐字节一致，否则跨界树两侧各算各的基，
    /// 「少一边树叶」缺陷的根因面）。
    ///
    /// 复刻 `generate` 柱层循环的自顶向下语义（扫描即填充）：
    /// - 起点 `fill_top = scan_top.max(SEA)`，`scan_top = crossing +
    ///   REAL_TOP_SCAN_MARGIN`（软空气余量保证，见常数注），其上缓冲恒
    ///   AIR（`ChunkVoxels::filled(0)` 起点，`generate` 不上探）；
    /// - `depth_above` 计数与 SurfaceSystem.java:126-128/146 同式（空气
    ///   重置、流体不重置、实心 +1）；
    /// - #175 含水层落地后**实心判据 = density>0 ∪ Rock 封口袋**：
    ///   pressure 门可把 y>SEA 的 density≤0 格压回石头（原版同语义，
    ///   Aquifer.java:255-258 返回 null → defaultBlock），且含水层水体
    ///   液面可高于 SEA（randomized level ≤ min(地表, 40k+20±9)，
    ///   Aquifer.java:482-496）——旧「水格只在 y≤SEA、扫描窗不触及」
    ///   前提作废。判据必须走与缓冲侧同一个 `AquiferField` 语义：免采
    ///   样线按**树干列所属区块**取（`chunk_skip` 纯世界坐标函数，两侧
    ///   块重放恒等），否则跨块树基漂移（trees_cross_chunk 锁面）；
    /// - 首个实心格 y ≤ `TREE_MAX_Y-1` 即返回：材质按柱层循环同款
    ///   （[`solid_material`]）——山体内壁在扫描窗顶入窗时 depth_above
    ///   已 >1，材质归 STONE，与缓冲读数一致；
    /// - 窗上界钳 `TREE_MAX_Y-1`（旧 `tree_pass::ground_spot` 同窗），
    ///   全程无实心 → `None`。
    #[must_use]
    pub fn column_real_top(
        &self,
        block_x: i32,
        block_z: i32,
        field: &mut crate::vanilla::aquifer::AquiferField<'_>,
    ) -> Option<(i32, u16)> {
        use crate::vanilla::aquifer::Substance as Sub;
        let cs = self.column_state(block_x, block_z);
        let (surface_id, subsoil_id) = self.column_surface_ids(block_x, block_z, &cs);
        // 实物亚表带（#192 SURF-1 ②，与 generate 柱层循环同式）。
        let surface_depth = self.surface_depth(block_x, block_z);
        let jag = cs.unscaled_jag * half_negative(cs.jag_noise);
        let crossing = (128.0_f64 + 128.0 * (cs.offset + jag)).round() as i32;
        let scan_top =
            (crossing + REAL_TOP_SCAN_MARGIN).clamp(MIN_Y + 1, mcv_core::WORLD_MAX_Y - 1);
        let fill_top = scan_top.max(SEA);
        let min_surface = crossing - PRELIM_SURFACE_BAND;
        // 树干列所属区块的免采样线（缓冲在所属块 generate 时用同线构建）
        let skip = field.chunk_skip(block_x.div_euclid(16) * 16, block_z.div_euclid(16) * 16);
        let mut depth_above = 0_i32;
        // #175 终修实顶门（generate 同型，见该处注）：首个实心位
        // 以上 = 开放柱，本扫描窗恒 y>SEA → 恒 Air，不进胞格噪声
        // （压力封口袋不允许悬浮在半空被认作实顶——否则树基落点与
        // 缓冲实顶漂移）。
        let mut hit_solid = false;
        for y in ((SEA + 1)..=fill_top).rev() {
            let dval = self.density(&cs, block_x, y, block_z);
            let subst = if dval > 0.0 {
                None // 实心直判（含水层前置，Aquifer.java:150-152）
            } else if !hit_solid {
                Some(Sub::Air)
            } else {
                Some(field.substance(block_x, y, block_z, dval, skip))
            };
            if dval > 0.0 || matches!(subst, Some(Sub::Rock)) {
                hit_solid = true;
                depth_above += 1; // SurfaceSystem.java:146（同款计数）
                if y < crate::vanilla::tree_pass::TREE_MAX_Y {
                    let id = self.solid_material(
                        block_x,
                        y,
                        block_z,
                        min_surface,
                        depth_above,
                        surface_id,
                        subsoil_id,
                        surface_depth,
                    );
                    return Some((y + 1, id));
                }
            } else if subst == Some(Sub::Air) {
                depth_above = 0; // SurfaceSystem.java:126-128（空气重置；流体不重置）
            }
        }
        None
    }

    /// 采样并编排一列。
    #[must_use]
    pub fn column_state(&self, block_x: i32, block_z: i32) -> ColumnState {
        let sample = self.climate.sample(block_x, block_z);
        let c = sample.spline_climate();
        // offset 通道唯一常数是 GLOBAL_OFFSET（NoiseRouterData.java:152-160
        // `add(constant(-0.50375), spline(overworldOffset))`）；旧 BACKBONE_BIAS
        // 解常数（0.2 → v6 重解 0.0671875）随气候波长还原一并删除——地形保真
        // 波 T2（triage ②：整体抬升掩膜波长错误、扭曲岸线与平原落点带）。
        let offset = self.offset_spline.eval(&c) + GLOBAL_OFFSET;
        let factor = self.factor_spline.eval(&c);
        let unscaled_jag = self.jag_spline.eval(&c);
        // 地表零点：yClampGradient(-64,320,+1.5,-1.5) 与 offset 抵消处
        //（y_clamp 斜率 -1/128 格/密度单位）
        let surface_mc = (128.0 + 128.0 * offset).clamp(-64.0, 320.0);
        let surface_ours = (crate::vanilla::mc_y_to_ours(surface_mc).round() as i32)
            .clamp(MIN_Y + 4, mcv_core::WORLD_MAX_Y - 10);
        // jaggedness 通道：noise(JAGGED, 1500.0, 0.0)（NoiseRouterData.java:99），
        // yScale=0 → 列常量，提升到列级（每体素省 32 次倍频求值，值逐位不变）。
        let jag_x = f64::from(block_x) * 1500.0;
        let jag_z = f64::from(block_z) * 1500.0;
        let jag_noise = self.jagged.get_value(jag_x, 0.0, jag_z);
        // 层状意面海拔：2D 通道（yScale=0，NoiseRouterData.java:273-275），
        // 列常量；mapFromUnitTo(noise, -8.0, 8.0)。
        let elev_n = self
            .sp2d_elevation
            .get_value(f64::from(block_x), 0.0, f64::from(block_z));
        let sp2d_elev = span_map(elev_n, -8.0, 8.0);
        ColumnState {
            offset,
            factor,
            unscaled_jag,
            jag_noise,
            sp2d_elev,
            surface_ours,
            ocean: surface_ours <= SEA,
            erosion: sample.erosion,
        }
    }

    /// 含水层列探针（`aquifer::ColumnProbe` 的构造点）：crossing =
    /// 密度场解析过零面（原版 preliminarySurfaceLevel 的语义替身，登记
    /// AQUI-1），附 deep dark 门两条通道值（erosion 原值 + offset，
    /// depth = yClampedGradient(−64,320,1.5,−1.5)+offset，
    /// NoiseRouterData.java:180-182 在探针消费侧合成）。
    #[must_use]
    pub fn aquifer_probe(
        &self,
        block_x: i32,
        block_z: i32,
    ) -> crate::vanilla::aquifer::ColumnProbe {
        let cs = self.column_state(block_x, block_z);
        let jag = cs.unscaled_jag * half_negative(cs.jag_noise);
        let crossing = (128.0_f64 + 128.0 * (cs.offset + jag)).round() as i32;
        crate::vanilla::aquifer::ColumnProbe {
            prelim: crossing,
            offset: cs.offset,
            erosion: cs.erosion,
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
        let w1 = rarity
            * self
                .sp3d_1
                .get_value(bx / rarity, y_mc / rarity, bz / rarity)
                .abs();
        let w2 = rarity
            * self
                .sp3d_2
                .get_value(bx / rarity, y_mc / rarity, bz / rarity)
                .abs();
        // thickness = mapped(SPAGHETTI_3D_THICKNESS, -0.065, -0.088)
        let thick = span_map(self.sp3d_thickness.get_value(bx, y_mc, bz), -0.065, -0.088);
        let sp3d = (w1.max(w2) + thick).clamp(-1.0, 1.0);
        // bigEntrances = noise(CAVE_ENTRANCE, 0.75, 0.5) + 0.37 + yClampedGradient(-10, 30, 0.3, 0)
        let big_noise = self
            .cave_entrance
            .get_value(bx * 0.75, y_mc * 0.5, bz * 0.75);
        let big = big_noise + 0.37 + y_grad(y_mc, -10.0, 30.0, 0.3, 0.0);
        big.min(sp3d + self.spaghetti_roughness(bx, y_mc, bz))
    }

    /// spaghetti_2D 函数（NoiseRouterData.spaghetti2D:268-286）：层状意面雕刻带。
    fn spaghetti_2d(&self, cs: &ColumnState, bx: f64, y_mc: f64, bz: f64) -> f64 {
        // spaghetti2DRarityModulator = noise(SPAGHETTI_2D_MODULATOR, 2.0, 1.0)
        let rarity_mod = self.sp2d_mod.get_value(bx * 2.0, y_mc, bz * 2.0);
        let rarity = spaghetti_rarity_2d(rarity_mod);
        // WeirdScaledSampler TYPE2
        let cave = rarity
            * self
                .sp2d
                .get_value(bx / rarity, y_mc / rarity, bz / rarity)
                .abs();
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

    /// Pillars 函数（`density_function/overworld/caves/pillars.json` 逐节点
    /// 对位，cache_once 为纯缓存不改语义）：实物外层是 **add**——
    /// `mul{ add{ mul{2.0, P}, add{−1.0, mul{−1.0, R}} }, cube{…} }`，
    /// 即 `(2·P − 1 − R) · cube(0.55 + 0.55·T)`。
    /// 旧实现把该 add 误作 mul（`2P·(−1−R)·T³`）：场符号翻转、激活域镜像
    /// ——P=0.9/R=0 实物门开（+0.8·c³），旧实现门关；P=−0.9/R=0 实物门关
    /// （−2.8·c³），旧实现门开（python 对拍见本刀提交信息）。
    /// （P=pillar noise xz_scale 25.0/y_scale 0.3；R=pillar_rareness
    /// xz/y 1.0；T=pillar_thickness xz/y 1.0）。消费侧（final_density
    /// out_of_range 支外层 max）：值 < 0.03 → −10⁶（max 恒不生效）；
    /// 值 ≥ 0.03 → 把奶酪洞内的 −密度抬正 = 洞中独立石柱。
    fn pillars(&self, bx: f64, y_mc: f64, bz: f64) -> f64 {
        let p = self.pillar.get_value(bx * 25.0, y_mc * 0.3, bz * 25.0);
        let r = self.pillar_rareness.get_value(bx, y_mc, bz);
        let t = self.pillar_thickness.get_value(bx, y_mc, bz);
        let c = 0.55 + 0.55 * t;
        // 实物 add 结构（pillars.json :5-31）：add(mul(2.0,P), add(−1, mul(−1,R)))
        ((2.0 * p) + (-1.0 - r)) * (c * c * c)
    }

    /// Noodle 函数（`density_function/overworld/caves/noodle.json` 逐节点
    /// 对位）：掩膜 = noise(noodle, 1, 1)，range_choice
    /// `y ∈ [-60, 321)` 越界 → −1（本域 y ≥ −64：仅 y ∈ [−64, −61) 越界，
    /// 该带 slide 底封恒实心，无观感差）；掩膜 ∈ [−10⁶, 0) → 64（min 不
    /// 生效）；掩膜 ≥ 0 → 管状雕刻
    /// `−0.075… − 0.025·thickness + 1.5·max(|ridgeA|, |ridgeB|)`
    /// （ridge 族 xz/y 缩放 2.6666…，thickness 1.0）。
    /// interpolated 外壳按模块头注既定口径跳过（判定语义不变）。
    fn noodle(&self, bx: f64, y_mc: f64, bz: f64) -> f64 {
        if y_mc < -60.0 {
            return NOODLE_OFF; // range_choice when_out_of_range = -1 → <0 → 64
        }
        let mask = self.noodle_noise.get_value(bx, y_mc, bz);
        if mask < 0.0 {
            return NOODLE_OFF;
        }
        let t = self.noodle_thickness.get_value(bx, y_mc, bz);
        let s = NOODLE_RIDGE_SCALE;
        let a = self.noodle_ridge_a.get_value(bx * s, y_mc * s, bz * s);
        let b = self.noodle_ridge_b.get_value(bx * s, y_mc * s, bz * s);
        -0.075_000_000_000_000_01_f64 - 0.025 * t + 1.5 * a.abs().max(b.abs())
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
        // base_3d_noise：vanilla BlendedNoise 破面（xz/yMultiplier=
        // 684.412·scale 已在结构内，BlendedNoise.java:64-65；密度函数上下文
        // 对 base_3d 无外层乘子——旧 fork 的 (x·2, y·0.75, z·2) 随迁删除，
        // NoiseRouterData.java:86 注册原样引用）。
        let base_3d_v = self.base_3d.get_value(bx, y_mc, bz);
        let sloped = initial + base_3d_v;
        // 双洞穴（sloped_cheese 的 rangeChoice 分界 1.5625，overworld.json
        // final_density 的 range_choice max_exclusive）
        let e = self.entrances(bx, y_mc, bz);
        let caves = if sloped < 1.5625 {
            sloped.min(5.0 * e)
        } else {
            let sp2d = self.spaghetti_2d(cs, bx, y_mc, bz);
            let u = self.underground(bx, y_mc, bz, sloped);
            let base = (u.min(e)).min(sp2d + self.spaghetti_roughness(bx, y_mc, bz));
            // pillars 门（final_density out_of_range 外层 max，#192 补全）：
            // 值 < 0.03 → −10⁶（max 恒不生效），否则以 pillars 值抬升——
            // 把已雕负的洞位抬正 = 奶酪洞内的独立石柱。
            let p = self.pillars(bx, y_mc, bz);
            base.max(if p < PILLARS_GATE_MIN { CARVE_OFF } else { p })
        };
        // slide（slideOverworld: -64, 384, 80, 64, -0.078125, 0, 24, 0.1171875）
        let top_factor = y_grad(y_mc, 240.0, 256.0, 1.0, 0.0);
        let v = lerp1(-0.078125, caves, top_factor);
        let bottom_factor = y_grad(y_mc, -64.0, -40.0, 0.0, 1.0);
        let v = lerp1(0.1171875, v, bottom_factor);
        // postProcess = squeeze(0.64·slide)，与 noodle 取 min（final_density
        // 最外层 min，noodle.json——掩膜外=64 不生效，掩膜内雕细长管洞）。
        let noodle_v = self.noodle(bx, y_mc, bz);
        squeeze(0.64 * v).min(noodle_v)
    }

    /// 测试出口：base_3d 通道在 density() 同坐标约定下的原始值（破面输入）。
    /// 地形保真波 T4 红测试（quality.rs base_3d_channel_*）统计该通道的
    /// σ 与相邻块 |Δ| p90；T4 接入 BlendedNoise 后乘子 (x·2, y·0.75, z·2)
    /// 已随迁入 BlendedNoise 内部（BlendedNoise.java:64-65 xz/yMultiplier），
    /// 本出口坐标约定不变。
    #[must_use]
    pub fn base_3d_sample(&self, block_x: i32, block_y: i32, block_z: i32) -> f64 {
        let bx = f64::from(block_x);
        let bz = f64::from(block_z);
        let y_mc = crate::vanilla::ours_y_to_mc(f64::from(block_y));
        self.base_3d.get_value(bx, y_mc, bz)
    }

    /// 一列的 MultiNoise 群系（复用本编排器的气候通道；pass 2 树投影的
    /// 群系密度门/树种门用，分类入口 = [`crate::vanilla::biome_from_sample`]，
    /// 不另建平行分类）。
    #[must_use]
    pub fn biome_at(&self, block_x: i32, block_z: i32) -> crate::vanilla::Biome {
        crate::vanilla::biome_from_sample(&self.climate.sample(block_x, block_z))
    }

    /// 26.1 vertical_gradient 条件（SurfaceRules.java:735-781
    /// `VerticalGradientConditionSource.apply` → `VerticalGradientCondition
    /// .compute`）：`blockY <= trueAtAndBelow` 恒真、`blockY >= falseAtAndAbove`
    /// 恒假，之间线性概率 `Mth.map(y, true, false, 1.0, 0.0)` 与逐位随机
    /// `random.nextFloat() < probability` 比较（SurfaceRules.java:764-775，
    /// 每次 compute 新建 `randomFactory.at(x,y,z)`——本方法同构，无共享
    /// 游标）。随机链 = RandomState.java:113-115
    /// `getOrCreateRandomFactory(name) = random.fromHashOf(name)
    /// .forkPositional()` 实物（[`crate::vanilla::xoroshiro`]，#192 RNG-1
    /// 销账：同 seed 同坐标与原版同位型，不再是 hash01 通道域替身）。
    #[must_use]
    fn vertical_gradient_hit(
        &self,
        factory: &crate::vanilla::xoroshiro::PositionalRandomFactory,
        wx: i32,
        y: i32,
        wz: i32,
        true_at_and_below: i32,
        false_at_and_above: i32,
    ) -> bool {
        if y <= true_at_and_below {
            return true;
        }
        if y >= false_at_and_above {
            return false;
        }
        // Mth.map(double, inMin, inMax, 1.0, 0.0)
        //   = (v−inMin)·(0−1)/(inMax−inMin) + 1 = 1 − t（取负/除法均为
        // IEEE 精确步，与「先除后减」同位型）。
        let t =
            f64::from(y - true_at_and_below) / f64::from(false_at_and_above - true_at_and_below);
        // 原版比较 = float→double 拓宽后 `<` double 概率（Java 二元数值提升）。
        f64::from(factory.at(wx, y, wz).next_float()) < 1.0 - t
    }

    /// defaultBlock（石头）位的 surface_rule 终判——overworld.json surface_rule
    /// 树实物 top[0]+top[2]（data/minecraft/worldgen/noise_settings/
    /// overworld.json:446-463 bedrock_floor 段 + 树尾 deepslate 段），只对「本位
    /// 将出 STONE」的格生效（原版 SurfaceSystem.java:147 `old ==
    /// this.defaultBlock` 是规则应用门：草/土/沙等表层材质不参与替换）：
    /// - top[0] bedrock_floor：vertical_gradient true_at_and_below=
    ///   above_bottom(0)、false_at_and_above=above_bottom(5)、random_name
    ///   "minecraft:bedrock_floor" → y≤−64 恒基岩，−63..−60 概率
    ///   0.8/0.6/0.4/0.2，≥−59 恒假；
    /// - top[2] deepslate：vertical_gradient true_at_and_below=absolute(0)、
    ///   false_at_and_above=absolute(8)、random_name "minecraft:deepslate" →
    ///   y≤0 恒深板岩，1..7 概率 (8−y)/8，y≥8 留石。
    ///
    /// 序列次序 bedrock→（above_preliminary 表层带）→deepslate 与原版 sequence
    /// 一致：bedrock 散点未中的位落到 deepslate 项（y≤0 恒真）——−63..−60
    /// 基岩散点带的缝隙出深板岩而非石头（实物语义）。
    #[must_use]
    fn stone_replacement(&self, wx: i32, y: i32, wz: i32) -> u16 {
        if self.vertical_gradient_hit(&self.bedrock_random, wx, y, wz, MIN_Y, MIN_Y + 5) {
            BEDROCK
        } else if self.vertical_gradient_hit(&self.deepslate_random, wx, y, wz, 0, 8) {
            DEEPSLATE
        } else {
            STONE
        }
    }
}

fn squeeze(c: f64) -> f64 {
    let c = c.clamp(-1.0, 1.0);
    c / 2.0 - c * c * c / 24.0
}

/// 统一入口：生成一个区块（pass 1 柱层序 + pass 2 树投影）。
///
/// # Errors
/// 恒返回 `Ok(())`；保留 `Result` 以与 FFI 路径同构。
pub fn generate(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    out_voxels: &mut [u16],
    out_heightmap: &mut [i16],
) -> Result<(), i32> {
    debug_assert!(out_voxels.len() >= mcv_core::CHUNK_VOL);
    debug_assert!(out_heightmap.len() >= 256);
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    let orch = Orchestrator::new(seed);
    // #175 含水层：density≤0 空腔的终判（原版 NoiseBasedAquifer 结构
    // 移植，aquifer.rs 头注机制摘要）。缓存只省算力；本区块判据用本
    // 区块免采样线（chunk_skip，Aquifer.java:134-137）。
    let aq = crate::vanilla::aquifer::Aquifer::new(seed);
    let mut field = crate::vanilla::aquifer::AquiferField::new(&aq, &orch);
    let skip = field.chunk_skip(base_x, base_z);

    for cz in 0..16_i32 {
        for cx in 0..16_i32 {
            let wx = base_x.wrapping_add(cx);
            let wz = base_z.wrapping_add(cz);
            let cs = orch.column_state(wx, wz);
            // 群系面表（#140 表层波，滩/雪带 + 群系顶格判定——判定语义与
            // 引注集中在 Orchestrator::column_surface_ids，树 pass 的
            // column_real_top 共用同一函数，杜绝两套表层规则漂移）。
            let (surface_id, subsoil_id) = orch.column_surface_ids(wx, wz, &cs);
            // 实物亚表带（#192 SURF-1 ②：getSurfaceDepth 逐列值替旧常数 3，
            // SurfaceSystem.java:165-168 实物；与 column_real_top 同式）。
            let surface_depth = orch.surface_depth(wx, wz);
            // 表层锚 = 密度场**真实过零面**（逐列实顶扫描），不再锚样条估计
            // 面 `surface_ours`（旧 y==surface→GRASS 的 ±1-2 格偏差把暴露层
            // 打进 DIRT 带 = 用户可见整片裸土）。26.1 同语义：
            // SurfaceSystem.buildSurface（SurfaceSystem.java:106-163）逐列从
            // WORLD_SURFACE_WG 高度图顶自顶向下扫描，实心 stoneAboveDepth++
            // （:146）、空气重置（:126-128）；ON_FLOOR = stoneDepthAbove ≤ 1
            // （SurfaceRules.java:680 StoneDepthCheck）。解析过零面 =
            // 128+128·(offset+jag)（jag 与 density() 内同式、列常量），向上
            // 留 REAL_TOP_SCAN_MARGIN 软空气余量后并进同一趟自顶向下循环
            // （扫描即填充，无二次 density 求值）。
            let jag = cs.unscaled_jag * half_negative(cs.jag_noise);
            let crossing = (128.0_f64 + 128.0 * (cs.offset + jag)).round() as i32;
            let scan_top =
                (crossing + REAL_TOP_SCAN_MARGIN).clamp(MIN_Y + 1, mcv_core::WORLD_MAX_Y - 1);
            let fill_top = scan_top.max(SEA);
            // abovePreliminarySurface 门（SurfaceRuleData.java:278 全表层规则
            // 的外包；SurfaceRules.java:379 = preliminary + surfaceDepth − 8，
            // 此处 surfaceDepth 取 0 的块级简化）——带下实心一律 STONE。
            let min_surface = crossing - PRELIM_SURFACE_BAND;
            // vanilla 上下文（SurfaceSystem.java:119-148）：自顶向下，
            // 空气重置 stoneAboveDepth、流体不重置只记水面（:129-132）、
            // 实心 +1；depth_above==1 即 ON_FLOOR 顶层格。
            let mut depth_above = 0_i32;
            // #175 终修（geometry_locks 裁决）实顶门：自顶向下扫描
            // 未碰到首个实心格前 = 开放大气柱，走无条件全局流体
            // （`aquifer::open_column`，引注集中在该函数头注：原版
            // skip 线以上 globalFluid.at 短路 Aquifer.java:159-161 +
            // 胞体戳出修正面的 pokes/global 段 :416-429）；碰到实心
            // 后 = 地表以下岩层空腔，才允许进含水层胞格终判。
            // column_real_top 与 lake_pass Replay 共用同型门，缓冲与
            // 重放由构造一致。
            let mut hit_solid = false;
            for y in (MIN_Y..=fill_top).rev() {
                let dval = orch.density(&cs, wx, y, wz);
                // #192：旧「y==MIN_Y 恒基岩 + 其上 4 格 50% 噪声」前置分支
                // 删除——实物里基岩是 surface_rule 的 vertical_gradient
                // （overworld.json top[0]），只对 defaultBlock=石头位生效
                // （洞腔/流体不换）；y=−64 底部 slide 恒实心
                // （final_density bottom 常数 0.1171875 → squeeze ≈ +0.037）
                // 故该行照旧 100% 基岩，只是改经 stone_replacement。
                let id = if dval <= 0.0 && hit_solid {
                    // #175：含水层终判取代旧「y≤SEA 一律灌水」平铺模型
                    //（1.18 前旧语义；原版地下默认干、注水由胞格噪声门控
                    //——aquifer.rs 头注 file:line 摘要）。流体两态不重置
                    // depth_above（SurfaceSystem.java:129-132）、空气重置
                    //（:126-128）、压力封口袋=实心计 +1（原版该位出
                    // defaultBlock，Aquifer.java:255-258 返回 null 经
                    // NoiseChunk.java:160 兜底成石头）。
                    use crate::vanilla::aquifer::Substance as Sub;
                    let subst = field.substance(wx, y, wz, dval, skip);
                    match subst {
                        Sub::Rock => {
                            depth_above += 1;
                            orch.solid_material(
                                wx,
                                y,
                                wz,
                                min_surface,
                                depth_above,
                                surface_id,
                                subsoil_id,
                                surface_depth,
                            )
                        }
                        Sub::Air => {
                            depth_above = 0; // SurfaceSystem.java:126-128
                            AIR
                        }
                        Sub::Water => WATER,
                        Sub::Lava => LAVA,
                    }
                } else if dval <= 0.0 {
                    // 开放柱：洋面恒灌满 / 大气 / 全局岩浆海（三态
                    // 见 open_column；流体不重置 depth_above，此时
                    // 计数恒 0 无需动作）。
                    match crate::vanilla::aquifer::open_column(y) {
                        crate::vanilla::aquifer::Substance::Lava => LAVA,
                        crate::vanilla::aquifer::Substance::Water => WATER,
                        _ => AIR,
                    }
                } else {
                    hit_solid = true;
                    depth_above += 1; // SurfaceSystem.java:146
                    orch.solid_material(
                        wx,
                        y,
                        wz,
                        min_surface,
                        depth_above,
                        surface_id,
                        subsoil_id,
                        surface_depth,
                    )
                };
                out_voxels[mcv_core::vidx(cx as usize, y, cz as usize)] = id;
            }
        }
    }
    // Pass 1.5（#175 刀C）：岩浆湖特征（26.1 lake_lava_underground
    // 1/9 + lake_lava_surface 1/200，LakeFeature 判据环 + barrier 铺
    // 石，见 lake_pass 模块头注实证摘要）。特征阶早于植被——置于树
    // pass 前，与原版 lakes < vegetation 阶序一致。
    crate::vanilla::lake_pass::place_lakes(seed, chunk_x, chunk_z, &orch, &mut field, out_voxels);
    // Pass 1.7（#178 刀3b）：矿石投影（26.1 UNDERGROUND_ORES 阶，
    // GenerationStep.java:8-18 枚举序 LAKES<UNDERGROUND_ORES<
    // VEGETAL_DECORATION → 湖后树前；执行器与重放模型见 ore_pass 头注）。
    crate::vanilla::ore_pass::project_ores(seed, chunk_x, chunk_z, &orch, &mut field, out_voxels);
    // Pass 2（#139 树接线）：5×5 树池邻域扫描——树落点世界坐标纯函数；
    // 密度门/树种门走群系分类（原版 per-chunk 尝试数换算，见 tree_pass
    // 模块头注），树基 = pass-1 体素实顶+1，替换门对位原版 validTreePos。
    crate::vanilla::tree_pass::project_trees(seed, chunk_x, chunk_z, &orch, out_voxels);
    // 高度图与 cpp kernel pass 3 / 存档重算（crate::recompute_heightmap）
    // 同一谓词（damp!=0 且非流体）——直接复用，三方由构造保证逐块一致。
    // （此前 vanilla 路径漏写高度图，恒 0 提交：出生列搜索全部 reject。）
    // 树投影在高度图重算前完成：树荫计入 heightmap（legacy pass 3 同序）。
    let hm = crate::recompute_heightmap(out_voxels);
    out_heightmap[..256].copy_from_slice(&hm[..]);
    Ok(())
}

/// 表层亚表带深（depth_above ∈ [2, 4] 层 = 26.1 UNDER_FLOOR 带
/// `stoneDepthCheck(0, true, FLOOR)` = stoneDepthAbove ≤ 1 + surfaceDepth
/// （SurfaceRules.java:676-680；surfaceDepth = surface 噪声 0..4 取块级常数 3，
/// SurfaceSystem.java:165-168）。
const SUBSOIL_DEPTH: i32 = 3;

/// 实心格材质（#175 抽公共：pass-1 实心分支与含水层 Rock 封口袋分支、
/// `column_real_top` 三处共用，杜绝规则漂移）。语义原样：带下 STONE 兜底
/// （abovePreliminarySurface 门）、depth_above==1 顶格、≤1+SUBSOIL_DEPTH
/// 亚表带、其余 STONE。
#[must_use]
pub(crate) const fn solid_material(
    y: i32,
    min_surface: i32,
    depth_above: i32,
    surface_id: u16,
    subsoil_id: u16,
) -> u16 {
    if y < min_surface {
        STONE
    } else if depth_above == 1 {
        surface_id
    } else if depth_above <= 1 + SUBSOIL_DEPTH {
        subsoil_id
    } else {
        STONE
    }
}

pub(crate) fn bedrock_noise(seed: u64, wx: i32, y: i32, wz: i32) -> bool {
    let h = crate::vanilla::hash01(
        crate::vanilla::channel_seed(seed, 0x0D00_D00D),
        i64::from(wx),
        i64::from(y),
        i64::from(wz),
    );
    h < 0.5
}
