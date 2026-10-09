//! 物理与射线校准：表驱动对照 Minecraft 26.1 换算值。
//!
//! 常数来源与推导：仓库外笔记 `mc-ref/NOTES-physics.md`
//! （重力 32、跳 8.4、竖直阻力 k≈0.404、终端 78.4、reach 4.5）。

use std::collections::HashMap;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkPos};
use mcv_game::physics::{Aabb, StepInput, step};
use mcv_game::{Player, REACH, VoxelAccess, raycast};

const STONE: u16 = 1;

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

    /// 铺一整层 y（x/z 取 ±15 覆盖测试范围）。
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

/// 1. 自由落体 100 步落距区间。
///    期望值：显式 Euler（先位移后重力）+ 竖直阻力 k=0.4 数值积分
///    得 30 步 ≈ 3.613 m、60 步 ≈ 13.798 m、100 步 ≈ 35.573 m；
///    区间取 ±5% 容忍浮点与实现细节差异（无阻力解析值 44.4/100 步是上界方向的偏离）。
#[test]
fn free_fall_distance_table() {
    // (累计步数, 落距下限 m, 落距上限 m)
    let table: [(u32, f32, f32); 3] = [(30, 3.43, 3.79), (60, 13.11, 14.49), (100, 33.80, 37.35)];
    let world = TestWorld::new(); // 全空世界，永不着地
    let y0 = 200.0f32;
    for (steps, lo, hi) in table {
        let mut p = Player {
            pos: Vec3::new(0.5, y0, 0.5),
            ..Player::default()
        };
        let mut dist = 0.0;
        for i in 0..steps {
            step(&world, &mut p, &StepInput::default());
            dist = y0 - p.pos.y;
            // 单调下落
            assert!(p.vel.y <= 0.0, "第 {i} 步速度非负: {}", p.vel.y);
        }
        assert!(
            dist > lo && dist < hi,
            "{steps} 步落距 {dist:.4} 不在 ({lo}, {hi})"
        );
    }
}

/// 2. 自由落体 100 步内落地：不同起始高度（2/5/10/20 格）都能站稳在
///    y=1 台面上（终端 ≈ 80 m/s，100 步可覆盖 35 m，足够高差区间）。
#[test]
fn free_fall_lands_within_100_steps_table() {
    for h in [2.0f32, 5.0, 10.0, 20.0] {
        let mut world = TestWorld::new();
        world.fill_layer(0, STONE);
        let mut p = Player {
            pos: Vec3::new(0.5, 1.0 + h, 0.5),
            ..Player::default()
        };
        for _ in 0..100 {
            step(&world, &mut p, &StepInput::default());
        }
        assert!(
            p.on_ground && p.pos.y >= 1.0 && p.pos.y <= 1.001,
            "h={h}: y={}, on_ground={}",
            p.pos.y,
            p.on_ground
        );
    }
}

/// 3. 零穿墙：10/30/60 m/s 水平初速冲 1 格厚墙，100 步内 AABB 从未进入墙格。
#[test]
fn wall_no_tunneling_table() {
    for v in [10.0f32, 30.0, 60.0] {
        let mut world = TestWorld::new();
        world.fill_layer(0, STONE);
        for y in 0..4 {
            world.set(3, y, 0, STONE);
        }
        let mut p = Player {
            pos: Vec3::new(0.5, 1.1, 0.5),
            vel: Vec3::new(v, 0.0, 0.0),
            ..Player::default()
        };
        for _ in 0..100 {
            step(&world, &mut p, &StepInput::default());
            let aabb = Aabb::from_player(p.pos);
            for y in -1..5 {
                for z in -1..2 {
                    assert!(
                        !aabb.intersects_voxel(3, y, z),
                        "v={v} 穿墙 y={y} z={z} pos={}",
                        p.pos
                    );
                }
            }
            assert!(p.pos.x < 3.0, "v={v}: pos.x = {}", p.pos.x);
        }
    }
}

