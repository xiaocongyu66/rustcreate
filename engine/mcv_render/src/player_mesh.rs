//! 玩家模型网格：按 64x64 皮肤 UV 布局程序化生成六部位盒体（steve / alex 两套
//! 常数表），外加帽/外套/衣袖/裤腿第二层（overlay）盒体，共 12 个盒体 ×2 款式。
//!
//! 常数照 MC 26.1 反编译提取（来源见 mc-ref/NOTES-model.md）：
//! - `model/HumanoidModel.java createMesh`：head texOffs(0,0) 8x8x8 pivot(0,0,0)；
//!   body (16,16) 8x12x4 pivot(0,0,0)；right_arm (40,16) 4x12x4 pivot(-5,2,0)；
//!   left_arm (40,16) mirror；right_leg (0,16) 4x12x4 pivot(-1.9,12,0)；
//!   hat (32,0) 扩张 0.5；jacket (16,32)、right_sleeve (40,32)、right_pants
//!   (0,32) 扩张 0.25（HumanoidModel.OVERLAY_SCALE）。
//! - `model/player/PlayerModel.java createMesh`：steve(wide)/alex(slim) 手臂宽
//!   4 / 3；left_arm 覆盖为 texOffs(32,48)（不 mirror）、left_sleeve (48,48)、
//!   left_pants (0,48) —— 64x64 皮肤第二行左侧布局（镜像已烘焙进像素）。
//!
//! 皮肤展开（MC 皮肤规范 / 26.1 像素核对）：区域原点 (u,v)、宽 w 高 h 深 d，
//! 侧面条带位于 `v+d` 行、自左至右 = **right(d) front(w) left(d) back(w)**；
//! 顶面 (u+d, v, w×d)、底面 (u+d+w, v, w×d)；每面像素按"正视该面所见"绘制。
//!
//! 坐标换算：MC 模型空间 y 向下、正面朝 +z、头顶根 y=0 脚底 y=24；本引擎
//! 模型正面朝 -z（与 `Camera::dir` yaw=0 视线一致，第三人称后视看到背面）、
//! y 向上、脚底原点。变换 = 绕 y 转 180°（x→-x, z→-z）+ y 翻转：
//! `local = (-px, -py, -pz) × PX`（box 坐标以 pivot 为原点，pivot 平移在矩阵），
//! pivot 世界偏移 = `(-p.x, 24-p.y, -p.z) × PX`（再乘身体 yaw 旋转）。
//! 右手定则：角色右手 = 本空间 +x（steve 右臂 pivot -5·PX→+5·PX? 转 180°
//! 后右臂落在 +x，与角色右=+x 一致）。

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// 皮肤纹理边长（像素，64x64 布局）。
pub const SKIN_PX: usize = 64;
/// 像素 → 格。
pub const PX: f32 = 1.0 / 16.0;
/// 皮肤 array 层数（0=steve/wide，1=alex/slim）。
pub const SKIN_LAYERS: u32 = 2;
/// 盒体数（6 基础 + 6 第二层）。
pub const PART_COUNT: usize = 12;

// 部位索引（模型矩阵数组下标；steve/alex 两款式共用同一矩阵表）。
pub const P_HEAD: usize = 0;
#[allow(dead_code)]
pub const P_HAT: usize = 1;
#[allow(dead_code)]
pub const P_BODY: usize = 2;
#[allow(dead_code)]
pub const P_JACKET: usize = 3;
pub const P_R_ARM: usize = 4;
#[allow(dead_code)]
pub const P_R_SLEEVE: usize = 5;
#[allow(dead_code)]
pub const P_L_ARM: usize = 6;
#[allow(dead_code)]
pub const P_L_SLEEVE: usize = 7;
pub const P_R_LEG: usize = 8;
#[allow(dead_code)]
pub const P_R_PANTS: usize = 9;
pub const P_L_LEG: usize = 10;
#[allow(dead_code)]
pub const P_L_PANTS: usize = 11;

