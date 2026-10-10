//! 纯 Rust 地形内核——`cpp/src/terrain.cpp`（frozen oracle，任务板 #77
//! 已删除）的逐位移植，本文件是被黄金数据锚定的冻结基线。
//!
//! 机制：三层噪声高度图（大陆度/侵蚀度/山脊 + 锯齿细节）+ 双阈值洞穴
//! （意面管道 + 奶酪大洞）+ 跨区块确定性树投影 + 遮光高度图。公共入口
//! [`generate`] 与 C ABI `mcv_terrain_generate`（cpp/include/mcv.h:43-44）
//! 同语义：填充 65536 个 u16 方块 id（布局 `(y<<8)|(z<<4)|x`）+ 256 项
//! u8 高度图（索引 `(z<<4)|x`，值 = 最高遮光格 y + 1）。
//!
//! 位一致纪律见 [`crate::rust_noise`] 模块注释；对拍由
//! `tests/golden.rs` 以黄金数据（#77 删除前从该 oracle 落盘）逐字节
//! 回归锁定——**回归锁，非正确性标准**（黄金值是旧近似语义，26.1 权威
//! 门在 vanilla 后端 + tests/quality.rs）。出处标注 `terrain.cpp:行号`。

use crate::rust_noise::{fbm2, fbm2_w, fbm3, fbm3_w, hash01, peaks_valleys};

/// 世界高（terrain.cpp:20）。
const SY: i32 = 256;
/// 海平面（terrain.cpp:21）。
const SEA: i32 = 96;

// Block ids mirror mcv_core BLOCKS order（terrain.cpp:24-35）。
const AIR: u16 = 0;
const STONE: u16 = 1;
const DIRT: u16 = 2;
const GRASS: u16 = 3;
const SAND: u16 = 4;
const WATER: u16 = 5;
const LOG: u16 = 6;
const LEAVES: u16 = 7;
const BEDROCK: u16 = 10;
const SNOW_GRASS: u16 = 11;
const FLOWER_RED: u16 = 12;
const FLOWER_YELLOW: u16 = 13;

// ---- 通道波长（terrain.cpp:38-45）----
const WAVELENGTH_CONT: f32 = 256.0; // continentalness 2048 ÷8（封顶）
const WAVELENGTH_EROSION: f32 = 256.0; // erosion 2048 ÷8
const WAVELENGTH_RIDGE: f32 = 64.0; // ridges 512 ÷8（保持 4:1）
const WAVELENGTH_DETAIL: f32 = 48.0; // jagged 基波 ≈43.7（1:1 可步行）
const SPAG_XZ: f32 = 128.0; // spaghetti_3d_* 单倍频 128（1:1）
const SPAG_Y: f32 = 128.0;
const CHEESE_XZ: f32 = 256.0; // cave_cheese 384 → 封顶 256
const CHEESE_Y: f32 = 160.0; // 世界高 384→256，纵向再压缩

// ---- 倍频振幅序列（terrain.cpp:49-52）----
const AMP_CONT: [f32; 6] = [1.0, 1.0, 2.0, 2.0, 2.0, 1.0];
const AMP_EROSION: [f32; 5] = [1.0, 1.0, 0.0, 1.0, 1.0];
const AMP_RIDGE: [f32; 3] = [1.0, 2.0, 1.0];
const AMP_CHEESE: [f32; 5] = [0.5, 1.0, 2.0, 1.0, 2.0];

// ---- 洞穴阈值（terrain.cpp:57-65）----
const SPAG_THICK_MIN: f32 = 0.065;
const SPAG_THICK_DEEP: f32 = 0.023; // = 0.088 − 0.065，深处更宽
const SPAG_DEEP_REF_Y: f32 = 40.0; // 深度基准
const CHEESE_THRESH: f32 = 0.60;
const CHEESE_THRESH_SHALLOW: f32 = 0.66;
const ENTRANCE_FADE: i32 = 14; // 洞口渐隐带厚度

