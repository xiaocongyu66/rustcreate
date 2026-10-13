//! Shared foundation: chunk layout, block registry, positions, task pool,
//! chunk state machine.

pub mod atlas;
pub mod chunk;
pub mod pool;
pub mod rand;
pub mod shape;
pub mod tint;
pub mod tool;
pub mod voxel_store;

pub use chunk::dirty;
pub use chunk::{ChunkHandle, ChunkStatus, SECTION_COUNT, SECTION_VOL, Stage, section_view};
pub use pool::{TaskPool, world_worker_count};
pub use shape::Shape;
pub use voxel_store::{VoxelSection, VoxelStore};

pub const CHUNK_SX: usize = 16;
pub const CHUNK_SY: usize = 384;
pub const CHUNK_SZ: usize = 16;
pub const CHUNK_VOL: usize = CHUNK_SX * CHUNK_SY * CHUNK_SZ; // 98304
/// One chunk's voxel storage in bytes: BlockId is u16 since the block-id
/// widening (registry grows toward ~1000 blocks). 192 KiB per chunk.
pub const CHUNK_VOXEL_BYTES: usize = CHUNK_VOL * std::mem::size_of::<BlockId>(); // 196608
/// 世界竖直下界（绝对 y；26.1 dimension_type/overworld.json:44
/// `min_y: -64`——v6 几何升原版后，全仓方块/实体/heightmap 的 y 一律为
/// 绝对世界 y，本常量为唯一定义源）。
pub const WORLD_MIN_Y: i32 = -64;
/// 世界竖直上界（不含；= WORLD_MIN_Y + CHUNK_SY，overworld.json:47
/// `height: 384`）。
pub const WORLD_MAX_Y: i32 = WORLD_MIN_Y + CHUNK_SY as i32; // 320
/// 海平面（绝对 y；26.1 noise_settings/overworld.json:392
/// `sea_level: 63`——v6 起为原生值，旧 96 是 bloomcraft 256 基准自选）。
pub const SEA_LEVEL: i32 = 63;

