//! #175 刀C 岩浆湖特征 — 26.1 `LakeFeature` + `lake_lava_*` placed
//! feature 的最小可用移植（实证摘要，快照 /root/mc-ref/src-26.1）：
//!
//! - **特征本体** `feature/LakeFeature.java:32-160`：origin 偏移 (−8,−4,−8)
//!   的 16×8×16 盒内，`nextInt(4)+4` 个随机椭球并集成洞格（:38-66，
//!   半径 (3..9, 2..6, 3..9)、中心约束盒内）；**判据环**（:68-92）：洞格
//!   边界非洞格位——上半部（yy≥4）读到流体 → 整个特征放弃（岩浆湖不
//!   贴水上檐，`blockState.liquid()` :80-82）；下半部（yy<4）读到非实心
//!   且非本流体 → 放弃（湖底/侧壁必须封住，:84-86）——这就是原版
//!   canBePlacementInLava/solidify 语义在 26.1 的形态（旧 FeatureLake 的
//!   solidify 已被该判据环 + barrier 铺装取代，以实物为准）。
//! - **铺装**（:113-138）：配置 barrier provider（lake_lava.json =
//!   STONE）在洞格边界位替换实心方块（yy<4 恒铺、yy≥4 抛硬币 1/2），
//!   LAVA_POOL_STONE_CANNOT_REPLACE（:120-122，实物 = features_cannot_replace
//!   ∪ leaves ∪ logs，见 data/tags/block/lava_pool_stone_cannot_replace.json）
//!   不换——湖早于植被 pass（leaves/logs 尚不存在）、bedrock 单列门已在
//!   放置侧排除；湖底铺石**不依赖 SurfaceRules**（provider 直写），
//!   按原样接（无需登记 #176）。
//! - **放置/放弃路径**（:94-111）：洞格内 yy≥4 → CAVE_AIR（本仓 AIR），
//!   yy<4 → fluid；canReplaceBlock = !FEATURES_CANNOT_REPLACE（基岩/
//!   门户族，本仓面 = 排除 BEDROCK）。
//! - **频度**（data/minecraft/worldgen/placed_feature/ 实物）：
//!   `lake_lava_underground.json` rarity_filter **1/9** 每区块 +
//!   height_range y∈[0, top) + environment_scan **向下 ≤32 步**找非空气
//!   （命中位=origin，EnvironmentScanPlacement.java:51-70）+
//!   surface_relative_threshold_filter（OCEAN_FLOOR_WG，min 缺省
//!   INT_MIN、max_inclusive −5 ⇒ y ≤ hm−5，
//!   SurfaceRelativeThresholdFilter.java:11-38）；
//!   `lake_lava_surface.json` rarity_filter **1/200** 每区块 +
//!   heightmap WORLD_SURFACE_WG（origin = 高度图值 = 顶块 +1，
//!   HeightmapPlacement.java:26-31）。群系注册：65 个群系 json 中 53 个
//!   在两特征同位的 features[1] 注册这对（下/上相邻两项）；缺席的 12 个
//!   = 下界 5 + 末地 5 + the_void + deep_dark——对本仓（主世界生成面）
//!   仅 deep_dark 有差。biome placement filter 未逐群系建模（本仓
//!   不分群系跑特征），差异登记 LAKE-1。
//! - **水湖不存在（实证）**：26.1 数据仅 lake_lava configured/placed；
//!   Feature.LAKE 的水变体不在 overworld 生成设置（旧版 lake.json 已
//!   移除）。地表水体由密度场负值 + 含水层 pokes/global 路给出
//!   （aquifer.rs），非特征——协调令「水/岩浆两种 fluid 分叉」的水侧
//!   即该既有路，登记之；冻结群系成冰面（:140-150）无 frozen 群系面，
//!   不可达不建模。
//!
//! ## 本仓接线模型（与 tree_pass 同构的纯函数重放）
//! 原版特征经 WorldGenRegion 写邻块；本仓单遍生成，特征跨块 = 候选集
//! 纯函数化 + 3×3 origin 区块重放 + 盒裁剪写块。判据环读 **pass-1 重放
//! 态**（密度/含水层实心 ∪ 开放柱门 `aquifer::open_column`（#175 终修
//! 同型，实顶以上恒三态）∪ 基岩 −64 恒实心并入），不读他块缓冲、不读
//! 先前湖写入——与 vanilla「未处理邻块 = 噪声填充态」对齐；origin 块
//! 内湖间重叠为低频边缘（登记 KNOWN-DIVERGENCE LAKE-1）。应用序 =
//! (chunk_x, chunk_z, variant) 全序，两侧块重放同序 ⇒ 覆盖写一致。

