//! 游戏线程侧的素材解析:解码缓存(LRU)+ 同 id 节流 + 缺失去重,产物是
//! 过环的 [`AudioCmd`](crate::AudioCmd)。
//!
//! 与 [`Mixer`](crate::Mixer) 的分工:文件 IO/解码/分配只发生在本侧(可以
//! 阻塞、可以慢);音频回调线程独占 Mixer,只做 apply + 混音——零锁零分配。
//! 衰减、增益合法性、一次性节流在本侧判定完毕,到达音频线程的命令只剩
//! "起声部"与"停循环"。
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::decode::{decode_to_stereo, SoundData};
use crate::{AudioCmd, AudioError, SoundId};

/// 常驻解码缓存上限(条)。
pub const CACHE_MAX: usize = 32;
/// 同一 id 一次性播放的最小触发间隔(节流窗口)。
pub const SAME_ID_MIN_INTERVAL: Duration = Duration::from_millis(80);

/// 游戏线程持有的素材加载器:不持有任何声部状态。
pub struct SoundLoader {
    sounds_dir: PathBuf,
    /// 解码缓存:id → PCM。
    cache: HashMap<SoundId, Arc<SoundData>>,
    /// LRU 顺序:队首最旧。
    lru: VecDeque<SoundId>,
    /// 已知缺失/解码失败的 id:不再重试,直接 no-op。
    missing: HashSet<SoundId>,
    /// 各 id 最近一次成功的一次性触发时间(节流用)。
    last_one_shot: HashMap<SoundId, Instant>,
}

impl SoundLoader {
    pub fn new(sounds_dir: impl AsRef<Path>) -> Self {
        Self {
            sounds_dir: sounds_dir.as_ref().to_path_buf(),
            cache: HashMap::new(),
            lru: VecDeque::new(),
            missing: HashSet::new(),
            last_one_shot: HashMap::new(),
        }
    }

    /// 3D 位置播放:按 16 格线性衰减 [`crate::distance_attenuation`]。
    /// 返回 `None` = 衰减为 0、被节流、增益非法或素材缺失。
    pub fn play_at(
        &mut self,
        id: SoundId,
        pos: [f32; 3],
        listener: [f32; 3],
        gain: f32,
        now: Instant,
    ) -> Option<AudioCmd> {
        let dist = crate::distance3(pos, listener);
        let atten = crate::distance_attenuation(dist, crate::DEFAULT_ATTENUATION_RADIUS);
        self.trigger(id, gain * atten, false, true, now)
    }

    /// 2D UI 播放:无衰减。语义同 [`SoundLoader::play_at`] 的返回值。
    pub fn play_ui(&mut self, id: SoundId, gain: f32, now: Instant) -> Option<AudioCmd> {
        self.trigger(id, gain, false, true, now)
    }

    /// 循环播放命令(挖掘持续音)。幂等判定在音频线程侧
    /// ([`Mixer::apply`](crate::Mixer::apply));循环不参与一次性节流。
    pub fn loop_start(&mut self, id: SoundId, gain: f32, now: Instant) -> Option<AudioCmd> {
        self.trigger(id, gain, true, false, now)
    }

    // ---- 内部 ----

    /// 通用触发:增益预乘后解析素材并产出命令。`throttle=true` 时启用同 id 最小间隔。
    fn trigger(
        &mut self,
        id: SoundId,
        gain: f32,
        looping: bool,
        throttle: bool,
        now: Instant,
    ) -> Option<AudioCmd> {
        if !gain.is_finite() || gain <= 0.0 {
            return None;
        }
        if throttle {
            if let Some(&last) = self.last_one_shot.get(&id) {
                if now.saturating_duration_since(last) < SAME_ID_MIN_INTERVAL {
                    return None;
                }
            }
        }
        let sound = self.get_or_load(id)?;
        if throttle {
            self.last_one_shot.insert(id, now);
        }
        Some(AudioCmd::Play {
            id,
            sound,
            gain,
            looping,
        })
    }

    /// 缓存命中直接返回;未命中读文件+解码;缺失/失败记入 missing 集合永久 no-op。
    fn get_or_load(&mut self, id: SoundId) -> Option<Arc<SoundData>> {
        if let Some(a) = self.cache.get(&id) {
            let a = a.clone();
            self.touch_lru(id);
            return Some(a);
        }
        if self.missing.contains(&id) {
            return None;
        }
        let path = self.sounds_dir.join(id.file_name());
        let loaded = (|| -> Result<SoundData, AudioError> {
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
