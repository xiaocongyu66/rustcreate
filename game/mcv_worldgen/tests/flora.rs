//! 陆地植被 pass 2.2 行为锁（草原植被生成恢复案；实现面与 26.1 锚点全部
//! 见 `src/vanilla/flora.rs` 模块头注——SimpleBlockFeature.java:16-47 /
//! VegetationBlock.java:44-46 / placed patch_grass_*·patch_tall_grass_*·
//! flower_* 三族 JSON / NoiseThresholdCountPlacement.java:34-37 /
//! NoiseThresholdProvider.java:59-66 / TrapezoidInt.java:40-43）。
//!
//! 覆盖：① 固定种子确定性（同 seed 逐条相等、(seed, origin) 纯函数、异
//! seed 异输出、通道域盐隔离）；② 链上界（placed Repeating 面 per-chunk
//! 写帽）；③ 写域 = pass-1 判据下 air 前提 / SUPPORTS_VEGETATION 附着
//! 面 / tall 双半同列成对 / id 白名单（12 id 面）/ Ocean 零行；④
//! generate 末端体素逐格可见。
//!
//! K-FLORA 数值行常态入日志（cave_audit/lakes「数据两用」先例）；期望
//! 收紧值以 CI 实测打印回填（不凭空编）。

use std::collections::HashMap;
use std::sync::OnceLock;

use mcv_core::ChunkPos;
use mcv_worldgen::vanilla::aquifer::{Aquifer, AquiferField};
use mcv_worldgen::vanilla::biomes::Biome;
use mcv_worldgen::vanilla::flora::{
    FloraWrite, LAND_FLORA_DOMAIN, flora_writes, origin_flora_writes,
};
use mcv_worldgen::vanilla::ore_pass::{ORE_DOMAIN, Replay};
use mcv_worldgen::vanilla::terrain::Orchestrator;
use mcv_worldgen::vanilla::water_flora::WATER_FLORA_DOMAIN;
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

/// ores/water_flora 同源种子窗先例。
const SEED: u64 = 0x5EED_0000_0000_0007;

/// 植被 id 面（flora.rs 白名单 12 id——草丛/蕨/高草/花八族）。
const SHORT_GRASS: u16 = 907;
const TALL_GRASS: u16 = 1001;
const FERN: u16 = 404;
const DANDELION: u16 = 13;
const POPPY: u16 = 12;
const AZURE_BLUET: u16 = 46;
const CORNFLOWER: u16 = 245;
const OXEYE_DAISY: u16 = 675;
const ORANGE_TULIP: u16 = 672;
const RED_TULIP: u16 = 872;
const PINK_TULIP: u16 = 732;
const WHITE_TULIP: u16 = 1148;

fn is_land_flora(id: u16) -> bool {
    matches!(
        id,
        SHORT_GRASS
            | TALL_GRASS
            | FERN
            | DANDELION
            | POPPY
            | AZURE_BLUET
            | CORNFLOWER
            | OXEYE_DAISY
            | ORANGE_TULIP
            | RED_TULIP
            | PINK_TULIP
            | WHITE_TULIP
    )
}

/// 找块结果（窗 ±64 区块半径，128/256 逐级兜底；water_flora 先例）。
struct Survey {
    land: Vec<(i32, i32, Biome)>, // (ocx, ocz, 中心群系——主占块)
    ocean: Vec<(i32, i32)>,
}

/// 子网格 16 点（stride 4）：占额 ≥8/16 = 该群系主占（water_flora 先例）。
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

