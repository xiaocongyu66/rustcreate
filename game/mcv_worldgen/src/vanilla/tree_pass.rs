//! vanilla 后端 pass 2 —— 树投影（任务板 #139，T5 树收口；跨界完整性修复
//! 见 fix/tree-chunk-clip）。
//!
//! 把 [`crate::trees::place_tree`]（#135 六树种结构库）接进
//! [`crate::vanilla::terrain::generate`]：pass 1 柱层写完后，扫描本区块
//! 邻域（CROWN_REACH 环带）的树池，**每棵过门的树都在本块重放**——树干
//! 在本块的投影整体（裁剪到本块），树干在邻块的树补写伸入本块的冠部。
//! 落点/树基/树种全是世界坐标纯函数（同 (cell, seed) 恒同 [`TreeSpot`]），
//! 每个世界体素恰由其所属区块写一次，跨区块一致性由纯函数性保证，无需
//! 区块间通信。
//!
//! **跨区块写语义的本仓对位（fix/tree-chunk-clip 裁决，方案 (a)）**：
//! 原版 FEATURES 阶段允许特征跨区块写一格半径——
//! `ChunkPyramid.java:30-33`（`ChunkStatus.FEATURES` 步
//! `.blockStateWriteRadius(1)`）与 `WorldGenRegion.java:231-237`
//! （setBlock 写半径门 `distanceX/Z <= blockStateWriteRadius`），跨块写
//! 经 `Level.setBlock → updateNeighborsAt`（`Level.java:246`）落到
//! `ServerChunkCache.blockChanged`（:463-468）→ `ChunkHolder.blockChanged`
//! （`ChunkHolder.java:123-135`）标邻块脏、驱动重网格/重光照，保证「树写
//! 进邻块」的可见性。本仓无 12 态金字塔，地形生成在 TaskPool worker 并行
//! （E2 确定性锁：同 seed 同字节），worker 内跨块写会引入竞争/时序面；
//! 又因树是 (seed, cell) 纯函数（`TreesRng` 由池坐标派生，#135 契约面），
//! **邻块重放同一棵树**与原版「原块跨块写 + 邻块标脏」终态逐体素等价
//! （每个体素仍只被其所属块写一次、门判据读同一 pass-1 地形），且零跨块
//! 写面——故取重放。旧实现（#139 初版）把树干不在本块的池 `continue`
//! 丢弃，跨界冠两侧都没写 = 真机「树少一边树叶」缺陷；docs/
//! KNOWN-DIVERGENCE.md TREES-2 ④ 当时声称「由邻区块生成时以同一纯函数
//! 补齐」并未兑现，本次把实现补到文档承诺。
//!
//! 树基判定与所属块的一致性锚：[`crate::vanilla::terrain::Orchestrator::
//! column_real_top`]——pass-1 缓冲实顶扫描（原 `ground_spot`）的纯函数
//! 复刻，两侧块对同一树干列必得同 (base_y, ground)；`trees_wiring.rs` 的
//! 立地门/无悬空树锁与 `trees_cross_chunk.rs` 的双侧重放锁共同围堵漂移。
//!
//! 机制出处（原版 26.1，铁律 1；缩写：placed/configured feature JSON 相对
//! `data/minecraft/worldgen/`，Java 相对
//! `/root/mc-ref/src-26.1/net/minecraft/world/level/levelgen/`）：
//!
//! - **选址池化**：legacy 后端 `rust_terrain.rs::tree_in_cell` 的 5×5 池
//!   思路（terrain.cpp:202-233 先例），落点 = 池基 + 池内抖动；与 16×16
//!   区块的期望交叠池数 = 16/5 每轴（跨界池按重叠比例摊）→ 3.2² = 10.24。
//! - **密度门（26.1 数据，非派发默认）**：原版树木密度是 per-chunk 尝试
//!   数——placed_feature/trees_*.json 的 `count` placement：
//!   Forest/Birch Forest/Taiga = weighted_list(10 w9, 11 w1) → E=10.1，
//!   Savanna = (1 w9, 2 w1) → E=1.1，Snowy Plains = (0 w9, 1 w1) → E=0.1，
//!   Plains/Sunflower = (0 w19, 1 w1) → E=0.05，Dark Forest =
//!   dark_forest_vegetation count=16，Jungle = (50 w9, 51 w1) → E=50.1，
//!   Desert/Ocean/Beach/River 生物群系 JSON 无树特征 → 0。池化换算：
//!   每池成树概率 = E[count] / 10.24（与原版同为「每区块期望树数」；
//!   Jungle/Dark Forest 超池容量 → 记 KNOWN-DIVERGENCE TREES-2）。
//! - **群系判定在树落点**：placed feature `biome` filter 语义（每尝试点
//!   查群系，非池基）——本 pass 用 [`crate::vanilla::biomes`] 现有分类
//!   （无平行分类）。
//! - **树种（configured_feature/trees_*.json random_selector）**：
//!   Forest = default oak + birch 0.2（fancy oak 0.1→折回 oak、fallen
//!   系 0.0125/0.0025→折回主种，登记 KNOWN-DIVERGENCE TREES-2）；Birch
//!   Forest = birch；Taiga = spruce（pine 1/3→折回）；Snowy Plains =
//!   spruce；Plains = oak；Savanna = acacia 0.8 + default oak 0.2；
//!   Jungle/Dark Forest 树种表驱动已留位但 10 群系点表选不出这两个群系
//!   （biomes.rs v1 子集）→ 不可达，随 TREES-2 登记。
//! - **立地门（would_survive）**：trees_plains/trees_birch/trees_snowy 的
//!   `block_predicate_filter` = would_survive(树苗)（placed_feature/*.json）
//!   → VegetationBlock.canSurvive（VegetationBlock.java:44-47）→
//!   mayPlaceOn = SUPPORTS_VEGETATION（:23-25）→
//!   tags/block/supports_vegetation.json = #substrate_overworld + farmland
//!   → substrate = #dirt/#mud/#moss_blocks/#grass_blocks。本仓生成面只有
//!   草方块/泥土/雪草（雪草=雪覆盖草方块，grass_blocks 等价）→ 白名单
//!   三 id；沙/石/水=放弃该位。
//! - **树基 y**：pass-1 实顶（密度场零交叉的块级实现；重放路径经
//!   `column_real_top` 纯函数复刻，见上）+1——比编排器的 offset 估计
//!   surface_ours 更真（base_3d/jag 与真实地表差 ±1-2 格会悬空），且与渲染
//!   所见逐块一致。基座门 SEA+1 < base ≤ TREE_MAX_Y（TREE_MAX_Y=SEA+52，
//!   legacy rust_terrain.rs:27-33 先例）。
//! - **替换门（本 pass 的世界级写规则，补 #135 TREES-1 ①②的世界半边）**：
//!   原木/叶只进 `validTreePos` 位（TreeFeature.java:53-55 =
//!   isAir | REPLACEABLE_BY_TREES，tags/block/replaceable_by_trees.json 含
//!   #leaves/water/小型植物；本仓面 = 空气/叶/水）；基下 dirt 是条件替换
//!   （TreeConfiguration.java:20-26：非 CANNOT_REPLACE_BELOW_TREE_TRUNK
//!   才给 DIRT，tags/block/cannot_replace_below_tree_trunk.json =
//!   #dirt/#mud/#moss_blocks/podzol——草方块/雪草不在 tag，替换为 dirt；
//!   本仓生成面命中 tag 的只有 DIRT 自身，跳过）。
//!
//! 随机域：树 pass 专属域盐 [`TREE_DOMAIN`]，经 `channel_seed`（同
//! noise_seed 先例 splitmix(world^domain)）派生，与地形噪声通道种子流
//! 互不扰动——地形字节不变是硬约束（golden/geometry_locks 全程不重锚）。
//! 每池抽取序（hash01 通道 1..4，全部无状态 → 确定性只依赖 (seed, cell)）：
//! jx(1) → jz(2) → gate(3) → 树种 roll(4)，树结构随机流 = TreesRng（池
//! 坐标派生种子，#135 契约面）。
//!
//! 投影两段式（先选址后盖章）：选址全部走世界坐标纯函数（`column_real_top`
//! 读密度场，不读缓冲）——先放立的树冠不压制后选的树（原版各尝试独立采样
//! heightmap 语义）；盖章序 = 池序（cell_x 升序、cell_z 升序），原木/叶
//! 替换门在盖章时对「本块 pass-1 地形 + 本块已盖章树」生效。跨块差异面
//! （债，KNOWN-DIVERGENCE TREES-2 ⑤）：原版整树一次性写入、门判据含全局
//! 已放置特征；重放模型下跨界树重叠格的赢家由各自区块的池序决定，单格
//! 物种归属可与原版全局序不同（逐格仍确定、跨块无裂缝）。