use crate::vanilla::aquifer::{AquiferField, Substance, open_column};
use crate::vanilla::channel_seed;
use crate::vanilla::splitmix64;
use crate::vanilla::terrain::{AIR, BEDROCK, LAVA, Orchestrator, STONE, WATER};
use std::collections::HashMap;

/// 湖通道域盐（TREE_DOMAIN 先例；非原版 WorldgenRandom 大特征种子链，
/// 登记 LAKE-1）。
pub const LAKE_DOMAIN: u64 = 0x0000_1A3E_005E;

/// 洞格盒半宽/半高（origin 偏移 (−8,−4,−8)，LakeFeature.java:34）。
const HX: i32 = 8;
const HY: i32 = 4;

/// pass-1 重放器：实心/空气判据与 column_state 列缓存（湖判据环/扫描
/// 的热点列）。#175 终修开放柱门（与 terrain.rs `hit_solid` 同型，
/// 见 `aquifer::open_column` 头注）：实顶（`top_solid`，首个
/// density>0 位）以上 = 无条件三态 `open_column`，不进胞格噪声——
/// 判据环/扫描与缓冲体素由构造一致。实顶以下：solid = density>0 ∪
/// Rock 封口袋 ∪ 基岩层 −64；air = !solid ∧ substance==Air（水/岩浆
/// 为非空气 = environment_scan 的 not-air 判据面）。
struct Replay<'a, 'f> {
    field: &'a mut AquiferField<'f>,
    cell: crate::vanilla::cell_interp::CellShell<'a>,
    tops: HashMap<(i32, i32), i32>,
}

impl<'a, 'f> Replay<'a, 'f> {
    fn new(orch: &'a Orchestrator, field: &'a mut AquiferField<'f>) -> Self {
        Self {
            field,
            cell: crate::vanilla::cell_interp::CellShell::new(orch),
            tops: HashMap::new(),
        }
    }

    fn density(&mut self, x: i32, y: i32, z: i32) -> f64 {
        // #210：判据面与填充环同走 cell 插值外壳（读 pass-1 已生成块
        // 语义——原版 feature 读生成面；角列缓存纯世界坐标函数，
        // 跨块重放与缓冲构造一致）。column_state 列缓存随迁删除
        // （cell 面不再逐体素取列状态）。
        let cell = self.cell.cols4(x, z);
        crate::vanilla::cell_interp::CellShell::density_at(&cell, x, y, z)
    }

    /// 实顶：自顶向下首个 density>0 位（缓冲侧 `hit_solid` 首实心位
    /// 的纯密度像——Rock 封口袋只在首实心以下有意义，基岩地板 −64
    /// 兜底恒存在）。开放柱门的面。
    fn top_solid(&mut self, x: i32, z: i32) -> i32 {
        let key = (x, z);
        if let Some(&t) = self.tops.get(&key) {
            return t;
        }
        let mut y = mcv_core::WORLD_MAX_Y - 1;
        while y > mcv_core::WORLD_MIN_Y && self.density(x, y, z) <= 0.0 {
            y -= 1;
        }
        self.tops.insert(key, y);
        y
    }

    fn solid(&mut self, x: i32, y: i32, z: i32) -> bool {
        if y <= mcv_core::WORLD_MIN_Y {
            return true; // pass-1 基岩地板恒实心
        }
        if y > self.top_solid(x, z) {
            return false; // 开放柱无实心
        }
        let d = self.density(x, y, z);
        if d > 0.0 {
            return true;
        }
        let skip = self
            .field
            .chunk_skip(x.div_euclid(16) * 16, z.div_euclid(16) * 16);
        matches!(self.field.substance(x, y, z, d, skip), Substance::Rock)
    }