/// 每部位 24 顶点 / 36 索引（6 面 × 4/6）。
pub const PART_VERTS: usize = 24;
pub const PART_INDEXES: usize = 36;
/// 玩家管线顶点步长（pos f32x3 + uv unorm8x2 + 2 pad + meta u32x2）。
pub const PLAYER_STRIDE: usize = 24;

/// 玩家管线顶点。`meta.x` = 皮肤层号（0=steve 1=alex；皮肤为 texture_2d_array，
/// 两款式共用同一网格与矩阵）；`meta.y` = 部位索引 0..12。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct PlayerVertex {
    pub pos: [f32; 3],
    /// Unorm8x2 → 0..1。64px 皮肤：像素 p → p·255/64，精度足够。
    pub uv: [u8; 2],
    /// 显式 2B pad：消除 repr(C) 隐式 padding，bytemuck Pod 才合法（stride 24）。
    pub _pad: [u8; 2],
    pub meta: [u32; 2],
}

/// 盒体一个面。角索引位组合 bit0=bx bit1=by bit2=bz（MC 盒体 8 角，
/// 换算到本空间见模块头），`corners` 顺序从盒外看逆时针。
/// `su/tv ∈ {0,1}`：该角的 UV 在矩形内的水平/垂直位置（s 沿 u0→u1、
/// t 沿 v0→v1，皮肤 v 向下）。
#[derive(Clone, Copy)]
struct Face {
    corners: [usize; 4],
    uv: [f32; 4],        // u0 v0 u1 v1（像素）
    st: [(f32, f32); 4], // 与 corners 一一对应
}

/// 盒体定义（像素，MC Box/pivot 约定）。
#[derive(Clone, Copy)]
struct PartDef {
    box_min: [f32; 3],
    size: [f32; 3],
    faces: [Face; 6],
}

/// 面 0..5 = [+x(角色右), -x(左), 顶, 底, front(-z), back(+z)]。
fn b(x: u32, y: u32, z: u32) -> usize {
    (x | (y << 1) | (z << 2)) as usize
}

/// 经典展开 → 6 面。条带行 y=v+d：right(u..u+d) front(..+w) left(..+d) back(..+w)；
/// 顶 (u+d,v)+(w,d)、底 (u+d+w,v)+(w,d)。
fn classic_faces(u: f32, v: f32, w: f32, h: f32, d: f32) -> [Face; 6] {
    let vs = v + d;
    let ve = vs + h;
    [
        // +x 面（皮肤条带第 1 段 "right"）：正视右侧时角色正面在图像右
        // → u+ ↔ 本空间 -z ↔ bz=1；v+ ↔ 下 ↔ by=1
        Face {
            corners: [b(0, 0, 0), b(0, 1, 0), b(0, 1, 1), b(0, 0, 1)],
            uv: [u, vs, u + d, ve],
            st: [
                (0.0, 0.0), // bz0→u0? u+↔bz1 → s=bz
                (0.0, 1.0),
                (1.0, 1.0),
                (1.0, 0.0),
            ],
        },
        // -x 面（"left"）：u+ ↔ 本空间 +z ↔ bz=0；v+ ↔ by=1
        Face {
            corners: [b(1, 0, 1), b(1, 1, 1), b(1, 1, 0), b(1, 0, 0)],
            uv: [u + d + w, vs, u + d + w + d, ve],
            st: [
                (0.0, 0.0), // s = bz==0
                (0.0, 1.0),
                (1.0, 1.0),
                (1.0, 0.0),
            ],
        },
        // 顶：俯视、图像"下缘=角色正面"：u+ ↔ 本空间 -x ↔ bx=1；v+ ↔ -z ↔ bz=1
        Face {
            corners: [b(0, 0, 0), b(0, 0, 1), b(1, 0, 1), b(1, 0, 0)],
            uv: [u + d, v, u + d + w, v + d],
            st: [
                (0.0, 0.0), // s=bx? u+↔bx=1 → s = bx
                (0.0, 1.0), // t = bz
                (1.0, 1.0),
                (1.0, 0.0),
            ],
        },
        // 底：仰视、下缘=正面：u+ ↔ 本空间 +x ↔ bx=0；v+ ↔ bz=1
        Face {
            corners: [b(1, 1, 0), b(1, 1, 1), b(0, 1, 1), b(0, 1, 0)],
            uv: [u + d + w, v, u + d + 2.0 * w, v + d],
            st: [
                (0.0, 0.0), // s = !bx
                (0.0, 1.0),
                (1.0, 1.0),
                (1.0, 0.0),
            ],
        },
        // front(-z)：u+ ↔ 本空间 -x ↔ bx=1；v+ ↔ by=1
        Face {
            corners: [b(0, 0, 1), b(0, 1, 1), b(1, 1, 1), b(1, 0, 1)],
            uv: [u + d, vs, u + d + w, ve],
            st: [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)], // s = bx
        },
        // back(+z)：u+ ↔ 本空间 +x ↔ bx=0；v+ ↔ by=1
        Face {
            corners: [b(1, 0, 0), b(1, 1, 0), b(0, 1, 0), b(0, 0, 0)],
            uv: [u + 2.0 * d + w, vs, u + 2.0 * d + 2.0 * w, ve],
            st: [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)], // s = !bx
        },
    ]
}

