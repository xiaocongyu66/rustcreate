//! # mcv_audio —— 纯 Rust 游戏音效播放器
//!
//! 面向方块脚步 / 挖掘 / 放置 / 跳跃落地等行为音效,给主控接线用。
//!
//! ## 技术选型
//! - **输出后端:[`tinyaudio`] 2.x**。上游 2.0 起为纯 Rust 重写:Android 走 AAudio
//!   (经 `ndk` crate 绑定,无需手写 JNI),Windows 走 DirectSound,Linux 走 ALSA
//!   (可选 PulseAudio)。依赖面极小、不需要 cc/build.rs,比 cpal 轻一个数量级。
//!   输出格式为 IEEE float PCM,混音全程 f32,不存在 16-bit 转换环节。
//! - **解码:[`symphonia`] 0.5**(纯 Rust),开 `ogg + vorbis + wav + pcm` feature,
//!   覆盖开发期素材(ogg)并兼容 wav / mp3(mp3 可按需加 feature)。
//! - **混音**:简单 f32 叠加 + 主增益 + 输出限幅;同一 id 一次性播放 80ms 节流,
//!   循环声部按 id 幂等 —— 挖掘持续音不会叠加倍增爆音。
//! - **线程模型**:游戏线程 [`SoundLoader`] 解析素材(IO/解码/LRU/节流),把
//!   [`AudioCmd`] 推入 [`mcv_sync::Spsc`] 无锁环;音频回调线程独占 [`Mixer`]
//!   消费——回调热路径零锁零分配。主增益走 `AtomicU32` 位镜像,音量滑条
//!   无锁即时生效。
//!
//! ## 接线速览(主控)
//! ```no_run
//! use mcv_audio::{AudioManager, SoundId};
//!
//! # fn main() -> Result<(), mcv_audio::AudioError> {
//! let audio = AudioManager::open(mcv_audio::default_sounds_dir())?; // 失败也可退回 AudioManager::silent()
//! // 方块破坏进度圈:每 tick 起一次循环,停止/完成时 loop_stop
//! audio.loop_start(SoundId::DigStone, 1.0);
//! audio.loop_stop(SoundId::DigStone);
//! // 一次性 3D 事件:脚步/落地/受伤,16 格线性衰减
//! audio.play_at(SoundId::StepGrass, [10.5, 64.0, -3.5], [10.0, 64.0, -4.0], 0.6);
//! audio.play_at(SoundId::DigStone, [10.0, 63.0, -3.0], [10.0, 64.0, -4.0], 1.0); // 放置(石质同素材)
//! // 无衰减 UI 音(经验球、升级)
//! audio.play_ui(SoundId::XpOrb, 0.8);
//! audio.set_master_gain(0.8);
//! # Ok(())
//! # }
//! ```
//!
//! ## 素材
//! 素材目录为顶层 `sounds/`(与 `texturepack/` 同等待遇:开发期 Mojang 原版资产,
//! 发布前删除)。[`SoundId`] 枚举与文件一一对应;素材缺失时所有播放调用都是 no-op
//! (仅首次告警),不会 panic。
pub mod decode;
pub mod loader;
pub mod mixer;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub use decode::{decode_to_stereo, SoundData};
pub use loader::{SoundLoader, CACHE_MAX, SAME_ID_MIN_INTERVAL};
pub use mixer::{Mixer, MAX_VOICES};

/// 3D 音效线性衰减半径(格)。距离 ≥ 该值时衰减为 0。与原版线性滚存一致。
pub const DEFAULT_ATTENUATION_RADIUS: f32 = 16.0;
/// 输出采样率(Hz)。素材(22050)由混音端线性插值重采样到此率。
pub const OUTPUT_SAMPLE_RATE: usize = 44100;
/// 输出声道数(固定立体声)。
pub const OUTPUT_CHANNELS: usize = 2;
/// 每个输出回调的每声道帧数(1024/44100 ≈ 23ms 延迟粒度)。
pub const OUTPUT_BUFFER_FRAMES: usize = 1024;