/// 目标群系主占块普查（OnceLock：四测试共享窗扫描，cargo 默认并行）。
fn survey() -> &'static Survey {
    static SURVEY: OnceLock<Survey> = OnceLock::new();
    SURVEY.get_or_init(|| {
        let orch = Orchestrator::new(SEED);
        let mut land = Vec::new();
        let mut ocean = Vec::new();
        let mut per_biome: HashMap<Biome, usize> = HashMap::new();
        const PER_BIOME_QUOTA: usize = 6;
        'window: for &half in &[64_i32, 128, 256] {
            for ocz in -half..half {
                for ocx in -half..half {
                    let c0 = orch.biome_at(ocx * 16 + 8, ocz * 16 + 8);
                    if c0 == Biome::Ocean {
                        if ocean.len() < 8 {
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
                    } else if c0 != Biome::River {
                        let quota = per_biome.get(&c0).copied().unwrap_or(0);
                        if quota < PER_BIOME_QUOTA {
                            let hits = SUBGRID
                                .iter()
                                .filter(|&&(lx, lz)| {
                                    orch.biome_at(ocx * 16 + lx as i32, ocz * 16 + lz as i32) == c0
                                })
                                .count();
                            // 主占 ≥8/16 收（锚点落目标群系列概率 ≈ 群系
                            // 列占比；行链大 count 下 P(全空) 可忽略）。
                            if hits >= 8 {
                                land.push((ocx, ocz, c0));
                                per_biome.insert(c0, quota + 1);
                            }
                        }
                    }
                    let all: usize = per_biome.values().sum();
                    if ocean.len() >= 8 && all >= 8 * PER_BIOME_QUOTA {
                        break 'window;
                    }
                }
            }
        }
        Survey { land, ocean }
    })
}

/// Id 直方图（K- 行数据面）。
#[derive(Default)]
struct Hist {
    n: usize,
    sg: usize,
    tg: usize,
    fern: usize,
    dandelion: usize,
    poppy: usize,
    azure: usize,
    cornflower: usize,
    oxeye: usize,
    tulip_orange: usize,
    tulip_red: usize,
    tulip_pink: usize,
    tulip_white: usize,
}

impl Hist {
    fn add(&mut self, id: u16) {
        self.n += 1;
        match id {
            SHORT_GRASS => self.sg += 1,
            TALL_GRASS => self.tg += 1,
            FERN => self.fern += 1,
            DANDELION => self.dandelion += 1,
            POPPY => self.poppy += 1,
            AZURE_BLUET => self.azure += 1,
            CORNFLOWER => self.cornflower += 1,
            OXEYE_DAISY => self.oxeye += 1,
            ORANGE_TULIP => self.tulip_orange += 1,
            RED_TULIP => self.tulip_red += 1,
            PINK_TULIP => self.tulip_pink += 1,
            WHITE_TULIP => self.tulip_white += 1,
            _ => {}
        }
    }
}

/// K-FLORA per-biome 群系行面（chunks/with_writes/id 直方图）。
fn k_print(tag: &str, sv: &Survey, orch: &Orchestrator, field: &mut AquiferField<'_>) {
    let mut by: HashMap<Biome, (usize, usize, Hist)> = HashMap::new();
    for &(ocx, ocz, b) in &sv.land {
        let ws = flora_writes(SEED, ocx, ocz, orch, field);
        let e = by.entry(b).or_default();
        e.0 += 1;
        if !ws.is_empty() {
            e.1 += 1;
        }
        for w in &ws {
            e.2.add(w.id);
        }
    }
    let mut keys: Vec<Biome> = by.keys().copied().collect();
    keys.sort_by_key(|b| format!("{b:?}"));
    for b in keys {
        let (n, ww, h) = &by[&b];
        eprintln!(
            "K-FLORA {tag} biome={b:?} chunks={n} with_writes={ww} total={} ids=[sg={} tg={} \
             fern={} dandelion={} poppy={} azure={} cornflower={} oxeye={} tulip={}/{}/{}/{}]",
            h.n,
            h.sg,
            h.tg,
            h.fern,
            h.dandelion,
            h.poppy,
            h.azure,
            h.cornflower,
            h.oxeye,
            h.tulip_orange,
            h.tulip_red,
            h.tulip_pink,
            h.tulip_white
        );
    }
}

