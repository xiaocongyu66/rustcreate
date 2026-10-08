//! Shared foundation: chunk layout, block registry, positions, task pool,
//! chunk state machine.

pub mod atlas;
pub mod chunk;
pub mod pool;

pub use chunk::dirty;
pub use chunk::{ChunkHandle, Stage};
pub use pool::{world_worker_count, TaskPool};

pub const CHUNK_SX: usize = 16;
pub const CHUNK_SY: usize = 256;
pub const CHUNK_SZ: usize = 16;
pub const CHUNK_VOL: usize = CHUNK_SX * CHUNK_SY * CHUNK_SZ; // 65536
pub const SEA_LEVEL: i32 = 96;

/// Chunk-local voxel index: `(y<<8) | (z<<4) | x`.
#[inline]
pub const fn vidx(x: usize, y: usize, z: usize) -> usize {
    debug_assert!(x < CHUNK_SX && y < CHUNK_SY && z < CHUNK_SZ);
    (y << 8) | (z << 4) | x
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn chunk(self) -> ChunkPos {
        ChunkPos::new(self.x.div_euclid(16), self.z.div_euclid(16))
    }

    pub const fn local(self) -> [usize; 3] {
        [
            self.x.rem_euclid(16) as usize,
            self.y.rem_euclid(256) as usize,
            self.z.rem_euclid(16) as usize,
        ]
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BlockId(pub u8);

pub const AIR: BlockId = BlockId(0);

/// Texture array layer indices (see mcv_core::atlas in M3).
pub mod tiles {
    pub const GRASS_TOP: u8 = 1;
    pub const GRASS_SIDE: u8 = 2;
    pub const DIRT: u8 = 3;
    pub const STONE: u8 = 4;
    pub const SAND: u8 = 5;
    pub const WATER: u8 = 6;
    pub const LOG_SIDE: u8 = 7;
    pub const LOG_TOP: u8 = 8;
    pub const LEAVES: u8 = 9;
    pub const PLANKS: u8 = 10;
    pub const COBBLE: u8 = 11;
    pub const BEDROCK: u8 = 12;
    pub const SNOW: u8 = 13;
    pub const SNOW_SIDE: u8 = 14;
    pub const FLOWER_RED: u8 = 15;
    pub const FLOWER_YELLOW: u8 = 16;
}

pub struct BlockDef {
    pub name: &'static str,
    pub solid: bool,
    pub opaque: bool,
    pub liquid: bool,
    pub light_emit: u8,
    /// Texture array layer per face: [+X, -X, +Y, -Y, +Z, -Z].
    pub tiles: [u8; 6],
    /// Seconds to mine; 0 = instant (creative).
    pub hardness: f32,
}

macro_rules! block {
    ($name:literal, solid: $solid:expr, opaque: $opaque:expr,
     liquid: $liquid:expr, emit: $emit:expr, tiles: $tiles:expr, hardness: $h:expr) => {
        BlockDef {
            name: $name,
            solid: $solid,
            opaque: $opaque,
            liquid: $liquid,
            light_emit: $emit,
            tiles: $tiles,
            hardness: $h,
        }
    };
}

/// 硬度与发光已对照反编译 Minecraft 26.1 `Blocks.java` 校准，
/// 数值来源与完整对照表见仓库外笔记 `/root/mc-ref/NOTES-blocks.md`。
/// 注意：MC 的 `strength(x)` 单参同时设硬度与抗爆值；`strength(-1)`（基岩）
/// 在此用 `f32::INFINITY` 表示不可挖；水按注册表原值 strength(100)。
pub static BLOCKS: [BlockDef; 14] = [
    block!("air", solid: false, opaque: false, liquid: false, emit: 0,
           tiles: [0; 6], hardness: 0.0),
    block!("stone", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::STONE; 6], hardness: 1.5),
    block!("dirt", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::DIRT; 6], hardness: 0.5),
    block!("grass", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::GRASS_SIDE, tiles::GRASS_SIDE, tiles::GRASS_TOP,
                   tiles::DIRT, tiles::GRASS_SIDE, tiles::GRASS_SIDE],
           hardness: 0.6),
    block!("sand", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::SAND; 6], hardness: 0.5),
    // MC 26.1 water: strength(100)（原版注册表原值；靠 liquid/replaceable
    // 判定不可获取，手挖 100 秒等同不可挖。原 0.0 会让水面被瞬间"挖掉"）
    block!("water", solid: false, opaque: false, liquid: true, emit: 0,
           tiles: [tiles::WATER; 6], hardness: 100.0),
    block!("log", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::LOG_SIDE, tiles::LOG_SIDE, tiles::LOG_TOP,
                   tiles::LOG_TOP, tiles::LOG_SIDE, tiles::LOG_SIDE],
           // MC 26.1 oak_log: strength(2.0)（原为 1.0）
           hardness: 2.0),
    block!("leaves", solid: true, opaque: false, liquid: false, emit: 0,
           tiles: [tiles::LEAVES; 6], hardness: 0.2),
    // MC 26.1 oak_planks: strength(2.0, 3.0)（原为 1.0）
    block!("planks", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::PLANKS; 6], hardness: 2.0),
    // MC 26.1 cobblestone: strength(2.0, 6.0)（原误用石头的 1.5）
    block!("cobble", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::COBBLE; 6], hardness: 2.0),
    // MC 26.1 bedrock: strength(-1, 3600000)→不可破坏,
    // 沿用本引擎现有不可挖表示法 f32::INFINITY（不引入新字段/新约定）
    block!("bedrock", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::BEDROCK; 6], hardness: f32::INFINITY),
    block!("snow_grass", solid: true, opaque: true, liquid: false, emit: 0,
           tiles: [tiles::SNOW_SIDE, tiles::SNOW_SIDE, tiles::SNOW,
                   tiles::DIRT, tiles::SNOW_SIDE, tiles::SNOW_SIDE],
           hardness: 0.6),
    block!("flower_red", solid: false, opaque: false, liquid: false, emit: 0,
           tiles: [tiles::FLOWER_RED; 6], hardness: 0.0),
    block!("flower_yellow", solid: false, opaque: false, liquid: false, emit: 0,
           tiles: [tiles::FLOWER_YELLOW; 6], hardness: 0.0),
];

impl BlockId {
    #[inline]
    pub fn def(self) -> &'static BlockDef {
        &BLOCKS[self.0 as usize]
    }
}

/// Chunk voxel storage owned by Rust; C++ borrows per call.
pub struct ChunkVoxels(pub Box<[BlockId; CHUNK_VOL]>);

/// Chunk light storage: low nibble = block light, high nibble = sky light.
pub struct ChunkLight(pub Box<[u8; CHUNK_VOL]>);

impl ChunkVoxels {
    pub fn filled(id: BlockId) -> Self {
        Self(Box::new([id; CHUNK_VOL]))
    }

    /// Raw byte view for FFI (BlockId is repr(transparent) over u8).
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.0.as_mut_ptr().cast(), CHUNK_VOL) }
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.as_ptr().cast(), CHUNK_VOL) }
    }
}

impl ChunkLight {
    pub fn zeroed() -> Self {
        Self(Box::new([0; CHUNK_VOL]))
    }
}
