//! 玩家移动域回归锁（fix/movement-physics）：行走/冲刺/潜行稳态速度、
//! 空中控速、冲刺跳水平增补、创造飞行三速、双格墙、百格坠落不穿墙。
//!
//! 对拍基准 = 仓库外反编译 `src-26.1/`（Minecraft 26.1），断言注释附
//! file:line；换算规则见 `consts.rs` 模块注（1 块/tick = 20 m/s，
//! 逐 tick 阻 r/tick ⇔ 一阶滞后 k = −20·ln r）。

use std::collections::HashMap;

use glam::Vec3;
use mcv_game::consts;
use mcv_game::physics::{StepInput, step};
use mcv_game::{Player, VoxelAccess};

const STONE: u16 = 1;

/// 基于 HashMap 的测试世界：缺省 AIR，chunk 恒已加载。
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

    /// 铺一整层 y（x/z 取 ±24 覆盖测试范围）。
    fn fill_layer(&mut self, y: i32, id: u16) {
        for x in -24..24 {
            for z in -24..24 {
                self.set(x, y, z, id);
            }
        }
    }

    /// 立一面沿 y-z 的墙（x 列，高 h 格）。
    fn wall(&mut self, x: i32, y0: i32, h: i32, id: u16) {
        for y in y0..y0 + h {
            for z in -24..24 {
                self.set(x, y, z, id);
            }
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

/// 平地上的站立玩家。
fn grounded_player(x: f32) -> Player {
    Player {
        pos: Vec3::new(x, 1.0, 0.5),
        ..Player::default()
    }
}

/// 朝 +X 的移动意图（可选模拟量幅度）。
fn walk_x(analog: f32) -> StepInput {
    StepInput {
        wish_dir: Vec3::new(analog, 0.0, 0.0),
        look_dir: Vec3::X,
        ..StepInput::default()
    }
}

/// 表驱动：平地按表 (输入, 热身步数, 期望稳态速度) 核对水平速度收敛值。
///
/// 原版地面递推「每 tick 加输入增量再 ×0.546」（LivingEntity.java:2616+
/// :2443）等价于朝稳态速度的一阶滞后——热身后 1s 位移即稳态速度：
/// - 行走 4.317 m/s（movement_speed 0.1，Player.java:208；取保守口径，
///   源码稳态 4.405）
/// - 冲刺 ×1.3 = 5.612（SPRINTING 修饰 ADD_MULTIPLIED_TOTAL 0.3，
///   LivingEntity.java:156-157）
/// - 潜行 ×0.3 = 1.295（SNEAKING_SPEED 属性，Attributes.java:79-80）
#[test]
fn ground_steady_speed_table() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let table: [(StepInput, u32, f32, &str); 3] = [
        (walk_x(1.0), 60, consts::WALK_SPEED, "行走"),
        (
            StepInput {
                sprint: true,
                ..walk_x(1.0)
            },
            60,
            consts::SPRINT_SPEED,
            "冲刺",
        ),
        (
            StepInput {
                sneak: true,
                ..walk_x(1.0)
            },
            90,
            consts::SNEAK_SPEED,
            "潜行",
        ),
    ];
    for (input, warm, want, tag) in table {
        let mut p = grounded_player(0.5);
        for _ in 0..warm {
            step(&world, &mut p, &input);
        }
        let v = Vec3::new(p.vel.x, 0.0, p.vel.z).length();
        assert!(
            (v - want).abs() / want < 0.01,
            "{tag} 稳态速度 {v:.4} ≠ {want}"
        );
        // 热身后 1s（60 步）位移 ≈ 稳态速度 × 1s。
        let d0 = p.pos.x;
        for _ in 0..60 {
            step(&world, &mut p, &input);
        }
        let dist = p.pos.x - d0;
        assert!(
            (dist - want).abs() / want < 0.01,
            "{tag} 1s 位移 {dist:.4} ≠ {want:.3}"
        );
        // 不越冲：速度恒 ≤ 目标（原版稳态封顶语义）。
        assert!(v <= want * 1.01, "{tag} 越冲：{v} > {want}");
    }
}

/// 冷启动 1s 行走位移（含起步加速）：原版离散递推 20 tick 合计 ≈ 4.14 块
/// （稳态 0.2203 块/tick，前几 tick 被阻力压低）；连续化模型 ≈ 3.96。
/// 派单口径「1s ≈ 4.317」指热身后稳态（见 ground_steady_speed_table）。
#[test]
fn walk_first_second_from_standstill() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let mut p = grounded_player(0.5);
    for _ in 0..60 {
        step(&world, &mut p, &walk_x(1.0));
    }
    // 区间下界 = 连续模型 3.96 −4%；上界 = 原版离散 4.14 +1%。
    let dist = p.pos.x - 0.5;
    assert!(
        dist > 3.80 && dist < 4.18,
        "冷启动 1s 位移 {dist:.4} 不在 (3.80, 4.18)"
    );
}

