//! 解码链路:仓库 12 个扁平开发音效经**事件流**(fixture 事件 → loader 抽变体
//! → 读 ogg → symphonia 解码)必须解出**非零** PCM(验证 symphonia feature 组合
//! 与真实 ogg 的兼容性)。CI 无全量原版树也能跑:fixture 只依赖已跟踪文件。
mod common;

use common::{DEV_SOUNDS, SoundFixture};
use mcv_audio::{AudioCmd, SoundLoader, decode_to_stereo, default_sounds_dir};

const ORIGIN: [f32; 3] = [0.0; 3];

#[test]
fn all_dev_sounds_decode_via_event_stream() {
    // 为 12 个扁平文件各建一个单变体事件,变体路径 = 文件基名。
    let json = format!(
        "{{{}",
        DEV_SOUNDS
            .iter()
            .map(|n| format!(r#""dev.{n}": {{"sounds": ["{n}"]}}"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    let json = format!("{json}}}");
    let fx = SoundFixture::with_json(&json);
    let mut loader = SoundLoader::new(fx.dir());
    let t0 = std::time::Instant::now();

    for name in DEV_SOUNDS {
        let cmd = loader
            .play_at(&format!("dev.{name}"), ORIGIN, ORIGIN, 1.0, t0)
            .unwrap_or_else(|| panic!("{name}: 事件流未产出命令"));
        let AudioCmd::Play { sound, gain, .. } = cmd else {
            panic!("{name}: 应为 Play 命令")
        };
        assert!((gain - 1.0).abs() < 1e-6, "{name}: 增益应原样");

        assert!(sound.frames > 128, "{name}: 帧数过少 ({})", sound.frames);
        assert!(
            (8000..=96000).contains(&sound.sample_rate),
            "{name}: 采样率异常 {}",
            sound.sample_rate
        );
        assert_eq!(
            sound.samples.len(),
            sound.frames * 2,
            "{name}: 交错立体声长度不符"
        );

        // 前 2048 帧内必须出现可闻样本(排除全零缓冲/静音占位)。
        let probe = sound
            .samples
            .iter()
            .take(2048 * 2)
            .filter(|s| s.abs() > 1e-3)
            .count();
        assert!(
            probe >= 10,
            "{name}: 前 2048 帧几乎是全零静音 ({} 个非零样本)",
            probe
        );
        // 样本幅度应在合理范围(f32 归一语义)。
        let peak = sound.samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.01 && peak <= 4.0, "{name}: 峰值异常 {peak}");
    }
}

#[test]
fn garbage_bytes_fail_cleanly() {
    // 素材损坏路径:必须返回 Err 而不是 panic(上层按缺失 no-op 处理)。
    let err = decode_to_stereo(b"this is not an ogg file at all".to_vec());
    assert!(err.is_err());
}

#[test]
fn default_dir_reports_missing_tree_gracefully() {
    // 引擎对"表在但文件缺"必须静默降级:构造假事件,文件不存在 → no-op。
    let dir = default_sounds_dir();
    let has_table = dir.join("sounds.json").is_file();
    if !has_table {
        println!("skip: 全量树未 fetch,无 sounds.json 可验");
        return;
    }
    let mut loader = SoundLoader::new(dir);
    assert!(
        loader
            .play_ui("no.such.event", 1.0, std::time::Instant::now())
            .is_none(),
        "不存在的事件必须 no-op"
    );
}