// ---- 通道/装饰种子（terrain.cpp:67-77）----
const SEED_CONT: u64 = 0x1111_2222_3333_4444;
const SEED_EROSION: u64 = 0x2468_ACE0_1357_9BDF;
const SEED_RIDGE: u64 = 0x5555_6666_7777_8888;
const SEED_DETAIL: u64 = 0x9999_AAAA_BBBB_CCCC;
const SEED_SPAG1: u64 = 0x5EED_0000_C0DE_0001;
const SEED_SPAG2: u64 = 0x5EED_0000_C0DE_0002;
const SEED_CHEESE: u64 = 0x5EED_0000_C0DE_0003;
const SEED_FOREST: u64 = 0xF0FE_F0FE_F0FE_F0FE;
const SEED_TREE: u64 = 0x7EE5_7EE5_7EE5_7EE5;
const SEED_FLOWER: u64 = 0xF10B_F10B_F10B_F10B;
const SEED_MISC: u64 = 0x0D00_D00D_0D00_D00D;

/// 体素索引（terrain.cpp:79-82）：`(y<<8) | (z<<4) | x`。
fn vidx(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

/// 欧几里得向下取整除（terrain.cpp:84-90）：C++ 截断除 + 符号修正。
fn floor_div(v: i32, d: i32) -> i32 {
    let mut q = v / d;
    if v % d != 0 && (v < 0) != (d < 0) {
        q -= 1;
    }
    q
}

/// 地表高度（terrain.cpp:96-132）：大陆度 + 侵蚀度门控折叠山脊 + 锯齿
/// 细节。加法链结合序与 C++ 左结合一致（常数 96+4 先折叠为 100.0）。
fn base_height(seed: u64, wx: f32, wz: f32) -> f32 {
    let cont = fbm2_w(
        seed ^ SEED_CONT,
        wx / WAVELENGTH_CONT,
        wz / WAVELENGTH_CONT,
        &AMP_CONT,
    ) * 2.0
        - 1.0;
    let ero = fbm2_w(
        seed ^ SEED_EROSION,
        wx / WAVELENGTH_EROSION,
        wz / WAVELENGTH_EROSION,
        &AMP_EROSION,
    ) * 2.0
        - 1.0;
    let r = fbm2_w(
        seed ^ SEED_RIDGE,
        wx / WAVELENGTH_RIDGE,
        wz / WAVELENGTH_RIDGE,
        &AMP_RIDGE,
    ) * 2.0
        - 1.0;
    let ridge = 0.0_f32.max(peaks_valleys(r));
    let inland = (cont * 1.6 + 0.15).clamp(0.0, 1.0) * (1.0 - 0.6 * ero.max(0.0));
    let det = (fbm2(
        seed ^ SEED_DETAIL,
        wx / WAVELENGTH_DETAIL,
        wz / WAVELENGTH_DETAIL,
        3,
    ) * 2.0
        - 1.0)
        * 3.0;
    SEA as f32 + 4.0 + cont * 26.0 + ridge * inland * 38.0 + det
}

/// 森林密度掩码 [0, 1)（terrain.cpp:135-137）。
fn forest_mask(seed: u64, wx: f32, wz: f32) -> f32 {
    fbm2(seed ^ SEED_FOREST, wx / 300.0, wz / 300.0, 2)
}

/// 双阈值洞穴判定（terrain.cpp:144-200）：意面管道（两路噪声取 max、
/// |n| < 厚度带）+ 奶酪大洞；洞口随深度渐隐；海底水密。
fn carve_cave(seed: u64, x: i32, y: i32, z: i32, surface: i32, ocean: bool) -> bool {
    if y <= 2 || y > surface {
        return false;
    }
    if ocean && y > SEA - 6 {
        return false;
    }

    let a = fbm3(
        seed ^ SEED_SPAG1,
        x as f32 / SPAG_XZ,
        y as f32 / SPAG_Y,
        z as f32 / SPAG_XZ,
        2,
    ) * 2.0
        - 1.0;
    let b = fbm3(
        seed ^ SEED_SPAG2,
        x as f32 / SPAG_XZ,
        y as f32 / SPAG_Y,
        z as f32 / SPAG_XZ,
        2,
    ) * 2.0
        - 1.0;
    let deep = ((SPAG_DEEP_REF_Y - y as f32) / SPAG_DEEP_REF_Y).clamp(0.0, 1.0);
    let thick = SPAG_THICK_MIN + SPAG_THICK_DEEP * deep;
    let m = a.abs().max(b.abs());
    let tunnel = m < thick;

    let c = fbm3_w(
        seed ^ SEED_CHEESE,
        x as f32 / CHEESE_XZ,
        y as f32 / CHEESE_Y,
        z as f32 / CHEESE_XZ,
        &AMP_CHEESE,
    ) * 2.0
        - 1.0;
    let cavern = c > if y < 40 {
        CHEESE_THRESH
    } else {
        CHEESE_THRESH_SHALLOW
    };

    if !tunnel && !cavern {
        return false;
    }

    // 洞口渐隐（terrain.cpp:190-198）：tunnels 比 caverns 更易开口。
    let depth_below = surface - y;
    if depth_below < ENTRANCE_FADE {
        let t = depth_below as f32 / ENTRANCE_FADE as f32;
        let gate = hash01(seed ^ SEED_MISC, i64::from(x), i64::from(y), i64::from(z));
        let keep = if tunnel { 0.35 + 0.65 * t } else { 0.75 * t };
        if gate > keep {
            return false;
        }
    }
    true
}

/// 一格 5×5 抖动树池的确定性树查询（terrain.cpp:202-233）。返回 `None`
/// 表示该池无树。
struct TreeInfo {
    x: i32, // 世界坐标
    z: i32,
    base_y: i32, // 地表 y（树干从 base_y + 1 起）
    height: i32, // 树干 4..6
}

fn tree_in_cell(seed: u64, cell_x: i32, cell_z: i32) -> Option<TreeInfo> {
    let cx = i64::from(cell_x);
    let cz = i64::from(cell_z);
    let gate = hash01(seed ^ SEED_TREE, cx, cz, 1);
    let jx = hash01(seed ^ SEED_TREE, cx, cz, 2);
    let jz = hash01(seed ^ SEED_TREE, cx, cz, 3);
    let jh = hash01(seed ^ SEED_TREE, cx, cz, 4);
    // C++ 截断转换 + 有符号乘（界内不溢出；wrapping 仅防御极端 chunk 坐标）。
    let wx = cell_x.wrapping_mul(5) + (jx * 5.0) as i32;
    let wz = cell_z.wrapping_mul(5) + (jz * 5.0) as i32;
    let sy = base_height(seed, wx as f32, wz as f32) as i32;
    if sy <= SEA || sy > 148 {
        return None; // 滩涂/水面/雪峰不长树
    }
    let density = 0.010 + forest_mask(seed, wx as f32, wz as f32) * 0.045;
    if gate >= density {
        return None;
    }
    Some(TreeInfo {
        x: wx,
        z: wz,
        base_y: sy,
        height: 4 + (jh * 3.0) as i32,
    })
}

/// 树冠 + 树干投影（terrain.cpp:237-275）：世界→本区块局部裁剪使跨区块
/// 投影天然隐式（树落点是世界坐标纯函数）。
fn stamp_tree(seed: u64, voxels: &mut [u16], t: &TreeInfo, base_x: i32, base_z: i32) {
    let top = t.base_y + t.height;
    for dy in -2..=1 {
        let y = top + dy;
        if !(0..SY).contains(&y) {
            continue;
        }
        let r: i32 = if dy <= -1 { 2 } else { 1 };
        for dx in -r..=r {
            for dz in -r..=r {
                if dy == 1 && dx.abs() + dz.abs() > 1 {
                    continue; // 顶盖是十字形
                }
                if dy <= -1
                    && dx.abs() == r
                    && dz.abs() == r
                    && hash01(
                        seed ^ SEED_MISC,
                        i64::from(t.x + dx),
                        i64::from(y),
                        i64::from(t.z + dz),
                    ) < 0.5
                {
                    continue; // 削下层四角
                }
                let lx = t.x + dx - base_x;
                let lz = t.z + dz - base_z;
                if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
                    continue;
                }
                let cell = &mut voxels[vidx(lx as usize, y as usize, lz as usize)];
                if *cell == AIR {
                    *cell = LEAVES;
                }
            }
        }
    }
    let tx = t.x - base_x;
    let tz = t.z - base_z;
    if (0..16).contains(&tx) && (0..16).contains(&tz) {
        let mut y = t.base_y + 1;
        while y <= top && y < SY {
            voxels[vidx(tx as usize, y as usize, tz as usize)] = LOG;
            y += 1;
        }
    }
}

