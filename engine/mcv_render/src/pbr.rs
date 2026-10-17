//! PBR 材质域（#223 续）：MaterialUniform 打包 + GGX/Lambert/ACES 的
//! CPU 对照实现（与 `assets/pbr.wgsl` / `assets/hdr.wgsl` 逐公式同形，
//! tests/pbr.rs 按「uniform 打包 / tone mapping 离线对照表」锁死两侧行
//! 为等价）。
//!
//! 无 GPU 依赖：本模块纯 CPU 数学与字节布局，默认 `cargo test` 即全量
//! 执行；WGSL 文本经 [`PBR_WGSL`] / [`HDR_WGSL`] 内嵌，headless
//! （gpu-tests feature）再走 create_shader_module 与探测管线验证编译面。
//!
//! 默认关断纪律：现有 8 pass 的 WGSL 与 golden 零改动——本模块只提供
//! 蓝图实现与验证通路，不挂进任何生产 draw。

/// PBR 材质光照库源（crate 内嵌，独立可编译的库模块，含 probe 入口
/// vs_probe/fs_probe，供 headless 编译面与 uniform 对拍）。
pub const PBR_WGSL: &str = include_str!("../assets/pbr.wgsl");

/// HDR tone mapping 后处理源（bevy core_pipeline tonemapping 蓝图：
/// ACESFitted + screen_space_dither；由 `hdr` feature 的
/// [`crate::hdr::HdrStage`] 建管线，默认关）。
pub const HDR_WGSL: &str = include_str!("../assets/hdr.wgsl");

const PI: f32 = std::f32::consts::PI;

/// MaterialUniform 位段（`assets/pbr.wgsl` MaterialUniform 逐字段同构，
/// 3×vec4 = 48B，WGSL vec4 对齐 16B 无隐式填充）：
/// - `albedo`：xyz = 基色（线性 RGB），w = 透明度；
/// - `normal`：xyz = 世界空间法线（单位向量），w = 自由；
/// - `params`：x = metallic、y = perceptual roughness（线性感知粗糙度，
///   非线性 α 在 shader/CPU 由 [`perceptual_roughness_to_roughness`]
///   求取）、z = exposure（线性乘子，默认 1.0 = 行为不变），w = 自由。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialUniform {
    pub albedo: [f32; 4],
    pub normal: [f32; 4],
    pub params: [f32; 4],
}

const _: () = assert!(size_of::<MaterialUniform>() == 48);

impl Default for MaterialUniform {
    /// 默认 = 全零位段，唯一例外 exposure = 1.0（默认参数行为不变契约）。
    fn default() -> Self {
        Self {
            albedo: [0.0; 4],
            normal: [0.0, 0.0, 0.0, 0.0],
            params: [0.0, 0.0, 1.0, 0.0],
        }
    }
}

impl MaterialUniform {
    /// 语义化构造：albedo（线性 RGBA）+ 法线 + metallic/perceptual
    /// roughness；exposure 固定 1.0（默认行为不变）。
    pub fn new(
        albedo: [f32; 4],
        normal: [f32; 3],
        metallic: f32,
        perceptual_roughness: f32,
    ) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&metallic),
            "metallic 必须在 [0,1]（clamp 在 shader 端兜底，CPU 端越界即 bug）"
        );
        debug_assert!(
            (0.0..=1.0).contains(&perceptual_roughness),
            "perceptual roughness 必须在 [0,1]"
        );
        Self {
            albedo,
            normal: [normal[0], normal[1], normal[2], 0.0],
            params: [metallic, perceptual_roughness, 1.0, 0.0],
        }
    }
}

