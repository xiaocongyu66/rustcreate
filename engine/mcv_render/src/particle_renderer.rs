//! 粒子渲染：独立小 draw（单管线 + 静态索引 + 每帧顶点重建）。
//!
//! 对照 26.1：单 quad 粒子走 `ParticleRenderType.SINGLE_QUADS`
//! （`SingleQuadParticle.java:109-111`），原版按 Layer（不透明/半透明 ×
//! 方块/物品/particles 图集）分管线；本引擎收敛为一个半透明管线 +
//! 顶点携带纹理组（粒子量小、GLES 兼容，任务书「量小怎么简单怎么来」）。
//!
//! GLES/wgpu30 纪律（与 gpu.rs MeshUploader 相同）：创建期映射的 buffer
//! 一律经 `slice(..).get_mapped_range_mut()` 写入、`queue.write_buffer`
//! 只用于未映射 buffer（Vulkan 容忍、GLES 直接 fatal，见 gpu.rs:66-69 注）。

use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::particles::{MAX_DRAW_QUADS, PARTICLE_PX, ParticleEngine, ParticleVertex, sprites};

/// 单帧绘制上下文（压缩 draw 形参）。
pub struct DrawCtx<'a> {
    pub engine: &'a ParticleEngine,
    pub cam: &'a Camera,
    pub day: f32,
    pub partial_tick: f32,
}

/// 粒子渲染器：particles 纹理数组、管线、静态索引缓冲、顶点暂存。
pub struct ParticleRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    particles_view: wgpu::TextureView,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    /// 顶点暂存（每帧复用；`scratch.len()/4` = 提取的 quad 数）。
    scratch: Vec<ParticleVertex>,
    /// 已告警过的截断标记（防日志刷屏）。
    warned_truncation: bool,
}

