//! 测试公共脚手架:临时目录内搭建「fixture sounds.json + 真实树 ogg 子集」。
//!
//! 全量原版音效树不入 git(CI 由 `ci/fetch-sounds.sh` 构建时拉取);单测从
//! [`default_sounds_dir`] 指向的真实树复制 12 个已验证存在的原版变体(原版
//! 路径如 `dig/stone1`),走真实解码/混音链路。未 fetch 时这些测试直接失败
//! 并提示先跑 fetch(CI 的 test job 必跑 fetch)。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mcv_audio::default_sounds_dir;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// 真实树中已验证存在(index 30)的 12 个原版变体路径(不带 `.ogg`)。
pub const DEV_SOUNDS: [&str; 12] = [
    "dig/stone1",
    "dig/grass1",
    "dig/wood1",
    "step/stone1",
    "step/grass1",
    "step/wood1",
    "random/bow",
    "random/orb",
    "damage/hit1",
    "damage/hit2",
    "damage/hit3",
    "damage/fallsmall",
];

/// fixture 事件表:覆盖裸字符串/对象条目、乘子默认值、event/dictionary 展平、
/// 权重分布、零权重、坏引用与引用环。变体路径全部指向真实树路径。
pub const FIXTURE_JSON: &str = r#"{
  "dev.oneshot": {"sounds": ["dig/stone1"]},
  "dev.oneshot2": {"sounds": ["dig/stone1"]},
  "dev.other": {"sounds": ["step/stone1"]},
  "dev.bow": {"sounds": ["random/bow"]},
  "dev.orb": {"sounds": ["random/orb"]},
  "dev.mixed": {"sounds": ["dig/stone1", {"name": "step/stone1", "pitch": 1.5, "volume": 0.5, "weight": 3}]},
  "dev.alias": {"sounds": [{"name": "dev.mixed", "type": "event"}]},
  "dev.alias_mod": {"sounds": [{"name": "dev.mixed", "pitch": 2.0, "type": "event", "volume": 0.25}]},
  "dev.dict": {"sounds": [{"name": "dev.dict.map", "type": "dictionary"}]},
  "dev.dict.map": {"sounds": {"dev.other": 3, "dev.oneshot": 1}},
  "dev.weighted": {"sounds": [{"name": "dig/stone1", "weight": 1}, {"name": "step/stone1", "weight": 3}]},
  "dev.zerow": {"sounds": [{"name": "dig/stone1", "weight": 0}, {"name": "step/stone1", "weight": 0}]},
  "dev.bad_ref": {"sounds": ["dig/stone1", {"name": "dev.nope", "type": "event"}]},
  "dev.cycle_a": {"sounds": [{"name": "dev.cycle_b", "type": "event"}]},
  "dev.cycle_b": {"sounds": [{"name": "dev.cycle_a", "type": "event"}]}
}"#;

/// 临时素材目录。`Drop` 时整体删除(测试失败也会清理)。
pub struct SoundFixture {
    dir: PathBuf,
}

impl SoundFixture {
    /// 建临时目录:写入 `sounds.json` 并按原样路径复制真实树 12 个 ogg。
    pub fn new() -> Self {
        Self::with_json(FIXTURE_JSON)
    }

    /// 同 [`SoundFixture::new`],但用自定义 sounds.json 内容。
    pub fn with_json(json: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "mcv-audio-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("建临时素材目录失败");
        std::fs::write(dir.join("sounds.json"), json).expect("写 fixture sounds.json 失败");
        let src = default_sounds_dir();
        for name in DEV_SOUNDS {
            let dest = dir.join(format!("{name}.ogg"));
            let Some(parent) = dest.parent() else {
                continue;
            };
            std::fs::create_dir_all(parent).expect("建 fixture 子目录失败");
            std::fs::copy(src.join(format!("{name}.ogg")), &dest)
                .expect("复制真实树 ogg 失败——先运行 `bash ci/fetch-sounds.sh` 拉取音效树");
        }
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Default for SoundFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SoundFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