/// PbrFrameUniform（`assets/pbr.wgsl` PbrFrameUniform 逐字段同构，48B）：
/// - `sun_dir_day`：xyz = 指向太阳方向（世界空间，单位向量），w = day
///   factor（信息位：昼夜轨已由 CPU 预乘进 sun_color_ambient，shader
///   不再乘 w，避免双重乘）；
/// - `sun_color_ambient`：rgb = 太阳辐射色（线性，已含昼/晨昏轨），w =
///   常数环境强度（只乘漫反射项）；
/// - `cam_pos`：xyz = 相机世界坐标，w = 自由。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PbrFrameUniform {
    pub sun_dir_day: [f32; 4],
    pub sun_color_ambient: [f32; 4],
    pub cam_pos: [f32; 4],
}

const _: () = assert!(size_of::<PbrFrameUniform>() == 48);

// ------------------------------------------------------------------
// BRDF 对照实现（公式与 pbr.wgsl 逐行同形；函数名保持 bevy 原名）。

/// 感知粗糙度 → 非线性 α（bevy pbr_lighting.wesl:546-552：clamp
/// 0.089..1 后平方）。
pub fn perceptual_roughness_to_roughness(perceptual_roughness: f32) -> f32 {
    let clamped = perceptual_roughness.clamp(0.089, 1.0);
    clamped * clamped
}

/// D_GGX（bevy pbr_lighting.wesl:146-152，Filament walter07 k 形式）。
pub fn d_ggx(roughness: f32, ndot_h: f32) -> f32 {
    let one_minus_n_dh_sq = 1.0 - ndot_h * ndot_h;
    let a = ndot_h * roughness;
    let k = roughness / (one_minus_n_dh_sq + a * a);
    k * k * (1.0 / PI)
}

/// 可见性函数 V = G / (4 (n·v)(n·l))（bevy pbr_lighting.wesl:185-191，
/// 高度相关 Smith）。
pub fn v_smith_ggx_correlated(roughness: f32, ndot_v: f32, ndot_l: f32) -> f32 {
    let a2 = roughness * roughness;
    let lambda_v = ndot_l * ((ndot_v - a2 * ndot_v) * ndot_v + a2).sqrt();
    let lambda_l = ndot_v * ((ndot_l - a2 * ndot_l) * ndot_l + a2).sqrt();
    0.5 / (lambda_v + lambda_l)
}

/// fresnel（bevy pbr_lighting.wesl:319-324）：f90 = saturate(dot(f0,
/// 50·0.33))，F = f0 + (f90−f0)(1−LdotH)⁵。
pub fn fresnel(f0: [f32; 3], ldot_h: f32) -> [f32; 3] {
    let dot = f0[0] * 50.0 * 0.33 + f0[1] * 50.0 * 0.33 + f0[2] * 50.0 * 0.33;
    let f90 = dot.clamp(0.0, 1.0);
    let w = (1.0 - ldot_h).powi(5);
    [
        f0[0] + (f90 - f0[0]) * w,
        f0[1] + (f90 - f0[1]) * w,
        f0[2] + (f90 - f0[2]) * w,
    ]
}

/// 电介质 F0 = 0.16·reflectance²（bevy pbr_functions.wesl:277-280）。
pub fn calculate_f0_dielectric(reflectance: f32) -> f32 {
    0.16 * reflectance * reflectance
}

/// F0 remapping（bevy pbr_functions.wesl:284-286）：电介质↔金属线性混合。
pub fn calculate_f0(base_color: [f32; 3], metallic: f32, reflectance: f32) -> [f32; 3] {
    let d = calculate_f0_dielectric(reflectance);
    [
        d + (base_color[0] - d) * metallic,
        d + (base_color[1] - d) * metallic,
        d + (base_color[2] - d) * metallic,
    ]
}

/// Lambert 漫反射 BRDF（任务面指定；bevy 默认 Fd_Burley 为有意偏差，
/// metal 衰减项与 bevy calculate_diffuse_color 一致）。
pub fn lambert_diffuse(diffuse_color: [f32; 3]) -> [f32; 3] {
    [
        diffuse_color[0] * (1.0 / PI),
        diffuse_color[1] * (1.0 / PI),
        diffuse_color[2] * (1.0 / PI),
    ]
}

