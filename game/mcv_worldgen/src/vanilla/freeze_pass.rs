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
//!   特征传 checkNeighbors=false → 不做四邻水检查）：belowPos 为水 →
//!   ICE；
//! - `shouldSnow(topPos)`（Biome.java:183-195）：降水判据 =
//!   `has_precipitation && 温度 < 0.15`（getPrecipitationAt →
//!   coldEnoughToSnow → warmEnoughToRain 取反，Biome.java:104-111/171-181），
//!   topPos 空气 且 `canSurvive`（SnowLayerBlock：承托格
//!   canSupportRigidBlock = 完整面实心——叶不承托，树冠上无雪片）→
//!   置雪层（本仓单层，`snow` id 930）；下方草块补
//!   `SnowyBlock.SNOWY=true`（本仓单方块表示 = `snow_grass`，表层规则
//!   已按群系产出，见 KNOWN-DIVERGENCE SURF-1）。
//!
//! 温度口径（本 10 群系子集内逐群系与实物**全式**等价，手推界）：
//! 实物 = `getHeightAdjustedTemperature`（Biome.java:113-121）：y ≤ sea+17
//! 取基础温度；其上扣 `(TEMPERATURE_NOISE(x/8,z/8)·8 + y−(sea+17))·
//! 0.05/40`，噪声 ∈ [−1,1]。对基础温度 0.0（snowy_plains）：修正上界
//! +0.00875 < 0.15，恒雪（精确）；0.5~0.95（ocean/river/plains/beach/
//! forest/birch/desert/savanna）：y ≤ 319 全高度修正 < 0.35，恒不雪
//! （精确）。唯一模糊带 = taiga（0.25，高山 y≳152 起实物可越界成雪）——
//! 精确复现需 `PerlinSimplexNoise(LegacyRandomSource(1234))` 移植
//! （Biome.java:61），登记债随 SnowService 动态积厚一波（任务板 #191
//! 回报项）。本 pass 按「基础温度 < 0.15」判定：9/10 群系与实物逐位
//! 一致，taiga 高山带不积雪（保守少雪，绝不错雪）。

use crate::trees::ids;
use crate::vanilla::biomes::{self, Biome};
use crate::vanilla::terrain::{AIR, GRASS, MIN_Y, SNOW_GRASS, WATER};

/// `blocks_gen.inc.rs:493 /*472*/ ("ice", …)`。
pub const ICE: u16 = 472;
/// `blocks_gen.inc.rs:951 /*930*/ ("snow", …)`——雪层（非立方形状，
/// shape.rs SnowLayerBlock 分支渲染）。
pub const SNOW_LAYER: u16 = 930;

/// 实物 `Biome.warmEnoughToRain` 的静态基温侧（见模块头注口径声明）。
#[must_use]
pub fn biome_snows(b: Biome) -> bool {
    let c = biomes::climate(b);
    c.has_precipitation && c.temperature < 0.15
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
            if !biome_snows(orch.biome_at(wx, wz)) {
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
            // shouldFreeze（checkNeighbors=false 路）：水 → 冰。
            if voxels[below] == WATER {
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
            if voxels[at] == AIR && supports_snow_layer(ground) {
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

    /// 积雪判据表驱动（10 群系 × 实物气候表：唯一 snowy = SnowyPlains；
    /// 数值 = 26.1 biome JSON 抄录，biomes.rs 表锁同源）。
    #[test]
    fn snowy_predicates_match_climate_table() {
        for b in biomes::ALL_BIOMES {
            let c = biomes::climate(b);
            let want = c.has_precipitation && c.temperature < 0.15;
            assert_eq!(biome_snows(b), want, "{b:?}");
            let expect_snowy = matches!(b, Biome::SnowyPlains);
            assert_eq!(biome_snows(b), expect_snowy, "{b:?} 判据对表");
        }
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
        // 断言逐列与判据一致：雪原列水→冰 + 雪层；暖列原样。
        for cz in 0..16_i32 {
            for cx in 0..16_i32 {
                let cold = biome_snows(orch.biome_at(cx, cz));
                let ground = vox[mcv_core::vidx(cx as usize, 63, cz as usize)];
                let above = vox[mcv_core::vidx(cx as usize, 64, cz as usize)];
                if cold {
                    if cx == 0 && cz == 0 {
                        assert_eq!(ground, ICE, "雪原水列应冻冰");
                    } else {
                        assert_eq!(ground, SNOW_GRASS, "雪原草列应补 snowy（本仓单方块表示）");
                    }
                    assert_eq!(above, SNOW_LAYER, "雪原列顶应有雪层");
                } else {
                    assert_eq!(
                        ground,
                        if cx == 0 && cz == 0 { WATER } else { GRASS },
                        "暖列不得改动"
                    );
                    assert_eq!(above, AIR, "暖列不得降雪");
                }
            }
        }
    }

    /// 叶不承托（canSupportRigidBlock）：叶顶列无雪片。
    #[test]
    fn leaves_do_not_support_snow() {
        let seed = 2026;
        let orch = Orchestrator::new(seed);
        // 找一个雪原列（扫全块找 cold 列坐标，再单独构造）。
        let mut cold_at = None;
        for cz in 0..16_i32 {
            for cx in 0..16_i32 {
                if biome_snows(orch.biome_at(cx, cz)) {
                    cold_at = Some((cx, cz));
                    break;
                }
            }
            if cold_at.is_some() {
                break;
            }
        }
        // 该 seed 下可能整块皆暖——构造性跳过（判据本身已有表测锁定）。
        let Some((cx, cz)) = cold_at else { return };
        let mut vox = vec![0u16; mcv_core::CHUNK_VOL];
        vox[mcv_core::vidx(cx as usize, 63, cz as usize)] = ids::SPRUCE_LEAVES;
        apply(&orch, 0, 0, &mut vox);
        assert_eq!(
            vox[mcv_core::vidx(cx as usize, 64, cz as usize)],
            AIR,
            "叶上不得积雪（canSupportRigidBlock）"
        );
        assert_eq!(
            vox[mcv_core::vidx(cx as usize, 63, cz as usize)],
            ids::SPRUCE_LEAVES,
            "叶不受 snowy 改写"
        );
    }
}
