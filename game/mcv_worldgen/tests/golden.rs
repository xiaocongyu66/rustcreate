//! legacy 地形内核（src/rust_terrain.rs + src/rust_noise.rs）↔ 黄金数据
//! 逐字节回归锁——任务板 #77：C++ frozen oracle（原 mcv_ffi →
//! cpp/src/terrain.cpp）删除前，其输出在本机全量落盘固化（tests/golden/）。
//!
//! **回归锁，非正确性标准**：黄金值是 frozen oracle 的输出，即 legacy
//! 常数体系的旧近似语义——它不是「原版应该如此」的依据；26.1 的正确性
//! 门在 vanilla 后端 + tests/quality.rs（对标 src-26.1 反编译源码）。
//! 黄金文件唯一职责：C++ 删除后，Rust 移植仍与删除前的 oracle 输出
//! 逐字节一致（防重构手滑），对拍不过修 Rust 侧。
//!
//! 覆盖：4 seed × 4 个 5×5 区块网格偏移（两轴都含负坐标）= 400 对区块的
//! FNV-1a 哈希表 + 4 个代表区块的全量字节落盘（131,328 B/文件）；
//! 另有 Rust 侧基本不变量（基岩地板 + 高度图语义自洽）防同错哨兵。
//! 输入规格唯一来源见 tests/golden/cases.rs。

#[path = "golden/cases.rs"]
mod cases;

use cases::{DUMPS, GRID, OFFSETS, SEEDS, fnv1a, voxels_le};
use mcv_core::ChunkPos;
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

const VOL: usize = 65536;

const AIR: u16 = 0;
const WATER: u16 = 5;
const BEDROCK: u16 = 10;
const FLOWER_RED: u16 = 12;
const FLOWER_YELLOW: u16 = 13;

fn golden_dir() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden").to_string()
}

/// 黄金 TSV 逐行：`seed cx cz vox_hash hm_hash`（哈希为 016x 十六进制）。
fn load_table() -> Vec<(u64, i32, i32, u64, u64)> {
    let text = std::fs::read_to_string(format!("{}/terrain_parity.tsv", golden_dir()))
        .expect("黄金表应随仓库存在");
    let mut rows = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parts.len(), 5, "黄金表行格式漂移: {line}");
        rows.push((
            u64::from_str_radix(parts[0], 16).expect("seed hex"),
            parts[1].parse().expect("cx"),
            parts[2].parse().expect("cz"),
            u64::from_str_radix(parts[3], 16).expect("vox hash"),
            u64::from_str_radix(parts[4], 16).expect("hm hash"),
        ));
    }
    rows
}