/// 直射合成对照实现（pbr.wgsl `evaluate_pbr` 逐公式同构）：
/// `(diffuse + D·V·F)·NdotL·sun_color + diffuse·ambient`。N/V/L 均世界
/// 空间单位向量（调用方保证）。
pub fn evaluate_pbr_reference(
    n: [f32; 3],
    v: [f32; 3],
    l: [f32; 3],
    mat: &MaterialUniform,
    frame: &PbrFrameUniform,
) -> [f32; 3] {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let metallic = mat.params[0].clamp(0.0, 1.0);
    let perceptual = mat.params[1].clamp(0.0, 1.0);
    let roughness = perceptual_roughness_to_roughness(perceptual);
    let reflectance = 0.5; // bevy StandardMaterial 默认 reflectance

    // Neubelt/Pettineo 2013（bevy pbr_functions.wesl:348-349）：NdotV 抬下限。
    let ndot_v = dot(n, v).max(0.0001);
    let ndot_l = dot(n, l).max(0.0);
    // 半向量 h = normalize(L + V)。
    let hl = [l[0] + v[0], l[1] + v[1], l[2] + v[2]];
    let h_len = dot(hl, hl).sqrt();
    let h = [hl[0] / h_len, hl[1] / h_len, hl[2] / h_len];
    let ndot_h = dot(n, h).max(0.0);
    let ldot_h = dot(l, h).max(0.0);

    let a = mat.albedo;
    let albedo = [a[0], a[1], a[2]];
    let diffuse_color = [
        albedo[0] * (1.0 - metallic),
        albedo[1] * (1.0 - metallic),
        albedo[2] * (1.0 - metallic),
    ];
    let f0 = calculate_f0(albedo, metallic, reflectance);

    let spec = d_ggx(roughness, ndot_h) * v_smith_ggx_correlated(roughness, ndot_v, ndot_l);
    let f = fresnel(f0, ldot_h);
    let dl = lambert_diffuse(diffuse_color);
    let sun = frame.sun_color_ambient;
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let direct = (dl[i] + spec * f[i]) * ndot_l * sun[i];
        let ambient = diffuse_color[i] * sun[3];
        out[i] = direct + ambient;
    }
    out
}

// ------------------------------------------------------------------
// ACES tone mapping 对照实现（hdr.wgsl 逐值同构；bevy
// tonemapping.wesl:106-140，Stephen Hill fit / BakingLab ACES.hlsl）。

/// RRT+ODT 多项式拟合（tonemapping.wesl:106-110）。
fn rrt_and_odt_fit(v: [f32; 3]) -> [f32; 3] {
    let a = [
        v[0] * (v[0] + 0.0245786) - 0.000090537,
        v[1] * (v[1] + 0.0245786) - 0.000090537,
        v[2] * (v[2] + 0.0245786) - 0.000090537,
    ];
    let b = [
        v[0] * (0.983729 * v[0] + 0.432951) + 0.238081,
        v[1] * (0.983729 * v[1] + 0.432951) + 0.238081,
        v[2] * (0.983729 * v[2] + 0.432951) + 0.238081,
    ];
    [a[0] / b[0], a[1] / b[1], a[2] / b[2]]
}

/// 行向量右乘 3×3 矩阵：out_i = Σⱼ vⱼ·M[j][i]。参数 m 的行 = WGSL
/// `mat3x3` 字面量的**列**（列主存储），保证与 `v * mat3x3` 同值。
fn mul_mat3_cols(v: [f32; 3], m: [[f32; 3]; 3]) -> [f32; 3] {
    [
        v[0] * m[0][0] + v[1] * m[1][0] + v[2] * m[2][0],
        v[0] * m[0][1] + v[1] * m[1][1] + v[2] * m[2][1],
        v[0] * m[0][2] + v[1] * m[1][2] + v[2] * m[2][2],
    ]
}

