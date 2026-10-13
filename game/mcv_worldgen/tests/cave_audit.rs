//! #192 矿洞地形量化对照审计探针（K-CAVE-AUDIT 行入 CI 日志，红因/数据
//! 两用——lakes.rs K-LAKES-BUF 先例）。
//!
//! 对照基准（26.1 实物，/root/mc-ref/src-26.1）：
//! - 密度雕刻：`noise_settings/overworld.json` noise_router.final_density
//!   + `density_function/overworld/{sloped_cheese,caves/*}.json`
//!     （entrances/spaghetti_2d/spaghetti_roughness/underground 已移植，
//!     terrain.rs 逐常数引注；pillars/noodle 未移植=缺口 CAVE-1）。
//! - 管状 carver：`configured_carver/{cave,cave_extra_underground,canyon}
//!   .json`（probability 0.15/0.07/0.01，y 窗 -56..180 / -56..47 / 10..67）
//!   ——本仓整体未移植=缺口 CAVE-2（KNOWN-DIVERGENCE）。
//!
//! 本探针量化「本仓现状」的空洞率随深度分布 + 最大连通体，供跨任务对照；
//! 断言只放宽松包络（防空洞塌方/全实心两类系统病），数值本身入日志。

use mcv_core::{ChunkPos, WORLD_MIN_Y};
use mcv_worldgen::vanilla::terrain::BEDROCK;
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

const SEED: u64 = 0x7E_A20_0B1;

const AIR: u16 = 0;
const WATER: u16 = 5;
const LAVA: u16 = 516;

/// 审计窗：4×4 区块（64×64 列），y ∈ [−64, 96)。
const N: i32 = 4;
const Y_LO: i32 = WORLD_MIN_Y;
const Y_HI: i32 = 96;
const XW: usize = (N * 16) as usize;
const ZW: usize = (N * 16) as usize;
const YH: usize = (Y_HI - Y_LO) as usize;

fn gen_window() -> Vec<u16> {
    let mut win = vec![AIR; XW * YH * ZW];
    let base_x = 2 * 16;
    let base_z = -4 * 16;
    for cz in 0..N {
        for cx in 0..N {
            let t = generate_terrain_with(
                TerrainBackend::Vanilla,
                SEED,
                ChunkPos::new(base_x / 16 + cx, base_z / 16 + cz),
            )
            .expect("vanilla gen");
            let vox = t.voxels.as_u16_slice();
            for lz in 0..16usize {
                for lx in 0..16usize {
                    let wx = (cx as usize) * 16 + lx;
                    let wz = (cz as usize) * 16 + lz;
                    for y in Y_LO..Y_HI {
                        let gy = (y - Y_LO) as usize;
                        win[(wz * XW + wx) * YH + gy] = vox[mcv_core::vidx(lx, y, lz)];
                    }
                }
            }
        }
    }
    win
}

#[inline]
fn wid(x: usize, y: i32, z: usize) -> usize {
    (z * XW + x) * YH + (y - Y_LO) as usize
}

/// 空洞率随深度分布（16 格层带）+ 最大连通空洞体（26 邻接、
/// y ∈ [−56, 64) 洞域窗）——数值入 CI 日志，断言只锁包络。
#[test]
fn cave_audit_stats() {
    let win = gen_window();
    // 1) 层带统计。
    let mut lo = Y_LO;
    let mut worst_deep_void = 0.0_f64;
    while lo < Y_HI {
        let hi = (lo + 16).min(Y_HI);
        let (mut air, mut fluid, mut solid, mut bedrock) = (0usize, 0usize, 0usize, 0usize);
        for z in 0..ZW {
            for x in 0..XW {
                for y in lo..hi {
                    match win[wid(x, y, z)] {
                        AIR => air += 1,
                        WATER | LAVA => fluid += 1,
                        BEDROCK => bedrock += 1,
                        _ => solid += 1,
                    }
                }
            }
        }
        let total = air + fluid + solid + bedrock;
        let void = air as f64 / total as f64;
        // K-CAVE-AUDIT band=[lo,hi) air=x fluid=x solid=x bedrock=x void=..
        println!(
            "K-CAVE-AUDIT seed={SEED:#x} band=[{lo},{hi}) air={air} fluid={fluid} \
             solid={solid} bedrock={bedrock} void={:.4}",
            void
        );
        // 包络只罩洞网主力深度域 −48..48：[48,64) 贴近 preliminary
        // surface（≈63+），原版同段也以实心为主（entrances 带 −10..30、
        // cheese 近表面被 sloped 抬升压制），允许空带；仅 −48..32 参与
        // 「洞网存在性」判定。
        if lo >= -48 && hi <= 48 {
            assert!(void < 0.5, "带 [{lo},{hi}) 空洞率 {void:.3} ≥0.5——全空腔病");
        }
        if lo >= -48 && hi <= 32 {
            assert!(
                void > 0.005,
                "带 [{lo},{hi}) 空洞率 {void:.3} ≤0.005——全实心塌方病"
            );
            worst_deep_void = worst_deep_void.max(void);
        }
        lo += 16;
    }
    assert!(
        worst_deep_void > 0.01,
        "−48..32 深度域空洞率上限 {worst_deep_void:.3} 过低——洞网消失"
    );
    // 2) 最大连通体（BFS，洞域 y ∈ [−56, 64)）。
    let mut seen = vec![false; XW * YH * ZW];
    let mut biggest = 0usize;
    let mut stack: Vec<(usize, i32, usize)> = Vec::new();
    for z in 0..ZW {
        for x in 0..XW {
            for y in -56..64_i32 {
                let i = wid(x, y, z);
                if win[i] != AIR || seen[i] {
                    continue;
                }
                let mut size = 0usize;
                seen[i] = true;
                stack.push((x, y, z));
                while let Some((cx0, cy, cz0)) = stack.pop() {
                    size += 1;
                    for dz in -1..=1_isize {
                        for dy in -1..=1_i32 {
                            for dx in -1..=1_isize {
                                let nx = cx0 as i32 + dx as i32;
                                let nz = cz0 as i32 + dz as i32;
                                let ny = cy + dy;
                                if nx < 0
                                    || nz < 0
                                    || nx as usize >= XW
                                    || nz as usize >= ZW
                                    || !(-56..64).contains(&ny)
                                {
                                    continue;
                                }
                                let j = wid(nx as usize, ny, nz as usize);
                                if win[j] == AIR && !seen[j] {
                                    seen[j] = true;
                                    stack.push((nx as usize, ny, nz as usize));
                                }
                            }
                        }
                    }
                }
                biggest = biggest.max(size);
            }
        }
    }
    let air_total: usize = win.iter().filter(|&&v| v == AIR).count();
    println!(
        "K-CAVE-AUDIT seed={SEED:#x} window={}x{ZW}x{YH} air_total~={} \
         largest_void_component={biggest}",
        XW, air_total
    );
    // 连通性下限只防「洞网全碎片化」系统病（carver 管层缺席导致的细洞
    // 连通性偏弱是已知缺口 CAVE-2，量化数值以 K-CAVE-AUDIT 日志为准）。
    assert!(biggest > 50, "最大洞体仅 {biggest} 格——洞网全碎片化病");
}
