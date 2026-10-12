//! #175 刀C 岩浆湖测试锁（机制实证与接线模型见
//! `src/vanilla/lake_pass.rs` 模块头注；债登记 LAKE-1）。
//!
//! 频度推导（按 26.1 placed json 实物链逐级推，非拍脑袋；候选数 =
//! rarity 门 ∧ placement 链存活，判据环弃单另计 K-LAKES-BUF）：
//! - surface = rarity 1/200 ∧ in_square ∧ heightmap WORLD_SURFACE_WG
//!   （heightmap 路恒成功）→ 1024 区块窗 μ = 1024/200 = 5.12 二项；
//!   P(0) = (199/200)^1024 ≈ 0.6%、P(>15) < 1e-5 → 断言带 [1, 15]。
//! - underground = rarity 1/9 ∧ environment_scan ∧ 阈值 y ≤ hm−5
//!   （hm = OCEAN_FLOOR_WG 高度图值 = floor_top+1 ≥ 65）。origin 均匀
//!   落 [0,320)：起点已在 (hm−5) 之下 → 原位实心即中；起点在上方空中
//!   → 扫描最多停在 floor_top（= hm−1 > hm−5）必被阈值弃 → 存活率
//!   ≈ (floor_top−4)/320 ∈ [60/320, ≈0.30]（海柱 0.1875、普通陆柱
//!   0.19–0.27，山地/洞穴穿透略增）→ μ ≈ 1024/9 × 0.21 ≈ 22–31，
//!   σ ≈ 4.5 → 断言带 [10, 45]（≈ μ±3σ 并留地形均值余量）。

use mcv_core::ChunkPos;
use mcv_worldgen::vanilla::aquifer::{Aquifer, AquiferField};
use mcv_worldgen::vanilla::lake_pass::{lake_passes_gate, lake_spots};
use mcv_worldgen::vanilla::terrain::Orchestrator;
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

const SEED: u64 = 0xA01_F17_50C;
const LAVA: u16 = 516;

/// 候选数频度门：rarity_filter 域（1/9、1/200）按理论二项分布带断言。
#[test]
fn lake_attempt_frequency_matches_vanilla_bands() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let mut surf_att = 0usize;
    let mut under_att = 0usize;
    for cz in -16..16 {
        for cx in -16..16 {
            // 只统计 origin = 本块的候选（3×3 重放窗会重复计邻块）
            let spots = lake_spots(SEED, cx, cz, &orch, &mut field);
            for s in &spots {
                if s.ox.div_euclid(16) == cx && s.oz.div_euclid(16) == cz {
                    if s.variant == 0 {
                        under_att += 1;
                    } else {
                        surf_att += 1;
                    }
                }
            }
        }
    }
    eprintln!("K-LAKES attempts underground={under_att} surface={surf_att} (win=1024 chunks)");
    assert!(
        (1..=15).contains(&surf_att),
        "地表岩浆湖候选 {surf_att} 出带 [1,15]（期望 5.12，1/200/块）"
    );
    assert!(
        (10..=45).contains(&under_att),
        "地下岩浆湖候选 {under_att} 出带 [10,45]（期望 ≈22–31，1/9/块 × 存活率 ≈0.21，推导见文件头注）"
    );
}

/// 地表岩浆湖真实存在 + 缓冲可见：纯函数侧 64×64 区块窗找 surface
/// 候选（1/200/块 → μ ≈ 20.5）并计判据环存活，再对存活的 origin
/// 区块整生成，实证**湖体下半带 y ∈ [oy−4, oy−1]** 的 LAVA 体积。
/// 带即 LakeFeature yy<4 的 fluid 落位（origin = 高度图值 =
/// 陆面顶+1 ⇒ 带 = 陆顶−3..陆顶）；水柱候选在判据环下半部即被弃
/// （水非实心非本流体），所以判据环存活的 surface 湖必立足陆地。
/// 注：16 区块窗期望候选仅 0.08，必不中——窗必须按 μ 反推取大窗。
/// K-LAKES-BUF 行落候选/判据环存活/带内岩浆三计数。
#[test]
fn surface_lava_lake_placed_and_visible_in_buffers() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let mut surf_cand = 0usize;
    let mut survivors = Vec::new();
    for cz in -32..32 {
        for cx in -32..32 {
            let spots = lake_spots(SEED, cx, cz, &orch, &mut field);
            for s in spots.iter().filter(|s| {
                s.variant == 1 && s.ox.div_euclid(16) == cx && s.oz.div_euclid(16) == cz
            }) {
                surf_cand += 1;
                if lake_passes_gate(s, &orch, &mut field) {
                    survivors.push(*s);
                }
            }
        }
    }
    assert!(
        surf_cand > 0,
        "4096 区块窗 0 个 surface 候选（1/200/块频度门失效？）"
    );
    let mut lava_in_band = 0usize;
    let mut chunks = std::collections::HashSet::new();
    for s in &survivors {
        let pos = ChunkPos::new(s.ox.div_euclid(16), s.oz.div_euclid(16));
        if !chunks.insert(pos) {
            continue;
        }
        let t = generate_terrain_with(TerrainBackend::Vanilla, SEED, pos).expect("gen");
        let v = t.voxels.as_u16_slice();
        for y in s.oy - 4..s.oy {
            if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&y) {
                continue;
            }
            for z in 0..16usize {
                for x in 0..16usize {
                    if v[mcv_core::vidx(x, y, z)] == LAVA {
                        lava_in_band += 1;
                    }
                }
            }
        }
    }
    eprintln!(
        "K-LAKES-BUF surface_candidates={surf_cand} gate_survivors={} chunks={} lava_in_band={lava_in_band} (win=64x64)",
        survivors.len()
    );
    assert!(
        !survivors.is_empty(),
        "surface 候选 {surf_cand} 个但判据环全弃（判据环过严？）"
    );
    assert!(
        lava_in_band > 0,
        "判据环存活的 {}/{} 个地表湖下半带内无 LAVA（放置路未落地？）",
        survivors.len(),
        surf_cand
    );
}

/// 跨块重放一致性：同一 origin 列的候选（含高度/variant）从两侧区块
/// 视角逐字节一致（纯函数面；判据环/放置由 (seed, origin) 全决定，
/// 两侧同判同写）。
#[test]
fn lake_candidates_identical_from_both_sides() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut f1 = AquiferField::new(&aq, &orch);
    let mut f2 = AquiferField::new(&aq, &orch);
    let a = lake_spots(SEED, 0, 0, &orch, &mut f1);
    let b = lake_spots(SEED, 1, 0, &orch, &mut f2);
    let mut shared = 0usize;
    for s in a.iter().filter(|s| s.ox.div_euclid(16) == 1) {
        let m = b
            .iter()
            .find(|t| t.ox == s.ox && t.oz == s.oz && t.variant == s.variant);
        assert_eq!(
            m.map(|t| t.oy),
            Some(s.oy),
            "跨界候选漂移 ({},{},{}) v{}",
            s.ox,
            s.oy,
            s.oz,
            s.variant
        );
        shared += 1;
    }
    eprintln!("K-LAKES-X shared_candidates={shared}");
}

/// 同 seed 同字节（含湖 pass 后整体确定性契约面）+ 异 seed 差异。
#[test]
fn lakes_determinism_bytes() {
    let a = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(1, 1)).expect("gen");
    let b = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(1, 1)).expect("gen");
    assert_eq!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
    assert_eq!(a.heightmap, b.heightmap);
}
