//! PBR/HDR 等价验证（#223 续）：
//! - CPU 面（默认跑）：MaterialUniform/PbrFrameUniform 48B 打包位表 +
//!   ACES tone mapping 离线对照表（独立 spec 抄写 × 属性锚）；
//! - GPU 面（`gpu-tests` feature，lavapipe）：pbr.wgsl/hdr.wgsl 编译面、
//!   PBR 探测管线渲染逐像素 vs CPU 参考实现（uniform 打包↔着色器同构）、
//!   HdrStage 端到端 Rgba16Float→ACES→Rgba8UnormSrgb 读回逐像素 vs 离线
//!   表（dither 逐像素入期望，±1 LSB 容差）。
//!
//! 蓝图对照（bevy）：tonemapping.wesl（ACESFitted/dither）、
//! pbr_lighting.wesl（D_GGX/V_Smith/F_Schlick）、pbr_functions.wesl（F0）。

use mcv_render::pbr::{
    HDR_WGSL, MaterialUniform, PBR_WGSL, PbrFrameUniform, aces_fitted, calculate_f0,
    calculate_f0_dielectric, d_ggx, evaluate_pbr_reference, fresnel, lambert_diffuse,
    linear_to_srgb_u8, perceptual_roughness_to_roughness, screen_space_dither,
    v_smith_ggx_correlated,
};

#[test]
fn hdr_wgsl_and_pbr_wgsl_are_embedded() {
    // 文本面存在性 + 关键锚词（防 include 截断/ stray 改动静默漂移）。
    assert!(PBR_WGSL.contains("fn d_ggx"), "pbr.wgsl 必须含 D_GGX 段");
    assert!(PBR_WGSL.contains("struct MaterialUniform"));
    assert!(PBR_WGSL.contains("params: vec4<f32>"), "exposure 位段 y/z");
    assert!(HDR_WGSL.contains("fn aces_fitted"));
    assert!(HDR_WGSL.contains("0.983729"), "ACES fit 常量锚");
    assert!(HDR_WGSL.contains("params.x"), "exposure 线性乘子语义");
}

#[test]
fn material_uniform_packing_offline_table() {
    // 48B 位段（offline 对照表）：与 src/pbr.rs 单测互补——这里从公开
    // API 构造曝光非默认值+法线轴，锁位串。
    let m = MaterialUniform::new([0.5, 0.25, 0.125, 1.0], [0.0, 0.0, 1.0], 0.75, 0.3);
    let bytes: &[u8] = bytemuck::bytes_of(&m);
    assert_eq!(bytes.len(), 48);
    let got: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    assert_eq!(
        got,
        vec![
            0.5, 0.25, 0.125, 1.0, // albedo（线性 RGBA）
            0.0, 0.0, 1.0, 0.0, // normal + pad
            0.75, 0.3, 1.0, 0.0, // metallic, perceptual, exposure(默认 1.0), pad
        ]
    );
    // 曝光契约位：默认 1.0（行为不变），可覆写。
    let mut m2 = m;
    m2.params[2] = 0.5;
    assert_eq!(bytemuck::bytes_of(&m2).len(), 48);
    // 帧常量段同构：48B。
    let f = PbrFrameUniform::default();
    assert_eq!(bytemuck::bytes_of(&f).len(), 48);
}

/// 独立 spec 抄写的 ACES 参照（BakingLab ACES.hlsl / bevy
/// tonemapping.wesl 同式二次实现——与 src/pbr.rs 交叉互证，双抄写位
/// 不会同错）。
fn aces_ref(v: [f32; 3]) -> [f32; 3] {
    let mat = |m: [[f32; 3]; 3], x: [f32; 3]| -> [f32; 3] { mul_cols(m, x) };
    let rrt = [
        [0.59719, 0.35458, 0.04823],
        [0.076, 0.90834, 0.01566],
        [0.0284, 0.13383, 0.83777],
    ];
    let odt = [
        [1.60475, -0.53108, -0.07367],
        [-0.10208, 1.10813, -0.00605],
        [-0.00327, -0.07276, 1.07602],
    ];
    // 行向量右乘（同 mul_cols 语义），两次 + fit：
    let x = mat(rrt, v);
    let fit = [
        (x[0] * (x[0] + 0.0245786) - 0.000090537)
            / (x[0] * (0.983729 * x[0] + 0.432951) + 0.238081),
        (x[1] * (x[1] + 0.0245786) - 0.000090537)
            / (x[1] * (0.983729 * x[1] + 0.432951) + 0.238081),
        (x[2] * (x[2] + 0.0245786) - 0.000090537)
            / (x[2] * (0.983729 * x[2] + 0.432951) + 0.238081),
    ];
    // saturate 即刻截断（spec `saturate`）。
    mat(odt, fit).map(|c| c.clamp(0.0, 1.0))
}

