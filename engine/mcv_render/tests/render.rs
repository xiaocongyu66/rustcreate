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
    // 相机贴近目标方块（玩法语义：挖掘距离 ≤5）：quad 屏占 ~30px，
    // LOD<0.5 落在 mip0 全分辨率裂纹上；旧机位 dist≈20 → LOD 舍入到
    // mip1（8x8 稀疏裂纹大概率采到 alpha=0 全 discard）。
    let camera = Camera {
        pos: Vec3::new(8.0, 103.0, 14.0),
        yaw: 0.0,
        pitch: -0.5,
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

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        let png = mcv_render::offscreen::encode_png(extent.width, extent.height, &with);
        let _ = std::fs::write(std::path::Path::new(&dir).join("overlay-test.png"), png);
    }

    // 差分断言：同场景两次渲染只应差在 overlay 投影区（裂纹/描边把草地
    // 明显压暗）。按像素 diff 计数，不依赖草地底色，抗驱动差异。
    let (changed, bbox) = a_diff_pixels(&base, &with);
    assert!(
        changed > 40,
        "overlay should visibly darken target region, changed={changed} bbox={bbox:?}"
    );
}

/// diff 像素数 + 包围盒 (min_x,min_y,max_x,max_y)（无 diff 时 None），
/// 断言消息带 bbox 便于从 CI 日志定位投影区。
fn a_diff_pixels(a: &[u8], b: &[u8]) -> (usize, Option<(u32, u32, u32, u32)>) {
    let w = 320u32;
    let mut n = 0usize;
    let mut bb: Option<(u32, u32, u32, u32)> = None;
    for (i, (p, q)) in a.chunks(4).zip(b.chunks(4)).enumerate() {
        let dp: i32 = p[..3].iter().map(|&v| v as i32).sum();
        let dq: i32 = q[..3].iter().map(|&v| v as i32).sum();
        if (dp - dq).abs() > 40 {
            n += 1;
            let (x, y) = ((i as u32) % w, (i as u32) / w);
            bb = Some(match bb {
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                None => (x, y, x, y),
            });
        }
    }
    (n, bb)
}

/// GLES 后端守护（2026-10-10 真机两连 fatal：cloud.wgsl naga 校验、GLES 拒绝
/// 对创建期映射 buffer 做 queue.write_buffer）。CI render-headless 全走
/// lavapipe Vulkan，GLES 回退后端的 buffer 映射/上传语义从未覆盖。本测试
/// 向 adapter 要 fallback：拿到 swrast GL 就用 MeshUploader（出事入口）+ 云 +
/// HUD 渲一整帧；fallback 仍是 Vulkan 或无 GL/EGL 环境则跳过，不为守护引入
/// flake。
#[test]
fn gles_fallback_world_frame_smoke() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: true,
        apply_limit_buckets: false,
    })) else {
        return;
    };
    if !matches!(adapter.get_info().backend, wgpu::Backend::Gl) {
        return; // fallback 仍是 Vulkan（如 lavapipe），本环境无新增覆盖
    }
    if adapter.limits().max_texture_array_layers < mcv_core::atlas::LAYERS as u32 {
        return; // swrast 数组层数装不下图集，跳过
    }
    let mut limits = wgpu::Limits::downlevel_defaults();
    limits.max_texture_array_layers = adapter.limits().max_texture_array_layers;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gl-smoke"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("gl device");
    let mut renderer = mcv_render::Renderer::new(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    let clouds = mcv_render::Clouds::new(&device, &queue);
    // 事故现场复跑：MeshUploader 创建期映射写入路径（GLES 曾在此 fatal）。
    let up = mcv_render::gpu::MeshUploader::new(device.clone(), queue.clone());
    let y = 100.0f32;
    let mk = |p: [f32; 3], uv: [u16; 2]| Tv {
        pos: p,
        uv,
        layer: mcv_core::tiles::GRASS_TOP,
        block_light: 0,
        sky_light: 15,
        ao: 3,
        flags: 2,
        pad: [0; 2],
    };
    let verts = [
        mk([0.0, y, 0.0], [0, 0]),
        mk([0.0, y, 16.0], [0, 65535]),
        mk([16.0, y, 16.0], [65535, 65535]),
        mk([16.0, y, 0.0], [65535, 0]),
    ];
    let idx: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let chunk = up.build_chunk([0.0, 0.0, 0.0], bytemuck::cast_slice(&verts), &idx, None);

    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
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
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: day,
        sun_dir: sun,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(&chunk),
        hud: &hud,
        cloud: Some((&clouds, mcv_render::CloudSettings::default())),
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
        let _ = std::fs::write(std::path::Path::new(&dir).join("gles-smoke.png"), png);
    }
    // GL 回退下草地必须仍然画得出（颜色路径跨后端一致性弱，阈值放宽）。
    let (green, _) = sample_stats(&rgba, extent.width, extent.height);
    assert!(green > 0.15, "GLES 回退下草地缺失，green={green}");
}