/// 输入侧矩阵（tonemapping.wesl:116-120）：sRGB → XYZ → D65_2_D60 →
/// AP1 → RRT_SAT。数组按 WGSL 字面列序存放（hdr.wgsl 同一字面量）。
pub const RGB_TO_RRT_COLS: [[f32; 3]; 3] = [
    [0.59719, 0.35458, 0.04823],
    [0.076, 0.90834, 0.01566],
    [0.0284, 0.13383, 0.83777],
];

/// 输出侧矩阵（tonemapping.wesl:123-127）：ODT_SAT → XYZ → D60_2_D65
/// → sRGB。
pub const ODT_TO_RGB_COLS: [[f32; 3]; 3] = [
    [1.60475, -0.53108, -0.07367],
    [-0.10208, 1.10813, -0.00605],
    [-0.00327, -0.07276, 1.07602],
];

/// ACESFitted 对照实现（输入线性 HDR，输出显示线性 0..1 截断）。
pub fn aces_fitted(color: [f32; 3]) -> [f32; 3] {
    let m = mul_mat3_cols(color, RGB_TO_RRT_COLS);
    let f = rrt_and_odt_fit(m);
    let o = mul_mat3_cols(f, ODT_TO_RGB_COLS);
    [
        o[0].clamp(0.0, 1.0),
        o[1].clamp(0.0, 1.0),
        o[2].clamp(0.0, 1.0),
    ]
}

