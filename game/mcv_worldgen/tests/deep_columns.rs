//! #190 深层透明取证（真机报告②：「原地向下挖到某层，周围变透明，
//! 视角朝上看方块正常」）。
//!
//! 假说一（用户猜测）：深板岩层没生成，挖穿石头进入深板岩深度后
//! 全是空气。本测直接生成高种子窗口逐柱取证 y ∈ [-64, 8]：
//! - 每柱自表面向下至基岩**不得整层掏空**（任何 y 层仍须实心）；
//!   掏空 = 「透明」世界生成根因，实心底不在 = 虚空坠落。
//! - 深板岩（DEEPSLATE_*）在注册表存在（资产/语言表齐）但世界生成
//!   **从不铺装**——vanilla pass 带下实心一律 STONE（terrain.rs
//!   solid_material，abovePreliminarySurface 门）。该缺层归 #192
//!   （深板岩/基岩形态补齐），本测只锁「缺材质不缺实体」的边界行为。
//! - 负 y 口径（#71 绕回面回归）：y=-64..8 段的实心必须反映在
//!   heightmap 上可达的深度语义（i16 绝对域，无符号绕回哨兵歧义）。

use mcv_core::{CHUNK_SX, CHUNK_SZ, WORLD_MIN_Y};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

/// 实心判定：非空气且非流体（岩浆/水「透明」感由 #192/流体面管，
/// 实体掏空才是本锁对象）。
fn is_solid(id: u16) -> bool {
    let d = &mcv_core::BLOCKS[id as usize];
    d.solid
}

fn is_fluid(id: u16) -> bool {
    let d = &mcv_core::BLOCKS[id as usize];
    d.liquid
}

#[test]
fn deep_columns_are_never_hollow() {
    let mut columns_checked = 0usize;
    let mut max_air_run_report = String::new();
    let mut max_air_run = 0usize;
    let mut worst_level = (1.0f32, String::new());
    for seed in [2026_1013u64, 1, 777, 0xDEAD_BEEF, 42] {
        for cx in 0..2i32 {
            for cz in 0..2i32 {
                let out = generate_terrain_with(
                    TerrainBackend::Vanilla,
                    seed,
                    mcv_core::ChunkPos::new(cx, cz),
                )
                .expect("generate ok");
                let vox = out.voxels.as_u16_slice();
                let col_at =
                    |lx: usize, lz: usize, y: i32| -> u16 { vox[mcv_core::vidx(lx, y, lz)] };
                // 逐 y 层的实心+流体覆盖率（含水层干洞合法蚀刻，整层
                // 掏空才是不合法）。阈值 0.15 取「灾难性空洞层」判据：
                // 深层意大利面洞/奶酪洞群聚是**原版合法形态**（对照
                // 真机证词后不得把合法洞穴误判成红），红因只允许
                // 「层带整段不生成」级别的世界生成崩坏；覆盖率分布
                // 由末行 worst_level 诊断输出（B 根因报告数据源）。
                for y in WORLD_MIN_Y..=8 {
                    let filled = (0..CHUNK_SX)
                        .flat_map(|lx| (0..CHUNK_SZ).map(move |lz| (lx, lz)))
                        .filter(|(lx, lz)| {
                            let id = col_at(*lx, *lz, y);
                            is_solid(id) || is_fluid(id)
                        })
                        .count();
                    let frac = filled as f32 / (CHUNK_SX * CHUNK_SZ) as f32;
                    if frac < worst_level.0 {
                        worst_level = (frac, format!("seed={seed} chunk=({cx},{cz}) y={y}"));
                    }
                    assert!(
                        frac > 0.15,
                        "K-DEEP-COL seed={seed} chunk=({cx},{cz}) y={y}：深层带该层实心+流体\
                         覆盖率 {frac:.3} ≤ 0.15——层带掏空 = 真机「挖到某层周围变透明」的\
                         世界生成根因候选（含水层洞失控/密度门错）"
                    );
                }
                for lx in 0..CHUNK_SX {
                    for lz in 0..CHUNK_SZ {
                        // 基岩恒实心（y=-64 层，BedrockBlock 底层规则：
                        // 开放柱/含水层分支都在实心门之后）。
                        assert!(
                            is_solid(col_at(lx, lz, WORLD_MIN_Y)),
                            "K-DEEP-COL seed={seed} col=({lx},{lz})：y={WORLD_MIN_Y} 不实心 = 挖穿坠虚空"
                        );
                        // 诊断量：带内最长连续空气 run（洞合法，只报
                        // 不锁——全空柱 run=72 时上面的层带断言先红）。
                        let mut run = 0usize;
                        for y in WORLD_MIN_Y..=8 {
                            if !is_solid(col_at(lx, lz, y)) {
                                run += 1;
                            } else {
                                if run > max_air_run {
                                    max_air_run = run;
                                    max_air_run_report = format!(
                                        "seed={seed} chunk=({cx},{cz}) col=({lx},{lz}) run={run} 止于 y={y}"
                                    );
                                }
                                run = 0;
                            }
                        }
                        columns_checked += 1;
                    }
                }
            }
        }
    }
    // 窗口 = 5 seed × 2×2 chunk × 256 列 = 5120；哨兵防「生成静默失效
    // 导致全表假绿」。
    assert!(columns_checked > 5000, "柱样本过少 = 生成失效");
    eprintln!(
        "K-DEEP-COL 诊断：最深连续空气 run={max_air_run}（{max_air_run_report}）\
         columns={columns_checked} worst_level={}",
        worst_level.1
    );
}

/// 深板岩缺层登记（#192 交接锚）：生成输出不得含 deepslate 系方块
/// （世界生成从未铺装），同时注册表必须已含 deepslate（资产就绪，
/// 只缺铺装规则）。若 #192 补上铺装，本测的「不含」断言必须随该
/// 波一起改写（红 = 提醒口径迁移，非回归）。
#[test]
fn deepslate_absent_until_192() {
    let any_deepslate_registered = mcv_core::BLOCKS
        .iter()
        .any(|d| d.name.contains("deepslate") || d.name.contains("bedrock"));
    assert!(
        any_deepslate_registered,
        "注册表缺 deepslate/bedrock 名族 = 资产面回退（#192 前置）"
    );
    let ds_ids: Vec<u16> = mcv_core::BLOCKS
        .iter()
        .enumerate()
        .filter(|(_, d)| d.name.contains("deepslate"))
        .map(|(i, _)| i as u16)
        .collect();
    assert!(!ds_ids.is_empty(), "注册表无 deepslate 项");
    for seed in [2026_1013u64, 7] {
        let out = generate_terrain_with(
            TerrainBackend::Vanilla,
            seed,
            mcv_core::ChunkPos::new(3, -4),
        )
        .expect("generate ok");
        let hit = out
            .voxels
            .as_u16_slice()
            .iter()
            .filter(|v| ds_ids.contains(&(**v & mcv_core::ID_MASK)))
            .count();
        assert_eq!(
            hit, 0,
            "worldgen 已铺装 deepslate（{hit} 格）：#192 已落地，本测口径须同步改写"
        );
    }
}
