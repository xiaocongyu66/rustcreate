//! Chunk storage and the chunk pipeline state machine.
//!
//! Stages advance one way; corrections only flip dirty bits.
//! 实际推进路径（全仓 `advance_to` 唯一三处，mcv_logic game.rs）：
//! `Empty -> TerrainReady -> LightLocalReady -> Uploaded`。
//! `Lit`/`MeshReady` 是预留中间态（跨区块边光同步、网格编译完成的
//! 记账点），当前流水线从不推进它们——比较「光照/网格完成度」请用
//! `>= LightLocalReady` / `== Uploaded`，勿按六态全链写谓词
//! （审计 stream-arch §2 / edit-mesh-staleness C3）。
//!
//! 与 stage 正交的还有 [`ChunkStatus`]（原版 12 态装载阶梯的类型镜像，
//! 本波只接线 Empty→Surface→Light；加载门仍读 stage——解耦在刀 3）。

use std::sync::RwLock;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::{BlockId, CHUNK_SY, CHUNK_VOL, ChunkPos, WORLD_MIN_Y};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Stage {
    Empty = 0,
    TerrainReady = 1,
    LightLocalReady = 2,
    /// 预留：跨区块边光同步完成（现由 border_synced 记账，不推进本态）。
    Lit = 3,
    /// 预留：网格编译完成（现直接推进 [`Self::Uploaded`]，不经过本态）。
    MeshReady = 4,
    Uploaded = 5,
}

pub mod dirty {
    pub const LIGHT: u8 = 1;
    pub const MESH: u8 = 2;
    pub const SAVE: u8 = 4;
}

/// 原版 12 态装载阶梯的类型镜像（26.1 ChunkStatus.java:21-32 的
/// EMPTY→STRUCTURE_STARTS→STRUCTURE_REFERENCES→BIOMES→NOISE→SURFACE→
/// CARVERS→FEATURES→INITIALIZE_LIGHT→LIGHT→SPAWN→FULL 顺序，:39-43
/// register(parent,…) 单亲线性链；前 9 态 PROTOCHUNK，FULL 才
/// LEVELCHUNK）。与 [`Stage`] **正交**：stage 是渲染/加载记账位（语义
/// 不动），status 是 worldgen 完成度阶梯；二者各自单调、互不推进。
///
/// 本波接线（同点同条件推进，行为零变化）：`Empty→Surface`（worldgen
/// commit）→`Light`（块内布光）；`Full` 刀 3 接 ticket 驱动，`Spawn`
/// 不接线（出生门仍在 stage 侧）。
///
/// 12→6 裁剪映射（显式登记，禁止沉默近似）：
/// - stage `Empty` = `Empty`（EMPTY）；
/// - stage `TerrainReady` = `Surface`（NOISE+SURFACE 合并——本仓
///   worldgen 一次产出，透传先例 ChunkStep/ChunkPyramid；
///   STRUCTURE_STARTS/STRUCTURE_REFERENCES 仅类型占位不接线，BIOMES
///   随 noise 一次产出，CARVERS/FEATURES 本仓无对应 pass）；
/// - stage `LightLocalReady` = `Light`（原版 LIGHT 拆两档之一：块内
///   布光完成。原版 LIGHT 蕴含跨块边一致，本仓第二档——跨块边同步——
///   在 stage 侧 `border_synced`/预留态 Lit 记账，不另立 status 档；
///   **债**：`status()==Light` 不蕴含边已同步，刀 3 加载门解耦前勿把
///   status 当就绪判据）；
/// - stage `MeshReady`/`Uploaded` 是渲染记账，不占 status 档（stage 侧）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum ChunkStatus {
    #[default]
    Empty = 0,
    StructureStarts = 1,
    StructureReferences = 2,
    Biomes = 3,
    Noise = 4,
    Surface = 5,
    Carvers = 6,
    Features = 7,
    InitializeLight = 8,
    Light = 9,
    Spawn = 10,
    /// 预留：刀 3 ticket 驱动时启用（原版 LEVELCHUNK 终态）。
    Full = 11,
}

impl ChunkStatus {
    /// 宽容解码：越界值归 `Empty`（与 [`ChunkHandle::stage`] 的匹配同形）。
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::StructureStarts,
            2 => Self::StructureReferences,
            3 => Self::Biomes,
            4 => Self::Noise,
            5 => Self::Surface,
            6 => Self::Carvers,
            7 => Self::Features,
            8 => Self::InitializeLight,
            9 => Self::Light,
            10 => Self::Spawn,
            11 => Self::Full,
            _ => Self::Empty,
        }
    }
}

