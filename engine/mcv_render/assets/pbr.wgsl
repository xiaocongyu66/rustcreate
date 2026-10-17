// PBR 材质光照库：GGX 高光 + Lambert 漫反射（#223 续）。
//
// 红线：本文件是**新增独立模块**，不替换现有 8 pass 的任何 WGSL——
// 默认（无 `hdr`/PBR 接线）不参与渲染，仅作为后续 PBR pass 的材质
// 光照段源。整文件可独立编译（末尾有 probe 入口，供 headless 验证
// 与 uniform 打包对拍，不进生产 draw）。
//
// 蓝图锚（bevy 0.20 bevy_pbr，/root/engine-ref/bevy/crates/bevy_pbr）：
// - D_GGX（pbr_lighting.wesl:146-152，Filament walter07 k 形式）
// - V_SmithGGXCorrelated（:185-191，高度相关 Smith 可见性 V = G/4(n·v)(n·l)）
// - F_Schlick / fresnel（:306-324，f90 = saturate(dot(f0, 50·0.33))）
// - calculate_F0 / calculate_F0_dielectric（pbr_functions.wesl:277-286，
//   电介质 F0 = 0.16·reflectance²，Filament remapping）
// - perceptualRoughnessToRoughness（pbr_lighting.wesl:546-552，
//   clamp 0.089..1 后平方）
// - 直射光合成 directional_light（:898）`color = (diffuse + specular)·NdotL`
//   再乘 light.color（:925）。
// 有意偏差（任务面指定 Lambert 漫反射；bevy 默认 Fd_Burley）：
// - 漫反射用 Lambert BRDF `albedo·(1−metallic)/π`。
// - 单直射光（MC 太阳），无 clearcoat/透射/各向异性分支。

diagnostic(off, derivative_uniformity);

const PI: f32 = 3.141592653589793;

// ------------------------------------------------------------------
// MaterialUniform：位段与 mcv_render::pbr::MaterialUniform（Rust, repr(C),
// 3×vec4 = 48B）逐字段同构。uniform 步长按 WGSL 对齐——vec4 对齐 16B，
// 总 48B 无隐式填充；打包对拍由 tests/pbr.rs 逐字节锁死。
struct MaterialUniform {
    // xyz = 基色（线性 RGB，0..1+），w = 透明度。
    albedo: vec4<f32>,
    // xyz = 法线（世界空间，单位向量；默认接线前由调用方保证），
    // w = 自由。
    normal: vec4<f32>,
    // x = metallic [0,1]；y = perceptual roughness [0,1]（线性感知粗糙度，
    // 非线性 α 由 perceptual_roughness_to_roughness 求取）；z = exposure
    // 曝光系数（线性乘子，默认 1.0 = 行为不变），w = 自由。
    params: vec4<f32>,
};

// 单直射光 + 帧常量（MC 太阳；与 FrameUniforms 分开，避免动 176B 旧布局）。
// 与 mcv_render::pbr::PbrFrameUniform（Rust，48B）逐字段同构。
struct PbrFrameUniform {
    // xyz = 指向太阳的方向（世界空间，单位向量），w = day factor（昼轨，
    // lib.rs day_factor 产出；0..1）。
    sun_dir_day: vec4<f32>,
    // rgb = 太阳辐射颜色（线性，已含 dawn/dusk 色轨），w = 环境光强度
    // （半球常数项，几何不遮蔽）。
    sun_color_ambient: vec4<f32>,
    // xyz = 相机世界坐标（高光用），w = 自由。
    cam_pos: vec4<f32>,
};

// ------------------------------------------------------------------
// BRDF 段（公式与 bevy 逐行同形；名字保持 bevy 原名以便对照）。

// 感知粗糙度 → 非线性 α 粗糙度（bevy pbr_lighting.wesl:546-552）。
// clamp 0.089 防 fp 精度问题（Filament 推荐；bevy 默认 0.045 仅留注释）
fn perceptual_roughness_to_roughness(perceptual_roughness: f32) -> f32 {
    let clamped = clamp(perceptual_roughness, 0.089, 1.0);
    return clamped * clamped;
}

// 法线分布函数 D_GGX（bevy pbr_lighting.wesl:146-152，Filament walter07）：
// D(h,α) = α² / ( π ((n·h)²(α²−1) + 1)² )，k 形式等价展开防 fp16 问题。
fn d_ggx(roughness: f32, NdotH: f32) -> f32 {
    let one_minus_n_dh_sq = 1.0 - NdotH * NdotH;
    let a = NdotH * roughness;
    let k = roughness / (one_minus_n_dh_sq + a * a);
    return k * k * (1.0 / PI);
}

// 可见性函数 V = G / (4 (n·v)(n·l))（bevy pbr_lighting.wesl:185-191，
// 高度相关 Smith，Filament 形式）。
fn v_smith_ggx_correlated(roughness: f32, NdotV: f32, NdotL: f32) -> f32 {
    let a2 = roughness * roughness;
    let lambda_v = NdotL * sqrt((NdotV - a2 * NdotV) * NdotV + a2);
    let lambda_l = NdotV * sqrt((NdotL - a2 * NdotL) * NdotL + a2);
    return 0.5 / (lambda_v + lambda_l);
}

