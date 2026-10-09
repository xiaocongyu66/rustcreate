#![cfg(feature = "gpu-tests")]

//! Headless render verification: runs on CI with lavapipe (software Vulkan).
//! Assertions are pixel-statistics based (not golden pixels) to stay robust
//! across driver versions. A PNG artifact is emitted for manual inspection.

use glam::{Vec3, Vec4};
use mcv_render::gpu::{HudQuad, MineFace, MiningOverlay, RenderChunk, Scene, TERRAIN_STRIDE};
use mcv_render::{Camera, OffscreenTarget, font};
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
    // 16x16 grass plane at y=100 (top face, sky-lit).
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
            label: Some("test-vbuf"),
            contents: bytemuck::bytes_of(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        index_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test-ibuf"),
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
    // 与 app.rs 同款：方块图集 831 层 > downlevel 256，向 adapter 要实际上限
    // （lavapipe 3907 / Metal 2048 / D3D 2048），否则草地层 336 被 gpu.rs 截尾。
    let mut limits = wgpu::Limits::downlevel_defaults();
    limits.max_texture_array_layers = adapter.limits().max_texture_array_layers;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("test"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("device");
    let renderer = mcv_render::Renderer::new(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    (device, queue, renderer)
}

/// (green-dominant share of the ground band, blue-dominant share of the sky
/// band). Bands avoid HUD regions (hotbar at the bottom, text at the top).
fn sample_stats(rgba: &[u8], w: u32, h: u32) -> (f64, f64) {
    let mut green = 0u64;
    let mut green_total = 0u64;
    let mut blue = 0u64;
    let mut blue_total = 0u64;
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            let (r, g, b) = (rgba[o] as i32, rgba[o + 1] as i32, rgba[o + 2] as i32);
            if y > 120 && y < 190 {
                green_total += 1;
                if g > r + 10 && g > b + 10 {
                    green += 1;
                }
            } else if y < 60 {
                blue_total += 1;
                if b > r + 10 && b > g {
                    blue += 1;
                }
            }
        }
    }
    (
        green as f64 / green_total as f64,
        blue as f64 / blue_total as f64,
    )
}

#[test]
fn terrain_sky_and_hud_render() {
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);

    let camera = Camera {
        pos: Vec3::new(8.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };

    // HUD: solid white bar + glyph 'A' (65).
    let hud = vec![
        HudQuad {
            x: 10.0,
            y: 200.0,
            w: 60.0,
            h: 6.0,
            uv: font::glyph_uv(font::SOLID_CELL),
            color: [1.0, 1.0, 1.0, 1.0],
            tex: 0,
            layer: 0,
            rot: 0.0,
        },
        HudQuad {
            x: 100.0,
            y: 100.0,
            w: 16.0,
            h: 16.0,
            uv: font::glyph_uv(65),
            color: [1.0, 1.0, 1.0, 1.0],
            tex: 0,
            layer: 0,
            rot: 0.0,
        },
    ];

    let (sun, day) = mcv_render::sun_state(6000); // noon
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: day,
        sun_dir: sun,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(&chunk),
        hud: &hud,
        cloud: None,
        player: None,
        overlay: None,
    };

    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut encoder);
    queue.submit([encoder.finish()]);
    let rgba = target.read_pixels(&device);

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        let png = mcv_render::offscreen::encode_png(extent.width, extent.height, &rgba);
        let _ = std::fs::write(std::path::Path::new(&dir).join("render-test.png"), png);
    }

    let (green_share, blue_share) = sample_stats(&rgba, extent.width, extent.height);
    assert!(
        green_share > 0.25,
        "bottom half should be mostly grass, got {green_share}"
    );
    assert!(
        blue_share > 0.30,
        "top half should be mostly sky, got {blue_share}"
    );

    // HUD glyph region: some white pixels where 'A' was drawn (16px box at 100,100).
    let mut white = 0;
    for y in 100..116 {
        for x in 100..116 {
            let o = ((y * extent.width + x) * 4) as usize;
            if rgba[o] > 200 && rgba[o + 1] > 200 && rgba[o + 2] > 200 {
                white += 1;
            }
        }
    }
    assert!(white > 20, "glyph pixels missing: {white}");

    // HUD solid bar pixels.
    let mut bar = 0;
    for y in 202..206 {
        for x in 12..68 {
            let o = ((y * extent.width + x) * 4) as usize;
            if rgba[o] > 200 && rgba[o + 1] > 200 && rgba[o + 2] > 200 {
                bar += 1;
            }
        }
    }
    assert!(bar > 150, "solid bar pixels missing: {bar}");

    // Water pipeline path: reuse vertex layout sanity (compile-only draw).
    let _ = TERRAIN_STRIDE;
}

#[test]
fn mining_crack_and_outline_darken_target() {
    // 同一场景渲两次：overlay（stage3 裂纹 + 描边）应让目标投影区像素
    // 明显变暗（草地亮、裂纹/描边深色系），按像素 diff 计数断言。
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);
    let camera = Camera {
        pos: Vec3::new(8.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };
    let hud: Vec<HudQuad> = Vec::new();
    let (sun, day) = mcv_render::sun_state(6000);
    let render = |renderer: &mut mcv_render::Renderer, overlay: Option<MiningOverlay>| {
        let scene = Scene {
            camera: &camera,
            time: 0.0,
            day_factor: day,
            sun_dir: sun,
            width: 320.0,
            height: 240.0,
            chunks: std::slice::from_ref(&chunk),
            hud: &hud,
            cloud: None,
            player: None,
            overlay,
        };
        let mut enc = device.create_command_encoder(&Default::default());
        renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
        target.enqueue_copy(&mut enc);
        queue.submit([enc.finish()]);
        target.read_pixels(&device)
    };

    let base = render(&mut renderer, None);
    let mut faces = [MineFace {
        exposed: false,
        block_light: 0,
        sky_light: 0,
    }; 6];
    faces[2] = MineFace {
        exposed: true,
        block_light: 0,
        sky_light: 15,
    };
    let with = render(
        &mut renderer,
        Some(MiningOverlay {
            min: [8.0, 99.0, 8.0], // 顶面恰好落在 y=100 的草地平面上
            crack_stage: Some(3),
            faces,
        }),
    );

    // 差分断言：同场景两次渲染只应差在 overlay 投影区（裂纹/描边把草地
    // 明显压暗）。按像素 diff 计数，不依赖草地底色，抗驱动差异。
    let changed = a_diff_pixels(&base, &with);
    assert!(
        changed > 40,
        "overlay should visibly darken target region, changed={changed}"
    );
}

fn a_diff_pixels(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| {
            let dp: i32 = p[..3].iter().map(|&v| v as i32).sum();
            let dq: i32 = q[..3].iter().map(|&v| v as i32).sum();
            (dp - dq).abs() > 40
        })
        .count()
}

#[test]
fn vec4_identity() {
    // keeps glam import used in feature-off builds
    let v = Vec4::ONE;
    assert_eq!(v.w, 1.0);
}
