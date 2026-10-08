//! GPU renderer: pipelines, bind groups, per-frame draw orchestration.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::font;
use crate::frustum::Frustum;
use crate::gui::SpriteSheet;
use crate::player_mesh::{self, PART_COUNT, PLAYER_STRIDE, PlayerVertex, SKIN_LAYERS};
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
    /// Water geometry is meshed into its own index buffer (separate pass).
    pub water_index_buf: Option<wgpu::Buffer>,
    pub water_range: Range<u32>,
    pub aabb: (Vec3, Vec3),
}

/// 网格上传器：游戏层只交出顶点/索引字节，拿回 [`RenderChunk`]。
///
/// 这是引擎把 wgpu 挡在游戏层之外的唯一入口——C++ mesher 产出裸字节，
/// 本结构负责建 GPU 缓冲；游戏层因此不 `use wgpu`。水几何只上传独立索引
/// 缓冲、复用 opaque 顶点缓冲（与既有渲染语义一致）。
pub struct MeshUploader {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl MeshUploader {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self { device, queue }
    }

    fn vertex_index(&self, v: &[u8], i: &[u32]) -> (wgpu::Buffer, wgpu::Buffer) {
        let vb = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-vb"),
            size: (v.len() as u64).max(1),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        self.queue.write_buffer(&vb, 0, v);
        vb.unmap();
        let ib = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-ib"),
            size: (i.len() as u64 * 4).max(4),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(i));
        ib.unmap();
        (vb, ib)
    }

    /// 由裸网格字节组装整块 [`RenderChunk`]（16×256×16 AABB，origin 传入）。
    pub fn build_chunk(
        &self,
        origin: [f32; 3],
        vbytes: &[u8],
        ibytes: &[u32],
        water_ibytes: Option<&[u32]>,
    ) -> RenderChunk {
        let (vertex_buf, index_buf) = self.vertex_index(vbytes, ibytes);
        let water_index_buf = water_ibytes.map(|wi| self.vertex_index(&[], wi).1);
        RenderChunk {
            origin,
            vertex_buf,
            index_buf,
            opaque_range: 0..ibytes.len() as u32,
            water_index_buf,
            water_range: 0..water_ibytes.map_or(0, <[u32]>::len) as u32,
            aabb: (
                Vec3::new(origin[0], 0.0, origin[2]),
                Vec3::new(origin[0] + 16.0, 256.0, origin[2] + 16.0),
            ),
        }
    }
}

/// One HUD rectangle (pixels, top-left origin). `tex` selects the source:
/// 0 = font atlas cell (glyph or the reserved solid-white cell 127),
/// 1 = terrain array layer, 2 = GUI sprite sheet (MC 素材).
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
    /// 绕 quad 中心的旋转角(弧度,splash 文字用);常规 quad 传 0。
    pub rot: f32,
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
    /// 云：(资源, 设置)；None 或 enabled=false 不画（天空后、地形前）。
    pub cloud: Option<(&'a crate::Clouds, crate::CloudSettings)>,
    /// 玩家模型：(12 部位模型矩阵, 皮肤层 0=steve 1=alex)；第三人称时传入。
    pub player: Option<(&'a [glam::Mat4; PART_COUNT], u32)>,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    terrain_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    hud_pipeline: wgpu::RenderPipeline,
    frame_buf: wgpu::Buffer,
    origins_buf: wgpu::Buffer,
    sky_buf: wgpu::Buffer,
    hud_uniform: wgpu::Buffer,
    hud_vbuf: wgpu::Buffer,
    hud_ibuf: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    hud_bind: wgpu::BindGroup,
    sky_bind: wgpu::BindGroup,
    /// MC GUI 精灵表(texturepack/gui/);None = 回退程序化绘制。
    gui: Option<SpriteSheet>,
    player_pipeline: wgpu::RenderPipeline,
    player_bind_layout: wgpu::BindGroupLayout,
    player_uniform: wgpu::Buffer,
    player_vbuf: wgpu::Buffer,
    player_ibuf: wgpu::Buffer,
    player_bind: wgpu::BindGroup,
    player_sampler: wgpu::Sampler,
    skins_loaded: bool,
    pub max_chunks: u32,
    pub max_hud_quads: u32,
}

/// 玩家管线 uniform:view_proj + 12 部位模型矩阵(mat4x4 align 16,无填充)。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PlayerUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub models: [[[f32; 4]; 4]; PART_COUNT],
}