/// 一个可播放的音效 id,与 `sounds/` 下文件一一对应。
///
/// 文件名是 26.1 资源索引(index 30)内 ogg 资产路径的扁平化,`vanilla_path()`
/// 给出原始资产路径备查。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundId {
    /// 石头挖掘/放置 —— `dig_stone1.ogg`(原版 dig/stone1)。
    DigStone,
    /// 泥土/草地挖掘 —— `dig_grass1.ogg`。
    /// 注意:26.1 素材库**没有** dig/dirt1,原版泥土与草地共用 grass 音组。
    DigDirt,
    /// 木头挖掘/放置 —— `dig_wood1.ogg`。
    DigWood,
    /// 石头脚步 —— `step_stone1.ogg`。
    StepStone,
    /// 草地脚步 —— `step_grass1.ogg`。
    StepGrass,
    /// 木头脚步 —— `step_wood1.ogg`。
    StepWood,
    /// 弓箭发射 —— `random_bow.ogg`。
    BowShot,
    /// 经验球拾取(UI)—— `random_orb.ogg`。
    XpOrb,
    /// 玩家受伤(变体 1)—— `damage_hit1.ogg`。
    PlayerHurt1,
    /// 玩家受伤(变体 2)—— `damage_hit2.ogg`。
    PlayerHurt2,
    /// 通用实体受伤 —— `damage_hit3.ogg`。26.1 已无 entity/generic/hurt,
    /// 取同组第三变体作开发期占位。
    EntityHurt,
    /// 落地/摔落 —— `damage_fallsmall.ogg`(原版小fall damage即落地音)。
    LandFall,
}

impl SoundId {
    /// 全部变体(遍历用,测试与工具)。
    pub const ALL: &'static [SoundId] = &[
        SoundId::DigStone,
        SoundId::DigDirt,
        SoundId::DigWood,
        SoundId::StepStone,
        SoundId::StepGrass,
        SoundId::StepWood,
        SoundId::BowShot,
        SoundId::XpOrb,
        SoundId::PlayerHurt1,
        SoundId::PlayerHurt2,
        SoundId::EntityHurt,
        SoundId::LandFall,
    ];

    /// `sounds/` 目录内的文件名。
    pub fn file_name(self) -> &'static str {
        match self {
            SoundId::DigStone => "dig_stone1.ogg",
            SoundId::DigDirt => "dig_grass1.ogg",
            SoundId::DigWood => "dig_wood1.ogg",
            SoundId::StepStone => "step_stone1.ogg",
            SoundId::StepGrass => "step_grass1.ogg",
            SoundId::StepWood => "step_wood1.ogg",
            SoundId::BowShot => "random_bow.ogg",
            SoundId::XpOrb => "random_orb.ogg",
            SoundId::PlayerHurt1 => "damage_hit1.ogg",
            SoundId::PlayerHurt2 => "damage_hit2.ogg",
            SoundId::EntityHurt => "damage_hit3.ogg",
            SoundId::LandFall => "damage_fallsmall.ogg",
        }
    }

    /// 26.1 资源索引内的原始资产路径(相对 jar 虚拟 `assets/minecraft/`)。
    pub fn vanilla_path(self) -> &'static str {
        match self {
            SoundId::DigStone => "sounds/dig/stone1.ogg",
            SoundId::DigDirt => "sounds/dig/grass1.ogg",
            SoundId::DigWood => "sounds/dig/wood1.ogg",
            SoundId::StepStone => "sounds/step/stone1.ogg",
            SoundId::StepGrass => "sounds/step/grass1.ogg",
            SoundId::StepWood => "sounds/step/wood1.ogg",
            SoundId::BowShot => "sounds/random/bow.ogg",
            SoundId::XpOrb => "sounds/random/orb.ogg",
            SoundId::PlayerHurt1 => "sounds/damage/hit1.ogg",
            SoundId::PlayerHurt2 => "sounds/damage/hit2.ogg",
            SoundId::EntityHurt => "sounds/damage/hit3.ogg",
            SoundId::LandFall => "sounds/damage/fallsmall.ogg",
        }
    }
}

