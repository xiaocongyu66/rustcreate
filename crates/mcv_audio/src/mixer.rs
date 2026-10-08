//! 软件混音核心:声部叠加 + 节流 + LRU 解码缓存。**与音频后端无关,可无设备单测。**
//!
//! 设计:
//! - 混音为 f32 线性叠加,输出前统一限幅到 [-1.0, 1.0];
//!   tinyaudio 后端按 PCM_Float(IEEE float)直接吃 f32,无需 u16 转换;
//! - 同一 [`SoundId`] 的一次性播放有最小间隔节流(默认 80ms),防止挖掘/脚步连触发
//!   叠加成爆音;循环声部(loop)按 id 幂等,不参与节流;
//! - 解码结果按 id 缓存,上限 [`CACHE_MAX`](crate::CACHE_MAX) 条,超限按 LRU 淘汰
//!   (正在播放的声部持有 `Arc` 克隆,淘汰不影响其继续播放);
//! - 素材缺失/解码失败:首次告警一次,之后彻底 no-op,不 panic。
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::decode::{decode_to_stereo, SoundData};
use crate::{AudioError, SoundId};

/// 声部数上限:超出时淘汰最早建立的声部。
pub const MAX_VOICES: usize = 24;
/// 常驻解码缓存上限(条)。
pub const CACHE_MAX: usize = 32;
/// 同一 id 一次性播放的最小触发间隔(节流窗口)。
pub const SAME_ID_MIN_INTERVAL: Duration = Duration::from_millis(80);

/// 一个活动声部:指向缓存 PCM 的游标。
struct Voice {
    id: SoundId,
    sound: Arc<SoundData>,
    /// 源帧坐标系下的读取位置(线性插值的小数部分即 `pos - floor(pos)`)。
    pos: f64,
    /// 每输出帧推进的源帧数 = 源采样率 / 输出采样率。
    step: f64,
    /// 声部增益(已乘衰减值)。
    gain: f32,
    /// 是否循环(挖掘持续音)。
    looping: bool,
    alive: bool,
}

/// 纯软件混音器。`AudioManager` 内部持有它;测试可脱离后端直接使用。
pub struct Mixer {
    sounds_dir: PathBuf,
    /// 解码缓存:id → PCM。
    cache: HashMap<SoundId, Arc<SoundData>>,
    /// LRU 顺序:队首最旧。
    lru: VecDeque<SoundId>,
    /// 已知缺失/解码失败的 id:不再重试,直接 no-op。
    missing: HashSet<SoundId>,
    voices: Vec<Voice>,
    /// 各 id 最近一次成功的一次性触发时间(节流用)。
    last_one_shot: HashMap<SoundId, Instant>,
    master_gain: f32,
    out_rate: u32,
}

impl Mixer {
    /// 新建混音器。`out_rate` 为输出设备采样率(Hz);素材采样率不同会自动线性重采样。
    pub fn new(sounds_dir: impl AsRef<Path>, out_rate: u32) -> Self {
        Self {
            sounds_dir: sounds_dir.as_ref().to_path_buf(),
            cache: HashMap::new(),
            lru: VecDeque::new(),
            missing: HashSet::new(),
            voices: Vec::new(),
            last_one_shot: HashMap::new(),
            master_gain: 1.0,
            out_rate: out_rate.max(1),
        }
    }

    /// 3D 位置播放:按 16 格线性衰减 [`crate::distance_attenuation`]。
    /// 返回是否真正起播(衰减为 0、被节流、素材缺失均返回 false)。
    pub fn play_at(
        &mut self,
        id: SoundId,
        pos: [f32; 3],
        listener: [f32; 3],
        gain: f32,
        now: Instant,
    ) -> bool {
        let dist = crate::distance3(pos, listener);
        let atten = crate::distance_attenuation(dist, crate::DEFAULT_ATTENUATION_RADIUS);
        self.trigger(id, gain * atten, false, true, now)
    }

    /// 2D UI 播放:无衰减。语义同 [`Mixer::play_at`] 的返回值。
    pub fn play_ui(&mut self, id: SoundId, gain: f32, now: Instant) -> bool {
        self.trigger(id, gain, false, true, now)
    }

