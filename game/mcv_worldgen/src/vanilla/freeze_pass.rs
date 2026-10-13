//! #191 积雪/冻水 pass — 26.1 `freeze_top_layer` 特征实物移植
//! （`SnowAndFreezeFeature.java:23-49`，Feature.java:81-82 注册；
//! 特征阶 = biome JSON `features` 步 10「freeze」，植被阶之后——
//! 本仓调用点同步置树 pass 后、高度图重算前）。
//!
//! 实物语义（逐条对位）：
//! - 每列 `y = getHeight(MOTION_BLOCKING)`：MOTION_BLOCKING 含流体/叶，
//!   返回顶格 +1 → 本仓自顶向下第一个非空气格 `ty`，`topPos = ty+1`、
//!   `belowPos = ty`；
//! - `shouldFreeze(belowPos, checkNeighbors=false)`（Biome.java:145-168，
//!   特征传 checkNeighbors=false → 不做四邻水检查）：**无降水标志门**
//!   （实物 warmEnoughToRain 只看温度，Biome.java:145-146,175-177），
//!   高度调整温度 < 0.15 且 belowPos 为水 → ICE；
//! - `shouldSnow(topPos)`（Biome.java:183-195）：降水判据 =
//!   `has_precipitation && 高度调整温度 < 0.15`（getPrecipitationAt →
//!   coldEnoughToSnow → warmEnoughToRain 取反，Biome.java:104-111/171-181），
//!   topPos 空气 且 `canSurvive`（SnowLayerBlock：承托格
//!   canSupportRigidBlock = 完整面实心——叶不承托，树冠上无雪片）→
//!   置雪层（本仓单层，`snow` id 930）；下方草块补
//!   `SnowyBlock.SNOWY=true`（本仓单方块表示 = `snow_grass`，表层规则
//!   已按群系产出，见 KNOWN-DIVERGENCE SURF-1）。
//!
//! 温度口径（#191 尾款：全式实物化）：判据温度 =
//! [`biomes::height_adjusted_temperature`]（Biome.java:112-121 逐句转写：
//! y ≤ sea+17 取基础温度，其上扣 `(TEMPERATURE_NOISE(x/8,z/8)·8 +
//! y−(sea+17))·0.05/40`，噪声 = `PerlinSimplexNoise(
//! LegacyRandomSource(1234), [0])` 常量种子移植，见
//! [`crate::vanilla::simplex`]）。原「基础温度 < 0.15」近似遗留的
//! taiga（0.25）高山模糊带（y ∈ (152,168] 噪声仲裁、y > 168 恒雪）
//! 随本刀做实——此前该带保守少雪，现与实物同判。列级预剪枝
//! [`coldest_possible`] 只做结果等价裁剪（全高域最冷仍 ≥ 0.15 的群系
//! 整列跳过，本表下 = 除 taiga/snowy_plains 外 8 群系，判定结果与
//! 逐列求值一致）。

use crate::trees::ids;
use crate::vanilla::biomes::{self, Biome};
use crate::vanilla::terrain::{AIR, GRASS, MIN_Y, SNOW_GRASS, WATER};

/// `blocks_gen.inc.rs:493 /*472*/ ("ice", …)`。
pub const ICE: u16 = 472;
/// `blocks_gen.inc.rs:951 /*930*/ ("snow", …)`——雪层（非立方形状，
/// shape.rs SnowLayerBlock 分支渲染）。
pub const SNOW_LAYER: u16 = 930;

/// 实物 `Biome.getPrecipitationAt(...) == SNOW`（Biome.java:104-111 +
/// coldEnoughToSnow 171-177）：`has_precipitation && 高度调整温度 < 0.15`。
/// `x/y/z` 为判据位置的绝对世界坐标（实物 BlockPos 语义）。
#[must_use]
pub fn snows_at(b: Biome, x: i32, y: i32, z: i32) -> bool {
    biomes::climate(b).has_precipitation
        && biomes::height_adjusted_temperature(b, x, y, z, mcv_core::SEA_LEVEL) < 0.15
}

/// 群系全高域最冷调整温度（列级预剪枝用下界）：
/// `base − (8 + max_dy)·0.05/40`，`max_dy = WORLD_MAX_Y-1 − (sea+17)`，
/// 噪声上界 v = +8（simplex ×8）。y ≤ sea+17 处修正不生效、值 = base
/// ≥ 该下界，故为全域有效下界；下界仍 ≥ 0.15 → 冻水/积雪两判据恒假。
#[must_use]
fn coldest_possible(b: Biome) -> f32 {
    let base = biomes::climate(b).temperature as f32;
    let max_dy = mcv_core::WORLD_MAX_Y - 1 - (mcv_core::SEA_LEVEL + 17);
    base - (8.0_f32 + max_dy as f32) * 0.05 / 40.0
}

/// 叶不承托雪片（SnowLayerBlock.canSurvive → canSupportRigidBlock，
/// 叶非完整面；id 表 = trees::ids 同一批）。
#[must_use]
fn supports_snow_layer(id: u16) -> bool {
    !matches!(
        id,
        ids::OAK_LEAVES
            | ids::ACACIA_LEAVES
            | ids::BIRCH_LEAVES
            | ids::DARK_OAK_LEAVES
            | ids::JUNGLE_LEAVES
            | ids::SPRUCE_LEAVES
    )
}