use crate::trees::{BlockSetter, TreeKind, TreesRng, ids, place_tree};
use crate::vanilla::biomes::Biome;
use crate::vanilla::terrain::{AIR, DIRT, GRASS, Orchestrator, SNOW_GRASS, WATER};

/// 树装饰专属域盐（树 pass 唯一随机域入口；与地形/气候/洞穴通道隔离）。
const TREE_DOMAIN: u64 = 0x7BEE_5501;

/// 树池边长（legacy tree_in_cell 同构）。
const CELL: i32 = 5;

/// 树冠水平延伸上界：acacia 最坏情形——ForkingTrunkPlacer.java:50-54 斜枝
/// ≤3 步 + AcaciaFoliagePlacer.java:33-54 主冠 r=2+radiusOffset 1 → 6 格。
const CROWN_REACH: i32 = 6;

/// 树基上界（SEA+52；legacy rust_terrain.rs TREE_MAX_Y 先例）。
pub const TREE_MAX_Y: i32 = mcv_core::SEA_LEVEL + 52;

/// 立地白名单（SUPPORTS_VEGETATION 的本仓生成子集；引注见模块头注）。
const GROUND: [u16; 3] = [GRASS, DIRT, SNOW_GRASS];

/// 六树种原木 id（trees::ids；放置门与测试共用）。
const LOGS: [u16; 6] = [
    ids::OAK_LOG,
    ids::ACACIA_LOG,
    ids::BIRCH_LOG,
    ids::DARK_OAK_LOG,
    ids::JUNGLE_LOG,
    ids::SPRUCE_LOG,
];

