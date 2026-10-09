// Particle pass: camera-facing quads, one draw for all live particles.
// 绑定组与 terrain 的 frame_bind 同源（frame uniform + 方块图集数组），
// 另挂一张 particles 数组（原版 textures/particle 的 splash/crit/bubble），
// 由顶点携带 tex_set 选择；方块图集粒子（crack 碎屑）走 terrain 数组。

struct FrameUniforms {
    view_proj: mat4x4<f32>,
    cam_pos_time: vec4<f32>,
    sun_dir_day: vec4<f32>,
    fog_params: vec4<f32>,
};

// 复用 terrain 的 lightmap 亮度曲线（lightmap.fsh 乘序：先 get_brightness
// 曲线再乘昼夜因子），CPU 侧逐顶点预算成标量传下来。
fn fog_factor(dist: f32, fog: vec4<f32>) -> f32 {
    return exp2(-dist * fog.x);
}

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var terrain_tex: texture_2d_array<f32>;
@group(0) @binding(2) var particles_tex: texture_2d_array<f32>;
@group(0) @binding(3) var terrain_samp: sampler;

struct PVertIn {
    @location(0) pos: vec3<f32>,   // 世界坐标（CPU 已展开 billboard）
    @location(1) uv: vec2<f32>,    // tile 内 0..1
    @location(2) layer: u32,       // 图集层
    @location(3) tex_set: u32,     // 0 = 方块图集, 1 = particles 数组
    @location(4) color: vec4<f32>, // rgb tint × alpha（LifetimeAlpha）
    @location(5) light: f32,       // light_curve 预算值
};

struct PVertOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) @interpolate(flat) tex_set: u32,
    @location(3) color: vec4<f32>,
    @location(4) light: f32,
    @location(5) dist: f32,
};

@vertex
fn vs_particle(v: PVertIn) -> PVertOut {
    var out: PVertOut;
    out.clip = frame.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.layer = v.layer;
    out.tex_set = v.tex_set;
    out.color = v.color;
    out.light = v.light;
    out.dist = length(v.pos - frame.cam_pos_time.xyz);
    return out;
}

@fragment
fn fs_particle(v: PVertOut) -> @location(0) vec4<f32> {
    let tex = select(
        textureSample(particles_tex, terrain_samp, v.uv, v.layer),
        textureSample(terrain_tex, terrain_samp, v.uv, v.layer),
        v.tex_set == 0u,
    );
    // 与 fs_crack 同阈值：贴图透明像素直接丢弃（crack 碎屑矩形常带 alpha 0 边）。
    if (tex.a < 0.02) {
        discard;
    }
    let fog = fog_factor(v.dist, frame.fog_params);
    let sky_horizon = vec3<f32>(0.62, 0.76, 0.95);
    let day = frame.sun_dir_day.w;
    let fog_color = mix(vec3<f32>(0.02, 0.03, 0.08), sky_horizon, day);
    let lit = tex.rgb * v.color.rgb * v.light;
    let rgb = mix(fog_color, lit, fog);
    return vec4<f32>(rgb, tex.a * v.color.a);
}
