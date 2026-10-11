//! 第一人称手持渲染（右臂盒体 + 手持方块/物品图标）与挥臂动画数学。
//!
//! 机制对标 26.1 `client/renderer/ItemInHandRenderer.java`（只提取机制、
//! 常数与几何约定，代码全自写，仓库不含 Mojang 代码）：
//!
//! - 渲染时机：世界 pass 之后**清深度**、独立透视 pass（GameRenderer.java
//!   :726-729：hudProjection.setupPerspective 后 `clearDepthTexture` →
//!   renderItemInHand；手不受近处方块深度裁剪，但保留自身部件间遮挡）。
//! - 挥臂（swingArm :617-623 + applyItemArmAttackTransform :332-340）：
//!   `√a·π / √a·2π / a·π` 三正弦位移 + 劈砍旋转，`a` = 攻击/挖掘挥臂
//!   进度 0..1（本仓 20 Hz tick 制，game 层 `swing_progress` 喂入）。
//! - 位移基数：物品锚点 ITEM_POS = (0.56, −0.52, −0.72)（applyItemArmTransform
//!   :342-345）；方块显示缩放 0.40、绕 Y 转 45°（原版 cube.json 的
//!   firstperson_righthand display）；空手右臂 = renderPlayerArm :267-281
//!   矩阵链全抄（见 [`arm_view_matrix`]：肩 pivot 推到视空间屏外深处、
//!   只有前臂/手斜穿入画面右下角）。
//! - 手持方块 = 缩小立方体，六面取 `BLOCKS[].tiles[f]` 图集层；手持物品 =
//!   GUI 精灵图标 quad；空手 = 只有手臂（renderArmWithItem :447-451 分支）。

use glam::{Mat4, Vec3};
use std::f32::consts::PI;

use crate::Camera;
use crate::player_mesh::PX;

/// 物品锚点（视空间，applyItemArmTransform :343 常数）。
pub const ITEM_POS: Vec3 = Vec3::new(0.56, -0.52, -0.72);
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

/// 挥臂位移（26.1 swingArm :618-620，物品幅度）。
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

/// 原版空手右臂静止矩阵链（视空间），26.1 `ItemInHandRenderer.renderPlayerArm`
/// :267-281（右臂 invert=1、attack=0、inverseArmHeight=0 时的常数项）+
/// `AvatarRenderer.renderHand` :263 `rightArm.zRot = 0.1` +
/// `ModelPart.translateAndRotate`（ModelPart.java:163-168：pivot/16 平移）。
///
/// 逐段（视空间 x 右 / y 上 / z 朝观者，与原版 poseStack 同向）：
/// 1. `T(0.64000005, −0.6, −0.71999997)`——臂锚基点（:270）；
/// 2. `Ry(45°)`（:271）；
/// 3. `T(−1, 3.6, 3.5)`——把 pivot 推到视点上方 3.6、身后 3.5、左 1
///    （:277，肩关节由此出画）；
/// 4. `Rz(120°)·Rx(200°)·Ry(−135°)`（:278-280）；
/// 5. `T(5.6, 0, 0)`（:281）；
/// 6. `T(−5·PX, 2·PX, 0)·Rz(0.1)`——rightArm `PartPose.offset(−5,2,0)`
///    的 pivot/16 平移 + renderHand 的 zRot（HumanoidModel.java:101 /
///    AvatarRenderer.java:263）；
/// 7. `from_scale(NEG_ONE)`——本仓 mesh 顶点 = 原版 raw 模型坐标取负
///    （player_mesh.rs:21 `local=(−px,−py,−pz)·PX`），先把 mesh 局部翻回
///    raw 帧再走原版链。NEG_ONE 翻转三角面绕序：手 pass 两条管线
///    （player/hand_icon）均 `cull_mode: None`（gpu.rs player 管线注释
///    同源），无需改 FrontFace/cull。
///
/// 效果（本机全链复算，16:9、hud 70°）：肩端 pivot 视空间 ≈(0.50,−0.82,−0.66)
/// → NDC y≈−1.8 屏外；手端 4 角 NDC x∈[0.41,0.72]、y∈[−0.72,−0.41] 屏内
/// 右下——「肩在屏外、前臂斜穿入右下角」，与原版一致。
pub fn arm_view_matrix() -> Mat4 {
    let d = |deg: f32| deg.to_radians();
    Mat4::from_translation(Vec3::new(0.64000005, -0.6, -0.71999997))
        * Mat4::from_rotation_y(d(45.0))
        * Mat4::from_translation(Vec3::new(-1.0, 3.6, 3.5))
        * Mat4::from_rotation_z(d(120.0))
        * Mat4::from_rotation_x(d(200.0))
        * Mat4::from_rotation_y(d(-135.0))
        * Mat4::from_translation(Vec3::new(5.6, 0.0, 0.0))
        * Mat4::from_translation(Vec3::new(-5.0 * PX, 2.0 * PX, 0.0))
        * Mat4::from_rotation_z(0.1)
        * Mat4::from_scale(Vec3::NEG_ONE)
}

