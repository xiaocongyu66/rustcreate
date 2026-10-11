//! 掉落物渲染（#150 通用物品渲染通路）：ItemDrop billboard 批。
//!
//! 机制对标 26.1 `ItemEntityRenderer.java`（只提取机制/常数/几何约定，
//! 代码全自写，仓库不含 Mojang 代码）：
//!
//! - 实体尺寸：掉落物 AABB 0.25³（Item.java SIZE=0.25，仓内
//!   `mcv_entity::ITEM_HALF` 同源）→ billboard 图标 quad 边长 0.25 块。
//! - 上下浮动（:47-49）：`bob = sin(ageInTicks / 10 + bobOffset) × 0.1
//!   + 0.1`（[`bob`]），渲染原点抬升含 `ITEM_MIN_HOVER_HEIGHT = 0.0625`
//!   （:18，简化为并入浮动基线，见 [`DROP_CENTER_Y`] 注）。
//! - 自转（ItemEntity.java:431-432）：`getSpin = ageInTicks / 20 +
//!   bobOffset`（弧度，[`spin`]）。原版是物品模型绕世界 Y 轴整体旋转；
//!   billboard 简化 = 竖直卡片绕竖轴自转（水平基向量随 spin 旋转），
//!   侧对相机时投影收窄成线——无穿帮闪没。**3D 微缩模型债登记**：
//!   方块物品的 1/4 缩放立方体与材料物品的 sprite 挤出体（extruded
//!   sprite）都不做，billboard 图标统一替代。
//! - 计数堆叠（submitMultipleFromCount :58-110）：count>1 画多份模型
//!   随机散布——不做，单枚 billboard 代表整堆（债登记）。
//!
//! 渲染序：世界 pass 内不透明地形/实体之后、水前（26.1 entity 不透明
//! 相），cutout（alpha<0.1 discard）+ 写深度。材质物品采 GUI 精灵表
//! （tex_set=0），方块物品采方块图集侧面 tile（tex_set=1，与 HUD
//! tile_icon 先例 game.rs `tiles[2]` 同源）。

use wgpu::util::DeviceExt;

use glam::Vec3;

use crate::camera::Camera;
use crate::gui::SpriteSheet;
use crate::hand::HandItem;

/// 掉落物渲染 quad 边长（块）：ItemEntity AABB 0.25（Item.java:24 SIZE）。
pub const DROP_QUAD: f32 = 0.25;
/// 渲染原点相对 AABB 脚底的抬升：半盒 0.125 + 悬浮下限 0.0625
/// （ItemEntityRenderer.java:18 ITEM_MIN_HOVER_HEIGHT；原版 = −bb.minY +
/// 0.0625，bb 依模型而定，这里取方块物品 0.25 盒的对应量，浮动项
/// [`bob`] ∈ [0, 0.2] 叠加其上——最少也悬在离地 0.0625 处，与原版观感同域）。
pub const DROP_CENTER_Y: f32 = DROP_QUAD * 0.5 + 0.0625;
/// 单帧绘制上限（掉落物量级几十；超出截断告警，与粒子 MAX_DRAW_QUADS 同策）。
pub const MAX_DROPS: u32 = 512;

/// 上下浮动量（26.1 ItemEntityRenderer.java:48 原式）：
/// `sin(ageInTicks / 10 + bobOffset) × 0.1 + 0.1`。
#[must_use]
pub fn bob(age_ticks: f32, phase: f32) -> f32 {
    (age_ticks / 10.0 + phase).sin() * 0.1 + 0.1
}

/// 自转角（弧度，26.1 ItemEntity.java:431-432 getSpin 原式）：
/// `ageInTicks / 20 + bobOffset`。
#[must_use]
pub fn spin(age_ticks: f32, phase: f32) -> f32 {
    age_ticks / 20.0 + phase
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
    /// 图标：Block = 方块图集侧面 tile，Sprite = GUI 精灵表名
    /// （HUD/手持同源解析，缺失时渲染端跳过该枚——素材红线不造假像素）。
    pub icon: HandItem,
    /// 所在格光照 (block, sky) 0..=15（game 层 `light_at` 供给，
    /// lightmap 曲线上屏；掉落物在暗处不发光）。
    pub light: [u8; 2],
}

/// 顶点（stride 40 B，布局见 item_drop.wgsl DVertIn）：CPU 展开 billboard。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DropVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    /// tex_set=1 时的方块图集层（GUI 精灵路径恒 0）。
    layer: u32,
    /// 0 = GUI 精灵表（材料物品），1 = 方块图集（方块物品）。
    tex_set: u32,
    /// lightmap RGB（CPU 侧 light_curve 彩轨，particles.rs 同式）。
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

