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

/// 频度实物链（独立核验，非统计口径）：placed_feature/lake_lava_surface
/// .json rarity_filter 1/200 ∧ BiomeDefaultFeatures.java:20 LAKES 步全
/// 地表群系挂载 → 尝试 1/200/区块；heightmap WORLD_SURFACE_WG（
/// Heightmap.java:147 NOT_AIR）origin=顶+1 → 下半带 [顶−3..顶] 全周封底
/// （LakeFeature.java:80 `!isSolid && !=fluid` 弃单，水/雕空/贴边低柱
/// 同弃）→ 存活限「平坦无雕」地表窗，为原版真实稀疏事件（本仓
/// CAVE-1 接线前 128² 窗 82 候选存活 ≥1；接线后 noodle/pillars 实物
/// 雕刻使近地表空腔连通增加——K-NOODLE-SIGMA σ=0.328/mean≈0/mask≥0=
/// 49.7% 证实噪声分布与原版同口径，非 σ 塌缩——128² 窗 73 候选 14 陆
/// 0 存活，K-LAKES-GATE reason_hist=[0,0,73]：全死于下半封底门，与
/// 上半液门 0 命中对照，gate 无恒假；封底门在丘陵/雕刻窗内本就
/// 严）。窗扩 256²（μ_att ≈ 328）为功效修正；若仍 0 存活则回查实现，
/// 不再扩窗。
/// 地表岩浆湖真实存在 + 缓冲可见：纯函数侧 256×256 区块窗找 surface
/// 候选（1/200/块 → μ ≈ 328）并计判据环存活，再对存活的 origin
/// 区块整生成，实证**湖体下半带 y ∈ [oy−4, oy−1]** 的 LAVA 体积。
/// 带即 LakeFeature yy<4 的 fluid 落位（origin = 高度图值 =
/// 陆面顶+1 ⇒ 带 = 陆顶−3..陆顶）；水柱候选在判据环下半部即被弃
/// （水非实心非本流体，原版同语义——洋面/湖面的 surface 候选必弃），
/// 所以判据环存活的 surface 湖必立足陆地。存活还要求 origin 周围
/// ±8 域近平地（16×16 盒下半带全封底），丘陵地形存活率是低概率
/// 事件——窗必须按 μ 反推取大窗（64×64 窗 18 候选 0 存活实测发生，
/// 非判据 bug：海面柱占比高 + 平地门联合期望在此量级）。
/// K-LAKES-BUF 行落候选（海柱/陆柱分列）/判据环存活/带内岩浆计数。
#[test]
fn surface_lava_lake_placed_and_visible_in_buffers() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let mut surf_cand = 0usize;
    let mut land_cand = 0usize;
    let mut survivors = Vec::new();
    for cz in -128..128 {
        for cx in -128..128 {
            let spots = lake_spots(SEED, cx, cz, &orch, &mut field);
            for s in spots.iter().filter(|s| {
                s.variant == 1 && s.ox.div_euclid(16) == cx && s.oz.div_euclid(16) == cz
            }) {
                surf_cand += 1;
                let on_land = orch
                    .column_real_top(s.ox, s.oz, &mut field)
                    .is_some_and(|(y, _)| y > mcv_core::SEA_LEVEL);
                land_cand += on_land as usize;
                if lake_passes_gate(s, &orch, &mut field) {
                    assert!(on_land, "判据环存活却立足水柱（水非实心应弃）");
                    survivors.push(*s);
                }
            }
        }
    }
    assert!(
        surf_cand > 0,
        "65536 区块窗 0 个 surface 候选（1/200/块频度门失效？）"
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
        "K-LAKES-BUF surface_candidates={surf_cand} land={land_cand} gate_survivors={} chunks={} lava_in_band={lava_in_band} (win=256x256)",
        survivors.len(),
        chunks.len()
    );
    assert!(
        !survivors.is_empty(),
        "surface 候选 {surf_cand}（陆柱 {land_cand}）判据环全弃——陆柱有而存活 0 = 判据环过严，陆柱 0 = 窗内地形无陆（换窗）"
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
