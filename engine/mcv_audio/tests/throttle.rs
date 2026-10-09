//! 节流与循环幂等:同一变体路径的密集触发不得叠加声部(挖掘/脚步防爆音,
//! 节流键 = 变体路径),循环声部按事件名幂等(判定在音频线程侧的
//! `Mixer::apply`);同时验证混音输出确实非零、缺失素材 no-op 不 panic。
//!
//! 本文件**不创建音频后端设备**(无头环境可跑):按真实数据流驱动
//! [`SoundLoader`] → [`AudioCmd`] → [`Mixer`]。fixture 见 `common`(事件全部
//! 单变体,抽取与种子无关,节流行为确定;素材来自 fetch 的真实树)。
mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};

use common::SoundFixture;
use mcv_audio::{AudioCmd, DEFAULT_ATTENUATION_RADIUS, Mixer, SAME_ID_MIN_INTERVAL, SoundLoader};

const ORIGIN: [f32; 3] = [0.0; 3];

fn mixer() -> Mixer {
    let (_producer, consumer) = mcv_sync::Spsc::<AudioCmd>::new(64).split();
    Mixer::new(44100, consumer, Arc::new(AtomicU32::new(1.0f32.to_bits())))
}

fn rig() -> (SoundFixture, SoundLoader, Mixer) {
    let fx = SoundFixture::new();
    // 先建 loader 再组元组:元组元素从左到右求值,fx 会先被移走。
    let loader = SoundLoader::new(fx.dir());
    (fx, loader, mixer())
}

#[test]
fn same_variant_one_shots_are_throttled() {
    let (_fx, mut loader, _m) = rig();
    let t0 = Instant::now();

    assert!(
        loader
            .play_at("dev.oneshot", ORIGIN, ORIGIN, 1.0, t0)
            .is_some(),
        "首次触发必须成功"
    );
    // 节流窗口内的重复触发被丢弃。
    for i in 1..5 {
        let t = t0 + Duration::from_millis(i * 10);
        assert!(
            loader
                .play_at("dev.oneshot", ORIGIN, ORIGIN, 1.0, t)
                .is_none(),
            "窗口内第 {i} 次触发应被节流"
        );
    }
    // 窗口结束后恢复。
    let t_after = t0 + SAME_ID_MIN_INTERVAL + Duration::from_millis(10);
    assert!(
        loader
            .play_at("dev.oneshot", ORIGIN, ORIGIN, 1.0, t_after)
            .is_some()
    );
    // 不同变体路径互不影响(dev.other → step/stone1)。
    assert!(
        loader
            .play_at(
                "dev.other",
                ORIGIN,
                ORIGIN,
                1.0,
                t0 + Duration::from_millis(20)
            )
            .is_some()
    );
}

#[test]
fn throttle_key_is_variant_path_not_event_name() {
    let (_fx, mut loader, _m) = rig();
    let t0 = Instant::now();
    // dev.oneshot 与 dev.oneshot2 是两个事件、同一个变体路径(dig/stone1):
    // 节流按变体路径判,后者必须被前者压进窗口。
    assert!(
        loader
            .play_at("dev.oneshot", ORIGIN, ORIGIN, 1.0, t0)
            .is_some()
    );
    assert!(
        loader
            .play_at(
                "dev.oneshot2",
                ORIGIN,
                ORIGIN,
                1.0,
                t0 + Duration::from_millis(10)
            )
            .is_none(),
        "同变体路径的另一个事件也应被节流"
    );
    assert!(
        loader
            .play_at(
                "dev.other",
                ORIGIN,
                ORIGIN,
                1.0,
                t0 + Duration::from_millis(10)
            )
            .is_some(),
        "不同变体路径不被节流"
    );
}

#[test]
fn far_away_sounds_are_dropped_and_radius_is_inclusive() {
    let (_fx, mut loader, _m) = rig();
    let t0 = Instant::now();
    // 超出 16 格:衰减为 0,不起播(且不占用节流窗口:衰减检查在节流之前)。
    let far = [DEFAULT_ATTENUATION_RADIUS + 1.0, 0.0, 0.0];
    assert!(loader.play_at("dev.other", far, ORIGIN, 1.0, t0).is_none());
    assert!(
        loader
            .play_at(
                "dev.other",
                [16.0, 0.0, 0.0],
                ORIGIN,
                1.0,
                t0 + Duration::from_millis(200)
            )
            .is_none()
    );
    // 恰好在半径边缘之内 → 起播。
    assert!(
        loader
            .play_at(
                "dev.other",
                [15.0, 0.0, 0.0],
                ORIGIN,
                1.0,
                t0 + Duration::from_millis(400)
            )
            .is_some()
    );
    // 非法增益不起播。
    assert!(
        loader
            .play_at(
                "dev.other",
                ORIGIN,
                ORIGIN,
                0.0,
                t0 + Duration::from_secs(10)
            )
            .is_none()
    );
    assert!(
        loader
            .play_at(
                "dev.other",
                ORIGIN,
                ORIGIN,
                f32::NAN,
                t0 + Duration::from_secs(10)
            )
            .is_none()
    );
}

