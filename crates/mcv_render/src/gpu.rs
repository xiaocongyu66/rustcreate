//! GPU renderer: pipelines, bind groups, per-frame draw orchestration.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::font;
use crate::frustum::Frustum;
use mcv_core::atlas;

pub const TERRAIN_STRIDE: usize = 24;
pub const HUD_STRIDE: usize = 28;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FrameUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub cam_pos_time: [f32; 4],
    pub sun_dir_day: [f32; 4],
    pub fog_params: [f32; 4],
}

const _: () = assert!(size_of::<FrameUniforms>() == 112);

/// One draw per loaded chunk. Buffers are uploaded once per remesh.
#[derive(Clone)]
pub struct RenderChunk {
    pub origin: [f32; 3],
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub opaque_range: Range<u32>,
    pub water_range: Range<u32>,
    pub aabb: (Vec3, Vec3),
}

/// One HUD rectangle (pixels, top-left origin). `tex` selects the source:
/// 0 = font atlas cell (glyph or the reserved solid-white cell 127),
/// 1 = terrain array layer.
pub struct HudQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// UV rect inside the source (0..1). For terrain tiles use 0..1.
    pub uv: [[f32; 2]; 2],
    pub color: [f32; 4],
    pub tex: u32,
    pub layer: u32,
}

pub struct Scene<'a> {
    pub camera: &'a Camera,
    pub time: f32,
    pub day_factor: f32,
    pub sun_dir: Vec3,
    /// Render target size in pixels (HUD coordinate space).
    pub width: f32,
    pub height: f32,
    pub chunks: &'a [RenderChunk],
    pub hud: &'a [HudQuad],
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    terrain_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    hud_pipeline: wgpu::RenderPipeline,
    frame_bind_layout: wgpu::BindGroupLayout,
    sky_bind_layout: wgpu::BindGroupLayout,
    hud_bind_layout: wgpu::BindGroupLayout,
    frame_buf: wgpu::Buffer,
    origins_buf: wgpu::Buffer,
    sky_buf: wgpu::Buffer,
    hud_uniform: wgpu::Buffer,
    hud_vbuf: wgpu::Buffer,
    hud_ibuf: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    hud_bind: wgpu::BindGroup,
    sky_bind: wgpu::BindGroup,
    pub max_chunks: u32,
    pub max_hud_quads: u32,
}

fn terrain_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: TERRAIN_STRIDE as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint16x2,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint16,
                offset: 16,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x2,
                offset: 18,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x2,
                offset: 20,
                shader_location: 4,
            },
        ],
    }
}

fn hud_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: HUD_STRIDE as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Unorm8x4,
                offset: 16,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32x2,
                offset: 20,
                shader_location: 3,
            },
        ],
    }
}