/// Chunk-local voxel index: `((y - WORLD_MIN_Y) << 8) | (z << 4) | x`。
/// `y` 为**绝对**世界 y ∈ [WORLD_MIN_Y, WORLD_MAX_Y)，`x`/`z` 为区块内
/// 局部坐标（0..16）。入口断言防越界回绕（coords 审计 P2）。
#[inline]
pub const fn vidx(x: usize, y: i32, z: usize) -> usize {
    // 稳定版 const fn 里 Range::contains 是 conditionally-const（E0658）
    // ——等价展开为裸比较。
    debug_assert!(x < CHUNK_SX && y >= WORLD_MIN_Y && y < WORLD_MAX_Y && z < CHUNK_SZ);
    (((y - WORLD_MIN_Y) as usize) << 8) | (z << 4) | x
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

    /// 区块内局部索引（存储槽位，y_local = y − WORLD_MIN_Y ∈ 0..CHUNK_SY）。
    /// 界外 y 产出越界槽位（索引即 panic，不再 rem_euclid 回绕成「另一端
    /// 幽灵方块」，coords 审计 P2）——调用方必须先做 [WORLD_MIN_Y,
    /// WORLD_MAX_Y) 门（mcv_logic WorldView::block 即此判据）。
    pub const fn local(self) -> [usize; 3] {
        [
            self.x.rem_euclid(16) as usize,
            (self.y - WORLD_MIN_Y) as usize,
            self.z.rem_euclid(16) as usize,
        ]
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BlockId(pub u16);

const _: () = assert!(std::mem::size_of::<BlockId>() == 2);

pub const AIR: BlockId = BlockId(0);

/// 体素 u16 低位掩码：bit0-11 = 方块 id（≤4095），bit12-15 = 状态 nibble
/// （见 `BlockId::with_state`）。kBarrier(0xFFFF) 等哨兵值掩码后为 4095
/// （未注册 id，C++ 侧按未知=不透明处理），语义不变。
pub const ID_MASK: u16 = 0x0FFF;
pub const STATE_SHIFT: u32 = 12;

/// Texture array layer indices into the real-texture atlas region
/// (layers 0..MANIFEST_LAYERS, 字典序 = tiles_manifest.json 索引)。
/// 值为 manifest 的真实层号；u16 以匹配 `BlockDef::tiles` / 网格器顶点
/// `tex_layer`。特殊层（missing 哨兵 827、裂纹叠加 828+）见 mcv_core::atlas。
pub mod tiles {
    pub const GRASS_TOP: u16 = 336; // grass_block_top
    pub const GRASS_SIDE: u16 = 333; // grass_block_side
    pub const DIRT: u16 = 274;
    pub const STONE: u16 = 707;
    pub const SAND: u16 = 645;
    // water_still 初始帧（帧 0）——#125 起 atlas.rs 按原版帧语义精确裁取
    // （旧注释「取静水帧」在混采时代失实，现为真）；逐帧播放属渲染侧二期，
    // 帧序/frametime/interpolate 数据在 tiles_manifest.json animation 节。
    pub const WATER: u16 = 789;
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
    /// 形状编号（[`shape::Shape`] 判别值），按注册名派生，见 `shape.rs`。
    pub shape: u8,
    /// 掉落门属性（GEN_BLOCKS 第 9 列）：26.1
    /// `Properties.requiresCorrectToolForDrops()`（BlockBehaviour.java:1213
    /// 置位；ofFullCopy/ofLegacyCopy 经 :1054 逐字段复制继承）。原版消费：
    /// `Player.java:619` `!requiresCorrectToolForDrops() || 手持工具
    /// Tool.isCorrectForDrops(state)`。提取与继承闭包见 ci/gen-blocks.py。
    pub requires_tool: bool,
}

/// 旧 14 冻结行的行序常量（id 0..13 地形生成器硬依赖，永不重排）。
pub const GRASS_ROW: u16 = 3;
/// snow_grass = 官方 grass_block[snowy=true] 在本引擎的单方块独立行。
pub const SNOW_GRASS_ROW: u16 = 11;

/// 引擎行 id → 原版 tag 行 id：snow_grass(11) 的官方身份是 grass(3) 行的
/// snowy 状态，`mineable/*` 等 tag 成员资格逐字同 grass；其余行恒等。
/// （掉落门 requires_tool 列在生成期已按行落地，不走本别名。）
pub const fn tag_row_of(id: u16) -> u16 {
    if id == SNOW_GRASS_ROW { GRASS_ROW } else { id }
}

/// `blocks_gen.inc.rs` 中 GEN_BLOCKS 的元组类型（生成文件不导出别名，补一个）。
type GenBlock = (&'static str, bool, bool, bool, u8, [u16; 6], f32, u8, bool);

// 千块表：1171 方块，id 0..13 与旧 14 方块表逐字段一致（回归锁见 tests），
// 14+ 按官方名字典序。格式/来源/限制见 blocks_gen.inc.rs 头部注释。
include!("blocks_gen.inc.rs");

/// 原版方块 tag 静态表（id 域 = GEN_BLOCKS 行序）：构建期已解出全部
/// `#` 引用，运行期零解引用。生成与语义登记见 ci/gen-tags.py；
/// 等价锁 tests/tags_gen.rs。消费端硬编码切换是 #178 刀2。
pub mod tags {
    include!("tags_gen.inc.rs");
}

/// 生成表元组 → BlockDef。model_kind（第 8 字段）是网格器未来字段，此处丢弃；
/// 第 9 字段 requires_tool = 掉落门（#178 刀2a）。
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
        shape: shape::shape_of_name(t.0),
        requires_tool: t.8,
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
        requires_tool: false,
        shape: 0,
    }; GEN_BLOCKS.len()];
    let mut i = 0;
    while i < GEN_BLOCKS.len() {
        out[i] = gen_def(&GEN_BLOCKS[i]);
        i += 1;
    }
    out
}

/// 硬度与发光对照反编译 Minecraft 26.1 `Blocks.java`（数值来源见仓库外笔记
/// `mc-ref/NOTES-blocks.md`）。id 0-13 为地形生成器/网格器硬编码依赖的
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
    let (name, _solid, opaque, liquid, _emit, _tiles, _hard, model_kind, _requires) = t;
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
    //      粘液块/蜂蜜块 Blocks.java:2561/4913（noOcclusion，形状取默认整方块，
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

// ---------------------------------------------------------------------------
// 面遮蔽形状表（useShapeForLightOcclusion——#153 审计病灶#1 重建波，
// docs/light-audit.md 病灶与重建波清单①）
// ---------------------------------------------------------------------------

/// 遮蔽形状类别（SHAPE_OCCLUDE_KIND 元素值）。
pub const SHAPE_NONE: u8 = 0;
pub const SHAPE_SLAB: u8 = 1;
pub const SHAPE_STAIRS: u8 = 2;

