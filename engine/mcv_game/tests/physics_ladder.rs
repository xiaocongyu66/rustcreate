//! 梯子攀爬回归锁（任务板 #64 域 / 生存伤害与死亡闭环补全）。
//!
//! 原版机制（实查 src-26.1）：`LivingEntity.onClimbable` :1689-1713 = 身处
//! 格方块态属 CLIMBABLE；`handleOnClimbable` :2642-2654 竖直下落钳
//! −0.15 块/tick、水平钳 ±0.15、onClimbable 每拍 resetFallDistance；
//! `travelInAir` 碰撞爬升 :2620-2622 = (水平碰撞 || 跳) && onClimbable →
//! y 置 0.2 块/tick（下一拍扣重力 0.08 后实际位移 0.12 块/tick ≈ 2.4 格/s）；
//! 潜行钉停 = `isSuppressingSlidingDownLadder` :3585-3587（isShiftKeyDown，
//! LivingEntity 基类语义，Player 承接）。

use std::collections::HashMap;

use glam::Vec3;
use mcv_game::consts;
use mcv_game::physics::{StepInput, on_climbable, step};
use mcv_game::{Aabb, Player, VoxelAccess};

const STONE: u16 = 1;

/// 梯子 id 按注册名取（千块表 1171 方块，硬编码 id 脆）。
fn ladder_id() -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == "ladder")
        .unwrap() as u16
}

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

    fn fill_layer(&mut self, y: i32, id: u16) {
        for x in -8..8 {
            for z in -8..8 {
                self.set(x, y, z, id);
            }
        }
    }

    /// x=2 处立一面梯子（贴 +X 侧想象墙；碰撞满格 KNOWN-DIVERGENCE，
    /// 见 physics.rs LADDER_PROBE 注）。
    fn ladder_column(&mut self, y0: i32, h: i32) {
        let id = ladder_id();
        for y in y0..y0 + h {
            self.set(2, y, 0, id);
        }
    }
}

impl VoxelAccess for TestWorld {
    fn block(&self, p: mcv_core::BlockPos) -> mcv_core::BlockId {
        mcv_core::BlockId(*self.blocks.get(&(p.x, p.y, p.z)).unwrap_or(&0))
    }

    fn light(&self, _p: mcv_core::BlockPos) -> u8 {
        15
    }

    fn chunk_loaded(&self, _c: mcv_core::ChunkPos) -> bool {
        true
    }
}

/// 悬空贴在梯子面上（x=2 格 −X 侧面，玩家右缘贴到 2.0−SKIN）。
fn ladder_player(y: f32) -> Player {
    Player {
        pos: Vec3::new(2.0 - 0.3 - 1e-4, y, 0.5),
        ..Player::default()
    }
}

/// 贴面时的攀附谓词为真、离面 0.2 m 为假。
#[test]
fn climbable_predicate_touches_ladder() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.ladder_column(1, 8);
    let p = ladder_player(3.0);
    assert!(
        on_climbable(&world, &Aabb::from_player(p.pos)),
        "贴梯子面视为 onClimbable"
    );
    let far = Player {
        pos: Vec3::new(1.5, 3.0, 0.5),
        ..Player::default()
    };
    assert!(
        !on_climbable(&world, &Aabb::from_player(far.pos)),
        "离梯子面 0.2 m 不是 onClimbable"
    );
}

/// 梯子上松手：缓降（1 s 下落 ≤ 3 m 钳制），自由落体 1 s 应 ≈16 m。
#[test]
fn ladder_release_slides_instead_of_falling() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.ladder_column(1, 10);
    let mut p = ladder_player(8.0);
    let input = StepInput::default();
    for _ in 0..60 {
        step(&world, &mut p, &input);
    }
    let descent = 8.0 - p.pos.y;
    assert!(
        descent > 0.5 && descent < 3.0,
        "松手缓降且不坠（自由落体 1 s ≈16 m）：descent={descent:.3}"
    );
}

/// 松手时按潜行：钉停不下滑（isSuppressingSlidingDownLadder）。
#[test]
fn ladder_sneak_pins_descent() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.ladder_column(1, 8);
    let mut p = ladder_player(3.0);
    let input = StepInput {
        sneak: true,
        ..StepInput::default()
    };
    for _ in 0..60 {
        step(&world, &mut p, &input);
    }
    assert!(
        (p.pos.y - 3.0).abs() < 1e-3,
        "潜行钉停在梯子上: y={:.4}",
        p.pos.y
    );
}

/// 上移输入（朝梯子面的水平意图 → 水平碰撞）持续爬升 ≈ 2.4 格/s。
#[test]
fn ladder_up_input_climbs() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.ladder_column(1, 8);
    let mut p = ladder_player(3.0);
    let input = StepInput {
        wish_dir: Vec3::X, // 朝 x=2 的梯子格推
        look_dir: Vec3::X,
        ..StepInput::default()
    };
    for _ in 0..60 {
        step(&world, &mut p, &input);
    }
    let rise = p.pos.y - 3.0;
    assert!(
        rise > 1.5 && rise < 3.0,
        "按上移持续爬升 ≈2.4 格/s：rise={rise:.3}"
    );
}

/// 离开梯子面恢复重力：离面后 0.5 s 自由落体下坠 ≈ 4 m。
#[test]
fn leaving_ladder_restores_gravity() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.ladder_column(1, 10);
    let mut p = Player {
        pos: Vec3::new(1.5, 8.0, 0.5),
        ..Player::default()
    };
    assert!(
        !on_climbable(&world, &Aabb::from_player(p.pos)),
        "前置：离面玩家不攀附"
    );
    let input = StepInput::default();
    for _ in 0..30 {
        step(&world, &mut p, &input);
    }
    let descent = 8.0 - p.pos.y;
    assert!(
        descent > 3.0,
        "离面恢复自由落体（0.5 s ≈4 m）：descent={descent:.3}"
    );
}

/// 常数核对：爬升 0.12 块/tick、缓降钳 0.15 块/tick（原版 file:line）。
#[test]
fn ladder_consts_match_vanilla() {
    assert_eq!(consts::LADDER_CLIMB_SPEED, 0.12 * 20.0);
    assert_eq!(consts::LADDER_SLIDE_SPEED, 0.15 * 20.0);
    assert_eq!(consts::LADDER_H_CLAMP, 0.15 * 20.0);
}
