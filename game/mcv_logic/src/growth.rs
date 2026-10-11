//! 生长波（任务板 #146）：树苗→树 + 作物（小麦）生长阶段——20 Hz random
//! tick 驱动的纯决策层 + 收获掉落规格。
//!
//! 机制出处（26.1 反编译树 `/root/mc-ref/src-26.1`，铁律 1；缩写：Java 相对
//! `net/minecraft/`，loot/tag 相对 `data/minecraft/`）：
//!
//! - **驱动（每区块体积比例取证）**：`ServerLevel.tickChunk`
//!   （server/level/ServerLevel.java:483-526）对每个 ticking 区块的**每个
//!   16³ section** 做 `tickSpeed` 次（gamerule random_tick_speed 默认 3，
//!   GameRules.java:74）均匀采样（`Level.getBlockRandomPos`
//!   Level.java:1059-1063，x/z/y 各 4 bit），命中 `isRandomlyTicking` 方块
//!   才进 `randomTick`。小麦 `isRandomlyTicking = !isMaxAge`
//!   （block/CropBlock.java:74-76）→ 满 age 不再消耗尝试；树苗方块注册带
//!   `.randomTicks()`（block/Blocks.java:141-145）恒 ticking。
//!   本仓 [`scan_tick`] 逐条对齐该结构（24 section × 3 尝试/块/tick），
//!   仅两处显式简化（见各自注释）：section 内容跳过表（纯优化，无行为差）
//!   与 ticking 域半径 2（KNOWN-DIVERGENCE GROWTH-1 ④）。
//! - **树苗**：`SaplingBlock.randomTick`（block/SaplingBlock.java:45-49）
//!   = `getMaxLocalRawBrightness(pos.above()) >= 9 && nextInt(7) == 0` →
//!   `advanceTree`（:51-57）：STAGE 0→1（`cycle(STAGE)`），STAGE 1 →
//!   `growTree`。亮度定义 `LevelReader.getMaxLocalRawBrightness`
//!   （world/level/LevelReader.java:163-172 → LevelLightEngine.java:146-151）
//!   = `max(blockLight, skyLight − skyDarken)`。生长条件无「上方空气」显式
//!   门（光照门已涵盖遮挡），树成活恢复语义见 `TreeGrower.growTree`
//!   （block/grower/TreeGrower.java:127,169-178：先清苗位再 place feature，
//!   失败回植）。STAGE 属性 = 0..1（BlockStateProperties.java:121）。
//!   **本仓结构生成**用 `mcv_worldgen::trees::place_tree`（#135 六树种）
//!   替代原版 sapling 特征参数——树形来自 #135 结构库，不是原版苗特征，
//!   登记 KNOWN-DIVERGENCE GROWTH-1 ①。
//! - **作物**：`CropBlock.randomTick`（block/CropBlock.java:79-89）=
//!   `getRawBrightness(pos, 0) >= 9`（无 darken）→ age<MAX_AGE(7) →
//!   `nextInt((int)(25.0F/growthSpeed) + 1) == 0` → age+1。速度公式
//!   `getGrowthSpeed`（:100-142）：3×3 底层 `GROWS_CROPS`（tag =
//!   farmland，tags/block/grows_crops.json）中心计 1.0、邻位 ÷4，同行/
//!   同列且同作物 → ÷2，否则对角同作物 → ÷2。`getBaseSeedId`
//!   （:164-166）= `WHEAT_SEEDS`；canSurvive 光照 ≥8（:149-151）是放置门，
//!   非 randomTick 门（randomTick 门 = 9）。
//! - **收获**：26.1 `loot_table/blocks/wheat.json`（实读）：age=7 → 池 1
//!   alternatives = 1×`wheat`；池 2（age=7 门）= `wheat_seeds`
//!   `binomial_with_bonus_count{extra=3, probability=0.5714286}`（无时运
//!   时 n=extra=3，p=4/7 精确值）→ 0..3 粒；age<7 → alternatives 落第二
//!   分支 = 1×`wheat_seeds`。本仓 [`harvest_drops`] 按名出规格，物品 id 解析
//!   交 `mcv_item::item_by_name`——ITEMS 表（#144 在扩）目前无 wheat/
//!   wheat_seeds，解析为 None 即不掉，登记 KNOWN-DIVERGENCE GROWTH-1 ②。
//! - **阶段表达（选型说明）**：千块表（`mcv_core/src/blocks_gen.inc.rs`）
//!   无 `*_stage0..7` 变体（grep 证）；本仓状态位先例 = slab/stairs 的
//!   `BlockId` 高 nibble（`mcv_core/src/lib.rs:94-98,387-418`，写入路径
//!   `game.rs` interact :4049-4057）→ 小麦 age 0..7 用状态 nibble 低 3 位、
//!   树苗 STAGE 用 bit0（两方块类型互斥，无位冲突；#138「状态位 5bit」债
//!   指未来可能扩位，age 只占 3 位无冲突）。随体素自动进存档/脏标记，
//!   免去独立 Map<(ChunkPos,local),u8> 世界态表的全套生命周期管理。
//!
//! 确定性契约：[`scan_tick`] 的尝试流 = [`trees::TreesRng`]（同 seed 同
//! 字节，`mcv_worldgen/src/trees.rs` 模块 doc 契约面），种子从
//! `(world_seed, game_tick)` 经 [`vanilla::channel_seed`]/[`vanilla::splitmix64`]
//! 派生；区块按入参序（调用方给 (x, z) 字典序）消耗流——同世界态同 seed
//! 恒同决策。树结构流种子在接线侧从 `(domain, tick, pos)` 派生（见
//! [`Growth::GrowTree`] 注）。

