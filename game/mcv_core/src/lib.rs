//! Shared foundation: chunk layout, block registry, positions, task pool,
//! chunk state machine.

pub mod atlas;
pub mod chunk;
pub mod pool;

pub use chunk::dirty;
pub use chunk::{ChunkHandle, Stage};
pub use pool::{TaskPool, world_worker_count};

pub const CHUNK_SX: usize = 16;
pub const CHUNK_SY: usize = 256;
pub const CHUNK_SZ: usize = 16;
pub const CHUNK_VOL: usize = CHUNK_SX * CHUNK_SY * CHUNK_SZ; // 65536
/// One chunk's voxel storage in bytes: BlockId is u16 since the block-id
/// widening (registry grows toward ~1000 blocks). 128 KiB per chunk.
pub const CHUNK_VOXEL_BYTES: usize = CHUNK_VOL * std::mem::size_of::<BlockId>(); // 131072
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
pub struct BlockId(pub u16);

const _: () = assert!(std::mem::size_of::<BlockId>() == 2);

pub const AIR: BlockId = BlockId(0);

/// Texture array layer indices into the real-texture atlas region
/// (layers 0..MANIFEST_LAYERS, 字典序 = tiles_manifest.json 索引)。
/// 值为 manifest 的真实层号；u16 以匹配 `BlockDef::tiles` / 网格器顶点
/// `tex_layer`。特殊程序化层（裂纹叠加等）在 827+，见 mcv_core::atlas。
pub mod tiles {
    pub const GRASS_TOP: u16 = 336; // grass_block_top
    pub const GRASS_SIDE: u16 = 333; // grass_block_side
    pub const DIRT: u16 = 274;
    pub const STONE: u16 = 707;
    pub const SAND: u16 = 645;
    pub const WATER: u16 = 789; // water_still（动画未实现，取静水帧）
    pub const LOG_SIDE: u16 = 490; // oak_log
    pub const LOG_TOP: u16 = 491; // oak_log_top
    pub const LEAVES: u16 = 489; // oak_leaves
    pub const PLANKS: u16 = 492; // oak_planks
    pub const COBBLE: u16 = 168; // cobblestone
    pub const BEDROCK: u16 = 41;
    pub const SNOW: u16 = 336; // 官方无独立雪层贴图：+Y 直接用 grass_block_top（snowy 状态）
    pub const SNOW_SIDE: u16 = 335; // grass_block_snow（雪覆盖侧）
    pub const FLOWER_RED: u16 = 574; // poppy
    pub const FLOWER_YELLOW: u16 = 230; // dandelion

    const _: () = assert!(super::atlas::MANIFEST_LAYERS as u16 > STONE);
}

#[derive(Clone, Copy)]
pub struct BlockDef {
    pub name: &'static str,
    pub solid: bool,
    pub opaque: bool,
    pub liquid: bool,
    pub light_emit: u8,
    /// Texture array layer per face: [+X, -X, +Y, -Y, +Z, -Z].
    pub tiles: [u16; 6],
    /// Seconds to mine; 0 = instant (creative).
    pub hardness: f32,
}

