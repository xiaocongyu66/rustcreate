//! C++ oracle（mcv_ffi → cpp/src/terrain.cpp）↔ 纯 Rust 地形生成
//! （src/rust_terrain.rs + src/rust_noise.rs）逐字节对拍——任务板 #78
//! 第一阶段硬门禁。
//!
//! 铁律：对拍不过修 Rust 侧，禁改 cpp/**；断言只许收紧（逐字节相等），
//! 不许放宽（无 epsilon、无抽样）。覆盖：4 seed × 4 个 5×5 区块网格偏移
//! （两轴都含负坐标）= 400 对区块；voxels 65536×u16 + heightmap 256×u8
//! 全量比对。噪声层另有 f32 位锚点单测（src/rust_noise.rs
//! `#[cfg(test)]`，已知答案 + 位模式，无 epsilon）。

use mcv_core::ChunkPos;
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

const VOL: usize = 65536;

const AIR: u16 = 0;
const WATER: u16 = 5;
const BEDROCK: u16 = 10;
const FLOWER_RED: u16 = 12;
const FLOWER_YELLOW: u16 = 13;

/// 4 个 seed：0 与 u64::MAX 是异或/回绕通路的两端边界。
const SEEDS: [u64; 4] = [0, 42, 0xDEAD_BEEF_CAFE_F00D, u64::MAX];

/// 每 seed 的网格偏移：保证负坐标区块进入 x/z 两轴。
const OFFSETS: [(i32, i32); 4] = [(0, 0), (-7, -3), (9, -12), (-15, 8)];

/// 网格半径：每 seed 覆盖 5×5 = 25 个区块（远超派单下限 16）。
const GRID: i32 = 2;

/// 生成 Rust 输出的高度图语义复算（排除集与 terrain.cpp:355-367 一致：
/// air/water/两种花不遮光）。
fn expected_heightmap(voxels: &[u16]) -> [u8; 256] {
    let mut hm = [0u8; 256];
    for z in 0..16usize {
        for x in 0..16usize {
            let mut y = 255usize;
            let mut top = 0usize; // 无遮光格 ⇒ 1（与 C++ 全开柱语义一致）
            while y > 0 {
                let id = voxels[(y << 8) | (z << 4) | x];
                if id != AIR && id != WATER && id != FLOWER_RED && id != FLOWER_YELLOW {
                    top = y;
                    break;
                }
                y -= 1;
            }
            hm[(z << 4) | x] = (top + 1) as u8;
        }
    }
    hm
}

/// 同 seed 同区块跑两条后端，全量逐字节断言；并锁 Rust 侧基本不变量。
fn parity_one(seed: u64, cx: i32, cz: i32) {
    let oracle = generate_terrain_with(TerrainBackend::Ffi, seed, ChunkPos::new(cx, cz))
        .expect("oracle 路径生成失败");
    let rust = generate_terrain_with(TerrainBackend::Legacy, seed, ChunkPos::new(cx, cz))
        .expect("rust 路径生成失败");

    let a = oracle.voxels.as_u16_slice();
    let b = rust.voxels.as_u16_slice();
    assert_eq!(a.len(), VOL);
    // 同进程同端序下 u16 逐元素相等 ⇔ 131072 字节逐字节相等。
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            x,
            y,
            "seed {seed} chunk ({cx},{cz}): voxel 首差异 idx {i}（x={}, z={}, y={}）",
            i & 0xF,
            (i >> 4) & 0xF,
            i >> 8
        );
    }
    assert_eq!(
        oracle.heightmap, rust.heightmap,
        "seed {seed} chunk ({cx},{cz}): heightmap 不一致"
    );

    // Rust 侧不变量（防两侧同错的独立哨兵）：基岩地板 + 高度图语义自洽。
    // 索引 (y<<8)|(z<<4)|x：(0,0,0) 即 0；(0,15,15) 即 (15<<4)|15。
    assert_eq!(b[0], BEDROCK, "Rust 路径 (0,0,0) 应为基岩");
    assert_eq!(b[(15 << 4) | 15], BEDROCK, "Rust 路径 (15,0,15) 应为基岩");
    let rust_hm: [u8; 256] = *rust.heightmap;
    assert_eq!(
        rust_hm,
        expected_heightmap(b),
        "seed {seed} chunk ({cx},{cz}): Rust 高度图与其体素语义不自洽"
    );
}

#[test]
fn terrain_byte_parity_4_seeds_x_25_chunks() {
    let mut pairs = 0usize;
    for seed in SEEDS {
        for (ox, oz) in OFFSETS {
            for dx in -GRID..=GRID {
                for dz in -GRID..=GRID {
                    parity_one(seed, ox + dx, oz + dz);
                    pairs += 1;
                }
            }
        }
    }
    // 覆盖 = 4 seed × 4 网格偏移 × 25 区块 = 400 对（每 seed 4 片各含负
    // 坐标的 5×5 邻域，远超派单下限 3 seed × 16 区块）。
    assert_eq!(pairs, 400, "对拍覆盖应为 4 seed × 4 偏移 × 25 区块");
}

#[test]
fn rust_backend_deterministic_same_seed() {
    // Rust 路径自确定性（oracle 侧同类性质已由 tests/terrain.rs 锁定）。
    let a = generate_terrain_with(TerrainBackend::Legacy, 42, ChunkPos::new(3, -7)).expect("gen");
    let b = generate_terrain_with(TerrainBackend::Legacy, 42, ChunkPos::new(3, -7)).expect("gen");
    assert_eq!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
    assert_eq!(a.heightmap, b.heightmap);
}

#[test]
fn rust_backend_differs_across_seeds() {
    // 种子敏感性哨兵：全 1 位（u64::MAX）与 0 的异或通路必须分叉。
    let a = generate_terrain_with(TerrainBackend::Legacy, 0, ChunkPos::new(0, 0)).expect("gen");
    let b =
        generate_terrain_with(TerrainBackend::Legacy, u64::MAX, ChunkPos::new(0, 0)).expect("gen");
    assert_ne!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
}