use mcv_core::{BlockId, BlockPos, ChunkPos, SECTION_COUNT, WORLD_MIN_Y};
use mcv_worldgen::trees::{Rng, TreeKind, TreesRng};
use mcv_worldgen::vanilla::{channel_seed, splitmix64};

/// 方块 id 常数——`game/mcv_core/src/blocks_gen.inc.rs` 引注（行号对应该
/// 文件当前版本；名字锁防漂移测试见 `tests/growth.rs`）。
pub mod ids {
    /// "oak_sapling"
    pub const OAK_SAPLING: u16 = 647; // blocks_gen.inc.rs:668
    /// "birch_sapling"
    pub const BIRCH_SAPLING: u16 = 86; // :107
    /// "spruce_sapling"
    pub const SPRUCE_SAPLING: u16 = 951; // :972
    /// "jungle_sapling"
    pub const JUNGLE_SAPLING: u16 = 499; // :520
    /// "acacia_sapling"
    pub const ACACIA_SAPLING: u16 = 23; // :44
    /// "dark_oak_sapling"
    pub const DARK_OAK_SAPLING: u16 = 307; // :328
    /// "wheat"（单 id，无 stage 变体——见模块头注「阶段表达」）
    pub const WHEAT: u16 = 1135; // :1156
    /// "farmland"（GROWS_CROPS/SUPPORTS_CROPS tag 全集，
    /// tags/block/grows_crops.json + supports_crops.json）
    pub const FARMLAND: u16 = 403; // :424
    /// 旧 14 方块（id 0..13 与旧表一致，blocks_gen.inc.rs:21-34）。
    pub const DIRT: u16 = 2; // :23
    pub const GRASS: u16 = 3; // :24
    pub const WATER: u16 = 5; // :26
    pub const SNOW_GRASS: u16 = 11; // :32
}

/// 六树苗 id（#135 六树种面；cherry/pale_oak/bamboo 苗在面外）。
pub const SAPLINGS: [u16; 6] = [
    ids::OAK_SAPLING,
    ids::BIRCH_SAPLING,
    ids::SPRUCE_SAPLING,
    ids::JUNGLE_SAPLING,
    ids::ACACIA_SAPLING,
    ids::DARK_OAK_SAPLING,
];

/// 六树叶 id（tree_pass.rs LEAF_IDS 同表，`vanilla/tree_pass.rs:100-107`）。
const LEAVES: [u16; 6] = [7, 19, 82, 303, 495, 947];

pub fn is_sapling(id: u16) -> bool {
    SAPLINGS.contains(&id)
}

fn is_leaves(id: u16) -> bool {
    LEAVES.contains(&id)
}

/// 本波作物面（26.1 CropBlock 子集；carrot/potato/beetroot 等同属
/// CropBlock 族，随 GROWTH-1 ③ 扩表）。
pub fn is_crop(id: u16) -> bool {
    id == ids::WHEAT
}

