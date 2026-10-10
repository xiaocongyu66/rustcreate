//! 第一人称手持渲染（右臂盒体 + 手持方块/物品图标）与挥臂动画数学。
//!
//! 机制对标 26.1 `client/renderer/ItemInHandRenderer.java`（只提取机制、
//! 常数与几何约定，代码全自写，仓库不含 Mojang 代码）：
//!
//! - 渲染时机：世界 pass 之后**清深度**、独立透视 pass（GameRenderer.java
//!   :724-729：`clearDepthTexture` 后 renderItemInHand；手不受近处方块深度
//!   裁剪，但保留自身部件间遮挡）。
//! - 摆动（swingArm :660-668 + applyItemArmAttackTransform :349-357）：
//!   `√a·π / √a·2π / a·π` 三正弦位移 + 绕右轴劈砍旋转，`a` = 攻击/挖掘挥臂
//!   进度 0..1（本仓 20 Hz tick 制，game 层 `swing_progress` 喂入）。
//! - 位移基数：物品锚点 ITEM_POS = (0.56, −0.52, −0.72)（applyItemArmTransform
//!   :358-363）；方块显示缩放 0.40、绕 Y 转 45°（原版 cube.json 的
//!   firstperson_righthand display）；空手右臂锚点取 renderPlayerArm :308-311
//!   的 (0.64, −0.6, −0.72) 一系（本仓用自定盒体摆位，观感对标原版）。
//! - 手持方块 = 缩小立方体，六面取 `BLOCKS[].tiles[f]` 图集层；手持物品 =
//!   GUI 精灵图标 quad；空手 = 只有手臂（renderArmWithItem :553-558 分支）。

use glam::{Mat4, Vec3};
use std::f32::consts::PI;

use crate::Camera;
use crate::player_mesh::PX;

/// 物品锚点（视空间，applyItemArmTransform 常数）。
pub const ITEM_POS: Vec3 = Vec3::new(0.56, -0.52, -0.72);
/// 空手右臂腕点（视空间；观感对标 renderPlayerArm 基数系）。
const ARM_REST: Vec3 = Vec3::new(0.50, -0.42, -0.62);
/// 手持方块显示缩放（原版 cube display scale 0.40）。
pub(crate) const BLOCK_SCALE: f32 = 0.40;
/// 手持物品图标 quad 边长（视空间）。
const ICON_SIZE: f32 = 0.45;

/// 第一人称手持物语义（game 层 `hand_item` 产出）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HandItem {
    /// 空手：只有手臂。
    Empty,
    /// 手持方块（`BLOCKS` 注册 id）：缩小立方体。
    Block(u16),
    /// 手持非方块物品：GUI 精灵图标（素材缺失时渲染端降级只画手臂）。
    Sprite(&'static str),
}

/// Scene.hand 快照：手持物 + 挥臂进度。
#[derive(Clone, Copy, Debug)]
pub struct HandRender {
    pub item: HandItem,
    /// 挥臂进度 0..=1（0 = 静止；挖掘长按周期 ≈3 tick）。
    pub swing: f32,
}

/// 相机正交基（视空间 → 世界）：返回 (右, 上, 后)。
/// `Camera::view` 用 look_at_rh(eye, eye + dir, Y)：右 = f × Y 的负？——
/// look_at_rh 的 s = normalize(cross(f, up))，f=(0,0,−1) 时 cross((0,0,−1),
/// (0,1,0)) = (0·0−(−1)·1, (−1)·0−0·0, 0·1−0·0) = (1,0,0) ✓；上 = s × f
/// = (1,0,0)×(0,0,−1) = (0·(−1)−0·0, 0·0−1·(−1), 0) = (0,1,0) ✓。
pub fn camera_basis(cam: &Camera) -> (Vec3, Vec3, Vec3) {
    let f = cam.dir().normalize_or_zero();
    let right = Vec3::new(-f.z, 0.0, f.x).normalize_or_zero();
    let up = right.cross(f).normalize_or_zero();
    (right, up, -f)
}

/// 挥臂位移（26.1 swingArm :661-665）。
pub fn swing_translate(a: f32) -> Vec3 {
    let s = a.sqrt();
    Vec3::new(
        -0.4 * (s * PI).sin(),
        0.2 * (s * 2.0 * PI).sin(),
        -0.2 * (a * PI).sin(),
    )
}

