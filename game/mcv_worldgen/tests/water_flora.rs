//! 水草植被 pass 2.3 行为锁（ocean-flora2；实现面与 26.1 锚点全部见
//! `src/vanilla/water_flora.rs` 模块头注——SeagrassFeature.java:20-51 /
//! KelpFeature.java:19-54 / placed JSON 三行 / Heightmap.java:149-150
//! MATERIAL_MOTION_BLOCKING 水不计顶）。
//!
//! 覆盖：① 固定种子确定性（同 seed 逐条相等、(seed, origin) 纯函数、
//! 异 seed 异输出、通道域盐隔离）；② OCEAN 行两特征齐全（seagrass 丛、
//! kelp 柱），柱形 = 连续列 + 顶头体中 + total ≤ 11（h 循环 0..=height
//! 的实物柱面；任务单「1..25」系 KelpBlock.AGE 状态域，头注已勘误）+
//! 洋底/河床实面附着；③ 写域 = 只替换 pass-1 水柱格 + 写入帽 y ≤ 62
//! （geometry_locks「63 层非水」对偶锁）+ 陆块（全 256 列无 Ocean/River
//! 行位）零写；④ RIVER 行也有水草且只出海草族 id（river.json:93 无
//! kelp 行），generate 末端体素逐格可见。
//!
//! 找块策略：ORE goldens 同源种子 `0x5EED_0000_0000_0007`（ores.rs 测试
//! 先例）± 邻域窗，Orchestrator 群系查点三级采样——锚群系门只读群系
//! （world 门 = biome_at，PlacedFeature.java:47 链尾 biome 零随机），故
//! 找块只采样群系：中心 1 点止损 → 候选 16 点子网格占额 → 陆地全 256
//! 列复检（锚点均匀落整块，漏检即「陆块零写」的假阳根）。本仓 10 群系
//! 点表无 swamp/深海变体（头注数据行段）。断言带余量推导：每区块单锚
//! 48 次特征内重试、anchor 落目标群系列概率 = 该群系列占比（子网格 ≥8/16
//! 的候选 ⇒ ≥50%，配额 24 ⇒ P(<2 有写) ≈ 0.3% 级；海块有写率下限 2/3，
//! anchor 落 magma 床/干滩柱为离群）。

use std::collections::HashMap;
use std::sync::OnceLock;

use mcv_core::ChunkPos;
use mcv_worldgen::vanilla::aquifer::{Aquifer, AquiferField};
use mcv_worldgen::vanilla::biomes::Biome;
use mcv_worldgen::vanilla::ore_pass::{ORE_DOMAIN, Replay};
use mcv_worldgen::vanilla::terrain::Orchestrator;
use mcv_worldgen::vanilla::water_flora::{
    FloraWrite, WATER_FLORA_DOMAIN, flora_writes, origin_flora_writes,
};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

const SEED: u64 = 0x5EED_0000_0000_0007;
const WATER: u16 = 5;
const KELP: u16 = 508;
const KELP_PLANT: u16 = 509;
const SEAGRASS: u16 = 905;
const TALL_SEAGRASS: u16 = 1002;

/// 水草四 id。
fn is_flora(id: u16) -> bool {
    id == KELP || id == KELP_PLANT || id == SEAGRASS || id == TALL_SEAGRASS
}

/// 找块结果（窗 `ORE_DOMAIN` 同源种子 ±64 区块半径，128/256 逐级兜底）。
struct Survey {
    ocean: Vec<(i32, i32)>,
    river: Vec<(i32, i32)>,
    /// 全 256 列无 Ocean/River 的纯陆块（「行位全不可达」锚）。
    land: Option<(i32, i32)>,
}

/// 子网格 16 点（stride 4）：占额 ≥8/16 = 该群系主占。锚群系门只读群系，
/// 群系查点是廉价气候采样（Orchestrator::biome_at，无列构建）。
const SUBGRID: [(usize, usize); 16] = [
    (2, 2),
    (2, 6),
    (2, 10),
    (2, 14),
    (6, 2),
    (6, 6),
    (6, 10),
    (6, 14),
    (10, 2),
    (10, 6),
    (10, 10),
    (10, 14),
    (14, 2),
    (14, 6),
    (14, 10),
    (14, 14),
];

