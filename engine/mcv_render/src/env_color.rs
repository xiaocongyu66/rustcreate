//! 时间→色彩轨道（26.1 overworld 天象）。
//!
//! **版权红线**：Mojang 数据文件（`data/minecraft/timeline/day.json` 等）
//! 绝不入仓——本模块把 day.json 的关键帧 tick/颜色**硬编码为 Rust 常数**，
//! 逐条注明原版源 file:line（先例：`celestial.rs moon_phase` ←
//! timeline/moon.json）。数值全部核对自：
//! - 关键帧/轨道结构：`net/minecraft/world/timeline/Timelines.java`
//!   （bootstrap 内 OVERWORLD_DAY，:43-159）；
//! - 属性默认值：`net/minecraft/world/attribute/EnvironmentAttributes.java`；
//! - 维度基色：`data/minecraft/dimension_type/overworld.json`（仅在此引用
//!   其键名与取值，文件本身不拷贝）。
//!
//! 插值语义：Timeline 数值/颜色轨默认**逐 tick 线性插值**、周期 24000
//! 回绕（段 = 末关键帧 → 次周期首关键帧）；`MULTIPLY_RGB` 乘子轨只乘
//! RGB（alpha 位无关）。曲线基色 × 乘子即当帧色。

use glam::Vec3;

/// 渲染侧周期（day.json setPeriodTicks(24000)，Timelines.java:46）。
pub const PERIOD_TICKS: u64 = 24_000;

// ---- 基色（dimension_type/overworld.json "minecraft:visual/*"）----
/// `visual/sky_color` = #78a7ff（overworld.json:37）。
pub const SKY_COLOR: Vec3 = Vec3::new(0.470_588_24, 0.654_902, 1.0);
/// `visual/fog_color` = #c0d8ff（overworld.json:36）。
pub const FOG_COLOR: Vec3 = Vec3::new(0.752_941_2, 0.847_058_83, 1.0);
/// `visual/ambient_light_color` = #0a0a0a（overworld.json:33）。
pub const AMBIENT_LIGHT_COLOR: Vec3 = Vec3::splat(0.039_215_688);
/// `visual/block_light_tint` = 0xFFD88C（EnvironmentAttributes.java:71-73
/// 默认 -10100；overworld 未覆写）——方块光暖橙。
pub const BLOCK_LIGHT_TINT: Vec3 = Vec3::new(1.0, 0.847_058_83, 0.549_019_63);
/// `visual/water_fog_color` = 0xFF050533（EnvironmentAttributes.java:33-35
/// 默认 -16448205）。
pub const WATER_FOG_COLOR: Vec3 = Vec3::new(0.019_607_844, 0.019_607_844, 0.2);

// ---- Timelines.java:30-34 夜段常数 ----
/// NIGHT_SKY_LIGHT_COLOR = ARGB(1.0, 0.48, 0.48, 1.0)（Timelines.java:30）。
pub const NIGHT_SKY_LIGHT_COLOR: Vec3 = Vec3::new(0.48, 0.48, 1.0);
/// NIGHT_FOG_COLOR_MULTIPLIER = (0.06, 0.06, 0.09)（Timelines.java:33）。
pub const NIGHT_FOG_COLOR_MULTIPLIER: Vec3 = Vec3::new(0.06, 0.06, 0.09);
/// NIGHT_SKY_COLOR_MULTIPLIER = 纯黑 -16777216（Timelines.java:32）。
pub const NIGHT_SKY_COLOR_MULTIPLIER: Vec3 = Vec3::ZERO;
/// NIGHT_CLOUD_COLOR_MULTIPLIER = (0.1, 0.1, 0.15)（Timelines.java:34）。
pub const NIGHT_CLOUD_COLOR_MULTIPLIER: Vec3 = Vec3::new(0.1, 0.1, 0.15);

/// 白 = MULTIPLY_RGB 轨的 -1 关键帧。
const WHITE: Vec3 = Vec3::ONE;

