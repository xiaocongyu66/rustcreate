//! 原版 `sounds.json` 解析:事件名 → 变体表(MC 26.1,assets index 30)。
//!
//! 格式:顶层为 `事件名 → {sounds: [...], subtitle, replace}`;`sounds` 条目是
//! 字符串(资产路径,相对 `assets/minecraft/`,不带 `.ogg`)或对象
//! `{name, volume=1, pitch=1, weight=1, stream=false, type="sound"|"event"|"dictionary"}`。
//! 加载期两阶段处理:
//! 1. 逐事件解析条目;单个事件解析失败只跳过并告警,不拖垮整表;
//! 2. 递归展平引用(与原版 SoundHandler 一致,引用条目自身的 volume/pitch/weight
//!    乘到子变体上):`type:"event"` 引用另一事件;`type:"dictionary"` 引用
//!    「事件名 → 权重」字典事件,按权重把各键指向的事件展平进本事件。
//!    防环用「在途事件集」剪断,成环条目跳过并告警。
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde_json::Value;

use crate::AudioError;

/// 单个可播放变体:ogg 相对路径(loader 拼 `.ogg`)+ 原版音量/音高/权重乘子。
#[derive(Debug, Clone)]
pub struct Variant {
    /// 资产路径(如 `dig/stone1`),相对 `sounds/`,不带扩展名。
    pub path: Cow<'static, str>,
    /// 变体音量乘子(与触发增益预乘)。
    pub volume: f32,
    /// 变体音高乘子(混音端改播放速率)。
    pub pitch: f32,
    /// 加权随机权重(原版 SoundHandler.select)。
    pub weight: u32,
    /// 原版流式标记(音乐类大文件;本引擎暂按整块解码,仅透传给校验工具)。
    pub stream: bool,
}

/// 一个原版音效事件:触发时按权重随机选一变体。
#[derive(Debug, Clone, Default)]
pub struct SoundEvent {
    pub variants: Vec<Variant>,
}

/// 全量音效表:事件名 → 事件。`sounds.json` 缺失/不可解析时为空表
/// (播放静默降级)。
#[derive(Debug, Default)]
pub struct SoundTable {
    events: HashMap<Cow<'static, str>, SoundEvent>,
}

/// 加载期中间形态:字典事件的「事件名 → 权重」映射,或普通事件的条目列表。
enum RawEvent {
    Sounds(Vec<RawEntry>),
    Dict(BTreeMap<String, u32>),
}

/// 普通事件的未解析条目。
enum RawEntry {
    /// `type:"sound"`(或裸字符串):直接文件变体。
    File(Variant),
    /// `type:"event"` / `type:"dictionary"`:引用另一事件(加载期递归展平,
    /// 具体展开方式由被引事件的形态决定,两者的解析机制相同)。
    Ref {
        name: String,
        volume: f32,
        pitch: f32,
        weight: u32,
    },
}

impl SoundTable {
    /// 读取 `sounds_dir/sounds.json` 并展平。IO 失败(文件缺失)或根结构非法
    /// 返回 `Err`;单个事件解析失败只跳过并 `log::warn`。
    pub fn load(sounds_dir: impl AsRef<Path>) -> Result<Self, AudioError> {
        let path = sounds_dir.as_ref().join("sounds.json");
        let bytes = std::fs::read(&path)?;
        let root: Value = serde_json::from_slice(&bytes)
            .map_err(|e| AudioError::Decode(format!("{:?}: {}", path, e)))?;
        let Some(top) = root.as_object() else {
            return Err(AudioError::Decode(format!("{:?}: 顶层不是对象", path)));
        };
        // 一阶段:逐事件解析成中间形态。
        let mut raw: BTreeMap<String, RawEvent> = BTreeMap::new();
        for (name, ev) in top {
            match parse_event(ev) {
                Some(re) => {
                    raw.insert(name.clone(), re);
                }
                None => log::warn!("mcv_audio: sounds.json 事件 {name:?} 解析失败,已跳过"),
            }
        }
        // 二阶段:递归展平引用(备忘录 + 在途集防环)。
        let mut table = Self {
            events: HashMap::with_capacity(raw.len()),
        };
        let mut path = Vec::new();
        for name in raw.keys() {
            table.resolve(name, &raw, &mut path);
        }
        Ok(table)
    }

    /// 事件数(供诊断/校验)。
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// 是否空表(sounds.json 缺失或全坏)。
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// 遍历全部事件(名称,事件)。供校验工具/诊断用;顺序不保证。
    pub fn events(&self) -> impl Iterator<Item = (&str, &SoundEvent)> {
        self.events.iter().map(|(k, v)| (&**k, v))
    }

    /// 加权随机选一变体。`rng_seed` 为调用方自增种子,经 SplitMix64 扩散后
    /// 对总权重取模——同种子序列可复现(可测)。`weight <= 0` 按 1 参与抽取;
    /// 全零权重取首个;事件不存在/无变体返回 `None`。
    pub fn pick(&self, event: &str, rng_seed: u64) -> Option<&Variant> {
        let ev = self.events.get(event)?;
        let variants = &ev.variants;
        let first = variants.first()?;
        let raw_total: u64 = variants.iter().map(|v| u64::from(v.weight)).sum();
        if raw_total == 0 {
            return Some(first);
        }
        let total: u64 = variants.iter().map(|v| u64::from(v.weight.max(1))).sum();
        let mut roll = splitmix64(rng_seed) % total;
        for v in variants {
            let w = u64::from(v.weight.max(1));
            if roll < w {
                return Some(v);
            }
            roll -= w;
        }
        // 数学上不可达(roll < 总权重,且每步至少减 1);兜底返回末位。
        Some(&variants[variants.len() - 1])
    }

