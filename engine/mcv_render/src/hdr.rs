//! HDR 管线（`hdr` feature，默认关；#223 续）：RGBA16F 主纹理 +
//! ACES tone mapping 独立后处理 pass。
//!
//! 蓝图锚（bevy 0.20 bevy_core_pipeline）：
//! - tonemapping/node.rs:46-47：跑 tone mapping 的 view 主纹理恒
//!   `Rgba16Float`，pass 读主纹理、渲染到交换链视图；
//! - tonemapping_frag.wesl:22-47：全屏采样 → tone_mapping → 输出；
//! - tonemapping.wesl:112-140：ACESFitted（hdr.wgsl 同源照抄）；
//! - bevy_camera/components.rs:93：hdr 相机 = `Rgba16Float` 主纹理 +
//!   一个全屏 pass。
//!
//! 本仓接线（对应于 bevy graph 的降维直写版）：
//! 1. `hdr` feature 同时在 mcv_app 开启且运行期后端可用时，
//!    [`is_hdr_capable`] 为桌面 非 GLES 后端 真——GLES（真机 Mali）
//!    恒走现状 Rgba8UnormSrgb 链（「GLES 保持现状」红线）；
//! 2. `Renderer` 管线以 `Rgba16Float` 目标构建（`Renderer::new
//!    (color_format)` 形参直配，默认路 = surface 格式不变）；
//! 3. `draw_frame` 画进 [`HdrStage::color_view`]（HDR 主纹理），
//!    [`HdrStage::tonemap`] 读它渲染到交换链视图（sRGB 交换链做硬件
//!    sRGB 编码，ACES 输出大错误控制在线性显示段）。
//!
//! 曝光：[`HdrStage::set_exposure`] 直取 `Camera::exposure`（线性乘子，
//! 默认 1.0 = 行为不变；bevy `2^exposure` stops 语义为有意偏差）。

use wgpu::util::DeviceExt;

/// HDR 主纹理格式（bevy tonemapping/node.rs:47 同款）。
pub const HDR_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// 后端可用性判定：GLES/Noop 恒 false（真机 Mali 与软件 GL 保现状链，
/// Android native-activity 无 Vulkan 设备直接回退）。
pub fn is_hdr_capable(backend: wgpu::Backend) -> bool {
    !matches!(backend, wgpu::Backend::Gl | wgpu::Backend::Noop)
}

/// tone mapping 全屏 pass 的帧常量（hdr.wgsl TonemapUniform 同构，16B）：
/// x = 曝光线性乘子，yzw = 自由。
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TonemapUniform {
    params: [f32; 4],
}

const _: () = assert!(size_of::<TonemapUniform>() == 16);

/// HDR 中间目标 + tone mapping 管线。持有 Rgba16Float 主纹理、
/// 全屏三角管线（目标格式 = 交换链格式）与 uniform（exposure）。
/// resize 走 [`HdrStage::resize`] 重建纹理（管线/布局照旧复用）。
pub struct HdrStage {
    hdr_color: wgpu::Texture,
    hdr_view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    extent: wgpu::Extent3d,
}

impl HdrStage {
    /// 建纹理 + 管线 + bind group。`out_format` = 交换链格式
    /// （tone map 输出的 sRGB 硬件编码就发生在该目标上）。
    pub fn new(
        device: &wgpu::Device,
        out_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let hdr_color = make_hdr_texture(device, width, height);
        let hdr_view = hdr_color.create_view(&wgpu::TextureViewDescriptor::default());
        // 曝光初值 1.0（默认行为不变契约；首个 draw 前由 set_exposure 覆写）。
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hdr-tonemap-uniform"),
            contents: bytemuck::bytes_of(&TonemapUniform {
                params: [1.0, 0.0, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hdr-tonemap-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<TonemapUniform>() as u64),
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
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hdr-tonemap-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hdr-tonemap"),
            source: wgpu::ShaderSource::Wgsl(crate::pbr::HDR_WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hdr-tonemap-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: out_format,
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
        let bind = create_bind(device, &bind_layout, &uniform, &hdr_view);
        Self {
            hdr_color,
            hdr_view,
            uniform,
            bind,
            bind_layout,
            pipeline,
            extent: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        }
    }

    /// HDR 主纹理视图（`Renderer::draw_frame` 的 color 目标）。
    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.hdr_view
    }

    /// HDR 主纹理尺寸（与交换链同，resize 时同步）。
    pub fn extent(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }

    /// HDR 主纹理（测试 seam：写入已知 HDR 值做端到端离线对拍；
    /// 生产路径只经 render pass 写入）。
    pub fn hdr_texture(&self) -> &wgpu::Texture {
        &self.hdr_color
    }

    /// 曝光（线性乘子；取 `Camera::exposure`，默认 1.0 = 恒等不提亮；
    /// 负输入截 0）。
    pub fn set_exposure(&self, queue: &wgpu::Queue, exposure: f32) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&TonemapUniform {
                params: [exposure.max(0.0), 0.0, 0.0, 0.0],
            }),
        );
    }

    /// 尺寸变化时重建主纹理（bind group 的纹理视图必须随换）。
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 || (self.extent.width == width && self.extent.height == height)
        {
            return;
        }
        self.hdr_color = make_hdr_texture(device, width, height);
        self.hdr_view = self
            .hdr_color
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.bind = create_bind(device, &self.bind_layout, &self.uniform, &self.hdr_view);
        self.extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
    }

    /// tone map 编码（不 submit——调用方与 present 顺序自控/测试用）。
    pub fn encode_tonemap(&self, encoder: &mut wgpu::CommandEncoder, out_view: &wgpu::TextureView) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hdr-tonemap-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind, &[]);
        rpass.draw(0..3, 0..1);
    }

    /// 一步到位：编码 + submit（与 draw_frame 的 submit 同 queue 顺序，
    /// 跨 submit 的纹理读写依赖由 wgpu 跟踪）。
    pub fn tonemap(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        out_view: &wgpu::TextureView,
    ) {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hdr-tonemap-encoder"),
        });
        self.encode_tonemap(&mut enc, out_view);
        queue.submit([enc.finish()]);
    }
}

fn make_hdr_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr-color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn create_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("hdr-tonemap-sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hdr-tonemap-bind"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}