/// 六树种叶 id。
const LEAF_IDS: [u16; 6] = [
    ids::OAK_LEAVES,
    ids::ACACIA_LEAVES,
    ids::BIRCH_LEAVES,
    ids::DARK_OAK_LEAVES,
    ids::JUNGLE_LEAVES,
    ids::SPRUCE_LEAVES,
];

fn is_log(id: u16) -> bool {
    LOGS.contains(&id)
}

fn is_leaves(id: u16) -> bool {
    LEAF_IDS.contains(&id)
}

/// 群系 → (每池成树概率, 主种, 副种, 副种比例)。
///
/// 密度 = 26.1 placed feature E[count] ÷ 10.24（换算引注见模块头注）；
/// 树种 mix = configured feature random_selector 的副种 chance（fancy/
/// pine/fallen 折回主种随 KNOWN-DIVERGENCE TREES-2）。Jungle/Dark Forest
/// 表已留位、10 群系点表选不出 → None 路径由群系枚举穷尽保证。
fn profile(b: Biome) -> Option<(f64, TreeKind, TreeKind, f64)> {
    use Biome as B;
    use TreeKind as T;
    // 期望交叠池数 = 16×16 窗 ÷ 5 格池 = 3.2² = 10.24（跨界池按重叠比例
    // 摊入期望——与原版 in_square 均匀撒点的每区块期望尝试数同口径）。
    const CELLS_PER_CHUNK: f64 = 10.24;
    let (density, primary, secondary, mix) = match b {
        // trees_plains: (0 w19, 1 w1) → E=0.05；全 oak。
        B::Plains => (0.05 / CELLS_PER_CHUNK, T::Oak, T::Oak, 0.0),
        // trees_birch_and_oak_leaf_litter: (10 w9, 11 w1) → E=10.1；birch 0.2。
        B::Forest => (10.1 / CELLS_PER_CHUNK, T::Oak, T::Birch, 0.2),
        // trees_birch: (10 w9, 11 w1)；全 birch。
        B::BirchForest => (10.1 / CELLS_PER_CHUNK, T::Birch, T::Birch, 0.0),
        // trees_savanna: (1 w9, 2 w1) → E=1.1；acacia 0.8 + default oak 0.2。
        B::Savanna => (1.1 / CELLS_PER_CHUNK, T::Acacia, T::Oak, 0.2),
        // trees_taiga: (10 w9, 11 w1)；spruce（pine 1/3 折回）。
        B::Taiga => (10.1 / CELLS_PER_CHUNK, T::Spruce, T::Spruce, 0.0),
        // trees_snowy: (0 w9, 1 w1) → E=0.1；spruce。
        B::SnowyPlains => (0.1 / CELLS_PER_CHUNK, T::Spruce, T::Spruce, 0.0),
        // 无树特征的群系（biome JSON vegetal_decoration 无 trees_*）。
        B::Desert | B::Ocean | B::Beach | B::River => return None,
    };
    Some((density, primary, secondary, mix))
}