/// 遮蔽形状类别判定（按注册名后缀 → 26.1 类属：`*_slab` → SlabBlock、
/// `*_stairs` → StairBlock；字符串判据复用 [`name_ends_with`]）。
const fn gen_shape_kind(name: &str) -> u8 {
    if name_ends_with(name, "_slab") {
        SHAPE_SLAB
    } else if name_ends_with(name, "_stairs") {
        SHAPE_STAIRS
    } else {
        SHAPE_NONE
    }
}

/// 全 1171 块遮蔽形状类别：`useShapeForLightOcclusion=true` 的方块族
/// （SlabBlock.java:49-51 `TYPE != DOUBLE`、StairBlock.java:69-71 恒
/// true）。其余方块 `LightEngine.isEmptyShape`（LightEngine.java:66-68
/// `!canOcclude || !useShapeForLightOcclusion`）→ 空遮蔽形，不参与面遮蔽。
///
/// KNOWN-DIVERGENCE：26.1 同判据方块还有 farmland/dirt_path/snow 层/
/// daylight_detector/enchanting_table/end_portal_frame/lectern/
/// stonecutter/sculk_sensor/sculk_shrieker/shelf/piston 系——其满格面
/// 分支（如 farmland 整格宽 15px 高 → DOWN 面满格）未建表，登记为病灶#1
/// 后续数据项；实践中均贴地放置（邻居 damp!=0 先行截断），源柱语义不受
/// 影响。
pub static SHAPE_OCCLUDE_KIND: [u8; GEN_BLOCKS.len()] = {
    let mut t = [SHAPE_NONE; GEN_BLOCKS.len()];
    let mut i = 0usize;
    while i < t.len() {
        t[i] = gen_shape_kind(GEN_BLOCKS[i].0);
        i += 1;
    }
    t
};

/// 查表（越界 id → SHAPE_NONE）。
#[inline]
pub fn shape_occlude_kind(id: u16) -> u8 {
    SHAPE_OCCLUDE_KIND
        .get(id as usize)
        .copied()
        .unwrap_or(SHAPE_NONE)
}

/// 源柱边遮蔽形判据（#153 审计病灶#1）：26.1
/// `ChunkSkyLightSources.isEdgeOccluded` 的第二分支
/// （ChunkSkyLightSources.java:140-148）——
/// `faceShapeOccludes(LightEngine.getOcclusionShape(top, DOWN),
/// LightEngine.getOcclusionShape(bottom, UP))`；`getOcclusionShape` 对
/// 非形状方块返回空形（LightEngine.java:62-68），`Shapes.faceShapeOccludes`
/// 任一侧满格即真、否则两侧并集铺满整面才真（Shapes.java:286-292）。
///
/// 本表覆盖板/梯两族的竖直两面（damp 维度已由 `OPACITY` 先行短路，本
/// 函数只在 damp==0 的边上被询问）：
/// - 半砖：上半态 UP 面满格（SHAPE_TOP = column(16, 8, 16) 贴 y=16）、
///   下半态 DOWN 面满格（SHAPE_BOTTOM = column(16, 0, 8) 贴 y=0）
///   （SlabBlock.java:35-36）；
/// - 楼梯同构（底座/顶板为整格宽半高盒：下半态 DOWN 满格、上半态 UP
///   满格）；半格面为朝向侧半条——下半楼梯的 UP 面 / 上半楼梯的 DOWN
///   面各占 facing 半格（facing 0=+Z 1=-Z 2=+X 3=-X，与
///   [`BlockId::state`]/网格器 emit_stairs 同一约定），两半条 facing
///   互补（0↔1、2↔3）时并集铺满整面（Shapes.java:290 的 OR 分支）。
///
/// 状态位约定见 [`BlockId::state`]：半砖 bit0=上半；楼梯 bit0-1 朝向、
/// bit2=上半。消费方：`mcv_light::column_top`（源柱截断）与
/// `mcv_light::propagate_channel` 的直下豁免维。
#[inline]
pub fn face_shape_occludes(top: BlockId, bottom: BlockId) -> bool {
    let tk = shape_occlude_kind(top.id());
    let bk = shape_occlude_kind(bottom.id());
    if tk == SHAPE_NONE && bk == SHAPE_NONE {
        return false; // 双侧空遮蔽形（Shapes.java:290 isEmpty&&isEmpty → false）
    }
    let (ts, bs) = (top.state(), bottom.state());
    // 满格分支：top 的 DOWN 面满格（下半砖/下半楼梯）或 bottom 的 UP 面
    // 满格（上半砖/上半楼梯）。
    let top_down_full = (tk == SHAPE_SLAB && ts & 1 == 0) || (tk == SHAPE_STAIRS && ts & 4 == 0);
    let bottom_up_full = (bk == SHAPE_SLAB && bs & 1 != 0) || (bk == SHAPE_STAIRS && bs & 4 != 0);
    if top_down_full || bottom_up_full {
        return true;
    }
    // 半格并集分支：top=上半楼梯（DOWN 面为朝向半条）× bottom=下半楼梯
    // （UP 面为朝向半条），facing 互补时并集铺满整面。
    tk == SHAPE_STAIRS
        && ts & 4 != 0
        && bk == SHAPE_STAIRS
        && bs & 4 == 0
        && (bs & 3) == ((ts & 3) ^ 1)
}