/// 树苗 id → 树种（#135 [`TreeKind`] 表）。
pub fn tree_kind_of(sapling_id: u16) -> Option<TreeKind> {
    match sapling_id {
        ids::OAK_SAPLING => Some(TreeKind::Oak),
        ids::BIRCH_SAPLING => Some(TreeKind::Birch),
        ids::SPRUCE_SAPLING => Some(TreeKind::Spruce),
        ids::JUNGLE_SAPLING => Some(TreeKind::Jungle),
        ids::ACACIA_SAPLING => Some(TreeKind::Acacia),
        ids::DARK_OAK_SAPLING => Some(TreeKind::DarkOak),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 原版节奏常数（全部 file:line 引注见模块头注）
// ---------------------------------------------------------------------------

/// 树苗生长亮度门：`getMaxLocalRawBrightness(pos.above()) >= 9`
/// （SaplingBlock.java:46）。
pub const SAPLING_LIGHT_MIN: u8 = 9;
/// 树苗尝试节奏：`nextInt(7) == 0`（SaplingBlock.java:46）。
pub const SAPLING_GROW_ONE_IN: u32 = 7;
/// 作物生长亮度门：`getRawBrightness(pos, 0) >= 9`（CropBlock.java:80）。
pub const CROP_LIGHT_MIN: u8 = 9;
/// 作物最大 age（CropBlock.java:29 `MAX_AGE = 7`）。
pub const CROP_MAX_AGE: u8 = 7;
/// 作物增长基数（CropBlock.java:84 `25.0F / growthSpeed`）。
pub const CROP_GROW_BASE: f32 = 25.0;
/// 每区块每个 section 每 tick 的 random tick 尝试数（random_tick_speed
/// 默认 3，GameRules.java:74；ServerLevel.java:506）。
pub const RANDOM_TICKS_PER_SECTION: u32 = 3;
/// 收获种子二项分布 n（loot wheat.json `extra: 3`，无时运）。
pub const SEEDS_EXTRA: u32 = 3;
/// 收获种子二项分布 p（loot wheat.json `probability: 0.5714286` = 4/7）。
pub const SEEDS_P_NUM: u32 = 4;
pub const SEEDS_P_DEN: u32 = 7;

/// 生长域盐（生长 tick 专属随机域；与地形/树 pass 域隔离，
/// `vanilla::channel_seed` 先例）。
pub const GROWTH_DOMAIN: u64 = 0x0C0B_CAFE;

// ---------------------------------------------------------------------------
// 阶段位（状态 nibble 选型见模块头注）
// ---------------------------------------------------------------------------

/// 小麦 age（CropBlock.java AGE_7 → 0..7）。状态 nibble 低 3 位。
#[inline]
pub const fn crop_age(id: BlockId) -> u8 {
    id.state() & 0x7
}

/// 写小麦 age（保留方块 id；CropBlock.java:85 `getStateForAge(age+1)`）。
#[inline]
pub const fn with_crop_age(id: BlockId, age: u8) -> BlockId {
    id.with_state(age & 0x7)
}

/// 树苗 STAGE（BlockStateProperties.java:121 → 0..1）。状态 nibble bit0。
#[inline]
pub const fn sapling_stage(id: BlockId) -> u8 {
    id.state() & 1
}

/// 写树苗 STAGE（SaplingBlock.java:53 `cycle(STAGE)` 的本仓面）。
#[inline]
pub const fn with_sapling_stage(id: BlockId, stage: u8) -> BlockId {
    id.with_state(stage & 1)
}

// ---------------------------------------------------------------------------
// 决策核
// ---------------------------------------------------------------------------

/// 一格的天空/方块光照（原始双通道；`ChunkLight` 低 nibble = block、高
/// nibble = sky，`mcv_core/src/chunk.rs`）。
#[derive(Clone, Copy, Debug)]
pub struct LightCell {
    pub sky: u8,
    pub block: u8,
}

impl LightCell {
    /// `getRawBrightness(pos, darkening)`（LevelLightEngine.java:146-151）：
    /// `max(blockLight, skyLight − darkening)`。
    pub fn raw_brightness(self, darkening: u8) -> u8 {
        self.sky.saturating_sub(darkening).max(self.block)
    }
}

/// 一次 random tick 命中的生长决策（接线侧 [`Growth`] 消费面）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Growth {
    /// 非 生长目标 / 门未过（原版 randomTick 无事路径）。
    None,
    /// 小麦 age → `new_age`（CropBlock.java:85）。
    CropGrow { new_age: u8 },
    /// 树苗 STAGE 0→1（SaplingBlock.java:53）。
    SaplingAdvance,
    /// 树苗 STAGE=1 → 长树（SaplingBlock.java:55 → TreeGrower.growTree）。
    /// 本仓树结构 = `place_tree`（#135），树随机流种子由接线侧从
    /// `(GROWTH_DOMAIN, game_tick, pos)` 派生（`game.rs::growth_tick`）。
    GrowTree(TreeKind),
}

/// 树立地白名单（tree_pass.rs:87 GROUND 同表 = SUPPORTS_VEGETATION 的本仓
/// 生成子集，`docs/KNOWN-DIVERGENCE.md` TREES-2；farmland 在原版 tag 内但
/// 本仓苗立地语义复用 tree_pass 生成面三 id）。
pub const TREE_GROUND: [u16; 3] = [ids::GRASS, ids::DIRT, ids::SNOW_GRASS];

fn tree_ground_ok(below: u16) -> bool {
    TREE_GROUND.contains(&below)
}

/// 生长速度公式移植（CropBlock.java:100-142 `getGrowthSpeed`）。
///
/// `at` 读世界（绝对坐标）。偏差：farmland MOISTURE 状态本仓未建模
/// （无湿度位）→ 恒取干耕地基值 1.0，湿耕地 3.0 分支不可达——登记
/// KNOWN-DIVERGENCE GROWTH-1 ⑥（干耕地产量下限，节奏偏慢侧安全）。
pub fn crop_growth_speed(
    crop: u16,
    x: i32,
    y: i32,
    z: i32,
    at: &dyn Fn(i32, i32, i32) -> BlockId,
) -> f32 {
    // :101-102 speed = 1.0F；below = pos.below()。
    let mut speed = 1.0_f32;
    // :104-121 3×3 底层：GROWS_CROPS（本仓 = farmland）中心 1.0（湿 3.0，
    // 债）、邻位 ÷4.0F。
    for xx in -1..=1 {
        for zz in -1..=1 {
            let mut block_speed = if at(x + xx, y - 1, z + zz).id() == ids::FARMLAND {
                1.0_f32
            } else {
                0.0_f32
            };
            if xx != 0 || zz != 0 {
                block_speed /= 4.0;
            }
            speed += block_speed;
        }
    }
    // :123-139 同作物行/列减半：horizontal = 西或东、vertical = 北或南；
    // 行列齐 → ÷2，否则对角同作物 → ÷2。
    let same = |dx: i32, dz: i32| at(x + dx, y, z + dz).id() == crop;
    let horizontal = same(-1, 0) || same(1, 0);
    let vertical = same(0, -1) || same(0, 1);
    if horizontal && vertical {
        speed /= 2.0;
    } else {
        let diagonal = same(-1, -1) || same(1, -1) || same(1, 1) || same(-1, 1);
        if diagonal {
            speed /= 2.0;
        }
    }
    speed
}

/// 决策核：一格 random tick 的生长判定（原版 `SaplingBlock.randomTick` /
/// `CropBlock.randomTick` 的纯函数面）。
///
/// - `here` = 本格光照（作物门 `getRawBrightness(pos, 0)`，无 darken）；
/// - `above` = pos.above() 光照（树苗门 `getMaxLocalRawBrightness` =
///   `getRawBrightness(above, getSkyDarken())`——darken 只压天光通道，
///   LevelLightEngine.java:146-151，故双通道分开传）；
/// - 短路序与原版一致：亮度门先行，未过**不消耗**随机流（SaplingBlock
///   .java:46 / CropBlock.java:80 的 `&&` 左操作数恒评估）；
/// - 长树立地门 = 树苗下方 ∈ [`TREE_GROUND`]（tree_pass ground 语义复用；
///   原版苗位 canSurvive 在放置期已保证，此处防运行期基座被挖）。
/// - `at` 读邻域（作物速度 3×3 + 同作物 8 邻）。
pub fn random_tick(
    id: BlockId,
    pos: BlockPos,
    here: LightCell,
    above: LightCell,
    sky_darken: u8,
    rng: &mut impl Rng,
    at: &dyn Fn(BlockPos) -> BlockId,
) -> Growth {
    let raw = id.id();
    if is_sapling(raw) {
        // SaplingBlock.java:46：亮度门（above − skyDarken ≥ 9）。
        if above.raw_brightness(sky_darken) < SAPLING_LIGHT_MIN {
            return Growth::None;
        }
        // :46 nextInt(7) == 0。
        if rng.next_int(SAPLING_GROW_ONE_IN) != 0 {
            return Growth::None;
        }
        // :51-57 advanceTree：STAGE 0→1；STAGE 1 → growTree。
        if sapling_stage(id) == 0 {
            return Growth::SaplingAdvance;
        }
        let Some(kind) = tree_kind_of(raw) else {
            return Growth::None;
        };
        // 立地门（tree_pass GROUND；不过 = 苗悬空/基座非土族 → 不长）。
        if !tree_ground_ok(at(BlockPos::new(pos.x, pos.y - 1, pos.z)).id()) {
            return Growth::None;
        }
        return Growth::GrowTree(kind);
    }
    if is_crop(raw) {
        // CropBlock.java:79-89：亮度门（pos 本格，dim 0 无 darken）。
        if here.raw_brightness(0) < CROP_LIGHT_MIN {
            return Growth::None;
        }
        let age = crop_age(id);
        if age >= CROP_MAX_AGE {
            return Growth::None; // :74-76 isRandomlyTicking = !isMaxAge
        }
        // :83-84 nextInt((int)(25.0F / growthSpeed) + 1) == 0 —— Java 强转
        // 向零截断，Rust `as i32` 同语义。
        let speed = crop_growth_speed(raw, pos.x, pos.y, pos.z, &|px, py, pz| {
            at(BlockPos::new(px, py, pz))
        });
        let bound = (CROP_GROW_BASE / speed) as i32 + 1;
        if rng.next_int(bound.max(1) as u32) != 0 {
            return Growth::None;
        }
        return Growth::CropGrow { new_age: age + 1 };
    }
    Growth::None
}

// ---------------------------------------------------------------------------
// tick 扫描（原版 ServerLevel.tickChunk 的 per-section 结构）
// ---------------------------------------------------------------------------

/// 一个 game tick 的生长扫描（[`Growth`] 决策列表，命中序 = 消耗序）。
///
/// - `chunks`：ticking 域，**必须 (x, z) 字典序**（接线侧排序；流消耗序
///   契约面）；
/// - 结构 = ServerLevel.java:500-523：每块 × 每 section（0..[`SECTION_COUNT`]）
///   × [`RANDOM_TICKS_PER_SECTION`] 次均匀采样（x/z/yl 各 next_int(16)，
///   Level.java:1059-1063 的 4bit 三轴同构）；
/// - 简化（无行为差）：原版 per-section `isRandomlyTicking` 内容跳过表
///   （LevelChunkSection 优化）不移植——空抽在均匀采样下概率同分布；
/// - 简化（登记债）：ticking 域 = 接线侧给的区块集（本仓 player ±2 的
///   5×5），原版 = 模拟距内全部 ticking 块（GROWTH-1 ④）。
pub fn scan_tick(
    chunks: &[ChunkPos],
    world_seed: u64,
    game_tick: u64,
    sky_darken: u8,
    read: &dyn Fn(BlockPos) -> BlockId,
    light: &dyn Fn(BlockPos) -> LightCell,
) -> Vec<(BlockPos, Growth)> {
    let mut rng = TreesRng::new(tick_stream_seed(world_seed, game_tick));
    let mut out = Vec::new();
    for &c in chunks {
        for section in 0..SECTION_COUNT {
            for _ in 0..RANDOM_TICKS_PER_SECTION {
                let lx = rng.next_int(16) as i32;
                let yl = rng.next_int(16) as i32;
                let lz = rng.next_int(16) as i32;
                let pos = BlockPos::new(
                    c.x * 16 + lx,
                    WORLD_MIN_Y + (section as i32) * 16 + yl,
                    c.z * 16 + lz,
                );
                let id = read(pos);
                let raw = id.id();
                // isRandomlyTicking 门（CropBlock.java:74-76：满 age 不进
                // randomTick → 不消耗门抽取；树苗 randomTicks() 恒进）。
                if !(is_sapling(raw) || (is_crop(raw) && crop_age(id) < CROP_MAX_AGE)) {
                    continue;
                }
                let above = BlockPos::new(pos.x, pos.y + 1, pos.z);
                let g = random_tick(
                    id,
                    pos,
                    light(pos),
                    light(above),
                    sky_darken,
                    &mut rng,
                    read,
                );
                if g != Growth::None {
                    out.push((pos, g));
                }
            }
        }
    }
    out
}

/// 尝试流种子：`(world_seed ^ GROWTH_DOMAIN)` 终混合 ^ tick 重混（同 seed
/// 同 tick 恒同流；跨 tick 独立——原版 ServerLevel.random 为长流，本仓按
/// tick 重播种是既定简化，节奏统计不变，字节级对拍不适用，GROWTH-1 ⑤）。
pub fn tick_stream_seed(world_seed: u64, game_tick: u64) -> u64 {
    let channel = splitmix64(world_seed ^ GROWTH_DOMAIN);
    splitmix64(channel ^ splitmix64(game_tick))
}

/// 树结构流种子（接线侧 [`Growth::GrowTree`] 消费）：pos 与 tick 派生，
/// 同 (seed, tick, pos) 恒同树（#135 TreesRng 契约面）。
pub fn tree_stream_seed(world_seed: u64, game_tick: u64, pos: BlockPos) -> u64 {
    let channel = channel_seed(world_seed, GROWTH_DOMAIN) as u64;
    splitmix64(
        channel
            ^ (pos.x as u64).wrapping_mul(0x9E37_79B1_9E37_79B1)
            ^ (pos.y as u64).wrapping_mul(0x85EB_CA77_85EB_CA77)
            ^ (pos.z as u64).wrapping_mul(0xC2B2_AE3D_C2B2_AE3D)
            ^ game_tick,
    )
}

// ---------------------------------------------------------------------------
// 树写入门（tree_pass ChunkClipper 语义复用）
// ---------------------------------------------------------------------------

/// 树投影写槽（tree_pass.rs:169-189 三门同表）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeSlot {
    /// 原木位：validTreePos（TreeFeature.java:53-55 → TrunkPlacer.java:93-99
    /// placeLog 门）= 本仓空气 | 叶；暗橡树立柱 isAirOrLeaves
    /// （DarkOakTrunkPlacer.java:63）取交集。
    Log,
    /// 叶位：tryPlaceLeaf（FoliagePlacer.java:171-172）= 空气 | 水 | 叶。
    Leaf,
    /// 基下泥土：TreeConfiguration.java:20-26 条件替换（非
    /// CANNOT_REPLACE_BELOW_TREE_TRUNK 才给 DIRT；本仓面 = DIRT 自身跳过）。
    Soil,
}