    fn air(&mut self, x: i32, y: i32, z: i32) -> bool {
        if y <= mcv_core::WORLD_MIN_Y {
            return false;
        }
        if y > self.top_solid(x, z) {
            return open_column(y) == Substance::Air;
        }
        let d = self.density(x, y, z);
        if d > 0.0 {
            return false;
        }
        let skip = self
            .field
            .chunk_skip(x.div_euclid(16) * 16, z.div_euclid(16) * 16);
        matches!(self.field.substance(x, y, z, d, skip), Substance::Air)
    }

    /// 流体位（判据环上半部 liquid() 门）：密度≤0 且判据 Water/Lava。
    fn fluid(&mut self, x: i32, y: i32, z: i32) -> bool {
        if y <= mcv_core::WORLD_MIN_Y {
            return false;
        }
        if y > self.top_solid(x, z) {
            return matches!(open_column(y), Substance::Water | Substance::Lava);
        }
        let d = self.density(x, y, z);
        if d > 0.0 {
            return false;
        }
        let skip = self
            .field
            .chunk_skip(x.div_euclid(16) * 16, z.div_euclid(16) * 16);
        matches!(
            self.field.substance(x, y, z, d, skip),
            Substance::Water | Substance::Lava
        )
    }

    /// 本流体位（判据环下半部 `state != fluid` 豁免，
    /// LakeFeature.java:84-86：非实心但已是 LAVA 的边界位不弃单——
    /// 含水层岩浆袋/−54 岩浆海顶在扫描范围内可达，豁免必须实装，
    /// 不能按「恒不可达」化简）。
    fn lava(&mut self, x: i32, y: i32, z: i32) -> bool {
        if y <= mcv_core::WORLD_MIN_Y {
            return false;
        }
        if y > self.top_solid(x, z) {
            return open_column(y) == Substance::Lava;
        }
        let d = self.density(x, y, z);
        if d > 0.0 {
            return false;
        }
        let skip = self
            .field
            .chunk_skip(x.div_euclid(16) * 16, z.div_euclid(16) * 16);
        matches!(self.field.substance(x, y, z, d, skip), Substance::Lava)
    }

    /// OCEAN_FLOOR_WG 顶（blocksMotion 谓词 = 实心 ∪ 流体，26.1
    /// Heightmap.java:149,31——水/岩浆计入，海洋列顶是水面非海床）。
    /// 开放柱恒灌面（`open_column`）⇒ 封闭岩层内流体不高于实顶，
    /// 顶 = max(实顶, 海平面)（海柱 = 海面 y=SEA，陆柱 = 实顶）。
    fn top_floor(&mut self, x: i32, z: i32) -> i32 {
        self.top_solid(x, z).max(mcv_core::SEA_LEVEL)
    }
}

/// 候选湖（origin 区块纯函数导出；`variant` 0=underground 1=surface）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LakeSpot {
    pub ox: i32,
    pub oy: i32,
    pub oz: i32,
    pub variant: u8,
    /// 形状/铺装续流种子（rarity/in_square/高度消耗之后的流状态——
    /// 两侧块重放同序同耗，恒等）。
    rng_seed: u64,
}

/// 全序候选枚举：一个候选湖 = origin 区块一次尝试（下/上两 variant）。
/// 盒从 origin 最远伸 ±8 → 3×3 origin 区块覆盖全部交叠候选。
#[must_use]
pub fn lake_spots(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
) -> Vec<LakeSpot> {
    let ts = splitmix64(channel_seed(seed, LAKE_DOMAIN) as u64);
    let mut rp = Replay::new(orch, field);
    let mut spots = Vec::new();
    for dz in -1..=1 {
        for dx in -1..=1 {
            for variant in 0..2u8 {
                let (cx, cz) = (chunk_x + dx, chunk_z + dz);
                if let Some(s) = try_spot(ts, cx, cz, variant, &mut rp) {
                    spots.push(s);
                }
            }
        }
    }
    // 应用序 = (origin chunk_x, chunk_z, variant) 全序（两侧块重放同序）
    spots.sort_by_key(|s| {
        (
            s.ox.div_euclid(16),
            s.oz.div_euclid(16),
            s.variant,
            s.ox,
            s.oz,
        )
    });
    spots
}