/// 音效系统错误(仅 `AudioManager::open` 会返回;播放路径一律 no-op)。
#[derive(Debug)]
pub enum AudioError {
    /// 素材文件读取失败。
    Io(std::io::Error),
    /// symphonia 探测/解码失败。
    Decode(String),
    /// 容器内没有可用音频轨。
    NoAudioTrack,
    /// 后端设备创建失败(无声卡/无权限等)。
    Backend(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Io(e) => write!(f, "音效文件 IO 错误: {}", e),
            AudioError::Decode(msg) => write!(f, "音效解码错误: {}", msg),
            AudioError::NoAudioTrack => write!(f, "音效容器内没有音频轨"),
            AudioError::Backend(msg) => write!(f, "音频后端错误: {}", msg),
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AudioError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AudioError {
    fn from(e: std::io::Error) -> Self {
        AudioError::Io(e)
    }
}

/// 欧氏距离(3D)。
pub fn distance3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// 线性距离衰减:近处 1.0,`dist >= radius` 处 0.0,中间线性过渡。
///
/// 边界约定:`dist <= 0`(含同点)→ 1.0;`dist` 为 NaN/inf 或非有限 `radius` → 0.0;
/// `radius <= 0` → 0.0(视作不发声,避免除零)。
pub fn distance_attenuation(dist: f32, radius: f32) -> f32 {
    if !dist.is_finite() || !radius.is_finite() {
        return 0.0;
    }
    if dist <= 0.0 {
        return 1.0;
    }
    if radius <= 0.0 || dist >= radius {
        return 0.0;
    }
    1.0 - dist / radius
}

/// 开发期默认素材目录:顶层 `sounds/`。
///
/// 解析顺序:环境变量 `MCV_SOUNDS_DIR` → 编译期 workspace 根下 `sounds/`
/// (dev 构建即 `/root/mcv-engine/sounds`)→ 运行时相对路径 `./sounds`。
/// 发布构建(Android)应由主控解包素材后用 [`AudioManager::open`] 传入实际路径。
pub fn default_sounds_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("MCV_SOUNDS_DIR") {
        return PathBuf::from(dir);
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sounds");
    if workspace.is_dir() {
        return workspace;
    }
    PathBuf::from("sounds")
}

/// 跨线程命令:游戏线程产出 → 音频回调线程消费(SPSC 环,非阻塞)。
#[derive(Debug)]
pub enum AudioCmd {
    /// 起声部:PCM 已解码;衰减/增益合法性/一次性节流已在游戏线程
    /// ([`SoundLoader`])判定完毕,循环幂等由音频线程 ([`Mixer::apply`])把关。
    Play {
        id: SoundId,
        sound: Arc<SoundData>,
        gain: f32,
        looping: bool,
    },
    /// 停止指定 id 的循环声部。
    StopLoop { id: SoundId },
}

/// 命令环容量(向上取 2 的幂):命令频率是人类操作级,256 绰绰有余;
/// 满时 push 退回、命令丢弃——实时线程永不阻塞。
const CMD_RING: usize = 256;

/// 音频管理器:后端设备线程 + 软件混音器。**应用生命周期内持有单例即可。**
///
/// 所有播放方法均为 `&self`、永不 panic;设备为 None(静音模式)时播放调用
/// 是廉价 no-op(零 IO)。游戏线程调用 `play_*`/`loop_*`/`set_master_gain`
/// 只经过无锁环与原子镜像,与音频回调线程之间不存在共享锁。
pub struct AudioManager {
    cmds: mcv_sync::Producer<AudioCmd>,
    /// 游戏线程侧素材解析;此锁只在游戏线程内部使用,永不与音频回调竞争。
    loader: Mutex<SoundLoader>,
    /// 主增益 f32 位镜像:`set_master_gain` 直写,回调 `mix` 每帧读。
    master: Arc<AtomicU32>,
    device: Option<tinyaudio::OutputDevice>,
}

impl AudioManager {
    /// 创建后端输出设备并启动混音回调。
    ///
    /// 无声卡环境(CI/无头)会返回 [`AudioError::Backend`],调用方应退回
    /// [`AudioManager::silent`](或直接忽略音效)。
    pub fn open(sounds_dir: impl Into<PathBuf>) -> Result<Self, AudioError> {
        let (cmds, cons) = mcv_sync::Spsc::new(CMD_RING).split();
        let master = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let mut mixer = Mixer::new(OUTPUT_SAMPLE_RATE as u32, cons, Arc::clone(&master));
        let device = tinyaudio::run_output_device(
            tinyaudio::OutputDeviceParameters {
                channels_count: OUTPUT_CHANNELS,
                sample_rate: OUTPUT_SAMPLE_RATE,
                channel_sample_count: OUTPUT_BUFFER_FRAMES,
            },
            move |data| {
                // data:交错立体声 f32,长度 = OUTPUT_BUFFER_FRAMES * OUTPUT_CHANNELS。
                // 回调线程独占 Mixer:排空命令环 + 混音,全程零锁零分配。
                mixer.mix(data);
            },
        )
        .map_err(|e| AudioError::Backend(e.to_string()))?;
        Ok(Self {
            cmds,
            loader: Mutex::new(SoundLoader::new(sounds_dir.into())),
            master,
            device: Some(device),
        })
    }

    /// 静音模式:不建后端设备,播放调用 no-op(连素材都不解析)。
    /// 适合无头测试或后端失败后的降级;`master_gain()` 仍可读。
    pub fn silent(sounds_dir: impl Into<PathBuf>) -> Self {
        // Consumer 就地丢弃:无消费者,而 silent 的播放方法直接 return,永不入环。
        let (cmds, _unused) = mcv_sync::Spsc::new(CMD_RING).split();
        Self {
            cmds,
            loader: Mutex::new(SoundLoader::new(sounds_dir.into())),
            master: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            device: None,
        }
    }

    /// 是否运行在静音模式。
    pub fn is_silent(&self) -> bool {
        self.device.is_none()
    }

    /// 3D 一次性播放:`pos` 处发声,`listener` 处收听,16 格线性衰减。
    ///
    /// `gain` 为额外增益(1.0 = 原音量)。素材缺失/被节流时静默 no-op。
    pub fn play_at(&self, id: SoundId, pos: [f32; 3], listener: [f32; 3], gain: f32) {
        if self.device.is_none() {
            return;
        }
        if let Ok(mut l) = self.loader.lock() {
            if let Some(cmd) = l.play_at(id, pos, listener, gain, Instant::now()) {
                let _ = self.cmds.push(cmd);
            }
        }
    }

    /// 2D(UI)播放:无距离衰减,用于经验球/升级/按钮音。
    pub fn play_ui(&self, id: SoundId, gain: f32) {
        if self.device.is_none() {
            return;
        }
        if let Ok(mut l) = self.loader.lock() {
            if let Some(cmd) = l.play_ui(id, gain, Instant::now()) {
                let _ = self.cmds.push(cmd);
            }
        }
    }

    /// 循环播放(挖掘持续音)。同一 id 重复调用幂等,不会叠加声部造成爆音
    /// (幂等判定在音频线程)。
    pub fn loop_start(&self, id: SoundId, gain: f32) {
        if self.device.is_none() {
            return;
        }
        if let Ok(mut l) = self.loader.lock() {
            if let Some(cmd) = l.loop_start(id, gain, Instant::now()) {
                let _ = self.cmds.push(cmd);
            }
        }
    }

    /// 停止指定 id 的循环(挖掘完成/中断/方块消失)。
    pub fn loop_stop(&self, id: SoundId) {
        if self.device.is_none() {
            return;
        }
        let _ = self.cmds.push(AudioCmd::StopLoop { id });
    }

    /// 主音量(音量滑条接线点)。推荐 [0.0, 1.0]。原子直写,无锁。
    pub fn set_master_gain(&self, gain: f32) {
        self.master.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// 当前主音量。
    pub fn master_gain(&self) -> f32 {
        f32::from_bits(self.master.load(Ordering::Relaxed))
    }
}

impl Drop for AudioManager {
    fn drop(&mut self) {
        if let Some(mut d) = self.device.take() {
            d.close();
        }
    }
}
