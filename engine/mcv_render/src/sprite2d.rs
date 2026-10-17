//! 2D 精灵批次（#222 批 A；bevy_sprite 对等）。
//!
//! 模式取自 bevy_sprite：[`Anchor`] = 归一化枢轴（`sprite.rs:306-331`，
//! 9 个命名常量 + Vec2 互换，y 向下屏帧语义翻转）；布局语义只取「全 tile
//! 直通」量级，不搬纹理管理（bevy texture_slice/TextureAtlas 不引入）。
//!
//! 管线形态：正交像素空间 + **instanced quad**（静态 4 角 + 每实例
//! [`SpriteInstance`]）+ 复用 MANIFEST_LAYERS 方块图集数组通道（terrain
//! views / hud sampler / hud uniform 全复用，`sample_terrain_sprite` 由
//! gpu.rs [`atlas_arrays_wgsl`](crate::gpu) 烘入 sprite2d.wgsl 占位）。
//! 收集 [`SpriteCmd`] → **同 key（blending 预设）连续合批** → 每 key 组一次
//! instanced draw（组内 layer 逐实例携带，同层不拆 draw；HUD 图标全组同
//! blending ⇒ 单批一次 draw）。混合/丢弃/乘 tint 与 fs_hud 逐字同式，接线面
//! 像素等价（[`sprite_vertices`] 测试锁）。
//!
//! GLES/wgpu30 纪律（同 gpu.rs/particle_renderer.rs）：实例缓冲
//! `mapped_at_creation: false`，逐帧 `queue.write_buffer` 写未映射 buffer。

use wgpu::util::DeviceExt;

/// 单帧实例上限（对齐 gpu.rs `max_hud_quads` 量级；超出截断 + 告警一次）。
pub const MAX_SPRITES: u32 = 4096;

/// bevy Anchor 模式：归一化（相对自身尺寸）枢轴偏移，y 向下屏帧。
/// 9 常量的元组值与 bevy `sprite.rs:307-314` 一致（bevy y 向上，此处
/// 语义翻转：bevy `BOTTOM_LEFT(-0.5,-0.5)` 元组在 y 向下帧即左上角）。
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Anchor(pub [f32; 2]);

impl Anchor {
    pub const TOP_LEFT: Self = Self([-0.5, -0.5]);
    pub const TOP_CENTER: Self = Self([0.0, -0.5]);
    pub const TOP_RIGHT: Self = Self([0.5, -0.5]);
    pub const CENTER_LEFT: Self = Self([-0.5, 0.0]);
    pub const CENTER: Self = Self([0.0, 0.0]);
    pub const CENTER_RIGHT: Self = Self([0.5, 0.0]);
    pub const BOTTOM_LEFT: Self = Self([-0.5, 0.5]);
    pub const BOTTOM_CENTER: Self = Self([0.0, 0.5]);
    pub const BOTTOM_RIGHT: Self = Self([0.5, 0.5]);

    /// bevy `Anchor::as_vec` 对应（元组形式）。
    pub fn as_vec(&self) -> [f32; 2] {
        self.0
    }
}

impl From<[f32; 2]> for Anchor {
    fn from(v: [f32; 2]) -> Self {
        Self(v)
    }
}

/// 一条精灵命令（CPU 收集面）。`pos` = 锚点（枢轴）屏幕像素坐标，
/// `layer` = 方块图集数组层号（MANIFEST_LAYERS 通道），`rot` 绕锚点
/// （弧度），`tint` RGBA 乘色，`anchor` = 枢轴归一化偏移。
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct SpriteCmd {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub layer: u32,
    pub rot: f32,
    pub tint: [f32; 4],
    pub anchor: Anchor,
}