/// 单 origin 尝试：rarity_filter → in_square → 高度路（variant 语义见
/// 模块头注）。None = 本区块该 variant 无候选。
fn try_spot(ts: u64, cx: i32, cz: i32, variant: u8, rp: &mut Replay<'_, '_>) -> Option<LakeSpot> {
    // 每 (chunk, variant) 一条独立流（同 (seed,chunk,variant) 恒同结果）
    let vs = splitmix64(
        ts ^ (i64::from(cx) as u64).wrapping_mul(0x9E37_79B1_9E37_79B1)
            ^ (i64::from(cz) as u64).wrapping_mul(0x85EB_CA77_85EB_CA77)
            ^ u64::from(variant).wrapping_mul(0x36B1_C4A2_77B1_23A3),
    );
    let mut rng = LakeRng::new(vs);
    // rarity_filter：underground 1/9、surface 1/200（placed json 实物）
    let chance = if variant == 0 { 9 } else { 200 };
    if rng.next_int(chance) != 0 {
        return None;
    }
    // in_square：区块内均匀 x/z
    let wx = cx * 16 + rng.next_int(16) as i32;
    let wz = cz * 16 + rng.next_int(16) as i32;
    let floor_top = rp.top_floor(wx, wz); // OCEAN_FLOOR_WG 顶（max(实顶,海面)）
    let oy = if variant == 0 {
        // height_range y∈[0, 320) + environment_scan down ≤32 步找非空气
        // （EnvironmentScanPlacement.java:51-70：原位先测、命中即原位、
        // 否则步进 down 再测）
        let mut y = rng.next_int(mcv_core::WORLD_MAX_Y as u32) as i32;
        let mut found = None;
        // 33 个测试位（循环内 32 次 + 步进 32 格后补测 1 次，
        // EnvironmentScanPlacement.java:54-70 实物）；
        // inside_world_bounds(−5) 失败位永不命中，扫到即止与原版
        // 「继续下探但条件恒假」结果等价（minY+5 以下必非目标）。
        for _ in 0..=32 {
            if !rp.air(wx, y, wz) {
                found = Some(y);
                break;
            }
            if y <= mcv_core::WORLD_MIN_Y + 5 {
                break; // inside_world_bounds offset −5（placed json）
            }
            y -= 1;
        }
        let y = found?;
        // surface_relative_threshold_filter：y ≤ hm + max_inclusive(−5)，
        // hm = 高度图值 = floor_top + 1（SurfaceRelativeThresholdFilter
        // .java:35-38，min_inclusive 缺省 = INT_MIN）
        if y > floor_top - 4 {
            return None;
        }
        y
    } else {
        // heightmap WORLD_SURFACE_WG：高度图值 = 顶块 +1
        // （HeightmapPlacement.java:26-31）
        floor_top + 1
    };
    Some(LakeSpot {
        ox: wx,
        oy,
        oz: wz,
        variant,
        rng_seed: rng.s, // 形状/铺装消耗自此续流
    })
}

/// 洞格索引（LakeFeature.java:59 `grid[(xx*16+zz)*8+yy]` 逐字对位）。
const fn gidx(xx: usize, yy: usize, zz: usize) -> usize {
    (xx * 16 + zz) * 8 + yy
}

