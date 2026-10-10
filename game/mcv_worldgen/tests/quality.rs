//! vanilla 后端质量统计断言 — 防「形似神不似」（任务板 #91 完成标准）：
//! 1. 多样本群系占比各在 2%-35%（10 群系全部可达）；
//! 2. 海陆比（地表低于海平面（`mcv_core::SEA_LEVEL`，v6=63）的比例）25%-45%；
//! 3. 洞穴空气占比 3%-12%；
//! 4. 相邻列高度连续率 ≥99%（无断崖墙）；
//! 5. 同 seed 同字节确定性。
//!
//! 采样口径：3D 指标用列/单元级函数直接采样（生成路径另有全区块确定性
//! 测试锁字节），同 seed 下与分块生成同构、成本可控。

use mcv_core::{ChunkPos, SEA_LEVEL};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

const SEED: u64 = 0x7E_A20_0B1_u64;

/// 简单 LCG 用于确定性地采样伪随机坐标（与生成通路不同源）。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

use mcv_worldgen::vanilla as wg;

/// 1) 群系占比：512×512 列网格（区块级稀疏采样），各群系 2%-35%。
///
/// River 一度实测 1.76% 越下界——裁决为**修生成器**（非放松本带）：26.1 的
/// river 是 weirdness 谷地带 span(-0.05,0.05)（OverworldBiomeBuilder.java:217
/// addValleys）在特定 cont/erosion 组合的细条带（:753-815），其占比由
/// `NormalNoise` 双层独立语义决定；src/vanilla/noise.rs 曾两层共用倍频种子
/// （NormalNoise.java:51-53 要求两次 create 各自 fork，XoroshiroRandomSource
/// .java:38-40），通道 σ 抬高 ~1.4× 使谷地带占比塌缩。独立化后 River
/// 回到 ~3.9%（两 seed 验证），本带 2%-35% 原样保留。
#[test]
fn biome_distribution_in_bands() {
    let mut counts = std::collections::HashMap::new();
    let stride = 16_i32; // 每 16 列采一列 ≈ 每 2×2 区块网格采样
    let half = 128;
    let mut total = 0usize;
    for dz in -half..half {
        for dx in -half..half {
            let wx = dx * stride;
            let wz = dz * stride;
            let b = wg::biome_at(SEED, wx, wz);
            *counts.entry(b).or_insert(0usize) += 1;
            total += 1;
        }
    }
    for (b, c) in &counts {
        let pct = 100.0 * *c as f64 / total as f64;
        assert!(
            (2.0..=35.0).contains(&pct),
            "群系 {b:?} 占比 {pct:.2}% 越带（c={c} total={total}）"
        );
    }
    // 10 群系全部可达
    for want in [
        wg::Biome::Plains,
        wg::Biome::Forest,
        wg::Biome::BirchForest,
        wg::Biome::Desert,
        wg::Biome::Savanna,
        wg::Biome::Taiga,
        wg::Biome::SnowyPlains,
        wg::Biome::Ocean,
        wg::Biome::Beach,
        wg::Biome::River,
    ] {
        let c = counts.get(&want).copied().unwrap_or(0);
        assert!(c > 0, "群系 {want:?} 不可达");
    }
}

/// 2) 海陆比：地表 < 海平面（v6 = 63，引 `SEA_LEVEL` 不钉字面量）的列占比
/// 25%-45%。
#[test]
fn sea_land_ratio_in_bands() {
    let stride = 8_i32;
    let half = 64;
    let mut sea = 0usize;
    let mut total = 0usize;
    for dz in -half..half {
        for dx in -half..half {
            let y = wg::surface_block_y(SEED, dx * stride, dz * stride);
            if y < SEA_LEVEL {
                sea += 1;
            }
            total += 1;
        }
    }
    let pct = 100.0 * sea as f64 / total as f64;
    assert!(
        (25.0..=45.0).contains(&pct),
        "海陆比 {pct:.2}% 越带（sea={sea} total={total}）"
    );
}

/// 3) 洞穴空气占比：地下随机单元中空气占比 3%-12%。
///
/// 一度实测 18.39% 越上界——裁决为**修生成器**（非放松本带）：src/vanilla/
/// terrain.rs 的洞穴通道原为裸单倍频 OS2S，值分布远宽于 26.1 NormalNoise
/// 归一（cave_cheese/cave_entrance/cave_layer 等 json 参数表，σ≈0.14-0.30），
/// 且 cave_layer/cave_cheese 轴错位、spaghetti_2d_elevation 误作 3D 且值域
/// [-64,0]（26.1 为 2D 列常量、mapFromUnitTo [-8,+8]，NoiseRouterData
/// .java:273-278）。按 26.1 逐通道对位后实测 ~7.5-8.6%（两 seed），本带
/// 3%-12% 原样保留。
#[test]
fn cave_air_in_bands() {
    let orch = wg::terrain::Orchestrator::new(SEED);
    let mut rng = Rng(0x5EED_0001_u64);
    let mut air = 0usize;
    let total = 20_000usize;
    for _ in 0..total {
        let wx = (rng.below(1024) as i32) - 512;
        let wz = (rng.below(1024) as i32) - 512;
        let cs = orch.column_state(wx, wz);
        // 地下主体带（v6 绝对 y）：旧 256 基准 y∈[8,88)（全部低于旧地表
        // 最低点 96）；密度场以 mc 空间书写且 v6 恒等映射，取同一段
        // mc 区间 [−52,68)（旧 [8,88) 经 y_mc=1.5y−64 的原像带）。
        let y = rng.below(120) as i32 - 52;
        if y < cs.surface_ours && orch.density(&cs, wx, y, wz) <= 0.0 {
            air += 1;
        }
    }
    let pct = 100.0 * air as f64 / total as f64;
    assert!(
        (3.0..=12.0).contains(&pct),
        "洞穴空气占比 {pct:.2}% 越带（air={air} total={total}）"
    );
}

/// 4) 相邻列高度连续率：相邻列高度差 ≤32 的比例 ≥99%。
#[test]
fn column_continuity() {
    let stride = 1_i32;
    let half = 32;
    let mut cont = 0usize;
    let mut total = 0usize;
    for dz in -half..half {
        for dx in -half..half {
            let a = wg::surface_block_y(SEED, dx * stride, dz * stride);
            let b = wg::surface_block_y(SEED, (dx + 1) * stride, dz * stride);
            if (a - b).abs() <= 32 {
                cont += 1;
            }
            total += 1;
        }
    }
    let pct = 100.0 * cont as f64 / total as f64;
    assert!(pct >= 99.0, "列连续率 {pct:.2}% < 99%（cont={cont}）");
}

/// 5) 确定性：同 seed 同字节；异 seed 不同。
#[test]
fn same_seed_deterministic() {
    let a =
        generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(3, -7)).expect("gen");
    let b =
        generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(3, -7)).expect("gen");
    assert_eq!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
    assert_eq!(a.heightmap[..], b.heightmap[..]);
    let c = generate_terrain_with(TerrainBackend::Vanilla, SEED ^ 0xA5A5, ChunkPos::new(3, -7))
        .expect("gen");
    assert_ne!(a.voxels.as_u16_slice(), c.voxels.as_u16_slice());
    // 非零土壤（非全空气区块）
    assert!(a.voxels.as_u16_slice().iter().any(|&v| v != 0));
}