impl SpriteCmd {
    /// HUD 物品图标形态：左上锚定、不旋转、白色 tint——与
    /// `text::tile_icon`（mcv_render::text）签名/语义逐参对齐，
    /// 接线面等价测试（`sprite_vertices` vs flatten_hud 公式）锁此点。
    pub fn icon(layer: u16, x: f32, y: f32, size: f32) -> Self {
        Self {
            pos: [x, y],
            size: [size, size],
            layer: u32::from(layer),
            rot: 0.0,
            tint: [1.0, 1.0, 1.0, 1.0],
            anchor: Anchor::TOP_LEFT,
        }
    }
}

/// GPU 实例数据（48 B：pos/size/anchor @0/8/16，rot @24、layer @28、
/// tint @32（16B 对齐自然落位，无 pad））。
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct SpriteInstance {
    pos: [f32; 2],
    size: [f32; 2],
    anchor: [f32; 2],
    rot: f32,
    layer: u32,
    tint: [f32; 4],
}

impl From<&SpriteCmd> for SpriteInstance {
    fn from(c: &SpriteCmd) -> Self {
        Self {
            pos: c.pos,
            size: c.size,
            anchor: c.anchor.0,
            rot: c.rot,
            layer: c.layer,
            tint: c.tint,
        }
    }
}

pub const SPRITE_INSTANCE_STRIDE: usize = 48;

const _: () = assert!(size_of::<SpriteInstance>() == SPRITE_INSTANCE_STRIDE);

/// CPU 镜像（sprite2d.wgsl `vs_sprite2d` 同式）：命令 → 世界像素四角
/// （TL,TR,BR,BL，y 向下）。rot=0 对任意 Anchor 与 hud flatten_hud 公式
/// 逐顶点重合；rot≠0 仅 Anchor=CENTER 时重合（旋转围绕锚点 vs 围绕中心，
/// 见接线面等价测试的负例锁）。
pub fn sprite_vertices(cmd: &SpriteCmd) -> [[f32; 2]; 4] {
    let (cos, sin) = (cmd.rot.cos(), cmd.rot.sin());
    for_corner(|c| {
        let local = [c[0] - cmd.anchor.0[0], c[1] - cmd.anchor.0[1]];
        let local = [local[0] * cmd.size[0], local[1] * cmd.size[1]];
        [
            cmd.pos[0] + local[0] * cos - local[1] * sin,
            cmd.pos[1] + local[0] * sin + local[1] * cos,
        ]
    })
}

/// CPU 镜像（sprite2d.wgsl `vs_sprite2d` NDC 同式）：像素 → 裁剪空间。
/// 端点：左上 (0,0)→(−1,+1)，右下 (w,h)→(+1,−1)，中心→(0,0)。
pub fn ndc_of(px: f32, py: f32, screen_w: f32, screen_h: f32) -> [f32; 2] {
    [px / screen_w * 2.0 - 1.0, 1.0 - py / screen_h * 2.0]
}

/// 单位 quad 角序（与 flatten_hud 的 (−hw,−hh),(+hw,−hh),(+hw,+hh),(−hw,+hh)
/// 及 UV 行序一致）。
const QUAD_CORNERS: [[f32; 2]; 4] = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];

fn for_corner(mut f: impl FnMut([f32; 2]) -> [f32; 2]) -> [[f32; 2]; 4] {
    let mut out = [[0.0; 2]; 4];
    for (i, c) in QUAD_CORNERS.iter().enumerate() {
        out[i] = f(*c);
    }
    out
}

/// 合批 key（blending 预设）。当前唯一预设 = 半透明（与 hud 管线
/// `BlendState::ALPHA_BLENDING` 一致）；未来预设扩枚举，层号**不**参与
/// key（逐实例携带，同层不拆 draw）。
pub type BlendKey = u32;

/// 唯一预设：半透明（同层同 blending 合批的 blending 分量）。
pub const BLEND_ALPHA: BlendKey = 0;

/// 一个合批组：连续实例区间（first..first+count）→ 一次 instanced draw。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpriteGroup {
    pub key: BlendKey,
    pub first: u32,
    pub count: u32,
}