    /// 递归解析一个事件为变体列表,并备忘录进 `events`。
    /// `path` 为当前在途引用链(防环)。引用缺失/成环返回空列表(条目跳过)。
    fn resolve(
        &mut self,
        name: &str,
        raw: &BTreeMap<String, RawEvent>,
        path: &mut Vec<String>,
    ) -> Vec<Variant> {
        if let Some(ev) = self.events.get(name) {
            return ev.variants.clone();
        }
        if path.iter().any(|p| p == name) {
            log::warn!("mcv_audio: sounds.json 音效引用环({name}),相关条目已跳过");
            return Vec::new();
        }
        let Some(re) = raw.get(name) else {
            log::warn!("mcv_audio: sounds.json 引用了不存在的事件 {name:?},条目已跳过");
            return Vec::new();
        };
        path.push(name.to_string());
        let mut variants = Vec::new();
        match re {
            RawEvent::Sounds(entries) => {
                for entry in entries {
                    match entry {
                        RawEntry::File(v) => variants.push(v.clone()),
                        RawEntry::Ref {
                            name: target,
                            volume,
                            pitch,
                            weight,
                        } => {
                            let sub = self.resolve(target, raw, path);
                            push_multiplied(&mut variants, sub, *volume, *pitch, *weight);
                        }
                    }
                }
            }
            RawEvent::Dict(mapping) => {
                for (key, weight) in mapping {
                    let sub = self.resolve(key, raw, path);
                    push_multiplied(&mut variants, sub, 1.0, 1.0, *weight);
                }
            }
        }
        path.pop();
        self.events.insert(
            Cow::Owned(name.to_string()),
            SoundEvent {
                variants: variants.clone(),
            },
        );
        variants
    }
}

/// 把展开出的子变体按引用条目的乘子(volume/pitch/weight)并入 `out`。
fn push_multiplied(
    out: &mut Vec<Variant>,
    sub: Vec<Variant>,
    volume: f32,
    pitch: f32,
    weight: u32,
) {
    for mut v in sub {
        v.volume *= volume;
        v.pitch *= pitch;
        v.weight = v.weight.saturating_mul(weight.max(1));
        out.push(v);
    }
}

/// 一阶段:解析单个事件的 JSON 值。`sounds` 为数组 → 普通事件;为对象 →
/// 「事件名 → 权重」字典事件;缺失/其它类型 → `None`(事件级跳过)。
fn parse_event(ev: &Value) -> Option<RawEvent> {
    let sounds = ev.get("sounds")?;
    match sounds {
        Value::Array(arr) => {
            let mut entries = Vec::with_capacity(arr.len());
            for s in arr {
                match parse_entry(s) {
                    Some(e) => entries.push(e),
                    None => log::warn!("mcv_audio: sounds.json 条目非法,已跳过: {s}"),
                }
            }
            Some(RawEvent::Sounds(entries))
        }
        Value::Object(map) => {
            let mut weights = BTreeMap::new();
            for (key, w) in map {
                match w.as_u64() {
                    Some(w) => {
                        weights.insert(key.clone(), u32::try_from(w).unwrap_or(u32::MAX));
                    }
                    None => log::warn!("mcv_audio: sounds.json 字典条目权重非法,已跳过: {key}"),
                }
            }
            Some(RawEvent::Dict(weights))
        }
        _ => None,
    }
}

/// 解析 `sounds` 数组的一个条目:裸字符串 → 默认乘子的文件变体;对象 → 按
/// `type` 分派(sound/event/dictionary)。无法解析返回 `None`(条目级跳过)。
fn parse_entry(s: &Value) -> Option<RawEntry> {
    match s {
        Value::String(path) => Some(RawEntry::File(Variant {
            path: Cow::Owned(path.clone()),
            volume: 1.0,
            pitch: 1.0,
            weight: 1,
            stream: false,
        })),
        Value::Object(o) => {
            let name = o.get("name").and_then(Value::as_str)?;
            let ty = o.get("type").and_then(Value::as_str).unwrap_or("sound");
            let volume = num_field(o.get("volume")).unwrap_or(1.0);
            let pitch = num_field(o.get("pitch")).unwrap_or(1.0);
            let weight = o
                .get("weight")
                .and_then(Value::as_u64)
                .map(|w| u32::try_from(w).unwrap_or(u32::MAX))
                .unwrap_or(1);
            let stream = o.get("stream").and_then(Value::as_bool).unwrap_or(false);
            match ty {
                "sound" => Some(RawEntry::File(Variant {
                    path: Cow::Owned(name.to_string()),
                    volume,
                    pitch,
                    weight,
                    stream,
                })),
                "event" | "dictionary" => Some(RawEntry::Ref {
                    name: name.to_string(),
                    volume,
                    pitch,
                    weight,
                }),
                other => {
                    log::warn!("mcv_audio: sounds.json 未知条目类型 {other:?}(name={name}),已跳过");
                    None
                }
            }
        }
        _ => None,
    }
}

/// 可选 f32 字段:数字且有限才有效(非有限按字段默认处理)。
fn num_field(v: Option<&Value>) -> Option<f32> {
    v.and_then(Value::as_f64)
        .map(|x| x as f32)
        .filter(|x| x.is_finite())
}

/// SplitMix64:把自增种子扩散成均匀位型,供 [`SoundTable::pick`] 取模。
fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