/// overlay（第二层）盒体：每面向外扩张 e 像素。
const fn grow(min: [f32; 3], size: [f32; 3], e: f32) -> ([f32; 3], [f32; 3]) {
    (
        [min[0] - e, min[1] - e, min[2] - e],
        [size[0] + 2.0 * e, size[1] + 2.0 * e, size[2] + 2.0 * e],
    )
}

/// 12 盒体定义表。`slim` = alex（手臂宽 3）。UV 原点/pivot/尺寸全部来自
/// HumanoidModel/PlayerModel createMesh 常数（见 NOTES-model.md 表格）。
fn part_defs(slim: bool) -> [PartDef; PART_COUNT] {
    let e = 0.25; // HumanoidModel.OVERLAY_SCALE（衣袖/裤/外套）
    let eh = 0.5; // hat 扩张（createMesh: hat extend 0.5）
    let aw: f32 = if slim { 3.0 } else { 4.0 };
    // slim: right_arm addBox(-2,-2,-2,3,12,4)；wide: (-3,-2,-2,4,12,4)
    let ra_min = if slim {
        [-2.0, -2.0, -2.0]
    } else {
        [-3.0, -2.0, -2.0]
    };
    let la_min = [-1.0, -2.0, -2.0]; // slim 与 wide 左臂起点同为 -1

    let (rs_min, rs_size) = grow(ra_min, [aw, 12.0, 4.0], e);
    let (ls_min, ls_size) = grow(la_min, [aw, 12.0, 4.0], e);
    let (hat_min, hat_size) = grow([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], eh);
    let (j_min, j_size) = grow([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], e);
    let (p_min, p_size) = grow([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], e);

    // uv_w/h/d：UV 展开区域尺寸。MC CubeDeformation 只撑大顶点、UV 区域
    // 仍按原始尺寸 —— overlay 必须传基础层尺寸。
    let mk = |min: [f32; 3], size: [f32; 3], uv_whd: [f32; 3], tu: f32, tv: f32| -> PartDef {
        PartDef {
            box_min: min,
            size,
            faces: classic_faces(tu, tv, uv_whd[0], uv_whd[1], uv_whd[2]),
        }
    };
    [
        // head：box(-4,-8,-4,8,8,8) pivot(0,0,0) tex(0,0)
        mk(
            [-4.0, -8.0, -4.0],
            [8.0, 8.0, 8.0],
            [8.0, 8.0, 8.0],
            0.0,
            0.0,
        ),
        // hat：扩张 0.5，tex(32,0)，UV 区域仍是 8x8x8
        mk(hat_min, hat_size, [8.0, 8.0, 8.0], 32.0, 0.0),
        // body：box(-4,0,-2,8,12,4) tex(16,16)
        mk(
            [-4.0, 0.0, -2.0],
            [8.0, 12.0, 4.0],
            [8.0, 12.0, 4.0],
            16.0,
            16.0,
        ),
        // jacket：tex(16,32)
        mk(j_min, j_size, [8.0, 12.0, 4.0], 16.0, 32.0),
        // right_arm：pivot(-5,2,0) tex(40,16)
        mk(ra_min, [aw, 12.0, 4.0], [aw, 12.0, 4.0], 40.0, 16.0),
        // right_sleeve：tex(40,32)
        mk(rs_min, rs_size, [aw, 12.0, 4.0], 40.0, 32.0),
        // left_arm：pivot(5,2,0) tex(32,48)（PlayerModel 64x64 布局）
        mk(la_min, [aw, 12.0, 4.0], [aw, 12.0, 4.0], 32.0, 48.0),
        // left_sleeve：tex(48,48)
        mk(ls_min, ls_size, [aw, 12.0, 4.0], 48.0, 48.0),
        // right_leg：pivot(-1.9,12,0) box(-2,0,-2,4,12,4) tex(0,16)
        mk(
            [-2.0, 0.0, -2.0],
            [4.0, 12.0, 4.0],
            [4.0, 12.0, 4.0],
            0.0,
            16.0,
        ),
        // right_pants：tex(0,32)
        mk(p_min, p_size, [4.0, 12.0, 4.0], 0.0, 32.0),
        // left_leg：pivot(1.9,12,0) tex(16,48)
        mk(
            [-2.0, 0.0, -2.0],
            [4.0, 12.0, 4.0],
            [4.0, 12.0, 4.0],
            16.0,
            48.0,
        ),
        // left_pants：tex(0,48)
        mk(p_min, p_size, [4.0, 12.0, 4.0], 0.0, 48.0),
    ]
}