/// 合批规划：按 key **连续游程**切段（不重排——插入序 = 画家序，与
/// scene.hud 同语义）。同 key 相邻合并；截断 [`MAX_SPRITES`]。
pub fn plan_keyed(cmds: impl ExactSizeIterator<Item = (BlendKey, SpriteCmd)>) -> Vec<SpriteGroup> {
    let mut groups: Vec<SpriteGroup> = Vec::new();
    for (i, (key, _)) in cmds.enumerate() {
        if i as u64 >= MAX_SPRITES as u64 {
            break;
        }
        match groups.last_mut() {
            Some(g) if g.key == key => g.count += 1,
            _ => groups.push(SpriteGroup {
                key,
                first: i as u32,
                count: 1,
            }),
        }
    }
    groups
}

/// 默认规划：全部命令同 key（HUD 图标路径即此形态）⇒ 单组一次 draw。
pub fn plan(cmds: &[SpriteCmd]) -> Vec<SpriteGroup> {
    plan_keyed(cmds.iter().map(|&c| (BLEND_ALPHA, c)))
}

/// HUD 收集产物：quad 列表 + 精灵命令 + 栅栏（精灵画在 hud[fence] 之前，
/// 保证计数文字/耐久条仍在图标之上——画家序与旧通路逐字节同序）。
#[derive(Default)]
pub struct HudBatch {
    pub quads: Vec<crate::gpu::HudQuad>,
    pub sprites: Vec<SpriteCmd>,
    pub sprite_fence: u32,
}

/// 精灵批次渲染器：管线 + 静态 quad/索引 + 实例缓冲。
pub struct SpriteBatch {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    quad_buf: wgpu::Buffer,
    idx_buf: wgpu::Buffer,
    inst_buf: wgpu::Buffer,
    array_count: usize,
    warned_truncation: bool,
}