fn mul_cols(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        v[0] * m[0][0] + v[1] * m[1][0] + v[2] * m[2][0],
        v[0] * m[0][1] + v[1] * m[1][1] + v[2] * m[2][1],
        v[0] * m[0][2] + v[1] * m[1][2] + v[2] * m[2][2],
    ]
}

#[test]
fn aces_tone_mapping_offline_table() {
    let c = 1e-6;
    // 交叉互证：独立 spec 抄写 vs src/pbr.rs 实现（8 点表）。
    let samples: [[f32; 3]; 8] = [
        [0.0, 0.0, 0.0],
        [0.18, 0.18, 0.18],
        [0.25, 0.5, 0.75],
        [0.5, 1.0, 2.0],
        [1.0, 1.0, 1.0],
        [2.0, 4.0, 8.0],
        [10.0, 0.0, 0.0],
        [0.0, 0.0, 100.0],
    ];
    for s in samples {
        let a = aces_fitted(s);
        let b = aces_ref(s);
        for i in 0..3 {
            assert!(
                (a[i] - b[i]).abs() < c,
                "ACES 表 ch{i}: {s:?} → {a:?} vs 特写 {b:?}"
            );
        }
        assert!(a.iter().all(|x| (0.0..=1.0).contains(x)), "{s:?} → {a:?}");
    }
    // 曝光线性乘子语义（hdr.wgsl params.x）：color×exposure 先于 ACES。
    let e = aces_fitted([0.36, 0.36, 0.36]); // 0.18 × 2
    let h = aces_fitted([0.18, 0.18, 0.18]);
    assert!(e[0] > h[0] * 1.5, "曝光倍增必须提亮 {e:?} vs {h:?}");
}

#[test]
fn pbr_brdf_and_composition_offline_table() {
    let c = 1e-5;
    // α per perceptual（0.089 clamp 平方）。
    assert!((perceptual_roughness_to_roughness(0.0) - 0.007921).abs() < c);
    // 电介质 F0（reflectance 0.5）→ 0.04；金属 F0 → base_color。
    assert!((calculate_f0_dielectric(0.5) - 0.04).abs() < c);
    assert_eq!(calculate_f0([1.0, 0.0, 0.0], 1.0, 0.5), [1.0, 0.0, 0.0]);
    // Lambert 白 = 1/π。
    let lambert = lambert_diffuse([1.0; 3]);
    assert!((lambert[0] - std::f32::consts::FRAC_1_PI).abs() < c);
    // fresnel 端点。
    let f = fresnel([0.04; 3], 0.0);
    assert!((f[0] - 1.0).abs() < c, "LdotH=0 → f90 饱和 {f:?}");
    let f = fresnel([0.04; 3], 1.0);
    assert!((f[0] - 0.04).abs() < c);
    // D 峰与 V 正入射（手算锚）。
    assert!((d_ggx(0.25, 1.0) - 16.0 / std::f32::consts::PI).abs() < 1e-4);
    assert!((v_smith_ggx_correlated(0.5, 1.0, 1.0) - 0.25).abs() < c);
    // 合成锚：白电介质正对照（与 src/pbr.rs 单测同点不同断言域）。
    // w=0.25 环境直乘 albedo（无 1/π——环境辐照语义直乘），背光仍得 0.25。
    let mat = MaterialUniform::new([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 1.0], 0.0, 0.5);
    let frame = PbrFrameUniform {
        sun_dir_day: [0.0, 0.0, 1.0, 1.0],
        sun_color_ambient: [1.0, 1.0, 1.0, 0.25],
        cam_pos: [0.0, 0.0, 2.0],
    };
    let out = evaluate_pbr_reference(
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        &mat,
        &frame,
    );
    // diffuse=1/π；spec=(16/π)·V(0.25)·F(0.04)；ambient=0.25·albedo。
    let expect = 1.0 / std::f32::consts::PI + (16.0 / std::f32::consts::PI) * 0.25 * 0.04 + 0.25;
    for x in out {
        assert!((x - expect).abs() < 1e-4, "合成锚 {x} vs {expect}");
    }
    // 环境项（w=0.25）在背光点（l=-z）：NdotL=0 → 纯环境。
    let out_back = evaluate_pbr_reference(
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        &mat,
        &frame,
    );
    assert!(
        (out_back[0] - 0.25).abs() < 1e-4,
        "背光=纯环境直乘 albedo {out_back:?}"
    );
}

