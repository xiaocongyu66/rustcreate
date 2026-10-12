//! #172 sub-chunk 重网格等价锁：`build_products(...).combine()` 与
//! `Mesher::build`（= mesher::build_mesh 直呼）**逐字节相同**。
//!
//! 两把锁：
//! 1. 全柱重放（rows=None）：所有黄金场景 × 两种 kind，产物拼接 == 直呼；
//! 2. 单格编辑重放（rows=Some(edit)）：旧快照全量产物 + 编辑盒重放 +
//!    其余桶复用 == 编辑后快照直呼——热路径的字节等价面。
//!
//! 另附工作量观测面断言（重放行数 < 全柱，验收 3）。

// golden/cases.rs 是全族测试共用输入规格（场景/哈希/随机），本目标只用
// 得到其中一块——未用项在 -D warnings 下按死代码红，统一在目标顶层放行。
#![allow(dead_code)]

#[path = "golden/cases.rs"]
mod cases;

use cases::{Chunk, Rng, SCENES, chunk, full9, only_center, put, random_chunk, scene_by_name};
use mcv_mesher::{
    MESH_OPAQUE, MESH_WATER, Mesher, Rows, SECTION_COUNT, Slot, TOTAL_SCANNED_ROWS, edit_rows,
};

fn blob(m: &mcv_mesher::MeshData) -> (Vec<u8>, Vec<u32>) {
    (m.vertices.clone(), m.indices.clone())
}

fn slots_of<'a>(center: &'a Chunk, side: &'a Chunk) -> [Option<mcv_mesher::Slot<'a>>; 9] {
    full9(center, side)
}

/// 锁 1：全柱重放 == 直呼（黄金场景 × loaded/center-only × kind）。
#[test]
fn full_products_combine_equals_direct_build() {
    let mesher = Mesher::new();
    for (name, loaded) in SCENES {
        let center = scene_by_name(name);
        let side = chunk(0, 0xF0);
        // only_center：邻槽 None（opaque 边界），复用 cases 共用构造。
        let slots = if loaded {
            slots_of(&center, &side)
        } else {
            only_center(&center)
        };
        for kind in [MESH_OPAQUE, MESH_WATER] {
            let direct = mesher.build(&slots, kind).expect("direct build");
            let (prod, work) = mesher
                .build_products(&slots, kind, None, None)
                .expect("products");
            let combined = prod.combine();
            assert_eq!(
                blob(&combined),
                blob(&direct),
                "{name} k{kind}: 全柱重放 combine != 直呼"
            );
            assert_eq!(
                work,
                TOTAL_SCANNED_ROWS + SECTION_COUNT,
                "{name} k{kind}: 全柱重放工作量观测面"
            );
        }
    }
}

/// 编辑一次中心区块的单格（含光照扰动），返回新快照。
fn edited(center: &Chunk, x: usize, y: i32, z: usize, id: u16, light: u8) -> Chunk {
    let mut vox = center.0.clone();
    let mut lit = center.1.clone();
    put(&mut vox, x, y, z, id);
    let idx = mcv_core::vidx(x, y, z);
    lit[idx] = light;
    // 邻格光扰动：验证重光照波及盒同一 rows 机制（脏盒 ±1 膨胀覆盖）。
    if y + 1 < mcv_core::WORLD_MAX_Y {
        lit[mcv_core::vidx(x, y + 1, z)] ^= 0x11;
    }
    (vox, lit)
}

