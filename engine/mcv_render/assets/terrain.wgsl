// Chunk terrain: opaque + water passes. One draw per chunk; chunk origin
// comes from a dynamic-offset uniform.

diagnostic(off, derivative_uniformity);

struct FrameUniforms {
    view_proj: mat4x4<f32>,
    cam_pos_time: vec4<f32>,
    sun_dir_day: vec4<f32>,
    // 雾参数（语义见 fog_factor/fog_color）：x = exp2 密度（水上）；
    // y/z = 水下线性 start/end；w = 水下旗标（>0.5 → 线性雾 + 雾色切
    // WATER_FOG_COLOR）。
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

// 雾透过率。两种语义按水下旗标 fog.w 选路（gpu.rs draw_frame 填参）：
// - fog.w ≤ 0.5：exp2 密度雾（常规大气，旧语义不变，density=0 → 无雾）；
// - fog.w > 0.5：线性 start..end 雾，start/end 取 fog.y/fog.z —— 水下用
//   原版端点 WATER_FOG_START/END_DISTANCE = −8/96
//   （EnvironmentAttributes.java:36-41，WaterFogEnvironment.java:16-21
//   直读该二值，无任何密度换算）。
fn fog_factor(dist: f32, fog: vec4<f32>) -> f32 {
    if (fog.w > 0.5) {
        return clamp((fog.z - dist) / (fog.z - fog.y), 0.0, 1.0);
    }
    return exp2(-dist * fog.x);
}

// 雾色：水下 = 原版 WATER_FOG_COLOR 0xFF050533（EnvironmentAttributes.java:33-35，
// WaterFogEnvironment.getBaseColor 直返该值；此前混天空蓝是「水下无水感」根因）；
// 水上 = 既有 夜色↔地平线 昼夜混色。fog.w > 0.5 = 水下旗标。
// 登记债：waterVision 提亮归一（FogRenderer.java:148-166）依赖玩家能力值，
// 本仓无该通路，暂按 brightenFactor=0（不提亮）。
const WATER_FOG_COLOR: vec3<f32> = vec3<f32>(5.0, 5.0, 51.0) / 255.0;

fn fog_color(fog: vec4<f32>) -> vec3<f32> {
    if (fog.w > 0.5) {
        return WATER_FOG_COLOR;
    }
    let sky_horizon = vec3<f32>(0.62, 0.76, 0.95);
    return mix(vec3<f32>(0.02, 0.03, 0.08), sky_horizon, frame.sun_dir_day.w);
}

// 水色（缺陷 1）：water_still.png 是灰度 tint 蒙版（实测均值 177,177,177），
// 蓝色来自逐顶点乘生物群系水色：FluidStateModelSet.java:13-18 给水流体模型
// 挂 BlockTintSources.water()（:129-139 → BiomeColors.getAverageWaterColor），
// overworld 默认色 OverworldBiomes.java:28 NORMAL_WATER_COLOR=4159204=0x3F76E4；
// FluidRenderer.java:88,169,205,307 把该色乘进每顶点。本仓无生物群系系统，
// 按 overworld 默认常数在片元乘（顶点/片元乘序对平面水等价）。
// 登记债：生物群系级水色变体 → mcv_core tint.rs 注册 water 层后改走
// biome_tint(tint_lut[layer])（该文件不在本波红线内）。
const WATER_TINT: vec3<f32> = vec3<f32>(63.0, 118.0, 228.0) / 255.0;

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
// gpu.rs 按设备 max_texture_array_layers 在下面占位行处展开：1..=4 个
// texture_2d_array 绑定声明 + sample_terrain 采样函数（上限 ≥ LAYERS 时
// 为单数组 binding 2，语义与拆分前一致；GLES 保底 256 时按 layer 区间
// if 链选数组，追加数组在 binding 5/6/7——4 固定给下方 tint_lut）。
// 占位标记必须独占整行——replace 会整行换生成代码，行尾不能带说明文字。
// @@TERRAIN_ARRAYS@@
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
    @location(4) aoflags: vec2<u32>, // x = ao, y = flags (bit0-2 face, bit3 水面顶面标识)
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
    // uv 单位 = 1/4096 tile（mesher kUvPerBlock）：除 4096 得 tile 数，
    // sampler repeat 平铺合并大面。此前除 65535 与 u16 满幅约定绑定，
    // 合并面无法表达多 tile 跨度。
    out.uv = vec2<f32>(v.uv) / 4096.0;
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
    let tex = sample_terrain(v.uv, v.layer);
    if (tex.a < 0.5) {
        discard;
    }
    // 按贴图层查 tint 类别并乘生物群系颜色（草顶/羊齿/树叶三族）。
    let tint = biome_tint(tint_lut[v.layer]);
    let fog = fog_factor(v.dist, frame.fog_params);
    let lit = tex.rgb * v.shade * tint;
    return vec4<f32>(mix(fog_color(frame.fog_params), lit, fog), 1.0);
}

// ---- mining crack overlay --------------------------------------------------
// 复用 vs_terrain（同顶点布局/同绑定组）；片元按裂纹层 alpha 混合。
// 顶点 CPU 侧外偏 0.003 防 z-fighting，管线不写深度。

@fragment
fn fs_crack(v: VtxOut) -> @location(0) vec4<f32> {
    let tex = sample_terrain(v.uv, v.layer);
    if (tex.a < 0.02) {
        discard;
    }
    let fog = fog_factor(v.dist, frame.fog_params);
    let lit = tex.rgb * v.shade;
    return vec4<f32>(mix(fog_color(frame.fog_params), lit, fog), tex.a);
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

// 顶面静态：原版流体几何无任何 time/顶点位移输入——FluidRenderer.java:25
// MAX_FLUID_HEIGHT=0.8888889，顶面高度只由邻居流体 getHeight 决定
// （:410-423），平静水面是等高静态平面；动画只有 water_still 图集逐帧
// （纹理帧动画为独立后续项）。旧 time-sin 顶点波（±0.05）系臆造，已删；
// flags bit3 保留仅作顶面标识（下沉量 WATER_TOP_SINK 在 mesher 侧，
// 归修复波 B）。
@vertex
fn vs_water(v: VtxIn) -> WaterOut {
    var out: WaterOut;
    let world = chunk.origin.xyz + v.pos;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = vec2<f32>(v.uv) / 4096.0;
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
    let tex = sample_terrain(v.uv, v.layer);
    let fog = fog_factor(v.dist, frame.fog_params);
    // 旧注释「26.1 水不染色、贴图自带颜色」是对 BlockColors.java:38-39 的
    // 误读——那两行只注册炼药锅/粒子色，流体网格染色走 FluidStateModelSet
    // 的 tintSource 线（见 WATER_TINT 注释）。灰度蒙版 × 0x3F76E4 = 蓝。
    let lit = tex.rgb * v.shade * WATER_TINT;
    let color = mix(fog_color(frame.fog_params), lit, fog);
    // alpha = 贴图 alpha（原版水不透明度语义：水色 ARGB=0xFF…，屏上
    // alpha≈贴图 180/255≈0.71 常数；雾只作用于 rgb，不改不透明度）。
    // 旧 0.72*fog+0.28（近处≈1.0 近不透明）系臆造，已回退。
    return vec4<f32>(color, tex.a);
}
