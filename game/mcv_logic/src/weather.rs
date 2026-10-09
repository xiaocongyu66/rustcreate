//! 天气状态机（26.1 `ServerLevel.advanceWeatherCycle` + `WeatherAttributes`
//! 视觉修饰；审计后遗留项）。旧实现无天气：`sky_darken` 注释自注"雷暴
//! skyDarken=10：无天气系统"。
//!
//! 源码依据（类名:行号均为 26.1 反编译实读；天气不叫 RainLevel 类——
//! 派单说法有误，状态机在 `ServerLevel.advanceWeatherCycle`（ServerLevel
//! .java:694-755），`rainLevel/thunderLevel` 字段与 ±0.01/tick 随机游走在
//! `Level.java:118-120/914-930`，时长采样器 ServerLevel.java:188-191）：
//! - **状态机**（ServerLevel.java:699-741）：`clearWeatherTime > 0` 递减并
//!   强制转晴（thunderTime/rainTime 置 0/1）；否则两个独立倒计时
//!   `thunderTime`/`rainTime`：>0 递减、减到 0 **翻转** raining/thundering；
//!   归零后按当前态重采样——继续 → DURATION、转停 → DELAY。
//! - **时长采样**（ServerLevel.java:188-191）：
//!   `RAIN_DELAY = uniform(12000..180000)`、`RAIN_DURATION = (12000..24000)`、
//!   `THUNDER_DELAY = (12000..180000)`、`THUNDER_DURATION = (3600..15600)`
//!   （单位 tick）。
//! - **强度游走**（ServerLevel.java:744-755 + Level.java:917-930）：
//!   每 tick `rainLevel/thunderLevel += 0.01`（下雨/雷暴）或 `−= 0.01`，
//!   clamp [0,1]。
//! - **视觉修饰**（`WeatherAttributes.java:9-63`）——分层叠加，雨层权重用
//!   `rainLevel − thunderLevel`（:47，雷暴时雨层不重复计）、雷暴层用
//!   thunderLevel，各自 `stateChangeLerp`：
//!   - `SKY_LIGHT_FACTOR`（渲染 day_factor 通道）向 **0.24**（夜底）混：
//!     雨 α=0.3125（:17）、雷 α=0.52734375（:30）；
//!   - `SKY_LIGHT_LEVEL` 向 **4.0** 混（:13/:26）→ skyDarken = 15 − 值
//!     （Level.java:736），喂刷怪亮度门与燃烧判定；
//!   - **雾色**（客户端）`AtmosphericFogEnvironment.applyWeatherDarken`
//!     （AtmosphericFogEnvironment.java:50-62）：雨 ×(1−0.5r, 1−0.5r,
//!     1−0.4r)，雷整体再 ×(1−0.5t)；
//!   - **雾距**（:20-22/:70-73）：`end − 256×rainFogMultiplier`（下限 96）、
//!     `start − 160×mult`；mult 以 `Δ×0.2` 逼近 `rainLevel`（:91-97，
//!     生物群系降水/天光门简化为恒可雨 → KNOWN-DIVERGENCE）。
//!
//! 出生排除（雨天不刷被动怪）/睡眠排除按派单标注**不做**（"出生/睡眠不排"）。
//! 雨粒子渲染为后续项（见报告）。

use crate::difficulty::Difficulty;

/// ServerLevel.java:188-191 — uniform 整数采样（tick）。
fn uniform(lo: u32, hi: u32, rng: &mut impl FnMut() -> u32) -> u32 {
    lo + rng() % (hi - lo + 1)
}

/// ServerLevel.java:188-191 的四个采样器（Tick 20/s）。
fn rain_delay(rng: &mut impl FnMut() -> u32) -> u32 {
    uniform(12_000, 180_000, rng)
}
fn rain_duration(rng: &mut impl FnMut() -> u32) -> u32 {
    uniform(12_000, 24_000, rng)
}
fn thunder_delay(rng: &mut impl FnMut() -> u32) -> u32 {
    uniform(12_000, 180_000, rng)
}
fn thunder_duration(rng: &mut impl FnMut() -> u32) -> u32 {
    uniform(3_600, 15_600, rng)
}

