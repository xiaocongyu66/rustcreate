//! Frustum culling: 6 planes extracted from view_proj (Gribb-Hartmann),
//! AABB plane tests.

use glam::{Mat4, Vec3, Vec4};

#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    /// 6 planes as (normal, d) with n·x + d >= 0 inside.
    planes: [Vec4; 6],
}

impl Frustum {
    pub fn from_view_proj(vp: &Mat4) -> Self {
        let r = vp.to_cols_array_2d();
        // rows
        let row = |i: usize| Vec4::new(r[0][i], r[1][i], r[2][i], r[3][i]);
        let (l, rgt, b, t, n, f) = (
            row(0) + row(3),
            row(3) - row(0),
            row(1) + row(3),
            row(3) - row(1),
            row(2) + row(3),
            row(3) - row(2),
        );
        let mut planes = [l, rgt, b, t, n, f];
        for p in &mut planes {
            let len = p.truncate().length();
            if len > 1e-6 {
                *p /= len;
            }
        }
        Self { planes }
    }

    pub fn intersects_aabb(&self, min: Vec3, max: Vec3) -> bool {
        for p in &self.planes {
            // pick the aabb corner most along the plane normal
            let px = if p.x >= 0.0 { max.x } else { min.x };
            let py = if p.y >= 0.0 { max.y } else { min.y };
            let pz = if p.z >= 0.0 { max.z } else { min.z };
            if p.x * px + p.y * py + p.z * pz + p.w < 0.0 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;

    #[test]
    fn center_aabb_visible_ahead() {
        let cam = Camera {
            pos: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 1.2,
            aspect: 1.6,
            near: 0.1,
            far: 256.0,
        };
        let f = Frustum::from_view_proj(&cam.view_proj());
        assert!(f.intersects_aabb(Vec3::new(-1.0, 0.0, -12.0), Vec3::new(1.0, 2.0, -10.0)));
        assert!(!f.intersects_aabb(Vec3::new(-1.0, 0.0, 10.0), Vec3::new(1.0, 2.0, 12.0)));
        // far side-clipped
        assert!(!f.intersects_aabb(Vec3::new(400.0, 0.0, -30.0), Vec3::new(500.0, 2.0, -29.0)));
    }
}