/// 关键帧线性插值（tick 回绕 24000；keys 升序）。段序 = [首帧,末帧] 各相邻
/// 对 + 末段回绕 [末帧, 首帧+24000]（覆盖 t < 首帧 与 t ≥ 末帧 两段）。
fn track_vec3(t: u64, keys: &[(u64, Vec3)]) -> Vec3 {
    let t = t % PERIOD_TICKS;
    if keys.len() == 1 {
        return keys[0].1;
    }
    for w in keys.windows(2) {
        let (t0, v0) = w[0];
        let (t1, v1) = w[1];
        if t >= t0 && t < t1 {
            let f = (t - t0) as f32 / (t1 - t0).max(1) as f32;
            return v0 + (v1 - v0) * f;
        }
    }
    // 末段回绕：末帧 → 首帧（+24000）；t < 首帧时换算进 +24000 坐标。
    let (t0, v0) = keys[keys.len() - 1];
    let (t1, v1) = (keys[0].0 + PERIOD_TICKS, keys[0].1);
    let tt = if t < keys[0].0 { t + PERIOD_TICKS } else { t };
    let f = (tt - t0) as f32 / (t1 - t0).max(1) as f32;
    v0 + (v1 - v0) * f
}

fn track_f32(t: u64, keys: &[(u64, f32)]) -> f32 {
    let lifted: Vec<(u64, Vec3)> = keys.iter().map(|(k, v)| (*k, Vec3::splat(*v))).collect();
    track_vec3(t, &lifted).x
}

/// SKY_COLOR 昼夜乘轨（Timelines.java:65-69 MULTIPLY_RGB：133/11867 白 →
/// 13670/22330 纯黑；夜里乘 0 = 黑，与 sky_light_factor 的 0.24 底是两条
/// 独立轨道——后者是亮度、前者是颜色，见审计 B 根因 A）。
pub fn sky_multiplier(t: u64) -> Vec3 {
    track_vec3(
        t,
        &[
            (133, WHITE),
            (11_867, WHITE),
            (13_670, NIGHT_SKY_COLOR_MULTIPLIER),
            (22_330, NIGHT_SKY_COLOR_MULTIPLIER),
        ],
    )
}

/// FOG_COLOR 昼夜乘轨（Timelines.java:57-64：133/11867 白 → 13670/22330
/// (0.06,0.06,0.09)）。
pub fn fog_multiplier(t: u64) -> Vec3 {
    track_vec3(
        t,
        &[
            (133, WHITE),
            (11_867, WHITE),
            (13_670, NIGHT_FOG_COLOR_MULTIPLIER),
            (22_330, NIGHT_FOG_COLOR_MULTIPLIER),
        ],
    )
}

/// SKY_LIGHT_COLOR 乘轨（Timelines.java:70-73：730/11270 白 → 13140/22860
/// NIGHT_SKY_LIGHT_COLOR #7a7aff 深蓝）——lightmap 天光颜色。
pub fn sky_light_color(t: u64) -> Vec3 {
    track_vec3(
        t,
        &[
            (730, WHITE),
            (11_270, WHITE),
            (13_140, NIGHT_SKY_LIGHT_COLOR),
            (22_860, NIGHT_SKY_LIGHT_COLOR),
        ],
    )
}

/// CLOUD_COLOR 乘轨（Timelines.java:136-143 MULTIPLY_ARGB：133/11867 白 →
/// 13670/22330 (0.1,0.1,0.15)）。
pub fn cloud_multiplier(t: u64) -> Vec3 {
    track_vec3(
        t,
        &[
            (133, WHITE),
            (11_867, WHITE),
            (13_670, NIGHT_CLOUD_COLOR_MULTIPLIER),
            (22_330, NIGHT_CLOUD_COLOR_MULTIPLIER),
        ],
    )
}

/// STAR_BRIGHTNESS 轨（Timelines.java:120-135 FloatModifier.MAXIMUM 12 帧，
/// 基值 0 → 轨值即亮度）：白天 0，11373→13228 爬到 0.5，22772→23758 回落。
pub fn star_brightness(t: u64) -> f32 {
    track_f32(
        t,
        &[
            (92, 0.037),
            (627, 0.0),
            (11_373, 0.0),
            (11_732, 0.016),
            (11_959, 0.044),
            (12_399, 0.143),
            (12_729, 0.258),
            (13_228, 0.5),
            (22_772, 0.5),
            (23_032, 0.364),
            (23_356, 0.225),
            (23_758, 0.101),
        ],
    )
}

