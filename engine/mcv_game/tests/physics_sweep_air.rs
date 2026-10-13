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
///
/// 预热窗口的物理口径：飞行水平是一阶滞后（physics.rs travel 飞行分支，
/// 收敛率 FLY_DRAG_H = AIR_CONVERGE_K = 1.886 s⁻¹ → 每步 α ≈ 0.031），
/// 从 0 到 90%（18.2 m/s）需 ln0.1/ln(1−α) ≈ 73 步 ≈ 1.2 s。电池初版在
/// i≥30 / i≥60 就要求 90% 下限、grazing 甚至要求 i=0 起 vel>18——渐近
/// 模型下恒不可满足（CI run 38392508746 红因），非引擎阻挡：观测位移
/// d ≡ v_pre·dt 逐速精确（失败读数 0.2058 = 12.35/60 正是无碰撞路）。
/// 本版把绝对下限挪到渐近充分热身后（各测 i≥120/200，均 ≥ 2×τ90），并
/// 对每一步加更强的一致性锁「位移 ≥ 步初速度×dt×98% 且速度不减」——
/// 任何假钳制（无论何时发生）都破坏其一，比旧窗口断言更严。一致性锁
/// 用 `>=`+eps 而非 `>`：步 0 v_pre=0 时引擎一阶滞后 d≡0，`0 > 0` 恒假
/// 是断言退化（run 38398420888 红因），非引擎阻挡——v_pre>0 后锁强不变。
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
        let v_pre = p.vel.x;
        step(&w, &mut p, &inp);
        let d = p.pos.x - before;
        // 每步一致性锁（窗口无关）：纯空气位移必须 ≡ 步初速度×dt，
        // 速度不得回退——任何隐形钳制都破坏其一。
        assert!(
            d >= v_pre * mcv_game::consts::FIXED_DT * 0.98 - 1e-9,
            "步 {i} 纯空气位移与速度脱节 d={d} v_pre={v_pre}（隐形阻挡假说）vel={:?}",
            p.vel
        );
        assert!(
            p.vel.x >= v_pre - 1e-6,
            "步 {i} 纯空气速度回退 vel={:?}",
            p.vel
        );
        if i >= 200 {
            assert!(
                d > step_floor(),
                "步 {i} 渐近后位移仍低于 90% 下限 d={d} vel={:?}",
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
        let v_pre = p.vel.z;
        step(&w, &mut p, &inp);
        let d = p.pos.z - before;
        assert!(
            d >= v_pre * mcv_game::consts::FIXED_DT * 0.98 - 1e-9,
            "步 {i}：贴腔壁 1.4 m 位移与速度脱节 = 假碰撞 d={d} v_pre={v_pre} vel={:?} pos={}",
            p.vel,
            p.pos
        );
        assert!(
            p.vel.z >= v_pre - 1e-6,
            "步 {i}：贴腔壁速度回退 = 假钳制 vel={:?} pos={}",
            p.vel,
            p.pos
        );
        if i >= 120 {
            assert!(
                d > step_floor(),
                "步 {i}：渐近后仍低于 90% 下限 d={d} vel={:?} pos={}",
                p.vel,
                p.pos
            );
        }
    }
    assert!(
        p.vel.z > 18.0,
        "收尾速度 <90% 渐近 = 全程有隐性阻挡 vel={:?}",
        p.vel.z
    );
    // 收尾自查：全程未触壳（AABB 与石壳严格不相交）。
    let b = Aabb::from_player(p.pos);
    for k in w.0.keys() {
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
    // 地坪长 168 格：420 步冲刺渐近程 ≈ 130 m（前 73 步爬坡耗程），
    // 与 tunnel 测同窗（其 420 步 >44 m 口径自洽）。
    for x in -8..160 {
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
        let mut v_pre = p.vel.x;
        for i in 0..420 {
            let before = p.pos.x;
            step(&w, &mut p, &inp);
            let d = p.pos.x - before;
            // 贴面每一步都不得啃掉速度（旧断言 i=0 就要 vel>18，
            // 渐近爬坡期恒假 = 电池窗口 bug，见 step_floor 注）。
            assert!(
                d >= v_pre * mcv_game::consts::FIXED_DT * 0.98 - 1e-9,
                "{tag}：贴面位移与速度脱节 = 地坪误钳 i={i} d={d} v_pre={v_pre} vel={:?}",
                p.vel
            );
            assert!(
                p.vel.x >= v_pre - 1e-6,
                "{tag}：水平被地坪误清零 i={i} vel={:?}",
                p.vel
            );
            if i >= 120 {
                assert!(p.vel.x > 18.0, "{tag}：渐近不足 i={i} vel={:?}", p.vel);
            }
            v_pre = p.vel.x;
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