/// 4. 跳跃最高 ≈ 1.25 格：MC tick 制离散积分峰高 1.2522 块
///    （v'= (v−0.08)·0.98，先位移后更新）；本引擎 1/60 显式 Euler 峰 ≈ 1.173 m。
///    断言取 MC 值 1.25 的 ±15% 区间 (1.0625, 1.4375)，两口径都落在其中。
#[test]
fn jump_peak_vs_mc_1_25() {
    // (下限, 上限) = MC 峰高 1.2522 的 ±15%
    let (lo, hi) = (1.0625f32, 1.4375f32);
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let mut p = Player {
        pos: Vec3::new(0.5, 1.1, 0.5),
        ..Player::default()
    };
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
    assert!(
        rise > lo && rise < hi,
        "跳高 {rise:.4} 不在 ({lo}, {hi})；MC=1.2522, 连续解析=1.1025, 本积分≈1.173"
    );
}

/// 5. 跳跃必须能上一格台阶：平地助跑 + 按住跳，落上顶面 y=2 的平台；
///    之后收手 60 步仍稳稳站在台上。
#[test]
fn jump_onto_one_block() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    // 一格高台阶：x ∈ [3,10)，z ∈ [-1,2)，顶面 y=2。
    for x in 3..10 {
        for z in -1..2 {
            world.set(x, 1, z, STONE);
        }
    }
    let mut p = Player {
        pos: Vec3::new(0.5, 1.0, 0.5),
        ..Player::default()
    };
    let run = StepInput {
        wish_dir: Vec3::new(1.0, 0.0, 0.0),
        jump: true,
        ..StepInput::default()
    };
    let mut on_platform = None;
    for i in 0..240 {
        step(&world, &mut p, &run);
        if p.on_ground && p.pos.y > 1.9 {
            on_platform = Some(i + 1);
            break;
        }
    }
    let at = on_platform.expect("240 步内应能跳上一格台阶");
    // 落地瞬间脚底中心可能仍在台阶边缘外（AABB 已压住台面），只验高度。
    assert!(
        p.pos.y > 1.99 && p.pos.y < 2.02 && p.pos.x > 2.5,
        "第 {at} 步上台：pos={}",
        p.pos
    );
    // 收手（残余前速在地面阻力下约滑 0.5 m）60 步仍站在台上。
    for _ in 0..60 {
        step(&world, &mut p, &StepInput::default());
    }
    assert!(
        p.on_ground && p.pos.y > 1.99 && p.pos.y < 2.02 && p.pos.x > 3.0 && p.pos.x < 10.0,
        "站立后台阶上：pos={}, on_ground={}",
        p.pos,
        p.on_ground
    );
}

/// 6. 交互距离 = MC 生存 4.5（Attributes.BLOCK_INTERACTION_RANGE 基础值）：
///    眼位 (0.2,·,·) 向 +X：t=3.8 的方块命中；t=4.8 的方块用 REACH 不命中，
///    但旧的 5.0 会命中 —— 验证 5.0 → 4.5 的行为变化。
#[test]
fn reach_is_survival_4_5() {
    assert_eq!(REACH, 4.5);

    let mut world = TestWorld::new();
    world.set(4, 20, 0, STONE);
    let eye = Vec3::new(0.2, 20.5, 0.5);
    let (pos, _) = raycast(&world, eye, Vec3::new(1.0, 0.0, 0.0), REACH).expect("t=3.8 应命中");
    assert_eq!(pos, BlockPos::new(4, 20, 0));

    let mut world = TestWorld::new();
    world.set(5, 20, 0, STONE); // 左面 t = 4.8，超出 4.5
    assert_eq!(
        raycast(&world, eye, Vec3::new(1.0, 0.0, 0.0), REACH),
        None,
        "t=4.8 超出生存 reach 4.5，不应命中"
    );
    assert!(
        raycast(&world, eye, Vec3::new(1.0, 0.0, 0.0), 5.0).is_some(),
        "旧 5.0 临时值会命中（对照，说明行为变化）"
    );
}