/// 掉落物渲染器：单管线 + 静态索引 + 每帧顶点重建（粒子渲染器同款形态）。
pub struct ItemDropRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    /// 顶点暂存（每帧复用；len/4 = 本帧 quad 数）。
    scratch: Vec<DropVertex>,
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
            size: (MAX_DROPS as usize * 4 * size_of::<DropVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 静态顺序索引：quad i → [4i..4i+3]（QUADS 模式等价），创建期一次
        // 上传（GLES 纪律：创建期映射写入走 create_buffer_init，帧内
        // 动态覆写才走 queue.write_buffer——粒子 ibuf 同款）。
        let mut indices = Vec::with_capacity((MAX_DROPS * 6) as usize);
        for q in 0..MAX_DROPS {
            let b = q * 4;
            indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("item-drop-ibuf"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            pipeline,
            bind_layout,
            vbuf,
            ibuf,
            scratch: Vec::with_capacity(256),
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
        for d in drops {
            if self.scratch.len() >= (MAX_DROPS as usize) * 4 {
                if !self.warned_truncation {
                    log::warn!(
                        "item-drops: {} drops > draw cap {MAX_DROPS}, truncating (once)",
                        drops.len()
                    );
                    self.warned_truncation = true;
                }
                break;
            }
            // 图标解析（渲染端兜底：精灵名缺失/方块 id 越界 → 跳过该枚，
            // 绝不程序化造假像素——素材红线与 hand/missing 策略同源）。
            let (u0v0, u1v1, layer, tex_set) = match d.icon {
                HandItem::Sprite(name) => {
                    let Some((uv0, uv1)) = gui_sheet.and_then(|s| s.sprite_uv(name)) else {
                        continue;
                    };
                    (uv0, uv1, 0u32, 0u32)
                }
                HandItem::Block(bid) => {
                    let Some(def) = mcv_core::BLOCKS.get(bid as usize) else {
                        continue;
                    };
                    ([0.0, 0.0], [1.0, 1.0], u32::from(def.tiles[2]), 1u32)
                }
                HandItem::Empty => continue,
            };
            let age = d.age_ticks.max(0.0);
            let lift = DROP_CENTER_Y + bob(age, d.phase);
            let center = Vec3::new(d.pos[0], d.pos[1] + lift, d.pos[2]);
            // 竖直卡片绕 Y 自转（spin 原式 ItemEntity.java:431-432）：水平
            // 基向量取相机右向旋转 spin 角，up 恒世界 Y——卡片不随视角低头。
            let (right, _, _) = crate::hand::camera_basis(ctx.cam);
            let (s, c) = spin(age, d.phase).sin_cos();
            let r = Vec3::new(right.x * c + right.z * s, 0.0, -right.x * s + right.z * c)
                .normalize_or_zero();
            let h = DROP_QUAD * 0.5;
            // lightmap 曲线（terrain.wgsl 同式，particles.rs 的 CPU 侧实现）。
            let light =
                crate::particles::light_curve(d.light[1], d.light[0], ctx.day, ctx.light_tint);
            // 角序（左上/右上/右下/左下）与 uv 对齐：精灵表 v 向下、
            // 方块 tile 顶面 v=0 同 HUD tile_icon 口径。
            let corners = [
                center - r * h + Vec3::Y * h,
                center + r * h + Vec3::Y * h,
                center + r * h - Vec3::Y * h,
                center - r * h - Vec3::Y * h,
            ];
            let uvs = [u0v0, [u1v1[0], u0v0[1]], u1v1, [u0v0[0], u1v1[1]]];
            for (p, uv) in corners.into_iter().zip(uvs) {
                self.scratch.push(DropVertex {
                    pos: p.to_array(),
                    uv,
                    layer,
                    tex_set,
                    light,
                });
            }
        }
        let quads = self.scratch.len() / 4;
        if quads == 0 {
            return;
        }
        queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(&self.scratch));
        let count = quads as u32 * 6;
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

    /// 26.1 ItemEntity.java:431-432 原式锁：spin = age/20 + φ（弧度）。
    #[test]
    fn spin_follows_vanilla_formula() {
        assert!((spin(0.0, 0.5) - 0.5).abs() < 1e-6);
        assert!((spin(20.0, 0.0) - 1.0).abs() < 1e-6, "20 tick 转 1 弧度");
        assert!((spin(40.0, 1.0) - 3.0).abs() < 1e-6);
    }

    /// 挥动相位差 → 画面位移域：同枚掉落物不同 age 的浮动中心必须不同
    /// （bob 消费链的行为锁；像素级消费在 tests/item_drops.rs）。
    #[test]
    fn bob_varies_with_age() {
        assert!((bob(0.0, 0.0) - bob(15.707_963, 0.0)).abs() > 0.05);
    }
}