impl SpriteBatch {
    /// 建管线。`src` = sprite2d.wgsl 全文（gpu.rs 已烘入图集数组占位），
    /// `color_format` = 目标色面（与 hud 管线同源）。管线无深度（HUD pass
    /// depth_stencil: None 同款），半透明混合、双面（cull None）。
    /// `array_count` = 烘入的图集数组数（与 atlas_arrays_wgsl 的 bindings
    /// 表同源——布局项必须恰好覆盖 shader 声明，多/少均过不了 wgpu 校验）。
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        src: &str,
        array_count: usize,
    ) -> Self {
        assert!(array_count >= 1 && array_count <= 4, "图集数组 1..=4");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite2d"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        // 数组声明 + 采样函数由 atlas_arrays_wgsl 烘入占位（binding 1..=4
        // 逐数组，sampler 5）——布局项与烘入表同源，永不错位。
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        for b in 1..=array_count as u32 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: b,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            });
        }
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 5,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite2d-layout"),
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite2d-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite2d"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_sprite2d"),
                compilation_options: Default::default(),
                buffers: &[Some(instance_vertex_layout()), Some(quad_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_sprite2d"),
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

        // 静态单位 quad（TL,TR,BR,BL + UV）+ 顺序索引（quad 展开同 flatten_hud）。
        // 打包成 f32x4（corner.xy + uv.xy，stride 16B）——Pod 数组，可 cast_slice。
        let quads: [[f32; 4]; 4] = [
            [-0.5, -0.5, 0.0, 0.0],
            [0.5, -0.5, 1.0, 0.0],
            [0.5, 0.5, 1.0, 1.0],
            [-0.5, 0.5, 0.0, 1.0],
        ];
        let quad_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sprite2d-quad"),
            contents: bytemuck::cast_slice(&quads),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let idx_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sprite2d-idx"),
            contents: bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            usage: wgpu::BufferUsages::INDEX,
        });
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite2d-inst"),
            size: (MAX_SPRITES as u64) * SPRITE_INSTANCE_STRIDE as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_layout,
            quad_buf,
            idx_buf,
            inst_buf,
            array_count,
            warned_truncation: false,
        }
    }

    /// 建 bind group：uniform 复用 gpu.rs hud uniform（screen vec4 同布局），
    /// 图集数组复用 terrain views（MANIFEST_LAYERS 通道），sampler 复用
    /// hud 最近邻采样器（16px tile 1:1 采样，与图标通路逐像素同源）。
    pub fn build_bind_group(
        &self,
        device: &wgpu::Device,
        hud_uniform: &wgpu::Buffer,
        terrain_views: &[wgpu::TextureView],
        hud_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        assert!(
            terrain_views.len() >= self.array_count,
            "terrain 数组数 {} < 布局数组数 {}",
            terrain_views.len(),
            self.array_count
        );
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: hud_uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(hud_sampler),
            },
        ];
        // 数组绑定 1..=4（与 sprite2d.wgsl 占位烘入的 bindings 表一致）。
        for (i, view) in terrain_views.iter().enumerate().take(self.array_count) {
            entries.push(wgpu::BindGroupEntry {
                binding: 1 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite2d-bind"),
            layout: &self.bind_layout,
            entries: &entries,
        })
    }

    /// 收集 → 合批 → 绘制。组数 = key 游程数（HUD 图标单 key ⇒ 一次 draw）。
    /// 实例数据逐帧 `queue.write_buffer`（未映射 buffer，GLES 合规）。
    /// screen uniform 由调用方写入（与 hud 复用同一 16B buffer，见 gpu.rs）。
    pub fn draw(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        bind: &wgpu::BindGroup,
        queue: &wgpu::Queue,
        cmds: &[SpriteCmd],
    ) {
        if cmds.is_empty() {
            return;
        }
        let truncated = cmds.len() > MAX_SPRITES as usize;
        let cmds = if truncated {
            &cmds[..MAX_SPRITES as usize]
        } else {
            cmds
        };
        if truncated && !self.warned_truncation {
            log::warn!(
                "sprite2d: {} cmds > cap {MAX_SPRITES}, truncating (once)",
                cmds.len()
            );
            self.warned_truncation = true;
        }
        let instances: Vec<SpriteInstance> = cmds.iter().map(SpriteInstance::from).collect();
        queue.write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&instances));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.set_vertex_buffer(
            0,
            self.inst_buf
                .slice(..(instances.len() * SPRITE_INSTANCE_STRIDE) as u64),
        );
        pass.set_vertex_buffer(1, self.quad_buf.slice(..));
        pass.set_index_buffer(self.idx_buf.slice(..), wgpu::IndexFormat::Uint32);
        for g in plan(cmds) {
            pass.draw_indexed(0..6, 0, g.first..g.first + g.count);
        }
    }
}

/// 实例步进布局（slot 0）：pos/size/anchor f32x2、rot f32、layer u32、
/// tint f32x4 @32（16B 对齐），stride 48。
fn instance_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: SPRITE_INSTANCE_STRIDE as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
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
                format: wgpu::VertexFormat::Float32x2,
                offset: 16,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 24,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 28,
                shader_location: 4,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 32,
                shader_location: 5,
            },
        ],
    }
}