/// 洞格网格（LakeFeature.java:38-66 椭球并集；rng 消耗序 = spot 数 →
/// 每椭球 6 draw，两侧块重放恒等）。
fn build_grid(spot: &LakeSpot) -> ([bool; 16 * 16 * 8], LakeRng) {
    let mut rng = LakeRng::new(spot.rng_seed);
    let mut grid = [false; 16 * 16 * 8];
    let spots_n = rng.next_int(4) + 4;
    for _ in 0..spots_n {
        let rx = rng.next_f64() * 6.0 + 3.0;
        let ry = rng.next_f64() * 4.0 + 2.0;
        let rz = rng.next_f64() * 6.0 + 3.0;
        let px = rng.next_f64() * (16.0 - rx - 2.0) + 1.0 + rx / 2.0;
        let py = rng.next_f64() * (8.0 - ry - 4.0) + 2.0 + ry / 2.0;
        let pz = rng.next_f64() * (16.0 - rz - 2.0) + 1.0 + rz / 2.0;
        for xx in 1..15usize {
            for zz in 1..15usize {
                for yy in 1..7usize {
                    let dx = (xx as f64 - px) / (rx / 2.0);
                    let dy = (yy as f64 - py) / (ry / 2.0);
                    let dz = (zz as f64 - pz) / (rz / 2.0);
                    if dx * dx + dy * dy + dz * dz < 1.0 {
                        grid[gidx(xx, yy, zz)] = true;
                    }
                }
            }
        }
    }
    (grid, rng)
}

fn boundary_of(grid: &[bool; 16 * 16 * 8], xx: usize, yy: usize, zz: usize) -> bool {
    if grid[gidx(xx, yy, zz)] {
        return false;
    }
    (xx < 15 && grid[gidx(xx + 1, yy, zz)])
        || (xx > 0 && grid[gidx(xx - 1, yy, zz)])
        || (zz < 15 && grid[gidx(xx, yy, zz + 1)])
        || (zz > 0 && grid[gidx(xx, yy, zz - 1)])
        || (yy < 7 && grid[gidx(xx, yy + 1, zz)])
        || (yy > 0 && grid[gidx(xx, yy - 1, zz)])
}

/// 判据环（LakeFeature.java:68-92，全盒判定、纯 pass-1 重放态——与
/// 调用块无关，两侧块同判同弃）。pub 供测试在纯函数面统计存活率。
pub fn lake_passes_gate(
    spot: &LakeSpot,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
) -> bool {
    let (grid, _rng) = build_grid(spot);
    let mut rp = Replay::new(orch, field);
    gate_ok(spot, &grid, &mut rp)
}

/// 该列
/// WORLD_SURFACE_WG 实顶（Heightmap.java:147 NOT_AIR 判据：水计非空气
/// ⇒ 海柱=水面、陆柱=实心顶；高度图值 = 本值 +1，
/// HeightmapPlacement.java:24-29 origin 即读该值）。供测试按实物口径
/// 判「立足干地（顶 ≥ SEA，无水覆盖）vs 水柱（顶 < SEA，水覆盖）」，
/// 替代 `column_real_top`（扫描窗 `(SEA+1)..` 恰漏 y=SEA 干顶列）代理。
#[must_use]
pub fn world_surface_top_y(
    x: i32,
    z: i32,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
) -> i32 {
    Replay::new(orch, field).top_solid(x, z)
}

fn gate_ok(spot: &LakeSpot, grid: &[bool; 16 * 16 * 8], rp: &mut Replay<'_, '_>) -> bool {
    // 盒原点世界坐标
    let ox = spot.ox - HX;
    let oy = spot.oy - HY;
    let oz = spot.oz - HX;
    for xx in 0..16usize {
        for zz in 0..16usize {
            for yy in 0..8usize {
                if !boundary_of(grid, xx, yy, zz) {
                    continue;
                }
                let (wx, wy, wz) = (ox + xx as i32, oy + yy as i32, oz + zz as i32);
                if yy as i32 >= HY {
                    // 上半部贴流体 → 整个特征放弃（liquid() 门）
                    if rp.fluid(wx, wy, wz) {
                        return false;
                    }
                } else if !rp.solid(wx, wy, wz) && !rp.lava(wx, wy, wz) {
                    // 下半部必须封底；原版 `!isSolid && state != fluid`
                    // 的本流体豁免（LakeFeature.java:84-86）：边界位
                    // 已是岩浆（含水层岩浆袋/岩浆海顶）不弃单。
                    return false;
                }
            }
        }
    }
    true
}