/// 模拟量输入：0.5 幅度摇杆 → 稳态恰为一半速度（原版 getInputVector
/// 只在模长 >1 时归一化，Entity.java:1677——亚单位输入保留）。
#[test]
fn analog_half_stick_is_half_speed() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let mut p = grounded_player(0.5);
    for _ in 0..60 {
        step(&world, &mut p, &walk_x(0.5));
    }
    let v = p.vel.x;
    assert!(
        (v - consts::WALK_SPEED * 0.5).abs() / consts::WALK_SPEED < 0.02,
        "半杆速度 {v:.4} ≠ {:.4}",
        consts::WALK_SPEED * 0.5
    );
    // 键盘斜向合成（模长 √2 > 1）仍被归一化到全速。
    let mut p = grounded_player(0.5);
    let diag = StepInput {
        wish_dir: Vec3::new(1.0, 0.0, 1.0),
        ..StepInput::default()
    };
    for _ in 0..60 {
        step(&world, &mut p, &diag);
    }
    let vh = Vec3::new(p.vel.x, 0.0, p.vel.z).length();
    assert!(
        (vh - consts::WALK_SPEED).abs() / consts::WALK_SPEED < 0.01,
        "斜向速度 {vh:.4} ≠ {:.4}（不得有对角加速）",
        consts::WALK_SPEED
    );
}

/// 空中控速稳态：0.02 块/tick 输入（Player.java:1955）+ 0.91 阻力
/// （LivingEntity.java:2424-2425）→ 4.044 m/s；冲刺空中 0.026 → 5.258。
/// 空中稳态**低于**地面目标——原版空中操控本就更弱，且不从静止凭空
/// 达到地面速度。
#[test]
fn air_terminal_speed_table() {
    let world = TestWorld::new(); // 全空世界，持续下落
    let table: [(bool, f32, &str); 2] = [
        (false, consts::AIR_TERMINAL, "空中"),
        (true, consts::AIR_TERMINAL_SPRINT, "空中冲刺"),
    ];
    for (sprint, want, tag) in table {
        let mut p = Player {
            pos: Vec3::new(0.5, 300.0, 0.5),
            ..Player::default()
        };
        let input = StepInput {
            sprint,
            ..walk_x(1.0)
        };
        for _ in 0..240 {
            step(&world, &mut p, &input);
        }
        assert!(
            (p.vel.x - want).abs() / want < 0.02,
            "{tag} 稳态 {:.4} ≠ {want}",
            p.vel.x
        );
    }
}