/// SUNRISE_SUNSET_COLOR 32 关键帧（Timelines.java:87-118，ARGB #AARRGGBB，
/// 即 day.json:356-487 的同一张表）：晨昏橙染带。返回 (rgb 不透明色, alpha)。
/// 格式说明：71 起 #5FEFA333（A=0.373 橙）→ 730 后 A=0 全天隐身 → 傍晚
/// 12732-12841 alpha 峰值 0.996 #FEDA6333 → 14192 归 0。
pub fn sunrise_sunset(t: u64) -> (Vec3, f32) {
    // u32 十六进制即原版 ARGB 字面（i32 负数位型转写，注释含 rgb 分量）。
    const KEYS: [(u64, u32); 32] = [
        (71, 0x5fefa333),     // (239,163,51)
        (310, 0x29f5ba33),    // (245,186,51)
        (565, 0x06fbd433),    // (251,212,51)
        (730, 0x00ffe533),    // (255,229,51)
        (11_270, 0x00ffe533), // (255,229,51)
        (11_397, 0x04fcd833), // (252,216,51)
        (11_522, 0x0ff9cb33), // (249,203,51)
        (11_690, 0x29f5ba33), // (245,186,51)
        (11_929, 0x5fefa333), // (239,163,51)
        (12_243, 0xb1e78733), // (231,135,51)
        (12_358, 0xcce47e33), // (228,126,51)
        (12_512, 0xe9e07233), // (224,114,51)
        (12_613, 0xf6dd6b33), // (221,107,51)
        (12_732, 0xfeda6333), // (218,99,51)
        (12_841, 0xfed75c33), // (215,92,51)
        (13_035, 0xecd25133), // (210,81,51)
        (13_252, 0xc1cc4733), // (204,71,51)
        (13_775, 0x36be3733), // (190,55,51)
        (13_888, 0x1fbb3533), // (187,53,51)
        (14_039, 0x09b73333), // (183,51,51)
        (14_192, 0x00b33333), // (179,51,51)
        (21_807, 0x00b23333), // (178,51,51)
        (21_961, 0x09b73333), // (183,51,51)
        (22_112, 0x1fbb3533), // (187,53,51)
        (22_225, 0x36be3733), // (190,55,51)
        (22_748, 0xc1cc4733), // (204,71,51)
        (22_965, 0xecd25133), // (210,81,51)
        (23_159, 0xfed75c33), // (215,92,51)
        (23_272, 0xfeda6333), // (218,99,51)
        (23_488, 0xe9e07233), // (224,114,51)
        (23_642, 0xcce47e33), // (228,126,51)
        (23_757, 0xb1e78733), // (231,135,51)
    ];
    // 注意：不能把 packed u32 位型当 f32 插值（跨符号/指数位非线性）——
    // 正确做法 = 先解码后逐通道插值（ARGB.srgbLerp 语义）：四条标量轨。
    let mut alpha_keys = Vec::with_capacity(KEYS.len());
    let mut r_keys = Vec::with_capacity(KEYS.len());
    let mut g_keys = Vec::with_capacity(KEYS.len());
    let mut b_keys = Vec::with_capacity(KEYS.len());
    for (tick, argb) in KEYS {
        alpha_keys.push((tick, ((argb >> 24) & 255) as f32 / 255.0));
        r_keys.push((tick, ((argb >> 16) & 255) as f32 / 255.0));
        g_keys.push((tick, ((argb >> 8) & 255) as f32 / 255.0));
        b_keys.push((tick, (argb & 255) as f32 / 255.0));
    }
    let rgb = Vec3::new(
        track_f32(t, &r_keys),
        track_f32(t, &g_keys),
        track_f32(t, &b_keys),
    );
    (rgb, track_f32(t, &alpha_keys))
}