impl BlockId {
    /// 低 12 位真实方块 id（丢弃状态 nibble）。C++ 侧 kBarrier(0xFFFF)
    /// 掩码后是 0x0FFF——未注册 id，两侧都按"未知=不透明"处理，哨兵语义
    /// 不变；kBarrier 只存在于 C++ 邻块视图，Rust 体素数组永不写入。
    #[inline]
    pub const fn id(self) -> u16 {
        self.0 & ID_MASK
    }

    /// 状态 nibble（bit12-15）：Slab bit0=上半砖；Stairs bit0-1 朝向
    /// (0=+Z 1=-Z 2=+X 3=-X)、bit2=上半。楼梯朝向 = 26.1 FACING，即放置
    /// 玩家水平视线同向（StairBlock.java:101-102 `FACING =
    /// context.getHorizontalDirection()`），几何上踏步（整高半）位于朝向侧
    /// 半格（StairBlock.java:37-38，facing=NORTH → 上半占 -Z 半格）；
    /// bit2=1 对应原版 Half.TOP（点底面放置，StairBlock.java:103-105）。
    /// 其余形状恒 0。
    #[inline]
    pub const fn state(self) -> u8 {
        ((self.0 >> STATE_SHIFT) & 0xF) as u8
    }

    /// 把 `state`（低 4 位有效）写入状态位，保留本值低 12 位为方块 id。
    #[inline]
    pub const fn with_state(self, state: u8) -> Self {
        Self((self.0 & ID_MASK) | (((state & 0xF) as u16) << STATE_SHIFT))
    }

    #[inline]
    pub fn def(self) -> &'static BlockDef {
        &BLOCKS[self.id() as usize]
    }

    /// 放置校验用「可替换」判据（#178 刀2b：查生成的 `replaceable` tag，
    /// 26.1 `BlockBehaviour.canBeReplaced`——空气恒可替换 `BlockState.isAir`
    /// （BlockPlaceContext.java:55-57 `canPlace` 消费端），其余取
    /// `Properties.replaceable()`（BlockBehaviour.java:270-272 + 819-829、
    /// 置位 :1268）；29 成员与旧手写名单逐位相等，快照锁
    /// tests/tool_gate_snapshot.rs::replaceable_snapshot_all_1171）。
    /// 反向 = 实心与装饰方块不可替换：stone/dirt/log/torch/旧表花草
    /// （poppy/dandelion 原版不带 `.replaceable()`）等。26.1 还有一道
    /// 「手持同种方块不可替换」（BlockBehaviour.java:271
    /// `!itemInHand.is(this.asItem())`），引擎物品-方块同名放置未建模。
    #[inline]
    pub fn is_replaceable(self) -> bool {
        tags::TAG_BLOCK_REPLACEABLE
            .set
            .contains(tag_row_of(self.id()))
    }
}

