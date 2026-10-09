#![cfg(feature = "gpu-tests")]

//! 粒子渲染离屏验证（lavapipe / CI render-headless job）。
//!
//! 断言策略与 render.rs 一致：像素统计 + 同场景 diff，不做黄金像素，
//! 阈值宽松防驱动 flake。crack 粒子直接采样方块图集（原版
//! TerrainParticle 从被破坏方块 sprite 取 1/4 随机小矩形），
//! 因此 base/with diff 像素簇即「图集色像素簇」。

use glam::Vec3;
use mcv_render::gpu::{RenderChunk, Scene, TERRAIN_STRIDE};
use mcv_render::particles::{NoWorld, ParticleEngine};
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
    // 16x16 grass plane at y=100（与 render.rs 同款：顶面天光草皮）。
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
            label: Some("particle-test-vbuf"),
            contents: bytemuck::bytes_of(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        index_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particle-test-ibuf"),
            contents: bytemuck::bytes_of(&idx),
            usage: wgpu::BufferUsages::INDEX,
        }),
        opaque_range: 0..6,
        water_range: 0..0,
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
        label: Some("particle-test"),
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

/// diff 像素数 + 包围盒（同 render.rs 的 a_diff_pixels）。
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
    // 贴近粒子块（语义同挖掘：quad 屏占可见；距离近避免 mip0 之外 LOD）。
    Camera {
        pos: Vec3::new(8.0, 103.0, 14.0),
        yaw: 0.0,
        pitch: -0.5,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    }
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut mcv_render::Renderer,
    target: &OffscreenTarget,
    chunk: &RenderChunk,
    particles: Option<&mcv_render::ParticleEngine>,
) -> Vec<u8> {
    let cam = camera();
    let hud: Vec<mcv_render::gpu::HudQuad> = Vec::new();
    let (sun, day) = mcv_render::sun_state(6000);
    let scene = Scene {
        camera: &cam,
        time: 0.0,
        day_factor: day,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(chunk),
        hud: &hud,
        cloud: None,
        player: None,
        overlay: None,
        particles: particles.map(|p| (p, 0.0_f32)),
    };
    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut enc);
    queue.submit([enc.finish()]);
    target.read_pixels(device)
}

#[test]
fn block_crack_particles_paint_atlas_pixels() {
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);

    let base = render(&device, &queue, &mut renderer, &target, &chunk, None);

    // 一整块石头的破坏爆裂：满块 4×4×4 = 64 粒（ClientLevel.java:951-953），
    // 浮在草皮上方一格（y 100..101，无遮挡）。
    let mut engine = ParticleEngine::with_seed(0xC0FFEE);
    engine.spawn_block_crack([8.0, 100.0, 8.0], mcv_core::tiles::STONE, 0);
    assert_eq!(engine.len(), 64, "full-cube crack = 64 particles");
    engine.tick(&NoWorld);
    let with = render(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        Some(&engine),
    );

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        let png = mcv_render::offscreen::encode_png(extent.width, extent.height, &with);
        let _ = std::fs::write(std::path::Path::new(&dir).join("particles-test.png"), png);
    }

    // 粒子簇出现在画面上：diff 像素 > 150（64 粒散布 quad，每粒 ≥2px；
    // 阈值宽松，只有全部粒子都未画出才可能低于）。
    let (changed, bbox) = diff_pixels(&base, &with);
    assert!(
        changed > 150,
        "crack particles should paint atlas pixels, changed={changed} bbox={bbox:?}"
    );
    // 粒子簇应紧凑投影在目标块附近（屏幕中带），而不是全屏噪声：
    // bbox 宽高都不超过画面一半。
    if let Some((x0, y0, x1, y1)) = bbox {
        assert!(
            x1 - x0 < 160 && y1 - y0 < 120,
            "cluster bbox sane: {bbox:?}"
        );
    }
}

#[test]
fn splash_particles_use_vanilla_particle_atlas() {
    // tex_set=1 路径：原版 textures/particle/splash_*.png 装进 particles
    // 数组（缺素材回退程序化占位——本测试喂仓库素材，验证原版路径）。
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);

    let base = render(&device, &queue, &mut renderer, &target, &chunk, None);

    let mut engine = ParticleEngine::with_seed(0xCAFE);
    // 水花喷在相机近前（splash 贴图 8×8、quad 小，须贴近才屏占足够；
    // y 浮空无遮挡）。
    engine.spawn_water_splash(8.0, 102.0, 9.0, [0.0, 0.0, 0.0], 0.6);
    engine.tick(&NoWorld);
    let with = render(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        Some(&engine),
    );

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        let png = mcv_render::offscreen::encode_png(extent.width, extent.height, &with);
        let _ = std::fs::write(
            std::path::Path::new(&dir).join("particles-splash-test.png"),
            png,
        );
    }

    let (changed, _) = diff_pixels(&base, &with);
    assert!(
        changed > 40,
        "splash particles should paint, changed={changed}"
    );
}

/// 粒子被地形遮挡（深度只读 + LessEqual）：粒子放到地面以下时不可见。
#[test]
fn particles_respect_depth() {
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);

    let base = render(&device, &queue, &mut renderer, &target, &chunk, None);

    let mut engine = ParticleEngine::with_seed(0xBEEF);
    // y 98..99 全在草皮（y=100）之下 → 深度测试剔除。
    engine.spawn_block_crack([8.0, 98.0, 8.0], mcv_core::tiles::STONE, 0);
    let with = render(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        Some(&engine),
    );
    let (changed, _) = diff_pixels(&base, &with);
    assert!(
        changed < 40,
        "buried particles must be depth-culled, changed={changed}"
    );
}

#[test]
fn terrain_stride_layout_unchanged() {
    // 防呆：粒子顶点独立布局（48B），不与 terrain 顶点（24B）混淆。
    assert_eq!(TERRAIN_STRIDE, 24);
    assert_eq!(size_of::<mcv_render::particles::ParticleVertex>(), 48);
}
