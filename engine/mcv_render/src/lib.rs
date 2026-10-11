//! wgpu 30 renderer: terrain/water/sky/HUD passes，原版贴图数组（827 张
//! 官方方块贴图 + missing 哨兵 + 裂纹），生物群系染色（mcv_core::tint），
//! offscreen capture for CI verification。素材红线：不产程序化假贴图。

pub mod camera;
pub mod celestial;
pub mod cloud;
pub mod env_color;
pub mod font;
pub mod frustum;
pub mod gpu;
pub mod gui;
pub mod hand;
pub mod mob_mesh;
pub mod offscreen;
pub mod particle_renderer;
pub mod particles;
pub mod player_mesh;
pub mod text;
pub mod unifont;

pub use camera::{Camera, view_bob_matrix};
pub use cloud::{CloudSettings, Clouds};
pub use gpu::{
    BatchingStats, FrameStats, FrameUniforms, HudQuad, MobInstance, PlayerUniforms, RenderChunk,
    Renderer, Scene, WaterPart,
};
pub use hand::{HandItem, HandRender};
pub use mob_mesh::{
    MAX_MOB_PARTS, MOB_KIND_COUNT, MOB_MAX_INSTANCES, MOB_PART_COUNTS, MOB_TEX_FILES,
    MOB_TEX_LAYERS, MobModelKind, MobPose, WALK_GAIN, build_mob_mesh, load_mob_payload,
    mob_model_matrices, update_wing_animation,
};
pub use offscreen::OffscreenTarget;
pub use particle_renderer::ParticleRenderer;
pub use particles::ParticleEngine;
pub use player_mesh::{
    PlayerPose, PlayerVertex, SKIN_LAYERS, model_matrices, update_walk_animation,
};

use glam::Vec3;

// terrain.wgsl 的 tint LUT 定长（TINT_LAYERS）与图集层数互锁：uniform 地址
// 空间不允许 runtime-sized 数组，层数变更必须同步 shader 常量。
const _: () = assert!(mcv_core::atlas::LAYERS == 838);

pub const EYE_HEIGHT: f32 = 1.62;

/// GUI 整数缩放比(MC 规则:h/240 向下取整,最小 2;见任务规定)。
pub fn gui_scale(h: f32) -> f32 {
    (h / 240.0).floor().max(2.0)
}

/// Sun direction + day factor from day time in ticks (24000 per day).
///
/// 相位对齐 26.1 `data/minecraft/timeline/day.json`：`visual/sun_angle` 在
/// tick 6000 = 0°（正午天顶；SkyRenderer.java:274 取度数转弧度、:323-325
/// 绕 X 旋转该角，0° 即天顶），tick 0 = 黎明（wakeup 标记 0）、12000 = 日落、
/// 18000 = 子夜。旧实现 `angle = frac*TAU` 把天顶放在 tick 0，比原版早 1/4 天。
///
/// **D2 修正（bezier 缓动）**：day.json:337-343 为 sun_angle 轨挂
/// `cubic_bezier(0.362, 0.241, 0.638, 0.759)` 缓动（Timelines.java:39
/// symmetricCubicBezier(0.362, 0.241)、:53 关键帧 (6000,360)/(6000,0)——
/// 同 tick 双帧，实际语义 = 跨周期 6000→30000 由 0° 缓动至 360°）。即
/// `angle = 360° × bezier_y(fract((tick−6000)/24000))`。线性近似在晨昏
/// 偏差最大（tick 0/12000 太阳仰角 0° vs 原版 12.4°——bezier y(0.25)=
/// y(0.75)=0.2156；开局数分钟太阳贴地/被地形挡的 D2 根因）。数值为
/// day.json 硬编码常数（版权红线：json 不入仓，先例 moon_phase）。
pub fn sun_state(time_ticks: u64) -> (Vec3, f32) {
    let tick = (time_ticks % 24_000) as f32;
    let frac = ((tick - 6_000.0) / 24_000.0).rem_euclid(1.0);
    let angle = sun_angle_bezier_y(frac) * std::f32::consts::TAU;
    // z 分量 −0.25·sin 为既有美术倾斜（登记于 KNOWN-DIVERGENCE，非本次范围）。
    let dir = Vec3::new(-angle.sin(), angle.cos(), -0.25 * angle.sin()).normalize();
    (dir, day_factor(time_ticks))
}