#[test]
fn vec4_identity() {
    // keeps glam import used in feature-off builds
    let v = Vec4::ONE;
    assert_eq!(v.w, 1.0);
}

/// 云管线回归锁（2026-10-10 真机闪退：cloud.wgsl `i32 | u32` 混合符号性
/// 位或被 naga 拒绝，Android 真机 create_shader_module fatal）。既有离屏
/// 测试全部 `cloud: None`，云管线从未在 CI 编译过。`Clouds::new` 内
/// `create_shader_module("cloud")` 即校验点；绘制断言用「云开/云关同机位
/// 同帧」的确定性像素差分（clouds.png 编译期嵌入，相位由固定 time/cam
/// 决定，逐位可复现），相机仰视、无地形，天空带必被云面覆盖。
#[test]
fn cloud_pipeline_compiles_and_paints_sky() {
    let (device, queue, mut renderer) = setup();
    let clouds = mcv_render::Clouds::new(&device, &queue);
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    // 相机在云层（底 192.33）下方仰视：pitch>0 朝上（同 terrain 测试的
    // pitch<0 俯地约定）。无地形、空 HUD，纯天空 + 云。
    let camera = Camera {
        pos: Vec3::new(8.0, 110.0, 8.0),
        yaw: 0.0,
        pitch: 1.1,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 512.0,
    };
    let empty_hud: &[HudQuad] = &[];
    let mut frame = |cloud_on: bool| -> Vec<u8> {
        let scene = Scene {
            camera: &camera,
            time: 0.0,
            day_factor: 1.0,
            sun_dir: Vec3::Y,
            width: 320.0,
            height: 240.0,
            chunks: &[],
            hud: empty_hud,
            cloud: cloud_on.then_some((&clouds, mcv_render::CloudSettings::default())),
            player: None,
            overlay: None,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
        target.enqueue_copy(&mut encoder);
        queue.submit([encoder.finish()]);
        target.read_pixels(&device)
    };
    let (off, on) = (frame(false), frame(true));
    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        for (name, px) in [("cloud-off.png", &off), ("cloud-on.png", &on)] {
            let png = mcv_render::offscreen::encode_png(extent.width, extent.height, px);
            let _ = std::fs::write(std::path::Path::new(&dir).join(name), png);
        }
    }
    let (n, bbox) = a_diff_pixels(&off, &on);
    // 断言只为「云管线真的在画」（真机闪退案的 CI 洞：云 pass 从未被
    // 执行过）。云图 clouds.png 仅 28% 单元占用，白色×0.7 面明暗经 sRGB
    // 编码后与亮天空底色接近，alpha 混合后逐像素差大多不足阈值——lavapipe
    // 实测 diff≈1.5k（阈值 3 倍余量即可），静默跳过场景 diff 恰为 0。
    // bbox 横向跨度防单点噪声假阳：云是横贯视场的整片平面。
    assert!(
        n > 400,
        "云开/云关天空带几乎无差（diff={n}, bbox={bbox:?}）——云管线可能被静默跳过"
    );
    let (x0, _y0, x1, _y1) = bbox.expect("diff>0 必有包围盒");
    assert!(x1 - x0 > 200, "云 diff 未横贯天空带，bbox={bbox:?}");
}