/// 世界 → 本区块局部裁剪写手：越界（跨区块冠/干）丢弃、块内按原版
/// validTreePos / 基下 dirt 条件替换门写（引注见模块头注）。
///
/// 「丢弃」不丢内容：跨界格由其所属区块对同一 [`TreeSpot`] 重放补齐
/// （原版 blockStateWriteRadius=1 跨块写 + ChunkHolder 标脏的重放等价，
/// 裁决与引注见模块头注；本仓 worker 并行无跨界写面）。
struct ChunkClipper<'a> {
    voxels: &'a mut [u16],
    base_x: i32,
    base_z: i32,
}

impl BlockSetter for ChunkClipper<'_> {
    fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        let lx = x - self.base_x;
        let lz = z - self.base_z;
        // 出块格不写本块缓冲：属邻块的格由邻块重放本树补齐（模块头注
        // fix/tree-chunk-clip 裁决；旧实现在此裁剪后两侧都不写 = 真机
        // 「树少一边树叶」缺陷）。
        if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
            return;
        }
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&y) {
            return;
        }
        let cell = &mut self.voxels[mcv_core::vidx(lx as usize, y, lz as usize)];
        if is_log(id) {
            // placeLog → validTreePos（TrunkPlacer.java:93-99/113-115 →
            // TreeFeature.java:53-55）；暗橡树立柱用更窄的 isAirOrLeaves
            // （DarkOakTrunkPlacer.java:63）——水在树基门下不可达，取交集。
            if *cell == AIR || is_leaves(*cell) {
                *cell = id;
            }
        } else if is_leaves(id) {
            // tryPlaceLeaf → validTreePos（FoliagePlacer.java:171-172）：
            // air | replaceable_by_trees（本仓面：空气/水/叶）。
            if *cell == AIR || *cell == WATER || is_leaves(*cell) {
                *cell = id;
            }
        } else if id == DIRT {
            // TreeConfiguration.java:20-26：非 CANNOT_REPLACE_BELOW_TREE_TRUNK
            // 才替换 DIRT；本仓生成面命中 tag 的只有 DIRT 自身（草/雪草
            // 不在 tag，按原版替换）。
            if *cell != DIRT {
                *cell = DIRT;
            }
        }
    }
}

/// pass 2 选中的一棵树（世界坐标 + 树基 + 树种 + 结构随机流种子）。
/// 全部字段是 (seed, cell) 纯函数——同一棵树在任意区块的重放读数一致
/// （`trees_cross_chunk.rs` 锁）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeSpot {
    pub x: i32,
    pub z: i32,
    pub base_y: i32,
    pub kind: TreeKind,
    pub rng_seed: u64,
}