/// day.json:338-343 cubic_bezier 控制点 (x1, y1, x2, y2)（Timelines.java:39
/// symmetricCubicBezier(0.362, 0.241) 的展开）。CSS cubic-bezier 同式：
/// x(u)/y(u) 三次多项式，给定 x 解 u（Newton 迭代，边界夹紧）。
const SUN_ANGLE_BEZIER: [f32; 4] = [0.362, 0.241, 0.638, 0.759];

fn sun_angle_bezier_y(x: f32) -> f32 {
    let (x1, y1, x2, y2) = (
        SUN_ANGLE_BEZIER[0],
        SUN_ANGLE_BEZIER[1],
        SUN_ANGLE_BEZIER[2],
        SUN_ANGLE_BEZIER[3],
    );
    let x = x.clamp(0.0, 1.0);
    let bx =
        |u: f32| 3.0 * (1.0 - u) * (1.0 - u) * u * x1 + 3.0 * (1.0 - u) * u * u * x2 + u * u * u;
    let by =
        |u: f32| 3.0 * (1.0 - u) * (1.0 - u) * u * y1 + 3.0 * (1.0 - u) * u * u * y2 + u * u * u;
    // x(u) 对 u 的导数（Newton 解 bx(u)=x，必须用 x 的导数）。
    let bx_dt = |u: f32| {
        3.0 * (1.0 - u) * (1.0 - 3.0 * u) * x1 + 3.0 * u * (2.0 - 3.0 * u) * x2 + 3.0 * u * u
    };
    let mut u = x;
    for _ in 0..8 {
        let err = bx(u) - x;
        if err.abs() < 1e-6 {
            break;
        }
        let d = bx_dt(u);
        if d.abs() < 1e-6 {
            break;
        }
        u = (u - err / d).clamp(0.0, 1.0);
    }
    // Newton 不收敛（导数平坦）时二分兜底：bezier x(u) 单调，必收敛。
    if (bx(u) - x).abs() > 1e-4 {
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..48 {
            let m = 0.5 * (lo + hi);
            if bx(m) < x {
                lo = m;
            } else {
                hi = m;
            }
        }
        u = 0.5 * (lo + hi);
    }
    by(u)
}

/// 昼夜光照系数（渲染逐帧乘到天光上的 `day_factor`）。
///
/// 按 26.1 `day.json` `visual/sky_light_factor` 线性关键帧（原版曲线为
/// keyframe 线性插值，非 cos 形）：
/// - `[730, 11270]` 白天平台 1.0；
/// - `11270 → 13140` 线性过渡到 0.24；
/// - `[13140, 22860]` 夜间平台 0.24（旧实现夜底 0.03，夜面比原版暗约 3 倍）；
/// - `22860 → 次日 730` 线性回到 1.0（跨 0 回绕段与傍晚段等长 1870 tick）。
pub fn day_factor(time_ticks: u64) -> f32 {
    let t = (time_ticks % 24_000) as f32;
    const DAWN: f32 = 730.0;
    const DUSK: f32 = 11_270.0;
    const NIGHT: f32 = 13_140.0;
    const NIGHT_END: f32 = 22_860.0;
    const DARK: f32 = 0.24;
    const SPAN: f32 = 1_870.0;
    if t < DAWN {
        // 22860 → 24730（=730+24000）黎明线性段。
        DARK + (t + 24_000.0 - NIGHT_END) / SPAN * (1.0 - DARK)
    } else if t <= DUSK {
        1.0
    } else if t <= NIGHT {
        1.0 - (t - DUSK) / SPAN * (1.0 - DARK)
    } else if t <= NIGHT_END {
        DARK
    } else {
        DARK + (t - NIGHT_END) / SPAN * (1.0 - DARK)
    }
}
