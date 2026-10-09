//! GPU renderer: pipelines, bind groups, per-frame draw orchestration.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::celestial;
use crate::font;
use crate::frustum::Frustum;
use crate::gui::SpriteSheet;
use crate::particle_renderer::ParticleRenderer;
use crate::particles::ParticleEngine;
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
    /// 生物群系染色基色（26.1 ColorResolver 机制）：xyz = plains 草色
    /// （sRGB 0..1，来自 colormap/grass.png 温度×湿度查表），w = 染色开关
    /// （colormap 素材缺失时 0 → 不染色，不伪造颜色）。
    pub tint_grass: [f32; 4],
    /// xyz = plains 叶色（colormap/foliage.png 查表），w = 自由。
    pub tint_foliage: [f32; 4],
}

const _: () = assert!(size_of::<FrameUniforms>() == 144);

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
}

impl MeshUploader {
    /// 只需 device：网格经创建期映射视图写入，不占队列写带宽。
    pub fn new(device: wgpu::Device) -> Self {
        Self { device }
    }

    fn vertex_index(&self, v: &[u8], i: &[u32]) -> (wgpu::Buffer, wgpu::Buffer) {
        let vb = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-vb"),
            // 映射视图要求长度是 4 的倍数且非空（wgpu 30 MapRangeError）。
            size: (v.len() as u64).next_multiple_of(4).max(4),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        // 创建期映射的 buffer 必须经 mapped slice 写入，不能 queue.write_buffer：
        // Vulkan 容忍但 GLES hal 直接报 "Buffer is expected to be unmapped"
        // （Android 真机走 GL 回退时进世界首个网格上传即 fatal，2026-10-10）。
        {
            let mut view = vb.slice(..).get_mapped_range_mut().expect("vb mapped");
            view.slice(0..v.len()).copy_from_slice(v);
        }
        vb.unmap();
        let ib = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-ib"),
            size: (i.len() as u64 * 4).next_multiple_of(4).max(4),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        let ib_bytes = bytemuck::cast_slice::<u32, u8>(i);
        {
            let mut view = ib.slice(..).get_mapped_range_mut().expect("ib mapped");
            view.slice(0..ib_bytes.len()).copy_from_slice(ib_bytes);
        }
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

/// 挖掘 overlay 的一面：暴露才画裂纹 quad；光照取相邻空气方块
/// （低 nibble=block、高 nibble=sky，与 mesher 面光照一致）。
#[derive(Clone, Copy, Debug)]
pub struct MineFace {
    pub exposed: bool,
    pub block_light: u8,
    pub sky_light: u8,
}

/// 选中/挖掘 overlay：`min` = 方块最小角世界坐标；`crack_stage` =
/// Some(0..=9) 时画裂纹层（CRACK_BASE+stage，原版 10 档
/// destroy_stage_0..9，MultiPlayerGameMode.java:551），描边始终画。
/// 面顺序与着色器 face_id 一致：+X,-X,+Y,-Y,+Z,-Z。
#[derive(Clone, Copy, Debug)]
pub struct MiningOverlay {
    pub min: [f32; 3],
    pub crack_stage: Option<u32>,
    pub faces: [MineFace; 6],
}

/// 裂纹 overlay 顶点：与 terrain 顶点逐字节同布局（TERRAIN_STRIDE）。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CrackVertex {
    pos: [f32; 3],
    uv: [u16; 2],
    layer: u16,
    block_light: u8,
    sky_light: u8,
    ao: u8,
    flags: u8,
    pad: [u8; 2],
}

const _: () = assert!(size_of::<CrackVertex>() == TERRAIN_STRIDE);

/// 面顶点模板（单位立方体局部坐标）与法线，索引 = face_id
/// （+X,-X,+Y,-Y,+Z,-Z，与 terrain.wgsl face_shade 表一致）。
const FACE_QUAD: [[[f32; 3]; 4]; 6] = [
    [
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ],
    [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0],
    ],
    [
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ],
    [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    ],
    [
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ],
    [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ],
];
const FACE_NORM: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];
const FACE_UV: [[u16; 2]; 4] = [[0, 0], [0, 65535], [65535, 65535], [65535, 0]];

