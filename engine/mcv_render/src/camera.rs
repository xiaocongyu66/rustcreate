//! Camera math (wgpu clip space: z in [0, 1]).

use glam::{Mat4, Vec3};
use std::f32::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
    /// 行走 bob 矩阵（视空间，[`view_bob_matrix`] 产出；静止 = IDENTITY）。
    /// 原版把它乘进世界投影矩阵（GameRenderer.java:684-691
    /// `projectionMatrix.mul(bobStack)`）并同乘手 pass 的 poseStack
    /// （:360-362）——本仓统一放进 view_proj 的 view 与 proj 之间
    /// （bob 是视空间刚体变换，P·bob·V ≡ 对视空间先 bob 再投影）。
    pub bob: Mat4,
    /// 曝光系数（线性乘子，默认 1.0 = 行为不变；#223 续 HDR 面）。
    /// 仅 HDR tone mapping 路径读取（`hdr` feature 默认关 → 默认构建
    /// 下零读取零影响）；bevy 的 `2^color_grading.exposure` stops 语义
    /// 为有意偏差（stops 语义下默认 1.0 会 ×2 提亮，违背「默认不变」）。
    pub exposure: f32,
}

impl Camera {
    pub fn dir(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            -self.pitch.cos() * self.yaw.cos(),
        )
        .normalize_or_zero()
    }

    pub fn view(&self) -> Mat4 {
        let eye = self.pos + Vec3::new(0.0, crate::EYE_HEIGHT, 0.0);
        Mat4::look_at_rh(eye, eye + self.dir(), Vec3::Y)
    }

    /// Perspective projection mapping z to [0, 1] (wgpu convention).
    pub fn proj(&self) -> Mat4 {
        let f = 1.0 / (self.fov_y * 0.5).tan();
        Mat4::from_cols_array(&[
            f / self.aspect,
            0.0,
            0.0,
            0.0,
            0.0,
            f,
            0.0,
            0.0,
            0.0,
            0.0,
            self.far / (self.near - self.far),
            -1.0,
            0.0,
            0.0,
            self.far * self.near / (self.near - self.far),
            0.0,
        ])
    }

    /// 视投影（含行走 bob）：`proj · bob · view`（原版 GameRenderer.java
    /// :684-691 把 bobStack 乘进投影矩阵；手 pass 经同一入口吃到同一 bob，
    /// :360-362）。静止（bob=IDENTITY）时与旧 view_proj 完全同值。
    pub fn view_proj(&self) -> Mat4 {
        self.proj() * self.bob * self.view()
    }

    /// 手 pass 独立透视投影：竖直 FOV 固定 70°、near=0.05、far=100、
    /// aspect 与世界同（GameRenderer.java:726
    /// `hudProjection.setupPerspective(0.05F, 100.0F, hudFov, width, height)`；
    /// Camera.java:221-223 `calculateHudFov` = 固定 70.0F；
    /// Projection.java:69-72 `setPerspective(fov·π/180, width/height, …)`）。
    /// 与世界 fov（options.fov×速度修正）解耦：疾跑/速度 FOV 变化不缩放手部
    /// （原版 26.1 同款；本仓 fov_y 恒 1.25 rad ≈71.6°，near/far 差异仍真实）。
    pub fn hud_proj(&self) -> Mat4 {
        const HUD_FOV_DEG: f32 = 70.0;
        const HUD_NEAR: f32 = 0.05;
        const HUD_FAR: f32 = 100.0;
        let f = 1.0 / (HUD_FOV_DEG.to_radians() * 0.5).tan();
        Mat4::from_cols_array(&[
            f / self.aspect,
            0.0,
            0.0,
            0.0,
            0.0,
            f,
            0.0,
            0.0,
            0.0,
            0.0,
            HUD_FAR / (HUD_NEAR - HUD_FAR),
            -1.0,
            0.0,
            0.0,
            HUD_FAR * HUD_NEAR / (HUD_NEAR - HUD_FAR),
            0.0,
        ])
    }

    /// 手 pass 视投影（含行走 bob）：`hud_proj · bob · view`。
    pub fn view_proj_hud(&self) -> Mat4 {
        self.hud_proj() * self.bob * self.view()
    }
}