/// 写入 id → 槽位（非树写 id = None，接线侧忽略）。
pub fn tree_slot_of(id: u16) -> Option<TreeSlot> {
    match id {
        6 | 20 | 83 | 304 | 496 | 948 => Some(TreeSlot::Log), // 六树种 log（trees.rs ids）
        7 | 19 | 82 | 303 | 495 | 947 => Some(TreeSlot::Leaf), // 六树种 leaves
        ids::DIRT => Some(TreeSlot::Soil),
        _ => None,
    }
}

/// 槽位替换门（`vanilla/tree_pass.rs::ChunkClipper::set` 同表，引注见
/// [`TreeSlot`]）。苗位由接线侧先清为 AIR 再投影，故门面无需苗特判。
pub fn tree_write_gate(slot: TreeSlot, current: BlockId) -> bool {
    match slot {
        TreeSlot::Log => current.id() == 0 || is_leaves(current.id()),
        TreeSlot::Leaf => {
            current.id() == 0 || current.id() == ids::WATER || is_leaves(current.id())
        }
        TreeSlot::Soil => current.id() != ids::DIRT,
    }
}

// ---------------------------------------------------------------------------
// 收获规格（26.1 loot_table/blocks/wheat.json 直译）
// ---------------------------------------------------------------------------

/// 破坏作物的掉落规格（物品名 × 数量；名字 → id 由接线侧
/// `mcv_item::item_by_name` 解析，缺项 = 不掉，GROWTH-1 ②）。
///
/// - age=7：1×`wheat` + `wheat_seeds` 0..3（binomial n=3、p=4/7，三次独立
///   伯努利，消耗序 = 模块 doc loot 语义）；
/// - age<7：1×`wheat_seeds`（alternatives 第二分支）。
///
/// `rng` = 全局掉落流（`game.rs::spawn_rng` 同源，与既有
/// `drops_for_block` 同一消耗点续流）。
pub fn harvest_drops(id: BlockId, rng: &mut dyn FnMut() -> u32) -> Vec<(&'static str, u8)> {
    if id.id() != ids::WHEAT {
        return Vec::new();
    }
    if crop_age(id) < CROP_MAX_AGE {
        return vec![("wheat_seeds", 1)];
    }
    let mut out = vec![("wheat", 1)];
    let mut seeds = 0u8;
    for _ in 0..SEEDS_EXTRA {
        if rng() % SEEDS_P_DEN < SEEDS_P_NUM {
            seeds += 1;
        }
    }
    if seeds > 0 {
        out.push(("wheat_seeds", seeds));
    }
    out
}
