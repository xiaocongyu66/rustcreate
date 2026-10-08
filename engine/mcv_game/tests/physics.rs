//! 玩家物理测试。

use std::collections::HashMap;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkPos};
use mcv_game::physics::{step, Aabb, StepInput};
use mcv_game::{interact, raycast, Player, VoxelAccess};

const STONE: u16 = 1;
const WATER: u16 = 5;
const FLOWER_RED: u16 = 12;

/// 基于 HashMap 的测试世界：缺省 AIR，光照恒 15，chunk 恒已加载。
struct TestWorld {
    blocks: HashMap<(i32, i32, i32), u16>,
}

impl TestWorld {
    fn new() -> Self {
        Self {
            blocks: HashMap::new(),
        }
    }

    fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        self.blocks.insert((x, y, z), id);
    }

    /// 铺一整层 y（x/z 取 ±16 覆盖测试范围）。
    fn fill_layer(&mut self, y: i32, id: u16) {
        for x in -16..16 {
            for z in -16..16 {
                self.set(x, y, z, id);
            }
        }
    }
}

impl VoxelAccess for TestWorld {
    fn block(&self, p: BlockPos) -> BlockId {
        BlockId(*self.blocks.get(&(p.x, p.y, p.z)).unwrap_or(&0))
    }

    fn light(&self, _p: BlockPos) -> u8 {
        15
    }

    fn chunk_loaded(&self, _c: ChunkPos) -> bool {
        true
    }
}

fn ground_player(x: f32, y: f32, z: f32) -> Player {
    Player {
        pos: Vec3::new(x, y, z),
        ..Player::default()
    }
}

/// 1. 自由落体：y=20 起跳落到 y=0 整层 STONE 上，100 步内落地站稳。
#[test]
fn free_fall_lands() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let mut p = ground_player(0.5, 20.0, 0.5);

    for _ in 0..100 {
        step(&world, &mut p, &StepInput::default());
    }

    assert!(p.pos.y >= 1.0 && p.pos.y <= 1.001, "feet y = {}", p.pos.y);
    assert!(p.on_ground);
}

/// 2. 无地板 600 步：水中下沉终端 2 m/s，|vel.y| ≤ 4。
#[test]
fn terminal_velocity_bounded() {
    let world = TestWorld::new();
    let mut p = ground_player(0.5, 20.0, 0.5);

    for _ in 0..600 {
        step(
            &world,
            &mut p,
            &StepInput {
                in_water: true,
                ..StepInput::default()
            },
        );
    }

    assert!(p.vel.y.abs() <= 4.0, "vel.y = {}", p.vel.y);
    // 水中下沉终端 ≈ 2 m/s。
    assert!(p.vel.y >= -2.01, "vel.y = {}", p.vel.y);
}

/// 3. 平地跳：峰高（脚底抬升量）∈ (1.1, 1.35)。
#[test]
fn jump_peak() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let mut p = ground_player(0.5, 1.1, 0.5);

    // 先站稳。
    for _ in 0..10 {
        step(&world, &mut p, &StepInput::default());
    }
    assert!(p.on_ground);
    let y0 = p.pos.y;

    let mut peak = f32::MIN;
    for _ in 0..40 {
        step(
            &world,
            &mut p,
            &StepInput {
                jump: true,
                ..StepInput::default()
            },
        );
        peak = peak.max(p.pos.y);
    }

    let rise = peak - y0;
    assert!(rise > 1.1 && rise < 1.35, "rise = {}", rise);
}

/// 4. 30 m/s 水平冲 1 格厚墙：100 步零穿透。
#[test]
fn wall_no_tunneling() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    // 墙：x=3 一列（1 格厚），y=0..4。
    for y in 0..4 {
        world.set(3, y, 0, STONE);
    }
    let mut p = ground_player(0.5, 1.1, 0.5);
    p.vel.x = 30.0;

    for _ in 0..100 {
        step(&world, &mut p, &StepInput::default());
        let aabb = Aabb::from_player(p.pos);
        for y in -1..5 {
            for z in -1..2 {
                assert!(
                    !aabb.intersects_voxel(3, y, z),
                    "tunneled into wall at y={y}, z={z}, pos={}",
                    p.pos
                );
            }
        }
        assert!(p.pos.x < 3.0, "pos.x = {}", p.pos.x);
    }
}

/// 5. raycast：眼位向 -Y 命中脚下 STONE（法线 +Y）；空中向 +X 无命中；
///    水/空气穿透、花可命中。
#[test]
fn raycast_hits_and_misses() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.set(2, 20, 0, WATER);
    world.set(5, 20, 0, FLOWER_RED);

    let eye = Vec3::new(0.5, 1.0 + Player::EYE, 0.5);
    let (pos, normal) = raycast(&world, eye, Vec3::new(0.0, -1.0, 0.0), 10.0).unwrap();
    assert_eq!(pos, BlockPos::new(0, 0, 0));
    assert_eq!(normal, [0, 1, 0]);

    assert_eq!(
        raycast(
            &world,
            Vec3::new(0.5, 30.5, 0.5),
            Vec3::new(1.0, 0.0, 0.0),
            50.0
        ),
        None,
        "air ray should miss"
    );

    // 穿过水 (2,20,0)，命中花 (5,20,0)；从 +X 侧进入 → 法线 [-1,0,0]。
    let (pos, normal) = raycast(
        &world,
        Vec3::new(0.5, 20.5, 0.5),
        Vec3::new(1.0, 0.0, 0.0),
        50.0,
    )
    .unwrap();
    assert_eq!(pos, BlockPos::new(5, 20, 0));
    assert_eq!(normal, [-1, 0, 0]);
}

/// 6. can_place_block：脚下格（STONE）不可放、面前一格可放、
///    与玩家重叠或已是实体方块不可放。
#[test]
fn can_place_block_rules() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let p = ground_player(0.5, 1.0 + 1e-4, 0.5);

    // 脚下格是 STONE。
    assert!(!interact::can_place_block(
        &world,
        BlockPos::new(0, 0, 0),
        &p
    ));
    // 面前一格（+X，脚部高度）是 AIR 且不与玩家相交。
    assert!(interact::can_place_block(
        &world,
        BlockPos::new(1, 1, 0),
        &p
    ));
    // 目标已是 STONE。
    assert!(!interact::can_place_block(
        &world,
        BlockPos::new(1, 0, 0),
        &p
    ));
    // AIR 但与玩家身体重叠。
    assert!(!interact::can_place_block(
        &world,
        BlockPos::new(0, 1, 0),
        &p
    ));
}