/// 锁 2：单格编辑 → 编辑盒重放 + 旧产物复用 == 编辑后快照直呼。
#[test]
fn single_edit_products_combine_equals_direct_build() {
    let mesher = Mesher::new();
    let side = chunk(0, 0xF0);
    // 边界敏感 y：世界底/顶、section 边界 ±1。
    let ys = [
        mcv_core::WORLD_MIN_Y,
        -63,
        -49,
        -48,
        -1,
        0,
        15,
        79,
        80,
        81,
        318,
        mcv_core::WORLD_MAX_Y - 1,
    ];
    let xs = [0usize, 8, 15];
    let zs = [0usize, 8, 15];
    let stone = mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == "stone")
        .unwrap_or(1) as u16;
    let scenes = ["plains", "cave", "water", "noncube_mix", "bounds"];
    let mut checked = 0usize;
    for name in scenes {
        let center_a = scene_by_name(name);
        let slots_a = slots_of(&center_a, &side);
        // 旧全量产物（两 kind）。
        let mut old = [None, None];
        for (i, kind) in [MESH_OPAQUE, MESH_WATER].iter().enumerate() {
            let (p, _) = mesher
                .build_products(&slots_a, *kind, None, None)
                .expect("old products");
            old[i] = Some(p);
        }
        for &y in &ys {
            for (&x, &z) in xs.iter().zip(zs.iter()) {
                let center_b = edited(&center_a, x, y, z, stone, 0x5A);
                let slots_b = slots_of(&center_b, &side);
                let rows = edit_rows(x, y, z);
                for (i, kind) in [MESH_OPAQUE, MESH_WATER].iter().enumerate() {
                    let direct = mesher.build(&slots_b, *kind).expect("direct build");
                    let (prod, work) = mesher
                        .build_products(&slots_b, *kind, Some(rows), old[i].take())
                        .expect("incremental");
                    let combined = prod.combine();
                    assert_eq!(
                        blob(&combined),
                        blob(&direct),
                        "{name} k{kind} edit({x},{y},{z}): 编辑重放 combine != 直呼"
                    );
                    assert!(
                        work < TOTAL_SCANNED_ROWS + SECTION_COUNT,
                        "{name} k{kind} edit({x},{y},{z}): 重放工作量未降（{work}）"
                    );
                    // 复用面观测：脏行集 = 2×(≤3+≤3+≤3) 行 + ≤3 形状段；
                    // y 顶/底界 clamp 只会更小。
                    assert!(work <= 2 * 9 + SECTION_COUNT, "work={work}");
                    old[i] = Some(prod);
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 200, "覆盖样本过少: {checked}");
}

/// 随机地形随机编辑扫描（seeded，无固定场景盲区）：单格 + 远处第二格
/// （Rows::union 包络路径）。
#[test]
fn random_terrain_edit_sweep_bit_identical() {
    let mesher = Mesher::new();
    let side = chunk(0, 0xF0);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..24 {
        let center_a = random_chunk(&mut rng);
        let slots_a = slots_of(&center_a, &side);
        let mut old = [None, None];
        for (i, kind) in [MESH_OPAQUE, MESH_WATER].iter().enumerate() {
            old[i] = Some(
                mesher
                    .build_products(&slots_a, *kind, None, None)
                    .expect("old products")
                    .0,
            );
        }
        let x0 = rng.below(16) as usize;
        let z0 = rng.below(16) as usize;
        let y0 = mcv_core::WORLD_MIN_Y + rng.below(mcv_core::CHUNK_SY as u64) as i32;
        let mut rows = edit_rows(x0, y0, z0);
        // 第二次编辑：任意方向；y 不相邻时验证 union 包络保守面。
        let x1 = rng.below(16) as usize;
        let z1 = rng.below(16) as usize;
        let y1 = mcv_core::WORLD_MIN_Y + rng.below(mcv_core::CHUNK_SY as u64) as i32;
        rows = rows.union(&edit_rows(x1, y1, z1));
        let mut center_b = edited(&center_a, x0, y0, z0, 1 + rng.below(6) as u16, 0xF0);
        center_b = edited(&center_b, x1, y1, z1, 1 + rng.below(6) as u16, 0xA5);
        let slots_b = slots_of(&center_b, &side);
        for (i, kind) in [MESH_OPAQUE, MESH_WATER].iter().enumerate() {
            let direct = mesher.build(&slots_b, *kind).expect("direct build");
            let (prod, work) = mesher
                .build_products(&slots_b, *kind, Some(rows), old[i].take())
                .expect("incremental");
            assert_eq!(
                blob(&prod.combine()),
                blob(&direct),
                "round {round} k{kind}: 随机双编辑重放 combine != 直呼"
            );
            assert!(work <= TOTAL_SCANNED_ROWS + SECTION_COUNT);
            old[i] = Some(prod);
        }
    }
}

/// rows=None 连续重放（无 old）与 build_mesh 的 counts 一致性冒烟 +
/// 空快照空产物。
#[test]
fn empty_and_noop_paths() {
    let mesher = Mesher::new();
    let air = chunk(0, 0xF0);
    let slots = slots_of(&air, &air);
    for kind in [MESH_OPAQUE, MESH_WATER] {
        let direct = mesher.build(&slots, kind).expect("direct");
        assert_eq!(direct.counts(), (0, 0), "空气快照应为空网格");
        let (prod, work) = mesher
            .build_products(&slots, kind, Some(Rows::NONE), None)
            .expect("noop rebuild");
        assert_eq!(prod.combine().counts(), (0, 0));
        assert_eq!(work, 0, "NONE 行集重放量 = 0");
    }
}