// ------------------------------------------------------------------
// GPU 面（gpu-tests feature；lavapipe headless）。

#[cfg(feature = "gpu-tests")]
mod gpu {
    use super::*;
    use mcv_render::OffscreenTarget;
    use wgpu::util::DeviceExt;

    fn setup() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("no adapter: install mesa-vulkan-drivers for lavapipe");
        let mut limits = wgpu::Limits::downlevel_defaults();
        limits.max_texture_array_layers = adapter.limits().max_texture_array_layers;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pbr-test"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .expect("device");
        (device, queue)
    }

    /// 测试数值域（0..16）的 IEEE754 f16 编码（正规数，无次正规/Inf）。
    fn f32_to_f16(v: f32) -> u16 {
        let bits = v.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let man = ((bits >> 13) & 0x3ff) as u16;
        assert!(exp > 0 && exp < 31, "测试域溢出 {v}");
        sign | ((exp as u16) << 10) | man
    }

    /// pbr.wgsl / hdr.wgsl 编译面（naga 校验 = 独立可编译性；pbr 为
    /// 库模块含 probe 入口，hdr 为全屏 pass；create_shader_module 即
    /// 校验，错误以 panic 显形）。
    #[test]
    fn pbr_and_hdr_wgsl_modules_compile() {
        let (device, _queue) = setup();
        for (name, src) in [("pbr", PBR_WGSL), ("hdr", HDR_WGSL)] {
            let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
        }
    }

    /// PBR 探测（uniform 打包↔shader 同构端到端）：16×16 全屏球，
    /// 逐像素 vs CPU 参考实现（±1 LSB sRGB 量化容差）。
    #[test]
    fn pbr_probe_render_matches_cpu() {
        let (device, queue) = setup();
        let target = OffscreenTarget::new(
            &device,
            wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-probe"),
            source: wgpu::ShaderSource::Wgsl(PBR_WGSL.into()),
        });
        // 位段布局锁（BGL min_binding_size = 48B）。
        let bind_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pbr-probe-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                size_of::<MaterialUniform>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                size_of::<PbrFrameUniform>() as u64
                            ),
                        },
                        count: None,
                    },
                ],
            });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr-probe-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr-probe-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_probe"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_probe"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let material = MaterialUniform::new([0.9, 0.6, 0.3, 1.0], [0.0, 0.0, 1.0], 0.35, 0.4);
        let frame = PbrFrameUniform {
            sun_dir_day: [0.0, 0.6, 1.0, 1.0],
            sun_color_ambient: [1.0, 0.9, 0.8, 0.15],
            cam_pos: [0.0, 0.0, 3.0],
        };
        let mat_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pbr-probe-mat"),
            contents: bytemuck::bytes_of(&material),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let frame_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pbr-probe-frame"),
            contents: bytemuck::bytes_of(&frame),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pbr-probe-bind"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: mat_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frame_buf.as_entire_binding(),
                },
            ],
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pbr-probe-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color_view(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        target.enqueue_copy(&mut enc);
        queue.submit([enc.finish()]);
        let px = target.read_pixels(&device);

        // 逐像素 CPU 对照：球内像素 = srgb(evaluate×exposure)，球外 = 0。
        let n = [0.0f32, 0.6, 1.0];
        let l_len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let l = [n[0] / l_len, n[1] / l_len, n[2] / l_len];
        let v = [0.0f32, 0.0, 1.0];
        let exposure = material.params[2];
        let mut checked_on_sphere = 0usize;
        let mut checked_off_sphere = 0usize;
        for y in 0..16u32 {
            for x in 0..16u32 {
                let uv = [(x as f32 + 0.5) / 16.0, 1.0 - (y as f32 + 0.5) / 16.0];
                let p = [uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0];
                let r2 = p[0] * p[0] + p[1] * p[1];
                let i = ((y * 16 + x) * 4) as usize;
                let expect = if r2 <= 1.0 {
                    let nz = (1.0 - r2).sqrt();
                    let normal = [p[0], p[1], nz];
                    let lin = evaluate_pbr_reference(normal, v, l, &material, &frame);
                    [
                        linear_to_srgb_u8(lin[0] * exposure),
                        linear_to_srgb_u8(lin[1] * exposure),
                        linear_to_srgb_u8(lin[2] * exposure),
                    ]
                } else {
                    [0, 0, 0]
                };
                for ch in 0..3 {
                    let got = px[i + ch];
                    assert!(
                        (got as i32 - expect[ch] as i32).abs() <= 1,
                        "pbr probe px({x},{y})ch{ch}: gpu={got} cpu={expect[ch]}"
                    );
                }
                if r2 <= 1.0 {
                    checked_on_sphere += 1;
                } else {
                    checked_off_sphere += 1;
                }
            }
        }
        assert!(checked_on_sphere > 64, "球面像素覆盖 {checked_on_sphere}");
        assert!(checked_off_sphere > 32, "球外像素覆盖 {checked_off_sphere}");
    }

    // HDR E2E 需要 `hdr` feature（HdrStage 才存在于编译面）。
    #[cfg(feature = "hdr")]
    mod hdr_e2e {
        use super::*;

        /// HDR 端到端（离线对照 GPU 读回）：Rgba16Float 主纹理 → ACES →
        /// sRGB 目标；曝光 1.0 / 0.5 两段（默认 1.0 = 行为不变契约位）。
        #[test]
        fn hdr_tonemap_matches_offline_table() {
            let (device, queue) = setup();
            let stage = mcv_render::hdr::HdrStage::new(
                &device,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                16,
                16,
            );
            let target = OffscreenTarget::new(
                &device,
                wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
            );

            // 主纹理值表：中灰/白/HDR 高光（f16 精确表示的值域）。
            let mut hdr_px = vec![0u8; 16 * 16 * 8];
            let values: [f32; 4] = [0.18, 1.0, 4.0, 0.0];
            for y in 0..16u32 {
                for x in 0..16u32 {
                    let v = values[(x % values.len() as u32) as usize];
                    let off = ((y * 16 + x) * 8) as usize;
                    for ch in 0..3 {
                        hdr_px[off + ch * 2..off + ch * 2 + 2]
                            .copy_from_slice(&f32_to_f16(v).to_le_bytes());
                    }
                    hdr_px[off + 6..off + 8].copy_from_slice(&f32_to_f16(1.0).to_le_bytes());
                }
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: stage.hdr_texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &hdr_px,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(16 * 8),
                    rows_per_image: Some(16),
                },
                wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
            );

            // 曝光 1.0 → ACES；dither 逐像素入期望（±1 LSB 容差吃硬件舍入）。
            stage.set_exposure(&queue, 1.0);
            {
                let mut enc = device.create_command_encoder(&Default::default());
                stage.encode_tonemap(&mut enc, &target.color_view());
                target.enqueue_copy(&mut enc);
                queue.submit([enc.finish()]);
            }
            let px = target.read_pixels(&device);
            for y in 0..16u32 {
                for x in 0..16u32 {
                    let v = values[(x % values.len() as u32) as usize];
                    let aces = aces_fitted([v, v, v]);
                    let d = screen_space_dither([x as f32 + 0.5, y as f32 + 0.5]);
                    let i = ((y * 16 + x) * 4) as usize;
                    for ch in 0..3 {
                        let expect = linear_to_srgb_u8((aces[ch] + d[ch]).clamp(0.0, 1.0));
                        let got = px[i + ch];
                        assert!(
                            (got as i32 - expect as i32).abs() <= 1,
                            "hdr( exposure=1) px({x},{y})ch{ch}: gpu={got} cpu={expect}"
                        );
                    }
                }
            }

            // 曝光 0.5 → 半倍再 ACES（非线性轨迹同源验证）。
            stage.set_exposure(&queue, 0.5);
            {
                let mut enc = device.create_command_encoder(&Default::default());
                stage.encode_tonemap(&mut enc, &target.color_view());
                target.enqueue_copy(&mut enc);
                queue.submit([enc.finish()]);
            }
            let px = target.read_pixels(&device);
            for y in 0..16u32 {
                for x in 0..16u32 {
                    let v = values[(x % values.len() as u32) as usize] * 0.5;
                    // 手工链复核（独立抄写）：spec ACES × 0.5 曝光。
                    let aces = aces_ref([v, v, v]);
                    let d = screen_space_dither([x as f32 + 0.5, y as f32 + 0.5]);
                    let i = ((y * 16 + x) * 4) as usize;
                    for ch in 0..3 {
                        let expect = linear_to_srgb_u8((aces[ch] + d[ch]).clamp(0.0, 1.0));
                        let got = px[i + ch];
                        assert!(
                            (got as i32 - expect as i32).abs() <= 1,
                            "hdr(exposure=0.5) px({x},{y})ch{ch}: gpu={got} cpu={expect}"
                        );
                    }
                }
            }
        }
    }
}