/// 静态 quad 步进布局（slot 1）：corner f32x2 @0、uv f32x2 @8，stride 16。
fn quad_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 16,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 6,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 7,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anchor 模式（bevy sprite.rs:307-314 元组值；y 向下语义命名）：
    /// 命名常量端点 + 自定义互换。
    #[test]
    fn anchor_constants_and_conversion() {
        assert_eq!(Anchor::TOP_LEFT.0, [-0.5, -0.5]);
        assert_eq!(Anchor::CENTER.0, [0.0, 0.0]);
        assert_eq!(Anchor::BOTTOM_RIGHT.0, [0.5, 0.5]);
        assert_eq!(Anchor::CENTER.as_vec(), [0.0, 0.0]);
        let custom: Anchor = [0.25, -0.75].into();
        assert_eq!(custom.as_vec(), [0.25, -0.75]);
        // 归一化域：命名枢轴落在 ±0.5 内。
        for a in [
            Anchor::TOP_LEFT,
            Anchor::TOP_CENTER,
            Anchor::TOP_RIGHT,
            Anchor::CENTER_LEFT,
            Anchor::CENTER,
            Anchor::CENTER_RIGHT,
            Anchor::BOTTOM_LEFT,
            Anchor::BOTTOM_CENTER,
            Anchor::BOTTOM_RIGHT,
        ] {
            assert!(a.0[0] >= -0.5 && a.0[0] <= 0.5);
            assert!(a.0[1] >= -0.5 && a.0[1] <= 0.5);
        }
    }

    /// 矩阵端点：正交像素→NDC 与 sprite2d.wgsl vs_sprite2d 同式——
    /// 左上原点 (0,0)→(−1,+1)，右下 (w,h)→(+1,−1)，中心→(0,0)，
    /// 左下 (0,h)→(−1,−1)，右上 (w,0)→(+1,+1)。
    #[test]
    fn ortho_matrix_endpoints() {
        let (w, h) = (640.0f32, 480.0f32);
        assert_eq!(ndc_of(0.0, 0.0, w, h), [-1.0, 1.0]);
        assert_eq!(ndc_of(w, h, w, h), [1.0, -1.0]);
        assert_eq!(ndc_of(0.0, h, w, h), [-1.0, -1.0]);
        assert_eq!(ndc_of(w, 0.0, w, h), [1.0, 1.0]);
        let c = ndc_of(w * 0.5, h * 0.5, w, h);
        assert!(c[0].abs() < 1e-6 && c[1].abs() < 1e-6);
    }

    /// 合批分组：同 key 连续合并；key 变化切段；插入序（画家序）保持；
    /// 层号不拆组（逐实例携带）；HUD 形态（全同 key）= 单组 = 一次 draw；
    /// 空批无组；超上限截断。
    #[test]
    fn batching_groups_by_key_runs() {
        // 空批。
        assert!(plan(&[]).is_empty());
        // HUD 图标形态：同 blending ⇒ 一组覆盖全部（「一次 draw」）。
        let icons: Vec<SpriteCmd> = (0..9u16)
            .map(|i| SpriteCmd::icon(i * 3, i as f32 * 20.0, 100.0, 16.0))
            .collect();
        let g = plan(&icons);
        assert_eq!(g.len(), 1, "同 blending 全量合批 = 单组单 draw");
        assert_eq!((g[0].first, g[0].count), (0, 9));
        // 显式 key 游程：[1, 0, 0, 1] → 3 组，区间与顺序锁死。
        let mk = |i: usize| SpriteCmd::icon(i as u16, 0.0, 0.0, 8.0);
        let keyed: Vec<(BlendKey, SpriteCmd)> =
            vec![(1, mk(0)), (0, mk(1)), (0, mk(2)), (1, mk(3))];
        let g = plan_keyed(keyed.into_iter());
        assert_eq!(g.len(), 3);
        assert_eq!(
            g[0],
            SpriteGroup {
                key: 1,
                first: 0,
                count: 1
            }
        );
        assert_eq!(
            g[1],
            SpriteGroup {
                key: 0,
                first: 1,
                count: 2
            }
        );
        assert_eq!(
            g[2],
            SpriteGroup {
                key: 1,
                first: 3,
                count: 1
            }
        );
        // 截断：> MAX_SPRITES 的命令不计组。
        let many: Vec<(BlendKey, SpriteCmd)> = (0..(MAX_SPRITES + 10) as usize)
            .map(|i| (i as BlendKey, mk(i)))
            .collect();
        let total: u32 = plan_keyed(many.into_iter()).iter().map(|g| g.count).sum();
        assert_eq!(total, MAX_SPRITES, "超上限命令被截断");
    }

    /// 接线面等价：`SpriteCmd::icon`（tile_icon 同参）的 CPU 端点四角与
    /// gpu.rs flatten_hud 公式（rot=0 直线分支：左上角 + 尺寸展开）逐顶点
    /// 重合——接线改走 instanced 批后像素不变的正向锁。
    #[test]
    fn wiring_face_matches_hud_flatten() {
        // flatten_hud 公式（gpu.rs:3885-3906，rot=0 分支）本地重放。
        let hud_corners =
            |x: f32, y: f32, w: f32, h: f32| [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        for &(x, y, s) in &[(8.0f32, 40.0, 16.0), (320.5, 221.0, 32.0), (0.0, 0.0, 1.0)] {
            let cmd = SpriteCmd::icon(7, x, y, s);
            assert_eq!(sprite_vertices(&cmd), hud_corners(x, y, s, s));
        }
        // UV/层/色面：icon 形态 = 全 tile（与 tile_icon 的 uv [[0,0],[1,1]]
        // 角展开一致）、白 tint、层透传。
        let cmd = SpriteCmd::icon(42, 0.0, 0.0, 16.0);
        assert_eq!(cmd.layer, 42);
        assert_eq!(cmd.tint, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(cmd.anchor, Anchor::TOP_LEFT);
        // 逐实例 POD 往返（GLES 上传面）。
        let inst = SpriteInstance::from(&cmd);
        let bytes = bytemuck::bytes_of(&inst);
        assert_eq!(bytes.len(), SPRITE_INSTANCE_STRIDE);
        let back: SpriteInstance = *bytemuck::from_bytes(bytes);
        assert_eq!(back, inst);
    }

    /// 旋转域（诚实域界）：rot≠0 时 sprite2d 绕**锚点**转（bevy 语义），
    /// flatten_hud 绕**中心**转——Anchor=CENTER 时两者重合（此时等价覆盖
    /// 全 rot）；非 CENTER + rot≠0 必须分叉（接线只用 rot=0，此处锁差异
    /// 存在，防止误把「全 rot 等价」说成事实）。
    #[test]
    fn rotation_domain_center_anchor_matches_flatten() {
        let hud_corners_center_rot = |cx: f32, cy: f32, w: f32, h: f32, rot: f32| {
            let (cos, sin) = (rot.cos(), rot.sin());
            let hw = w * 0.5;
            let hh = h * 0.5;
            [
                ([-hw, -hh], [0.0, 0.0]),
                ([hw, -hh], [1.0, 0.0]),
                ([hw, hh], [1.0, 1.0]),
                ([-hw, hh], [0.0, 1.0]),
            ]
            .map(|(o, _uv)| [cx + o[0] * cos - o[1] * sin, cy + o[0] * sin + o[1] * cos])
        };
        let (w, h, rot) = (16.0f32, 16.0, std::f32::consts::FRAC_PI_2);
        let center_cmd = SpriteCmd {
            pos: [100.0, 100.0],
            size: [w, h],
            layer: 1,
            rot,
            tint: [1.0; 4],
            anchor: Anchor::CENTER,
        };
        assert_eq!(
            sprite_vertices(&center_cmd),
            hud_corners_center_rot(100.0, 100.0, w, h, rot),
            "Anchor=CENTER 时全 rot 与 flatten_hud 中心旋转重合"
        );
        let topleft_cmd = SpriteCmd {
            pos: [100.0, 100.0],
            size: [w, h],
            layer: 1,
            rot,
            tint: [1.0; 4],
            anchor: Anchor::TOP_LEFT,
        };
        assert_ne!(
            sprite_vertices(&topleft_cmd),
            hud_corners_center_rot(100.0, 100.0, w, h, rot),
            "非 CENTER 枢轴 + rot≠0 与 flatten_hud 中心旋转分叉（域界锁）"
        );
    }
}
