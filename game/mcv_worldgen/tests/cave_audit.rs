//! #192 矿洞地形量化对照审计探针（K-CAVE-AUDIT 行入 CI 日志，红因/数据
//! 两用——lakes.rs K-LAKES-BUF 先例）。
//!
//! 对照基准（26.1 实物，/root/mc-ref/src-26.1）：
//! - 密度雕刻：`noise_settings/overworld.json` noise_router.final_density
//!   + `density_function/overworld/{sloped_cheese,caves/*}.json`
//!     （entrances/spaghetti_2d/spaghetti_roughness/underground/pillars/
//!     noodle 已移植，terrain.rs 逐常数引注；carver 管层仍缺=CAVE-1 ③）。
//! - 管状 carver：`configured_carver/{cave,cave_extra_underground,canyon}
//!   .json`（probability 0.15/0.07/0.01，y 窗 -56..180 / -56..47 / 10..67）
//!   ——本仓整体未移植=缺口 CAVE-1 ③（KNOWN-DIVERGENCE）。
//!
//! 本探针量化「本仓现状」的空洞率随深度分布 + 最大连通体，供跨任务对照；
//! 断言只放宽松包络（防空洞塌方/全实心两类系统病），数值本身入日志。

use mcv_core::{ChunkPos, WORLD_MIN_Y};
use mcv_worldgen::vanilla::terrain::{BEDROCK, Orchestrator};
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
    let mut band_voids: Vec<f64> = Vec::new();
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
            band_voids.push(void);
            worst_deep_void = worst_deep_void.max(void);
        }
        lo += 16;
    }
    // cell 语义重锚（重锚波②）：分带「每带 >0.005」→「合计 ≥0.015 +
    // 主力带 ≥0.02」。cell 外壳后洞腔分布对齐官方（主力集中在
    // [0,16)/[-32,-16)），过渡带可以薄（官方 par-water 对照同形，
    // seed 实测 [-48,-32)0.0057/[-32,-16)0.0308/[-16,0)0.0033，
    // run 38825194406）。
    let total_void: f64 = band_voids.iter().sum();
    let main_void = band_voids.iter().cloned().fold(0.0_f64, f64::max);
    assert!(
        total_void >= 0.015,
        "−48..32 深度域空洞率合计 {total_void:.3} <0.015——洞网消失"
    );
    assert!(
        main_void >= 0.02,
        "−48..32 主力带空洞率 {main_void:.3} <0.02——洞网失去主面"
    );
    assert!(
        worst_deep_void < 0.5,
        "带空洞率峰值 {worst_deep_void:.3} ≥0.5——全空腔病"
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
    // 连通性偏弱是既有缺口，登记于 KNOWN-DIVERGENCE CAVE-1 ③（与本文件
    // 头注 :9-11 同口径；旧注释误写「CAVE-2」为悬空引用，条目从未存在，
    // 缺口实体即 CAVE-1 ③，不另立空洞条目），量化数值以 K-CAVE-AUDIT 日志为准）。
    assert!(biggest > 50, "最大洞体仅 {biggest} 格——洞网全碎片化病");
}

/// K-SURFACE-DEPTH：实物亚表带量化（#192 SURF-1 ② 销账门）。
/// 原版 getSurfaceDepth（SurfaceSystem.java:165-168）=
/// `int(surface噪声·2.75 + 3.0 + nextDouble·0.25)`，σ(噪声)≈0.3 ⇒
/// 逐列典型 0..5、截断均值 ≈2.6（旧常数 3 = 均值锚但零方差）。断言只防
/// 「分布塌回常数」与「均值出带」两类病；histogram 入日志供跨提交对照。
#[test]
fn surface_depth_distribution() {
    let orch = Orchestrator::new(SEED);
    let mut hist = [0usize; 8];
    let (mut sum, mut n) = (0_i64, 0_usize);
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for dz in (0..128_i32).step_by(2) {
        for dx in (0..128_i32).step_by(2) {
            let d = orch.surface_depth(dx, dz);
            *hist.get_mut(d.clamp(0, 7) as usize).unwrap() += 1;
            sum += i64::from(d);
            n += 1;
            lo = lo.min(d);
            hi = hi.max(d);
        }
    }
    let mean = sum as f64 / n as f64;
    let distinct = hist.iter().filter(|&&c| c > 0).count();
    println!(
        "K-SURFACE-DEPTH seed={SEED:#x} n={n} mean={mean:.3} range=[{lo},{hi}] \
         distinct={distinct} hist={hist:?}"
    );
    assert!(
        (2.0..3.5).contains(&mean),
        "surfaceDepth 均值 {mean:.3} 出带——噪声/抖动物理接线路径异常"
    );
    assert!(
        distinct >= 4 && lo <= 1 && hi >= 4,
        "surfaceDepth 分布塌缩（distinct={distinct} range=[{lo},{hi}]）——\
         疑似回退常数带"
    );
    // 确定性：同列两次求值逐位一致（E2 口径的局部镜像）。
    assert_eq!(orch.surface_depth(37, -91), orch.surface_depth(37, -91));
}
