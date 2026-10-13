//! #190 取证锁②：AABB 扫掠的「空气必零阻挡」不变式 + 贴面擦行的
//! epsilon/off-by-one 口径。真机症状「平飞时突然撞到看不见的墙」的
//! 扫掠假说面：纯空气腔体内高速飞行必须**永远不触发碰撞**；贴实体面
//! 平飞（合法间隙 SKIN、精确贴面、亚像素嵌入）行为必须确定且与方块
//! 面对齐——不许出现「面前什么都没有也判阻挡」。

use std::collections::HashMap;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos};
use mcv_game::physics::{Aabb, Axis, StepInput, move_box, step};
use mcv_game::{Player, VoxelAccess};

const STONE: u16 = 1;

struct Map(HashMap<(i32, i32, i32), u16>);

impl VoxelAccess for Map {
    fn block(&self, p: BlockPos) -> BlockId {
        BlockId(*self.0.get(&(p.x, p.y, p.z)).unwrap_or(&0))
    }
    fn light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, _c: mcv_core::ChunkPos) -> bool {
        true
    }
}

/// 冲刺飞 +X 输入（FLY_SPEED × 2 = 20.2 m/s 档）。
fn fly(dir: Vec3) -> StepInput {
    StepInput {
        wish_dir: dir,
        look_dir: dir,
        sprint: true,
        ..StepInput::default()
    }
}

/// 稳态冲刺飞的每步位移下限（预热后按 90% 计）。
fn step_floor() -> f32 {
    mcv_game::consts::FLY_SPEED * mcv_game::consts::SPRINT_FLY_MULTIPLIER * (1.0 / 60.0) * 0.9
}

/// 纯空气宇宙：冲刺飞行 240 步——每步位移单调推进、任何轴不得被
/// 碰撞清零（隐形墙扫掠假说的直接反证锁）。
#[test]
fn high_speed_flight_in_pure_air_never_collides() {
    let w = Map(HashMap::new());
    let mut p = Player {
        pos: Vec3::new(0.0, 70.0, 0.0),
        flying: true,
        ..Player::default()
    };
    let inp = fly(Vec3::X);
    let mut min_vx = f32::INFINITY;
    for i in 0..240 {
        let before = p.pos.x;
        step(&w, &mut p, &inp);
        if i >= 30 {
            let d = p.pos.x - before;
            assert!(
                d > step_floor(),
                "步 {i} 纯空气位移骤降 d={d}（隐形阻挡假说）vel={:?}",
                p.vel
            );
            min_vx = min_vx.min(p.vel.x);
        }
    }
    assert!(min_vx > 18.0, "速度被非碰撞事件清零？min_vx={min_vx}");
    assert!(!p.on_ground, "纯空气不得落地");
}

/// 石壳腔体（内空 [0..64)³，壁厚 1）：贴 -X 内壁 1.4 m 冲刺平飞
/// 150 步（自由程 < 51 m < 64）——扫掠候选格 floor(min)-1 的保守外扩
/// 不得在空气中误报碰撞。
#[test]
fn cavity_shell_1m4_clearance_is_collision_free() {
    let mut m = HashMap::new();
    // 壳 = [-1..=65]³ 中所有「落在 [0..64)³ 之外」的格（6 面闭合、
    // 无洞；四角冗余无害）。
    for x in -1..=65 {
        for y in -1..=65 {
            for z in -1..=65 {
                let inside = (0..64).contains(&x) && (0..64).contains(&y) && (0..64).contains(&z);
                if !inside {
                    m.insert((x, y, z), STONE);
                }
            }
        }
    }
    let w = Map(m);
    let mut p = Player {
        pos: Vec3::new(1.4, 32.0, 1.5),
        flying: true,
        ..Player::default()
    };
    let inp = fly(Vec3::Z);
    for i in 0..150 {
        let before = p.pos.z;
        step(&w, &mut p, &inp);
        if i >= 60 {
            let d = p.pos.z - before;
            assert!(
                d > step_floor(),
                "步 {i}：贴腔壁 1.4 m 冲刺出现假碰撞 d={d} vel={:?} pos={}",
                p.vel,
                p.pos
            );
        }
    }
    // 收尾自查：全程未触壳（AABB 与石壳严格不相交）。
    let b = Aabb::from_player(p.pos);
    for k in m.keys() {
        let cell = Aabb {
            min: Vec3::new(k.0 as f32, k.1 as f32, k.2 as f32),
            max: Vec3::new(k.0 as f32 + 1.0, k.1 as f32 + 1.0, k.2 as f32 + 1.0),
        };
        assert!(!b.overlaps(&cell), "终点嵌入石壳 pos={} cell={k:?}", p.pos);
    }
}

