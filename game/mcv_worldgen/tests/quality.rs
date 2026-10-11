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
/// 地形保真波 T2 重锚（2026-10-11）：气候 xz_scale 还原 0.25（波长 2048 格）
/// 后，±2048 窗只覆盖 1 个波长、群系出现与否强依赖窗口落位 → 窗扩 ±8192
///（stride 16→64，样本数不变 65536）。点表同步升级为 OBB span 平铺分区表
/// （src/vanilla/biomes.rs 头注）：/root/audit/_sim3.py 移植复算校准，
/// 10 群系 3.8%-29.5%（seed 0x2a/0x7EA20B1），带 2%-35% 原样保留。
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
    let stride = 64_i32; // 每 64 列采一列：±8192 格 ≈ 8 个大陆度波长
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
///    25%-65%。
///
/// 地形保真波 T2 重锚（2026-10-11）：原带 25%-45% 是 XS=2.0（气候波长 ÷8）
/// 时代校准的；恢复原版 xz_scale 0.25（NoiseRouterData.java:90-96）后大陆度
/// 波长 2048 格，±512 窗只覆盖 0.5 个波长、失去代表性（seed 0x7EA20B1 原点
/// 区是 88% 深海的单窗假象）。窗扩到 ±8192（8 个波长，stride 64，样本数不变
/// 65536）；带按移植复算器 /root/audit/_sim3.py 重锚：去 BACKBONE_BIAS 后宽窗
/// 海占比 48-50%（seed 0x7EA20B1/42），带取 25-65%——「非全陆非全海」的
/// 原意图不变，数值随原版参数真相走。
#[test]
fn sea_land_ratio_in_bands() {
    let stride = 64_i32;
    let half = 128;
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
        (25.0..=65.0).contains(&pct),
        "海陆比 {pct:.2}% 越带（sea={sea} total={total}）"
    );
}

/// 6) 平原窗占比（地形保真波 T2 主门，任务卡 #122/triage ①目标「大平原出现」）：
///    样条地表（`column_state().surface_ours` 口径 = 128+128·offset 面）在
///    4 个 512×512 格区域（stride 2 采样）内，16×16 样本窗（=32×32 格）
///    「全陆（>海平面占 >98%）且窗 std ≤2」的占比 ≥ 0.10。
///
/// 校准（/root/audit/_sim3.py 移植复算，seed 0x7EA20B1/42 同口径）：
/// 现配置（XS=2.0 + BACKBONE_BIAS 0.0671875）**0.000-0.007**；恢复 xz_scale
/// 0.25 + 删 BIAS 后 **0.199-0.207**；triage 原版口径（1 格步距 16×16 窗，
/// XS 0.25 + BlendedNoise 级 σ）0.41。带取 ≥0.10：高于现配置 14 倍、低于
/// 修后值一半，专锁「气候波长 ÷8 / 解常数偏置」两类回归。
#[test]
fn plains_share_in_band() {
    let orch = wg::terrain::Orchestrator::new(SEED);
    let sea = f64::from(SEA_LEVEL);
    let mut ok = 0usize;
    let mut tot = 0usize;
    for (ox, oz) in [(0i32, 0i32), (4096, 0), (0, 4096), (4096, 4096)] {
        let mut ys = [0i32; 128 * 128];
        for (k, y) in ys.iter_mut().enumerate() {
            let i = k % 128;
            let j = k / 128;
            *y = orch
                .column_state(ox + (i * 2) as i32, oz + (j * 2) as i32)
                .surface_ours;
        }
        for j in (0..128 - 15).step_by(8) {
            for i in (0..128 - 15).step_by(8) {
                let mut land = 0f64;
                let mut sum = 0f64;
                let mut sum2 = 0f64;
                for dj in 0..16usize {
                    for di in 0..16usize {
                        let y = f64::from(ys[(j + dj) * 128 + (i + di)]);
                        if y > sea {
                            land += 1.0;
                        }
                        sum += y;
                        sum2 += y * y;
                    }
                }
                tot += 1;
                let mean = sum / 256.0;
                let var = (sum2 / 256.0 - mean * mean).max(0.0);
                if land / 256.0 > 0.98 && var.sqrt() <= 2.0 {
                    ok += 1;
                }
            }
        }
    }
    let share = ok as f64 / tot as f64;
    assert!(
        share >= 0.10,
        "平原窗占比 {share:.3} < 0.10（ok={ok} tot={tot}）——气候波长/偏置回归"
    );
}

/// 7) jaggedness 0.65 点第 4 标量 = 0.0（地形保真波 T3，1 字符修）：
///    TerrainProvider.java:61 `overworldJaggedness` 的 farInlandMiddle=0.65 点是
///    `buildErosionJaggednessSpline(…, 1.0F, 1.0F, 1.0F, 0.0F, …)`——高侵蚀带
///    山脊中段锯齿恒 0（第 4 标量 = RidgesFolded=1.0 端点的 atHigh）。
///    旧码抄录漂移为 (1,1,1,1)，高侵蚀带高原顶部意外获得锯齿而过碎。
///    探针：continents=0.65 × erosion=-0.6（落入 erosion_jaggedness 的
///    [-0.78,-0.5775] 段 = ridge_jaggedness(1.0, 第4参)）× ridges_folded=0.45
///    （pnv(0.4)=0.2 与 pnv(0.5666…)=0.7 的中段节点）：第 4 参 0 ⇒ 节点值 0；
///    第 4 参 1 ⇒ weirdness_jaggedness(1.0) ≈ 0.465。
#[test]
fn jaggedness_far_inland_high_erosion_middle_is_zero() {
    let jag = wg::terrain::overworld_jaggedness_spline();
    let c = wg::spline::Climate {
        continents: 0.65,
        erosion: -0.6,
        ridges: 0.0,
        ridges_folded: 0.45,
    };
    let v = jag.eval(&c);
    assert!(
        v.abs() < 1e-9,
        "0.65 点高侵蚀带山脊中段锯齿应为 0（TerrainProvider.java:61 第 4 标量 0.0），实得 {v}"
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
