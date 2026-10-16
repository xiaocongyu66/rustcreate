//! 掉落物渲染（#150 通用物品渲染通路）：方块物品 3D 微缩立方体 +
//! 非方块物品 billboard 图标批。
//!
//! 机制对标 26.1 `ItemEntityRenderer.java` / `ItemRenderer.render`（只提取
//! 机制/常数/几何约定，代码全自写，仓库不含 Mojang 代码）：
//!
//! - 实体尺寸：掉落物 AABB 0.25³（Item.java SIZE=0.25，仓内
//!   `mcv_entity::ITEM_HALF` 同源）。
//! - **方块物品 = baked 方块模型的 1/4 微缩立方体**（26.1 block item 走
//!   baked block model 3D；`models/block/block.json` 的 ground display：
//!   scale 0.25、translation 3（×1/16 = 0.1875）、rotation 0，经
//!   `ItemTransform.apply`（translate→rotate→scale→translate(−½,−½,−½)）
//!   合成后立方体中心恰在脚底上方 0.1875——[`DROP_CENTER_Y`] 常数同源；
//!   微缩立方体 = [`block_cube_verts`]，六面各取 `BLOCKS[].tiles[f]`
//!   官方图集层（[`mcv_core::BlockDef::tiles`] 序 +X,-X,+Y,-Y,+Z,-Z）、
//!   面亮度乘 CardinalLighting DEFAULT（terrain.wgsl `face_shade` 同表）。
//!   非全尺寸方块的物品（cross/torch/slab…）：原版走 item model 平面
//!   分支，本仓无 item model 烘焙，维持 billboard 侧面 tile（
//!   KNOWN-DIVERGENCE ITEMRENDER-1 登记）。
//! - 上下浮动（:47-49）：`bob = sin(ageInTicks / 10 + bobOffset) × 0.1
//!   + 0.1`（[`bob`]），渲染原点抬升含 `ITEM_MIN_HOVER_HEIGHT = 0.0625`
//!   （:18，简化为并入浮动基线，见 [`DROP_CENTER_Y`] 注）。
//! - 自转（ItemEntity.java:431-433）：`getSpin = ageInTicks / 20 +
//!   bobOffset`（弧度，[`spin`]）。原版 = 整个模型绕世界 Y 轴旋转
//!   （ItemEntityRenderer.java:51 `mulPose(Axis.YP.rotation(spin))`）；
//!   立方体顶点在 [`block_cube_verts`] 内直接绕 Y 旋转，billboard = 竖直
//!   卡片绕竖轴自转（水平基向量随 spin 旋转）。**sprite 挤出体债登记**
//!   （FLAT_ITEM_DEPTH_THRESHOLD :20 分支）与 **count 堆叠**（:58-110）
//!   见 docs/KNOWN-DIVERGENCE.md ITEMRENDER-1。
//! - 计数堆叠（submitMultipleFromCount :58-110）：count>1 画多份模型
//!   随机散布——不做，单枚几何代表整堆（债登记）。
//!
//! 渲染序：世界 pass 内不透明地形/实体之后、水前（26.1 entity 不透明
//! 相），cutout（alpha<0.1 discard）+ 写深度。材质物品采 GUI 精灵表
//! （tex_set=0），方块物品采方块图集（tex_set=1，层号逐面携带）。

use glam::Vec3;

use crate::camera::Camera;
use crate::gui::SpriteSheet;
use crate::hand::HandItem;

/// 掉落物渲染几何边长（块）：ItemEntity AABB 0.25（Item.java:24 SIZE），
/// 亦 = 原版 ground display 的 0.25 缩放（models/block/block.json:12）。
pub const DROP_QUAD: f32 = 0.25;
/// 几何中心相对 AABB 脚底的抬升：半盒 0.125 + 悬浮下限 0.0625
/// （ItemEntityRenderer.java:18 ITEM_MIN_HOVER_HEIGHT；原版 = −bb.minY +
/// 0.0625，bb 依模型盒而定；ground display 平移 3/16=0.1875 − 半盒 0.125
/// 恰回 0.0625——本常数与原版微缩立方体中心逐位同值，浮动项 [`bob`]
/// ∈ [0, 0.2] 叠加其上——最少也悬在离地 0.0625 处）。
pub const DROP_CENTER_Y: f32 = DROP_QUAD * 0.5 + 0.0625;
/// 单帧绘制上限（掉落物量级几十；超出截断告警，与粒子 MAX_DRAW_QUADS 同策）。
pub const MAX_DROPS: u32 = 512;

