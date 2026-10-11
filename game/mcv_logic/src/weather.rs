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
    /// 无档默认（26.1 `WeatherData` 无参构造，WeatherData.java:28-29，
    /// 字段 :22-26 全 0/假）——**不是** rainTime=1：advanceWeatherCycle
    /// 首 tick 对 0 计时器走 else 支重采样 RAIN_DELAY/THUNDER_DELAY
    /// （ServerLevel.java:723-730）→ 新世界 ≥12000 tick（10 分钟）长晴。
    /// 旧实现置 1，第二 tick 归零翻转 → 每个档进去默认就下雨（真机
    /// 报障①根因）。
    pub fn new() -> Self {
        Self {
            clear_weather_time: 0,
            rain_time: 0,
            thunder_time: 0,
            raining: false,
            thundering: false,
            rain_level: 0.0,
            thunder_level: 0.0,
            rain_fog_multiplier: 0.0,
        }
    }

    /// 档内恢复（ServerLevel.java:273 构造期 `prepareWeather`，:687-694：
    /// raining → rainLevel=1.0、thundering → thunderLevel=1.0，免
    /// 0.01/tick 慢爬）。五字段 = WeatherData.java:22-26；无天气块的
    /// 旧档由存档层给 `WeatherData` 无档默认（全 0/假，见 `new`）。
    pub fn restore(
        clear_weather_time: i32,
        rain_time: i32,
        thunder_time: i32,
        raining: bool,
        thundering: bool,
    ) -> Self {
        let mut w = Self::new();
        w.clear_weather_time = clear_weather_time.max(0) as u32;
        w.rain_time = rain_time.max(0) as u32;
        w.thunder_time = thunder_time.max(0) as u32;
        w.raining = raining;
        w.thundering = thundering;
        if raining {
            w.rain_level = 1.0; // prepareWeather ServerLevel.java:689-690
        }
        if thundering {
            w.thunder_level = 1.0; // :690-692
        }
        w
    }

    /// 落盘快照（WeatherData.java:39-82 getter 全集，顺序对齐 codec
    /// :11-15）。advanceWeatherCycle 每 tick 把计时器写回 WeatherData
    /// （ServerLevel.java:734-738），故任意时刻快照即合法存档态。
    pub fn weather_state(&self) -> (i32, i32, i32, bool, bool) {
        (
            self.clear_weather_time as i32,
            self.rain_time as i32,
            self.thunder_time as i32,
            self.raining,
            self.thundering,
        )
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

    /// 测试/指令用：直接置雨（等价 setWeather + setRainLevel）；倒计时给
    /// 足一个 DURATION 上限，避免下一 tick 立即翻转。
    pub fn set_for_test(&mut self, raining: bool, thundering: bool) {
        self.raining = raining;
        self.thundering = thundering;
        self.rain_time = 24_000;
        self.thunder_time = 15_600;
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

    /// 环境雾层端点 (start, end)（AtmosphericFogEnvironment.java:68-72
    /// 原式，overworld 属性基线 FOG_START/END_DISTANCE = 0/1024，
    /// EnvironmentAttributes.java:18-24）：
    /// `start = 0 − 160×rainFogMult`、
    /// `end = max(min(96, base), base − 256×rainFogMult)`
    /// （base=1024 → min 项恒 96，96 下限只在 base<96 的维度咬合）。
    /// 旧「exp 密度乘子」系本仓 exp 雾的等价映射，随双层线性雾废弃。
    pub fn fog_env_endpoints(&self) -> (f32, f32) {
        let base = 1024.0_f32;
        let start = -160.0 * self.rain_fog_multiplier;
        let end = (base - 256.0 * self.rain_fog_multiplier).max(96.0_f32.min(base));
        (start, end)
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
        w.thunder_level = 1.0; // rain_eff = rain−thunder = 0：只剩雷层混合
        let stormy = w.sky_light_factor(1.0);
        // lerp(0.52734375, 1.0, 0.24) = 1×0.47265625 + 0.24×0.52734375。
        let want = 1.0_f32.mul_add(1.0 - 0.527_343_75, 0.24 * 0.527_343_75);
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

    /// AtmosphericFogEnvironment.java:70-72 — 雨雾端点收缩。
    #[test]
    fn fog_env_shrinks_with_rain() {
        let mut w = Weather::new();
        w.rain_level = 1.0;
        w.rain_fog_multiplier = 1.0;
        let (start, end) = w.fog_env_endpoints();
        // start = −160；end = max(96, 1024−256) = 768（overworld base=1024，
        // 96 下限不咬合）。
        assert!((start + 160.0).abs() < 1e-4, "start {start}");
        assert!((end - 768.0).abs() < 1e-4, "end {end}");
        // 半雨：mult=0.5 → start −80、end 896。
        w.rain_fog_multiplier = 0.5;
        let (start, end) = w.fog_env_endpoints();
        assert!((start + 80.0).abs() < 1e-4 && (end - 896.0).abs() < 1e-4);
        // 晴天 = 属性默认 0/1024。
        w.rain_fog_multiplier = 0.0;
        assert_eq!(w.fog_env_endpoints(), (0.0, 1024.0));
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

    /// 新世界长晴窗（W1 红测试）：26.1 无档默认 = WeatherData 无参构造
    /// 全 0/假（WeatherData.java:22-26 字段、:28-29 构造体），首 tick
    /// advanceWeatherCycle 对 0 计时器走 else 支重采样 RAIN_DELAY/
    /// THUNDER_DELAY = 12000..180000（ServerLevel.java:713-720/:723-730，
    /// 采样器 :186-189）→ 新档进世界 ≥12000 tick（10 分钟）内必晴。
    /// 回归根因：旧实现 `new()` 置 rain_time/thunder_time=1，第二 tick
    /// 归零翻转 → 每个档进去默认就下雨（真机报障 1）。
    #[test]
    fn new_world_stays_clear() {
        let mut w = Weather::new();
        let mut r = det(11);
        w.tick(&mut r);
        assert!(
            (12_000..=180_000).contains(&w.rain_time),
            "首 tick 后 rain_time 应重采样 RAIN_DELAY，得 {}",
            w.rain_time
        );
        assert!(
            (12_000..=180_000).contains(&w.thunder_time),
            "首 tick 后 thunder_time 应重采样 THUNDER_DELAY，得 {}",
            w.thunder_time
        );
        for t in 0..200 {
            w.tick(&mut r);
            assert!(!w.is_raining(), "tick {t}: 新世界开局不应下雨");
            assert!(!w.is_thundering(), "tick {t}: 新世界开局不应雷暴");
        }
    }

    /// 档内恢复 = prepareWeather 语义（W2）：ServerLevel.java:273 构造期
    /// `prepareWeather`（:687-694）——raining → rainLevel=1.0、thundering
    /// → thunderLevel=1.0，雨势不经历 0.01/tick 慢爬；state() 快照与
    /// restore() 对偶（负计时器钳 0，u32 域）。
    #[test]
    fn restore_matches_prepare_weather() {
        let w = Weather::restore(77, 23_456, 9_876, true, true);
        assert!(w.is_raining() && w.is_thundering());
        assert_eq!((w.rain_level(), w.thunder_level()), (1.0, 1.0));
        assert_eq!(
            w.weather_state(),
            (77, 23_456, 9_876, true, true),
            "state() 与 restore() 五字段对偶"
        );
        // 晴档：levels 不抬。
        let clear = Weather::restore(0, 5, 5, false, false);
        assert_eq!((clear.rain_level(), clear.thunder_level()), (0.0, 0.0));
        assert_eq!(clear.weather_state(), (0, 5, 5, false, false));
        // 负值钳 0（坏值防御；原版 int 域但语义上负计时器 = 立即重采样）。
        let neg = Weather::restore(-1, -5, 3, false, false);
        assert_eq!(neg.weather_state(), (0, 0, 3, false, false));
        // 恢复后下一 tick：raining 且计时器 >0 → 只递减不翻转。
        let mut w = Weather::restore(0, 2, 0, true, false);
        let mut r = det(13);
        w.tick(&mut r);
        assert!(w.is_raining(), "倒计时在档时恢复态不被首 tick 翻转");
    }
}