/// 选址段（两段式第一段）：扫描与区块窗口交叠 CROWN_REACH 的树池，返回
/// 过密度门/群系门/立地门的树（池序 = cell_x 升、cell_z 升）。
///
/// 与 [`project_trees`] 同一入口供测试重放：跨界树两侧区块各自调用本函数
/// 都能拿到同一 [`TreeSpot`]（树干所属性、树基、树种、结构种子与调用块
/// 无关），`place_tree(&mut TreesRng::new(rng_seed), …)` 重放即可复现该树
/// 全部写入格。
#[must_use]
pub fn tree_spots(seed: u64, chunk_x: i32, chunk_z: i32, orch: &Orchestrator) -> Vec<TreeSpot> {
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    let ts = crate::vanilla::channel_seed(seed, TREE_DOMAIN);
    let ts_u = ts as u64;
    // 邻域池扫描：树落点世界坐标纯函数；CROWN_REACH 环带内的池（含树干
    // 在邻块的）全部入选——树干所属块投影其裁剪整体，邻块重放伸入部分
    // （模块头注 fix/tree-chunk-clip）。
    let c0x = (base_x - CROWN_REACH).div_euclid(CELL);
    let c1x = (base_x + 15 + CROWN_REACH).div_euclid(CELL);
    let c0z = (base_z - CROWN_REACH).div_euclid(CELL);
    let c1z = (base_z + 15 + CROWN_REACH).div_euclid(CELL);
    let mut spots = Vec::new();
    for cell_z in c0z..=c1z {
        for cell_x in c0x..=c1x {
            let cx = i64::from(cell_x);
            let cz = i64::from(cell_z);
            // 池内抖动（legacy tree_in_cell 同构：hash 通道 1/2，(0,CELL) 截断）。
            let jx = f64::from(crate::vanilla::hash01(ts, cx, cz, 1));
            let jz = f64::from(crate::vanilla::hash01(ts, cx, cz, 2));
            let wx = cell_x * CELL + (jx * f64::from(CELL)) as i32;
            let wz = cell_z * CELL + (jz * f64::from(CELL)) as i32;
            // 群系判定在树落点（placed feature `biome` filter 语义）。
            let Some((density, primary, secondary, mix)) = profile(orch.biome_at(wx, wz)) else {
                continue;
            };
            // 密度门（hash 通道 3）。
            let gate = f64::from(crate::vanilla::hash01(ts, cx, cz, 3));
            if gate >= density {
                continue;
            }
            // 立地门（would_survive 白名单；沙/石/水放弃该位）。树基经
            // `column_real_top` 纯函数判定——树干出块不再跳过（旧实现在此
            // `continue`，跨界冠两侧皆失 = 真机「树少一边树叶」），且与
            // 树干所属块的读数字节一致（两侧各算各的必同 (base_y, ground)，
            // 否则重放错位出新裂缝）。
            let Some((base_y, ground)) = orch.column_real_top(wx, wz) else {
                continue;
            };
            if !GROUND.contains(&ground) {
                continue;
            }
            // 树种（hash 通道 4 副种 roll；random_selector 语义）。
            let roll = f64::from(crate::vanilla::hash01(ts, cx, cz, 4));
            let kind = if roll < mix { secondary } else { primary };
            // 树结构随机流：池坐标派生（同 (cell, seed) 恒同树）。
            let rng_seed = crate::vanilla::splitmix64(
                ts_u ^ (cx as u64).wrapping_mul(0x9E37_79B1_9E37_79B1)
                    ^ (cz as u64).wrapping_mul(0x85EB_CA77_85EB_CA77),
            );
            spots.push(TreeSpot {
                x: wx,
                z: wz,
                base_y,
                kind,
                rng_seed,
            });
        }
    }
    spots
}

/// pass 2：树投影。`orch` 复用 pass 1 的气候通道（群系判定无平行分类）。
///
/// 两段式：[`tree_spots`] 完成全部选址判定（密度门/群系门/立地门读密度场
/// 不读缓冲、互不看见彼此的冠——与原版各尝试独立采样 heightmap 同语义），
/// 再按池序统一投影盖章（裁剪写 [`ChunkClipper`]）。
pub fn project_trees(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    orch: &Orchestrator,
    voxels: &mut [u16],
) {
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    for spot in tree_spots(seed, chunk_x, chunk_z, orch) {
        let mut rng = TreesRng::new(spot.rng_seed);
        let mut clipper = ChunkClipper {
            voxels: &mut *voxels,
            base_x,
            base_z,
        };
        place_tree(
            &mut rng,
            spot.x,
            spot.base_y,
            spot.z,
            spot.kind,
            &mut clipper,
        );
    }
}
