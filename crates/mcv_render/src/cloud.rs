//! 体素云渲染（自包含模块，接线由主控完成）。
//!
//! 机制照抄 MC 26.1 `net/minecraft/client/renderer/CloudRenderer.java` +
//! `assets/minecraft/shaders/core/rendertype_clouds.{vsh,fsh}`：
//! - `clouds.png`（256×256，1-bit）是**单元占用表**而非采样贴图：alpha<10
//!   视为空单元（`isCellEmpty`），每个非空单元是一个 12×4×12 方块的盒子；
//! - CPU 每帧（相机跨单元/进出云层/设置变化时）按相机位置重建"面表"：
//!   FANCY=拉伸立方体（相邻为空的侧面 + 相机附近内壁面），FAST=每单元一个
//!   底面 quad；
//! - GPU 顶点着色器把面表项拉伸成盒子面（见 `shaders/cloud.wgsl`）；
//! - 云整体以 0.6 方块/秒向 +X 滚动（26.1: offset*0.03/tick），Z 固定相位
//!   +3.96；云底高度 192.33（`EnvironmentAttributes.CLOUD_HEIGHT`）；
//! - 淡出 = 线性雾 `a *= 1 - clamp(d/fog_end,0,1)`，fog_end=云距离
//!   （26.1 rendertype_clouds.fsh + AtmosphericFogEnvironment）。
//!
//! 与 26.1 的实现差异（详见 /root/mc-ref/NOTES-clouds.md）：面表用实例 u32
//! 属性代替 isamplerBuffer；`pass.draw(0..4, 0..n)` 代替索引 quad；深度按
//! 本引擎约定只读不写（26.1 为默认写深度）。
//!
//! 用法（sky 之后、不透明地形之前）：
//! ```ignore
//! clouds.draw(&mut pass, &view_proj, cam_pos, time_seconds, CloudSettings::default());
//! ```

use std::sync::Mutex;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// 云底高度（26.1 `EnvironmentAttributes.CLOUD_HEIGHT` 默认值 /
/// `DimensionDefaults.OVERWORLD_CLOUD_HEIGHT`）。
pub const CLOUD_HEIGHT: f32 = 192.33;
/// 单元边长，方块（26.1 `CELL_SIZE_IN_BLOCKS`）。
const CELL_SIZE: f32 = 12.0;
/// 云厚度，方块（26.1 `relativeBottomY + 4.0F`）。
const THICKNESS: f32 = 4.0;
/// 滚动速度：0.6 方块/秒，沿 +X（26.1 `BLOCKS_PER_SECOND`）。
const BLOCKS_PER_SECOND: f32 = 0.6;
/// Z 固定相位偏移（26.1 `cameraPosition.z + 3.96F`）。
const Z_PHASE: f32 = 3.96;
/// 距离上限，方块（≈MC cloudRange 上限 64 chunk×16；超过会被钳制）。
const MAX_RADIUS_BLOCKS: f32 = 1024.0;
/// 面表缓冲容量对应的最大单元半径（scale=1 时 = MAX_RADIUS_BLOCKS/12）。
const MAX_RADIUS_CELLS: i32 = (MAX_RADIUS_BLOCKS / CELL_SIZE).ceil() as i32;
/// alpha 低于该值视为空单元（26.1 `isCellEmpty`）。
const EMPTY_ALPHA: u8 = 10;

/// 云渲染设置。`distance` 对应 MC videoSettings cloudRange×16（方块）。
#[derive(Clone, Copy, Debug)]
pub struct CloudSettings {
    /// 关闭时 `draw` 直接返回（对应 CloudStatus::OFF）。
    pub enabled: bool,
    /// 云渲染距离（方块），同时是雾淡出终点 FogCloudsEnd。
    pub distance: f32,
    /// true = FAST（平面底面，对应 CloudStatus::FAST），
    /// false = FANCY（拉伸立方体，对应 CloudStatus::FANCY）。
    pub fast: bool,
    /// 单元尺寸倍率；1.0 = MC 原值（12×4×12）。26.1 无此设置，仅为引擎留的
    /// 调节量，默认 1.0 时行为与 MC 完全一致。
    pub scale: f32,
    /// 整体不透明度，对应 26.1 CloudColor 的 alpha（天气染色留给主控扩展）。
    pub opacity: f32,
}