/// 全窗普查（OnceLock：四测试共享一份窗扫描结果，cargo 默认并行线程）。
fn survey() -> &'static Survey {
    static SURVEY: OnceLock<Survey> = OnceLock::new();
    SURVEY.get_or_init(|| {
        let orch = Orchestrator::new(SEED);
        let mut ocean = Vec::new();
        let mut river = Vec::new();
        let mut land = None;
        'window: for &half in &[64_i32, 128, 256] {
            for ocz in -half..half {
                for ocx in -half..half {
                    // 中心止损：陆心即陆地候选（Land 需全 256 列复检）、
                    // 交由子网格判群系主占。
                    let c0 = orch.biome_at(ocx * 16 + 8, ocz * 16 + 8);
                    if c0 == Biome::Ocean && ocean.len() < 24 {
                        let hits = SUBGRID
                            .iter()
                            .filter(|&&(lx, lz)| {
                                orch.biome_at(ocx * 16 + lx as i32, ocz * 16 + lz as i32)
                                    == Biome::Ocean
                            })
                            .count();
                        if hits >= 8 {
                            ocean.push((ocx, ocz));
                        }
                    }
                    if c0 == Biome::River && river.len() < 24 {
                        let hits = SUBGRID
                            .iter()
                            .filter(|&&(lx, lz)| {
                                orch.biome_at(ocx * 16 + lx as i32, ocz * 16 + lz as i32)
                                    == Biome::River
                            })
                            .count();
                        // 河道有宽窄：≥4/16（≥25% 覆盖）即收——锚中率仍
                        // ≈ 占比，24 块配额下 P(<2 有写) 可忽略。
                        if hits >= 4 {
                            river.push((ocx, ocz));
                        }
                    }
                    if land.is_none() && c0 != Biome::Ocean && c0 != Biome::River {
                        // 陆地候选：全 256 列无 Ocean/River（锚群系均匀落
                        // 整块；漏检即「陆块零写」假阳根）。
                        let mut all_dry = true;
                        'cols: for lz in 0..16usize {
                            for lx in 0..16usize {
                                let b = orch.biome_at(ocx * 16 + lx as i32, ocz * 16 + lz as i32);
                                if b == Biome::Ocean || b == Biome::River {
                                    all_dry = false;
                                    break 'cols;
                                }
                            }
                        }
                        if all_dry {
                            land = Some((ocx, ocz));
                        }
                    }
                    if ocean.len() >= 24 && river.len() >= 24 && land.is_some() {
                        break 'window;
                    }
                }
            }
        }
        Survey { ocean, river, land }
    })
}

/// ① OCEAN 行两特征齐全 + kelp 柱形（连续/顶头体中/total ≤ 11/洋底
/// 附着）与单锚 48 次重试上限的 count 面。
#[test]
fn ocean_rows_seagrass_and_kelp_present_with_column_shape() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let sv = survey();
    assert!(
        sv.ocean.len() >= 6,
        "窗内海块不足（{}/6）——找块窗或海陆比退化",
        sv.ocean.len()
    );
    let mut with_writes = 0usize;
    let mut with_kelp = 0usize;
    let mut kelp_cols = 0usize;
    let mut kelp_col_seen_multi = false;
    for (ocx, ocz) in &sv.ocean {
        let mut rp = Replay::new(SEED, &orch, &mut field);
        let ws = origin_flora_writes(SEED, *ocx, *ocz, &mut rp);
        if ws.is_empty() {
            continue;
        }
        with_writes += 1;
        // 单锚 48 次重试 × short/tall 双格分支 → 单 origin 海草族 ≤ 96。
        let seagrass_family = ws
            .iter()
            .filter(|w| w.id == SEAGRASS || w.id == TALL_SEAGRASS)
            .count();
        assert!(
            seagrass_family <= 96,
            "海草族写 {seagrass_family} > 96（48 次上限破）"
        );
        // 按 (x,z) 分组：kelp 柱形 + 海草族 id 面。
        let mut cols: HashMap<(i32, i32), Vec<&FloraWrite>> = HashMap::new();
        for w in &ws {
            cols.entry((w.x, w.z)).or_default().push(w);
        }
        let mut kelp_here = false;
        for ((x, z), cells) in &cols {
            let mut kelp_ys: Vec<i32> = cells
                .iter()
                .filter(|w| w.id == KELP || w.id == KELP_PLANT)
                .map(|w| w.y)
                .collect();
            if kelp_ys.is_empty() {
                // 海草族格：id 面锁定（kelp 列外不允许其它 id——本 pass
                // 只产四 id）。
                for w in cells.iter() {
                    assert!(
                        w.id == SEAGRASS || w.id == TALL_SEAGRASS,
                        "kelp 列位外的非水草 id {} 入写表",
                        w.id
                    );
                }
                continue;
            }
            kelp_here = true;
            kelp_cols += 1;
            kelp_ys.sort_unstable();
            kelp_ys.dedup();
            // 连续列（体逐节 + 头）。
            for pair in kelp_ys.windows(2) {
                assert_eq!(pair[0] + 1, pair[1], "kelp 柱断 ({x},{z}) @ {pair:?}");
            }
            // total = 头 + 体 ∈ [1, 11]（h 循环 0..=height，height ≤ 10）。
            assert!(
                (1..=11).contains(&kelp_ys.len()),
                "kelp 柱高 {} 越带 [1,11] ({x},{z})",
                kelp_ys.len()
            );
            if kelp_ys.len() >= 2 {
                kelp_col_seen_multi = true;
            }
            // 顶头：柱顶 buffer 终态 = KELP 头（「后写者赢」：kelp 行
            // (idx 1) 覆写 seagrass 行 (idx 0) 同格；同柱同锚二次 attempt
            // 的 truncate 头可与体混叠——pass-1 重放判据件，头注
            // KNOWN-DIVERGENCE 第 2 条），故只锁顶格 id、不锁中段逐格。
            let top = kelp_ys[kelp_ys.len() - 1];
            let got_top = cells
                .iter()
                .rev()
                .find(|w| w.y == top)
                .map(|w| w.id)
                .unwrap_or(0);
            assert_eq!(
                got_top, KELP,
                "kelp 柱顶 ({x},{top},{z}) 期望头 508 got {got_top}"
            );
            // 洋底附着：柱底 pass-1 下方为实面（同 rp：判据读重放态）。
            let bottom = kelp_ys[0];
            let below = rp.block_at(*x, bottom - 1, *z);
            assert!(
                mcv_core::BLOCKS[usize::from(below)].solid,
                "kelp 柱底附着非实面 {below} ({x},{},{z})",
                bottom - 1
            );
        }
        if kelp_here {
            with_kelp += 1;
        }
    }
    // 有写率 ≥2/3：配额里的海块以深洋主占块为主（子网格 8/16 下限包含
    // 少量 coastal 8-15/16 档，其锚中率仍在 ~50%+，48 次全灭概率可忽略；
    // 余韵失败源仅 magma/干滩柱，离群级）。
    assert!(
        with_writes * 3 >= sv.ocean.len() * 2,
        "海块有写比 {with_writes}/{} < 2/3——水位门/锚面/高度图口径 bug",
        sv.ocean.len()
    );
    // kelp = noise_based_count（n+0)·120 ≈ 半数区块为正的噪声计数）。
    assert!(with_kelp > 0, "窗内无任何 kelp 写——noise_based_count 断链");
    assert!(kelp_cols > 0, "无 kelp 柱");
    assert!(
        kelp_col_seen_multi,
        "窗内无柱高 ≥2 的 kelp 柱——h 循环体断（海带变单格）"
    );
}