#[test]
fn loops_are_idempotent_by_event_and_stoppable() {
    let (_fx, mut loader, mut m) = rig();
    let t0 = Instant::now();

    let cmd = loader
        .loop_start("dev.oneshot", 1.0, t0)
        .expect("素材应存在");
    let AudioCmd::Play { loop_event, .. } = &cmd else {
        panic!("loop_start 应产出 Play 命令");
    };
    assert_eq!(loop_event.as_deref(), Some("dev.oneshot"));
    assert!(m.apply(cmd));
    // 持续"按住挖掘"每帧调用:loader 每帧都产命令(不节流循环),
    // 幂等由音频线程按事件名把关——不得叠加第二个声部。
    for i in 1..10 {
        let cmd = loader
            .loop_start("dev.oneshot", 1.0, t0 + Duration::from_millis(i * 16))
            .expect("循环不受一次性节流窗口限制");
        assert!(!m.apply(cmd), "循环重复触发必须幂等");
    }
    // 不同事件的循环互不幂等,可并存。
    let cmd = loader
        .loop_start("dev.other", 1.0, t0 + Duration::from_millis(20))
        .unwrap();
    assert!(m.apply(cmd));
    assert_eq!(m.active_voices(), 2);
    // StopLoop 按事件名精确匹配。
    m.apply(AudioCmd::StopLoop {
        event: "dev.oneshot".into(),
    });
    assert_eq!(m.active_voices(), 1);
    m.apply(AudioCmd::StopLoop {
        event: "dev.other".into(),
    });
    assert_eq!(m.active_voices(), 0);
    // 停止后(即使立刻)可以重新起循环。
    let cmd = loader
        .loop_start("dev.oneshot", 1.0, t0 + Duration::from_millis(50))
        .unwrap();
    assert!(m.apply(cmd));
}

#[test]
fn mixing_produces_nonzero_output_and_end_of_sound() {
    let (_fx, mut loader, mut m) = rig();
    let t0 = Instant::now();
    let cmd = loader
        .play_at("dev.bow", ORIGIN, ORIGIN, 1.0, t0)
        .expect("素材应存在");
    m.apply(cmd);

    // 第一帧块应有声音(22050Hz 素材重采样到 44100)。
    let mut out = vec![0f32; 1024 * 2];
    m.mix(&mut out);
    assert!(out.iter().any(|s| s.abs() > 1e-4), "混音输出全零");
    assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)), "输出未限幅");

    // 素材很短(< 1s)。混完整个时长后声部应自动熄灭。
    let mut voiced = true;
    for i in 0..60 {
        m.mix(&mut out);
        voiced |= m.active_voices() > 0;
        if m.active_voices() == 0 {
            assert!(i > 0, "声部不应在第一次 mix 后就消失(素材不足 1024 帧?)");
            break;
        }
        assert!(i < 59, "短音效 60 个块(约 14s)后仍未结束?");
    }
    assert!(voiced);
}

#[test]
fn missing_table_and_missing_files_are_silent_noop() {
    // 情形一:表缺失(目录不存在)→ 事件查不到,no-op,不 panic。
    let mut loader = SoundLoader::new("/nonexistent/mcv-audio-test-dir");
    let mut m = mixer();
    let t0 = Instant::now();
    assert!(
        loader
            .play_at("dev.oneshot", ORIGIN, ORIGIN, 1.0, t0)
            .is_none()
    );
    assert!(loader.play_ui("dev.anything", 1.0, t0).is_none());
    assert!(loader.loop_start("dev.oneshot", 1.0, t0).is_none());
    m.apply(AudioCmd::StopLoop {
        event: "dev.oneshot".into(),
    });
    let mut out = vec![0f32; 1024 * 2];
    m.mix(&mut out);
    assert!(out.iter().all(|s| *s == 0.0));

    // 情形二:表在、变体文件缺失 → 记入 missing,重复触发不再重试。
    let fx = SoundFixture::with_json(
        r#"{"dev.gone": {"sounds": ["no_such_file"]}, "dev.ok": {"sounds": ["dig/stone1"]}}"#,
    );
    let mut loader = SoundLoader::new(fx.dir());
    assert!(
        loader
            .play_at("dev.gone", ORIGIN, ORIGIN, 1.0, t0)
            .is_none()
    );
    assert!(
        loader
            .play_at("dev.gone", ORIGIN, ORIGIN, 1.0, t0 + Duration::from_secs(5))
            .is_none(),
        "缺失记录后二次调用仍安静(无重复解码重试)"
    );
    assert!(
        loader.play_at("dev.ok", ORIGIN, ORIGIN, 1.0, t0).is_some(),
        "同表内的合法变体不受影响"
    );
}

#[test]
fn ui_playback_bypasses_attenuation() {
    let (_fx, mut loader, mut m) = rig();
    let t0 = Instant::now();
    // UI 音与"距离"无关:同样坐标下 play_ui 恒起播。
    let cmd = loader.play_ui("dev.orb", 1.0, t0).expect("素材应存在");
    m.apply(cmd);
    let mut out = vec![0f32; 256 * 2];
    m.mix(&mut out);
    assert!(out.iter().any(|s| s.abs() > 1e-4));
    // 距离衰减半径之外 play_at 会丢弃,play_ui 仍可播。
    let far = [DEFAULT_ATTENUATION_RADIUS + 5.0, 0.0, 0.0];
    assert!(
        loader
            .play_at(
                "dev.other",
                far,
                ORIGIN,
                1.0,
                t0 + Duration::from_millis(200)
            )
            .is_none()
    );
    let cmd = loader
        .play_ui("dev.other", 1.0, t0 + Duration::from_millis(300))
        .expect("UI 音不受距离影响");
    m.apply(cmd);
    m.mix(&mut out);
    assert!(out.iter().any(|s| s.abs() > 1e-4));
}