/// 特征落点（origin=(ox,oy,oz) 盒）盖到本块则写缓冲。判据环读重放态
/// （与调用块无关，两侧块同判同弃）；放置写实际缓冲。返回是否有写入。
fn stamp_lake(
    spot: LakeSpot,
    rp: &mut Replay<'_, '_>,
    voxels: &mut [u16],
    base_x: i32,
    base_z: i32,
) -> bool {
    let (grid, mut rng) = build_grid(&spot);
    if !gate_ok(&spot, &grid, rp) {
        return false;
    }
    let ox = spot.ox - HX;
    let oy = spot.oy - HY;
    let oz = spot.oz - HX;
    // 放置：洞格 y≥HY → AIR；y<HY → LAVA。canReplace 门 = 缓冲非基岩。
    let mut wrote = false;
    for xx in 0..16usize {
        for zz in 0..16usize {
            for yy in 0..8usize {
                if !grid[gidx(xx, yy, zz)] {
                    continue;
                }
                let (wx, wy, wz) = (ox + xx as i32, oy + yy as i32, oz + zz as i32);
                let Some(idx) = clip_idx(wx, wz, base_x, base_z, wy) else {
                    continue;
                };
                if voxels[idx] == BEDROCK {
                    continue; // FEATURES_CANNOT_REPLACE 在本仓生成面的像
                }
                voxels[idx] = if yy as i32 >= HY { AIR } else { LAVA };
                wrote = true;
            }
        }
    }
    // barrier 铺装（lake_lava.json provider=STONE）：边界实心位
    // （yy<HY 恒铺、yy≥HY 抛硬币）→ STONE
    for xx in 0..16usize {
        for zz in 0..16usize {
            for yy in 0..8usize {
                if !boundary_of(&grid, xx, yy, zz) {
                    continue;
                }
                if yy as i32 >= HY && rng.next_int(2) != 0 {
                    continue;
                }
                let (wx, wy, wz) = (ox + xx as i32, oy + yy as i32, oz + zz as i32);
                let Some(idx) = clip_idx(wx, wz, base_x, base_z, wy) else {
                    continue;
                };
                let id = voxels[idx];
                if id != AIR && id != WATER && id != LAVA && id != BEDROCK {
                    voxels[idx] = STONE;
                }
            }
        }
    }
    wrote
}

fn clip_idx(wx: i32, wz: i32, base_x: i32, base_z: i32, wy: i32) -> Option<usize> {
    let in_x = wx >= base_x && wx < base_x + 16;
    let in_z = wz >= base_z && wz < base_z + 16;
    if !in_x || !in_z || !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&wy) {
        return None;
    }
    Some(mcv_core::vidx(
        (wx - base_x) as usize,
        wy,
        (wz - base_z) as usize,
    ))
}

/// pass-1 后、树 pass 前入口（原版 lakes 特征阶早于 vegetation）。返回
/// 写入本块的湖数（测试/K- 行消费）。
pub fn place_lakes(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
    voxels: &mut [u16],
) -> usize {
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    let spots = lake_spots(seed, chunk_x, chunk_z, orch, field);
    let mut n = 0;
    let mut rp = Replay::new(orch, field);
    for spot in spots {
        if stamp_lake(spot, &mut rp, voxels, base_x, base_z) {
            n += 1;
        }
    }
    n
}

/// 特征形状随机流（clean-room：splitmix64 续流 + Lemire 无偏整除，
/// 对齐 trees.rs TreesRng 契约先例；非原版 WorldgenRandom 大特征种子链，
/// LAKE-1）。消耗序复刻 LakeFeature.place：spot 数 → 每椭球 6 draw →
/// barrier 硬币。
pub struct LakeRng {
    s: u64,
}

impl LakeRng {
    #[must_use]
    const fn new(seed: u64) -> Self {
        Self { s: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.s = splitmix64(self.s ^ 0x9E37_79B9_7F4A_7C15);
        self.s
    }

    /// [0,1) 双精度（53 位尾数，同 nextDouble 精度面）。
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// [0,n) 均匀（Lemire 高位乘无偏）。
    fn next_int(&mut self, n: u32) -> u32 {
        ((self.next_u64() as u128 * u128::from(n)) >> 64) as u32
    }
}