/// 挥臂劈砍角（弧度，对标 applyItemArmAttackTransform 的 rotX(80°·xz)，
/// xz = sin(√a·π)；绕视空间右轴正角 = 肢体前摆/下劈，物品/方块/臂共用）。
pub fn swing_tilt(a: f32) -> f32 {
    let xz = (a.sqrt() * PI).sin();
    80.0f32.to_radians() * xz
}

/// 视空间仿射 → 世界矩阵：`world = T(eye) · B · local`，B 列 = (右, 上, 后)。
fn to_world(cam: &Camera, local: Mat4) -> Mat4 {
    let (r, u, b) = camera_basis(cam);
    let basis = Mat4::from_cols(
        r.extend(0.0),
        u.extend(0.0),
        b.extend(0.0),
        Vec3::ZERO.extend(1.0),
    );
    let eye = cam.pos + Vec3::new(0.0, crate::EYE_HEIGHT, 0.0);
    Mat4::from_translation(eye) * basis * local
}

/// 第一人称右臂模型矩阵（player 管线第 4 部位 = 右臂盒体切片）。
///
/// 盒体相对 pivot 向下延伸 0.75 格（player_mesh 右臂：y ∈ −0.625..+0.125）。
/// 静止摆位 = 腕点在视空间 (0.50, −0.42, −0.62)（右下偏中），盒体向
/// 右下屏外伸（肘部出画）；挥臂 = 锚点随 swing_translate 位移 + 绕右轴
/// 劈砍（观感对标原版挖掘节奏）。
pub fn hand_arm_matrix(cam: &Camera, swing: f32) -> Mat4 {
    // 静止盒体旋转：先绕 Z 抬腕向右上（θ=+0.477 把盒体 −Y 端旋向 +x），
    // 再绕 X 前倾（φ=−0.176 让肘部略微朝向观者）——把 (0,−1,0) 映到
    // (0.458,−0.863,0.152)，即腕下盒体伸向右下屏外。
    let rest = Mat4::from_rotation_x(-0.176) * Mat4::from_rotation_z(0.477);
    let tilt = Mat4::from_axis_angle(camera_basis(cam).0, swing_tilt(swing));
    to_world(
        cam,
        Mat4::from_translation(ARM_REST + swing_translate(swing)) * tilt * rest,
    )
}

/// 手持方块中心（视空间）：物品锚点 + 原版 cube 显示抬升（3/16）+ 挥臂位移。
pub fn hand_block_center(swing: f32) -> Vec3 {
    ITEM_POS + Vec3::new(0.0, 3.0 / 16.0, 0.0) + swing_translate(swing)
}

/// 手持方块的 6 面旋转（原版 cube display：绕 Y 45° + 挥臂劈砍）。
/// 返回把"单位立方体局部角 (−0.5..0.5)"映到视空间的旋转。
pub fn hand_block_rotation(cam: &Camera, swing: f32) -> Mat4 {
    let tilt = Mat4::from_axis_angle(camera_basis(cam).0, swing_tilt(swing));
    tilt * Mat4::from_rotation_y(PI * 0.25)
}

/// 手持物品图标 quad：视空间中心 + 旋转与边长。
/// 返回 (角点视空间坐标 ×4, 对应 uv ×4)，角序与 player_mesh 面展开一致
/// （左上/右上/右下/左下）。
///
/// 静止姿态 = 面向观者的平面（26.1 普通物品走 WHACK 挥臂接线，rest 时
/// `applyItemArmAttackTransform` 的 rotY(45°)/rotY(-45°) 书挡相消、净旋转
/// 恒等；builtin/generated 的 sprite 经 firstperson_righthand 旋转后观感
/// 即"举卡面朝自己"），只保留原版 display 旋转 [0,-90,25] 里的 25° 面内
/// 滚转（绕视轴旋转不破坏平面性）。挥臂时叠加绕右轴劈砍。
pub fn hand_icon_quad(cam: &Camera, swing: f32) -> ([Vec3; 4], [[f32; 2]; 4]) {
    let tilt = Mat4::from_axis_angle(camera_basis(cam).0, swing_tilt(swing));
    let rot = tilt * Mat4::from_rotation_z(25.0f32.to_radians());
    let c = ITEM_POS + swing_translate(swing);
    let s = ICON_SIZE * 0.5;
    let corners = [
        Vec3::new(-s, s, 0.0),
        Vec3::new(s, s, 0.0),
        Vec3::new(s, -s, 0.0),
        Vec3::new(-s, -s, 0.0),
    ];
    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let pts = corners.map(|p| (rot * p.extend(1.0)).truncate() + c);
    (pts, uv)
}