/// 冲刺跳水平增补：起跳瞬间沿朝向 +0.2 块/tick = 4.0 m/s
/// （LivingEntity.java:2349-2351，仅 isSprinting）→ 起跳后水平速度
/// ≈ 5.612 + 4.0 = 9.6；普通跳无增补。整个跳跃弧线内该速度向空中稳态
/// 衰减（0.91/tick），落点必比平跑远——冲刺跳更快是原版核心机制。
#[test]
fn sprint_jump_horizontal_boost() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let sprint = StepInput {
        sprint: true,
        ..walk_x(1.0)
    };
    // 热身到冲刺稳态。
    let mut p = grounded_player(0.5);
    for _ in 0..60 {
        step(&world, &mut p, &sprint);
    }
    // 起跳步：本步末 vel.y = 8.4 且水平已含增补。
    let jump = StepInput {
        sprint: true,
        jump: true,
        look_dir: Vec3::X,
        wish_dir: Vec3::X,
        ..StepInput::default()
    };
    step(&world, &mut p, &jump);
    assert!((p.vel.y - consts::JUMP_SPEED).abs() < 1e-4, "未起跳");
    assert!(
        p.vel.x > 9.0 && p.vel.x < 10.2,
        "冲刺跳起跳水平速度 {} 不在 (9.0, 10.2)",
        p.vel.x
    );
    // 弧线结束（落地）时速度已向空中稳态衰减（< 9.6 增补值）。
    let vx_at_jump = p.vel.x;
    let mut landed = false;
    for _ in 0..60 {
        step(&world, &mut p, &sprint);
        if p.on_ground {
            landed = true;
            break;
        }
    }
    assert!(landed, "60 步内未落地");
    assert!(
        p.vel.x < vx_at_jump - 1.0,
        "弧线末水平速度 {} 未向空中稳态衰减",
        p.vel.x
    );
    // 普通跳无增补：起跳后水平速度仍 ≈ 4.317。
    let mut p = grounded_player(0.5);
    for _ in 0..60 {
        step(&world, &mut p, &walk_x(1.0));
    }
    step(
        &world,
        &mut p,
        &StepInput {
            jump: true,
            look_dir: Vec3::X,
            ..walk_x(1.0)
        },
    );
    assert!(
        (p.vel.x - consts::WALK_SPEED).abs() < 0.3,
        "普通跳不应有增补：{}",
        p.vel.x
    );
}

/// 创造飞行三速（Player.java:1951-1953 + LocalPlayer.java:878 +
/// Player.java:1395-1397）：
/// - 水平 10.111 m/s（flyingSpeed 0.05 块/tick × 阻力 0.91 稳态）
/// - 冲刺飞行 ×2 = 20.222（Player.java:1953 `getFlyingSpeed() * 2.0F`）
/// - 竖直 ±4.5（输入 ±0.15/tick × 0.6 回写稳态），松键快速刹停（×0.6/tick）
/// - 无重力悬停
#[test]
fn flight_speed_table() {
    let world = TestWorld::new();
    let fly_x = StepInput {
        wish_dir: Vec3::X,
        look_dir: Vec3::X,
        ..StepInput::default()
    };
    // 水平巡航。
    let mut p = Player {
        pos: Vec3::new(0.5, 100.0, 0.5),
        flying: true,
        ..Player::default()
    };
    for _ in 0..240 {
        step(&world, &mut p, &fly_x);
    }
    assert!(
        (p.vel.x - consts::FLY_SPEED).abs() / consts::FLY_SPEED < 0.015,
        "飞行巡航 {} ≠ {}",
        p.vel.x,
        consts::FLY_SPEED
    );
    assert!(p.pos.y == 100.0, "飞行不应受重力：y={}", p.pos.y);
    // 冲刺飞行 ×2。
    let mut p = Player {
        pos: Vec3::new(0.5, 100.0, 0.5),
        flying: true,
        ..Player::default()
    };
    let sprint_fly = StepInput {
        sprint: true,
        ..fly_x
    };
    for _ in 0..240 {
        step(&world, &mut p, &sprint_fly);
    }
    assert!(
        (p.vel.x - consts::FLY_SPEED * consts::SPRINT_FLY_MULTIPLIER).abs()
            / (consts::FLY_SPEED * consts::SPRINT_FLY_MULTIPLIER)
            < 0.015,
        "冲刺飞行 {} ≠ {}",
        p.vel.x,
        consts::FLY_SPEED * consts::SPRINT_FLY_MULTIPLIER
    );
    // 竖直：jump 上升 → +4.5；sneak 下降 → −4.5；松键刹停。
    let up = StepInput {
        jump: true,
        ..StepInput::default()
    };
    let mut p = Player {
        pos: Vec3::new(0.5, 100.0, 0.5),
        flying: true,
        ..Player::default()
    };
    for _ in 0..60 {
        step(&world, &mut p, &up);
    }
    assert!(
        (p.vel.y - consts::FLY_VERT_SPEED).abs() < 0.1,
        "飞行上升 {} ≠ {}",
        p.vel.y,
        consts::FLY_VERT_SPEED
    );
    for _ in 0..30 {
        step(&world, &mut p, &StepInput::default());
    }
    assert!(
        p.vel.y.abs() < 0.05,
        "松键后竖直未刹停：{}（×0.6/tick 回写）",
        p.vel.y
    );
}