/// 生成 Rust 输出的高度图语义复算（排除集与删除前的 terrain.cpp pass 3
/// 一致：air/water/两种花不遮光）。
fn expected_heightmap(voxels: &[u16]) -> [u8; 256] {
    let mut hm = [0u8; 256];
    for z in 0..16usize {
        for x in 0..16usize {
            let mut y = 255usize;
            let mut top = 0usize; // 无遮光格 ⇒ 1（全开柱语义）
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

/// 同 seed 同区块跑 legacy 后端，全量断言哈希 + 锁 Rust 侧基本不变量。
fn check_one(seed: u64, cx: i32, cz: i32, want_vox: u64, want_hm: u64) {
    let rust = generate_terrain_with(TerrainBackend::Legacy, seed, ChunkPos::new(cx, cz))
        .expect("legacy 路径生成失败");

    let b = rust.voxels.as_u16_slice();
    assert_eq!(b.len(), VOL);
    let got_vox = fnv1a(&voxels_le(b));
    assert_eq!(
        got_vox, want_vox,
        "seed {seed} chunk ({cx},{cz}): voxel 黄金哈希漂移"
    );
    let got_hm = fnv1a(&rust.heightmap[..]);
    assert_eq!(
        got_hm, want_hm,
        "seed {seed} chunk ({cx},{cz}): heightmap 黄金哈希漂移"
    );

    // Rust 侧不变量（防同错的独立哨兵）：基岩地板 + 高度图语义自洽。
    assert_eq!(b[0], BEDROCK, "legacy 路径 (0,0,0) 应为基岩");
    assert_eq!(b[(15 << 4) | 15], BEDROCK, "legacy 路径 (15,0,15) 应为基岩");
    let rust_hm: [u8; 256] = *rust.heightmap;
    assert_eq!(
        rust_hm,
        expected_heightmap(b),
        "seed {seed} chunk ({cx},{cz}): legacy 高度图与其体素语义不自洽"
    );
}

#[test]
fn terrain_golden_hash_4_seeds_x_25_chunks() {
    let rows = load_table();
    let mut pairs = 0usize;
    for row in &rows {
        check_one(row.0, row.1, row.2, row.3, row.4);
        pairs += 1;
    }
    // 输入规格自检：黄金表必须恰好覆盖 cases.rs 声明的 4 seed × 4 偏移 ×
    // 25 区块网格（共享常量源纪律——表与常量源不同步即红）。
    assert_eq!(
        pairs,
        SEEDS.len() * OFFSETS.len() * ((2 * GRID + 1) * (2 * GRID + 1)) as usize,
        "黄金表覆盖漂移"
    );
    let mut i = 0usize;
    for &seed in &SEEDS {
        for &(ox, oz) in &OFFSETS {
            for dx in -GRID..=GRID {
                for dz in -GRID..=GRID {
                    let (s, cx, cz, _, _) = rows[i];
                    assert_eq!(
                        (s, cx, cz),
                        (seed, ox + dx, oz + dz),
                        "黄金表第 {i} 行坐标漂移"
                    );
                    i += 1;
                }
            }
        }
    }
}

#[test]
fn terrain_golden_full_dumps_byte_identical() {
    for &(seed, cx, cz) in &DUMPS {
        let path = format!(
            "{}/terrain/terrain_s{seed:016x}_c{cx}_{cz}.bin",
            golden_dir()
        );
        let blob = std::fs::read(&path).expect("黄金全量落盘应随仓库存在");
        let rust = generate_terrain_with(TerrainBackend::Legacy, seed, ChunkPos::new(cx, cz))
            .expect("legacy 路径生成失败");
        let mut want = voxels_le(rust.voxels.as_u16_slice());
        want.extend_from_slice(&rust.heightmap[..]);
        let (got_len, want_len) = (blob.len(), want.len());
        assert_eq!(
            got_len, want_len,
            "seed {seed} chunk ({cx},{cz}): 落盘长度 {got_len} != Rust 输出 {want_len}"
        );
        for (i, (x, y)) in blob.iter().zip(want.iter()).enumerate() {
            let region = if i < 131072 { "voxel" } else { "heightmap" };
            assert_eq!(
                x, y,
                "seed {seed} chunk ({cx},{cz}): 全量字节首个差异在 {i}（{region}）"
            );
        }
    }
}

#[test]
fn legacy_backend_deterministic_same_seed() {
    let a = generate_terrain_with(TerrainBackend::Legacy, 42, ChunkPos::new(3, -7)).expect("gen");
    let b = generate_terrain_with(TerrainBackend::Legacy, 42, ChunkPos::new(3, -7)).expect("gen");
    assert_eq!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
    assert_eq!(a.heightmap, b.heightmap);
}

#[test]
fn legacy_backend_differs_across_seeds() {
    // 种子敏感性哨兵：全 1 位（u64::MAX）与 0 的异或通路必须分叉。
    let a = generate_terrain_with(TerrainBackend::Legacy, 0, ChunkPos::new(0, 0)).expect("gen");
    let b =
        generate_terrain_with(TerrainBackend::Legacy, u64::MAX, ChunkPos::new(0, 0)).expect("gen");
    assert_ne!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
}