/// 生成暴露面的裂纹 quad（局部坐标沿法线外偏 0.003，深度只读时防
/// z-fighting）。layer = CRACK_BASE + stage（stage 0..=9 已由 game 层
/// 按原版公式钳好）。
fn build_crack_overlay(ov: &MiningOverlay) -> (Vec<CrackVertex>, Vec<u32>) {
    let stage = ov
        .crack_stage
        .unwrap_or(0)
        .min((atlas::CRACK_LAYERS - 1) as u32);
    let layer = (atlas::CRACK_BASE + stage as usize) as u16;
    let mut verts = Vec::with_capacity(24);
    let mut idx = Vec::with_capacity(36);
    for (f, face) in ov.faces.iter().enumerate() {
        if !face.exposed {
            continue;
        }
        let base = verts.len() as u32;
        for c in 0..4 {
            let p = &FACE_QUAD[f][c];
            let n = &FACE_NORM[f];
            verts.push(CrackVertex {
                pos: [
                    p[0] + n[0] * 0.003,
                    p[1] + n[1] * 0.003,
                    p[2] + n[2] * 0.003,
                ],
                uv: FACE_UV[c],
                layer,
                block_light: face.block_light,
                sky_light: face.sky_light,
                ao: 3,
                flags: f as u8,
                pad: [0; 2],
            });
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, idx)
}

pub struct Scene<'a> {
    pub camera: &'a Camera,
    pub time: f32,
    /// 天空亮度系数（day.json sky_light_factor；天气混合由调用方算——
    /// WeatherAttributes SKY_LIGHT_FACTOR 向夜底 0.24 混）。
    pub day_factor: f32,
    pub sun_dir: Vec3,
    /// 天气雾色 RGB 乘子（AtmosphericFogEnvironment.applyWeatherDarken:50-62；
    /// 晴 = [1,1,1]）乘到地平线色上（天空 + 地形雾同源）。
    pub fog_tint: [f32; 3],
    /// 天气雾密度乘子（雾距收缩 AtmosphericFogEnvironment.java:70-73 的
    /// exp 雾等价映射；晴 = 1.0）。
    pub fog_density_mult: f32,
    /// 月相序号 0..=7（MoonPhase 序，`celestial::moon_phase` 由游戏时间算出）。
    pub moon_phase: u32,
    /// Render target size in pixels (HUD coordinate space).
    pub width: f32,
    pub height: f32,
    pub chunks: &'a [RenderChunk],
    pub hud: &'a [HudQuad],
    /// 云：(资源, 设置)；None 或 enabled=false 不画（天空后、地形前）。
    pub cloud: Option<(&'a crate::Clouds, crate::CloudSettings)>,
    /// 玩家模型：(12 部位模型矩阵, 皮肤层 0=steve 1=alex)；第三人称时传入。
    pub player: Option<(&'a [glam::Mat4; PART_COUNT], u32)>,
    /// 挖掘裂纹 + 选中描边；None = 准星无目标。
    pub overlay: Option<MiningOverlay>,
    /// 眼睛在水中（Player.isEyeInFluid(WATER)）：帧雾切水下参数
    /// （26.1 水下视距骤减；GameRuntime::eye_under_water 喂入）。
    pub underwater: bool,
    /// 粒子引擎：(池, 帧内 tick 进度 partialTickTime 0..1)；None 不画
    /// （crack overlay 后、水前，26.1 translucent 序）。
    pub particles: Option<(&'a ParticleEngine, f32)>,
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
    /// MC GUI 精灵表(资源根 textures/ 下原版精灵);None = 素材缺失
    /// (上层按素材红线显示加载失败提示，无程序化回退)。
    gui: Option<SpriteSheet>,
    player_pipeline: wgpu::RenderPipeline,
    player_bind_layout: wgpu::BindGroupLayout,
    player_uniform: wgpu::Buffer,
    player_vbuf: wgpu::Buffer,
    player_ibuf: wgpu::Buffer,
    player_bind: wgpu::BindGroup,
    player_sampler: wgpu::Sampler,
    skins_loaded: bool,
    crack_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    /// 裂纹 quad 顶点/索引（每帧覆写，最多 6 面 × 4 顶点 / 36 索引）。
    overlay_vbuf: wgpu::Buffer,
    overlay_ibuf: wgpu::Buffer,
    /// 描边 12 条棱 = 24 顶点，创建时一次性上传（单位立方体，origin 定位）。
    outline_vbuf: wgpu::Buffer,
    /// 设备纹理数组层数是否容得下 CRACK_BASE..（GLES 256 层钳制时为 false，
    /// 只画描边不画裂纹）。
    crack_layers_ok: bool,
    /// 生物群系染色基色（FrameUniforms 的 tint_grass/tint_foliage 初值，
    /// 创建期由 plains 基线算出；colormap 缺失时 w=0 禁用染色）。
    tint_grass: [f32; 4],
    tint_foliage: [f32; 4],
    /// 粒子渲染（独立小 draw；纹理/管线在 ParticleRenderer 内）。
    particles: ParticleRenderer,
    particle_bind: wgpu::BindGroup,
    pub max_chunks: u32,
    pub max_hud_quads: u32,
}

/// origins_buf 末尾保留的 overlay/outline 专用 dynamic-offset 槽。
const OVERLAY_ORIGIN_PAD: u64 = 64;

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

/// 生成方块图集的 texture_2d_array 绑定声明 + 按 layer 区间选数组的采样
/// 函数（terrain.wgsl / hud.wgsl 的 @@...@@ 占位由 gpu.rs 烘入，与创建的
/// 数组 counts 同源，永不错位）。
///
/// 单数组（上限 ≥ atlas::LAYERS）时生成的就是一次 textureSample——与拆分前逐字节
/// 同语义，`min(layer, N-1)` 对合法层号是恒等（仅防御 mesher 异常层号）。
/// 多数组（GLES 保底 256）时按区间 if 链：
/// - 分支条件是 `@interpolate(flat)` 层号，逐图元一致，textureSample 的
///   隐式导数在各图元内仍是良定义的；
/// - naga 30.0.1 已不对 fragment 阶段强制 uniform 控制流
///   （naga-30.0.1 src/valid/analyzer.rs:23
///   DISABLE_UNIFORMITY_REQ_FOR_FRAGMENT_STAGE = true），if 链内
///   textureSample 可过 create_shader_module 校验；
/// - 每分支 min() 把局部层号钉死在本数组内——wgpu/naga GLES 后端对数组
///   层不做任何钳制（naga-30.0.1 src/back/glsl/writer.rs:2635-2639 裸拼
///   layer 分量；GLSL ES 3.0 §8.8 越界层结果未定义，Mali 实测常返回
///   透明黑 → fs_terrain alpha<0.5 discard → 「雾色平色面」），故越界
///   在这里从源头杜绝。
fn atlas_arrays_wgsl(counts: &[usize], bindings: &[u32], sampler: &str, fn_name: &str) -> String {
    assert_eq!(counts.len(), bindings.len());
    assert!(!counts.is_empty(), "图集至少要有一个数组");
    let mut s = String::new();
    // 占位标记写在 `// ` 注释行内，replace 只换标记本身——首行前补换行，
    // 让残留的 `// ` 孤立成空注释行，生成的声明才不会被注释掉。
    s.push('\n');
    for (i, &b) in bindings.iter().enumerate() {
        s.push_str(&format!(
            "@group(0) @binding({b}) var terrain_tex{i}: texture_2d_array<f32>;\n"
        ));
    }
    s.push_str(&format!(
        "fn {fn_name}(uv: vec2<f32>, layer: u32) -> vec4<f32> {{\n"
    ));
    let mut acc = 0usize;
    for (i, &cnt) in counts.iter().enumerate() {
        let last = i + 1 == counts.len();
        let local = if acc == 0 {
            format!("min(layer, {}u)", cnt - 1)
        } else {
            format!("min(layer - {}u, {}u)", acc, cnt - 1)
        };
        if last {
            // WGSL 规定 else 后只能是复合块或 if 语句，裸 return 不合法。
            if i > 0 {
                s.push_str(" {\n");
            }
            s.push_str(&format!(
                "    return textureSample(terrain_tex{i}, {sampler}, uv, {local});\n"
            ));
            if i > 0 {
                s.push_str("    }\n");
            }
        } else {
            s.push_str(&format!(
                "    if (layer < {}u) {{\n        return textureSample(terrain_tex{i}, {sampler}, uv, {local});\n    }} else ",
                acc + cnt
            ));
        }
        acc += cnt;
    }
    s.push_str("}\n");
    s
}

impl Renderer {
    /// 单数组图集（桌面/Vulkan 主路径）。
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        assets_dir: Option<&std::path::Path>,
    ) -> Self {
        Self::with_atlas_layer_cap(device, queue, color_format, assets_dir, None)
    }

    /// `atlas_layer_cap = Some(n)`：把单数组层数上限强制为
    /// min(n, 设备上限)（测试钩子——CI lavapipe 上限 3907，用它模拟
    /// GLES 256 层设备走多数组路径）。
    pub fn with_atlas_layer_cap(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        assets_dir: Option<&std::path::Path>,
        atlas_layer_cap: Option<u32>,
    ) -> Self {
        let max_chunks: u32 = 1024;
        let max_hud_quads: u32 = 4096;

        // ---- terrain texture arrays ------------------------------------
        // 真实官方贴图 827 层 + missing 哨兵 + 裂纹 10 层 = atlas::LAYERS。
        // 设备 max_texture_array_layers < LAYERS（GLES 规范下限 256）时按
        // atlas::split_layer_counts 拆成 N 个 texture_2d_array，
        // terrain.wgsl/hud.wgsl 由 gpu.rs 展开 layer 区间 if 链选数组；
        // 上限 ≥ LAYERS（桌面/Vulkan）保持单数组零分支零回归。
        let device_max = device.limits().max_texture_array_layers;
        let per = atlas_layer_cap.unwrap_or(device_max).min(device_max).max(1) as usize;
        let counts = atlas::split_layer_counts(per);
        let n_layers: usize = counts.iter().sum();
        // 图集容量取证（2026-10-10 真机 Mali「平色面」定位）：error 级常显，
        // 与 app.rs 的 adapter 侧 `atlas-cap` 行配对。真机 logcat 必见此行：
        // truncated=false 且 arrays=[838] 说明设备没被卡，崩坏另有根因。
        log::error!(
            "atlas-cap: device max_texture_array_layers={} max_texture_dimension_2d={} atlas::LAYERS={} arrays={:?} created={} truncated={} crack_layers_ok={}",
            device_max,
            device.limits().max_texture_dimension_2d,
            atlas::LAYERS,
            counts,
            n_layers,
            n_layers < atlas::LAYERS,
            n_layers > atlas::CRACK_BASE,
        );
        let payload = atlas::generate_payload_with_pack(assets_dir);
        let mip0_layer_bytes = atlas::TILE_PX * atlas::TILE_PX * 4;
        let mip1_layer_bytes = (atlas::TILE_PX / 2) * (atlas::TILE_PX / 2) * 4;
        assert_eq!(
            payload.len(),
            atlas::LAYERS * (mip0_layer_bytes + mip1_layer_bytes),
            "atlas payload 布局与拆分假设不符"
        );
        let mip0_total = atlas::LAYERS * mip0_layer_bytes;
        let mut terrain_views: Vec<wgpu::TextureView> = Vec::with_capacity(counts.len());
        let mut layer_base = 0usize;
        for (i, &cnt) in counts.iter().enumerate() {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("terrain-array{i}")),
                size: wgpu::Extent3d {
                    width: atlas::TILE_PX as u32,
                    height: atlas::TILE_PX as u32,
                    depth_or_array_layers: cnt as u32,
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
                &payload[layer_base * mip0_layer_bytes..(layer_base + cnt) * mip0_layer_bytes],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((atlas::TILE_PX * 4) as u32),
                    rows_per_image: Some(atlas::TILE_PX as u32),
                },
                wgpu::Extent3d {
                    width: atlas::TILE_PX as u32,
                    height: atlas::TILE_PX as u32,
                    depth_or_array_layers: cnt as u32,
                },
            );
            // mip1 必须单独上传：payload 尾部是 box 下采样结果（层序同 mip0）。
            // 采样器 mipmap_filter=Nearest 会把 LOD≥0.5 直接舍入到 mip1，
            // 漏传则采到未定义内容（lavapipe 清零 → 裂纹 discard、远景发黑）。
            let mip1_off = mip0_total + layer_base * mip1_layer_bytes;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 1,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &payload[mip1_off..mip1_off + cnt * mip1_layer_bytes],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((atlas::TILE_PX / 2) as u32 * 4),
                    rows_per_image: Some((atlas::TILE_PX / 2) as u32),
                },
                wgpu::Extent3d {
                    width: (atlas::TILE_PX / 2) as u32,
                    height: (atlas::TILE_PX / 2) as u32,
                    depth_or_array_layers: cnt as u32,
                },
            );
            terrain_views.push(tex.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }));
            layer_base += cnt;
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            ..Default::default()
        });

        // ---- 生物群系染色（26.1 BlockColors / GrassColor 等价）-----------
        // ① LUT：tile 层号 → tint 类别（vec4<u32>(kind, r, g, b)，std140
        //   stride 16）。草/叶族乘 FrameUniforms 基色，云杉/白桦常量色烤进
        //   LUT；裂纹/哨兵层 kind=0 不染色。
        let tint_lut_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tint-lut"),
            size: (atlas::LAYERS * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&tint_lut_buf, 0, &mcv_core::tint::tint_lut_bytes());
        // ② plains 基线草/叶色：colormap PNG（原版 GrassColorReloadListener
        //   同款 256x256 查表图）。缺失 → log::error + 关闭染色（草地按
        //   原版灰度贴图原样显示，不伪造颜色）。
        let load_colormap = |file: &str| -> Option<Vec<u8>> {
            let dir = assets_dir?;
            let bytes = std::fs::read(dir.join("textures/colormap").join(file)).ok()?;
            mcv_core::tint::decode_colormap(&bytes)
        };
        let (tint_grass, tint_foliage) =
            match (load_colormap("grass.png"), load_colormap("foliage.png")) {
                (Some(g), Some(f)) => {
                    let (g, f) = mcv_core::tint::world_grass_foliage_color(&g, &f)
                        .expect("colormap 已按 256x256 校验");
                    ([g[0], g[1], g[2], 1.0], [f[0], f[1], f[2], 1.0])
                }
                _ => {
                    log::error!(
                        "colormap 素材缺失（textures/colormap/{{grass,foliage}}.png）\
——生物群系染色禁用，草/树叶按灰度贴图原样显示（不伪造颜色）"
                    );
                    ([1.0, 1.0, 1.0, 0.0], [1.0, 1.0, 1.0, 0.0])
                }
            };

        // ---- celestial texture array（太阳 + 8 月相）--------------------
        // 原版 26.1 日月为贴图 quad（SkyRenderer.java:125-127/:149-157），
        // 素材 environment/celestial/{sun.png, moon/<phase>.png}。素材缺失
        // → 素材红线（2026-10）：删除程序化天体圆盘回退，上传全透明纹理、
        // log::error，天空保持无天体——绝不画假太阳/假月亮。
        let celestial_payload = assets_dir.and_then(celestial::load_payload);
        if celestial_payload.is_none() {
            log::error!(
                "天体贴图缺失：textures/environment/celestial/{{sun.png,moon/*.png}}\
——天空将没有太阳与月亮（无程序化回退，请检查部署的 assets/）"
            );
        }
        let celestial_data = celestial_payload
            .unwrap_or_else(|| vec![0u8; celestial::CELESTIAL_LAYERS * 32 * 32 * 4]);
        let celestial_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("celestial-array"),
            size: wgpu::Extent3d {
                width: celestial::CELESTIAL_PX as u32,
                height: celestial::CELESTIAL_PX as u32,
                depth_or_array_layers: celestial::CELESTIAL_LAYERS as u32,
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
                texture: &celestial_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &celestial_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((celestial::CELESTIAL_PX * 4) as u32),
                rows_per_image: Some(celestial::CELESTIAL_PX as u32),
            },
            wgpu::Extent3d {
                width: celestial::CELESTIAL_PX as u32,
                height: celestial::CELESTIAL_PX as u32,
                depth_or_array_layers: celestial::CELESTIAL_LAYERS as u32,
            },
        );
        let celestial_view = celestial_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let celestial_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("celestial-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // ---- font texture ---------------------------------------------
        // 原版字体贴图（textures/font/ascii.png + BitmapProvider 度量）。
        // 缺失/解码失败 → 素材红线（2026-10）：不再回退程序化 8x8 字体，
        // log::error 并以全透明纹理占位（HUD 文字整体不上屏）。
        let (font_data, font_widths) = match font::load_atlas(assets_dir) {
            Ok(v) => v,
            Err(e) => {
                log::error!("{e}——HUD 文字将不上屏");
                (vec![0u8; font::TEX_W * font::TEX_H * 4], [0u8; 256])
            }
        };
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

        // ---- GUI 精灵表(资源根 textures/ 下原版精灵)-------------------
        // 缺素材时建 1x1 占位纹理，gui 字段为 None → 上层按素材红线显示
        // 加载失败提示（无程序化面板回退）。
        let gui = assets_dir.and_then(SpriteSheet::load);
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
        // 追加图集数组槽（binding 5..）：仅当设备上限 < atlas::LAYERS 拆
        // 多数组时存在（4 固定给 tint LUT，见 frame_layout_entries 尾部）。
        let atlas_d2_array = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let mut frame_layout_entries = vec![
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
            atlas_d2_array(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // 生物群系染色 LUT（层号 → tint 类别，mcv_core::tint）。
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        for i in 1..terrain_views.len() {
            frame_layout_entries.push(atlas_d2_array(4 + i as u32));
        }
        let frame_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame-layout"),
            entries: &frame_layout_entries,
        });
        let sky_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-layout"),
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
                // 天体纹理数组（太阳 + 8 月相）：原版日月为贴图 quad，
                // fs_sky 解析投影采样（SkyRenderer.java:125-157）。
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let mut hud_layout_entries = vec![
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
            atlas_d2_array(3),
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
        ];
        // 追加图集数组槽（binding 6..8）：物品图标与 terrain 同规则选数组。
        for i in 1..terrain_views.len() {
            hud_layout_entries.push(atlas_d2_array(5 + i as u32));
        }
        let hud_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud-layout"),
            entries: &hud_layout_entries,
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
            // 末尾多留 1 槽给挖掘 overlay 的 origin（dynamic offset =
            // max_chunks * 256），复用 terrain/water 的绑定组布局。
            size: (max_chunks as u64 + 1) * 256,
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

        // ---- particles -------------------------------------------------
        // 粒子渲染器（原版 textures/particle 贴图）；frame uniform 复用
        // frame_buf，方块图集/sampler 复用 terrain 首数组（多数组拆分设备上
        // 碎屑仅数组 0 的层有真贴图，其余层采样钳在末层——粒子为装饰可接受，
        // 真机层数 ≥838 单数组无此问题）。
        let particles = ParticleRenderer::new(&device, &queue, color_format, assets_dir);
        let particle_bind =
            particles.build_bind_group(&device, &frame_buf, &terrain_views[0], &sampler);

        // ---- bind groups ----------------------------------------------
        // terrain 图集数组槽：binding 2 = terrain_views[0]，追加数组 5/6/7
        // （binding 4 固定给生物群系染色 LUT——tint_lut 在 terrain.wgsl 是
        // 静态声明，不能随数组数漂移）。
        let mut frame_bind_entries = vec![
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
                resource: wgpu::BindingResource::TextureView(&terrain_views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: tint_lut_buf.as_entire_binding(),
            },
        ];
        for (i, view) in terrain_views.iter().enumerate().skip(1) {
            frame_bind_entries.push(wgpu::BindGroupEntry {
                binding: 4 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let frame_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame-bind"),
            layout: &frame_bind_layout,
            entries: &frame_bind_entries,
        });
        let sky_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-bind"),
            layout: &sky_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sky_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&celestial_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&celestial_sampler),
                },
            ],
        });
        // hud 图集数组槽：binding 3 = terrain_views[0]，追加数组 6/7/8。
        let mut hud_bind_entries = vec![
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
                resource: wgpu::BindingResource::TextureView(&terrain_views[0]),
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
        ];
        for (i, view) in terrain_views.iter().enumerate().skip(1) {
            hud_bind_entries.push(wgpu::BindGroupEntry {
                binding: 5 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let hud_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hud-bind"),
            layout: &hud_bind_layout,
            entries: &hud_bind_entries,
        });

        // ---- pipelines -------------------------------------------------
        // 图集数组声明+采样函数按设备层数烘进 shader（与 counts 同源，见
        // atlas_arrays_wgsl）；terrain 图集绑定在 group0 的 2(+5/6/7)
        // （4 固定给 tint LUT），hud 在 3(+6/7/8)。
        let terrain_arrays = atlas_arrays_wgsl(
            &counts,
            &[2, 5, 6, 7][..counts.len()],
            "terrain_samp",
            "sample_terrain",
        );
        let hud_arrays = atlas_arrays_wgsl(
            &counts,
            &[3, 6, 7, 8][..counts.len()],
            "hud_samp",
            "sample_terrain_icon",
        );
        let terrain_src =
            include_str!("../assets/terrain.wgsl").replace("@@TERRAIN_ARRAYS@@", &terrain_arrays);
        let hud_src =
            include_str!("../assets/hud.wgsl").replace("@@HUD_TERRAIN_ARRAYS@@", &hud_arrays);
        // 占位替换必须生效，否则 shader 里 sample_terrain 无定义（naga 报错
        // 难定位到模板层），在这里直接把错抛出来。
        assert!(!terrain_src.contains("@@"), "terrain.wgsl 占位未替换");
        assert!(!hud_src.contains("@@"), "hud.wgsl 占位未替换");
        let frame_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(terrain_src.into()),
        });
        let sky_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/sky.wgsl").into()),
        });
        let hud_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud"),
            source: wgpu::ShaderSource::Wgsl(hud_src.into()),
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
            depth_stencil: depth_read_only.clone(),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // 裂纹：复用 vs_terrain 与 terrain 顶点布局，片元 alpha 混合；
        // 顶点 CPU 侧外偏 0.003，深度只读不写。
        let crack_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mining-crack"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_mod,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[Some(terrain_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frame_mod,
                entry_point: Some("fs_crack"),
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
            depth_stencil: depth_read_only.clone(),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // 描边：LineList，pos-only 顶点；12 条棱 24 顶点创建时传一次。
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("block-outline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_mod,
                entry_point: Some("vs_outline"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frame_mod,
                entry_point: Some("fs_outline"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: depth_read_only,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let overlay_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("overlay-vbuf"),
            size: 6 * 4 * TERRAIN_STRIDE as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let overlay_ibuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("overlay-ibuf"),
            size: 6 * 6 * 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 单位立方体 12 条棱（LineList），外扩 0.002 防与方块面 z-fighting。
        let outline_vbuf = {
            let e = 0.002f32;
            let (lo, hi) = (-e, 1.0 + e);
            // 8 角索引位打包：bit0=x bit1=y bit2=z
            let corners = |i: u32| {
                [
                    if i & 1 == 0 { lo } else { hi },
                    if i & 2 == 0 { lo } else { hi },
                    if i & 4 == 0 { lo } else { hi },
                ]
            };
            let edges: [[u32; 2]; 12] = [
                [0, 1],
                [2, 3],
                [4, 5],
                [6, 7], // X 向
                [0, 2],
                [1, 3],
                [4, 6],
                [5, 7], // Y 向
                [0, 4],
                [1, 5],
                [2, 6],
                [3, 7], // Z 向
            ];
            let mut verts = Vec::with_capacity(24);
            for [a, b] in edges {
                verts.extend_from_slice(&corners(a));
                verts.extend_from_slice(&corners(b));
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("outline-vbuf"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            })
        };
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
            crack_pipeline,
            outline_pipeline,
            overlay_vbuf,
            overlay_ibuf,
            outline_vbuf,
            crack_layers_ok: n_layers > atlas::CRACK_BASE,
            tint_grass,
            tint_foliage,
            particles,
            particle_bind,
            max_chunks,
            max_hud_quads,
        }
    }

    /// MC GUI 精灵表;None 表示资源根未带精灵，上层按素材红线显示
    /// 加载失败提示（无程序化回退）。
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
            // 雾：常规 = 基线 0.006 × 天气乘子（雨雾距收缩的 exp 映射，
            // AtmosphericFogEnvironment.java:70-73）；水下 = 高密度短视距
            // 覆盖（26.1 水下能见度骤减；fog_factor = exp2(-dist·x)，
            // x=0.05 → 20 m 处透过 0.5）。
            fog_params: if scene.underwater {
                [0.05, 0.0, 32.0, 0.0]
            } else {
                [0.006 * scene.fog_density_mult, 0.0, cam.far * 0.95, 0.0]
            },
            tint_grass: self.tint_grass,
            tint_foliage: self.tint_foliage,
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
        // 地平线基色 × 天气雾色乘子（雨/雷变灰变暗，天空 + 地形雾同源）。
        let hor = [
            0.62 * scene.fog_tint[0],
            0.76 * scene.fog_tint[1],
            0.95 * scene.fog_tint[2],
            0.0,
        ];
        sky_u[24..28].copy_from_slice(&hor);
        // 天体参数：x = 月相序（MoonPhase 序 0..7）、y = 自由（旧「素材缺失
        // → 程序化圆盘回退」开关已随红线删除）。
        sky_u[28..32].copy_from_slice(&[scene.moon_phase as f32, 0.0, 0.0, 0.0]);
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

            // clouds: sky 之后、不透明之前（26.1 RenderPassOrder）。
            // 云顶点是**相机相对**坐标（offset=(−xInCell, bottomY−eyeY, −zInCell)，
            // CloudRenderer.java:151/198——26.1 相机相对管线里 ModelViewMat 不含
            // 平移）。地形走世界空间 vp，云必须换无平移视图（eye 置原点），
            // 否则平移二次叠加把整片云推出视锥（离屏测试 diff=0 的根因）。
            if let Some((clouds, settings)) = scene.cloud {
                let vp_rel = cam.proj() * glam::Mat4::look_at_rh(Vec3::ZERO, cam.dir(), Vec3::Y);
                clouds.draw(
                    &mut pass,
                    &vp_rel.to_cols_array_2d(),
                    eye,
                    scene.time,
                    settings,
                );
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

            // 挖掘裂纹 + 选中描边：不透明后、水前（26.1 translucent 序）。
            if let Some(ov) = scene.overlay {
                let overlay_off = self.max_chunks * 256;
                let mut slot = [0.0f32; OVERLAY_ORIGIN_PAD as usize];
                slot[..3].copy_from_slice(&ov.min);
                self.queue.write_buffer(
                    &self.origins_buf,
                    overlay_off as u64,
                    bytemuck::cast_slice(&slot),
                );
                if ov.crack_stage.is_some() && self.crack_layers_ok {
                    let (verts, idx) = build_crack_overlay(&ov);
                    if !idx.is_empty() {
                        self.queue.write_buffer(
                            &self.overlay_vbuf,
                            0,
                            bytemuck::cast_slice(&verts),
                        );
                        self.queue
                            .write_buffer(&self.overlay_ibuf, 0, bytemuck::cast_slice(&idx));
                        pass.set_pipeline(&self.crack_pipeline);
                        pass.set_bind_group(0, &self.frame_bind, &[overlay_off]);
                        pass.set_vertex_buffer(
                            0,
                            self.overlay_vbuf
                                .slice(..(verts.len() * TERRAIN_STRIDE) as u64),
                        );
                        pass.set_index_buffer(
                            self.overlay_ibuf.slice(..(idx.len() * 4) as u64),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
                    }
                }
                pass.set_pipeline(&self.outline_pipeline);
                pass.set_bind_group(0, &self.frame_bind, &[overlay_off]);
                pass.set_vertex_buffer(0, self.outline_vbuf.slice(..));
                pass.draw(0..24, 0..1);
            }

            // 粒子：裂纹 overlay 后、水前（26.1 translucent 序；
            // ParticleEngine.tick 是 20Hz 固定步，提取/绘制在渲染帧）。
            if let Some((engine, partial_tick)) = scene.particles {
                self.particles.draw(
                    &mut pass,
                    &self.particle_bind,
                    crate::particle_renderer::DrawCtx {
                        engine,
                        cam,
                        day: scene.day_factor,
                        partial_tick,
                    },
                    &self.queue,
                );
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
