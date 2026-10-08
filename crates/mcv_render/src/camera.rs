//! Camera math (wgpu clip space: z in [0, 1]).

use glam::{Mat4, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
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

    pub fn view_proj(&self) -> Mat4 {
        self.proj() * self.view()
    }
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
}
