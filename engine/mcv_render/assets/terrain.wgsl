// Chunk terrain: opaque + water passes. One draw per chunk; chunk origin
// comes from a dynamic-offset uniform.

diagnostic(off, derivative_uniformity);

struct FrameUniforms {
    view_proj: mat4x4<f32>,
    cam_pos_time: vec4<f32>,
    sun_dir_day: vec4<f32>,
    fog_params: vec4<f32>,
    // 生物群系染色基色（26.1 ColorResolver 机制）：xyz = plains 草/叶色
    // （sRGB 0..1，colormap 温度×湿度查表），tint_grass.w = 染色开关
    // （colormap 素材缺失时 0 → 不染色）。
    tint_grass: vec4<f32>,
    tint_foliage: vec4<f32>,
};

struct ChunkOrigin {
    origin: vec4<f32>, // xyz = chunk world origin, w = free
};

// Face brightness by face_id: +X,-X,+Y(top),-Y(bottom),+Z,-Z
// 原版基数：DOWN0.5 UP1.0 NORTH0.8 SOUTH0.8 WEST0.6 EAST0.6
// （world/level/CardinalLighting.java:8 DEFAULT）。X 轴面（东/西）=0.6、
// Z 轴面（南/北）=0.8——此前 X/Z 数值互换且 Z 取 0.65，已按源码修正
// （MINOR 项，任务书说法与源码一致）。
fn face_shade(face_id: u32) -> f32 {
    var table = array<f32, 6>(0.60, 0.60, 1.00, 0.50, 0.80, 0.80);
    return table[face_id];
}

// 亮度曲线（26.1 lightmap.fsh:21-42 乘序）：先 get_brightness v/(4−3v) 曲线、
// 再乘昼夜因子 SkyFactor(=day)；旧实现先乘 day 后 pow 1.5 gamma，乘序相反，
// 夜间被压得过暗。常数底 0.08+0.92·x 保留（对应原版 AmbientColor≈0.039 的
// 既有映射，本次只调乘序与曲线，不动 shader 结构）。
fn light_curve(sky: f32, block: f32, day: f32) -> f32 {
    let s = sky / 15.0;
    let b = block / 15.0;
    let sb = s / (4.0 - 3.0 * s) * day;
    let bb = b / (4.0 - 3.0 * b);
    return 0.08 + 0.92 * max(sb, bb);
}

fn fog_factor(dist: f32, fog: vec4<f32>) -> f32 {
    return exp2(-dist * fog.x);
}

// 生物群系染色（26.1 BlockColors / BlockTintSources 等价）：灰度遮罩贴图
// × 生物群系颜色。colormap 族取 FrameUniforms 的 plains 基线色（素材缺失
// 时 w=0 → 不染色）；常量族直接用 LUT 烤色。无染色时返回 1（不改变贴图）。
fn biome_tint(entry: vec4<u32>) -> vec3<f32> {
    let kind = entry.x;
    if (kind == 1u) {
        return select(vec3<f32>(1.0), frame.tint_grass.rgb, frame.tint_grass.w > 0.5);
    }
    if (kind == 2u) {
        return select(vec3<f32>(1.0), frame.tint_foliage.rgb, frame.tint_foliage.w > 0.5);
    }
    if (kind == 3u || kind == 4u) {
        return vec3<f32>(f32(entry.y), f32(entry.z), f32(entry.w)) / 255.0;
    }
    return vec3<f32>(1.0);
}

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var<uniform> chunk: ChunkOrigin;
@group(0) @binding(2) var terrain_tex: texture_2d_array<f32>;
@group(0) @binding(3) var terrain_samp: sampler;
// 生物群系染色 LUT（引擎侧 mcv_core::tint 构建）：每层
// vec4<u32>(kind, r, g, b)。kind: 0=无 1=草 colormap 2=叶 colormap
// 3=云杉常量 4=白桦常量；常量色烤在 LUT 的 yzw（0..255）。
// 定长 838（uniform 地址空间不允许 runtime-sized 数组）——
// 与 mcv_core::atlas::LAYERS 由 mcv_render/src/lib.rs 的 const 断言互锁。
const TINT_LAYERS: u32 = 838u;
@group(0) @binding(4) var<uniform> tint_lut: array<vec4<u32>, TINT_LAYERS>;

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
    // 按贴图层查 tint 类别并乘生物群系颜色（草顶/羊齿/树叶三族）。
    let tint = biome_tint(tint_lut[v.layer]);
    let fog = fog_factor(v.dist, frame.fog_params);
    let sky_horizon = vec3<f32>(0.62, 0.76, 0.95);
    let lit = tex.rgb * v.shade * tint;
    let day = frame.sun_dir_day.w;
    let fog_color = mix(vec3<f32>(0.02, 0.03, 0.08), sky_horizon, day);
    return vec4<f32>(mix(fog_color, lit, fog), 1.0);
}

// ---- mining crack overlay --------------------------------------------------
// 复用 vs_terrain（同顶点布局/同绑定组）；片元按裂纹层 alpha 混合。
// 顶点 CPU 侧外偏 0.003 防 z-fighting，管线不写深度。

@fragment
fn fs_crack(v: VtxOut) -> @location(0) vec4<f32> {
    let tex = textureSample(terrain_tex, terrain_samp, v.uv, v.layer);
    if (tex.a < 0.02) {
        discard;
    }
    let fog = fog_factor(v.dist, frame.fog_params);
    let sky_horizon = vec3<f32>(0.62, 0.76, 0.95);
    let day = frame.sun_dir_day.w;
    let fog_color = mix(vec3<f32>(0.02, 0.03, 0.08), sky_horizon, day);
    let lit = tex.rgb * v.shade;
    return vec4<f32>(mix(fog_color, lit, fog), tex.a);
}

// ---- block selection outline -----------------------------------------------

struct OutlineIn {
    @location(0) pos: vec3<f32>,
};

@vertex
fn vs_outline(v: OutlineIn) -> @builtin(position) vec4<f32> {
    let world = chunk.origin.xyz + v.pos;
    return frame.view_proj * vec4<f32>(world, 1.0);
}

@fragment
fn fs_outline() -> @location(0) vec4<f32> {
    return vec4<f32>(0.02, 0.02, 0.02, 0.75);
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
    // 水不染色：26.1 起水贴图（water_still.png）自带颜色，原版仅方块
    // 粒子/炼药锅走 water tint（BlockColors.java:38-39），不在本次范围。
    let tex = textureSample(terrain_tex, terrain_samp, v.uv, v.layer);
    let fog = fog_factor(v.dist, frame.fog_params);
    let sky_horizon = vec3<f32>(0.62, 0.76, 0.95);
    let day = frame.sun_dir_day.w;
    let fog_color = mix(vec3<f32>(0.02, 0.03, 0.08), sky_horizon, day);
    let lit = tex.rgb * v.shade;
    let color = mix(fog_color, lit, fog);
    return vec4<f32>(color, 0.72 * fog + 0.28);
}