/// ② 写域：所有写只替换 pass-1 水柱格、y ≤ 62（geometry_locks「63 层
/// 非水」对偶锁）、seagrass short 附着实面 / tall 双半成对；陆块零写。
#[test]
fn writes_constrained_to_water_columns_and_land_silent() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let sv = survey();
    assert!(sv.land.is_some(), "窗内陆地找样失败");
    for (ocx, ocz) in sv.ocean.iter().take(8).chain(sv.river.iter().take(8)) {
        let mut rp = Replay::new(SEED, &orch, &mut field);
        let ws = origin_flora_writes(SEED, *ocx, *ocz, &mut rp);
        for w in &ws {
            assert!(
                (mcv_core::WORLD_MIN_Y..mcv_core::SEA_LEVEL).contains(&w.y),
                "写 y={} 越域（≤62 写入帽破）",
                w.y
            );
            assert_eq!(
                rp.block_at(w.x, w.y, w.z),
                WATER,
                "非水柱格落写 ({},{},{}) pass-1",
                w.x,
                w.y,
                w.z
            );
            let below = rp.block_at(w.x, w.y - 1, w.z);
            if w.id == SEAGRASS {
                // short：canSurvive = 下方实面（SeagrassBlock.java:46-47）。
                assert!(
                    mcv_core::BLOCKS[usize::from(below)].solid,
                    "seagrass 附着非实面 {below} ({},{},{})",
                    w.x,
                    w.y - 1,
                    w.z
                );
            } else if w.id == TALL_SEAGRASS {
                // tall 双半（HALF 状态面本仓不存，双写同 id 1002）：下半
                // 着实面、上半着下半同位写（pass-1 重放判据：上半的下方
                // 在重放态是水，靠同柱写目旁证）。
                let lower = ws
                    .iter()
                    .any(|v| v.x == w.x && v.z == w.z && v.y == w.y - 1 && is_flora(v.id));
                assert!(
                    mcv_core::BLOCKS[usize::from(below)].solid || lower,
                    "tall 半格悬空 {below} ({},{},{})",
                    w.x,
                    w.y - 1,
                    w.z
                );
            }
        }
    }
    // 陆块零写（全 256 列无 Ocean/River → 两行 placed 均不可达）。
    let (lx, lz) = sv.land.expect("陆地");
    let mut rp = Replay::new(SEED, &orch, &mut field);
    let ws = origin_flora_writes(SEED, lx, lz, &mut rp);
    assert!(ws.is_empty(), "陆块 ({lx},{lz}) 竟有 {} 条写", ws.len());
}