/// 对整区块施加 freeze_top_layer（原地改体素缓冲；调用方 =
/// `terrain::generate`，树 pass 后、高度图重算前——原版特征阶序）。
pub fn apply(
    orch: &crate::vanilla::terrain::Orchestrator,
    chunk_x: i32,
    chunk_z: i32,
    voxels: &mut [u16],
) {
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    for cz in 0..16_i32 {
        for cx in 0..16_i32 {
            let wx = base_x.wrapping_add(cx);
            let wz = base_z.wrapping_add(cz);
            let biome = orch.biome_at(wx, wz);
            // 列级预剪枝（判定结果等价，coldest_possible 头注）：
            // 全高域最冷仍 ≥ 0.15 → 冻水/积雪两判据恒假。
            if coldest_possible(biome) >= 0.15 {
                continue;
            }
            // MOTION_BLOCKING 顶（含流体/叶）：自顶向下第一个非空气。
            let mut ty = None;
            for y in (MIN_Y..mcv_core::WORLD_MAX_Y).rev() {
                if voxels[mcv_core::vidx(cx as usize, y, cz as usize)] != AIR {
                    ty = Some(y);
                    break;
                }
            }
            let Some(top) = ty else { continue };
            let below = mcv_core::vidx(cx as usize, top, cz as usize);
            // shouldFreeze（checkNeighbors=false 路）：**无降水标志门**、
            // 只看 belowPos 高度调整温度（Biome.java:145-146）；水 → 冰。
            if voxels[below] == WATER
                && biomes::height_adjusted_temperature(biome, wx, top, wz, mcv_core::SEA_LEVEL)
                    < 0.15
            {
                voxels[below] = ICE;
            }
            // shouldSnow：topPos 空气 + 承托面 → 雪层；草补 snowy 属性
            // （本仓 snow_grass 单方块表示，表层规则已按群系产出，
            // 此处兜住 freeze 带内非表层规则产出的裸 grass 列）。
            let Some(snow_y) = top.checked_add(1) else {
                continue;
            };
            if snow_y >= mcv_core::WORLD_MAX_Y {
                continue;
            }
            let at = mcv_core::vidx(cx as usize, snow_y, cz as usize);
            let ground = voxels[below];
            // shouldSnow 降水判据在 topPos=(wx, snow_y, wz) 求值
            // （Biome.java:183-186 getPrecipitationAt(topPos)，含
            // has_precipitation 门）。
            if voxels[at] == AIR && supports_snow_layer(ground) && snows_at(biome, wx, snow_y, wz) {
                voxels[at] = SNOW_LAYER;
                if ground == GRASS {
                    voxels[below] = SNOW_GRASS;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vanilla::terrain::Orchestrator;

    /// 积雪判据表驱动（10 群系 × 实物气候表，sea+17 以下无高度修正
    /// 区段：唯一 snowy = SnowyPlains；数值 = 26.1 biome JSON 抄录，
    /// biomes.rs 表锁同源）。
    #[test]
    fn snowy_predicates_match_climate_table() {
        for b in biomes::ALL_BIOMES {
            let c = biomes::climate(b);
            let want = c.has_precipitation && c.temperature < 0.15;
            assert_eq!(snows_at(b, 0, 63, 0), want, "{b:?}");
            let expect_snowy = matches!(b, Biome::SnowyPlains);
            assert_eq!(snows_at(b, 0, 63, 0), expect_snowy, "{b:?} 判据对表");
        }
    }

    /// 种子扫描集 × chunk(0,0) 全列，返回首个 taiga 列；找不到直接
    /// panic（#191 尾款③：防真空通过——旧 leaves 测试暖 seed 静默 return）。
    fn first_taiga_column() -> (u64, i32, i32) {
        for seed in [1u64, 7, 13, 42, 99, 123, 777, 2026, 31337, 555] {
            let orch = Orchestrator::new(seed);
            for cz in 0..16_i32 {
                for cx in 0..16_i32 {
                    if orch.biome_at(cx, cz) == Biome::Taiga {
                        return (seed, cx, cz);
                    }
                }
            }
        }
        panic!("种子集合内未找到 taiga 列——请扩种子集而非放宽断言");
    }

    /// 手搭列：雪原群系柱（水湖列 + 草列 + 叶列）→ 冰/雪层/无雪正确
    /// 落位；非雪原列不动。用真实 Orchestrator 会耦合噪声世界，这里
    /// 直接测列内核心（列扫描/替换逻辑抽列级函数不便——以 apply 全块
    /// 跑一遍，断言按 biome_at 实际结果分支：全 air 块恒等 + 手工注入
    /// 顶格语义）。
    #[test]
    fn apply_freezes_and_snows_only_cold_columns() {
        let seed = 2026;
        let orch = Orchestrator::new(seed);
        let mut vox = vec![0u16; mcv_core::CHUNK_VOL];
        // 逐列在地面高度（63）放草，水面列（cx=0,cz=0）放水。
        for cz in 0..16_i32 {
            for cx in 0..16_i32 {
                vox[mcv_core::vidx(cx as usize, 63, cz as usize)] =
                    if cx == 0 && cz == 0 { WATER } else { GRASS };
            }
        }
        apply(&orch, 0, 0, &mut vox);
        // 断言逐列与实物两判据一致（y=63/64 ≤ sea+17=80，无高度修正段）：
        // 冻水门 = 调整温度 < 0.15（belowPos 处，无降水门）；
        // 积雪门 = has_precipitation && 调整温度 < 0.15（topPos 处）。
        for cz in 0..16_i32 {
            for cx in 0..16_i32 {
                let b = orch.biome_at(cx, cz);
                let freeze =
                    biomes::height_adjusted_temperature(b, cx, 63, cz, mcv_core::SEA_LEVEL) < 0.15;
                let snow = snows_at(b, cx, 64, cz);
                let ground = vox[mcv_core::vidx(cx as usize, 63, cz as usize)];
                let above = vox[mcv_core::vidx(cx as usize, 64, cz as usize)];
                if cx == 0 && cz == 0 {
                    assert_eq!(ground, if freeze { ICE } else { WATER }, "水列冻冰门");
                } else {
                    assert_eq!(
                        ground,
                        if snow { SNOW_GRASS } else { GRASS },
                        "积雪草列 snowy 改写"
                    );
                }
                assert_eq!(above, if snow { SNOW_LAYER } else { AIR }, "雪层门");
            }
        }
    }

    /** 高度调整温度回归锁（#191 尾款②③）：taiga（基础 0.25，旧判据
    「<0.15」恒假 → 高山带此前**不雪**）高山列 y=200/雪位 201 > 168
    （任意噪声值 v 下 v+dy ≥ 113 > 80，biomes.rs 界带锁同源推演）实物化
    积雪；低山对照 y=150（≤152 恒 ≥0.15 带）前后一致不雪——锁死「错雪」
    方向，只放开「少雪」方向。 */
    #[test]
    fn taiga_high_alpine_column_snows_regression() {
        let (seed, cx, cz) = first_taiga_column();
        let orch = Orchestrator::new(seed);
        // 环境自证：该列雪位确实越界。
        assert!(
            snows_at(orch.biome_at(cx, cz), cx, 201, cz),
            "taiga y=201 判据应越界 (seed={seed},col=({cx},{cz}))"
        );
        let mut vox = vec![0u16; mcv_core::CHUNK_VOL];
        vox[mcv_core::vidx(cx as usize, 200, cz as usize)] = GRASS;
        apply(&orch, 0, 0, &mut vox);
        assert_eq!(
            vox[mcv_core::vidx(cx as usize, 201, cz as usize)],
            SNOW_LAYER,
            "taiga 高山列实物化积雪（本刀前为「不雪」——回归锁）"
        );
        assert_eq!(
            vox[mcv_core::vidx(cx as usize, 200, cz as usize)],
            SNOW_GRASS,
            "高山草块补 snowy"
        );
        // 低山对照：y=150（雪位 151 ≤ sea+17+…界带内恒 ≥0.15）不得雪。
        assert!(
            !snows_at(orch.biome_at(cx, cz), cx, 151, cz),
            "y=151 恒不雪"
        );
        let mut vox2 = vec![0u16; mcv_core::CHUNK_VOL];
        vox2[mcv_core::vidx(cx as usize, 150, cz as usize)] = GRASS;
        apply(&orch, 0, 0, &mut vox2);
        assert_eq!(vox2[mcv_core::vidx(cx as usize, 151, cz as usize)], AIR);
        assert_eq!(vox2[mcv_core::vidx(cx as usize, 150, cz as usize)], GRASS);
    }

    /// 叶不承托（canSupportRigidBlock）：叶顶列无雪片——**真雪环境**下
    /// 触发（#191 尾款③：旧版在暖 seed 下 `cold_at=None → return` 真空
    /// 通过；现锚定 taiga 高山列，先断言环境恒雪再验叶拒雪）。
    #[test]
    fn leaves_do_not_support_snow() {
        let (seed, cx, cz) = first_taiga_column();
        let orch = Orchestrator::new(seed);
        assert!(
            snows_at(orch.biome_at(cx, cz), cx, 201, cz),
            "环境必须真雪，否则本测试退化为真空通过 (seed={seed})"
        );
        let mut vox = vec![0u16; mcv_core::CHUNK_VOL];
        vox[mcv_core::vidx(cx as usize, 200, cz as usize)] = ids::SPRUCE_LEAVES;
        apply(&orch, 0, 0, &mut vox);
        assert_eq!(
            vox[mcv_core::vidx(cx as usize, 201, cz as usize)],
            AIR,
            "叶上不得积雪（canSupportRigidBlock）"
        );
        assert_eq!(
            vox[mcv_core::vidx(cx as usize, 200, cz as usize)],
            ids::SPRUCE_LEAVES,
            "叶不受 snowy 改写"
        );
    }
}