/// 天气运行时（等价 ServerLevel 的 WeatherData + Level 的 rain/thunderLevel）。
#[derive(Clone, Debug)]
pub struct Weather {
    clear_weather_time: u32,
    rain_time: u32,
    thunder_time: u32,
    raining: bool,
    thundering: bool,
    /// Level.java:118/120 — 0..1，每 tick ±0.01（ServerLevel.java:744-755）。
    rain_level: f32,
    thunder_level: f32,
    /// AtmosphericFogEnvironment.java:91-97 — 雾距倍率（Δ×0.2 逼近 rainLevel）。
    rain_fog_multiplier: f32,
}

impl Default for Weather {
    fn default() -> Self {
        Self::new()
    }
}

impl Weather {
    /// 晴天开局（ServerLevel 无天气存档时的默认态）。
    pub fn new() -> Self {
        Self {
            clear_weather_time: 0,
            rain_time: 1,
            thunder_time: 1,
            raining: false,
            thundering: false,
            rain_level: 0.0,
            thunder_level: 0.0,
            rain_fog_multiplier: 0.0,
        }
    }

    pub fn is_raining(&self) -> bool {
        self.raining
    }

    pub fn is_thundering(&self) -> bool {
        self.thundering
    }

    /// Level.java:924-925 — getRainLevel（本仓在 tick 边界读，无插值帧）。
    pub fn rain_level(&self) -> f32 {
        self.rain_level
    }

    pub fn thunder_level(&self) -> f32 {
        self.thunder_level
    }

    /// 测试/指令用：直接置雨（等价 setWeather + setRainLevel）。
    pub fn set_for_test(&mut self, raining: bool, thundering: bool) {
        self.raining = raining;
        self.thundering = thundering;
        self.rain_time = 1;
        self.thunder_time = 1;
    }