/// 上下浮动量（26.1 ItemEntityRenderer.java:48 原式）：
/// `sin(ageInTicks / 10 + bobOffset) × 0.1 + 0.1`。
#[must_use]
pub fn bob(age_ticks: f32, phase: f32) -> f32 {
    (age_ticks / 10.0 + phase).sin() * 0.1 + 0.1
}

/// 自转角（弧度，26.1 ItemEntity.java:431-433 getSpin 原式）：
/// `ageInTicks / 20 + bobOffset`。原版消费 = ItemEntityRenderer.java:51
/// `mulPose(Axis.YP.rotation(spin))` 整模型绕世界 Y。
#[must_use]
pub fn spin(age_ticks: f32, phase: f32) -> f32 {
    age_ticks / 20.0 + phase
}

/// 方块物品掉落形态分流：全尺寸立方（`Shape::Cube`，千块表 model_kind 0
/// 同域——“官方 blockstate model 为整盒立方”）→ 3D 微缩立方体（26.1
/// block item 走 baked block model）；非立方形状（cross/torch/slab/…，
/// `Shape::Slab` 等）本仓无对应 item model 烘焙，回落 billboard 侧面
/// tile（KNOWN-DIVERGENCE ITEMRENDER-1，非方块物品 sprite 同为原版
/// 平面观感）。
#[must_use]
pub fn block_drop_is_cube(bid: u16) -> bool {
    mcv_core::BLOCKS
        .get(usize::from(bid))
        .is_some_and(|def| def.shape == mcv_core::Shape::Cube as u8)
}

/// 单面模板：4 角单位立方坐标 + 4 组 uv（拆别名以满足 clippy type_complexity）。
type FaceTemplate = ([[f32; 3]; 4], [[f32; 2]; 4]);