/// 链上界 per-biome（placed Repeating 面推导——
/// Pre·copies × count 之和；rarity/air 门只减不增）：
/// - Plains: flower_plains 15·64 + grass 10·32 + tall 7·96 = 1952；
/// - Forest/Birch: flower 1 + grass 2·32 = 65；
/// - Taiga: flower 1 + grass 32 = 33；
/// - Snowy/Desert/Beach/River: 1 + 32 = 33；
/// - Savanna: flower 1 + tall 96 + grass 20·32 = 737；Ocean = 0。
fn cap(b: Biome) -> usize {
    match b {
        Biome::Plains => 15 * 64 + 10 * 32 + 7 * 96,
        Biome::Forest | Biome::BirchForest => 65,
        Biome::Taiga => 33,
        Biome::SnowyPlains | Biome::Desert | Biome::Beach | Biome::River => 33,
        Biome::Savanna => 1 + 96 + 20 * 32,
        Biome::Ocean => 0,
    }
}

/// ① 固定种子：同 seed 同输出（(seed, origin) 纯函数、两次调用逐条
/// 相等）、异 seed 异输出、通道域盐隔离；K- 行随跑打印。
#[test]
fn fixed_seed_determinism_and_channels() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let sv = survey();
    assert!(!sv.land.is_empty(), "窗内陆地主占块找样失败");
    assert!(!sv.ocean.is_empty(), "窗内海块找样失败");
    let mut checked = 0usize;
    let mut diff_seeds = 0usize;
    for &(ocx, ocz, _) in sv.land.iter().take(12) {
        let a = flora_writes(SEED, ocx, ocz, &orch, &mut field);
        let b = flora_writes(SEED, ocx, ocz, &orch, &mut field);
        assert_eq!(a, b, "同 seed 同 origin 输出漂移 ({ocx},{ocz})");
        let mut rp = Replay::new(SEED, &orch, &mut field);
        let w1 = origin_flora_writes(SEED, ocx, ocz, &mut rp);
        assert_eq!(a, w1, "纯函数入口漂移 ({ocx},{ocz})");
        checked += 1;
        let c = flora_writes(SEED ^ 0x9E37_79B9, ocx, ocz, &orch, &mut field);
        if c != a {
            diff_seeds += 1;
        }
    }
    assert!(checked > 0);
    assert!(diff_seeds > 0, "异 seed 全部同输出——种子未入判定面");
    // 通道域盐隔离哨兵（ore/lake/tree/water_flora 私域不重号）。
    assert_ne!(LAND_FLORA_DOMAIN, WATER_FLORA_DOMAIN);
    assert_ne!(LAND_FLORA_DOMAIN, ORE_DOMAIN);
    assert_ne!(LAND_FLORA_DOMAIN, 0x0000_1A3E_005E);
    assert_ne!(LAND_FLORA_DOMAIN, 0x7BEE_5501);
    k_print("determinism", sv, &orch, &mut field);
}

/// ② 链上界（placed Repeating 面推导——② 的 per-chunk 写帽；K- 实测
/// 对位面）。
#[test]
fn per_chunk_write_ceiling_by_chain_repeats() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let sv = survey();
    let mut totals: HashMap<Biome, usize> = HashMap::new();
    for &(ocx, ocz, b) in &sv.land {
        let ws = flora_writes(SEED, ocx, ocz, &orch, &mut field);
        assert!(
            ws.len() <= cap(b),
            "({ocx},{ocz}) {b:?} 写 {} 帽 {} 破（链 repeat 面漂移）",
            ws.len(),
            cap(b)
        );
        *totals.entry(b).or_insert(0) += ws.len();
    }
    for (b, t) in &totals {
        eprintln!(
            "K-FLORA ceiling biome={b:?} total_writes={t} cap*chunks={}",
            cap(*b)
        );
    }
}