/// 飞行水平漂移：松杆后按 0.91/tick 衰减（k=1.886/s），1s 后 ≈ 初速 × e^−k。
#[test]
fn flight_horizontal_coast_decay() {
    let world = TestWorld::new();
    let mut p = Player {
        pos: Vec3::new(0.5, 100.0, 0.5),
        vel: Vec3::new(consts::FLY_SPEED, 0.0, 0.0),
        flying: true,
        ..Player::default()
    };
    for _ in 0..60 {
        step(&world, &mut p, &StepInput::default());
    }
    let want = consts::FLY_SPEED * (-consts::FLY_DRAG_H).exp();
    assert!(
        (p.vel.x - want).abs() / want < 0.02,
        "漂移衰减 {} ≠ {:.4}",
        p.vel.x,
        want
    );
}

/// 撞 2 格墙停住：满冲刺撞墙，120 步内 x 永不越界（1 格墙已被跳跃跨越，
/// 2 格墙必须挡住——原版跳峰 1.25 格 < 2）。
#[test]
fn sprint_into_two_block_wall_stops() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.wall(5, 1, 2, STONE); // x=5, y∈[1,3)
    let mut p = grounded_player(0.5);
    let input = StepInput {
        sprint: true,
        jump: true, // 边跑边跳也翻不过 2 格墙
        look_dir: Vec3::X,
        ..walk_x(1.0)
    };
    for i in 0..120 {
        step(&world, &mut p, &input);
        assert!(p.pos.x < 4.7, "第 {i} 步穿/越 2 格墙：x={}", p.pos.x);
    }
}

/// 自由落体 100 步（~35 m）落地：高速下落经 0.5 m 子步扫掠不穿地
/// （MAX_SUBSTEP 上限保证；终端 ≈ 80 m/s ≫ 0.5 m/步）。
#[test]
fn hundred_block_fall_no_tunneling() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let mut p = Player {
        pos: Vec3::new(0.5, 101.0, 0.5), // 落差 100 格
        ..Player::default()
    };
    let mut tunneled = false;
    for _ in 0..200 {
        if p.pos.y < 0.0 {
            tunneled = true;
        }
        step(&world, &mut p, &StepInput::default());
    }
    assert!(!tunneled, "穿地：y={}", p.pos.y);
    assert!(
        p.on_ground && (p.pos.y - 1.0).abs() < 0.01,
        "未稳稳落地：y={}, on_ground={}",
        p.pos.y,
        p.on_ground
    );
}

// ---------------------------------------------------------------------------
// 水中（travelInWater，LivingEntity.java:2459-2486；连续换算见 consts.rs）
// ---------------------------------------------------------------------------