/// 视空间行走 bob 矩阵（26.1 `GameRenderer.bobView`，GameRenderer.java
/// :336-348 全文照抄）：
///
/// ```text
/// T(sin(bID·π)·bob·0.5, −|cos(bID·π)·bob|, 0)
///   · Rz(sin(bID·π)·bob·3°)
///   · Rx(|cos(bID·π − 0.2)·bob|·5°)
/// ```
///
/// - `bob`：插值后摆幅 0..=0.1（ClientAvatarState.java:76-78 的 0.4 收敛 +
///   AbstractClientPlayer.java:84-87 的 min(0.1,|Δv|) 门与 onGround 门）；
/// - `backwards_walk_dist`：bID = −(walkDist + Δ·partialTicks)
///   （ClientAvatarState.java:89-92）；
/// - 平移上限 0.1 格、rotZ ≤0.3°、rotX ≤0.5°（bob 上限 0.1）。
pub fn view_bob_matrix(bob: f32, backwards_walk_dist: f32) -> Mat4 {
    let s = (backwards_walk_dist * PI).sin();
    let c = (backwards_walk_dist * PI).cos();
    Mat4::from_translation(Vec3::new(s * bob * 0.5, -(c * bob).abs(), 0.0))
        * Mat4::from_rotation_z((s * bob * 3.0).to_radians())
        * Mat4::from_rotation_x(
            ((backwards_walk_dist * PI - 0.2).cos() * bob).abs() * 5.0f32.to_radians(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            pos: Vec3::new(0.0, 100.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 1.2,
            aspect: 1.6,
            near: 0.1,
            far: 256.0,
            bob: Mat4::IDENTITY,
            exposure: 1.0,
        }
    }

    #[test]
    fn point_ahead_maps_into_ndc() {
        let cam = camera();
        let vp = cam.view_proj();
        // 10 blocks ahead of the eye (looking toward -Z at yaw 0).
        let p = Vec3::new(0.0, 101.62, -10.0);
        let clip = vp * p.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < 0.3, "ndc.x {ndc}");
        assert!(ndc.y.abs() < 0.3, "ndc.y {ndc}");
        assert!((0.0..=1.0).contains(&ndc.z), "ndc.z {ndc}");
    }

    #[test]
    fn point_behind_is_clipped() {
        let cam = camera();
        let vp = cam.view_proj();
        let p = Vec3::new(0.0, 101.62, 10.0); // behind
        let clip = vp * p.extend(1.0);
        // Either w <= 0 or z outside [0, 1].
        assert!(clip.w <= 0.0 || !(0.0..=1.0).contains(&(clip.z / clip.w)));
    }

    #[test]
    fn yaw_rotates_view_dir() {
        let mut cam = camera();
        cam.yaw = std::f32::consts::FRAC_PI_2; // +90°
        let d = cam.dir();
        assert!((d.x - 1.0).abs() < 1e-5, "yaw 90° looks toward +X: {d}");
    }

    #[test]
    fn view_bob_matrix_matches_vanilla_and_zeroes_when_still() {
        // 26.1 GameRenderer.bobView（GameRenderer.java:336-348）：
        // T(sin(bID·π)·bob·0.5, −|cos(bID·π)·bob|, 0)·Rz(sin(bID·π)·bob·3°)
        // ·Rx(|cos(bID·π−0.2)·bob|·5°)。取 bid=−0.25、bob=0.1：
        // sin=−0.70711、cos=0.70711 → T=(−0.03536,−0.07071,0)。
        let m = view_bob_matrix(0.1, -0.25);
        let t = m.w_axis.truncate();
        assert!((t.x + 0.035355).abs() < 1e-4, "bob 位移 x {t}");
        assert!((t.y + 0.070711).abs() < 1e-4, "bob 位移 y（下沉）{t}");
        assert!(t.z.abs() < 1e-6, "bob 不做前后位移（公式 z=0）{t}");
        assert!(
            !m.abs_diff_eq(Mat4::IDENTITY, 1e-6),
            "行走中 bob 矩阵必须 ≠ 静止（用户症状「走路不晃」回归锁）"
        );
        // 静止（bob=0）= 恒等：不走路时任何 walkDist 都不产生位移。
        assert!(view_bob_matrix(0.0, -12.5).abs_diff_eq(Mat4::IDENTITY, 1e-9));
        // 全矩阵锁：bid=−0.25、bob=0.1 三因子常数
        // （θz = sin(−0.25π)·0.1·3 = −0.212132°；
        //   θx = |cos(−0.25π − 0.2)|·0.1·5 = 0.275969°——−0.2 在 **cos 括号内**
        //   （:347 `cos(f*π − 0.2F)`），曾误写 (cos−0.2)·bob 得 0.2536° 未被
        //   平移断言抓到，此锁防再犯）。
        let expected = Mat4::from_translation(Vec3::new(-0.035_355, -0.070_711, 0.0))
            * Mat4::from_rotation_z(-0.212_132f32.to_radians())
            * Mat4::from_rotation_x(0.275_969f32.to_radians());
        assert!(m.abs_diff_eq(expected, 1e-5), "bob 全矩阵对原版常数");
    }

    #[test]
    fn hud_projection_is_independent_from_world() {
        // 26.1 手 pass 独立投影（GameRenderer.java:726
        // `hudProjection.setupPerspective(0.05F, 100.0F, hudFov, width, height)`，
        // Camera.java:221-223 `calculateHudFov` = 固定 70.0F，
        // Projection.java:69-72 `setPerspective(fov·π/180, w/h, …)`）：
        // 与世界 fov/near 解耦——疾跑/速度 FOV 变化不缩放手部。
        let cam = camera();
        // fov_y=1.2 rad ≈ 68.75° ≠ 70°、near 0.1 ≠ 0.05、far 256 ≠ 100。
        let hud = cam.hud_proj();
        let world = cam.proj();
        assert!(
            !hud.abs_diff_eq(world, 1e-4),
            "hud 投影必须与世界投影解耦（fov/near/far 任一不同即不等）"
        );
        // hud fov = 70° 竖直：f = 1/tan(35°) ≈ 1.42815。
        let f = 1.0 / (70.0f32.to_radians() * 0.5).tan();
        assert!(
            (hud.x_axis.x * cam.aspect - f).abs() < 1e-5,
            "hud 竖直 fov 锁 70°（got {}）",
            hud.x_axis.x * cam.aspect
        );
        // near=0.05/far=100 的 z 项：far/(near−far) 与 far·near/(near−far)。
        assert!(
            (hud.z_axis.z - 100.0 / (0.05 - 100.0)).abs() < 1e-3
                && (hud.w_axis.z - 100.0 * 0.05 / (0.05 - 100.0)).abs() < 1e-3,
            "hud near/far 锁 0.05/100（z_col {}）",
            hud.z_axis.z
        );
        // near 解耦判别点：眼前 0.07 格——hud near=0.05 可见、世界 near=0.1
        // 裁掉（手部贴脸不被世界近平面吞，:726 语义）。
        let eye = cam.pos + Vec3::new(0.0, crate::EYE_HEIGHT, 0.0);
        let p = (eye + Vec3::new(0.0, 0.0, -0.07)).extend(1.0);
        let ch = cam.view_proj_hud() * p;
        let cw = cam.view_proj() * p;
        assert!(
            ch.w > 0.0 && (0.0..=1.0).contains(&(ch.z / ch.w)),
            "手 pass near=0.05 必须可见眼前 0.07 格点"
        );
        assert!(
            cw.w <= 0.0 || !(0.0..=1.0).contains(&(cw.z / cw.w)),
            "世界投影 near=0.1 必须裁掉同一点——两投影解耦失效"
        );
    }
}
