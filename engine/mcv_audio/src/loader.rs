//! 游戏线程侧的素材解析:音效表(事件名 → 变体)+ 解码缓存(LRU)+ 同变体
//! 节流 + 缺失去重,产物是过环的 [`AudioCmd`](crate::AudioCmd)。
//!
//! 与 [`Mixer`](crate::Mixer) 的分工:文件 IO/解码/分配只发生在本侧(可以
//! 阻塞、可以慢);音频回调线程独占 Mixer,只做 apply + 混音——零锁零分配。
//! 衰减、增益合法性、一次性节流在本侧判定完毕,到达音频线程的命令只剩
//! "起声部"与"停循环"。
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::decode::{SoundData, decode_to_stereo};
use crate::table::SoundTable;
use crate::{AudioCmd, AudioError};

/// 常驻解码缓存上限(条)。键为变体路径(如 `dig/stone1`)。
pub const CACHE_MAX: usize = 32;
/// 同一变体路径一次性播放的最小触发间隔(节流窗口)。
pub const SAME_ID_MIN_INTERVAL: Duration = Duration::from_millis(80);

/// 游戏线程持有的素材加载器:不持有任何声部状态。
pub struct SoundLoader {
    /// 事件表:构造时从 `sounds.json` 加载一次;缺失 = 空表(音效静默)。
    table: SoundTable,
    sounds_dir: PathBuf,
    /// 解码缓存:变体路径 → PCM。
    cache: HashMap<String, Arc<SoundData>>,
    /// LRU 顺序:队首最旧。
    lru: VecDeque<String>,
    /// 已知缺失/解码失败的变体路径:不再重试,直接 no-op。
    missing: HashSet<String>,
    /// 各变体路径最近一次成功的一次性触发时间(节流用)。
    last_one_shot: HashMap<String, Instant>,
    /// 已告警过"不存在/为空"的事件名(首次告警,之后静默)。
    missing_events: HashSet<String>,
    /// 变体抽取的自增种子(SplitMix64 输入;自增保证序列可复现)。
    rng: u64,
}

impl SoundLoader {
    pub fn new(sounds_dir: impl AsRef<Path>) -> Self {
        let dir = sounds_dir.as_ref();
        let table = SoundTable::load(dir).unwrap_or_else(|e| {
            log::warn!("mcv_audio: 音效表不可用,音效静默降级: {e}");
            SoundTable::default()
        });
        Self {
            table,
            sounds_dir: dir.to_path_buf(),
            cache: HashMap::new(),
            lru: VecDeque::new(),
            missing: HashSet::new(),
            last_one_shot: HashMap::new(),
            missing_events: HashSet::new(),
            rng: 0,
        }
    }

    /// 音效表(测试/诊断用):可查事件是否存在、变体路径等。
    pub fn table(&self) -> &SoundTable {
        &self.table
    }

    /// 3D 位置播放事件:按 16 格线性衰减 [`crate::distance_attenuation`]。
    /// 返回 `None` = 事件缺失、衰减为 0、被节流或素材缺失。
    pub fn play_at(
        &mut self,
        event: &str,
        pos: [f32; 3],
        listener: [f32; 3],
        gain: f32,
        now: Instant,
    ) -> Option<AudioCmd> {
        let dist = crate::distance3(pos, listener);
        let atten = crate::distance_attenuation(dist, crate::DEFAULT_ATTENUATION_RADIUS);
        self.trigger(event, gain * atten, false, true, now, None)
    }

    /// 2D UI 播放:无衰减。语义同 [`SoundLoader::play_at`] 的返回值。
    pub fn play_ui(&mut self, event: &str, gain: f32, now: Instant) -> Option<AudioCmd> {
        self.trigger(event, gain, false, true, now, None)
    }

    /// 循环播放命令(挖掘持续音)。幂等判定在音频线程侧
    /// ([`Mixer::apply`](crate::Mixer::apply),按事件名);循环不参与一次性节流。
    pub fn loop_start(&mut self, event: &str, gain: f32, now: Instant) -> Option<AudioCmd> {
        self.trigger(event, gain, true, false, now, Some(event.to_string()))
    }

    // ---- 内部 ----

    /// 通用触发:选变体 → 增益预乘变体音量 → 解析素材并产出命令。
    /// `throttle=true` 时启用同变体路径最小间隔。
    fn trigger(
        &mut self,
        event: &str,
        gain: f32,
        looping: bool,
        throttle: bool,
        now: Instant,
        loop_event: Option<String>,
    ) -> Option<AudioCmd> {
        if !gain.is_finite() || gain <= 0.0 {
            return None;
        }
        // 抽变体(消费一个自增种子);事件缺失首次告警。
        let (key, volume, pitch) = {
            let seed = self.rng;
            self.rng = self.rng.wrapping_add(1);
            let v = self.table.pick(event, seed);
            if v.is_none() && self.missing_events.insert(event.to_string()) {
                log::warn!("mcv_audio: 音效事件 {event:?} 不存在或为空,按静默处理");
            }
            let v = v?;
            (v.path.to_string(), v.volume, v.pitch)
        };
        let gain = gain * volume;
        if !gain.is_finite() || gain <= 0.0 {
            return None;
        }
        if throttle
            && let Some(&last) = self.last_one_shot.get(&key)
            && now.saturating_duration_since(last) < SAME_ID_MIN_INTERVAL
        {
            return None;
        }
        let sound = self.get_or_load(&key)?;
        if throttle {
            self.last_one_shot.insert(key.clone(), now);
        }
        Some(AudioCmd::Play {
            key,
            sound,
            gain,
            pitch,
            looping,
            loop_event,
        })
    }

    /// 缓存命中直接返回;未命中读文件+解码;缺失/失败记入 missing 集合永久 no-op。
    fn get_or_load(&mut self, key: &str) -> Option<Arc<SoundData>> {
        if let Some(a) = self.cache.get(key) {
            let a = a.clone();
            self.touch_lru(key.to_string());
            return Some(a);
        }
        if self.missing.contains(key) {
            return None;
        }
        let path = self.sounds_dir.join(format!("{key}.ogg"));
        let loaded = (|| -> Result<SoundData, AudioError> {
            let bytes = std::fs::read(&path).map_err(AudioError::from)?;
            decode_to_stereo(bytes)
        })();
        let data = match loaded {
            Ok(d) if d.frames > 0 => d,
            Ok(_) => {
                log::warn!("mcv_audio: {:?} 解出 0 帧,按缺失处理", path);
                self.missing.insert(key.to_string());
                return None;
            }
            Err(e) => {
                log::warn!("mcv_audio: {:?} 加载失败:{}(后续 no-op)", path, e);
                self.missing.insert(key.to_string());
                return None;
            }
        };
        let a = Arc::new(data);
        self.cache.insert(key.to_string(), a.clone());
        self.touch_lru(key.to_string());
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

    fn touch_lru(&mut self, key: String) {
        if let Some(p) = self.lru.iter().position(|x| *x == key) {
            self.lru.remove(p);
        }
        self.lru.push_back(key);
    }
}
