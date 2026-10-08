//! 节流与循环幂等:同一 id 的密集触发不得叠加声部(挖掘/脚步防爆音),
//! 循环声部按 id 幂等(判定在音频线程侧的 `Mixer::apply`);同时验证混音
//! 输出确实非零、缺失素材 no-op 不 panic。
//!
//! 本文件**不创建音频后端设备**(无头环境可跑):按真实数据流驱动
//! [`SoundLoader`] → [`AudioCmd`] → [`Mixer`]。
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mcv_audio::{
    default_sounds_dir, AudioCmd, Mixer, SoundId, SoundLoader, DEFAULT_ATTENUATION_RADIUS,
    SAME_ID_MIN_INTERVAL,
};

const ORIGIN: [f32; 3] = [0.0; 3];

fn mixer() -> Mixer {
    let (_producer, consumer) = mcv_sync::Spsc::<AudioCmd>::new(64).split();
    Mixer::new(44100, consumer, Arc::new(AtomicU32::new(1.0f32.to_bits())))
}

fn rig() -> (SoundLoader, Mixer) {
    (SoundLoader::new(default_sounds_dir()), mixer())
}

#[test]
fn same_id_one_shots_are_throttled() {
    let (mut loader, _m) = rig();
    let t0 = Instant::now();

    assert!(
        loader
            .play_at(SoundId::DigStone, ORIGIN, ORIGIN, 1.0, t0)
            .is_some(),
        "首次触发必须成功"
    );
    // 节流窗口内的重复触发被丢弃。
    for i in 1..5 {
        let t = t0 + Duration::from_millis(i * 10);
        assert!(
            loader
                .play_at(SoundId::DigStone, ORIGIN, ORIGIN, 1.0, t)
                .is_none(),
            "窗口内第 {i} 次触发应被节流"
        );
    }
    // 窗口结束后恢复。
    let t_after = t0 + SAME_ID_MIN_INTERVAL + Duration::from_millis(10);
    assert!(loader
        .play_at(SoundId::DigStone, ORIGIN, ORIGIN, 1.0, t_after)
        .is_some());
    // 不同 id 互不影响。
    assert!(loader
        .play_at(
            SoundId::StepGrass,
            ORIGIN,
            ORIGIN,
            1.0,
            t0 + Duration::from_millis(20)
        )
        .is_some());
}

#[test]
fn far_away_sounds_are_dropped_and_radius_is_inclusive() {
    let (mut loader, _m) = rig();
    let t0 = Instant::now();
    // 超出 16 格:衰减为 0,不起播(且不占用节流窗口:衰减检查在节流之前)。
    let far = [DEFAULT_ATTENUATION_RADIUS + 1.0, 0.0, 0.0];
    assert!(loader
        .play_at(SoundId::StepStone, far, ORIGIN, 1.0, t0)
        .is_none());
    assert!(loader
        .play_at(
            SoundId::StepStone,
            [16.0, 0.0, 0.0],
            ORIGIN,
            1.0,
            t0 + Duration::from_millis(200)
        )
        .is_none());
    // 恰好在半径边缘之内 → 起播。
    assert!(loader
        .play_at(
            SoundId::StepStone,
            [15.0, 0.0, 0.0],
            ORIGIN,
            1.0,
            t0 + Duration::from_millis(400)
        )
        .is_some());
    // 非法增益不起播。
    assert!(loader
        .play_at(
            SoundId::StepWood,
            ORIGIN,
            ORIGIN,
            0.0,
            t0 + Duration::from_secs(10)
        )
        .is_none());
    assert!(loader
        .play_at(
            SoundId::StepWood,
            ORIGIN,
            ORIGIN,
            f32::NAN,
            t0 + Duration::from_secs(10)
        )
        .is_none());
}

#[test]
fn loops_are_idempotent_and_stoppable() {
    let (mut loader, mut m) = rig();
    let t0 = Instant::now();

    let cmd = loader
        .loop_start(SoundId::DigStone, 1.0, t0)
        .expect("素材应存在");
    assert!(m.apply(cmd));
    // 持续"按住挖掘"每帧调用:loader 每帧都产命令(不节流循环),
    // 幂等由音频线程把关——不得叠加第二个声部。
    for i in 1..10 {
        let cmd = loader
            .loop_start(SoundId::DigStone, 1.0, t0 + Duration::from_millis(i * 16))
            .expect("循环不受一次性节流窗口限制");
        assert!(!m.apply(cmd), "循环重复触发必须幂等");
    }
    assert_eq!(m.active_voices(), 1);
    m.apply(AudioCmd::StopLoop {
        id: SoundId::DigStone,
    });
    assert_eq!(m.active_voices(), 0);
    // 停止后(即使立刻)可以重新起循环。
    let cmd = loader
        .loop_start(SoundId::DigStone, 1.0, t0 + Duration::from_millis(50))
        .unwrap();
    assert!(m.apply(cmd));
}

#[test]
fn mixing_produces_nonzero_output_and_end_of_sound() {
    let (mut loader, mut m) = rig();
    let t0 = Instant::now();
    let cmd = loader
        .play_at(SoundId::BowShot, ORIGIN, ORIGIN, 1.0, t0)
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
fn missing_assets_are_silent_noop() {
    // 指向不存在素材目录:所有调用 None/no-op,不 panic,混音输出全零。
    let mut loader = SoundLoader::new("/nonexistent/mcv-audio-test-dir");
    let mut m = mixer();
    let t0 = Instant::now();
    assert!(loader
        .play_at(SoundId::DigStone, ORIGIN, ORIGIN, 1.0, t0)
        .is_none());
    assert!(loader.play_ui(SoundId::XpOrb, 1.0, t0).is_none());
    assert!(loader.loop_start(SoundId::DigStone, 1.0, t0).is_none());
    m.apply(AudioCmd::StopLoop {
        id: SoundId::DigStone,
    });
    let mut out = vec![0f32; 1024 * 2];
    m.mix(&mut out);
    assert!(out.iter().all(|s| *s == 0.0));
    // 缺失记录后二次调用仍安静(无重复解码重试)。
    assert!(loader
        .play_at(
            SoundId::DigStone,
            ORIGIN,
            ORIGIN,
            1.0,
            t0 + Duration::from_secs(5)
        )
        .is_none());
}

#[test]
fn ui_playback_bypasses_attenuation() {
    let (mut loader, mut m) = rig();
    let t0 = Instant::now();
    // UI 音与"距离"无关:同样坐标下 play_ui 恒起播。
    let cmd = loader.play_ui(SoundId::XpOrb, 1.0, t0).expect("素材应存在");
    m.apply(cmd);
    let mut out = vec![0f32; 256 * 2];
    m.mix(&mut out);
    assert!(out.iter().any(|s| s.abs() > 1e-4));
}