/// ③ 写域面：pass-1 判据下每写位支撑面（VegetationBlock.canSurvive——
/// 下方 SUPPORTS_VEGETATION）；tall 双半同列成对（SimpleBlockFeature
/// :27-32 placeAt 面）；id 白名单；Ocean 行位不可达（无写）。
#[test]
fn writes_domain_support_and_pairs() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let sv = survey();
    for &(ocx, ocz, b) in &sv.land {
        let mut rp = Replay::new(SEED, &orch, &mut field);
        let ws = origin_flora_writes(SEED, ocx, ocz, &mut rp);
        for w in &ws {
            assert!(
                is_land_flora(w.id),
                "白名单外 id {} 入写表 ({ocx},{ocz})",
                w.id
            );
            let below = rp.block_at(w.x, w.y - 1, w.z);
            // 附入门（VegetationBlock.canSurvive）；tall 上半例外——其
            // 「下方」是同列下半（pass-1 重放态为 air，靠同列写旁证，
            // water_flora tall 断言先例）。
            if !mcv_core::tags::TAG_BLOCK_SUPPORTS_VEGETATION
                .set
                .contains(below)
            {
                if w.id == TALL_GRASS {
                    assert!(
                        ws.iter().any(|v| v.id == TALL_GRASS
                            && v.x == w.x
                            && v.z == w.z
                            && v.y == w.y - 1),
                        "非下半双半写 ({},{},{}) 基面 {below}",
                        w.x,
                        w.y,
                        w.z
                    );
                } else {
                    panic!("附入门外基面 {below} ({},{},{}) {b:?}", w.x, w.y - 1, w.z);
                }
            }
            if w.id == TALL_GRASS {
                let pair = ws.iter().any(|v| {
                    v.id == TALL_GRASS
                        && v.x == w.x
                        && v.z == w.z
                        && (v.y == w.y + 1 || v.y == w.y - 1)
                });
                assert!(
                    pair,
                    "tall 单半落写 ({},{},{})——placeAt 双半面破",
                    w.x, w.y, w.z
                );
            }
        }
    }
}

/// ④ Ocean 群系行位不可达（无陆族写）+ 端到端 generate 落位可见。
#[test]
fn ocean_silent_and_end_to_end_visibility() {
    let orch = Orchestrator::new(SEED);
    let aq = Aquifer::new(SEED);
    let mut field = AquiferField::new(&aq, &orch);
    let sv = survey();
    for &(ocx, ocz) in &sv.ocean {
        let ws = flora_writes(SEED, ocx, ocz, &orch, &mut field);
        assert!(
            ws.is_empty(),
            "Ocean 群系块 ({ocx},{ocz}) 有陆地族写：{} 条",
            ws.len()
        );
    }
    // 端到端：写表最多的主占块做锚（K- 随行打印）。
    let mut anchor: Option<(i32, i32, Vec<FloraWrite>)> = None;
    let mut best = 0usize;
    for &(ocx, ocz, b) in &sv.land {
        if b == Biome::Plains || b == Biome::Forest {
            let ws = flora_writes(SEED, ocx, ocz, &orch, &mut field);
            if ws.len() > best {
                best = ws.len();
                anchor = Some((ocx, ocz, ws));
            }
        }
    }
    let (ox, oz, ws) = anchor.expect("无任何植被写块（草原行全线断链）");
    eprintln!("K-FLORA e2e anchor=({ox},{oz}) writes={best}");
    let out = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(ox, oz))
        .expect("vanilla gen");
    let vox = out.voxels.as_u16_slice();
    let mut seen = 0usize;
    for w in &ws {
        let lx = w.x - ox * 16;
        let lz = w.z - oz * 16;
        if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
            continue; // offset ±7 抖出界的写由邻块重放落其自块
        }
        let got = vox[mcv_core::vidx(lx as usize, w.y, lz as usize)];
        // 后写者赢族（同 pass 行序覆写同格——KNOWN-DIVERGENCE 判据读
        // pass-1 重放态）：buffer 终态 = 植被族占格。冻结/水草不动陆
        // 面位（树荫位容忍缓冲互见，同 water_flora 测试族监面口径）。
        assert!(
            is_land_flora(got),
            "体素落位 ({},{},{}) 写表 {} 但 buffer {got} 非植被族",
            w.x,
            w.y,
            w.z,
            w.id
        );
        seen += 1;
    }
    eprintln!("K-FLORA e2e seen={seen}/{best} anchor=({ox},{oz})");
    assert!(seen > 0, "锚块写全部越界（offset 边角退化？）");
}