/// 表驱动：水中各竖直稳态。in_water 分支无 jumpFromGround——按住跳是
/// 上浮收敛（+0.04/tick，LivingEntity.java:2362-2364）而非跳跃初速。
#[test]
fn swim_vertical_table() {
    let world = TestWorld::new();
    // StepInput 非 Copy：每项独立构造（in_water 基础位一致）。
    fn swim() -> StepInput {
        StepInput {
            in_water: true,
            ..StepInput::default()
        }
    }
    let table: [(StepInput, f32, &str); 4] = [
        (swim(), -consts::SWIM_SINK_SPEED, "中性缓沉"),
        (
            StepInput {
                jump: true,
                ..swim()
            },
            consts::SWIM_UP_SPEED,
            "按跳上浮",
        ),
        (
            StepInput {
                sneak: true,
                ..swim()
            },
            -consts::SWIM_DOWN_SPEED,
            "按潜下潜",
        ),
        (
            StepInput {
                sprint: true,
                ..swim()
            },
            0.0,
            "冲刺游泳水平目视（免重力，Player.java:1395-1397）",
        ),
    ];
    for (input, want, tag) in table {
        let mut p = Player {
            pos: Vec3::new(0.5, 50.0, 0.5),
            ..Player::default()
        };
        for _ in 0..120 {
            step(&world, &mut p, &input);
        }
        assert!(
            (p.vel.y - want).abs() / want.abs().max(1.0) < 0.03,
            "{tag}：vy={:.4} ≠ {want:.3}",
            p.vel.y
        );
    }
}

/// 冲刺游泳跟随视线俯仰（Player.travel，Player.java:1383-1392）：视线
/// 下俯 45° → vy 收敛到 3.87×(−0.707) ≈ −2.74 m/s。
#[test]
fn sprint_swim_follows_look_pitch() {
    let world = TestWorld::new();
    let down = (
        std::f32::consts::FRAC_1_SQRT_2,
        -std::f32::consts::FRAC_1_SQRT_2,
    );
    let input = StepInput {
        in_water: true,
        sprint: true,
        wish_dir: Vec3::new(down.0, 0.0, down.0),
        look_dir: Vec3::new(down.0, down.1, down.0),
        ..StepInput::default()
    };
    let mut p = Player {
        pos: Vec3::new(0.5, 50.0, 0.5),
        ..Player::default()
    };
    for _ in 0..180 {
        step(&world, &mut p, &input);
    }
    let want = -consts::SWIM_LOOK_GAIN * down.1;
    assert!(
        (p.vel.y - want).abs() < 0.15,
        "俯冲 vy={:.4} ≠ {want:.3}",
        p.vel.y
    );
    // 冲刺游泳免重力（getFluidFallingAdjustedMovement 冲刺分支）。
    assert!(p.vel.y.abs() < 5.0, "不应叠加普通重力坠落");
}

/// 水面越出（jumpOutOfFluid，LivingEntity.java:2506-2511）：水面游泳撞
/// 一格高岸壁，无跳键即被逐 tick vy=6 m/s 顶到岸顶并登陆。
#[test]
fn swim_exits_onto_one_block_ledge() {
    let mut world = TestWorld::new();
    world.fill_layer(0, WATER); // 水面 y=1
    // 岸：x∈[3,9)、y=1 一格高（顶面 y=2），水中的玩家须爬上它。
    for x in 3..9 {
        for z in -3..3 {
            world.set(x, 1, z, STONE);
        }
    }
    let mut p = Player {
        pos: Vec3::new(0.5, 0.9, 0.5), // 身体没入水面
        ..Player::default()
    };
    let swim = StepInput {
        in_water: true,
        wish_dir: Vec3::X,
        look_dir: Vec3::X,
        ..StepInput::default()
    };
    let mut landed = None;
    for i in 0..240 {
        step(&world, &mut p, &swim);
        if p.on_ground && p.pos.y > 1.9 {
            landed = Some(i + 1);
            break;
        }
    }
    let at = landed.expect("240 步内应能无跳键游上一格岸");
    assert!(
        p.pos.x > 3.0 && p.pos.y > 1.99 && p.pos.y < 2.02,
        "第 {at} 步登陆：pos={}",
        p.pos
    );
}

const WATER: u16 = 5; // mcv_core blocks_gen.inc.rs:18（liquid=true）