/// `blocks_gen.inc.rs` 中 GEN_BLOCKS 的元组类型（生成文件不导出别名，补一个）。
type GenBlock = (&'static str, bool, bool, bool, u8, [u16; 6], f32, u8);

// 千块表：1171 方块，id 0..13 与旧 14 方块表逐字段一致（回归锁见 tests），
// 14+ 按官方名字典序。格式/来源/限制见 blocks_gen.inc.rs 头部注释。
include!("blocks_gen.inc.rs");

/// 生成表元组 → BlockDef。model_kind（第 8 字段）是网格器未来字段，此处丢弃；
/// `f32::from_bits` 保证 const 路径与旧表字面量逐位一致（含 INFINITY）。
const fn gen_def(t: &GenBlock) -> BlockDef {
    BlockDef {
        name: t.0,
        solid: t.1,
        opaque: t.2,
        liquid: t.3,
        light_emit: t.4,
        tiles: t.5,
        hardness: f32::from_bits(t.6.to_bits()),
    }
}

const fn gen_blocks() -> [BlockDef; GEN_BLOCKS.len()] {
    // BlockDef: Copy → 数组重复式可用，const while 逐位覆盖零值
    let mut out = [BlockDef {
        name: "",
        solid: false,
        opaque: false,
        liquid: false,
        light_emit: 0,
        tiles: [0; 6],
        hardness: 0.0,
    }; GEN_BLOCKS.len()];
    let mut i = 0;
    while i < GEN_BLOCKS.len() {
        out[i] = gen_def(&GEN_BLOCKS[i]);
        i += 1;
    }
    out
}

/// 硬度与发光对照反编译 Minecraft 26.1 `Blocks.java`（数值来源见仓库外笔记
/// `/root/mc-ref/NOTES-blocks.md`）。id 0-13 为地形生成器/网格器硬编码依赖的
/// 旧 14 方块，字段与旧表逐字节一致；基岩不可挖用 `f32::INFINITY` 表示
/// （MC strength(-1)），水按注册表原值 strength(100)。
pub static BLOCKS: [BlockDef; GEN_BLOCKS.len()] = gen_blocks();

// ---------------------------------------------------------------------------
// 光照衰减表（lightDampening）
// ---------------------------------------------------------------------------

/// const 字符串比较：名字后缀（26.1 例外类按官方命名后缀识别，见 `OPACITY` 注释）。
const fn name_ends_with(name: &str, suf: &str) -> bool {
    let (n, s) = (name.as_bytes(), suf.as_bytes());
    if s.len() > n.len() {
        return false;
    }
    let mut i = 0usize;
    while i < s.len() {
        if n[n.len() - s.len() + i] != s[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// 单块 lightDampening 判定（三段规则 + 例外覆盖，Java 依据见 [`OPACITY`]）。
const fn gen_opacity(t: &GenBlock) -> u8 {
    let (name, _solid, opaque, liquid, _emit, _tiles, _hard, model_kind) = t;
    // ① solidRender（整方块不透明渲染）→ 15。表内 opaque=true 即"全不透明
    //    整方块"，对应原版 solidRender=满块 occlusionShape
    //    （BlockBehaviour.java:512-513, 305-310）。
    if *opaque {
        return 15;
    }
    // ② 流体（水/岩浆）：propagatesSkylightDown=false → damp 1
    //    （LiquidBlock.java:114-116；原版 fluid 非空使默认判据为 false，
    //    BlockBehaviour.java:395-397）。
    if *liquid {
        return 1;
    }
    if name_eq(name, "air") {
        return 0;
    }
    // ③ 例外覆盖（对照 src-26.1 逐类核实，注意 tinted_glass 是 15 不是 1）：
    //    -  tinted_glass = 15（TintedGlassBlock.java:25-27 直接覆写 damp=15）；
    if name_eq(name, "tinted_glass") {
        return 15;
    }
    //    - 叶 = 1（LeavesBlock.java:84-86 覆写；oak/acacia/azalea… 全部
    //      *_leaves 后缀）；
    if name_eq(name, "leaves") || name_ends_with(name, "_leaves") {
        return 1;
    }
    //    - 玻璃/染色玻璃/铜格栅 = 0：TransparentBlock 覆写
    //      propagatesSkylightDown=true（TransparentBlock.java:34-37；玻璃
    //      Blocks.java:505-507 直接是 TransparentBlock::new，染色玻璃
    //      StainedGlassBlock.java:8 继承，铜格栅 Blocks.java:5359 的
    //      WeatheringCopperGrateBlock 继承 WaterloggedTransparentBlock）。
    if name_eq(name, "glass")
        || name_ends_with(name, "_stained_glass")
        || name_ends_with(name, "copper_grate")
    {
        return 0;
    }
    //    -  潜影盒/粘液块/蜂蜜块/紫颂植株 = 1：表内 model_kind=1（渲染非纯
    //      立方）但原版形状是整方块，且 noOcclusion 使 solidRender=false，
    //      propagatesSkylightDown=false → damp 1（截断源柱，15→14 逐格衰减，
    //      不能按规则④给 0）。依据：潜影盒 ShulkerBoxBlock.java:159-161 覆写
    //      false + Blocks.java:5948-5954（noOcclusion，getShape=Shapes.block()）；
    //      粘液块/蜂蜜块 Blocks.java:2562/4913（noOcclusion，形状取默认整方块，
    //      HoneyBlock 仅覆写 getCollisionShape）；紫颂植株 PipeBlock.java:59-61
    //      覆写 false（chorus_plant 属 PipeBlock 系）。
    if name_eq(name, "shulker_box")
        || name_ends_with(name, "_shulker_box")
        || name_eq(name, "slime_block")
        || name_eq(name, "honey_block")
        || name_eq(name, "chorus_plant")
    {
        return 1;
    }
    //    -  屏障/光源 = 0：形状整方块但 noOcclusion + propagatesSkylightDown
    //      覆写 true（BarrierBlock.java:40-42、LightBlock.java:71-73；
    //      Blocks.java:2570/2583 noOcclusion），规则④（model_kind=1→0）
    //      已给 0，此处仅登记依据。
    // ④ 默认规则（BlockBehaviour.java:305-310 + 395-397）：非整方块形状
    //    （model_kind=1：板/梯/栅栏/花/火把/栅栏门…）→ propagatesSkylightDown
    //    =true → 0；整方块形状但非不透明（冰族 ice/packed_ice/blue_ice/
    //    frosted_ice——noOcclusion 见 Blocks.java:1623）→ false → 1。
    if *model_kind == 1 { 0 } else { 1 }
}

const fn name_eq(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    if x.len() != y.len() {
        return false;
    }
    let mut i = 0usize;
    while i < x.len() {
        if x[i] != y[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// 全 1171 块 lightDampening 静态表（对照反编译 Minecraft 26.1；审计 C2：
/// 旧 mcv_light 仅 6 个 legacy id 有值，其余 1165 块一律按 15 全挡）。
/// 规则出处 `BlockBehaviour.getLightDampening`（BlockBehaviour.java:305-310）：
/// `solidRender ? 15 : (propagatesSkylightDown ? 0 : 1)`，例外覆盖见
/// [`gen_opacity`] 内逐条 file:line。语义要点：
/// - 实心整方块 15；水/叶/冰 1；玻璃/花/火把/板/梯/栅栏/格栅 0；
/// - tinted_glass 例外 15（TintedGlassBlock.java:25-27）；
/// - 本表只覆盖注册表；越界 id 由 `mcv_light::opacity` 保守按 15。
///
/// 消费方：`mcv_light::opacity`（传播代价 `max(1, damp)`，
/// LightEngine.java:77-79 语义）与天光源柱截断（column_top 判据，
/// ChunkSkyLightSources.java:140-148）。
pub static OPACITY: [u8; GEN_BLOCKS.len()] = {
    let mut t = [15u8; GEN_BLOCKS.len()];
    let mut i = 0usize;
    while i < GEN_BLOCKS.len() {
        t[i] = gen_opacity(&GEN_BLOCKS[i]);
        i += 1;
    }
    t
};

impl BlockId {
    #[inline]
    pub fn def(self) -> &'static BlockDef {
        &BLOCKS[self.0 as usize]
    }
}

/// Chunk voxel storage owned by Rust; C++ borrows per call.
///
/// 内存预算（u16 加宽后）：体素 128 KiB/区块 + 光照 64 KiB + 高度图 256 B。
/// 视距 8（17×17 = 289 区块）≈ 289 × 192 KiB ≈ 54 MiB 体素+光照常驻。
/// mesh 池（CxxMesher 256 MiB）只存网格不存体素，预算不变。
pub struct ChunkVoxels(pub Box<[BlockId; CHUNK_VOL]>);

/// Chunk light storage: low nibble = block light, high nibble = sky light.
pub struct ChunkLight(pub Box<[u8; CHUNK_VOL]>);

impl ChunkVoxels {
    pub fn filled(id: BlockId) -> Self {
        Self(Box::new([id; CHUNK_VOL]))
    }

    /// Raw u16 id view for FFI (BlockId is repr(transparent) over u16).
    pub fn as_u16_slice_mut(&mut self) -> &mut [u16] {
        bytemuck::cast_slice_mut(self.0.as_mut_slice())
    }

    pub fn as_u16_slice(&self) -> &[u16] {
        bytemuck::cast_slice(self.0.as_slice())
    }
}

impl ChunkLight {
    pub fn zeroed() -> Self {
        Self(Box::new([0; CHUNK_VOL]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// u16 加宽锁定：id 必须 2 字节，区块体素存储必须 128 KiB。
    #[test]
    fn block_id_widened_to_u16() {
        assert!(
            std::mem::size_of::<BlockId>() == 2 && CHUNK_VOXEL_BYTES == 131072,
            "size_of::<BlockId>()={}, CHUNK_VOXEL_BYTES={}",
            std::mem::size_of::<BlockId>(),
            CHUNK_VOXEL_BYTES
        );
    }

    /// 旧 14 方块表（千块表接入前 `BLOCKS` 的原样，含旧 tiles 常量层号）。
    /// 只保留作回归锁：地形生成器/C++ 网格器硬编码依赖 id 0-13 的行为。
    static LEGACY_BLOCKS: [BlockDef; 14] = [
        BlockDef {
            name: "air",
            solid: false,
            opaque: false,
            liquid: false,
            light_emit: 0,
            tiles: [0; 6],
            hardness: 0.0,
        },
        BlockDef {
            name: "stone",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [4; 6],
            hardness: 1.5,
        },
        BlockDef {
            name: "dirt",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [3; 6],
            hardness: 0.5,
        },
        BlockDef {
            name: "grass",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [2, 2, 1, 3, 2, 2],
            hardness: 0.6,
        },
        BlockDef {
            name: "sand",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [5; 6],
            hardness: 0.5,
        },
        BlockDef {
            name: "water",
            solid: false,
            opaque: false,
            liquid: true,
            light_emit: 0,
            tiles: [6; 6],
            hardness: 100.0,
        },
        BlockDef {
            name: "log",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [7, 7, 8, 8, 7, 7],
            hardness: 2.0,
        },
        BlockDef {
            name: "leaves",
            solid: true,
            opaque: false,
            liquid: false,
            light_emit: 0,
            tiles: [9; 6],
            hardness: 0.2,
        },
        BlockDef {
            name: "planks",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [10; 6],
            hardness: 2.0,
        },
        BlockDef {
            name: "cobble",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [11; 6],
            hardness: 2.0,
        },
        BlockDef {
            name: "bedrock",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [12; 6],
            hardness: f32::INFINITY,
        },
        BlockDef {
            name: "snow_grass",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [14, 14, 13, 3, 14, 14],
            hardness: 0.6,
        },
        BlockDef {
            name: "flower_red",
            solid: false,
            opaque: false,
            liquid: false,
            light_emit: 0,
            tiles: [15; 6],
            hardness: 0.0,
        },
        BlockDef {
            name: "flower_yellow",
            solid: false,
            opaque: false,
            liquid: false,
            light_emit: 0,
            tiles: [16; 6],
            hardness: 0.0,
        },
    ];

    /// 回归锁：GEN_BLOCKS 前 14 项必须与旧表逐字段一致（name/三标志/发光/
    /// solid/hardness 用 to_bits 逐位比较；tiles 允许换新图集层号，但面间
    /// 拓扑必须同构——同面同图 ↔ 旧表同面同图）。不一致 = 地形生成器行为漂移。
    #[test]
    fn first_14_match_legacy_table() {
        assert_eq!(BLOCKS.len(), GEN_BLOCKS.len());
        for i in 0..LEGACY_BLOCKS.len() {
            let (n, o) = (&BLOCKS[i], &LEGACY_BLOCKS[i]);
            assert_eq!(n.name, o.name, "id {i} 名字");
            assert_eq!(
                (n.solid, n.opaque, n.liquid),
                (o.solid, o.opaque, o.liquid),
                "id {i} 标志"
            );
            assert_eq!(n.light_emit, o.light_emit, "id {i} 发光");
            assert_eq!(
                n.hardness.to_bits(),
                o.hardness.to_bits(),
                "id {i} {} 硬度逐位不一致: 新 {} vs 旧 {}",
                n.name,
                n.hardness,
                o.hardness
            );
            for f in 0..6 {
                // 新层号 = 旧层号 当且仅当 旧层号在新表同面复用（贴图名换层号，拓扑不变）
                for g in 0..6 {
                    assert_eq!(
                        n.tiles[f] == n.tiles[g],
                        o.tiles[f] == o.tiles[g],
                        "id {i} {} 面 {f}/{g} 贴图复用拓扑改变",
                        n.name
                    );
                }
                if o.tiles[f] != 0 {
                    assert_ne!(n.tiles[f], 0, "id {i} {} 面 {f} 不应退化为占位层", n.name);
                }
            }
        }
    }

    /// 千块表完整性：id < 1171（u16 索引安全）由长度断言；所有 tiles 层号
    /// 必须落在真实贴图区（< MANIFEST_LAYERS+特殊层，且 ≠ 特殊层区间）。
    #[test]
    fn gen_table_layer_indices_in_real_region() {
        for (i, d) in BLOCKS.iter().enumerate() {
            for &t in &d.tiles {
                assert!(
                    (t as usize) < atlas::MANIFEST_LAYERS,
                    "id {i} {} 层号 {t} 越出真实贴图区",
                    d.name
                );
            }
        }
    }
}