const _: () = assert!(size_of::<PlayerUniforms>() == 832);

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

fn player_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: PLAYER_STRIDE as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Unorm8x2,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32x2,
                offset: 16,
                shader_location: 2,
            },
        ],
    }
}

const _: () = assert!(size_of::<PlayerVertex>() == PLAYER_STRIDE);

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
        // 真实官方贴图 827 层 + 裂纹；GLES downlevel 上限 256 → 按 device
        // limits 钳制（app.rs 建 device 时已尽量抬到 adapter 上限，Vulkan 桌面
        // 可吃满）。被钳掉的层按地址回绕采样，显示错贴图但不崩溃。
        let max_layers = atlas::LAYERS.min(device.limits().max_texture_array_layers as usize);
        let (payload, n_layers) = atlas::generate_payload_clamped(texture_pack_dir, max_layers);
        if n_layers < atlas::LAYERS {
            log::warn!(
                "texture array clamped {}→{} layers (device limit)",
                atlas::LAYERS,
                n_layers
            );
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terrain-array"),
            size: wgpu::Extent3d {
                width: atlas::TILE_PX as u32,
                height: atlas::TILE_PX as u32,
                depth_or_array_layers: n_layers as u32,
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
                depth_or_array_layers: n_layers as u32,
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
        // 优先 MC ascii.png,失败回退程序化字体(见 font.rs)
        let (font_data, font_widths, _mc_font) = font::load_atlas(texture_pack_dir);
        font::install_widths(font_widths);
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

        // ---- GUI 精灵表(texturepack/gui/)-----------------------------
        // 缺素材时建 1x1 占位纹理,gui 字段为 None → 上层回退程序化绘制。
        let gui = texture_pack_dir.and_then(SpriteSheet::load);
        let (gui_rgba, gui_w, gui_h) = match &gui {
            Some(s) => (s.rgba.clone(), s.w, s.h),
            None => (vec![0u8; 4], 1, 1),
        };
        let gui_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui-sprites"),
            size: wgpu::Extent3d {
                width: gui_w,
                height: gui_h,
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
            let bpr = gui_w as usize * 4;
            // COPY_BUFFER_ALIGN = 256:staging 行需补齐到 256 字节
            let padded_bpr = bpr.div_ceil(256) * 256;
            let mut padded = vec![0u8; padded_bpr * gui_h as usize];
            for row in 0..gui_h as usize {
                let s = row * bpr..(row + 1) * bpr;
                let d = row * padded_bpr..row * padded_bpr + bpr;
                padded[d].copy_from_slice(&gui_rgba[s]);
            }
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gui-staging"),
                contents: &padded,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bpr as u32),
                        rows_per_image: Some(gui_h),
                    },
                },
                gui_tex.as_image_copy(),
                wgpu::Extent3d {
                    width: gui_w,
                    height: gui_h,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([enc.finish()]);
        }
        let gui_view = gui_tex.create_view(&wgpu::TextureViewDescriptor::default());
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        // unifont CJK 位图图集（text.rs 经 OnceLock 用同一份 cjk.f16 生成 quad）
        let unifont_view = {
            let mk_white = || {
                let t = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("unifont-placeholder"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    t.as_image_copy(),
                    &[255, 255, 255, 255],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                t.create_view(&wgpu::TextureViewDescriptor::default())
            };
            match crate::text::unifont_shared() {
                None => Some(mk_white()),
                Some(u) => {
                    let (rgba, w, h) = u.atlas_rgba();
                    let max = device.limits().max_texture_dimension_2d;
                    if w as u32 > max || h as u32 > max {
                        log::warn!("unifont atlas {w}x{h} > device max {max}, CJK disabled");
                        Some(mk_white())
                    } else {
                        let t = device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("unifont-atlas"),
                            size: wgpu::Extent3d {
                                width: w as u32,
                                height: h as u32,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });
                        queue.write_texture(
                            t.as_image_copy(),
                            rgba,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some((w * 4) as u32),
                                rows_per_image: Some(h as u32),
                            },
                            wgpu::Extent3d {
                                width: w as u32,
                                height: h as u32,
                                depth_or_array_layers: 1,
                            },
                        );
                        Some(t.create_view(&wgpu::TextureViewDescriptor::default()))
                    }
                }
            }
        };

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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&gui_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(
                        unifont_view.as_ref().expect("unifont view"),
                    ),
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

        // ---- player pipeline -------------------------------------------
        let player_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("player"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/player.wgsl").into()),
        });
        let player_bind_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("player-layout"),
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
                            // nearest 采样不要求 filterable float(sRGB array 在
                            // GLES 上也不满足 filterable 要求)
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                        count: None,
                    },
                ],
            });
        let player_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("player-pipeline-layout"),
            bind_group_layouts: &[Some(&player_bind_layout)],
            immediate_size: 0,
        });
        let player_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("player"),
            layout: Some(&player_layout),
            vertex: wgpu::VertexState {
                module: &player_mod,
                entry_point: Some("vs_player"),
                compilation_options: Default::default(),
                buffers: &[Some(player_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &player_mod,
                entry_point: Some("fs_player"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None, // cutout:shader 内 alpha discard
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                // player_mesh 的角点序经镜像变换(行列式 -1)后从盒外看为 CW,
                // 与 wgpu 默认 CCW 前置相反;若剔背面会只剩内壁。剔正面又会
                // 在 overlay discard 处透出内背壁。两难之下不剔(每盒 ≤24 三角,
                // 代价可忽略),由深度测试取胜者。
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let mesh = player_mesh::build_player_mesh();
        let player_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("player-uniforms"),
            size: size_of::<PlayerUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let player_vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("player-vbuf"),
            contents: bytemuck::cast_slice(&mesh.verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let player_ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("player-ibuf"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let player_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("player-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        // 皮肤未加载前的 1x1x2 全透明占位(draw_player 亦以 skins_loaded 短路)。
        let placeholder = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("player-skin-placeholder"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: SKIN_LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let player_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("player-bind-placeholder"),
            layout: &player_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: player_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&placeholder.create_view(
                        &wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            ..Default::default()
                        },
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&player_sampler),
                },
            ],
        });

        Self {
            device,
            queue,
            player_pipeline,
            player_bind_layout,
            player_uniform,
            player_vbuf,
            player_ibuf,
            player_bind,
            player_sampler,
            skins_loaded: false,
            terrain_pipeline,
            water_pipeline,
            sky_pipeline,
            hud_pipeline,
            frame_buf,
            origins_buf,
            sky_buf,
            hud_uniform,
            hud_vbuf,
            hud_ibuf,
            frame_bind,
            hud_bind,
            sky_bind,
            gui,
            max_chunks,
            max_hud_quads,
        }
    }

    /// MC GUI 精灵表;None 表示 texturepack 未带 gui/ 素材,上层应回退
    /// 程序化绘制。
    pub fn gui(&self) -> Option<&SpriteSheet> {
        self.gui.as_ref()
    }

    /// 上传 steve/alex 皮肤为 64x64x2 texture_2d_array(layer 0=steve,
    /// 1=alex,与 PlayerVertex.meta.x 约定一致)。可在任意时刻调用(重建 bind group)。
    pub fn load_skins(&mut self, steve_png: &[u8], alex_png: &[u8]) -> Result<(), String> {
        let decode = |png: &[u8], name: &str| -> Result<Vec<u8>, String> {
            let img = image::load_from_memory(png)
                .map_err(|e| format!("{name} png: {e}"))?
                .to_rgba8();
            if img.width() != 64 || img.height() != 64 {
                return Err(format!(
                    "{name}: 需要 64x64 皮肤,得到 {}x{}",
                    img.width(),
                    img.height()
                ));
            }
            Ok(img.into_raw())
        };
        let mut data = decode(steve_png, "steve")?;
        data.extend_from_slice(&decode(alex_png, "alex")?);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("player-skins"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: SKIN_LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            tex.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64 * 4), // 256B 对齐,无需 padding
                rows_per_image: Some(64),
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: SKIN_LAYERS,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        self.player_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("player-bind"),
            layout: &self.player_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.player_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.player_sampler),
                },
            ],
        });
        self.skins_loaded = true;
        Ok(())
    }

    /// 皮肤是否已就绪(app.rs 可用 `player_mesh::update_walk_animation` 等
    /// 计算姿态,不必在皮肤缺失时白白计算矩阵)。
    pub fn has_skins(&self) -> bool {
        self.skins_loaded
    }

    /// 在(带 Depth24Plus 深度附件的)world pass 内绘制一次玩家。几何同时含
    /// steve/alex 两套顶点(meta.x 选皮肤层),`skin_layer` 选对应索引区间;
    /// `part_models` 来自 `player_mesh::model_matrices(&pose)`。
    /// 皮肤未加载时为 no-op。
    pub fn draw_player(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        view_proj: [[f32; 4]; 4],
        part_models: &[glam::Mat4; PART_COUNT],
        skin_layer: u32,
    ) {
        if !self.skins_loaded || skin_layer >= SKIN_LAYERS {
            return;
        }
        let mut models = [[[0f32; 4]; 4]; PART_COUNT];
        for (m, dst) in part_models.iter().zip(models.iter_mut()) {
            *dst = m.to_cols_array_2d();
        }
        let u = PlayerUniforms { view_proj, models };
        self.queue
            .write_buffer(&self.player_uniform, 0, bytemuck::bytes_of(&u));
        pass.set_pipeline(&self.player_pipeline);
        pass.set_bind_group(0, &self.player_bind, &[]);
        pass.set_vertex_buffer(0, self.player_vbuf.slice(..));
        pass.set_index_buffer(self.player_ibuf.slice(..), wgpu::IndexFormat::Uint32);
        // 整款一次 draw:shader 按 meta.y 逐顶点取模型矩阵、meta.x 取皮肤层。
        pass.draw_indexed(0..(PART_COUNT * player_mesh::PART_INDEXES) as u32, 0, 0..1);
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

            // clouds: sky 之后、不透明之前（26.1 RenderPassOrder）
            if let Some((clouds, settings)) = scene.cloud {
                clouds.draw(&mut pass, &vp.to_cols_array_2d(), eye, scene.time, settings);
            }

            // opaque
            pass.set_pipeline(&self.terrain_pipeline);
            for (slot, rc) in &visible {
                let off = *slot * 256;
                pass.set_bind_group(0, &self.frame_bind, &[off]);
                pass.set_vertex_buffer(0, rc.vertex_buf.slice(..));
                pass.set_index_buffer(rc.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                if !rc.opaque_range.is_empty() {
                    pass.draw_indexed(rc.opaque_range.clone(), 0, 0..1);
                }
            }

            // player: 不透明地形后、水前（entity 在 translucent 之前渲染）
            if let Some((models, skin)) = scene.player {
                self.draw_player(&mut pass, vp.to_cols_array_2d(), models, skin);
            }

            // water: far to near
            pass.set_pipeline(&self.water_pipeline);
            let mut water: Vec<(f32, u32, &RenderChunk)> = visible
                .iter()
                .filter(|(_, rc)| rc.water_index_buf.is_some() && !rc.water_range.is_empty())
                .map(|(slot, rc)| {
                    let c = Vec3::from(rc.origin) + Vec3::new(8.0, 0.0, 8.0);
                    (c.distance_squared(eye), *slot, *rc)
                })
                .collect();
            water.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, slot, rc) in water {
                pass.set_bind_group(0, &self.frame_bind, &[slot * 256]);
                pass.set_vertex_buffer(0, rc.vertex_buf.slice(..));
                pass.set_index_buffer(
                    rc.water_index_buf.as_ref().unwrap().slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(rc.water_range.clone(), 0, 0..1);
            }
        }

        // HUD pass
        if !scene.hud.is_empty() {
            // 防溢出:菜单铺贴 quad 数量超预期时截断并告警,而不是 panic
            let hud: &[HudQuad] = if scene.hud.len() as u32 > self.max_hud_quads {
                log::warn!(
                    "hud: {} quads > max {}, truncating",
                    scene.hud.len(),
                    self.max_hud_quads
                );
                &scene.hud[..self.max_hud_quads as usize]
            } else {
                scene.hud
            };
            let (verts, indices) = flatten_hud(hud);
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
        // rot != 0 时绕 quad 中心旋转(splash 文字);常规 quad 走直线分支
        let (cos, sin) = if q.rot == 0.0 {
            (1.0, 0.0)
        } else {
            (q.rot.cos(), q.rot.sin())
        };
        let hw = q.w * 0.5;
        let hh = q.h * 0.5;
        for (ox, oy, uv) in [
            (-hw, -hh, q.uv[0]),
            (hw, -hh, [q.uv[1][0], q.uv[0][1]]),
            (hw, hh, q.uv[1]),
            (-hw, hh, [q.uv[0][0], q.uv[1][1]]),
        ] {
            verts.push(HudVertex {
                pos: [
                    q.x + hw + ox * cos - oy * sin,
                    q.y + hh + ox * sin + oy * cos,
                ],
                uv,
                color: c,
                src: [q.tex, q.layer],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, indices)
}