/// 盒体像素尺寸换算 sanity（player_mesh 同款换算，arm 盒体高 12px = 0.75）。
pub const ARM_BOX_HEIGHT: f32 = 12.0 * PX;

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera {
            pos: Vec3::new(8.0, 70.0, 8.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 1.2,
            aspect: 1.6,
            near: 0.1,
            far: 256.0,
        }
    }

    #[test]
    fn camera_basis_matches_look_at() {
        // yaw=0 → 视线 −z：右 = +x、上 = +y、后 = +z。
        let (r, u, b) = camera_basis(&cam());
        assert!((r - Vec3::X).length() < 1e-5);
        assert!((u - Vec3::Y).length() < 1e-5);
        assert!((b - Vec3::Z).length() < 1e-5);
        // yaw=π/2 → 视线 +x：右应指 +z。
        let mut c = cam();
        c.yaw = std::f32::consts::FRAC_PI_2;
        let (r, _, _) = camera_basis(&c);
        assert!((r - Vec3::Z).length() < 1e-5, "right {r}");
    }

    #[test]
    fn arm_rest_wrist_is_bottom_right_front() {
        // 静止腕点视空间锚点：右半屏 + 屏幕下半 + 相机前方。
        assert!(ARM_REST.x > 0.0 && ARM_REST.y < 0.0 && ARM_REST.z < 0.0);
        let m = hand_arm_matrix(&cam(), 0.0);
        // 模型空间原点（盒体 pivot）应落在腕锚世界点。
        let eye = Vec3::new(8.0, 70.0 + crate::EYE_HEIGHT, 8.0);
        let want = eye + Vec3::new(0.50, -0.42, -0.62);
        let got = (m * Vec3::ZERO.extend(1.0)).truncate();
        assert!(got.distance(want) < 1e-4, "pivot {got} vs {want}");
        // 盒体末端（局部 (0,−0.75,0)）应向右下屏外延伸：世界 x 更大、y 更小。
        let tip = (m * Vec3::new(0.0, -0.75, 0.0).extend(1.0)).truncate();
        assert!(tip.x > got.x && tip.y < got.y, "tip {tip} vs pivot {got}");
    }

    #[test]
    fn swing_changes_arm_and_item_poses() {
        let c = cam();
        let rest = hand_arm_matrix(&c, 0.0);
        let mid = hand_arm_matrix(&c, 0.5);
        // 劈砍中臂末端 z（前向分量）明显前移：mid 的 tip 前于 rest 的 tip。
        let tip = |m: Mat4| (m * Vec3::new(0.0, -0.75, 0.0).extend(1.0)).truncate().z;
        assert!(tip(mid) < tip(rest) - 0.05, "swing must chop forward");
        // 挥臂位移在挥动中段让物品中心前移。
        assert!(hand_block_center(0.5).z < hand_block_center(0.0).z);
        // 进度 0 与 1 归位（1 的正弦回零）。
        assert!(hand_block_center(1.0).z - hand_block_center(0.0).z < 1e-4);
    }

    #[test]
    fn icon_quad_faces_viewer_and_is_sized() {
        let c = cam();
        let (pts, _uv) = hand_icon_quad(&c, 0.0);
        // 四角构成 ~ICON_SIZE 见方的 quad，中心在物品锚点。
        let center = (pts[0] + pts[1] + pts[2] + pts[3]) * 0.25;
        assert!((center - ITEM_POS).length() < 1e-4);
        assert!((pts[0].distance(pts[1]) - ICON_SIZE).abs() < 1e-4);
        // 静止姿态下四角 z 相同（近似面向观者的平面）且在相机前方。
        assert!(
            pts.iter()
                .all(|p| (p.z - pts[0].z).abs() < 1e-4 && p.z < 0.0)
        );
    }
}
