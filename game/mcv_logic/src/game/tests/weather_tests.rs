//! 天气存档往返 + 旧档回退专项测试（game.rs tests 子模块，共用
//! headless_rt 无头装配）。
//!
//! 原版语义（26.1 实读）：
//! - 天气是 SavedData `WeatherData`（net/minecraft/world/level/saveddata/
//!   WeatherData.java），字段 clearWeatherTime/rainTime/thunderTime/raining/
//!   thundering（:22-26），**无档默认全 0/假**（:28-29 无参构造）→
//!   advanceWeatherCycle 首 tick 对 0 计时器重采样 RAIN_DELAY/THUNDER_DELAY
//!   （ServerLevel.java:713-720/:723-730，采样器 :186-189）→ 新世界长晴。
//! - 档内恢复走 ServerLevel.java:273 构造期 prepareWeather（:687-694）：
//!   raining → rainLevel=1.0、thundering → thunderLevel=1.0。

use super::{ChunkPos, GameMode, GameRuntime, Vec3, headless_rt, lit_chunk, spawn_rng};

/// 天气状态随 level.meta 往返（W1 红测试）：退出重进后 raining/thundering
/// 必须从存档恢复（等价 WeatherData SavedData 落盘/读回）。旧实现天气
/// 完全无持久化（mcv_save grep rain 零命中）→ 重进必被 Weather::new()
/// 覆盖，两 tick 后转雨。
#[test]
fn save_meta_roundtrips_weather() {
    let mut rt = headless_rt("wx-save");
    rt.weather.set_for_test(true, true);
    rt.save_meta();
    // 模拟重进：全新 runtime 指向同一存档目录。
    let mut rt2 = GameRuntime::new_headless(
        rt.seed ^ 0x7777_7777_7777_7777,
        rt.save_dir.clone(),
        GameMode::Survival,
    );
    rt2.load_meta();
    assert!(rt2.weather.is_raining(), "raining 未随档恢复");
    assert!(rt2.weather.is_thundering(), "thundering 未随档恢复");
    // 下一 tick 不应立即翻转（存档的倒计时在档）：set_for_test 给的
    // rain_time=24000/thunder_time=15600 应原样回来。
    let mut r = spawn_rng();
    rt2.weather.tick(&mut r);
    assert!(
        rt2.weather.is_raining(),
        "存档倒计时未恢复 → 下 tick 即翻转"
    );
    assert!(
        rt2.weather.is_thundering(),
        "存档倒计时未恢复 → 下 tick 即翻转"
    );
}

/// 旧档（v5 及更早，无天气块）回退原版默认（W1 红测试）：WeatherData
/// 无档默认全 0/假（WeatherData.java:28-29）→ 首 tick 重采样 RAIN_DELAY
/// 长晴（ServerLevel.java:723-730）——**回退结果必须是晴，不是开局下雨**。
#[test]
fn old_meta_without_weather_stays_clear() {
    let mut rt = headless_rt("wx-old");
    // 手拼一份 v5 level.meta（布局对照 mcv_save::LevelMeta::encode v5：
    // MAGIC+ver+seed+day_time+name+player 块+生存 21B+mode 居末）。
    let mut out = Vec::new();
    out.extend_from_slice(b"MCV1");
    out.extend_from_slice(&5u16.to_le_bytes());
    out.extend_from_slice(&rt.seed.to_le_bytes());
    out.extend_from_slice(&1_000u64.to_le_bytes());
    let name = b"world";
    out.push(name.len() as u8);
    out.extend_from_slice(name);
    out.push(1); // player flag
    for v in [8.5f32, 71.0, 8.5, 90.0, 0.0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.push(0); // flying
    out.push(0); // sel_slot
    out.push(0); // hotbar 空
    out.push(0); // main 空
    // v5 生存数值：health/hunger/saturation/exhaustion f32 + air i32 + difficulty u8。
    for v in [20.0f32, 20.0, 5.0, 0.0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&300i32.to_le_bytes());
    out.push(2); // difficulty 普通
    out.push(0); // mode 居末
    std::fs::create_dir_all(&rt.save_dir).unwrap();
    std::fs::write(rt.save_dir.join("level.meta"), &out).unwrap();

    rt.load_meta();
    let mut r = spawn_rng();
    for t in 0..200 {
        rt.weather.tick(&mut r);
        assert!(
            !rt.weather.is_raining() && !rt.weather.is_thundering(),
            "tick {t}: 旧档无天气块应回退原版长晴默认，不得开局下雨"
        );
    }
}

// ---- W3：空中雨层（幕帘带随相机，WeatherEffectRenderer.extractRenderState
// :78-110）----

/// 三态断言装配：3×3 实心区块（floor_top=69 → heightmap 首空 70）+ 满雨
/// （set_for_test 只置状态位；rainLevel 以 0.01/tick 爬满，生成数公式
/// :232 用 rainLevel²）。
fn rainy_rt(tag: &str, feet_y: f32) -> GameRuntime {
    let mut rt = headless_rt(tag);
    for dx in -1i32..=1 {
        for dz in -1i32..=1 {
            rt.chunks
                .entry(ChunkPos::new(dx, dz))
                .or_insert_with(|| lit_chunk(dx, dz, 69));
        }
    }
    rt.player.pos = Vec3::new(8.5, feet_y, 8.5);
    rt.weather.set_for_test(true, false);
    let mut r = spawn_rng();
    for _ in 0..100 {
        rt.weather.tick(&mut r);
    }
    assert!(
        (rt.weather.rain_level() - 1.0).abs() < 1e-4,
        "前置：rainLevel 应爬满，得 {}",
        rt.weather.rain_level()
    );
    rt
}

/// 真机报障②回归：相机在高空气位（地表 70 < camY−10）→ 幕帘带
/// [camY−10, camY+10] 内仍出雨特效（原版幕帘 y0=max(camY−10, terrain)、
/// y1=max(camY+10, terrain)，飞行照样下；extractRenderState :91-92）。
#[test]
fn rain_effect_follows_camera_at_altitude() {
    let mut rt = rainy_rt("wx-air", 200.0);
    rt.spawn_rain_particles();
    assert!(
        !rt.particles.is_empty(),
        "高空气位必须有雨特效（幕帘带随相机）"
    );
}

/// 地表回归：脚落地表（camY≈71，surface 70 在 ±10 带内）→ 原版地表
/// 雨滴路径（tickRainParticles :238-240 门内）照常出。
#[test]
fn rain_on_ground_still_spawns() {
    let mut rt = rainy_rt("wx-gnd", 70.0);
    rt.spawn_rain_particles();
    assert!(!rt.particles.is_empty(), "地表雨滴路径不得回归");
}

/// 地下/悬顶负例：地表高出相机 +10 以上 → 无雨（:239-240 上界门保持；
/// 幕帘带分支只对 terrain < camY−10 生效，不得误放行悬顶上方）。
#[test]
fn rain_suppressed_under_cover() {
    let mut rt = rainy_rt("wx-cave", 40.0);
    rt.spawn_rain_particles();
    assert!(rt.particles.is_empty(), "地表高于相机 +10 不得出雨");
}
