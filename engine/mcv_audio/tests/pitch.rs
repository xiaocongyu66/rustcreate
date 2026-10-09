//! 变体音高语义:variant.pitch 乘进重采样步进——同一素材 pitch=2.0 时播放
//! 速率翻倍(时长减半、波形内容不同)。直接构造 [`AudioCmd::Play`] 驱动
//! [`Mixer`],不经事件表,聚焦混音端语义。
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use mcv_audio::{AudioCmd, Mixer, decode_to_stereo, default_sounds_dir};

fn mixer() -> Mixer {
    let (_producer, consumer) = mcv_sync::Spsc::<AudioCmd>::new(16).split();
    Mixer::new(44100, consumer, Arc::new(AtomicU32::new(1.0f32.to_bits())))
}

fn play_cmd(sound: &Arc<mcv_audio::SoundData>, pitch: f32) -> AudioCmd {
    AudioCmd::Play {
        key: "dig_stone1".into(),
        sound: Arc::clone(sound),
        gain: 1.0,
        pitch,
        looping: false,
        loop_event: None,
    }
}

/// 混到声部自然熄灭,返回消耗的输出块数与"最后一个非零样本"的输出帧位置。
fn drain(m: &mut Mixer, block: usize) -> (usize, usize) {
    let mut out = vec![0f32; block * 2];
    let mut blocks = 0usize;
    let mut tail = 0usize;
    while m.active_voices() > 0 {
        m.mix(&mut out);
        blocks += 1;
        if let Some(i) = out.iter().rposition(|s| s.abs() > 1e-4) {
            tail = (blocks - 1) * block + i / 2;
        }
    }
    (blocks, tail)
}

#[test]
fn pitch_doubles_playback_rate() {
    let bytes = std::fs::read(default_sounds_dir().join("dig_stone1.ogg"))
        .expect("仓库扁平音效缺失(dig_stone1.ogg)");
    let sound = Arc::new(decode_to_stereo(bytes).expect("解码失败"));
    assert!(
        sound.frames > 4096,
        "测试假设素材长于 4 块,实际 {}",
        sound.frames
    );

    let mut m1 = mixer();
    assert!(m1.apply(play_cmd(&sound, 1.0)));
    let mut m2 = mixer();
    assert!(m2.apply(play_cmd(&sound, 2.0)));

    // 单块内容:pitch 必须改变混音内容(播放速率变化的最短证明)。
    let mut a = vec![0f32; 1024 * 2];
    let mut b = vec![0f32; 1024 * 2];
    m1.mix(&mut a);
    m2.mix(&mut b);
    assert!(a.iter().any(|s| s.abs() > 1e-4), "pitch=1 输出全零");
    assert!(b.iter().any(|s| s.abs() > 1e-4), "pitch=2 输出全零");
    assert_ne!(a, b, "pitch 必须改变混音内容");

    // 时长:44100→44100 无重采样,pitch=1 约 N 块,pitch=2 约一半,严格更少。
    let (blocks1, tail1) = drain(&mut m1, 1024);
    let (blocks2, tail2) = drain(&mut m2, 1024);
    assert!(
        blocks2 < blocks1,
        "pitch=2 块数应更少: {blocks2} vs {blocks1}"
    );
    assert!(
        (blocks1 as f64 / blocks2 as f64 - 2.0).abs() < 0.5,
        "pitch=2 时长应约为一半: {blocks1} vs {blocks2}"
    );
    assert!(tail2 < tail1, "pitch=2 发声尾部应提前: {tail2} vs {tail1}");
    assert!(tail1 > 0);
}

#[test]
fn invalid_pitch_falls_back_to_normal_rate() {
    let bytes = std::fs::read(default_sounds_dir().join("dig_stone1.ogg")).unwrap();
    let sound = Arc::new(decode_to_stereo(bytes).unwrap());

    // NaN / 非正 pitch:混音端钳回 1.0,不 panic、行为同原速。
    for bad in [f32::NAN, 0.0, -1.0, f32::INFINITY] {
        let mut m_bad = mixer();
        let mut m_ref = mixer();
        assert!(m_bad.apply(play_cmd(&sound, bad)));
        assert!(m_ref.apply(play_cmd(&sound, 1.0)));
        let mut a = vec![0f32; 1024 * 2];
        let mut b = vec![0f32; 1024 * 2];
        m_bad.mix(&mut a);
        m_ref.mix(&mut b);
        assert_eq!(a, b, "非法 pitch {bad} 应按 1.0 处理");
    }
}