impl Renderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        texture_pack_dir: Option<&std::path::Path>,
    ) -> Self {
        let max_chunks: u32 = 1024;
        let max_hud_quads: u32 = 4096;

        // ---- terrain texture array ------------------------------------
        let payload = {
            let mut p = atlas::generate_payload();
            if let Some(dir) = texture_pack_dir {
                let n = atlas::load_pack_over(
                    dir,
                    &mut p[..atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4],
                );
                if n > 0 {
                    log::info!("texture pack: overrode {n} tiles from {}", dir.display());
                    // rebuild mip1 after override
                    let mut mip1 = vec![0u8; atlas::LAYERS * 8 * 8 * 4];
                    atlas::generate_mip1(
                        &p[..atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4],
                        &mut mip1,
                    );
                    let tail = atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4;
                    p[tail..].copy_from_slice(&mip1);
                }
            }
            p
        };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terrain-array"),
            size: wgpu::Extent3d {
                width: atlas::TILE_PX as u32,
                height: atlas::TILE_PX as u32,
                depth_or_array_layers: atlas::LAYERS as u32,
            },
            mip_level_count: atlas::MIP_LEVELS,
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
                bytes_per_row: Some((atlas::TILE_PX * 4) as u32),
                rows_per_image: Some(atlas::TILE_PX as u32),
            },
            wgpu::Extent3d {
                width: atlas::TILE_PX as u32,
                height: atlas::TILE_PX as u32,
                depth_or_array_layers: atlas::LAYERS as u32,
            },
        );
        let terrain_view = tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            ..Default::default()
        });

        // ---- font texture ---------------------------------------------
        let font_data = font::build_texture_data();
        let font_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("font"),
            size: wgpu::Extent3d {
                width: font::TEX_W as u32,
                height: font::TEX_H as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        {
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("font-staging"),
                contents: &font_data,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some((font::TEX_W * 4) as u32),
                        rows_per_image: Some(font::TEX_H as u32),
                    },
                },
                font_tex.as_image_copy(),
                wgpu::Extent3d {
                    width: font::TEX_W as u32,
                    height: font::TEX_H as u32,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([enc.finish()]);
        }
        let font_view = font_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let hud_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hud-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // ---- bind layouts ---------------------------------------------
        let frame_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame-layout"),
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
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
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
        let sky_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let hud_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        // ---- buffers ---------------------------------------------------
        let frame_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame-uniforms"),
            size: size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let origins_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-origins"),
            size: (max_chunks as u64) * 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky-uniforms"),
            size: 176, // inv_view_proj 64 + 3 vec4
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud-uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud-vbuf"),
            size: (max_hud_quads as u64) * 4 * HUD_STRIDE as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_ibuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud-ibuf"),
            size: (max_hud_quads as u64) * 6 * 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ---- bind groups ----------------------------------------------
        let frame_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame-bind"),
            layout: &frame_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &origins_buf,
                        offset: 0,
                        size: std::num::NonZeroU64::new(16),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&terrain_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let sky_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-bind"),
            layout: &sky_bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: sky_buf.as_entire_binding(),
            }],
        });
        let hud_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hud-bind"),
            layout: &hud_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: hud_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&font_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&hud_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&terrain_view),
                },
            ],
        });

        // ---- pipelines -------------------------------------------------
        let frame_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/terrain.wgsl").into()),
        });
        let sky_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/sky.wgsl").into()),
        });
        let hud_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/hud.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world-layout"),
            bind_group_layouts: &[Some(&frame_bind_layout)],
            immediate_size: 0,
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky-pipeline-layout"),
            bind_group_layouts: &[Some(&sky_bind_layout)],
            immediate_size: 0,
        });
        let hud_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud-pipeline-layout"),
            bind_group_layouts: &[Some(&hud_bind_layout)],
            immediate_size: 0,
        });

        let depth_stencil = Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        let depth_read_only = Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_mod,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[Some(terrain_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frame_mod,
                entry_point: Some("fs_terrain"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let water_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_mod,
                entry_point: Some("vs_water"),
                compilation_options: Default::default(),
                buffers: &[Some(terrain_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frame_mod,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: depth_read_only,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState {
                module: &sky_mod,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &sky_mod,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let hud_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud"),
            layout: Some(&hud_layout),
            vertex: wgpu::VertexState {
                module: &hud_mod,
                entry_point: Some("vs_hud"),
                compilation_options: Default::default(),
                buffers: &[Some(hud_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &hud_mod,
                entry_point: Some("fs_hud"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            device,
            queue,
            terrain_pipeline,
            water_pipeline,
            sky_pipeline,
            hud_pipeline,
            frame_bind_layout,
            sky_bind_layout,
            hud_bind_layout,
            frame_buf,
            origins_buf,
            sky_buf,
            hud_uniform,
            hud_vbuf,
            hud_ibuf,
            frame_bind,
            hud_bind,
            sky_bind,
            max_chunks,
            max_hud_quads,
        }
    }

    /// Renders one frame into `target` (color view + matching depth view).
    /// Chunks beyond max_chunks are ignored (frustum-culled first).
    pub fn draw_frame(
        &mut self,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        scene: &Scene,
    ) {
        let cam = scene.camera;
        let eye = cam.pos + glam::Vec3::new(0.0, crate::EYE_HEIGHT, 0.0);
        let vp = cam.view_proj();

        let uniforms = FrameUniforms {
            view_proj: vp.to_cols_array_2d(),
            cam_pos_time: [eye.x, eye.y, eye.z, scene.time],
            sun_dir_day: [
                scene.sun_dir.x,
                scene.sun_dir.y,
                scene.sun_dir.z,
                scene.day_factor,
            ],
            fog_params: [0.006, 0.0, cam.far * 0.95, 0.0],
        };
        self.queue
            .write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&uniforms));

        let inv_vp = vp.inverse();
        let mut sky_u = [0f32; 44];
        sky_u[..16].copy_from_slice(&inv_vp.to_cols_array());
        let eye_arr = [eye.x, eye.y, eye.z, scene.time];
        sky_u[16..20].copy_from_slice(&eye_arr);
        let sun_arr = [
            scene.sun_dir.x,
            scene.sun_dir.y,
            scene.sun_dir.z,
            scene.day_factor,
        ];
        sky_u[20..24].copy_from_slice(&sun_arr);
        let hor = [0.62, 0.76, 0.95, 0.0];
        sky_u[24..28].copy_from_slice(&hor);
        self.queue
            .write_buffer(&self.sky_buf, 0, bytemuck::cast_slice(&sky_u));

        // frustum cull + slot assignment
        let frustum = Frustum::from_view_proj(&vp);
        let mut visible: Vec<(u32, &RenderChunk)> = Vec::with_capacity(64);
        for (i, rc) in scene.chunks.iter().enumerate() {
            if (i as u32) >= self.max_chunks {
                break;
            }
            let (min, max) = rc.aabb;
            if frustum.intersects_aabb(min, max) {
                visible.push((i as u32, rc));
            }
        }
        if !visible.is_empty() {
            let mut origins = Vec::with_capacity(visible.len() * 64);
            for (_, rc) in &visible {
                origins.extend_from_slice(&[rc.origin[0], rc.origin[1], rc.origin[2], 0.0]);
                origins.extend_from_slice(&[0.0f32; 60]); // pad slot to 256 B
            }
            self.queue
                .write_buffer(&self.origins_buf, 0, bytemuck::cast_slice(&origins));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // sky first: writes no depth, terrain overdraws it
            pass.set_pipeline(&self.sky_pipeline);
            pass.set_bind_group(0, &self.sky_bind, &[]);
            pass.draw(0..3, 0..1);

            // opaque
            pass.set_pipeline(&self.terrain_pipeline);
            pass.set_bind_group(0, &self.frame_bind, &[]);
            for (slot, rc) in &visible {
                let off = (*slot as u32) * 256;
                pass.set_bind_group(0, &self.frame_bind, &[off]);
                pass.set_vertex_buffer(0, rc.vertex_buf.slice(..));
                pass.set_index_buffer(rc.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                if !rc.opaque_range.is_empty() {
                    pass.draw_indexed(rc.opaque_range.clone(), 0, 0..1);
                }
            }

            // water: far to near
            pass.set_pipeline(&self.water_pipeline);
            let mut water: Vec<(f32, u32, &RenderChunk)> = visible
                .iter()
                .filter(|(_, rc)| !rc.water_range.is_empty())
                .map(|(slot, rc)| {
                    let c = Vec3::from(rc.origin) + Vec3::new(8.0, 0.0, 8.0);
                    (c.distance_squared(eye), *slot, *rc)
                })
                .collect();
            water.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, slot, rc) in water {
                pass.set_bind_group(0, &self.frame_bind, &[slot * 256]);
                pass.set_vertex_buffer(0, rc.vertex_buf.slice(..));
                pass.set_index_buffer(rc.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(rc.water_range.clone(), 0, 0..1);
            }
        }

        // HUD pass
        if !scene.hud.is_empty() {
            let (verts, indices) = flatten_hud(scene.hud);
            self.queue
                .write_buffer(&self.hud_vbuf, 0, bytemuck::cast_slice(&verts));
            self.queue
                .write_buffer(&self.hud_ibuf, 0, bytemuck::cast_slice(&indices));
            let wh = [scene.width, scene.height, scene.time, 0.0];
            self.queue
                .write_buffer(&self.hud_uniform, 0, bytemuck::cast_slice(&wh));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.hud_pipeline);
            pass.set_bind_group(0, &self.hud_bind, &[]);
            pass.set_vertex_buffer(0, self.hud_vbuf.slice(..(verts.len() * HUD_STRIDE) as u64));
            pass.set_index_buffer(
                self.hud_ibuf.slice(..(indices.len() * 4) as u64),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..(indices.len() as u32), 0, 0..1);
        }

        self.queue.submit([encoder.finish()]);
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HudVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
    src: [u32; 2],
}

fn flatten_hud(quads: &[HudQuad]) -> (Vec<HudVertex>, Vec<u32>) {
    let mut verts = Vec::with_capacity(quads.len() * 4);
    let mut indices = Vec::with_capacity(quads.len() * 6);
    for q in quads {
        let base = verts.len() as u32;
        let c = [
            (q.color[0] * 255.0) as u8,
            (q.color[1] * 255.0) as u8,
            (q.color[2] * 255.0) as u8,
            (q.color[3] * 255.0) as u8,
        ];
        for (dx, dy, uv) in [
            (0.0, 0.0, q.uv[0]),
            (q.w, 0.0, [q.uv[1][0], q.uv[0][1]]),
            (q.w, q.h, q.uv[1]),
            (0.0, q.h, [q.uv[0][0], q.uv[1][1]]),
        ] {
            verts.push(HudVertex {
                pos: [q.x + dx, q.y + dy],
                uv,
                color: c,
                src: [q.tex, q.layer],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, indices)
}
