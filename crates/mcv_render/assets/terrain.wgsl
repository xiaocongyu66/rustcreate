// Chunk terrain: opaque + water passes. One draw per chunk; chunk origin
// comes from a dynamic-offset uniform.

diagnostic(off, derivative_uniformity);

struct FrameUniforms {
    view_proj: mat4x4<f32>,
    cam_pos_time: vec4<f32>,
    sun_dir_day: vec4<f32>,
    fog_params: vec4<f32>,
};

struct ChunkOrigin {
    origin: vec4<f32>, // xyz = chunk world origin, w = free
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var<uniform> chunk: ChunkOrigin;
@group(0) @binding(2) var terrain_tex: texture_2d_array<f32>;
@group(0) @binding(3) var terrain_samp: sampler;

struct VtxIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<u32>,
    @location(2) layer: u32,
    @location(3) lights: vec2<u32>,  // x = block light, y = sky light
    @location(4) aoflags: vec2<u32>, // x = ao, y = flags (bit0-2 face, bit3 wave)
};

struct VtxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) shade: f32,
    @location(3) dist: f32,
};

@vertex
fn vs_terrain(v: VtxIn) -> VtxOut {
    var out: VtxOut;
    let world = chunk.origin.xyz + v.pos;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = vec2<f32>(v.uv) / 65535.0;
    out.layer = v.layer;
    let face = v.aoflags.y & 0x7u;
    let ao = f32(v.aoflags.x) / 3.0;
    let sky = f32(v.lights.y);
    let blk = f32(v.lights.x);
    let day = frame.sun_dir_day.w;
    out.shade = face_shade(face) * mix(0.45, 1.0, ao)
        * light_curve(sky, blk, day);
    let rel = world - frame.cam_pos_time.xyz;
    out.dist = length(rel);
    return out;
}

@fragment
fn fs_terrain(v: VtxOut) -> @location(0) vec4<f32> {
    let tex = textureSample(terrain_tex, terrain_samp, v.uv, v.layer);
    if (tex.a < 0.5) {
        discard;
    }
    let fog = fog_factor(v.dist, frame.fog_params);
    let sky_horizon = vec3<f32>(0.62, 0.76, 0.95);
    let lit = tex.rgb * v.shade;
    let day = frame.sun_dir_day.w;
    let fog_color = mix(vec3<f32>(0.02, 0.03, 0.08), sky_horizon, day);
    return vec4<f32>(mix(fog_color, lit, fog), 1.0);
}

// ---- water ----------------------------------------------------------------

struct WaterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) shade: f32,
    @location(3) dist: f32,
};

@vertex
fn vs_water(v: VtxIn) -> WaterOut {
    var out: WaterOut;
    var world = chunk.origin.xyz + v.pos;
    let is_surface = (v.aoflags.y & 0x8u) != 0u;
    if (is_surface) {
        let t = frame.cam_pos_time.w;
        world.y += sin(t * 2.2 + world.x * 0.9 + world.z * 1.1) * 0.05;
    }
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = vec2<f32>(v.uv) / 65535.0;
    out.layer = v.layer;
    let face = v.aoflags.y & 0x7u;
    let sky = f32(v.lights.y);
    let blk = f32(v.lights.x);
    out.shade = face_shade(face) * light_curve(sky, blk, frame.sun_dir_day.w);
    let rel = world - frame.cam_pos_time.xyz;
    out.dist = length(rel);
    return out;
}

@fragment
fn fs_water(v: WaterOut) -> @location(0) vec4<f32> {
    let tex = textureSample(terrain_tex, terrain_samp, v.uv, v.layer);
    let fog = fog_factor(v.dist, frame.fog_params);
    let sky_horizon = vec3<f32>(0.62, 0.76, 0.95);
    let day = frame.sun_dir_day.w;
    let fog_color = mix(vec3<f32>(0.02, 0.03, 0.08), sky_horizon, day);
    let lit = tex.rgb * v.shade;
    let color = mix(fog_color, lit, fog);
    return vec4<f32>(color, 0.72 * fog + 0.28);
}