/// 原版 section 竖直切面 16³（LevelChunkSection.java:13-15）。
pub const SECTION_VOL: usize = 16 * 16 * 16; // 4096
/// 每块 section 数 = CHUNK_SY / 16 = 24（LevelHeightAccessor
/// getSectionsCount()；v6 几何 384）。
pub const SECTION_COUNT: usize = CHUNK_SY / 16; // 24

/// 逻辑 section 切片视图：第 `sy`（0..[`SECTION_COUNT`]）个 16³ 竖直段的
/// 借切片，数据仍整块平铺（无拷贝）。平铺布局（crate 根 `vidx`）的低
/// 12 位恰为 section 内索引 `(yl<<8)|(z<<4)|x`（yl = 绝对 y −
/// WORLD_MIN_Y − sy*16），故对任意 sy/x/yl/z 恒有
/// `section_view(v, sy)[(yl<<8)|(z<<4)|x] == v[vidx(x, sy*16+yl+WORLD_MIN_Y, z)]`。
/// 本波只加 API+单测，不接入现有调用点（刀 3 候选）。
///
/// # Panics
/// `sy >= SECTION_COUNT` 时 panic（防越界静默回绕，coords 审计 P2 同则）。
pub fn section_view(voxels: &[BlockId; CHUNK_VOL], sy: usize) -> &[BlockId; SECTION_VOL] {
    assert!(
        sy < SECTION_COUNT,
        "section index {sy} out of 0..{SECTION_COUNT}"
    );
    let start = sy * SECTION_VOL;
    voxels[start..start + SECTION_VOL]
        .try_into()
        .expect("SECTION_VOL-length slice")
}

/// Shared per-chunk state. Workers commit owned buffers; readers lock
/// narrowly. The stage counter is monotonic (see [`Stage`]); the status
/// counter is a separate monotonic ladder (see [`ChunkStatus`]) — the two
/// advance independently (正交，互不推进对方).
pub struct ChunkHandle {
    pub pos: ChunkPos,
    stage: AtomicU8,
    /// worldgen 完成度阶梯（[`ChunkStatus`]），与 stage 正交的独立记账位。
    status: AtomicU8,
    dirty: AtomicU8,
    pub voxels: RwLock<Box<[BlockId; CHUNK_VOL]>>,
    /// 地表高度图，索引 `(z<<4)|x`；值 = 首个非遮光格**绝对 y**
    /// （v6：i16 承载 -64..320；全空列 = WORLD_MIN_Y 哨兵，见
    /// `mcv_worldgen::recompute_heightmap`）。
    pub heightmap: RwLock<Box<[i16; 256]>>,
    pub light: RwLock<Box<[u8; CHUNK_VOL]>>,
}

