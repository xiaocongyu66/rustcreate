//! 四足（猪）行走摆动的渲染消费锁（纯 CPU，无 GPU）：26.1
//! `QuadrupedModel.setupAnim`（client/model/QuadrupedModel.java:66-71）的
//! leg0..3 角度公式逐腿落位——
//!
//! ```java
//! rightHindLeg  = cos(apos·0.6662)            · 1.4 · speed
//! leftHindLeg   = cos(apos·0.6662 + Math.PI)  · 1.4 · speed
//! rightFrontLeg = cos(apos·0.6662 + Math.PI)  · 1.4 · speed
//! leftFrontLeg  = cos(apos·0.6662)            · 1.4 · speed
//! ```
//!
//! 引擎 PARTS[Pig] 部位序 = [头 0, 躯干 1, 右后 2, 左后 3, 右前 4, 左前 5]
//! （MC 建模在 −x 的 "right_*" 腿经引擎 x 镜像与 +x 的左腿视觉对称，见
//! mob_mesh 模块头），对应 shift 序列 [0, π, π, 0]——对角步（右后+左前
//! 同相、左后+右前同相，两对互差 π）。消费面（AnimState.phase/amount →
//! WalkLeg 矩阵）与两足（僵尸）共用同一条矩阵通路（mob_ai_system 每
//! 60 Hz 步推进 advance_walk_anim，collect 读同一 AnimState），故本锁在
//! 引擎矩阵层固定"四腿相位序列"这一 26.1 语义面；游戏侧行走账本推进与
//! 四腿偏离零幅值基线由 game/mcv_logic/tests/mob_anim_lerp.rs 的 pig 用例锁。
//!
//! 矩阵形 = `T(枢)·RotY(−yaw)·Rz(z)·Ry(y)·Rx(x)`（mob_model_matrices）：
//! yaw=0 且部位只吃 x 角时旋转块恰为 Rx(θ)。rot_block 把列主序
//! `a[c][r] = M[r][c]` 转置回行主序后：blk[1][1]=blk[2][2]=cosθ、
//! blk[2][1]=sinθ、blk[1][2]=−sinθ（Rx(θ) 行主序 row1=[0,c,−s]、
//! row2=[0,s,c]）。

use glam::Vec3;
use mcv_render::mob_mesh::{MAX_MOB_PARTS, MobModelKind, MobPose, mob_model_matrices};

/// 猪 PARTS 序内四条腿的部位下标（[头0, 躯干1, 右后2, 左后3, 右前4, 左前5]）。
const PIG_LEGS: [usize; 4] = [2, 3, 4, 5];

/// 提取部位模型的 3x3 旋转块（列主序 → 数学行主序转置还原 M[r][c]）。
fn rot_block(m: &[[[f32; 4]; 4]; MAX_MOB_PARTS], part: usize) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for r in 0..3usize {
        for c in 0..3usize {
            out[r][c] = m[part][c][r];
        }
    }
    out
}

fn pose(pos: Vec3, phase: f32, amount: f32) -> MobPose {
    MobPose {
        pos,
        yaw: 0.0,
        phase,
        amount,
        ..Default::default()
    }
}

fn matrices(phase: f32, amount: f32) -> [[[f32; 4]; 4]; MAX_MOB_PARTS] {
    // MobInstance.models 的同一换算（glam Mat4 → 列主序 [f32;4];4）。
    let mats = mob_model_matrices(
        MobModelKind::Pig,
        &pose(Vec3::new(10.0, 64.0, -7.0), phase, amount),
    );
    let mut out = [[[0.0f32; 4]; 4]; MAX_MOB_PARTS];
    for (m, dst) in mats.iter().zip(out.iter_mut()) {
        *dst = m.to_cols_array_2d();
    }
    out
}

