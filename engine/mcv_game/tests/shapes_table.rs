//! 全表形状不变量：BLOCKS 每个 `solid=true` 条目必须能产出 ≥1 个非空
//! （非退化）碰撞 AABB；反之 Cross/Torch 形状（原版 noCollision 花草/
//! 火把族，碰撞恒空）必须 `solid=false`。
//!
//! 背景（沙穿透审计）：引擎的玩家碰撞谓词是
//! `blockshapes::push_boxes`（Cube 分支 = `def().solid → 整格盒`），
//! 它对位原版 26.1 的 `hasCollision ? getShape : Shapes.empty()`
//! （BlockBehaviour.java:333-334）。生成器曾把 `forceSolidOn`（只抬
//! isSolid，BlockBehaviour.java:482-487，不产生碰撞）并入 solid，
//! 造出「solid=true 但形状给不出碰撞盒」的自相矛盾条目（bamboo_sapling）
//! 与「原版可穿行、本引擎凭空全盒碰撞」的 115 块（招牌/压力板/旗帜/
//! 凋珊瑚/蛛网…）。本测试是这类表-形状脱节的常驻回归锁。

use std::collections::HashMap;

use mcv_core::{BlockId, BlockPos, ChunkPos, Shape};
use mcv_game::VoxelAccess;
use mcv_game::blockshapes::{EMPTY_AABB, MAX_SHAPE_BOXES, RayTarget, push_boxes};

/// 全空气世界（栅栏臂等邻格查询都取不到连接对象）。
struct EmptyWorld;

impl VoxelAccess for EmptyWorld {
    fn block(&self, _p: BlockPos) -> BlockId {
        mcv_core::AIR
    }
    fn light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, _c: ChunkPos) -> bool {
        true
    }
}

/// HashMap 世界（定向条目用）。
struct Mem {
    m: HashMap<(i32, i32, i32), u16>,
}

impl VoxelAccess for Mem {
    fn block(&self, p: BlockPos) -> BlockId {
        BlockId(*self.m.get(&(p.x, p.y, p.z)).unwrap_or(&0))
    }
    fn light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, _c: ChunkPos) -> bool {
        true
    }
}

fn by_name(name: &str) -> Option<u16> {
    (0..mcv_core::BLOCKS.len() as u16).find(|i| mcv_core::BLOCKS[*i as usize].name == name)
}

/// 非退化：三轴 max 严格大于 min。
fn degenerate(b: &mcv_game::physics::Aabb) -> bool {
    !(b.max.x > b.min.x && b.max.y > b.min.y && b.max.z > b.min.z)
}

#[test]
fn every_solid_block_yields_a_nonempty_collision_box() {
    let w = EmptyWorld;
    let p = BlockPos::new(0, 0, 0);
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    let mut offenders: Vec<String> = Vec::new();
    for (i, def) in mcv_core::BLOCKS.iter().enumerate() {
        let id = BlockId(i as u16);
        let n = push_boxes(id, &w, p, RayTarget::Collide, &mut boxes);
        let shapes = &boxes[..n];
        if def.solid && (shapes.is_empty() || shapes.iter().any(degenerate)) {
            offenders.push(format!(
                "id {i} {} shape={:?} solid=true 碰撞盒数 {n}（含退化盒：{}）",
                def.name,
                Shape::from_u8(def.shape),
                shapes.iter().filter(|b| degenerate(b)).count()
            ));
        }
        // 反向：Cross/Torch 无碰撞形状 → solid 必须 false（否则是
        // forceSolidOn 误并入碰撞谓词的复发）。
        if matches!(Shape::from_u8(def.shape), Shape::Cross | Shape::Torch) && def.solid {
            offenders.push(format!(
                "id {i} {} shape={:?} 无碰撞形状却 solid=true",
                def.name,
                Shape::from_u8(def.shape)
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "碰撞形状-表脱节（{} 条）：\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

#[test]
fn liquid_and_air_have_no_collision() {
    let w = EmptyWorld;
    let p = BlockPos::new(0, 0, 0);
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    for (i, def) in mcv_core::BLOCKS.iter().enumerate() {
        let is_air_family = def.name == "air" || def.name.ends_with("_air");
        if !(def.liquid || is_air_family) {
            continue;
        }
        let n = push_boxes(BlockId(i as u16), &w, p, RayTarget::Collide, &mut boxes);
        assert_eq!(n, 0, "{} 液体/空气占位不应有碰撞盒", def.name);
    }
}

#[test]
fn sand_regression_full_cube() {
    // 真机症状锚点：海滩沙必须给出整格非退化碰撞盒。
    let sand = by_name("sand").expect("sand 在表");
    let w = Mem {
        m: [((0, 0, 0), sand)].into_iter().collect(),
    };
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    let n = collision_at(&w, BlockPos::new(0, 0, 0), &mut boxes);
    assert_eq!(n, 1, "沙应有 1 个碰撞盒");
    assert!(!degenerate(&boxes[0]), "沙碰撞盒不得退化");
    assert_eq!(boxes[0].min.x, 0.0);
    assert_eq!(boxes[0].max.y, 1.0);
}

#[test]
fn force_solid_blocks_have_no_phantom_collision() {
    // 原版 noCollision+forceSolidOn 族（招牌/压力板/旗帜/凋珊瑚/蛛网/
    // 竹笋/萤火灌木/幽匿脉络）：isSolid≠有碰撞（BlockBehaviour.java:
    // 482-487 vs 333-334），本引擎必须可穿行。
    for name in [
        "oak_sign",
        "oak_wall_sign",
        "cherry_hanging_sign",
        "stone_pressure_plate",
        "white_banner",
        "dead_tube_coral",
        "cobweb",
        "bamboo_sapling",
        "sculk_vein",
        "firefly_bush",
    ] {
        let id = by_name(name).unwrap_or_else(|| panic!("{name} 不在表"));
        assert!(
            !mcv_core::BLOCKS[id as usize].solid,
            "{name} 原版 noCollision → solid(hasCollision) 必须 false"
        );
        let w = Mem {
            m: [((0, 0, 0), id)].into_iter().collect(),
        };
        let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
        let n = collision_at(&w, BlockPos::new(0, 0, 0), &mut boxes);
        assert_eq!(n, 0, "{name} 不应产出碰撞盒");
    }
}

fn collision_at(
    w: &dyn VoxelAccess,
    p: BlockPos,
    out: &mut [mcv_game::physics::Aabb; MAX_SHAPE_BOXES],
) -> usize {
    push_boxes(w.block(p), w, p, RayTarget::Collide, out)
}
