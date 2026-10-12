//! #175 含水层波测试锁（原版机制摘要与结构复刻裁决见
//! `src/vanilla/aquifer.rs` 模块头注；债登记 docs/KNOWN-DIVERGENCE.md AQUI-1）。
//!
//! 真机症状回归面（用户 2026-10-12 报告：海平面以下洞穴大多数有水）：
//! 旧判据「y≤SEA 一律 WATER」下 SEA 以下洞穴空气干占比恒 0；原版
//! NoiseBasedAquifer 语义（Aquifer.java:149-296）是默认干、注水由胞格
//! floodedness 噪声门控。本文件三条锁：
//! (a) 统计门：海平面以下**被盖空腔**（洞穴口径，定义见 `scan_pockets`）
//!     三态占比 dry ≥ 60%；
//! (b) 囊袋并存：干洞穴、注水囊袋、岩浆三者必须同时 > 0（防一刀切反向
//!     错——全灌水和全排空都拒）；
//! (c) 同 seed 字节确定 + 异 seed 有差异（含水层域盐入域）。
//!
//! 阈值推导（不许拍脑袋，按移植噪声参数算）：floodedness =
//! NormalNoise(firstOctave −7, amps [1.0])，value_factor = 0.1667/0.2 =
//! 0.8333（span=0 → expectedDeviation 0.2），双层独立 fBm σ ≈
//! 0.8333·0.271·√2 ≈ 0.32；陆地列（地表不在海平面下 → factor=0）湿胞
//! 需噪声 > 0.4（partial 阈值，Aquifer.java:464-466），P ≈ 1−Φ(1.25σ)
//! ≈ 10%，湿胞内液面以下体积占比再乘 ~1/2 → 湿占比期望 ~5%，叠加洋底
//! 列（海陆比 ≤45%，quality.rs 同口径）与全淹尾部 ≤ ~20% 上界。干阈值
//! 60% = 期望值 3 倍裕量；(b) 岩浆非零由 −54 全局带 + lavaNoise
//! |v|>0.3（σ≈0.32 → P≈36%）保证采样窗内必现。

use mcv_core::ChunkPos;
use mcv_worldgen::vanilla::aquifer::{Aquifer, AquiferField, Substance};
use mcv_worldgen::vanilla::terrain::Orchestrator;
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

const SEED: u64 = 0xA01_F17_50B;

/// 被盖空腔扫描：固定格点上按列做密度 run-length，取「上下皆有实心」的
/// 密度≤0 连段（= 洞穴口径；排除与天空/海面水体连通的开空段——洋面水
/// 体不是洞穴，原版由 pokes/global 路直接给水体，Aquifer.java:416-424）。
/// 段内 y ∈ [MIN_Y+5, SEA) 的格按含水层终判计数（基岩带 −64..−60 排除，
/// −54 以下全局岩浆带计入三态——原版语义如此，非「洞穴」但属判据面）。
#[allow(clippy::type_complexity)]
fn scan_pockets(seed: u64, half_chunks: i32, stride: i32) -> (usize, usize, usize, usize) {
    let orch = Orchestrator::new(seed);
    let aq = Aquifer::new(seed);
    let mut field = AquiferField::new(&aq, &orch);
    let (mut dry, mut wet, mut lava, mut rock) = (0_usize, 0_usize, 0_usize, 0_usize);
    let lo = mcv_core::WORLD_MIN_Y + 5;
    let hi = mcv_core::SEA_LEVEL - 1;
    let ext = half_chunks * 16;
    let mut wx = -ext;
    while wx < ext {
        let mut wz = -ext;
        while wz < ext {
            let cs = orch.column_state(wx, wz);
            // 自底向上找被盖段
            let mut y = lo;
            while y <= hi {
                if orch.density(&cs, wx, y, wz) > 0.0 {
                    y += 1;
                    continue;
                }
                // 空腔段起点 → 找段顶
                let y0 = y;
                while y <= hi && orch.density(&cs, wx, y, wz) <= 0.0 {
                    y += 1;
                }
                let y1 = y; // 首个实心（或越界）
                let capped = y1 <= hi && orch.density(&cs, wx, y1, wz) > 0.0;
                if capped {
                    // 被盖段 [y0, y1)：逐格含水层终判
                    let skip = field.chunk_skip(wx.div_euclid(16) * 16, wz.div_euclid(16) * 16);
                    for py in y0..y1 {
                        match field.substance(wx, py, wz, orch.density(&cs, wx, py, wz), skip) {
                            Substance::Air => dry += 1,
                            Substance::Water => wet += 1,
                            Substance::Lava => lava += 1,
                            Substance::Rock => rock += 1,
                        }
                    }
                }
            }
            wz += stride;
        }
        wx += stride;
    }
    (dry, wet, lava, rock)
}