/// 26.1 QuadrupedModel 的对角步组成：对角对（右后+左前，左后+右前）共享
/// 同相角，两对互差 π；腿摆满幅 1.4 rad（amount 0.88 × WALK_GAIN 1.4/0.88）。
#[test]
fn quadruped_diagonal_phase_sequence_matches_java() {
    let walking = matrices(1.1, 0.88);
    let b = |p: usize| rot_block(&walking, p);

    // 对角对内（右后 2 + 左前 5；左后 3 + 右前 4）：旋转块逐位相等
    //（pivot 平移列不同、旋转块同相——Java 侧同一 cos 表达式逐腿赋值）。
    for (a, d) in [(2usize, 5usize), (3usize, 4usize)] {
        assert!(
            rot_blocks_eq(&b(a), &b(d)),
            "对角对 {a}/{d} 旋转块必须同相（QuadrupedModel.java:68-71）"
        );
    }
    // 两对互差 π：cos（块 [1][1]/[2][2]）相等、sin（[1][2]/[2][1]）反号。
    // Rx(θ) 行主序：row1=[0,c,−s]、row2=[0,s,c] → [1][2]=−sinθ、[2][1]=sinθ。
    let (hind, front) = (b(2), b(3));
    assert!(
        (hind[1][1] - front[1][1]).abs() < 1e-4,
        "cosθ 对称：cos(p)=cos(p+π)"
    );
    assert!((hind[2][2] - front[2][2]).abs() < 1e-4, "cosθ 对称");
    assert!(
        (hind[1][2] + front[1][2]).abs() < 1e-4 && (hind[2][1] + front[2][1]).abs() < 1e-4,
        "sin(p+π) = −sin(p)：两对反相（对角步的前后/左右交替）"
    );
    // 摆动实打实非零（phase=1.1 → θ≈0.635 rad → |sinθ|≈0.59）。
    assert!(
        hind[2][1].abs() > 0.3,
        "摆幅应为满速 ±1.4 rad 域内实值，得 {}",
        hind[2][1]
    );
}

/// 静止（amount→0）= 四腿回正位（Rx(0)=单位块）——Java 侧 speed=0 时
/// cos·0 → xRot=0 同款。「四肢不动」两个面的分工：本文件锁"动起来时的
/// 四腿相位序列"，游戏侧行走账本推进面由 mob_anim_lerp pig 用例锁。
#[test]
fn pig_rest_pose_is_identity_blocks() {
    let rest = matrices(0.9, 0.0);
    for p in PIG_LEGS {
        let blk = rot_block(&rest, p);
        for r in 0..3usize {
            for c in 0..3usize {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!(
                    (blk[r][c] - want).abs() < 1e-5,
                    "腿 {p} 静止旋转块应为单位阵 at [{r}][{c}]，得 {}",
                    blk[r][c]
                );
            }
        }
    }
}

/// 相位推进的极性：phase += π 翻转每条腿的 sin（前进半步 = 对侧腿前摆）、
/// cos 同值——摆动沿行进方向轴往复（与 walk_body_yaw 的朝向面正交，互不
/// 耦合：yaw 只进 RotY(−yaw) 左乘，不进部位 x 角）。
#[test]
fn phase_half_cycle_flips_swing_sign() {
    let a = matrices(1.1, 0.88);
    let b = matrices(1.1 + std::f32::consts::PI, 0.88);
    for p in PIG_LEGS {
        let (ba, bb) = (rot_block(&a, p), rot_block(&b, p));
        assert!(
            (ba[1][2] + bb[1][2]).abs() < 1e-3 && (ba[2][1] + bb[2][1]).abs() < 1e-3,
            "半周期应翻转腿 {p} 的摆动符号"
        );
        assert!(
            (ba[1][1] - bb[1][1]).abs() < 1e-3,
            "半周期 cos 不变（腿 {p}）"
        );
    }
}

fn rot_blocks_eq(x: &[[f32; 3]; 3], y: &[[f32; 3]; 3]) -> bool {
    x.iter()
        .zip(y.iter())
        .all(|(rx, ry)| rx.iter().zip(ry.iter()).all(|(a, b)| (a - b).abs() < 1e-4))
}
