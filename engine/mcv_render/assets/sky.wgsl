// Sky: fullscreen triangle; day/night gradient, sun/moon disc, hashed stars.

struct SkyUniforms {
    inv_view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,   // w = time
    sun_dir_day: vec4<f32>, // xyz = sun dir, w = day factor
    fog_horizon: vec4<f32>, // xyz = horizon color, w = free
};

@group(0) @binding(0) var<uniform> sky: SkyUniforms;

struct VtxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> VtxOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VtxOut;
    out.clip = vec4<f32>(p[vi], 0.0, 1.0);
    out.ndc = p[vi];
    return out;
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = floor(p);
    let h = dot(q, vec3<f32>(127.1, 311.7, 74.7));
    return fract(sin(h) * 43758.5453);
}

@fragment
fn fs_sky(v: VtxOut) -> @location(0) vec4<f32> {
    // Unproject far plane point to get the view ray.
    let far_pt = sky.inv_view_proj * vec4<f32>(v.ndc, 1.0, 1.0);
    let dir = normalize(far_pt.xyz / far_pt.w - sky.cam_pos.xyz);

    let day = sky.sun_dir_day.w;
    let up = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);

    // Day sky: zenith blue -> horizon pale; night: near-black blue.
    let day_top = vec3<f32>(0.32, 0.55, 0.95);
    let day_hor = vec3<f32>(0.62, 0.76, 0.95);
    let night_top = vec3<f32>(0.008, 0.012, 0.035);
    let night_hor = vec3<f32>(0.02, 0.03, 0.08);
    let top = mix(night_top, day_top, day);
    let hor = mix(night_hor, day_hor, day);
    var color = mix(hor, top, pow(up, 0.7));

    // Sun / moon discs.
    let sun_d = dot(dir, normalize(sky.sun_dir_day.xyz));
    let moon_d = dot(dir, -normalize(sky.sun_dir_day.xyz));
    let sun = smoothstep(0.99930, 0.99965, sun_d);
    let moon = smoothstep(0.99955, 0.99985, moon_d) * (1.0 - day);
    color = mix(color, vec3<f32>(1.0, 0.95, 0.80), sun);
    color = mix(color, vec3<f32>(0.85, 0.88, 0.95), moon);

    // Stars at night: hash lattice on the ray direction.
    let star_grid = floor(dir * 220.0);
    let star = step(0.9992, hash3(star_grid)) * (1.0 - day) * step(0.05, dir.y);
    color = mix(color, vec3<f32>(0.9, 0.92, 1.0), star * 0.8);

    return vec4<f32>(color, 1.0);
}