/// 6 面立方模板（单位立方 0..1³ 的 4 角 + 贴图 uv），面序 = face_id
/// +X,-X,+Y,-Y,+Z,-Z（= [`mcv_core::BlockDef::tiles`] 序）。uv 轴向 =
/// mcv_mesher `face_uv_axes`/`box_face` 约定同源：侧面 u = 水平轴、
/// v = 竖直且 **v=0 落盒顶**（图集上传行序无翻转，row 0 = 贴图顶边，
/// mesher.rs:562-565/731 注）；顶/底面 u = x、v = z（无镜像）。
const CUBE_FACES: [FaceTemplate; 6] = [
    // +X：u 沿 +z（u_axis=2），v 镜像（v_axis=1）。
    (
        [
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
        [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
    ),
    // -X：u 沿 +z，v 镜像。
    (
        [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ],
        [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
    ),
    // +Y：u=x、v=z。
    (
        [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ],
        [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
    ),
    // -Y：u=x、v=z。
    (
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    ),
    // +Z：u 沿 +x（u_axis=0），v 镜像。
    (
        [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ],
        [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
    ),
    // -Z：u 沿 +x，v 镜像。
    (
        [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
    ),
];

/// 面亮度（terrain.wgsl `face_shade` 同表：26.1 CardinalLighting.java:8
/// DEFAULT——DOWN 0.5 / UP 1.0 / NORTH,SOUTH(Z) 0.8 / EAST,WEST(X) 0.6）；
/// CPU 乘入 lightmap RGB（billboard 单 tile 无面语义，立方体六面各乘）。
const FACE_SHADE: [f32; 6] = [0.60, 0.60, 1.00, 0.50, 0.80, 0.80];

/// 单枚微缩立方体的顶点数（6 面 × 4 角）：
/// `vbuf` 容量按此 × [`MAX_DROPS`] 预留（billboard 每枚仅 4，混批共用）。
pub const VERTS_PER_CUBE: usize = 24;
/// 单枚微缩立方体的索引数（6 面 × 2 三角 × 3）：`ibuf` 容量按此预留。
pub const INDICES_PER_CUBE: usize = 36;

/// 微缩立方体展开（26.1 baked block model 3D + ground display：scale
/// 0.25、translate 3/16—— [`DROP_QUAD`]/[`DROP_CENTER_Y`] 常数即两者合成
/// 的立方体边长/中心）。
///
/// `center` = 立方体中心（含 [`bob`] 抬升），`angle` = [`spin`] 自转角
/// （原版 :51 整模型绕世界 Y；顶点局部坐标直接绕 Y 旋转，x' = x·cosθ +
/// z·sinθ、z' = −x·sinθ + z·cosθ，与本文件 billboard 基向量同式），
/// `light` = [`light_curve`][crate::particles::light_curve] 产物 RGB；
/// 每面乘 [`FACE_SHADE`] 后随顶点携带。
fn block_cube_verts(
    tiles: &[u16; 6],
    center: Vec3,
    angle: f32,
    light: [f32; 3],
) -> [DropVertex; VERTS_PER_CUBE] {
    let (s, c) = angle.sin_cos();
    let mut out = [DropVertex {
        pos: [0.0; 3],
        uv: [0.0; 2],
        layer: 0,
        tex_set: 1,
        light: [0.0; 3],
    }; VERTS_PER_CUBE];
    let mut k = 0;
    for (f, (corners, uvs)) in CUBE_FACES.iter().enumerate() {
        let shade = FACE_SHADE[f];
        for (p, uv) in corners.iter().zip(uvs.iter()) {
            let local = Vec3::new(
                (p[0] - 0.5) * DROP_QUAD,
                (p[1] - 0.5) * DROP_QUAD,
                (p[2] - 0.5) * DROP_QUAD,
            );
            out[k] = DropVertex {
                pos: [
                    center.x + local.x * c + local.z * s,
                    center.y + local.y,
                    center.z - local.x * s + local.z * c,
                ],
                uv: *uv,
                layer: u32::from(tiles[f]),
                tex_set: 1,
                light: [light[0] * shade, light[1] * shade, light[2] * shade],
            };
            k += 1;
        }
    }
    out
}

/// 卡片水平基向量 = 相机右向绕世界 Y 旋转 `angle`（spin 原式
/// ItemEntity.java:431-433；up 恒世界 Y——卡片不随视角低头，26.1 平面
/// item model 的 billboard 等价观感）。旋转式与 [`block_cube_verts`] 的
/// 顶点绕 Y 同式（原版 :51 整模型绕 Y 的两个等价写法）。
fn card_basis(cam: &Camera, angle: f32) -> Vec3 {
    let (right, _, _) = crate::hand::camera_basis(cam);
    let (s, c) = angle.sin_cos();
    Vec3::new(right.x * c + right.z * s, 0.0, -right.x * s + right.z * c).normalize_or_zero()
}

/// 一枚掉落物的渲染快照（game 层 `mcv_entity::ItemDropSnap` 转换而来，
/// 图标经 game 层同源解析函数挂成 [`HandItem`]）。
#[derive(Clone, Copy, Debug)]
pub struct ItemDropDraw {
    /// 世界坐标（AABB 脚底中心）。
    pub pos: [f32; 3],
    /// 已存活 tick 数（含 partialTick 插值；驱动 [`bob`]/[`spin`]）。
    pub age_ticks: f32,
    /// 每实体随机相位（bobOffs 替身，见快照 API 注）。
    pub phase: f32,
    /// 图标：Block = 方块物品（Shape::Cube → 3D 微缩立方体，非立方 →
    /// billboard 侧面 tile），Sprite = GUI 精灵表名
    /// （HUD/手持同源解析，缺失时渲染端跳过该枚——素材红线不造假像素）。
    pub icon: HandItem,
    /// 所在格光照 (block, sky) 0..=15（game 层 `light_at` 供给，
    /// lightmap 曲线上屏；掉落物在暗处不发光）。
    pub light: [u8; 2],
}

/// 顶点（stride 40 B，布局见 item_drop.wgsl DVertIn）：CPU 展开 billboard /
/// 微缩立方体（[`block_cube_verts`]，世界坐标展开式，shader 零改动）。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DropVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    /// tex_set=1 时的方块图集层（GUI 精灵路径恒 0）。
    layer: u32,
    /// 0 = GUI 精灵表（材料物品），1 = 方块图集（方块物品）。
    tex_set: u32,
    /// lightmap RGB（CPU 侧 light_curve 彩轨，particles.rs 同式；
    /// 立方体已乘 [`FACE_SHADE`]）。
    light: [f32; 3],
}

/// 单帧绘制参数（压缩 draw 形参；frame uniform 复用世界 pass 的
/// frame_buf——雾/天象与地形同源）。
pub struct DropDrawCtx<'a> {
    pub cam: &'a Camera,
    pub day: f32,
    /// lightmap 天光颜色（Timelines SKY_LIGHT_COLOR 轨，gpu.rs 传入）。
    pub light_tint: Vec3,
}

/// 掉落物渲染器：单管线 + 每帧顶点/索引重建（粒子渲染器同款形态；索引
/// 随批次混合 billboard(6)/立方(36) 而逐帧上传）。
pub struct ItemDropRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    /// 顶点暂存（每帧复用；billboard 4/枚、立方 24/枚混排）。
    scratch: Vec<DropVertex>,
    /// 索引暂存（每帧复用；与 scratch 一一配对上传）。
    scratch_idx: Vec<u32>,
    /// 已告警过的截断标记（防日志刷屏）。
    warned_truncation: bool,
}

impl ItemDropRenderer {
    /// 管线 + 缓冲（bind group 构造期另建——依赖 gui/terrain 纹理视图）。
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let bind_layout = Self::make_bind_layout(device);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("item-drop-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("item-drop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/item_drop.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("item-drop"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_item_drop"),
                compilation_options: Default::default(),
                buffers: &[Some(drop_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_item_drop"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    // cutout：shader 内 alpha discard（手持图标同策）。
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                // 自转卡片侧对时绕序翻转 + 精灵非对称，双面保险。
                cull_mode: None,
                ..Default::default()
            },
            // 实体不透明相：写深度（与地形/实体一致；裂纹 overlay 之后画
            // 的粒子/水仍按半透明序不写深度，序不冲突）。
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
        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("item-drop-vbuf"),
            // 容量按最坏批（MAX_DROPS 枚全为 3D 立方 24 顶点）预留；
            // billboard 每枚仅 4，混批共用。
            size: (MAX_DROPS as usize * VERTS_PER_CUBE * size_of::<DropVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 索引随顶点每帧重建（billboard 6/枚、立方 36/枚混排，静态
        // [4i..] 模式无法覆盖混批）——容量按最坏批预留。GLES 纪律：帧内
        // 动态覆写走 queue.write_buffer（与 vbuf 同款路径）。
        let ibuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("item-drop-ibuf"),
            size: (MAX_DROPS as usize * INDICES_PER_CUBE * size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bind_layout,
            vbuf,
            ibuf,
            scratch: Vec::with_capacity(256),
            scratch_idx: Vec::with_capacity(384),
            warned_truncation: false,
        }
    }

    /// 绑定布局（0 = frame uniform（frame_buf 复用）、1 = GUI 精灵表
    /// texture_2d、2 = 方块图集 texture_2d_array、3 = terrain 采样器）。
    fn make_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("item-drop-layout"),
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
                        view_dimension: wgpu::TextureViewDimension::D2,
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
        })
    }

    /// 建 bind group（frame_buf 复用 gpu.rs 的 FrameUniforms 缓冲；
    /// gui/terrain 视图与 terrain 采样器沿用世界 pass 的实例）。
    pub fn build_bind_group(
        &self,
        device: &wgpu::Device,
        frame_buf: &wgpu::Buffer,
        gui_view: &wgpu::TextureView,
        terrain_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("item-drop-bind"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(gui_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(terrain_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// 提取 + 上传 + 绘制（世界 pass 内实体相调用；空批零开销返回）。
    pub fn draw(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        bind: &wgpu::BindGroup,
        drops: &[ItemDropDraw],
        ctx: DropDrawCtx<'_>,
        gui_sheet: Option<&SpriteSheet>,
        queue: &wgpu::Queue,
    ) {
        self.scratch.clear();
        self.scratch_idx.clear();
        let mut drawn: u32 = 0;
        for d in drops {
            if drawn >= MAX_DROPS {
                if !self.warned_truncation {
                    log::warn!(
                        "item-drops: {} drops > draw cap {MAX_DROPS}, truncating (once)",
                        drops.len()
                    );
                    self.warned_truncation = true;
                }
                break;
            }
            let age = d.age_ticks.max(0.0);
            let lift = DROP_CENTER_Y + bob(age, d.phase);
            let center = Vec3::new(d.pos[0], d.pos[1] + lift, d.pos[2]);
            // lightmap 曲线（terrain.wgsl 同式，particles.rs 的 CPU 侧实现）。
            let light =
                crate::particles::light_curve(d.light[1], d.light[0], ctx.day, ctx.light_tint);
            // 图标解析（渲染端兜底：精灵名缺失/方块 id 越界 → 跳过该枚，
            // 绝不程序化造假像素——素材红线与 hand/missing 策略同源）。
            match d.icon {
                HandItem::Sprite(name) => {
                    let Some((uv0, uv1)) = gui_sheet.and_then(|s| s.sprite_uv(name)) else {
                        continue;
                    };
                    let r = card_basis(ctx.cam, spin(age, d.phase));
                    self.push_card(center, r, (uv0, uv1), 0, 0, light);
                }
                HandItem::Block(bid) => {
                    let Some(def) = mcv_core::BLOCKS.get(bid as usize) else {
                        continue;
                    };
                    if block_drop_is_cube(bid) {
                        // 3D 微缩立方体（26.1 block item 走 baked block
                        // model 3D + ground display 0.25 / translate 3/16）：
                        // 六面各取 tiles[f] 官方图集层 + CardinalLighting 面
                        // 亮度，整模型绕世界 Y 自转（原版 :51）。
                        let base = self.scratch.len() as u32;
                        let verts = block_cube_verts(&def.tiles, center, spin(age, d.phase), light);
                        for f in 0..(VERTS_PER_CUBE as u32 / 4) {
                            let b = base + f * 4;
                            self.scratch_idx
                                .extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
                        }
                        self.scratch.extend_from_slice(&verts);
                    } else {
                        // 非全尺寸形状（cross/torch/slab/…）：无 baked 立方
                        // item model，维持 billboard 侧面 tile（#150 原路径；
                        // KNOWN-DIVERGENCE ITEMRENDER-1 登记）。
                        let r = card_basis(ctx.cam, spin(age, d.phase));
                        self.push_card(
                            center,
                            r,
                            ([0.0, 0.0], [1.0, 1.0]),
                            u32::from(def.tiles[2]),
                            1,
                            light,
                        );
                    }
                }
                HandItem::Empty => continue,
            }
            drawn += 1;
        }
        if self.scratch.is_empty() {
            return;
        }
        queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(&self.scratch));
        queue.write_buffer(&self.ibuf, 0, bytemuck::cast_slice(&self.scratch_idx));
        let count = self.scratch_idx.len() as u32;
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.set_vertex_buffer(
            0,
            self.vbuf
                .slice(..(self.scratch.len() * size_of::<DropVertex>()) as u64),
        );
        pass.set_index_buffer(
            self.ibuf.slice(..(count as u64 * 4)),
            wgpu::IndexFormat::Uint32,
        );
        pass.draw_indexed(0..count, 0, 0..1);
    }

    /// 竖直卡片 billboard 展开（4 顶点 + 6 索引）：`axis_r` = 自转后的水平
    /// 基向量（[`card_basis`]），二源采样由 `layer`/`tex_set` 选择。
    fn push_card(
        &mut self,
        center: Vec3,
        axis_r: Vec3,
        tile_uv: ([f32; 2], [f32; 2]),
        layer: u32,
        tex_set: u32,
        light: [f32; 3],
    ) {
        let h = DROP_QUAD * 0.5;
        let (uv0, uv1) = tile_uv;
        // 角序（左上/右上/右下/左下）与 uv 对齐：精灵表 v 向下、
        // 方块 tile 顶行 v=0 同 HUD tile_icon 口径。
        let corners = [
            center - axis_r * h + Vec3::Y * h,
            center + axis_r * h + Vec3::Y * h,
            center + axis_r * h - Vec3::Y * h,
            center - axis_r * h - Vec3::Y * h,
        ];
        let uvs = [uv0, [uv1[0], uv0[1]], uv1, [uv0[0], uv1[1]]];
        let base = self.scratch.len() as u32;
        for (p, uv) in corners.into_iter().zip(uvs) {
            self.scratch.push(DropVertex {
                pos: p.to_array(),
                uv,
                layer,
                tex_set,
                light,
            });
        }
        self.scratch_idx
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// 顶点布局（DropVertex 40 B）。
fn drop_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: size_of::<DropVertex>() as wgpu::BufferAddress,
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
                format: wgpu::VertexFormat::Float32x3,
                offset: 28,
                shader_location: 4,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 26.1 ItemEntityRenderer.java:48 原式锁：bob = sin(age/10 + φ)·0.1+0.1。
    #[test]
    fn bob_follows_vanilla_formula() {
        // age=0、φ=0：sin(0)=0 → 0.1（悬浮下限 = 0.0625 基线之上）。
        assert!((bob(0.0, 0.0) - 0.1).abs() < 1e-6);
        // age=15.707（=π/2·10）、φ=0：sin(π/2)=1 → 0.2（峰值）。
        assert!((bob(15.707_963, 0.0) - 0.2).abs() < 1e-4);
        // 值域 [0.1±0.1]（永不沉入 AABB 底）。
        for age in [0.0f32, 3.0, 7.5, 15.707_963, 31.4, 100.0] {
            let b = bob(age, 0.0);
            assert!((0.0..=0.2).contains(&b), "bob 越域: age={age} bob={b}");
        }
    }

    /// 26.1 ItemEntity.java:431-433 原式锁：spin = age/20 + φ（弧度）。
    #[test]
    fn spin_follows_vanilla_formula() {
        assert!((spin(0.0, 0.5) - 0.5).abs() < 1e-6);
        assert!((spin(20.0, 0.0) - 1.0).abs() < 1e-6, "20 tick 转 1 弧度");
        assert!((spin(40.0, 1.0) - 3.0).abs() < 1e-6);
    }

    /// #150 模型分类锁：全尺寸立方 → 3D 微缩立方体；非立方形状维持
    /// billboard（26.1 block item 走 baked block model 的分流条件）。
    #[test]
    fn block_drop_classification_cube_vs_non_cube() {
        // stone（id 1）= Shape::Cube：走六面微缩立方体。
        assert!(block_drop_is_cube(1), "stone 必须走 3D 立方路径");
        // 半砖（acacia_slab id 26，Shape::Slab）：无 baked 立方 item model，
        // 维持 billboard。
        assert!(!block_drop_is_cube(26), "slab 必须维持 billboard 路径");
        // id 越界 → false（draw 侧同步跳过该枚）。
        assert!(!block_drop_is_cube(u16::MAX));
    }

    /// 微缩立方体几何锁（ground display scale 0.25 + ItemTransform −½ 居中
    /// 合成的顶点语义）：24 顶点 = 6 面 × 4 角；面 layer = `tiles[f]`；
    /// 边长 0.25（±0.125）；顶/底面 y 恰 ±0.125；+Z 面顶边 v=0（mesher
    /// v=0 落盒顶约定）。
    #[test]
    fn block_cube_verts_geometry() {
        let tiles: [u16; 6] = [7, 8, 9, 10, 11, 12]; // 各面独立层，便于锁 face→tile 映射
        let center = Vec3::new(3.0, 65.5, -2.0);
        let verts = block_cube_verts(&tiles, center, 0.0, [0.9; 3]);
        assert_eq!(verts.len(), VERTS_PER_CUBE);
        let corner_r = DROP_QUAD * 0.5 * (3.0f32.sqrt());
        for (f, face) in verts.as_chunks::<4>().0.iter().enumerate() {
            for v in face {
                assert_eq!(v.layer, u32::from(tiles[f]), "面 {f} layer=tiles[{f}]");
                assert_eq!(v.tex_set, 1);
                // 立方角点距中心恒 = 0.125·√3（box 半对角）。
                let p = Vec3::from(v.pos);
                assert!(
                    (p.distance(center) - corner_r).abs() < 1e-5,
                    "面 {f} 角 {p} 应在立方体角点域"
                );
            }
        }
        // 顶面（f=2）4 角 y=+0.125、底面（f=3）y=−0.125。
        for v in &verts[8..12] {
            assert!((v.pos[1] - (center.y + 0.125)).abs() < 1e-5, "顶面 y");
        }
        for v in &verts[12..16] {
            assert!((v.pos[1] - (center.y - 0.125)).abs() < 1e-5, "底面 y");
        }
        // +Z 面（f=4）v=0 落盒顶：两个 y=+0.125 角的 uv[1]==0，底角 uv[1]==1。
        for v in &verts[16..20] {
            let top = v.pos[1] > center.y;
            if top {
                assert!(v.uv[1] < 1e-6, "+Z 顶角 v 应=0（贴图顶边）");
            } else {
                assert!((v.uv[1] - 1.0).abs() < 1e-6, "+Z 底角 v 应=1");
            }
        }
        // uv 域 [0,1]²（cube 面不跨 tile）。
        for v in &verts {
            assert!((0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1]));
        }
    }

    /// 面亮度锁：26.1 CardinalLighting DEFAULT（DOWN0.5 UP1.0 NS0.8 EW0.6）
    /// 乘入 per-face light RGB；顶面 = 基准光无衰减、底面 0.5。
    #[test]
    fn block_cube_verts_face_shade_cardinal_lighting() {
        let tiles: [u16; 6] = [0; 6];
        let base = [0.5f32, 0.75, 1.0];
        let verts = block_cube_verts(&tiles, Vec3::ZERO, 0.0, base);
        let expected: [f32; 6] = [0.60, 0.60, 1.00, 0.50, 0.80, 0.80];
        for (f, face) in verts.as_chunks::<4>().0.iter().enumerate() {
            for v in face {
                for (ch, e) in v.light.iter().zip(base.iter()) {
                    assert!((*ch - e * expected[f]).abs() < 1e-6, "面 {f} 亮度 {ch}");
                }
            }
        }
        // 常数与 terrain.wgsl face_shade 表同源（单源真理：字段级对照）。
        assert_eq!(FACE_SHADE, [0.60, 0.60, 1.00, 0.50, 0.80, 0.80]);
    }

    /// 自转锁：spin θ=π/2 下局部 +X 角 → −Z（与 billboard 基向量同式；
    /// ItemEntityRenderer.java:51 整模型绕世界 Y）。
    #[test]
    fn block_cube_verts_spin_rotates_about_y() {
        let tiles: [u16; 6] = [0; 6];
        let c = std::f32::consts::FRAC_PI_2;
        let verts = block_cube_verts(&tiles, Vec3::ZERO, c, [1.0; 3]);
        // +X 面（f=0）第一角 = 局部 (0.125,−0.125,−0.125)：
        // x' = x·cosθ + z·sinθ = −0.125、z' = −x·sinθ + z·cosθ = −0.125。
        assert!(
            (verts[0].pos[0] - (-0.125)).abs() < 1e-5 && (verts[0].pos[2] - (-0.125)).abs() < 1e-5,
            "θ=π/2 应把局部 +X 角转到 −X/−Z 象限: {:?}",
            verts[0].pos
        );
        // θ=0 与 θ=2π 全顶点复位（自转周期闭合）。
        let zero = block_cube_verts(&tiles, Vec3::ZERO, 0.0, [1.0; 3]);
        let full = block_cube_verts(&tiles, Vec3::ZERO, std::f32::consts::TAU, [1.0; 3]);
        for (a, bf) in zero.iter().zip(full.iter()) {
            assert!(
                Vec3::from(a.pos).abs_diff_eq(Vec3::from(bf.pos), 1e-4),
                "2π 自转必须复位"
            );
        }
    }

    /// 挥动相位差 → 画面位移域：同枚掉落物不同 age 的浮动中心必须不同
    /// （bob 消费链的行为锁；像素级消费在 tests/item_drops.rs）。
    #[test]
    fn bob_varies_with_age() {
        assert!((bob(0.0, 0.0) - bob(15.707_963, 0.0)).abs() > 0.05);
    }
}
