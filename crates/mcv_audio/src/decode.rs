//! symphonia 解码:ogg/mp3/wav 字节 → 交错立体声 f32 PCM。
//!
//! 输出统一为 2 声道交错(`LRLRLR...`)f32,采样率保持源文件原值(原版素材多为
//! 22050 Hz 单声道),重采样在混音阶段 [`crate::Mixer`] 里用线性插值完成。
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::conv::IntoSample;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::AudioError;

/// 一段已解码的音效 PCM(交错立体声 f32,[-1.0, 1.0] 附近,未硬限幅)。
#[derive(Debug, Clone)]
pub struct SoundData {
    /// 源采样率(Hz)。混音端按比例线性插值重采样到输出设备采样率。
    pub sample_rate: u32,
    /// 帧数(= `samples.len() / 2`)。
    pub frames: usize,
    /// 交错立体声样本:`[L0, R0, L1, R1, ...]`。
    pub samples: Vec<f32>,
}

/// 把音频文件字节(ogg/mp3/wav,靠 symphonia 探测)解码为 [`SoundData`]。
///
/// 错误只在"完全无法解出音轨"时返回;流中途的解码错误只丢弃剩余部分,
/// 保留已解码样本(游戏音效优先可用性)。
pub fn decode_to_stereo(bytes: Vec<u8>) -> Result<SoundData, AudioError> {
    // symphonia 从内存读取:io::Cursor<Vec<u8>> 实现了 MediaSource。
    let mss = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes)),
        MediaSourceStreamOptions::default(),
    );
    let mut hint = Hint::new();
    // 素材一律 ogg;探测失败时按扩展名兜底。
    hint.with_extension("ogg");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Decode(e.to_string()))?;
    let mut format = probed.format;

    // 先取轨道信息再解码,避免后续对 format 的可变借用冲突。
    let (track_id, codec_params) = {
        let track = format.default_track().ok_or(AudioError::NoAudioTrack)?;
        (track.id, track.codec_params.clone())
    };
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| AudioError::Decode(e.to_string()))?;

    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // EndOfStream(IoError) 或其它容器级错误:停止读取,保留已解出的部分。
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => push_interleaved_stereo(&buf, &mut samples),
            Err(e) => {
                log::warn!("mcv_audio: 解码中断,丢弃剩余包: {}", e);
                break;
            }
        }
    }

    if samples.is_empty() {
        return Err(AudioError::Decode("未解出任何样本".into()));
    }
    let frames = samples.len() / 2;
    let sample_rate = codec_params.sample_rate.filter(|r| *r > 0).unwrap_or(44100);
    Ok(SoundData {
        sample_rate,
        frames,
        samples,
    })
}

/// 把一个解码输出帧(任意样本类型)按交错立体声追加到 `out`。
///
/// 单声道复制为双声道;多声道取前两条。样本类型经 symphonia 的
/// [`IntoSample`] 归一到 f32([-1.0, 1.0] 语义);少见类型(u24/i24/f64)跳过并告警。
fn push_interleaved_stereo(buf: &AudioBufferRef<'_>, out: &mut Vec<f32>) {
    macro_rules! push_typed {
        ($b:expr) => {{
            let b = $b;
            let nch = b.spec().channels.count().max(1);
            out.reserve(b.frames() * 2);
            for f in 0..b.frames() {
                let l: f32 = b.chan(0)[f].into_sample();
                let r: f32 = if nch > 1 {
                    b.chan(1)[f].into_sample()
                } else {
                    l
                };
                out.push(l);
                out.push(r);
            }
        }};
    }
    match buf {
        // vorbis(全部开发期素材)走这条最快路径。
        AudioBufferRef::F32(b) => push_typed!(&**b),
        AudioBufferRef::S16(b) => push_typed!(&**b),
        AudioBufferRef::U16(b) => push_typed!(&**b),
        AudioBufferRef::S32(b) => push_typed!(&**b),
        AudioBufferRef::U8(b) => push_typed!(&**b),
        other => {
            log::warn!("mcv_audio: 不支持的样本格式 {:?},跳过该帧", other.spec());
        }
    }
}