impl ChunkHandle {
    pub fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            stage: AtomicU8::new(Stage::Empty as u8),
            status: AtomicU8::new(ChunkStatus::Empty as u8),
            dirty: AtomicU8::new(0),
            voxels: RwLock::new(Box::new([BlockId(0); CHUNK_VOL])),
            heightmap: RwLock::new(Box::new([WORLD_MIN_Y as i16; 256])),
            light: RwLock::new(Box::new([0; CHUNK_VOL])),
        }
    }

    pub fn stage(&self) -> Stage {
        match self.stage.load(Ordering::Acquire) {
            1 => Stage::TerrainReady,
            2 => Stage::LightLocalReady,
            3 => Stage::Lit,
            4 => Stage::MeshReady,
            5 => Stage::Uploaded,
            _ => Stage::Empty,
        }
    }

    /// Sets the stage if it is an advance (never regresses).
    pub fn advance_to(&self, target: Stage) {
        let t = target as u8;
        let mut cur = self.stage.load(Ordering::Acquire);
        while cur < t {
            match self
                .stage
                .compare_exchange_weak(cur, t, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
    }

    /// 当前 worldgen 完成度阶梯位（[`ChunkStatus`]；与 [`Self::stage`]
    /// 正交——本方法只读 status，不触 stage）。
    pub fn status(&self) -> ChunkStatus {
        ChunkStatus::from_u8(self.status.load(Ordering::Acquire))
    }

    /// 推进 status（只进不退；CAS 形状照抄 [`Self::advance_to`]，允许
    /// 跨档直进——合并档先例见 [`ChunkStatus`] 裁剪映射）。
    pub fn advance_status(&self, target: ChunkStatus) {
        let t = target as u8;
        let mut cur = self.status.load(Ordering::Acquire);
        while cur < t {
            match self
                .status
                .compare_exchange_weak(cur, t, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
    }

    pub fn reset(&self) {
        self.stage.store(Stage::Empty as u8, Ordering::Release);
        self.status
            .store(ChunkStatus::Empty as u8, Ordering::Release);
        self.dirty.store(0, Ordering::Release);
    }

    pub fn dirty(&self) -> u8 {
        self.dirty.load(Ordering::Acquire)
    }

    pub fn mark_dirty(&self, bits: u8) {
        self.dirty.fetch_or(bits, Ordering::AcqRel);
    }

    pub fn clear_dirty(&self, bits: u8) {
        self.dirty.fetch_and(!bits, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WORLD_MAX_Y, vidx};

    /// status 阶梯单调：只进不退，回退/同级推进是 no-op（CAS 模式同
    /// `advance_to`，且允许跨档直进——合并档先例见 [`ChunkStatus`] 文档）。
    #[test]
    fn status_ladder_is_monotonic() {
        let h = ChunkHandle::new(ChunkPos::new(0, 0));
        assert_eq!(h.status(), ChunkStatus::Empty);
        h.advance_status(ChunkStatus::Surface);
        assert_eq!(h.status(), ChunkStatus::Surface);
        // 回退 no-op：更低档不生效。
        h.advance_status(ChunkStatus::StructureStarts);
        assert_eq!(h.status(), ChunkStatus::Surface);
        // 同级 no-op。
        h.advance_status(ChunkStatus::Surface);
        assert_eq!(h.status(), ChunkStatus::Surface);
        // 跨档直进：Surface 一步到 Full（中间档合并不拦路）。
        h.advance_status(ChunkStatus::Full);
        assert_eq!(h.status(), ChunkStatus::Full);
        h.advance_status(ChunkStatus::Light);
        assert_eq!(h.status(), ChunkStatus::Full);
    }

    /// stage 与 status 正交：各自 advance 互不影响，reset 双双归零。
    #[test]
    fn status_and_stage_are_orthogonal() {
        let h = ChunkHandle::new(ChunkPos::new(1, 2));
        h.advance_status(ChunkStatus::Light);
        assert_eq!(h.status(), ChunkStatus::Light);
        assert_eq!(h.stage(), Stage::Empty);
        h.advance_to(Stage::Uploaded);
        assert_eq!(h.stage(), Stage::Uploaded);
        assert_eq!(h.status(), ChunkStatus::Light);
        // status 侧回退 no-op 不波及 stage。
        h.advance_status(ChunkStatus::Empty);
        assert_eq!(h.status(), ChunkStatus::Light);
        assert_eq!(h.stage(), Stage::Uploaded);
        h.reset();
        assert_eq!(h.stage(), Stage::Empty);
        assert_eq!(h.status(), ChunkStatus::Empty);
    }

    /// section_view 与平铺 vidx 逐字节一致：vidx 低 12 位 = section 内
    /// 索引（全 section × 全 4096 槽位 × 确定性伪随机填充双重核对）。
    #[test]
    fn section_view_matches_flat_vidx_byte_for_byte() {
        let mut voxels = Box::new([BlockId(0); CHUNK_VOL]);
        let mut x: u32 = 0x9E37_79B9;
        for v in voxels.iter_mut() {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *v = BlockId((x >> 16) as u16);
        }
        for sy in 0..SECTION_COUNT {
            let sec = section_view(&voxels, sy);
            for z in 0..16usize {
                for x in 0..16usize {
                    for yl in 0..16usize {
                        let y = WORLD_MIN_Y + (sy * 16 + yl) as i32;
                        assert_eq!(
                            sec[(yl << 8) | (z << 4) | x],
                            voxels[vidx(x, y, z)],
                            "sy={sy} x={x} yl={yl} z={z}"
                        );
                    }
                }
            }
        }
        // 端界 section 显式核对（0 与 SECTION_COUNT-1）。
        assert_eq!(
            section_view(&voxels, SECTION_COUNT - 1)[16 * 256 - 1],
            voxels[vidx(15, WORLD_MAX_Y - 1, 15)]
        );
        assert_eq!(section_view(&voxels, 0)[0], voxels[vidx(0, WORLD_MIN_Y, 0)]);
    }
}