/// ③ RIVER 行：也有水草、只出海草族（river.json:93 无 kelp 行）；锚窗
/// 配额 ≥4（扫配 24，子网格 ≥4/16 收宽窄河道）⇒ P(<2 有写) 可忽略；
/// generate
/// 末端体素逐格可见。
#[test]
fn river_rows_place_seagrass_only_end_to_end() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let sv = survey();
    assert!(sv.river.len() >= 4, "窗内河块不足（{}/4）", sv.river.len());
    let mut with_writes = 0usize;
    let mut anchor = None;
    for (ocx, ocz) in &sv.river {
        let ws = flora_writes(SEED, *ocx, *ocz, &orch, &mut field);
        if ws.is_empty() {
            continue;
        }
        with_writes += 1;
        if anchor.is_none() {
            // 只锚一份（clone），ws 本身留给本轮 id/写帽断言。
            anchor = Some((*ocx, *ocz, ws.clone()));
        }
        for w in &ws {
            assert!(
                w.id == SEAGRASS || w.id == TALL_SEAGRASS,
                "river 行出水草族外 id {} ({},{},{})",
                w.id,
                w.x,
                w.y,
                w.z
            );
            assert!(w.y < mcv_core::SEA_LEVEL, "river 写越海面行 y={}", w.y);
        }
    }
    // 有写率不设分数带：锚群系 = 区块 river 列占比（窄河道子网格 8/16
    // ⇒ 锚中率 ~30–50%，24 块配额下 P(<2 有写) ≈ 0.3% 级），只锁 ≥2。
    assert!(
        with_writes >= 2,
        "river 有写块 {with_writes} < 2——river.json:93 行断链"
    );
    // 端到端：generate 全链（湖/矿→树→水草→freeze→高度图）落位可见。
    let (ox, oz, ws) = anchor.expect("河块有写统计非零");
    let out = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(ox, oz))
        .expect("vanilla gen");
    let vox = out.voxels.as_u16_slice();
    let mut seen = 0usize;
    for w in &ws {
        let lx = w.x - ox * 16;
        let lz = w.z - oz * 16;
        if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
            continue; // 特征内 ±7 抖出界的写由邻块重放落其自块
        }
        let got = vox[mcv_core::vidx(lx as usize, w.y, lz as usize)];
        assert_eq!(
            got, w.id,
            "体素落位 ({},{},{}) 期望 {} got {got}",
            w.x, w.y, w.z, w.id
        );
        seen += 1;
    }
    assert!(seen > 0, "河块写全部越界（锚点 jitter 边角退化？）");
}

/// ④ 固定种子：同 seed 同输出（(seed, origin) 纯函数、两次调用逐条
/// 相等）、异 seed 异输出、通道域盐隔离（树域 0x7BEE_5501/湖域
/// 0x0000_1A3E_005E 私有常量，ores.rs::cross_chunk_vein_stitching 硬编码
/// 哨兵先例）。
#[test]
fn fixed_seed_determinism_and_channels() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let sv = survey();
    let chunks: Vec<(i32, i32)> = sv
        .ocean
        .iter()
        .take(4)
        .chain(sv.river.iter().take(4))
        .copied()
        .collect();
    let mut checked = 0usize;
    let mut diff_seeds = 0usize;
    for (ocx, ocz) in chunks {
        let a = flora_writes(SEED, ocx, ocz, &orch, &mut field);
        let b = flora_writes(SEED, ocx, ocz, &orch, &mut field);
        assert_eq!(a, b, "同 seed 同 origin 输出漂移 ({ocx},{ocz})");
        // 纯函数：独立 Replay 同值（便捷入口 = origin_flora_writes 同面）。
        let mut rp = Replay::new(SEED, &orch, &mut field);
        let w1 = origin_flora_writes(SEED, ocx, ocz, &mut rp);
        assert_eq!(a, w1, "纯函数入口漂移 ({ocx},{ocz})");
        if a.is_empty() {
            continue;
        }
        checked += 1;
        let c = flora_writes(SEED ^ 0x9E37_79B9, ocx, ocz, &orch, &mut field);
        if c != a {
            diff_seeds += 1;
        }
    }
    assert!(checked > 0, "无任何非空写块可比");
    assert!(diff_seeds > 0, "异 seed 全部同输出——种子未入判定面");
    // 通道域盐隔离哨兵。
    assert_ne!(WATER_FLORA_DOMAIN, ORE_DOMAIN);
    assert_ne!(WATER_FLORA_DOMAIN, 0x0000_1A3E_005E);
    assert_ne!(WATER_FLORA_DOMAIN, 0x7BEE_5501);
}
