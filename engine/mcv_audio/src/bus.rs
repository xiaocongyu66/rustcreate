//! 音频总线图:根 `master` → `music`/`sfx`/`ambient` 三轨的树 + send 语义
//! + 水下低通预留位。
//!
//! 树拓扑(单点真相是 [`BusId::parent`]/[`BusId::TRACKS`]):
//!
//! ```text
//! master(根:既有 master 增益原子,即 set_master_gain 的那个)
//! ├── music   音乐轨(唱片/游戏音乐;事件名 music* → Music)
//! ├── sfx     音效轨(方块/实体/UI 等;其余事件一律兜底落此轨)
//! └── ambient 环境轨(水环境/洞穴;事件名 ambient* → Ambient)
//! ```
//!
//! ## 增益级联(线性域,照抄 bevy_audio `Volume` 语义:线性 1.0 = 原音量)
//!
//! 声部有效增益 = `voice.gain × master × path(落轨→根)`,其中 `path` 是落轨
//! 与根之间所有轨增益的乘积(当前树深 1,即只有落轨自身)。**全默认(各轨
//! gain=1.0)时级联是 ×1.0 恒等——混音输出与无总线实现逐位相同**(IEEE754
//! 下 ×1.0 是精确恒等,见 `tests/bus.rs` 的等价性测试)。
//!
//! ## send 语义(后置 aux send)
//!
//! send 是"声部家庭路径(已乘 voice.gain×master×path(落轨))之后"按
//! `send 增益 × path(目标轨)` 旁路一份进目标轨;根 master 只乘一次(来源
//! 路径里已含,目标路径不再叠加)。存于 3×3 出向矩阵(BusMix::sends),
//! 0.0 = 未接;`from == to` 与涉及根的 send 一律 no-op。默认无人接 send,
//! 热路径上少一次乘法与一次累加,行为保持既有序。
//!
//! ## 水下低通效果位(预留)
//!
//! 每轨一个 `lowpass` 位(`BusMix::set_lowpass`)。**本波只存态不进 DSP**
//! ——混音输出与该位无关;水下滤波(下单边带 biquad)落地时直接读本位,
//! 位语义先钉死(默认 false = 不过滤,行为不变)。
//!
//! ## 线程模型
//!
//! 全部状态是 `Arc<BusMix>` 原子位镜像:游戏线程写、音频回调线程读,
//! **不经过命令环**——配置改写与既有 master 原子同模式,回调热路径零锁
//! 零分配(send 查询是固定步长数组读,无堆结构)。
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU32};

/// 总线标识:根 + 三轨。声部落轨(声音生来只在一个轨上);根只接收,
/// 不直接挂声部(既有的"无总线"声部等价于落根,见 `Mixer`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BusId {
    /// 根总线。增益即既有 master 原子(与 [`crate::AudioManager::set_master_gain`]
    /// 同一实例);"无声部直接落根"仅在等价性论证里出现(见 `Mixer` 文档)。
    Master,
    /// 音乐轨:唱片、游戏/菜单音乐。
    Music,
    /// 音效轨:方块/实体/物品/UI 等行为音。默认落轨(现有全部调用点都是
    /// 行为音,兜底语义不变)。
    #[default]
    Sfx,
    /// 环境轨:水环境(`ambient.underwater.*`)、洞穴氛围等。
    Ambient,
}

impl BusId {
    /// master 的子轨(树拓扑:三轨按声明序;完备性由 `tests/bus.rs` 锁定)。
    pub const TRACKS: [BusId; 3] = [BusId::Music, BusId::Sfx, BusId::Ambient];

    /// 轨名(诊断/日志用)。
    pub const fn name(self) -> &'static str {
        match self {
            BusId::Master => "master",
            BusId::Music => "music",
            BusId::Sfx => "sfx",
            BusId::Ambient => "ambient",
        }
    }

    /// 是否是可挂声部的轨(master 是根,只汇流)。
    pub const fn is_track(self) -> bool {
        !matches!(self, BusId::Master)
    }

    /// 三轨数组下标;根返回 `None`。
    pub const fn track_idx(self) -> Option<usize> {
        match self {
            BusId::Master => None,
            BusId::Music => Some(0),
            BusId::Sfx => Some(1),
            BusId::Ambient => Some(2),
        }
    }

    /// 树拓扑单点真相:父总线。三轨的父恒为根;根无父。
    pub const fn parent(self) -> Option<BusId> {
        match self {
            BusId::Master => None,
            BusId::Music | BusId::Sfx | BusId::Ambient => Some(BusId::Master),
        }
    }

    /// 按事件名分类落轨(**现调用点唯一接线点**——`SoundLoader::trigger` 调用;
    /// 游戏侧无字面量分支):
    /// - `music` 前缀(含 `music_disc.*` 唱片)→ [`BusId::Music`];
    /// - `ambient` 前缀(含 `ambient.underwater.*` 水环境)→ [`BusId::Ambient`];
    /// - 其余(`block.*`/`entity.*`/`ui.*`/`item.*`/…)→ [`BusId::Sfx`] 兜底
    ///   ——与"总线图之前全部直混"的行为等价面完全重合。
    pub fn for_event(event: &str) -> BusId {
        if event.starts_with("music") {
            BusId::Music
        } else if event.starts_with("ambient") {
            BusId::Ambient
        } else {
            BusId::Sfx
        }
    }
}

