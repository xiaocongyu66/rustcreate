//! 音频线程侧:消费 [`AudioCmd`](crate::AudioCmd) + 声部混音。
//! **由 tinyaudio 回调线程独占——无锁、无文件 IO、热路径零分配。**
//!
//! 设计:
//! - [`Mixer`] 持有 SPSC [`Consumer`](mcv_sync::Consumer),每次回调开头排空
//!   命令增删声部;素材解析在 [`SoundLoader`](crate::SoundLoader)(游戏线程),
//!   本文件不触碰锁与文件;
//! - 主增益从 `Arc<AtomicU32>` 位镜像读取(游戏线程直写),音量滑条即时生效
//!   且回调无锁;
//! - 循环声部按事件名幂等(`loop_event` 不叠加)在此判定:游戏侧可能每帧
//!   重复发命令,本侧见活动声部即丢;
//! - 混音为 f32 线性叠加,输出前统一限幅到 [-1.0, 1.0];
//!   tinyaudio 后端按 PCM_Float(IEEE float)直接吃 f32,无需 u16 转换;
//! - 超出 [`MAX_VOICES`] 时淘汰最早建立的声部(FIFO)。
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use mcv_sync::Consumer;

use crate::AudioCmd;
use crate::decode::SoundData;

/// 声部数上限:超出时淘汰最早建立的声部。
pub const MAX_VOICES: usize = 24;

/// 一个活动声部:指向已解码 PCM 的游标。
struct Voice {
    /// 循环声部的事件名(幂等键);一次性播放为 `None`。
    loop_event: Option<String>,
    sound: Arc<SoundData>,
    /// 源帧坐标系下的读取位置(线性插值的小数部分即 `pos - floor(pos)`)。
    pos: f64,
    /// 每输出帧推进的源帧数 = 源采样率 / 输出采样率(pitch 为其乘子)。
    step: f64,
    /// 变体音高乘子(>1 播放更快/更高)。
    pitch: f32,
    /// 声部增益(游戏线程已乘衰减与变体音量)。
    gain: f32,
    /// 是否循环(挖掘持续音)。
    looping: bool,
    alive: bool,
}

/// 纯软件混音器,音频回调线程独占。
pub struct Mixer {
    voices: Vec<Voice>,
    cmds: Consumer<AudioCmd>,
    /// 主增益 f32 位镜像(游戏线程 set_master_gain 直写)。
    master: Arc<AtomicU32>,
    out_rate: u32,
}

impl Mixer {
    /// 新建混音器。`out_rate` 为输出设备采样率(Hz),素材采样率不同会自动
    /// 线性重采样;`master` 为主增益原子镜像。
    pub fn new(out_rate: u32, cmds: Consumer<AudioCmd>, master: Arc<AtomicU32>) -> Self {
        Self {
            voices: Vec::new(),
            cmds,
            master,
            out_rate: out_rate.max(1),
        }
    }

    /// 应用一条命令;返回是否真正起了声部(非法增益/循环幂等命中/停止均 false)。
    pub fn apply(&mut self, cmd: AudioCmd) -> bool {
        match cmd {
            AudioCmd::Play {
                key: _,
                sound,
                gain,
                pitch,
                looping,
                loop_event,
            } => {
                // 增益在游戏线程已查过,此处是跨环后的最后防线(NaN 防护)。
                if !gain.is_finite() || gain <= 0.0 {
                    return false;
                }
                // pitch 非法(NaN/非正)按原速播放。
                let pitch = if pitch.is_finite() && pitch > 0.0 {
                    pitch
                } else {
                    1.0
                };
                if looping
                    && let Some(ev) = loop_event.as_deref()
                    && self
                        .voices
                        .iter()
                        .any(|v| v.looping && v.alive && v.loop_event.as_deref() == Some(ev))
                {
                    return false;
                }
                let step = f64::from(sound.sample_rate) / f64::from(self.out_rate);
                self.voices.push(Voice {
                    loop_event,
                    sound,
                    pos: 0.0,
                    step,
                    pitch,
                    gain,
                    looping,
                    alive: true,
                });
                if self.voices.len() > MAX_VOICES {
                    // 淘汰最早的声部(循环声部一般刚建立,不会被误伤到)。
                    self.voices.remove(0);
                }
                true
            }
            AudioCmd::StopLoop { event } => {
                self.voices
                    .retain(|v| !(v.looping && v.loop_event.as_deref() == Some(event.as_str())));
                false
            }
        }
    }

    /// 把当前所有声部混进 `out`(`out` 为交错立体声,长度 = 帧数 × 2)。
    /// 由音频后端回调驱动:先排空命令环,`out` 清零叠加后统一限幅。
    pub fn mix(&mut self, out: &mut [f32]) {
        while let Some(cmd) = self.cmds.pop() {
            self.apply(cmd);
        }
        out.fill(0.0);
        let master = f32::from_bits(self.master.load(Ordering::Relaxed));
        let mut voices = std::mem::take(&mut self.voices);
        for v in voices.iter_mut() {
            mix_voice(v, out, master);
        }
        voices.retain(|v| v.alive);
        self.voices = voices;
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    /// 当前活动声部数(测试/调试用)。
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.alive).count()
    }
}

/// 单声部混入:线性插值重采样(步进乘 pitch)+ 增益叠加;非循环读完即熄灭,
/// 循环取模续读。
fn mix_voice(v: &mut Voice, out: &mut [f32], master: f32) {
    let sound = v.sound.clone();
    let total = sound.frames;
    if total == 0 {
        v.alive = false;
        return;
    }
    let g = v.gain * master;
    let total_f = total as f64;
    let step = v.step * f64::from(v.pitch);
    let mut pos = v.pos;
    let n_out = out.len() / 2;
    for k in 0..n_out {
        if pos >= total_f {
            if !v.looping {
                v.alive = false;
                break;
            }
            pos %= total_f;
        }
        let i0 = pos as usize;
        let frac = (pos - i0 as f64) as f32;
        let mut i1 = i0 + 1;
        if i1 >= total {
            i1 = if v.looping { i1 % total } else { total - 1 };
        }
        let a = &sound.samples;
        let l = a[i0 * 2] + (a[i1 * 2] - a[i0 * 2]) * frac;
        let r = a[i0 * 2 + 1] + (a[i1 * 2 + 1] - a[i0 * 2 + 1]) * frac;
        out[k * 2] += l * g;
        out[k * 2 + 1] += r * g;
        pos += step;
    }
    v.pos = pos;
}