// fresnel（bevy pbr_lighting.wesl:319-324）：f90 由 f0 的亮度饱和推出
// （Filament occlusion 建议），避免 vec3 分支。
fn fresnel(f0: vec3<f32>, LdotH: f32) -> vec3<f32> {
    let f90 = saturate(dot(f0, vec3<f32>(50.0 * 0.33)));
    return f0 + (f90 - f0) * pow(1.0 - LdotH, 5.0);
}

// F0 remapping（bevy pbr_functions.wesl:277-286）：
// 电介质 F0 = 0.16·reflectance²，金属 = base_color，线性混合。
fn calculate_f0_dielectric(reflectance: f32) -> f32 {
    return 0.16 * reflectance * reflectance;
}
fn calculate_f0(base_color: vec3<f32>, metallic: f32, reflectance: f32) -> vec3<f32> {
    return mix(vec3<f32>(calculate_f0_dielectric(reflectance)), base_color, metallic);
}

// Lambert 漫反射 BRDF：albedo·(1−metallic)/π（任务面指定 Lambert；
// bevy 默认 Fd_Burley 为有意偏差，metal 衰减同 bevy
// calculate_diffuse_color pbr_functions.wesl:267-275 金属项）。
fn lambert_diffuse(diffuse_color: vec3<f32>) -> vec3<f32> {
    return diffuse_color * (1.0 / PI);
}

// ------------------------------------------------------------------
// 直射光合成（bevy directional_light 组合式：color = (diffuse + specular)
// × NdotL × light.color，pbr_lighting.wesl:898/925）。
// N/V/L 均世界空间单位向量；mat.params 已由调用方 clamp。
fn evaluate_pbr(N: vec3<f32>, V: vec3<f32>, L: vec3<f32>, mat: MaterialUniform, frame: PbrFrameUniform) -> vec3<f32> {
    let metallic = clamp(mat.params.x, 0.0, 1.0);
    let perceptual = clamp(mat.params.y, 0.0, 1.0);
    let roughness = perceptual_roughness_to_roughness(perceptual);
    let reflectance = 0.5; // bevy StandardMaterial 默认 reflectance

    // Neubelt/Pettineo 2013（bevy pbr_functions.wesl:348-349）：NdotV 抬下限。
    let NdotV = max(dot(N, V), 0.0001);
    let NdotL = max(dot(N, L), 0.0);
    let NdotH = max(dot(N, normalize(L + V)), 0.0);
    let LdotH = max(dot(L, normalize(L + V)), 0.0);

    let diffuse_color = mat.albedo.rgb * (1.0 - metallic);
    let f0 = calculate_f0(mat.albedo.rgb, metallic, reflectance);

    let specular = d_ggx(roughness, NdotH) * v_smith_ggx_correlated(roughness, NdotV, NdotL)
        * fresnel(f0, LdotH);
    let diffuse = lambert_diffuse(diffuse_color);

    // 直射（bevy directional_light 组合式）+ 常数环境（只乘漫反射；
    // bevy 环境在 pbr_ambient.wesl 单独走 IBL，本仓无 IBL → 用标量常数项）。
    // 昼夜/天气轨由 CPU 预乘进 sun_color_ambient（folded；shader 不再乘
    // w——w 仅作信息位下发，避免双重乘）。
    let direct = (diffuse + specular) * NdotL * frame.sun_color_ambient.rgb;
    let ambient = diffuse_color * frame.sun_color_ambient.w;
    return direct + ambient;
}

// ------------------------------------------------------------------
// probe 入口（独立可编译 + GPU 对拍用；不进生产 draw）：全屏三角形
// 逐像素算太阳照射下的单位球，材质/帧 uniform 走 group(0) 打包验证。
struct ProbeOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_probe(@builtin(vertex_index) vi: u32) -> ProbeOut {
    // 全屏三角形：vi 0..2 → NDC，uv 由 xy 推回。
    var xy = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = xy[vi];
    let uv = p * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    var out: ProbeOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var<uniform> probe_material: MaterialUniform;
@group(0) @binding(1) var<uniform> probe_frame: PbrFrameUniform;

@fragment
fn fs_probe(in: ProbeOut) -> @location(0) vec4<f32> {
    // 单位球位于屏幕中心：uv→球面法线，球外丢弃（静默输出 0）。
    let p = in.uv * 2.0 - vec2<f32>(1.0, 1.0);
    let r2 = dot(p, p);
    if (r2 > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let n = vec3<f32>(p.x, p.y, sqrt(1.0 - r2));
    // V 指向相机（固定 +Z 偏上），光取 uniform。
    let v = vec3<f32>(0.0, 0.0, 1.0);
    let l = probe_frame.sun_dir_day.xyz;
    let rgb = evaluate_pbr(n, v, l, probe_material, probe_frame);
    let exposure = probe_material.params.z;
    return vec4<f32>(rgb * exposure, 1.0);
}
