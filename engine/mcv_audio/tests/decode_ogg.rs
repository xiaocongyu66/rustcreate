//! 解码链路:每个素材 ogg 必须能解出**非零** PCM(验证 symphonia feature 组合
//! 与真实 Mojang ogg 的兼容性,即"首包非零 PCM")。
use mcv_audio::{SoundId, decode_to_stereo, default_sounds_dir};

#[test]
fn all_oggs_decode_to_nonzero_pcm() {
    let dir = default_sounds_dir();
    for id in SoundId::ALL {
        let path = dir.join(id.file_name());
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("读取 {:?} 失败: {e}", path));
        let sound = decode_to_stereo(bytes).unwrap_or_else(|e| panic!("解码 {:?} 失败: {e}", path));

        assert!(sound.frames > 128, "{id:?}: 帧数过少 ({})", sound.frames);
        assert!(
            (8000..=96000).contains(&sound.sample_rate),
            "{id:?}: 采样率异常 {}",
            sound.sample_rate
        );
        assert_eq!(
            sound.samples.len(),
            sound.frames * 2,
            "{id:?}: 交错立体声长度不符"
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
            "{id:?}: 前 2048 帧几乎是全零静音 ({} 个非零样本)",
            probe
        );
        // 样本幅度应在合理范围(f32 归一语义)。
        let peak = sound.samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.01 && peak <= 4.0, "{id:?}: 峰值异常 {peak}");
    }
}

#[test]
fn garbage_bytes_fail_cleanly() {
    // 素材损坏路径:必须返回 Err 而不是 panic(上层按缺失 no-op 处理)。
    let err = decode_to_stereo(b"this is not an ogg file at all".to_vec());
    assert!(err.is_err());
}