/// 总线图共享状态:根增益(既有 master 原子的同一 Arc)+ 三轨增益/低通位
/// + 出向 send 矩阵。游戏线程写、回调线程读,全原子位镜像(Relaxed:
/// 配置是"最终可见即可"的人耳级量,不需要 Acquire/Release 排序)。
pub struct BusMix {
    /// 根 master 增益 f32 位镜像。与 `AudioManager` 既有 master 原子是
    /// **同一实例**——`BusMix::set_gain(Master, g)` 与 `set_master_gain(g)`
    /// 完全等价(单点写,无双源)。
    master: Arc<AtomicU32>,
    /// 三轨增益 f32 位镜像(下标 = `BusId::track_idx`),默认 1.0。
    gains: [AtomicU32; 3],
    /// 三轨低通效果位(水下滤波预留;本波只存态,混音端不读)。
    lowpass: [AtomicBool; 3],
    /// 出向 send 矩阵 `[from][to]` 的 f32 位镜像(0.0 = 未接)。固定 3×3
    /// 数组——回调热路径固定步长读,零堆零分配。
    sends: [[AtomicU32; 3]; 3],
}

impl BusMix {
    /// 挂到既有根 master 原子上(与 `AudioManager::open/silent` 共用同一
    /// Arc;各轨 gain=1.0、低通位 false、无 send——全默认 = 逐位旧行为)。
    pub fn new(master: Arc<AtomicU32>) -> Self {
        Self {
            master,
            gains: [
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
            ],
            lowpass: [
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
            ],
            sends: [
                [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)],
                [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)],
                [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)],
            ],
        }
    }

    /// 读增益:轨读各自槽;根读既有 master 原子(与 `master_gain()` 同值)。
    pub fn gain(&self, bus: BusId) -> f32 {
        match bus.track_idx() {
            Some(i) => f32::from_bits(self.gains[i].load(Relaxed)),
            None => f32::from_bits(self.master.load(Relaxed)),
        }
    }

    /// 写增益。非有限值(NaN/±inf)一律 no-op——配置防线,不让坏值进级联。
    /// `Master` 即写根原子(与 [`crate::AudioManager::set_master_gain`] 等价)。
    pub fn set_gain(&self, bus: BusId, gain: f32) {
        if !gain.is_finite() {
            return;
        }
        match bus.track_idx() {
            Some(i) => self.gains[i].store(gain.to_bits(), Relaxed),
            None => self.master.store(gain.to_bits(), Relaxed),
        }
    }

    /// 读低通效果位。根无此位(恒 false——水下滤波只对轨)。
    pub fn lowpass(&self, bus: BusId) -> bool {
        match bus.track_idx() {
            Some(i) => self.lowpass[i].load(Relaxed),
            None => false,
        }
    }

    /// 写低通效果位。根 no-op。
    pub fn set_lowpass(&self, bus: BusId, on: bool) {
        if let Some(i) = bus.track_idx() {
            self.lowpass[i].store(on, Relaxed);
        }
    }

    /// 读 send 增益(未接 = 0.0;涉及根恒 0.0——send 只在轨间)。
    pub fn send_gain(&self, from: BusId, to: BusId) -> f32 {
        match (from.track_idx(), to.track_idx()) {
            (Some(fi), Some(ti)) => f32::from_bits(self.sends[fi][ti].load(Relaxed)),
            _ => 0.0,
        }
    }

    /// 接一条 send(`from` 轨旁路一份给 `to` 轨)。幂等覆盖同一条边;
    /// `gain = 0.0` 即撤线;`from == to`/涉及根/非有限值 no-op。
    pub fn set_send(&self, from: BusId, to: BusId, gain: f32) {
        if let (Some(fi), Some(ti)) = (from.track_idx(), to.track_idx()) {
            if fi == ti || !gain.is_finite() {
                return;
            }
            self.sends[fi][ti].store(gain.to_bits(), Relaxed);
        }
    }

    /// 撤 `from` 轨的全部出向 send(整行清零)。
    pub fn clear_sends(&self, from: BusId) {
        if let Some(fi) = from.track_idx() {
            for cell in &self.sends[fi] {
                cell.store(0.0f32.to_bits(), Relaxed);
            }
        }
    }

    /// 根路径增益积:`bus → 根` 链上各轨增益的乘积,**不含根**
    /// (根 master 由混音端作为既有因子单独乘一次,避免双乘)。
    /// 全默认时恒等于 1.0(×1.0 恒等是位级等价性的支点)。
    pub fn chain(&self, bus: BusId) -> f32 {
        let mut g = 1.0f32;
        let mut cur = bus;
        while let Some(i) = cur.track_idx() {
            g *= f32::from_bits(self.gains[i].load(Relaxed));
            cur = BusId::parent(cur).unwrap_or(BusId::Master);
        }
        g
    }
}