/// 合并网格（steve + alex 顶点共用一个缓冲，`meta.x` 分层）。
pub struct PlayerMesh {
    pub verts: Vec<PlayerVertex>,
    pub indices: Vec<u32>,
    /// `slices[款式][部位]` = 索引区间（逐部位 draw_indexed）。
    pub slices: [[std::ops::Range<u32>; PART_COUNT]; SKIN_LAYERS as usize],
}

pub fn build_player_mesh() -> PlayerMesh {
    let uv_scale = 255.0 / SKIN_PX as f32;
    let mut m = PlayerMesh {
        verts: Vec::with_capacity(SKIN_LAYERS as usize * PART_COUNT * PART_VERTS),
        indices: Vec::with_capacity(SKIN_LAYERS as usize * PART_COUNT * PART_INDEXES),
        // Range<u32> 非 Copy，不能用 [[r; N]; M] 重复表达式初始化
        slices: std::array::from_fn(|_| std::array::from_fn(|_| 0..0)),
    };
    for (s, slim) in [false, true].iter().enumerate() {
        let defs = part_defs(*slim);
        for (p, def) in defs.iter().enumerate() {
            let base = m.verts.len() as u32;
            for face in def.faces.iter() {
                for k in 0..4 {
                    let ci = face.corners[k];
                    let (su, tv) = face.st[k];
                    // MC 角像素坐标 → 本空间局部（180° 绕 y + y 翻转）
                    let bx = ci & 1 != 0;
                    let by = ci & 2 != 0;
                    let bz = ci & 4 != 0;
                    let px = def.box_min[0] + if bx { def.size[0] } else { 0.0 };
                    let py = def.box_min[1] + if by { def.size[1] } else { 0.0 };
                    let pz = def.box_min[2] + if bz { def.size[2] } else { 0.0 };
                    let uu = face.uv[0] + (face.uv[2] - face.uv[0]) * su;
                    let vv = face.uv[1] + (face.uv[3] - face.uv[1]) * tv;
                    m.verts.push(PlayerVertex {
                        // box 坐标以 pivot 为原点；pivot 的世界偏移由
                        // model_matrices 的平移给出，这里只留 -box（180°绕y+y翻转）
                        pos: [-px * PX, -py * PX, -pz * PX],
                        uv: [(uu * uv_scale).round() as u8, (vv * uv_scale).round() as u8],
                        _pad: [0; 2],
                        meta: [s as u32, p as u32],
                    });
                }
            }
            let ib = m.indices.len() as u32;
            for f in 0..6u32 {
                let vb = base + f * 4;
                m.indices
                    .extend_from_slice(&[vb, vb + 1, vb + 2, vb, vb + 2, vb + 3]);
            }
            m.slices[s][p] = ib..m.indices.len() as u32;
        }
    }
    m
}