/// (a)+(b) 统计门：干洞穴多数 + 三态并存。K- 行落 CI 日志。
#[test]
fn sub_sea_caves_majority_dry_with_wet_and_lava_pockets() {
    // 采样窗 ±6 区块、列间隔 3（≈64×64 列 × ~120 y ≈ 50 万密度求值，
    // CI 秒级）。
    let (dry, wet, lava, rock) = scan_pockets(SEED, 6, 3);
    let cav = dry + wet + lava;
    // 样本量下限：确保三态统计有意义（洞穴密度 ~5% 口径下本窗应有数千格）。
    assert!(cav >= 1000, "被盖空腔样本过少 {cav}，统计门失效");
    let dry_ratio = dry as f64 / cav as f64;
    eprintln!(
        "K-AQUIFER dry={dry} wet={wet} lava={lava} rock={rock} \
         dry_share={:.3} n={cav}",
        dry_ratio
    );
    // (a) 原版语义主门：干占比 ≥ 0.60（推导见文件头注；旧平铺模型恒 0）。
    assert!(
        dry_ratio >= 0.60,
        "海下洞穴干占比 {dry_ratio:.3} < 0.60（旧「一律灌水」回归？）"
    );
    // (b) 囊袋并存：注水与岩浆都必须 >0（全排空 = 另一侧的一刀切错）。
    assert!(wet > 0, "无注水囊袋——含水层噪声门失效（全排空）");
    assert!(lava > 0, "无岩浆（−54 全局带/lavaNoise 翻转型都不存在？）");
    // 湿占比上界（推导 ≤~20%，3 倍裕量放宽到 35%）：
    let wet_ratio = (wet + lava) as f64 / cav as f64;
    assert!(
        wet_ratio <= 0.35,
        "湿+岩浆占比 {wet_ratio:.3} 超上界，判据偏湿"
    );
}

/// (c) 同 seed 同字节（含水层并入后确定性契约面）+ 异 seed 差异。
#[test]
fn aquifer_determinism_bytes() {
    let a =
        generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(2, -3)).expect("gen");
    let b =
        generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(2, -3)).expect("gen");
    assert_eq!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
    assert_eq!(a.heightmap, b.heightmap);
    // 异 seed：含水层通道已入域盐——同坐标体素应有差异
    let c = generate_terrain_with(TerrainBackend::Vanilla, SEED ^ 0x5EED, ChunkPos::new(2, -3))
        .expect("gen");
    assert_ne!(a.voxels.as_u16_slice(), c.voxels.as_u16_slice());
}

/// 真实区块缓冲三态冒烟：±3 区块生成缓冲中，海平面以下必须同时存在
/// AIR（干洞穴）与 WATER/LAVA 体素（或至少与旧「全水」判据有本质差异：
/// 海下 AIR 体素占比 > 30%——旧模型该值恒 0）。
#[test]
fn generated_chunks_have_dry_caves_below_sea() {
    const AIR: u16 = 0;
    const WATER: u16 = 5;
    const LAVA: u16 = 516;
    let (mut air, mut water, mut lava) = (0usize, 0usize, 0usize);
    for cz in -3..3 {
        for cx in -3..3 {
            let t = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(cx, cz))
                .expect("gen");
            let v = t.voxels.as_u16_slice();
            for y in (mcv_core::WORLD_MIN_Y + 5)..mcv_core::SEA_LEVEL {
                for z in 0..16usize {
                    for x in 0..16usize {
                        match v[mcv_core::vidx(x, y, z)] {
                            AIR => air += 1,
                            WATER => water += 1,
                            LAVA => lava += 1,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    let tot = air + water + lava;
    eprintln!(
        "K-AQUIFER-BUF air={air} water={water} lava={lava} dry_share={:.3} n={tot}",
        air as f64 / tot.max(1) as f64
    );
    assert!(tot > 10_000, "样本异常小 {tot}");
    assert!(
        air as f64 / tot as f64 > 0.30,
        "海下空气占比 {:.3} ≤ 0.30（含水层未生效或全灌水回归）",
        air as f64 / tot as f64
    );
}