impl Default for CloudSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            distance: 512.0,
            fast: false,
            scale: 1.0,
            opacity: 1.0,
        }
    }
}

/// uniform（与 cloud.wgsl 的 CloudUniform 逐字节对应，112B）。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CloudUniform {
    view_proj: [[f32; 4]; 4],
    color: [f32; 4],
    offset: [f32; 3],
    _pad0: f32,
    cell_size: [f32; 3],
    fog_end: f32,
}

/// Direction.values() 顺序（26.1）：DOWN UP NORTH SOUTH WEST EAST。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

const FLAG_INSIDE_FACE: u32 = 16;
const FLAG_USE_TOP_COLOR: u32 = 32;

/// 相机与云层（bottom..bottom+4）的相对位置（26.1 RelativeCameraPos）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum RelCamPos {
    Above,
    Inside,
    Below,
}

/// 单元位图：bit0=西邻空 bit1=南邻空 bit2=东邻空 bit3=北邻空 bit4=本元非空
/// （邻居位定义对应 26.1 `packCellData` 的 N/E/S/W bit3..0）。
const CELL_FILLED: u8 = 1 << 4;

/// 面表等随相机移动而变的状态（draw 只收 &self，故用 Mutex）。
struct Mutable {
    faces: Vec<u32>,
    prev_key: Option<(i32, i32, u8, u64)>, // (cell_x, cell_z, rel, 设置指纹)
}

/// 体素云渲染器：管线 + 云噪声单元表 + 面表缓冲 + 每帧 uniform。
pub struct Clouds {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
    face_buf: wgpu::Buffer,
    width: usize,
    height: usize,
    /// width*height 单元位图。
    cells: Vec<u8>,
    mut_: Mutex<Mutable>,
}

