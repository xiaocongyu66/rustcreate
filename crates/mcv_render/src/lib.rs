//! wgpu 30 renderer: terrain/water/sky/HUD passes, procedural texture array,
//! offscreen capture for CI verification.

pub mod camera;
pub mod cloud;
pub mod font;
pub mod frustum;
pub mod gpu;
pub mod gui;
pub mod offscreen;
pub mod player_mesh;
pub mod text;
pub mod unifont;

pub use camera::Camera;
pub use cloud::{CloudSettings, Clouds};
pub use gpu::{FrameUniforms, HudQuad, PlayerUniforms, RenderChunk, Renderer, Scene};
pub use offscreen::OffscreenTarget;
pub use player_mesh::{
    model_matrices, update_walk_animation, PlayerPose, PlayerVertex, SKIN_LAYERS,
};

use glam::Vec3;

pub const EYE_HEIGHT: f32 = 1.62;

/// GUI 整数缩放比(MC 规则:h/240 向下取整,最小 2;见任务规定)。
pub fn gui_scale(h: f32) -> f32 {
    (h / 240.0).floor().max(2.0)
}

/// Sun direction + day factor from day time in ticks (24000 per day, MC
/// convention: 0 = sunrise, 6000 = noon, 12000 = sunset, 18000 = midnight).
/// TODO(research): calibrate against MC 26.1 celestial math (NOTES-2).
pub fn sun_state(time_ticks: u64) -> (Vec3, f32) {
    let frac = (time_ticks % 24_000) as f32 / 24_000.0;
    let angle = frac * std::f32::consts::TAU; // 0 = sunrise (east horizon)
    let dir = Vec3::new(-angle.sin(), angle.cos(), -0.25 * angle.sin()).normalize();
    let day = (angle.cos() * 1.6 + 0.5).clamp(0.03, 1.0);
    (dir, day)
}