    /// 每 game tick 一次（ServerLevel.advanceWeatherCycle，:694-755）。
    pub fn tick(&mut self, rng: &mut impl FnMut() -> u32) {
        // ---- 状态机（:699-741）----
        if self.clear_weather_time > 0 {
            self.clear_weather_time -= 1;
            self.thunder_time = if self.thundering { 0 } else { 1 };
            self.rain_time = if self.raining { 0 } else { 1 };
            self.thundering = false;
            self.raining = false;
        } else {
            if self.thunder_time > 0 {
                self.thunder_time -= 1;
                if self.thunder_time == 0 {
                    self.thundering = !self.thundering; // :712-714 翻转
                }
            } else if self.thundering {
                self.thunder_time = thunder_duration(rng);
            } else {
                self.thunder_time = thunder_delay(rng);
            }
            if self.rain_time > 0 {
                self.rain_time -= 1;
                if self.rain_time == 0 {
                    self.raining = !self.raining; // :723-725 翻转
                }
            } else if self.raining {
                self.rain_time = rain_duration(rng);
            } else {
                self.rain_time = rain_delay(rng);
            }
        }
        // ---- 强度游走（:744-755，±0.01/tick，clamp [0,1]）----
        self.thunder_level =
            (self.thunder_level + if self.thundering { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        self.rain_level =
            (self.rain_level + if self.raining { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        // ---- 雾距倍率（AtmosphericFogEnvironment.java:91-97）----
        let target = self.rain_level;
        self.rain_fog_multiplier += (target - self.rain_fog_multiplier) * 0.2;
    }

    /// 雨层有效权重 = rainLevel − thunderLevel（WeatherAttributes.java:47）。
    fn rain_eff(&self) -> f32 {
        (self.rain_level - self.thunder_level).max(0.0)
    }

    /// 渲染 day_factor 通道（WeatherAttributes.java:17/:30 SKY_LIGHT_FACTOR
    /// 向 0.24 混：先雨 α=0.3125、后雷 α=0.52734375）。`day` 传
    /// `mcv_render::day_factor(t)`。
    pub fn sky_light_factor(&self, day: f32) -> f32 {
        const NIGHT: f32 = 0.24;
        let mut v = lerp(self.rain_eff() * 0.3125, day, NIGHT);
        v = lerp(self.thunder_level * 0.527_343_75, v, NIGHT);
        v
    }

    /// skyDarken（Level.java:736 = 15 − SKY_LIGHT_LEVEL；WeatherAttributes
    /// .java:13/:26 SKY_LIGHT_LEVEL 向 4.0 混）。`base` 传时间线的
    /// `sky_darken(t)`（0..11）。喂 MobServices（燃烧/游走亮度）与刷怪门；
    /// 注意 Monster.java:89 雷暴时刷怪测试另用 darken=10 覆盖（spawn 区
    /// 接线归主控，见报告）。
    pub fn sky_darken(&self, base: u8) -> u8 {
        let level = 15 - i32::from(base); // SKY_LIGHT_LEVEL 基线
        let mut v = lerp(self.rain_eff() * 0.3125, level as f32, 4.0);
        v = lerp(self.thunder_level * 0.527_343_75, v, 4.0);
        (15.0 - v).round().clamp(0.0, 15.0) as u8
    }

    /// 雾色 RGB 乘子（AtmosphericFogEnvironment.applyWeatherDarken，
    /// :50-62）：雨 ×(1−0.5r, 1−0.5r, 1−0.4r)，雷整体再 ×(1−0.5t)。
    pub fn fog_tint(&self) -> [f32; 3] {
        let r = self.rain_level;
        let t = self.thunder_level;
        let mut c = [1.0 - 0.5 * r, 1.0 - 0.5 * r, 1.0 - 0.4 * r];
        let k = 1.0 - 0.5 * t;
        for ch in &mut c {
            *ch *= k;
        }
        c
    }

    /// 雾距密度乘子（AtmosphericFogEnvironment.java:70-73 等价映射：
    /// 本仓雾是 `exp2(−dist×density)`，能见度∝1/density；end 收缩 k 倍 →
    /// density 放大 1/k。end' = max(96, end − 256×mult)）。
    pub fn fog_density_multiplier(&self, base_end: f32) -> f32 {
        let end = (base_end - 256.0 * self.rain_fog_multiplier).max(96.0);
        base_end / end
    }

    /// 难度联动（Mob.java:799-811 的 difficulty 输入归 pathfinding，此处
    /// 只暴露雨势给刷怪侧未来扩展，避免误用：和平仍按 Difficulty 判）。
    pub fn spawn_difficulty_gate(&self, difficulty: Difficulty) -> bool {
        !difficulty.is_peaceful()
    }
}

/// `Mth.lerp(delta, from, to)`（权重 α 指向 to）。
fn lerp(a: f32, from: f32, to: f32) -> f32 {
    from + a * (to - from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(seed: u64) -> impl FnMut() -> u32 {
        let mut s = seed | 1;
        move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as u32
        }
    }

    /// ServerLevel.java:744-755 — 降雨时 rainLevel +0.01/tick 收敛到 1，
    /// 停雨 −0.01/tick 收敛到 0。
    #[test]
    fn level_random_walk() {
        let mut w = Weather::new();
        let mut r = det(3);
        w.set_for_test(true, false);
        for _ in 0..100 {
            w.tick(&mut r);
        }
        assert!(
            (w.rain_level() - 1.0).abs() < 1e-4,
            "rain {}",
            w.rain_level()
        );
        assert!(!w.is_thundering());
        w.set_for_test(false, false);
        for _ in 0..100 {
            w.tick(&mut r);
        }
        assert!(w.rain_level().abs() < 1e-4);
    }

    /// ServerLevel.java:704-731 — 倒计时归零翻转（雷：置 1 → 下 tick 翻转）。
    #[test]
    fn timer_flip() {
        let mut w = Weather::new();
        let mut r = det(5);
        w.set_for_test(false, false);
        w.thunder_time = 1; // 直逼字段：测试里经由 set_for_test 后手动置短
        w.rain_time = 1;
        w.tick(&mut r);
        assert!(w.is_thundering() && w.is_raining(), "归零翻转");
        w.tick(&mut r);
        // 翻回停后重采样 DURATION（12000..15600 段），短时间内不会再翻。
        assert!(!w.is_thundering() || w.thunder_time > 1);
    }

    /// WeatherAttributes.java:17/:30 — day_factor 混合：满雨 1.0 →
    /// lerp(1,0.24,0.3125)=0.7625；再满雷 → ×(1−0.527)+0.24×0.527。
    #[test]
    fn sky_light_factor_blend() {
        let mut w = Weather::new();
        w.set_for_test(true, false);
        w.rain_level = 1.0;
        w.thunder_level = 0.0;
        let rainy = w.sky_light_factor(1.0);
        assert!((rainy - 0.7625).abs() < 1e-4, "rain {rainy}");
        w.thunder_level = 1.0; // rain_eff = 0
        let stormy = w.sky_light_factor(1.0);
        let want = 0.7625_f32.mul_add(1.0 - 0.527_343_75, 0.24 * 0.527_343_75);
        assert!((stormy - want).abs() < 1e-4, "storm {stormy} vs {want}");
        // 晴天原样。
        w.rain_level = 0.0;
        w.thunder_level = 0.0;
        assert_eq!(w.sky_light_factor(0.9), 0.9);
    }

    /// WeatherAttributes.java:13/:26 + Level.java:736 — skyDarken 变深。
    /// 注：Monster.java:89 的雷暴 darken=10 是刷怪测试里的**显式覆盖**
    /// （getMaxLocalRawBrightness(pos, 10)），不来自属性混合；满雷暴白天
    /// 的环境混合结果 = 15 − lerp(0.527, 15, 4) = 5.80 → 6。
    #[test]
    fn sky_darken_deepens() {
        let mut w = Weather::new();
        w.rain_level = 1.0;
        w.thunder_level = 1.0;
        // 白天 base=0：rain_eff=0，只剩雷层 α=0.527 → darken=6。
        let d = w.sky_darken(0);
        assert_eq!(d, 6, "thunder darken {d}");
        // 满雨（无雷）：α=0.3125 → darken = 15 − 11.5625 = 3.4375 → 3。
        w.thunder_level = 0.0;
        assert_eq!(w.sky_darken(0), 3);
        // 无天气透传。
        w.rain_level = 0.0;
        w.thunder_level = 0.0;
        assert_eq!(w.sky_darken(7), 7);
    }

    /// AtmosphericFogEnvironment.java:50-62 — 雾色乘子。
    #[test]
    fn fog_tint_channels() {
        let mut w = Weather::new();
        w.rain_level = 1.0;
        w.thunder_level = 0.0;
        assert_eq!(w.fog_tint(), [0.5, 0.5, 0.6]);
        w.thunder_level = 1.0;
        assert_eq!(w.fog_tint(), [0.25, 0.25, 0.3]); // 雨层权重归 0，只剩雷
    }

    /// AtmosphericFogEnvironment.java:70-73 — 雾距收缩 → 密度放大。
    #[test]
    fn fog_density_grows_with_rain() {
        let mut w = Weather::new();
        w.rain_level = 1.0;
        w.rain_fog_multiplier = 1.0;
        let m = w.fog_density_multiplier(200.0);
        // end = max(96, 200−256) = 96 → 密度 ×200/96。
        assert!((m - 200.0 / 96.0).abs() < 1e-4, "mult {m}");
        // 晴天无变化。
        w.rain_fog_multiplier = 0.0;
        assert_eq!(w.fog_density_multiplier(200.0), 1.0);
    }

    /// ServerLevel.java:188-191 — 采样区间（长周期模拟粗检）。
    #[test]
    fn durations_in_range() {
        let mut r = det(9);
        for _ in 0..50 {
            let d = rain_delay(&mut r);
            assert!((12_000..=180_000).contains(&d));
            let t = thunder_duration(&mut r);
            assert!((3_600..=15_600).contains(&t));
        }
    }
}
