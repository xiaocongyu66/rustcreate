#![cfg(feature = "gpu-tests")]

//! 掉落物渲染离屏验证（lavapipe / CI render-headless job，#150 刀 1）。
//!
//! 断言策略与 particles.rs 一致：像素统计 + 同场景 diff，不做黄金像素，
//! 阈值宽松防驱动 flake。窗内投掷掉落物 → 帧差出图标像素样本：
//! - 材料物品（diamond）→ GUI 精灵表 billboard 上屏；
//! - 方块物品（stone）→ 方块图集侧面 tile billboard 上屏；
//! - bob/spin 相位（ageInTicks）驱动画面变化（原版 :48/:431-32 消费链）；
//! - 地面以下被深度剔除；
//! - 覆盖建缓冲到绘制全链 validation 无错误。

use glam::Vec3;
use mcv_render::gpu::{RenderChunk, Scene};
use mcv_render::item_render::{ItemDropDraw, MAX_DROPS};
use mcv_render::{Camera, OffscreenTarget};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Tv {
    pos: [f32; 3],
    uv: [u16; 2],
    layer: u16,
    block_light: u8,
    sky_light: u8,
    ao: u8,
    flags: u8,
    pad: [u8; 2],
}

fn ground_chunk(device: &wgpu::Device) -> RenderChunk {
    // 16x16 grass plane at y=100（与 particles.rs 同款：顶面天光草皮）。
    let y = 100.0f32;
    let mk = |p: [f32; 3], uv: [u16; 2]| Tv {
        pos: p,
        uv,
        layer: mcv_core::tiles::GRASS_TOP,
        block_light: 0,
        sky_light: 15,
        ao: 3,
        flags: 2, // face_id +Y
        pad: [0; 2],
    };
    let verts = [
        mk([0.0, y, 0.0], [0, 0]),
        mk([0.0, y, 16.0], [0, 65535]),
        mk([16.0, y, 16.0], [65535, 65535]),
        mk([16.0, y, 0.0], [65535, 0]),
    ];
    let idx: [u32; 6] = [0, 1, 2, 0, 2, 3];
    RenderChunk {
        water_index_buf: None,
        origin: [0.0, 0.0, 0.0],
        vertex_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("item-drop-test-vbuf"),
            contents: bytemuck::bytes_of(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        index_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("item-drop-test-ibuf"),
            contents: bytemuck::bytes_of(&idx),
            usage: wgpu::BufferUsages::INDEX,
        }),
        opaque_range: 0..6,
        water_parts: Vec::new(),
        aabb: (Vec3::new(0.0, y - 0.1, 0.0), Vec3::new(16.0, y + 0.1, 16.0)),
    }
}