/// 贴面平飞：y 精确贴地（64.0）与 SKIN 间隙（64.0+1e-4）两态水平
/// 推进零阻挡；亚像素嵌入（64.0−1e-5）= 与脚下格严格重叠 → 原版同款
/// 「贴实心侧面受阻」，行为确定（就近钳面，不瞬移、不穿地）。
#[test]
fn grazing_flight_over_floor() {
    let mut m = HashMap::new();
    for x in -8..40 {
        for z in -8..8 {
            for y in 60..=63 {
                m.insert((x, y, z), STONE); // 地坪顶面 y=64.0
            }
        }
    }
    let w = Map(m);
    for (tag, y) in [("精确贴面", 64.0), ("SKIN 间隙", 64.0 + 1e-4)] {
        let mut p = Player {
            pos: Vec3::new(0.0, y, 0.5),
            flying: true,
            ..Player::default()
        };
        let inp = fly(Vec3::X);
        for i in 0..120 {
            step(&w, &mut p, &inp);
            assert!(
                p.vel.x > 18.0,
                "{tag}：水平被地坪误清零 i={i} vel={:?}",
                p.vel
            );
        }
        assert!(p.pos.x > 70.0, "{tag}：推进不足 pos.x={}", p.pos.x);
        assert!(
            (p.pos.y - y).abs() < 1e-3,
            "{tag}：y 漂移 pos.y={}",
            p.pos.y
        );
    }
    // 亚像素嵌入（y=64−1e-5，AABB 与脚下格重叠 1e-5）：只锁「确定且
    // 有界」——本仓 nearest_face 的 in_front 过滤（physics.rs:120-130，
    // 沙坑隔块瞬移修复）在嵌入态排除了身后的 min 面，行为 = 沿移动向
    // 在实体层内滑行的有界确定态，不再复刻旧「被甩到沙柱另一侧」的
    // 反常倒退。与原版（重叠即钳面）的口径差登记在此为**取证观察项**：
    // 隐形墙证词是「多阻挡」方向，嵌入态是「少阻挡」方向，方向相反、
    // 不是本案根因；若真机日志出现嵌入滑行样本再升级为 KNOWN-DIVERGENCE。
    let mut p = Player {
        pos: Vec3::new(0.0, 64.0 - 1e-5, 0.5),
        flying: true,
        ..Player::default()
    };
    let inp = fly(Vec3::X);
    let mut max_step_adv = 0.0f32;
    let mut prev_x = p.pos.x;
    for i in 0..120 {
        step(&w, &mut p, &inp);
        assert!(p.pos.x.is_finite(), "嵌入态 NaN i={i}");
        max_step_adv = max_step_adv.max(p.pos.x - prev_x);
        prev_x = p.pos.x;
        assert!(
            p.pos.x < 60.0 && p.pos.x > -2.0,
            "嵌入态失控（爆冲/隔块瞬移）i={i} pos={}",
            p.pos.x
        );
    }
    eprintln!(
        "K-SWEEP-EMBED 诊断：嵌入态每步最大推进 {max_step_adv:.4}（0=受阻钳面，\
         ≈0.34=实体层内滑行）pos.x={} vel.x={}",
        p.pos.x, p.vel.x
    );
}

/// 薄墙门洞：高速穿洞不失配（洞内零阻挡）；撞实心钳位面 = 墙面
/// 40 − 半宽 − SKIN（off-by-one 口径：不许钳在 39/41 面）。
#[test]
fn tunnel_hole_pass_and_wall_face_alignment() {
    let mut m = HashMap::new();
    for y in 50..70 {
        for z in -10..10 {
            // 门洞：格 y ∈ {68,69} × 格 z ∈ {-1,0}（2×2），其余石。
            let hole = (68..70).contains(&y) && (-1..1).contains(&z);
            if !hole {
                m.insert((40, y, z), STONE);
            }
        }
    }
    let w = Map(m);
    // 穿洞：脚底 68.0 → AABB y [68,69.8] ⊂ 洞 [68,70)；z=-0.2 →
    // AABB [-0.5,0.1] ⊂ 洞 [-1,1)。
    let mut p = Player {
        pos: Vec3::new(0.0, 68.0, -0.2),
        flying: true,
        ..Player::default()
    };
    let inp = fly(Vec3::X);
    for _ in 0..420 {
        step(&w, &mut p, &inp);
    }
    assert!(p.pos.x > 44.0, "门洞被隐形阻挡 pos={}", p.pos);
    assert!(p.vel.x > 18.0, "穿洞后速度应保留 vel.x={}", p.vel.x);
    // 实心行：钳位面 = 40 − 0.3 − SKIN。
    let mut p = Player {
        pos: Vec3::new(0.0, 60.0, 4.5),
        flying: true,
        ..Player::default()
    };
    for _ in 0..420 {
        step(&w, &mut p, &inp);
    }
    let expect = 40.0 - mcv_game::Player::HALF[0] - 1e-4;
    assert!(
        (p.pos.x - expect).abs() < 1e-3,
        "钳位面偏离墙面：got={} expect={expect}",
        p.pos.x
    );
}

/// move_box 子步口径：±MAX_SUBSTEP（0.5）与亚 SKIN 微位移（1e-6）
/// 在纯空气中全部零阻挡；无碰撞路不改 `pos`（pos 由步末从盒回填，
/// 与 `step` 玩家路径同构）——盒位移逐段可加。
#[test]
fn substep_edges_deterministic() {
    let air = Map(HashMap::new());
    let mut pos = Vec3::ZERO;
    let mut vel = Vec3::ZERO;
    let mut b = Aabb::from_player(pos);
    let mut want = 0.0f32;
    for d in [0.5, -0.5, 0.499_9, 1e-6, -1e-6] {
        let hit = move_box(&air, &mut pos, &mut vel, &mut b, Axis::X, d);
        assert!(!hit, "纯空气 move_box({d}) 报碰撞");
        want += d;
    }
    assert_eq!(pos, Vec3::ZERO, "无碰撞路不得改写 pos");
    assert!(
        (b.feet_center().x - want).abs() < 1e-4,
        "盒位移不守恒 got={} want={}",
        b.feet_center().x,
        want
    );
}