/// 每帧姿态（由游戏侧计算后传入渲染）。
#[derive(Clone, Copy, Debug)]
pub struct PlayerPose {
    /// 脚底世界坐标。
    pub pos: Vec3,
    /// 身体 yaw（弧度，与 `Camera::dir` 同约定）。
    pub yaw: f32,
    /// 头俯仰（弧度，正 = 抬头，同 player.pitch）。
    pub pitch: f32,
    /// 摆动相位（弧度）。
    pub phase: f32,
    /// 腿摆幅值（弧度；臂幅 = 0.71 倍）。
    pub amount: f32,
}

/// 各部位 pivot（像素，MC 约定，与 part_defs 一致）。
const PIVOTS: [[f32; 3]; PART_COUNT] = [
    [0.0, 0.0, 0.0],   // head
    [0.0, 0.0, 0.0],   // hat
    [0.0, 0.0, 0.0],   // body
    [0.0, 0.0, 0.0],   // jacket
    [-5.0, 2.0, 0.0],  // right arm
    [-5.0, 2.0, 0.0],  // right sleeve
    [5.0, 2.0, 0.0],   // left arm
    [5.0, 2.0, 0.0],   // left sleeve
    [-1.9, 12.0, 0.0], // right leg
    [-1.9, 12.0, 0.0], // right pants
    [1.9, 12.0, 0.0],  // left leg
    [1.9, 12.0, 0.0],  // left pants
];

/// 计算 12 部位模型矩阵（局部 → 世界）。
///
/// 摆动参照 `HumanoidModel.setupAnim`（L223-228）：腿 xRot = cos(pos·0.6662)
/// ·1.4·k 与反相；臂 = cos(…)·1.0·k（与同侧腿反相）。本空间绕局部 +x 正角
/// = 肢体前摆（y 向上推导）；头 pitch 正 = 抬头 = 绕局部 +x 正角（与
/// `Camera::dir` 的 pitch 同型）。
pub fn model_matrices(pose: &PlayerPose) -> [Mat4; PART_COUNT] {
    let s = pose.phase.cos() * pose.amount;
    let a = s * 0.71; // 臂幅/腿幅 = 1.0/1.4
    let rots = [
        pose.pitch, // head
        pose.pitch, // hat 随头
        0.0, 0.0, -a, // right arm：与右腿反相
        -a, a, // left arm：与右腿同相
        a, s, // right leg
        s, -s, // left leg
        -s,
    ];
    let ry = Mat4::from_axis_angle(Vec3::Y, pose.yaw);
    let rx_axis = ry.transform_vector3(Vec3::X);
    let mut out = [Mat4::IDENTITY; PART_COUNT];
    for p in 0..PART_COUNT {
        // pivot 相对脚底的本空间偏移（MC y↓ 根在头顶，脚底 y=24）
        let off = Vec3::new(
            -PIVOTS[p][0] * PX,
            (24.0 - PIVOTS[p][1]) * PX,
            -PIVOTS[p][2] * PX,
        );
        let world_piv = pose.pos + ry.transform_point3(off);
        let rx = Mat4::from_axis_angle(rx_axis, rots[p]);
        out[p] = Mat4::from_translation(world_piv) * ry * rx;
    }
    out
}