impl ParticleRenderer {
    /// 建 particles 纹理数组 + 管线 + 缓冲。
    ///
    /// 贴图从 `<assets>/textures/particle/` 读原版 PNG（splash_0..3 /
    /// crit_0..11 / bubble，素材红线：一律原版），缺失回退程序化占位
    /// （无素材部署路径，与 font/celestial 同策略）。
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        assets_dir: Option<&std::path::Path>,
    ) -> Self {
        let layers = sprites::PARTICLE_LAYERS as usize;
        let mut payload = vec![0u8; layers * PARTICLE_PX * PARTICLE_PX * 4];
        for layer in 0..layers as u32 {
            let img = assets_dir
                .and_then(|root| {
                    let p = root
                        .join("textures/particle")
                        .join(particle_file_name(layer));
                    image::open(&p).ok()
                })
                .map(|img| img.to_rgba8())
                .map(|rgba| {
                    image::imageops::resize(
                        &rgba,
                        PARTICLE_PX as u32,
                        PARTICLE_PX as u32,
                        image::imageops::FilterType::Nearest,
                    )
                    .into_raw()
                })
                .unwrap_or_else(|| fallback_frame(layer));
            let off = layer as usize * PARTICLE_PX * PARTICLE_PX * 4;
            payload[off..off + PARTICLE_PX * PARTICLE_PX * 4].copy_from_slice(&img);
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particles-array"),
            size: wgpu::Extent3d {
                width: PARTICLE_PX as u32,
                height: PARTICLE_PX as u32,
                depth_or_array_layers: sprites::PARTICLE_LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &payload,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((PARTICLE_PX * 4) as u32),
                rows_per_image: Some(PARTICLE_PX as u32),
            },
            wgpu::Extent3d {
                width: PARTICLE_PX as u32,
                height: PARTICLE_PX as u32,
                depth_or_array_layers: sprites::PARTICLE_LAYERS,
            },
        );
        let particles_view = tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        // 绑定布局：0 = frame uniform（复用 gpu.frame_buf，view_proj/cam/fog
        // 与 terrain 同源）、1 = terrain 数组（crack 碎屑直接采样方块图集，
        // 「crack 粒子采样方块图集同张纹理」）、2 = particles 数组、
        // 3 = sampler（复用 terrain 最近采样器）。
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/particle.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_particle"),
                compilation_options: Default::default(),
                buffers: &[Some(particle_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_particle"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None, // billboard 双面
                ..Default::default()
            },
            // 深度只读不写：与水/裂纹 overlay 一致（半透明序）。
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle-vbuf"),
            size: (MAX_DRAW_QUADS * 4 * size_of::<ParticleVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 静态顺序索引：quad i → [4i, 4i+1, 4i+2, 4i, 4i+2, 4i+3]
        // （原版 QUADS 模式顺序索引等价，QuadParticleRenderState prepare）。
        let mut indices = Vec::with_capacity(MAX_DRAW_QUADS * 6);
        for q in 0..MAX_DRAW_QUADS as u32 {
            let b = q * 4;
            indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particle-ibuf"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            pipeline,
            bind_layout,
            vbuf,
            ibuf,
            scratch: Vec::with_capacity(1024),
            warned_truncation: false,
            particles_view,
        }
    }

    /// 建 bind group（frame_buf 复用 gpu.rs 的 FrameUniforms 缓冲）。
    pub fn build_bind_group(
        &self,
        device: &wgpu::Device,
        frame_buf: &wgpu::Buffer,
        terrain_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle-bind"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(terrain_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.particles_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// 提取 + 上传 + 绘制（在 crack overlay 后、水前的 pass 段内调用）。
    pub fn draw(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        bind: &wgpu::BindGroup,
        ctx: DrawCtx<'_>,
        queue: &wgpu::Queue,
    ) {
        self.scratch.clear();
        ctx.engine
            .extract_vertices(ctx.cam, ctx.day, ctx.partial_tick, &mut self.scratch);
        let quads = self.scratch.len() / 4;
        if quads == 0 {
            return;
        }
        let quads = quads.min(MAX_DRAW_QUADS);
        if self.scratch.len() / 4 > MAX_DRAW_QUADS && !self.warned_truncation {
            log::warn!(
                "particles: {} visible quads > draw cap {MAX_DRAW_QUADS}, truncating (once)",
                self.scratch.len() / 4
            );
            self.warned_truncation = true;
        }
        let verts = &self.scratch[..quads * 4];
        queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(verts));
        let count = quads as u32 * 6;
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.set_vertex_buffer(
            0,
            self.vbuf
                .slice(..(quads * 4 * size_of::<ParticleVertex>()) as u64),
        );
        pass.set_index_buffer(
            self.ibuf.slice(..(count as u64 * 4)),
            wgpu::IndexFormat::Uint32,
        );
        pass.draw_indexed(0..count, 0, 0..1);
    }
}

/// particles 数组帧文件名（`assets/minecraft/textures/particle/`）。
fn particle_file_name(layer: u32) -> &'static str {
    match layer {
        0..=3 => [
            "splash_0.png",
            "splash_1.png",
            "splash_2.png",
            "splash_3.png",
        ][layer as usize],
        4..=15 => {
            const CRIT: [&str; 12] = [
                "crit_0.png",
                "crit_1.png",
                "crit_2.png",
                "crit_3.png",
                "crit_4.png",
                "crit_5.png",
                "crit_6.png",
                "crit_7.png",
                "crit_8.png",
                "crit_9.png",
                "crit_10.png",
                "crit_11.png",
            ];
            CRIT[(layer - 4) as usize]
        }
        _ => "bubble.png",
    }
}

/// 缺素材时的程序化占位（无素材部署路径；正常部署不触发）。
fn fallback_frame(layer: u32) -> Vec<u8> {
    let mut px = vec![0u8; PARTICLE_PX * PARTICLE_PX * 4];
    let center = match layer {
        l if (sprites::SPLASH_BASE..sprites::SPLASH_BASE + sprites::SPLASH_COUNT).contains(&l) => {
            [160u8, 200, 255, 220] // splash：淡蓝
        }
        l if (sprites::CRIT_BASE..sprites::CRIT_BASE + sprites::CRIT_COUNT).contains(&l) => {
            [255u8, 220, 90, 230] // crit：金黄
        }
        _ => [120u8, 180, 255, 180], // bubble：更淡
    };
    for y in 4..12usize {
        for x in 4..12usize {
            let o = (y * PARTICLE_PX + x) * 4;
            px[o..o + 4].copy_from_slice(&center);
        }
    }
    px
}

/// 顶点布局（ParticleVertex 48 B，见 particle.wgsl PVertIn）。
fn particle_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: size_of::<ParticleVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 20,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 24,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 28,
                shader_location: 4,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 44,
                shader_location: 5,
            },
        ],
    }
}
