//! 测试公共脚手架:临时目录内搭建「fixture sounds.json + 仓库 12 个扁平 ogg」。
//!
//! 全量原版音效树不入 git(CI 由 `ci/fetch-sounds.sh` 构建时拉取);单测靠
//! 仓库已跟踪的 12 个扁平开发音效(`dig_stone1.ogg` 等)走真实解码/混音链路:
//! fixture 事件的变体路径直接写扁平文件基名(loader 拼 `.ogg`)。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mcv_audio::default_sounds_dir;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// 仓库内已跟踪的 12 个扁平开发音效基名(不带 `.ogg`)。
pub const DEV_SOUNDS: [&str; 12] = [
    "dig_stone1",
    "dig_grass1",
    "dig_wood1",
    "step_stone1",
    "step_grass1",
    "step_wood1",
    "random_bow",
    "random_orb",
    "damage_hit1",
    "damage_hit2",
    "damage_hit3",
    "damage_fallsmall",
];

/// fixture 事件表:覆盖裸字符串/对象条目、乘子默认值、event/dictionary 展平、
/// 权重分布、零权重、坏引用与引用环。变体路径全部指向仓库扁平 ogg。
pub const FIXTURE_JSON: &str = r#"{
  "dev.oneshot": {"sounds": ["dig_stone1"]},
  "dev.oneshot2": {"sounds": ["dig_stone1"]},
  "dev.other": {"sounds": ["step_stone1"]},
  "dev.bow": {"sounds": ["random_bow"]},
  "dev.orb": {"sounds": ["random_orb"]},
  "dev.mixed": {"sounds": ["dig_stone1", {"name": "step_stone1", "pitch": 1.5, "volume": 0.5, "weight": 3}]},
  "dev.alias": {"sounds": [{"name": "dev.mixed", "type": "event"}]},
  "dev.alias_mod": {"sounds": [{"name": "dev.mixed", "pitch": 2.0, "type": "event", "volume": 0.25}]},
  "dev.dict": {"sounds": [{"name": "dev.dict.map", "type": "dictionary"}]},
  "dev.dict.map": {"sounds": {"dev.other": 3, "dev.oneshot": 1}},
  "dev.weighted": {"sounds": [{"name": "dig_stone1", "weight": 1}, {"name": "step_stone1", "weight": 3}]},
  "dev.zerow": {"sounds": [{"name": "dig_stone1", "weight": 0}, {"name": "step_stone1", "weight": 0}]},
  "dev.bad_ref": {"sounds": ["dig_stone1", {"name": "dev.nope", "type": "event"}]},
  "dev.cycle_a": {"sounds": [{"name": "dev.cycle_b", "type": "event"}]},
  "dev.cycle_b": {"sounds": [{"name": "dev.cycle_a", "type": "event"}]}
}"#;

/// 临时素材目录。`Drop` 时整体删除(测试失败也会清理)。
pub struct SoundFixture {
    dir: PathBuf,
}

impl SoundFixture {
    /// 建临时目录:写入 `sounds.json` 并复制仓库 12 个扁平 ogg。
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
            std::fs::copy(
                src.join(format!("{name}.ogg")),
                dir.join(format!("{name}.ogg")),
            )
            .expect("复制仓库扁平 ogg 失败(sounds/ 目录完整吗?)");
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