/// ARGB.srgbLerp（ARGB.java:155-161）：8bit 通道逐分量线性插值（「srgb」
/// 指在 sRGB 空间插值，区别于 linearLerp 的线性空间往返）。
pub fn srgb_lerp(a: f32, p0: Vec3, p1: Vec3) -> Vec3 {
    p0 + (p1 - p0) * a.clamp(0.0, 1.0)
}

/// AtmosphericFogEnvironment.getBaseColor（:42-47）的天穹混雾因子：
/// `skyFogEnd = min(SKY_FOG_END/16, rd_chunks)`（SKY_FOG_END_DISTANCE 默认
/// 512，EnvironmentAttributes.java:25-27）、
/// `mixFactor = 1 − clampedLerp(skyFogEnd/32, 0.25, 1)^0.25`。
/// vd=8 → mixFactor = 1 − 0.25^0.25 ≈ 0.2929（审计 A Q4 口径）。
pub fn sky_fog_mix_factor(rd_chunks: f32) -> f32 {
    let sky_fog_end = (512.0_f32 / 16.0).min(rd_chunks.max(0.0));
    let clamped = (sky_fog_end / 32.0).clamp(0.25, 1.0);
    1.0 - clamped.powf(0.25)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Timelines.java:57-73 关键帧处的轨道值（硬编码表与原版 Java 常数对账）。
    #[test]
    fn track_keyframes_match_vanilla() {
        // 白天平台（133..11867 之间）。
        assert_eq!(sky_multiplier(6_000), Vec3::ONE);
        assert_eq!(fog_multiplier(6_000), Vec3::ONE);
        assert_eq!(sky_light_color(6_000), Vec3::ONE);
        assert_eq!(cloud_multiplier(6_000), Vec3::ONE);
        // 夜平台：SKY 纯黑、FOG (0.06,0.06,0.09)、天光 #7a7aff、云 (0.1,0.1,0.15)。
        let night = 18_000;
        assert_eq!(sky_multiplier(night), Vec3::ZERO);
        assert_eq!(fog_multiplier(night), NIGHT_FOG_COLOR_MULTIPLIER);
        assert_eq!(sky_light_color(night), NIGHT_SKY_LIGHT_COLOR);
        assert_eq!(cloud_multiplier(night), NIGHT_CLOUD_COLOR_MULTIPLIER);
        // 临界 tick：乘轨 11867 仍白、13670 已夜色（B 报「关键错位」窗口）。
        assert_eq!(sky_multiplier(11_867), Vec3::ONE);
        assert_eq!(sky_multiplier(13_670), Vec3::ZERO);
        // 星轨：白天 0、深夜 0.5（13228..22772 平台）。
        assert_eq!(star_brightness(6_000), 0.0);
        assert!((star_brightness(18_000) - 0.5).abs() < 1e-6);
    }

    /// Timelines.java:87-118 — 晨昏带：白天隐身、傍晚 alpha 峰值 0.996。
    #[test]
    fn sunrise_sunset_windows() {
        let (c, a) = sunrise_sunset(6_000);
        assert!(a.abs() < 1e-6, "正午隐身 alpha={a}");
        let (_, dusk) = sunrise_sunset(12_800);
        assert!((dusk - 0.996).abs() < 0.01, "傍晚峰值 {dusk}");
        let (dawn_c, dawn_a) = sunrise_sunset(71);
        assert!((dawn_a - 0.373).abs() < 0.01);
        assert!(
            dawn_c.x > 0.9 && dawn_c.y > 0.5 && dawn_c.z < 0.3,
            "晨橙 {dawn_c}"
        );
        // 夜里（14192..21807）全程不可见。
        assert_eq!(sunrise_sunset(18_000).1, 0.0);
    }

    /// AtmosphericFogEnvironment.java:44-47 — vd=8 → mixFactor 0.2929；
    /// rd≥32 chunks → 0（雾不吃天色）。
    #[test]
    fn sky_fog_mix_factor_vanilla() {
        assert!((sky_fog_mix_factor(8.0) - 0.292_893_22).abs() < 1e-6);
        assert_eq!(sky_fog_mix_factor(64.0), 0.0);
    }
}