    /// 开始循环播放(挖掘持续音)。同一 id 已在循环则幂等返回 false,不叠加声部。
    pub fn loop_start(&mut self, id: SoundId, gain: f32, now: Instant) -> bool {
        // 幂等判定:已有该 id 的活动循环声部则直接忽略。
        if self
            .voices
            .iter()
            .any(|v| v.looping && v.alive && v.id == id)
        {
            return false;
        }
        // 循环声部不参与一次性节流(幂等性已足够防叠加)。
        self.trigger(id, gain, true, false, now)
    }

    /// 停止指定 id 的循环声部(挖掘结束)。一次性声部不受影响。
    pub fn loop_stop(&mut self, id: SoundId) {
        self.voices.retain(|v| !(v.looping && v.id == id));
    }

    /// 设置主增益,推荐范围 [0.0, 1.0](>1 会更容易触发限幅)。
    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = gain;
    }

    /// 当前主增益。
    pub fn master_gain(&self) -> f32 {
        self.master_gain
    }

    /// 把当前所有声部混进 `out`(`out` 为交错立体声,长度 = 帧数 × 2)。
    /// 由音频后端回调驱动;`out` 会被先清零再叠加,最后统一限幅。
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let master = self.master_gain;
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

    // ---- 内部 ----

    /// 通用触发:增益预乘后启动一个声部。`throttle=true` 时启用同 id 最小间隔节流。
    fn trigger(
        &mut self,
        id: SoundId,
        gain: f32,
        looping: bool,
        throttle: bool,
        now: Instant,
    ) -> bool {
        if !gain.is_finite() || gain <= 0.0 {
            return false;
        }
        if throttle {
            if let Some(&last) = self.last_one_shot.get(&id) {
                if now.saturating_duration_since(last) < SAME_ID_MIN_INTERVAL {
                    return false;
                }
            }
        }
        let Some(sound) = self.get_or_load(id) else {
            return false;
        };
        let step = sound.sample_rate as f64 / self.out_rate as f64;
        self.voices.push(Voice {
            id,
            sound,
            pos: 0.0,
            step,
            gain,
            looping,
            alive: true,
        });
        if self.voices.len() > MAX_VOICES {
            // 淘汰最早的声部(简单 FIFO;循环声部一般刚建立,不会被误伤到)。
            self.voices.remove(0);
        }
        if throttle {
            self.last_one_shot.insert(id, now);
        }
        true
    }

    /// 缓存命中直接返回;未命中读文件+解码;缺失/失败记入 missing 集合永久 no-op。
    fn get_or_load(&mut self, id: SoundId) -> Option<Arc<SoundData>> {
        if let Some(a) = self.cache.get(&id) {
            self.touch_lru(id);
            return Some(a.clone());
        }
        if self.missing.contains(&id) {
            return None;
        }
        let path = self.sounds_dir.join(id.file_name());
        let loaded = (|| -> Result<SoundData, crate::AudioError> {
            let bytes = std::fs::read(&path).map_err(AudioError::from)?;
            decode_to_stereo(bytes)
        })();
        let data = match loaded {
            Ok(d) if d.frames > 0 => d,
            Ok(_) => {
                log::warn!("mcv_audio: {:?} 解出 0 帧,按缺失处理", path);
                self.missing.insert(id);
                return None;
            }
            Err(e) => {
                log::warn!("mcv_audio: {:?} 加载失败:{}(后续 no-op)", path, e);
                self.missing.insert(id);
                return None;
            }
        };
        let a = Arc::new(data);
        self.cache.insert(id, a.clone());
        self.touch_lru(id);
        while self.lru.len() > CACHE_MAX {
            match self.lru.pop_front() {
                Some(old) => {
                    self.cache.remove(&old);
                }
                None => break,
            }
        }
        Some(a)
    }

    fn touch_lru(&mut self, id: SoundId) {
        if let Some(p) = self.lru.iter().position(|x| *x == id) {
            self.lru.remove(p);
        }
        self.lru.push_back(id);
    }
}

/// 单声部混入:线性插值重采样 + 增益叠加;非循环读完即熄灭,循环取模续读。
fn mix_voice(v: &mut Voice, out: &mut [f32], master: f32) {
    let sound = v.sound.clone();
    let total = sound.frames;
    if total == 0 {
        v.alive = false;
        return;
    }
    let g = v.gain * master;
    let total_f = total as f64;
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
        pos += v.step;
    }
    v.pos = pos;
}