/// 纯 Rust 生成一个区块：体素 + 高度图。语义与 `mcv_terrain_generate`
/// 一致（mcv.h:39-44）；安全切片入参使空指针分支（MCV_ERR_NULL_ARG）在
/// 类型层面不可达。
///
/// # Errors
/// 恒返回 `Ok(())`；保留 `Result` 以与 FFI 路径
/// （[`crate::generate_terrain_with`]）同构。
pub fn generate(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    out_voxels: &mut [u16],
    out_heightmap: &mut [u8],
) -> Result<(), i32> {
    debug_assert!(out_voxels.len() >= mcv_core::CHUNK_VOL);
    debug_assert!(out_heightmap.len() >= 256);
    // terrain.cpp:285-286（wrapping 对齐 C++ 实际回绕行为，防 debug panic）。
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);

    // Pass 1（terrain.cpp:289-335）：柱层序、水、基岩、洞穴、花。
    for z in 0..16_i32 {
        for x in 0..16_i32 {
            let wx = base_x.wrapping_add(x);
            let wz = base_z.wrapping_add(z);
            let surface = (base_height(seed, wx as f32, wz as f32) as i32).clamp(4, SY - 10);
            let ocean = surface <= SEA;
            let beach = surface <= SEA + 1;
            let snowy = surface > 142;
            let top = surface.max(SEA);

            for y in 0..=top {
                let id = if y == 0
                    || (y <= 2
                        && hash01(seed ^ SEED_MISC, i64::from(wx), i64::from(y), i64::from(wz))
                            < 0.5)
                {
                    BEDROCK
                } else if y > surface {
                    if y <= SEA { WATER } else { AIR }
                } else if y == surface {
                    if beach {
                        SAND
                    } else if snowy {
                        SNOW_GRASS
                    } else {
                        GRASS
                    }
                } else if y >= surface - 3 {
                    if beach { SAND } else { DIRT }
                } else {
                    STONE
                };

                let id = if id != AIR
                    && id != BEDROCK
                    && id != WATER
                    && carve_cave(seed, wx, y, wz, surface, ocean)
                {
                    AIR
                } else {
                    id
                };
                out_voxels[vidx(x as usize, y as usize, z as usize)] = id;
            }

            // 完好草柱上撒花（terrain.cpp:326-333）。
            if out_voxels[vidx(x as usize, surface as usize, z as usize)] == GRASS
                && surface + 1 < SY
                && out_voxels[vidx(x as usize, (surface + 1) as usize, z as usize)] == AIR
            {
                let f = hash01(seed ^ SEED_FLOWER, i64::from(wx), 7, i64::from(wz));
                if f < 0.006 {
                    out_voxels[vidx(x as usize, (surface + 1) as usize, z as usize)] =
                        if f < 0.003 { FLOWER_RED } else { FLOWER_YELLOW };
                }
            }
        }
    }

    // Pass 2（terrain.cpp:340-351）：本区块及 8 邻的 5×5 树池扫描——树落
    // 点是世界坐标纯函数，跨区块树冠由此确定。
    let c0x = floor_div(base_x.wrapping_sub(2), 5);
    let c1x = floor_div(base_x.wrapping_add(17), 5);
    let c0z = floor_div(base_z.wrapping_sub(2), 5);
    let c1z = floor_div(base_z.wrapping_add(17), 5);
    for cell_z in c0z..=c1z {
        for cell_x in c0x..=c1x {
            if let Some(t) = tree_in_cell(seed, cell_x, cell_z) {
                stamp_tree(seed, out_voxels, &t, base_x, base_z);
            }
        }
    }

    // Pass 3（terrain.cpp:355-367）：高度图 = 最高遮光格 y + 1（空气/水/
    // 花不遮直射天光，树叶遮以便树荫）；全开柱 y 归零后取 1。
    for z in 0..16_usize {
        for x in 0..16_usize {
            let mut y = SY - 1;
            while y > 0 {
                let id = out_voxels[vidx(x, y as usize, z)];
                if id != AIR && id != WATER && id != FLOWER_RED && id != FLOWER_YELLOW {
                    break;
                }
                y -= 1;
            }
            out_heightmap[(z << 4) | x] = (y + 1) as u8;
        }
    }

    Ok(())
}
