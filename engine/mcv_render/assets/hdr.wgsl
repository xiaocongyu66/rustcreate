// HDR tone mapping 后处理（独立 pass，默认关——仅 `hdr` feature 开启且
// 后端可用时由 mcv_render::hdr::HdrStage 创建管线）。
//
// 蓝图锚（bevy 0.20 bevy_core_pipeline）：
// - tonemapping_frag.wesl:22-47：全屏采样 HDR 主纹理 → tone_mapping →
//   输出到交换链（主纹理恒 Rgba16Float，node.rs:46-47 debug_assert）。
// - tonemapping.wesl:106-140：ACESFitted（Stephen Hill fit，BakingLab
//   ACES.hlsl，与 Godot 4.0 同款实现；矩阵/常量逐值照抄）。
// - tonemapping.wesl:305-309：screen_space_dither（Valve GDC 2015）。
// - tonemapping.wesl:399：exposure 乘子 `color·2^exposure`（stops 语义）。
// 有意偏差：本仓 Camera::exposure 为**线性乘子**（默认 1.0 = 行为不变），
// 不做 2^x stops 换算（stops 语义下默认 1.0 会 ×2 提亮）；无 ColorGrading
// 分段调色（bevy SECTIONAL_COLOR_GRADING 分支不移植，本仓无该配置面）。

struct TonemapUniform {
    // x = 曝光系数（线性乘子，CPU 写入 Camera::exposure，默认 1.0），
    // yzw = 自由。
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> tonemap: TonemapUniform;
@group(0) @binding(1) var hdr_texture: texture_2d<f32>;
@group(0) @binding(2) var hdr_sampler: sampler;

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FullscreenVertexOutput {
    var xy = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: FullscreenVertexOutput;
    out.position = vec4<f32>(xy[vi], 0.0, 1.0);
    // NDC(-1..1) → uv(0..1)，y 翻转对齐 wgpu 纹理坐标（+Y 向下采样坐标）。
    out.uv = xy[vi] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

// RRT + ODT 拟合（tonemapping.wesl:106-110 逐值照抄）。
fn rrt_and_odt_fit(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return a / b;
}

// ACESFitted（tonemapping.wesl:112-140 逐值照抄）：
// sRGB → XYZ → D65_2_D60 → AP1 → RRT_SAT → ODT_SAT → XYZ → D60_2_D65 → sRGB。
fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    var fitted = color;

    // 输入侧矩阵（行主 vec3 构造 = bevy 同形）。
    let rgb_to_rrt = mat3x3<f32>(
        vec3<f32>(0.59719, 0.35458, 0.04823),
        vec3<f32>(0.07600, 0.90834, 0.01566),
        vec3<f32>(0.02840, 0.13383, 0.83777),
    );

    // 输出侧矩阵。
    let odt_to_rgb = mat3x3<f32>(
        vec3<f32>(1.60475, -0.53108, -0.07367),
        vec3<f32>(-0.10208, 1.10813, -0.00605),
        vec3<f32>(-0.00327, -0.07276, 1.07602),
    );

    fitted = fitted * rgb_to_rrt;
    fitted = rrt_and_odt_fit(fitted);
    fitted = fitted * odt_to_rgb;
    return clamp(fitted, vec3<f32>(0.0), vec3<f32>(1.0));
}

// screen_space_dither（tonemapping.wesl:305-309 逐值照抄）：
// 破坏色带（banding）的逐像素 ±0.5/255 抖动。
fn screen_space_dither(frag_coord: vec2<f32>) -> vec3<f32> {
    var dither = vec3<f32>(dot(vec2<f32>(171.0, 231.0), frag_coord)).xxx;
    dither = fract(dither / vec3<f32>(103.0, 71.0, 97.0));
    return (dither - 0.5) / 255.0;
}

@fragment
fn fs_main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // HDR 线性色读取（Rgba16Float 主纹理，可为 0..1 之外值）。
    let hdr_color = textureSample(hdr_texture, hdr_sampler, in.uv).rgb;

    // 曝光（线性乘子；bevy 对应 tonemapping.wesl:399 的 2^exposure，本仓
    // Camera::exposure 默认 1.0 → 乘子恒等）。
    let color = max(hdr_color, vec3<f32>(0.0)) * tonemap.params.x;

    // ACES 拟合 + 逐像素抖动（bevy tonemapping_frag DEBAND_DITHER 分支）。
    let mapped = aces_fitted(color);
    let out_rgb = mapped + screen_space_dither(in.position.xy);

    return vec4<f32>(clamp(out_rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