/// 第一人称右臂模型矩阵（player 管线第 4 部位 = 右臂盒体切片）。
///
/// 静止（swing=0）= 原版 renderPlayerArm 矩阵链（[`arm_view_matrix`]）。
/// 挥臂 = 臂随挥臂位移整体移动 + 绕视右轴劈砍（暂沿 [`swing_translate`]
/// / [`swing_tilt`] 物品幅度组合；原版臂专用幅度 (−0.3,0.4,−0.4) 与
/// rotY(70°·sin√aπ)/rotZ(−20°·sin a²π)（ItemInHandRenderer.java:267-275）
/// 在挥臂曲线刀接入）。
pub fn hand_arm_matrix(cam: &Camera, swing: f32) -> Mat4 {
    let tilt = Mat4::from_axis_angle(camera_basis(cam).0, swing_tilt(swing));
    to_world(
        cam,
        Mat4::from_translation(swing_translate(swing)) * tilt * arm_view_matrix(),
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
            aspect: 16.0 / 9.0,
            near: 0.1,
            far: 256.0,
        }
    }

    /// 原版手 pass 投影的 NDC（16:9 桌面基准）：竖直 FOV 固定 70°
    /// （GameRenderer.java:726 `hudProjection.setupPerspective(0.05,100,hudFov,…)`；
    /// Camera.java:221-223 `calculateHudFov` = 70.0F；Projection.java:69-72
    /// `setPerspective(fov·π/180, w/h, …)`）。近平面只影响 z 映射，xy 判定不用。
    fn hud_ndc(cam: &Camera, world: Vec3) -> (f32, f32) {
        let f = 1.0 / (70.0f32.to_radians() * 0.5).tan();
        let v = (cam.view() * world.extend(1.0)).truncate();
        (f / cam.aspect * v.x / -v.z, f * v.y / -v.z)
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
    fn arm_vanilla_chain_shoulder_offscreen_hand_bottom_right() {
        // 原版 renderPlayerArm（ItemInHandRenderer.java:267-281）把肩 pivot 推到
        // 视空间 (0.64,−0.6,−0.72) 上方 3.6、身后 3.5（:277）——**肩端屏外**、
        // 只有前臂/手斜穿入画面右下角。本机全链复算（16:9、hud 70°）：
        // 肩端 4 角 NDC y∈[−2.67,−2.0]（屏外）、手端 4 角 NDC
        // x∈[0.41,0.72]、y∈[−0.72,−0.41]（屏内右下象限）。
        let m = hand_arm_matrix(&cam(), 0.0);
        // 右臂盒体 8 角（player_mesh.rs:21 mesh 局部 = 原版 raw 取负 ×PX；
        // wide 款 right_arm addBox(−3,−2,−2,4,12,4)：raw y∈[−2,10] →
        // mesh y∈[−0.625,+0.125]，肩端 = mesh +y、手端 = mesh −y）。
        let corners = [
            [-0.0625f32, 0.125, -0.125],
            [-0.0625, 0.125, 0.125],
            [0.1875, 0.125, -0.125],
            [0.1875, 0.125, 0.125],
            [-0.0625, -0.625, -0.125],
            [-0.0625, -0.625, 0.125],
            [0.1875, -0.625, -0.125],
            [0.1875, -0.625, 0.125],
        ];
        for (i, c) in corners.iter().enumerate() {
            let ndc = hud_ndc(&cam(), (m * Vec3::from_array(*c).extend(1.0)).truncate());
            let shoulder = i < 4;
            if shoulder {
                assert!(
                    ndc.1 < -1.0,
                    "肩端角 {c:?} NDC {ndc:?} 应在屏外（y<−1）——肩 pivot 摆进画面即回归"
                );
            } else {
                assert!(
                    ndc.0 > 0.0 && ndc.0 < 1.0 && ndc.1 < 0.0 && ndc.1 > -1.0,
                    "手端角 {c:?} NDC {ndc:?} 应在屏内右下象限——手出画/越界即回归"
                );
            }
        }
    }

    #[test]
    fn swing_changes_arm_and_item_poses() {
        let c = cam();
        let rest = hand_arm_matrix(&c, 0.0);
        let mid = hand_arm_matrix(&c, 0.5);
        // 挥臂进度必须驱动臂姿态变化（矩阵差非零）。
        assert!(
            !rest.abs_diff_eq(mid, 1e-3),
            "swing=0.5 与静止臂矩阵相同——挥臂未接线"
        );
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