/// Chunk voxel storage owned by Rust; C++ borrows per call.
///
/// 内存预算（u16 加宽 + v6 几何 384 后）：体素 192 KiB/区块、光照 96 KiB、
/// 高度图（i16×256）512 B。视距 8（17×17 = 289 区块）≈ 289 × 288.5 KiB，
/// 即约 81 MiB 体素+光照常驻（刀 5 的 resident 断言按此重估）。
/// 网格缓冲（mcv_mesher，纯 Rust 自有缓冲）只存网格不存体素，预算不变。
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

    /// u16 加宽锁定：id 必须 2 字节，区块体素存储必须 192 KiB
    /// （v6 几何 16×384×16 × u16）。
    #[test]
    fn block_id_widened_to_u16() {
        assert!(
            std::mem::size_of::<BlockId>() == 2 && CHUNK_VOXEL_BYTES == 196608,
            "size_of::<BlockId>()={}, CHUNK_VOXEL_BYTES={}",
            std::mem::size_of::<BlockId>(),
            CHUNK_VOXEL_BYTES
        );
    }

    /// v6 几何锁：16×16×384、y ∈ [-64, 320)、海平面 63、vidx 以绝对 y
    /// 入口（对齐 26.1 dimension_type/overworld.json:44,47 +
    /// noise_settings/overworld.json:392）。改动即漂移。
    #[test]
    fn v6_geometry_locks() {
        assert_eq!(CHUNK_SY, 384);
        assert_eq!(CHUNK_VOL, 98304);
        assert_eq!(WORLD_MIN_Y, -64);
        assert_eq!(WORLD_MAX_Y, 320);
        assert_eq!(SEA_LEVEL, 63);
        assert_eq!(vidx(0, WORLD_MIN_Y, 0), 0);
        assert_eq!(vidx(15, WORLD_MAX_Y - 1, 15), CHUNK_VOL - 1);
        assert_eq!(
            vidx(3, 63, 9),
            ((63 - WORLD_MIN_Y) as usize) << 8 | (9 << 4) | 3
        );
        let l = BlockPos::new(-1, WORLD_MIN_Y, -1).local();
        assert_eq!(l, [15, 0, 15]);
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
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "stone",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [4; 6],
            hardness: 1.5,
            shape: 0,
            requires_tool: true,
        },
        BlockDef {
            name: "dirt",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [3; 6],
            hardness: 0.5,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "grass",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [2, 2, 1, 3, 2, 2],
            hardness: 0.6,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "sand",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [5; 6],
            hardness: 0.5,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "water",
            solid: false,
            opaque: false,
            liquid: true,
            light_emit: 0,
            tiles: [6; 6],
            hardness: 100.0,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "log",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [7, 7, 8, 8, 7, 7],
            hardness: 2.0,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "leaves",
            solid: true,
            opaque: false,
            liquid: false,
            light_emit: 0,
            tiles: [9; 6],
            hardness: 0.2,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "planks",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [10; 6],
            hardness: 2.0,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "cobble",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [11; 6],
            hardness: 2.0,
            shape: 0,
            requires_tool: true,
        },
        BlockDef {
            name: "bedrock",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [12; 6],
            hardness: f32::INFINITY,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "snow_grass",
            solid: true,
            opaque: true,
            liquid: false,
            light_emit: 0,
            tiles: [14, 14, 13, 3, 14, 14],
            hardness: 0.6,
            shape: 0,
            requires_tool: false,
        },
        BlockDef {
            name: "flower_red",
            solid: false,
            opaque: false,
            liquid: false,
            light_emit: 0,
            tiles: [15; 6],
            hardness: 0.0,
            shape: 1, // 十字植物 → Cross（C++ 网格器按模板出双面 quad）
            requires_tool: false,
        },
        BlockDef {
            name: "flower_yellow",
            solid: false,
            opaque: false,
            liquid: false,
            light_emit: 0,
            tiles: [16; 6],
            hardness: 0.0,
            shape: 1, // 十字植物 → Cross（C++ 网格器按模板出双面 quad）
            requires_tool: false,
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
                n.requires_tool, o.requires_tool,
                "id {i} {} 掉落门（#178 刀2a 新列，旧 14 按原版 Blocks.java 实物锁定）",
                n.name
            );
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

    /// 形状表抽查：`BlockDef::shape` 由注册名派生（生成逻辑见 shape.rs），
    /// 抽查代表 id 并锁定 name→shape 与 shape_of_name 一致（防手工漂移）。
    #[test]
    fn shape_table_assignments() {
        let by_name = |want: &str| {
            BLOCKS
                .iter()
                .position(|b| b.name == want)
                .unwrap_or_else(|| panic!("未注册方块 {want}"))
        };
        assert_eq!(Shape::from_u8(BLOCKS[12].shape), Shape::Cross); // flower_red
        assert_eq!(
            Shape::from_u8(BLOCKS[by_name("acacia_fence")].shape),
            Shape::Fence
        );
        assert_eq!(
            Shape::from_u8(BLOCKS[by_name("acacia_slab")].shape),
            Shape::Slab
        );
        assert_eq!(
            Shape::from_u8(BLOCKS[by_name("acacia_stairs")].shape),
            Shape::Stairs
        );
        assert_eq!(Shape::from_u8(BLOCKS[by_name("torch")].shape), Shape::Torch);
        assert_eq!(Shape::from_u8(BLOCKS[1].shape), Shape::Cube); // stone
        for (i, b) in BLOCKS.iter().enumerate() {
            assert_eq!(
                b.shape,
                shape::shape_of_name(b.name),
                "id {i} {} 形状与 shape_of_name 不一致",
                b.name
            );
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

    /// B3 可替换名单完整性（#178 刀2b：数据源 = 生成的 `replaceable`
    /// tag，查询走 tag 侧对拍）：26.1 `Properties.replaceable()` 全集
    /// 29 名（本测试锚定名单 = 555fde1f 手写名单原样，名单侧断言语义
    /// 不变）必须与 tag 成员逐名相等、全部在注册表且互不重复。
    /// 版权卫生：只写方块名（事实），不含原版 JSON 原文。
    #[test]
    fn replaceable_names_all_registered() {
        const ANCHOR: [&str; 29] = [
            "air",
            "bubble_column",
            "bush",
            "cave_air",
            "crimson_roots",
            "dead_bush",
            "fern",
            "fire",
            "glow_lichen",
            "hanging_roots",
            "large_fern",
            "lava",
            "leaf_litter",
            "light",
            "nether_sprouts",
            "resin_clump",
            "seagrass",
            "short_dry_grass",
            "short_grass",
            "snow",
            "soul_fire",
            "structure_void",
            "tall_dry_grass",
            "tall_grass",
            "tall_seagrass",
            "vine",
            "void_air",
            "warped_roots",
            "water",
        ];
        // tag 成员 → 注册名（tag id 越界直接 panic = 未注册即红）。
        let mut from_tag: Vec<&str> = tags::TAG_BLOCK_REPLACEABLE
            .set
            .iter()
            .map(|i| BLOCKS[*i as usize].name)
            .collect();
        from_tag.sort_unstable();
        let mut sorted = ANCHOR;
        sorted.sort_unstable();
        assert_eq!(
            sorted.windows(2).filter(|w| w[0] == w[1]).count(),
            0,
            "锚定名单有重复项"
        );
        assert_eq!(
            from_tag, sorted,
            "replaceable tag 成员与旧手写 29 名名单不再相等（tag 侧漂移？核对 Blocks.java .replaceable()）"
        );
        for n in ANCHOR {
            assert!(
                BLOCKS.iter().any(|b| b.name == n),
                "可替换名单 {n} 不在注册表"
            );
        }
    }

    /// B3 `is_replaceable` 表驱动断言：可替换（空气/水/岩浆/雪层/植被/火）
    /// 与不可替换（实心块/旧表花草/火把/雪块）各若干例。26.1 依据：
    /// `replaceable` tag（`Properties.replaceable()` 全集，tag 侧查询，
    /// #178 刀2b）+ 空气特判（BlockState.isAir → canBeReplaced，tag 内）。
    #[test]
    fn is_replaceable_matches_vanilla_property() {
        let by_name = |want: &str| -> BlockId {
            BlockId(
                BLOCKS
                    .iter()
                    .position(|b| b.name == want)
                    .unwrap_or_else(|| panic!("未注册方块 {want}")) as u16,
            )
        };
        // 可替换：空气、流体（水 Blocks.java:202 / 岩浆同带 replaceable）、
        // 雪层、植被/草/藤、火、虚空族。
        for n in [
            "air",
            "cave_air",
            "void_air",
            "structure_void",
            "water",
            "lava",
            "snow",
            "short_grass",
            "fern",
            "tall_grass",
            "large_fern",
            "seagrass",
            "vine",
            "glow_lichen",
            "crimson_roots",
            "dead_bush",
            "fire",
            "soul_fire",
            "resin_clump",
        ] {
            assert!(
                by_name(n).is_replaceable(),
                "{n} 应可替换（26.1 replaceable()/空气）"
            );
        }
        // 不可替换：实心块与装饰方块（旧表花草=poppy/dandelion 原版**不带**
        // replaceable，与 short_grass 不同；snow_block 是整块雪不是雪层）。
        for n in [
            "stone",
            "dirt",
            "grass",
            "sand",
            "log",
            "planks",
            "cobble",
            "bedrock",
            "flower_red",
            "flower_yellow",
            "torch",
            "snow_block",
            "water_cauldron",
            "oak_fence",
        ] {
            assert!(!by_name(n).is_replaceable(), "{n} 不应可替换");
        }
    }
}