impl Clouds {
    /// 加载内嵌的 26.1 `clouds.png`（编译期嵌入 `texturepack/misc/clouds.png`），
    /// 在 CPU 端二值化为单元占用表（26.1 同样只在 CPU 读像素，不建 GPU 纹理）。
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let img = image::load_from_memory(include_bytes!("../../../texturepack/misc/clouds.png"))
            .expect("clouds.png")
            .to_rgba8();
        let (width, height) = (img.width() as usize, img.height() as usize);
        let empty = |x: isize, y: isize| -> bool {
            let x = x.rem_euclid(width as isize) as u32;
            let y = y.rem_euclid(height as isize) as u32;
            img[(x, y)].a() < EMPTY_ALPHA
        };
        let mut cells = vec![0u8; width * height];
        for y in 0..height as isize {
            for x in 0..width as isize {
                if !empty(x, y) {
                    let mut bits = CELL_FILLED;
                    if empty(x, y - 1) {
                        bits |= 1 << 3; // north
                    }
                    if empty(x + 1, y) {
                        bits |= 1 << 2; // east
                    }
                    if empty(x, y + 1) {
                        bits |= 1 << 1; // south
                    }
                    if empty(x - 1, y) {
                        bits |= 1 << 0; // west
                    }
                    cells[x as usize + y as usize * width] = bits;
                }
            }
        }

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cloud"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/cloud.wgsl").into()),
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cloud-uniform"),
            size: std::mem::size_of::<CloudUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cloud-bind-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<CloudUniform>() as u64
                    ),
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cloud-bind"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cloud-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cloud"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_clouds"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 4,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Uint32,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_clouds"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    // 与主渲染目标一致（gpu.rs / offscreen.rs 均为 Rgba8UnormSrgb）。
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                // 26.1：CLOUDS 开背面剔除、FLAT_CLOUDS 关（内壁面靠绕序区分）。
                // 这里统一关剔除，靠深度保证正确遮挡。
                cull_mode: None,
                ..Default::default()
            },
            // 天空之后、不透明之前渲染：深度只读不写。
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

        // 面表缓冲一次分配满最大距离所需容量（26.1 getSizeForCloudDistance）：
        // maxCells = 2*(r+1)^2，maxFaces = maxCells*4 + 54（内壁面），每面 u32。
        let r = MAX_RADIUS_CELLS as u64;
        let max_faces = 2 * (r + 1) * (r + 1) * 4 + 54;
        let face_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cloud-faces"),
            size: max_faces * 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            queue: queue.clone(),
            pipeline,
            bind_group,
            uniform_buf,
            face_buf,
            width,
            height,
            cells,
            mut_: Mutex::new(Mutable {
                faces: Vec::new(),
                prev_key: None,
            }),
        }
    }

    /// 渲染云。`pass` 已处于天空之后、不透明几何之前；`time` 为**秒**
    /// （滚动相位 = time × 0.6 方块，对应 26.1 gameTime(tick)×0.03）。
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        view_proj: &[[f32; 4]; 4],
        cam_pos: Vec3,
        time: f32,
        settings: CloudSettings,
    ) {
        if !settings.enabled || settings.opacity <= 0.0 {
            return;
        }
        let scale = settings.scale.max(0.05);
        let cell = CELL_SIZE * scale;
        let distance = settings.distance.clamp(cell, MAX_RADIUS_BLOCKS);
        // 钳到面表缓冲容量对应的最大单元半径（scale<1 时按距离算出的
        // radius_cells 可能更大，这里截断以保证不越界）。
        let radius_cells = ((distance / cell).ceil() as i32).clamp(1, MAX_RADIUS_CELLS);

        // ---- 滚动相位（26.1 render(): L161-171）----
        let period_x = self.width as f64 * cell as f64;
        let period_z = self.height as f64 * cell as f64;
        let cloud_x = (cam_pos.x as f64 + time as f64 * BLOCKS_PER_SECOND as f64) % period_x;
        let cloud_z = (cam_pos.z as f64 + Z_PHASE as f64) % period_z;
        let cloud_x = cloud_x - (cloud_x / period_x).floor() * period_x; // floorMod
        let cloud_z = cloud_z - (cloud_z / period_z).floor() * period_z;
        let cell_x = (cloud_x / cell as f64).floor() as i32;
        let cell_z = (cloud_z / cell as f64).floor() as i32;
        let x_in_cell = (cloud_x - cell_x as f64 * cell as f64) as f32;
        let z_in_cell = (cloud_z - cell_z as f64 * cell as f64) as f32;

        // ---- 相机相对云层位置（26.1 L150-159）----
        let bottom = CLOUD_HEIGHT - cam_pos.y;
        let rel = if bottom + THICKNESS * scale < 0.0 {
            RelCamPos::Above
        } else if bottom > 0.0 {
            RelCamPos::Below
        } else {
            RelCamPos::Inside
        };

        // ---- 面表：仅在 26.1 的重建条件下重建（跨单元/进出云/设置变化）----
        let settings_bits = {
            let mut b = [0u8; 8];
            b[0..4].copy_from_slice(&settings.distance.to_bits().to_le_bytes());
            b[4..8].copy_from_slice(&scale.to_bits().to_le_bytes());
            u64::from_le_bytes(b) | ((settings.fast as u64) << 63)
        };
        let key = (cell_x, cell_z, rel_discriminant(&rel), settings_bits);
        let mut m = self.mut_.lock().unwrap();
        if m.prev_key != Some(key) {
            m.prev_key = Some(key);
            m.faces.clear();
            self.build_faces(
                rel,
                cell_x,
                cell_z,
                !settings.fast,
                radius_cells,
                &mut m.faces,
            );
        }
        let face_count = m.faces.len() as u32;
        if face_count == 0 {
            return;
        }

        // ---- uniform（对应 26.1 CloudInfo + FogCloudsEnd）----
        let uniform = CloudUniform {
            view_proj: *view_proj,
            // CloudColor：本引擎固定白色（天气染色由主控经环境属性扩展）
            color: [1.0, 1.0, 1.0, settings.opacity],
            offset: [-x_in_cell, bottom, -z_in_cell],
            _pad0: 0.0,
            cell_size: [cell, THICKNESS * scale, cell],
            fog_end: distance,
        };
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniform));
        self.queue
            .write_buffer(&self.face_buf, 0, bytemuck::cast_slice(&m.faces));

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.face_buf.slice(0..face_count as u64 * 4));
        // 4 顶点/实例代替 26.1 的 6·n 索引 quad（顶点着色器内展开角点）
        pass.draw(0..4, 0..face_count);
    }

    /// 面表构建，逐行对应 26.1 `buildMesh`/`tryBuildCell`/`buildExtrudedCell`：
    /// 以相机单元为圆心按圆盘（ring + x²+z²≤r²）遍历单元。
    fn build_faces(
        &self,
        rel: RelCamPos,
        center_x: i32,
        center_z: i32,
        extrude: bool,
        radius_cells: i32,
        out: &mut Vec<u32>,
    ) {
        for ring in 0..=2 * radius_cells {
            for rx in -ring..=ring {
                let rz = ring - rx.abs();
                if rz > radius_cells || rx * rx + rz * rz > radius_cells * radius_cells {
                    continue;
                }
                if rz != 0 {
                    self.try_build_cell(rel, center_x, center_z, extrude, rx, -rz, out);
                }
                self.try_build_cell(rel, center_x, center_z, extrude, rx, rz, out);
            }
        }
    }

    fn try_build_cell(
        &self,
        rel: RelCamPos,
        center_x: i32,
        center_z: i32,
        extrude: bool,
        rx: i32,
        rz: i32,
        out: &mut Vec<u32>,
    ) {
        let ix = (center_x + rx).rem_euclid(self.width as i32) as usize;
        let iz = (center_z + rz).rem_euclid(self.height as i32) as usize;
        let bits = self.cells[ix + iz * self.width];
        if bits & CELL_FILLED == 0 {
            return;
        }
        if extrude {
            self.build_extruded_cell(rel, rx, rz, bits, out);
        } else {
            // FAST：仅底面，用顶色（26.1 buildFlatCell: DOWN | FLAG_USE_TOP_COLOR）
            encode_face(out, rx, rz, Dir::Down, FLAG_USE_TOP_COLOR);
        }
    }

    fn build_extruded_cell(&self, rel: RelCamPos, x: i32, z: i32, bits: u8, out: &mut Vec<u32>) {
        if rel != RelCamPos::Below {
            encode_face(out, x, z, Dir::Up, 0);
        }
        if rel != RelCamPos::Above {
            encode_face(out, x, z, Dir::Down, 0);
        }
        // 侧面：相邻为空且面朝相机一侧才生成（26.1 L309-322）
        if bits & (1 << 3) != 0 && z > 0 {
            encode_face(out, x, z, Dir::North, 0);
        }
        if bits & (1 << 1) != 0 && z < 0 {
            encode_face(out, x, z, Dir::South, 0);
        }
        if bits & (1 << 0) != 0 && x > 0 {
            encode_face(out, x, z, Dir::West, 0);
        }
        if bits & (1 << 2) != 0 && x < 0 {
            encode_face(out, x, z, Dir::East, 0);
        }
        // 相机附近 3×3 单元加内壁面（穿云时可见内壁，26.1 L325-330）
        if x.abs() <= 1 && z.abs() <= 1 {
            for d in [
                Dir::Down,
                Dir::Up,
                Dir::North,
                Dir::South,
                Dir::West,
                Dir::East,
            ] {
                encode_face(out, x, z, d, FLAG_INSIDE_FACE);
            }
        }
    }
}

fn rel_discriminant(rel: &RelCamPos) -> u8 {
    match rel {
        RelCamPos::Above => 0,
        RelCamPos::Inside => 1,
        RelCamPos::Below => 2,
    }
}

/// 打包一面，对应 26.1 `encodeFace`（x>>1、z>>1 各存 8bit，低位补位存
/// flags 的 bit7/bit6），此处折叠为单个 u32 实例属性。
fn encode_face(out: &mut Vec<u32>, x: i32, z: i32, dir: Dir, flags: u32) {
    let mut dir_and_flags = dir as u32 | flags;
    dir_and_flags |= ((x as u32 & 1) << 7) | ((z as u32 & 1) << 6);
    out.push((((x >> 1) as u32 & 0xFF) << 16) | (((z >> 1) as u32 & 0xFF) << 8) | dir_and_flags);
}