fn setup() -> (wgpu::Device, wgpu::Queue, mcv_render::Renderer) {
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
        label: Some("item-drop-test"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("device");
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/minecraft");
    let renderer = mcv_render::Renderer::new(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Some(&assets),
    );
    (device, queue, renderer)
}

/// diff 像素数 + 包围盒（particles.rs 同款，320 宽展开）。
fn diff_pixels(a: &[u8], b: &[u8]) -> (usize, Option<(u32, u32, u32, u32)>) {
    let mut changed = 0;
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for i in 0..a.len() / 4 {
        if a[i * 4..i * 4 + 3] != b[i * 4..i * 4 + 3] {
            changed += 1;
            let (x, y) = ((i % 320) as u32, (i / 320) as u32);
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    (changed, bbox)
}

fn camera() -> Camera {
    // 贴近掉落物（quad 0.25 块，距离 ~3 屏占 ~13px；俯角对准悬浮点）。
    Camera {
        pos: Vec3::new(8.0, 101.5, 11.0),
        yaw: 0.0,
        pitch: -0.42,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
        bob: glam::Mat4::IDENTITY,
    }
}

fn drop(icon: mcv_render::HandItem, pos: [f32; 3], age_ticks: f32) -> ItemDropDraw {
    ItemDropDraw {
        pos,
        age_ticks,
        phase: 0.0,
        icon,
        light: [0, 15], // 露天：天光满
    }
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut mcv_render::Renderer,
    target: &OffscreenTarget,
    chunk: &RenderChunk,
    drops: Option<&[ItemDropDraw]>,
) -> Vec<u8> {
    let cam = camera();
    let hud: Vec<mcv_render::gpu::HudQuad> = Vec::new();
    let (sun, day) = mcv_render::sun_state(6000);
    let scene = Scene {
        camera: &cam,
        time: 0.0,
        day_factor: day,
        fog_tint: [1.0, 1.0, 1.0],
        fog_env: [0.0, 1024.0],
        fog_rd_end: 1024.0,
        water_vision: 0.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(chunk),
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None,
        hand: None,
        drops,
    };
    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut enc);
    queue.submit([enc.finish()]);
    target.read_pixels(device)
}

/// 材料物品（diamond sprite）与方块物品（stone tile）billboard 都上屏；
/// bob/spin 相位驱动画面变化；地面以下深度剔除。
#[test]
fn item_drops_paint_billboard_icons() {
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let guard = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let chunk = ground_chunk(&device);

    let base = render(&device, &queue, &mut renderer, &target, &chunk, None);

    // 窗内投掷：草皮上悬浮的 diamond 掉落物（feet=地面上）。
    let sprite_drops = [drop(
        mcv_render::HandItem::Sprite("diamond"),
        [8.0, 100.0, 8.0],
        0.0,
    )];
    let with_sprite = render(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        Some(&sprite_drops),
    );
    // 方块物品：石头 tile（方块图集，tex_set=1 路径）。
    let block_drops = [drop(mcv_render::HandItem::Block(1), [8.0, 100.0, 8.0], 0.0)];
    let with_block = render(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        Some(&block_drops),
    );

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        for (name, px) in [
            ("item-drops-sprite.png", &with_sprite),
            ("item-drops-block.png", &with_block),
        ] {
            let png = mcv_render::offscreen::encode_png(extent.width, extent.height, px);
            let _ = std::fs::write(std::path::Path::new(&dir).join(name), png);
        }
    }

    let (sprite_diff, sprite_bb) = diff_pixels(&base, &with_sprite);
    assert!(
        sprite_diff > 40,
        "材料物品掉落物应上屏图标像素：diff={sprite_diff}（bbox {sprite_bb:?}）"
    );
    let (block_diff, block_bb) = diff_pixels(&base, &with_block);
    assert!(
        block_diff > 40,
        "方块物品掉落物应上屏 tile 像素：diff={block_diff}（bbox {block_bb:?}）"
    );
    // 掉落物在瞄准点附近（画面中心带）：bbox 紧凑，不越出中带。
    if let Some((x0, y0, x1, y1)) = sprite_bb {
        assert!(
            x1 - x0 < 120 && y1 - y0 < 120,
            "sprite 簇 bbox 紧凑: {sprite_bb:?}"
        );
    }
    if let Some((x0, y0, x1, y1)) = block_bb {
        assert!(
            x1 - x0 < 120 && y1 - y0 < 120,
            "block 簇 bbox 紧凑: {block_bb:?}"
        );
    }

    // bob/spin 相位消费链：ageInTicks 驱动画面变化（原版浮动 :48 + 自转
    // :431-432——age=π/2·10 时 bob 峰值 +0.1、spin 45°）。快照字段停在
    // CPU 而未进顶点时本断言 diff=0 即红。
    let aged_drops = [drop(
        mcv_render::HandItem::Sprite("diamond"),
        [8.0, 100.0, 8.0],
        15.707_963,
    )];
    let aged = render(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        Some(&aged_drops),
    );
    let (age_diff, _) = diff_pixels(&with_sprite, &aged);
    assert!(
        age_diff > 20,
        "ageInTicks（bob/spin）未被渲染消费：diff={age_diff}"
    );

    // 深度：掉落物在草皮之下 → 深度剔除（cutout 写深度，实体不透明相）。
    let buried_drops = [drop(
        mcv_render::HandItem::Sprite("diamond"),
        [8.0, 99.0, 8.0],
        0.0,
    )];
    let buried = render(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        Some(&buried_drops),
    );
    let (buried_diff, _) = diff_pixels(&base, &buried);
    assert!(
        buried_diff < 20,
        "地面下的掉落物必须被深度剔除：diff={buried_diff}"
    );

    let scope_err = pollster::block_on(guard.pop());
    assert!(
        scope_err.is_none(),
        "掉落物 pass 产生 validation error: {scope_err:?}"
    );
}

/// 超上限截断：MAX_DROPS+1 枚同点掉落物 → 截断到上限绘制，不越界、
/// 不产生 validation error（索引缓冲按 MAX_DROPS 预生成的契约锁）。
#[test]
fn item_drops_cap_truncates_without_validation_error() {
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let guard = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let chunk = ground_chunk(&device);
    let many: Vec<ItemDropDraw> = (0..=MAX_DROPS)
        .map(|i| {
            drop(
                mcv_render::HandItem::Sprite("diamond"),
                [8.0, 100.0, 8.0],
                i as f32 * 0.05,
            )
        })
        .collect();
    let with_many = render(&device, &queue, &mut renderer, &target, &chunk, Some(&many));
    // 截断后仍画出上限枚（首枚 age=0 与 base 的差异簇存在即可）。
    let base = render(&device, &queue, &mut renderer, &target, &chunk, None);
    let (d, _) = diff_pixels(&base, &with_many);
    assert!(d > 40, "截断批仍须上屏（diff={d}）");
    let scope_err = pollster::block_on(guard.pop());
    assert!(
        scope_err.is_none(),
        "截断路径 validation error: {scope_err:?}"
    );
}