/// 线性显示值 → sRGB 8bit（目标 `Rgba8UnormSrgb` 的硬件编码曲线；规格
/// 分段公式，脚点 0.0031308/12.92）。离线对照表与 GPU 读回对拍用。
pub fn linear_to_srgb_u8(x: f32) -> u8 {
    let s = if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (s.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8
}

/// screen_space_dither 对照（tonemapping.wesl:305-309；WGSL `fract` =
/// x − floor(x)，此处同式，d 为负时与 Rust `%` 余元语义不同——必须用
/// floor 形式，frag_coord 非负下两式等价）。
pub fn screen_space_dither(frag_coord: [f32; 2]) -> [f32; 3] {
    let d = 171.0 * frag_coord[0] + 231.0 * frag_coord[1];
    [103.0, 71.0, 97.0].map(|m| {
        let f = d / m;
        f - f.floor()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// uniform 打包对照表：48B 位级（f32 小端位模式逐段）锚定 repr(C)
    /// 排布 × WGSL 同构；曝光默认 1.0 契约位同锁。
    #[test]
    fn material_uniform_packing_table() {
        let m = MaterialUniform {
            albedo: [0.5, 1.0, 0.25, 0.8],
            normal: [0.0, 1.0, 0.0, 0.0],
            params: [1.0, 0.5, 1.0, 0.0],
        };
        let bytes: &[u8] = bytemuck::bytes_of(&m);
        assert_eq!(bytes.len(), 48, "3×vec4 位段总长 48B");
        let got: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let expect: &[f32] = &[
            0.5, 1.0, 0.25, 0.8, // albedo
            0.0, 1.0, 0.0, 0.0, // normal
            1.0, 0.5, 1.0, 0.0, // params（metallic, perceptual, exposure, pad）
        ];
        assert_eq!(got, expect, "MaterialUniform 位段必须与 WGSL 同构排布");
        // 默认构造：exposure 恒 1.0（「默认 1.0 行为不变」契约位）。
        assert_eq!(MaterialUniform::default().params[2], 1.0);
        let n = MaterialUniform::new([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 1.0], 0.0, 0.5);
        assert_eq!(n.params[2], 1.0);
        // PbrFrameUniform 同构锁：48B 三段顺序。
        let f = PbrFrameUniform {
            sun_dir_day: [1.0, 0.0, 0.0, 1.0],
            sun_color_ambient: [0.5, 0.5, 0.5, 0.1],
            cam_pos: [3.0, 4.0, 5.0, 0.0],
        };
        let fb: &[u8] = bytemuck::bytes_of(&f);
        assert_eq!(fb.len(), 48);
        let got: Vec<f32> = fb
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(
            got,
            vec![1.0, 0.0, 0.0, 1.0, 0.5, 0.5, 0.5, 0.1, 3.0, 4.0, 5.0, 0.0]
        );
    }

    /// GGX BRDF 离线对照表（手算锚点 + 特性点）。
    #[test]
    fn ggx_reference_table() {
        let c = 1e-6;
        // 感知粗糙度 → α：0.089 下限平方、上限 1 截断。
        assert!((perceptual_roughness_to_roughness(0.0) - 0.089 * 0.089).abs() < c);
        assert!((perceptual_roughness_to_roughness(0.5) - 0.25).abs() < c);
        assert_eq!(perceptual_roughness_to_roughness(1.2), 1.0);
        // D_GGX 正入射峰 = 1/(π·α²)：α=0.25 → 16/π。
        let d = d_ggx(0.25, 1.0);
        assert!((d - 16.0 / PI).abs() < 1e-4, "D 峰 {d}");
        // clamp 粗糙度（perceptual=0 → α=0.089²）下的 D 峰。
        let a = perceptual_roughness_to_roughness(0.0);
        let d0 = d_ggx(a, 1.0);
        assert!((d0 - 1.0 / (PI * a * a)).abs() < 1e-3, "clamp α D 峰 {d0}");
        // V 正入射（NdotV=NdotL=1）：λ=1 → v=0.25。
        assert!((v_smith_ggx_correlated(0.5, 1.0, 1.0) - 0.25).abs() < c);
        // 45° 三角 roughness=1（α=1）：λ=NdotL·√(NdotV²)=0.5 → v=0.5。
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!((v_smith_ggx_correlated(1.0, s, s) - 0.5).abs() < c);
        // F0：reflectance 0.5 → 0.04；fresnel 两端点（LdotH=1 → f0；
        // LdotH=0 → f90 饱和 1.0）。
        assert!((calculate_f0_dielectric(0.5) - 0.04).abs() < c);
        let f = fresnel([0.04; 3], 1.0);
        assert!((f[0] - 0.04).abs() < c && (f[1] - 0.04).abs() < c, "{f:?}");
        let f = fresnel([0.04; 3], 0.0);
        assert!(f.iter().all(|x| (x - 1.0).abs() < c), "{f:?}");
        // F0 remapping 两端：全金属 = base_color；0 金属 = 电介质灰。
        assert_eq!(calculate_f0([1.0, 0.8, 0.6], 1.0, 0.5), [1.0, 0.8, 0.6]);
        assert_eq!(calculate_f0([1.0, 0.8, 0.6], 0.0, 0.5), [0.04, 0.04, 0.04]);
        // Lambert 白 albedo = 1/π。
        let lambert = lambert_diffuse([1.0; 3]);
        assert!(
            lambert
                .iter()
                .all(|x| (x - std::f32::consts::FRAC_1_PI).abs() < c)
        );
    }

    /// evaluate_pbr 端到端手算分解：白电介质正对照（n=v=l=+z）。
    #[test]
    fn evaluate_pbr_reference_composition() {
        let mat = MaterialUniform::new([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 1.0], 0.0, 0.5);
        let frame = PbrFrameUniform {
            sun_dir_day: [0.0, 0.0, 1.0, 1.0],
            sun_color_ambient: [1.0, 1.0, 1.0, 0.0],
            cam_pos: [0.0, 0.0, 2.0, 0.0],
        };
        let out = evaluate_pbr_reference(
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            &mat,
            &frame,
        );
        // 逐项手算锚（perceptual=0.5 → α=0.25）：D=16/π；V=0.25；F=0.04
        // （LdotH=1）；Lambert=1/π；NdotL=1 → direct+ambient 同基线性相加。
        let d = 16.0 / PI;
        let spec = d * 0.25 * 0.04;
        let expect = (1.0 / PI) + spec;
        for x in out {
            assert!(
                (x - expect).abs() < 1e-4,
                "白电介质正对照点 out={x} expect={expect}"
            );
        }
    }

    /// ACES tone mapping 离线对照表（hdr.wgsl aces_fitted 离线锚）。
    #[test]
    fn aces_reference_table() {
        let c = 1e-6;
        // 0 → 0（saturate 下限）。
        let z = aces_fitted([0.0, 0.0, 0.0]);
        assert!(z.iter().all(|x| x.abs() < 1e-5), "{z:?}");
        // 1.0（白）端点：RRT/ODT fit 白点（v=1）：
        // a = 1.0245786−0.000090537 = 1.02448806；
        // b = 0.983729+0.432951+0.238081 = 1.654761 → fit≈0.619。
        let w = aces_fitted([1.0, 1.0, 1.0]);
        for x in w {
            assert!(x > 0.58 && x < 0.70, "白点输出区间（未 gamma 编码）{x}");
        }
        // 手算链复核（v=1 → M1 → RRT/ODT → M2）：白→白大致对角，w 与手算
        // 链逐值一致（同函数内对照，锁住 aces_fitted 与分步实现同源）。
        let mid = rrt_and_odt_fit(mul_mat3_cols([1.0; 3], RGB_TO_RRT_COLS));
        let after = mul_mat3_cols(mid, ODT_TO_RGB_COLS);
        for i in 0..3 {
            assert!(
                (w[i] - after[i].clamp(0.0, 1.0)).abs() < c,
                "白点 ch{i} 分步链 {} vs aces_fitted {}",
                after[i],
                w[i]
            );
        }
        // 灰点 0.18（18% 中灰）在显示线性段的合法区间（≈0.12..0.16）。
        let g = aces_fitted([0.18, 0.18, 0.18]);
        for x in g {
            assert!(x > 0.10 && x < 0.17, "中灰区间 x={x}");
        }
        // 单调性： brighter 输入必不更暗（三通道逐点）。
        let lo = aces_fitted([0.3, 0.3, 0.3]);
        let hi = aces_fitted([0.6, 0.6, 0.6]);
        for i in 0..3 {
            assert!(hi[i] >= lo[i], "单调性 ch{i}: {} < {}", hi[i], lo[i]);
        }
        // HDR 高光编码：10.0 输入后仍饱和到 ≤1.0（RRT 肩部压缩而非裁剪炸裂）。
        let hdr = aces_fitted([10.0, 5.0, 1.0]);
        assert!(hdr.iter().all(|x| (0.0..=1.0).contains(x)), "{hdr:?}");
        // sRGB 量化表锚（linear_to_srgb_u8）：0 → 0；显示线性 0.5 →
        // 1.055·0.5^(1/2.4)−0.055 ≈ 0.735357 → u8 188。
        assert_eq!(linear_to_srgb_u8(0.0), 0);
        let mid_lin = 1.055 * 0.5f32.powf(1.0 / 2.4) - 0.055;
        assert_eq!(
            linear_to_srgb_u8(0.5),
            (mid_lin * 255.0 + 0.5).floor() as u8
        );
        assert!(linear_to_srgb_u8(0.5) >= 187 && linear_to_srgb_u8(0.5) <= 189);
        assert_eq!(linear_to_srgb_u8(1.0), 255);
        // dither 幅界：|dither| ≤ 0.5/255（±半 LSB）。
        let d0 = screen_space_dither([0.0, 0.0]);
        let d9 = screen_space_dither([9.0, 9.0]);
        for x in d0.into_iter().chain(d9) {
            assert!(x.abs() <= 0.5 / 255.0 + 1e-7, "dither {x}");
        }
    }
}
