// Item drop pass: upright spinning billboards, one draw for all visible drops.
// 绑定组复用世界 pass 的 frame_buf（FrameUniforms 与 terrain/particle 同源，
// 字段布局逐字节对齐）；材料物品采 GUI 精灵表（texture_2d），方块物品采
// 方块图集数组（texture_2d_array，顶点携带层号），tex_set 二选一。
// cutout 语义（alpha discard），写深度（实体不透明相，26.1 ItemEntityRenderer
// 的 cutout 批语义；billboard 摆位/自转在 CPU 侧完成）。

struct FrameUniforms {
    view_proj: mat4x4<f32>,
    cam_pos_time: vec4<f32>,
    sun_dir_day: vec4<f32>,
    fog_params: vec4<f32>,
    tint_grass: vec4<f32>,
    tint_foliage: vec4<f32>,
    fog_color: vec4<f32>,
    light_tint: vec4<f32>,
};

// 雾语义与 terrain.wgsl 完全同步（gpu.rs 同一份 fog_params）：双层线性
// max(env 层球距, renderDist 层柱距)（26.1 fog.glsl:13-24/36-40）。
fn linear_fog(dist: f32, start: f32, end: f32) -> f32 {
    return clamp((dist - start) / (end - start), 0.0, 1.0);
}

fn fog_factor(dist_sph: f32, dist_cyl: f32, fog: vec4<f32>) -> f32 {
    let coverage = max(
        linear_fog(dist_sph, fog.x, fog.y),
        linear_fog(dist_cyl, fog.z, fog.w),
    );
    return 1.0 - coverage;
}

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var gui_tex: texture_2d<f32>;
@group(0) @binding(2) var terrain_tex: texture_2d_array<f32>;
@group(0) @binding(3) var drop_samp: sampler;

struct DVertIn {
    @location(0) pos: vec3<f32>,   // 世界坐标（CPU 已展开 billboard + 自转）
    @location(1) uv: vec2<f32>,    // 精灵表空间 / tile 内 0..1
    @location(2) layer: u32,       // 方块图集层（GUI 路径恒 0）
    @location(3) tex_set: u32,     // 0 = GUI 精灵表, 1 = 方块图集
    @location(4) light: vec3<f32>, // lightmap RGB（CPU 侧 light_curve）
};

struct DVertOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) @interpolate(flat) tex_set: u32,
    @location(3) light: vec3<f32>,
    @location(4) dist: f32,
    @location(5) dist_cyl: f32,
};

@vertex
fn vs_item_drop(v: DVertIn) -> DVertOut {
    var out: DVertOut;
    out.clip = frame.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.layer = v.layer;
    out.tex_set = v.tex_set;
    out.light = v.light;
    let rel = v.pos - frame.cam_pos_time.xyz;
    out.dist = length(rel);
    out.dist_cyl = max(length(rel.xz), abs(rel.y));
    return out;
}

@fragment
fn fs_item_drop(v: DVertOut) -> @location(0) vec4<f32> {
    let tex = select(
        textureSample(gui_tex, drop_samp, v.uv),
        textureSample(terrain_tex, drop_samp, v.uv, v.layer),
        v.tex_set == 1u,
    );
    // 与 fs_hand_icon 同阈值：物品 sprite 的透明像素直接丢弃（cutout）。
    if (tex.a < 0.1) {
        discard;
    }
    let fog = fog_factor(v.dist, v.dist_cyl, frame.fog_params);
    let rgb = mix(frame.fog_color.rgb, tex.rgb * v.light, fog);
    return vec4<f32>(rgb, tex.a);
}