/// 行走摆动推进（每帧调用）。
///
/// 参照 `HumanoidModel.setupAnim`：腿摆 = cos(phase)·幅值，幅值与移动速度
/// 成正比（walkAnimation.speed/speedValue 的近似）。取值：满速走路 ≈ ±45°
/// （0.785 rad）、上限 ±50°；相位每刻 +ratio·1.2 rad（源自 walk pos·0.6662
/// 约定，约 1.9 步/秒）；静止时幅值指数衰减归零。返回 (新相位, 新幅值)。
pub fn update_walk_animation(phase: f32, amount: f32, horiz_speed: f32, dt: f32) -> (f32, f32) {
    const WALK_SPEED: f32 = 4.317; // 玩家基础移速 m/s（0.1 属性 × 43.17，见 mcv_entity/defs.rs）
    let ratio = (horiz_speed / WALK_SPEED).min(1.0);
    let target = ratio * 0.88;
    let mut new_amount = if horiz_speed > 0.15 {
        amount + (target - amount) * (dt * 10.0).min(1.0)
    } else {
        amount * (-dt * 12.0).exp()
    };
    if horiz_speed <= 0.15 && new_amount < 0.005 {
        new_amount = 0.0;
    }
    let new_phase = (phase + ratio * 1.2 * (dt * 20.0)) % std::f32::consts::TAU;
    (new_phase, new_amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_sizes_and_uv_in_bounds() {
        let m = build_player_mesh();
        assert_eq!(
            m.verts.len(),
            SKIN_LAYERS as usize * PART_COUNT * PART_VERTS
        );
        assert_eq!(
            m.indices.len(),
            SKIN_LAYERS as usize * PART_COUNT * PART_INDEXES
        );
        for v in &m.verts {
            assert!(v.meta[0] < SKIN_LAYERS);
            assert!(v.meta[1] < PART_COUNT as u32);
            for c in v.uv {
                let px = c as f32 / 255.0 * SKIN_PX as f32;
                assert!(px >= -0.5 && px <= SKIN_PX as f32 + 0.5, "uv px {px}");
            }
        }
        let max_i = *m.indices.iter().max().unwrap() as usize;
        assert_eq!(max_i, m.verts.len() - 1);
        for s in 0..SKIN_LAYERS as usize {
            for p in 0..PART_COUNT {
                assert_eq!(m.slices[s][p].len(), PART_INDEXES);
            }
        }
    }

    #[test]
    fn part_heights_match_mc_layout() {
        // 静止姿态下各部位的世界 y 范围（对照 MC 26.1 像素表：腿 12px 在下
        // 0..0.75、身 12px 中 0.75..1.5、头 8px 上 1.5..2.0、臂自颈部下垂
        // 0.75..1.5，总高 2.0）
        let m = build_player_mesh();
        let pose = PlayerPose {
            pos: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            phase: 0.0,
            amount: 0.0,
        };
        let mm = model_matrices(&pose);
        let yspan = |part: usize| {
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for v in m
                .verts
                .iter()
                .filter(|v| v.meta[0] == 0 && v.meta[1] == part as u32)
            {
                let w = mm[part] * Vec3::from(v.pos).extend(1.0);
                lo = lo.min(w.y);
                hi = hi.max(w.y);
            }
            (lo, hi)
        };
        let (hl, hh) = yspan(P_HEAD);
        let (bl, bh) = yspan(P_BODY);
        let (ll, lh) = yspan(P_R_LEG);
        let (al, ah) = yspan(P_R_ARM);
        assert!(
            (hl - 1.5).abs() < 1e-5 && (hh - 2.0).abs() < 1e-5,
            "head {hl}..{hh}"
        );
        assert!(
            (bl - 0.75).abs() < 1e-5 && (bh - 1.5).abs() < 1e-5,
            "body {bl}..{bh}"
        );
        assert!(
            ll.abs() < 1e-5 && (lh - 0.75).abs() < 1e-5,
            "leg {ll}..{lh}"
        );
        assert!(
            (al - 0.75).abs() < 1e-5 && (ah - 1.5).abs() < 1e-5,
            "arm {al}..{ah}"
        );
    }

    #[test]
    fn head_front_uses_face_region() {
        // 正脸（front 面）应落在皮肤 (8,8)-(16,16) 的 steve 脸区
        let m = build_player_mesh();
        let defs = part_defs(false);
        let face = &defs[P_HEAD].faces[4]; // front
        assert_eq!(face.uv, [8.0, 8.0, 16.0, 16.0]);
        // 网格顶点按 (款式, 部位, 面 0..5, 角 0..3) 顺序生成：steve 头
        // = 前 24 顶点，front 是第 4 面 → [16..20]（不能用 z<0 过滤，
        // 顶/底面也有 z<0 的角，它们的 UV 在顶/底区域）
        for v in &m.verts[16..20] {
            assert_eq!((v.meta[0], v.meta[1]), (0, P_HEAD as u32));
            assert!(v.pos[2] < 0.0, "front corner must sit at -z");
            let u = v.uv[0] as f32 / 255.0 * SKIN_PX as f32;
            let w = v.uv[1] as f32 / 255.0 * SKIN_PX as f32;
            assert!(
                u > 7.9 && u < 16.1 && w > 7.9 && w < 16.1,
                "front uv {u},{w}"
            );
        }
    }

    #[test]
    fn right_arm_on_character_right() {
        // 角色面向 -z（yaw=0），右手侧 = +x：右臂 pivot 应在 +x
        let m = build_player_mesh();
        let pose = PlayerPose {
            pos: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            phase: 0.0,
            amount: 0.0,
        };
        let mm = model_matrices(&pose);
        let cx = |part: usize| {
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for v in m
                .verts
                .iter()
                .filter(|v| v.meta[0] == 0 && v.meta[1] == part as u32)
            {
                let w = mm[part] * Vec3::from(v.pos).extend(1.0);
                lo = lo.min(w.x);
                hi = hi.max(w.x);
            }
            (lo, hi)
        };
        let (ral, rah) = cx(P_R_ARM);
        let (lal, lah) = cx(P_L_ARM);
        assert!(ral > 0.0, "right arm on +x: {ral}..{rah}");
        assert!(lah < 0.0, "left arm on -x: {lal}..{lah}");
    }

    #[test]
    fn walk_animation_builds_and_rests() {
        let (mut ph, mut am) = (0.0f32, 0.0f32);
        for _ in 0..30 {
            (ph, am) = update_walk_animation(ph, am, 4.317, 1.0 / 60.0);
        }
        assert!(am > 0.7 && am <= 0.88, "walk amplitude {am}");
        assert!(ph > 0.1, "phase advances {ph}");
        for _ in 0..60 {
            (ph, am) = update_walk_animation(ph, am, 0.0, 1.0 / 60.0);
        }
        assert_eq!(am, 0.0);
    }

    #[test]
    fn arm_swing_opposes_leg() {
        let pose = PlayerPose {
            pos: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            phase: 0.0,
            amount: 0.5,
        };
        let mm = model_matrices(&pose);
        // 肢体末端（pivot 正下方 0.5m）：负 z = 前摆
        let tip = |p: usize| mm[p].transform_point3(Vec3::new(0.0, -0.5, 0.0)).z;
        assert!(tip(P_R_LEG) < 0.0, "right leg forward {}", tip(P_R_LEG));
        assert!(tip(P_R_ARM) > 0.0, "right arm back {}", tip(P_R_ARM));
    }
}
