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
        mk([0.0, y, 16.0], [0, 4096]),
        mk([16.0, y, 16.0], [4096, 4096]),
        mk([16.0, y, 0.0], [4096, 0]),
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
        // #80 合批：单块条目无水分段（旧 water_range: 0..0 的等价形态）。
        water_parts: Vec::new(),
        aabb: (Vec3::new(0.0, y - 0.1, 0.0), Vec3::new(16.0, y + 0.1, 16.0)),
    }
}

fn ground_chunk_at(device: &wgpu::Device, origin: [f32; 3]) -> RenderChunk {
    let mut rc = ground_chunk(device);
    rc.origin = origin;
    rc.aabb = (
        Vec3::new(origin[0], 99.9, origin[2]),
        Vec3::new(origin[0] + 16.0, 100.1, origin[2] + 16.0),
    );
    rc
}

fn setup() -> (wgpu::Device, wgpu::Queue, mcv_render::Renderer) {
    setup_with_assets(Some(&workspace_assets()))
}

/// 防纯色回归（2026-10-10 真机事故：方块全呈纯绿/纯棕、无任何花纹）。
/// 原版草 tile 是灰度噪声图（colortype 0），正确采样下地面带会有几十种
/// 颜色（噪声×tint×雾渐变）；uv 失效恒采样 tile 角点、或贴图上载退化成
/// 单色 → 颜色数塌缩成 1~2。旧的「绿色占比」断言对纯色失明（纯色也绿），
/// 这个洞由此断言堵死。
#[test]
fn grass_surface_contains_pattern_not_flat_tint() {
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
    let hud: Vec<HudQuad> = vec![];
    let (sun, day) = mcv_render::sun_state(6000); // noon
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: day,
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(&chunk),
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None,
        hand: None,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut encoder);
    queue.submit([encoder.finish()]);
    let rgba = target.read_pixels(&device);

    // 地面带（与 sample_stats 同带）distinct 颜色数，32 级/通道量化：
    // 草 tile 灰度噪声经 tint 后仍有数十个量化格；纯色塌缩到 1~2。
    let mut colors = std::collections::HashSet::new();
    let mut green_total = 0u64;
    for y in 121..190u32 {
        for x in 0..extent.width {
            let o = ((y * extent.width + x) * 4) as usize;
            let (r, g, b) = (rgba[o], rgba[o + 1], rgba[o + 2]);
            if g > r.saturating_add(10) && g > b.saturating_add(10) {
                green_total += 1;
                colors.insert((r >> 3, g >> 3, b >> 3));
            }
        }
    }
    assert!(
        colors.len() >= 8,
        "grass band must carry tile pattern (vanilla grayscale noise × tint), got {} distinct colors on {green_total} green px — flat tint means uv/texel pipeline regression",
        colors.len()
    );
}

/// 真机症状守护（2026-10-10「动一下山没了」）：origins buffer 按可见列表
/// 连续打包写入，而 draw 曾按 scene.chunks 原始索引取 origin——剔除任一区块
/// 后其后所有区块画错位置。可见区块 origin 非零：错位会读零值空槽，整块
/// 地形被画到视锥外，绿色占比崩塌。
#[test]
fn frustum_cull_does_not_shift_chunk_origins() {
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let culled = ground_chunk_at(&device, [600.0, 0.0, 600.0]);
    let visible = ground_chunk_at(&device, [64.0, 0.0, 0.0]);

    let camera = Camera {
        pos: Vec3::new(72.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };
    let hud: Vec<HudQuad> = vec![];
    let (sun, day) = mcv_render::sun_state(6000); // noon
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: day,
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: &[culled, visible],
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None,
        hand: None,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut encoder);
    queue.submit([encoder.finish()]);
    let rgba = target.read_pixels(&device);
    let (green_share, _) = sample_stats(&rgba, extent.width, extent.height);
    assert!(
        green_share > 0.25,
        "chunk behind a culled chunk must render at its own origin (slot misalignment regression), got {green_share}"
    );
}

/// 喂仓库内原版素材的 setup（正常部署路径）：裂纹层吃真实 destroy_stage、
/// 天空吃真实 environment/celestial、草地吃真实 grass_block_top + 生物群系
/// 染色（mcv_core::tint plains 基线）。绿色断言从此对真实素材成立。
/// 素材缺失路径（原「冒烟测试拆双路径」的 None 分支）由
/// [`missing_assets_never_paint_fake_pixels`] 专门守护——渲染器在该路径
/// 只产出原版 missing 标记，无任何程序化假贴图。
fn setup_with_assets(
    assets: Option<&std::path::Path>,
) -> (wgpu::Device, wgpu::Queue, mcv_render::Renderer) {
    setup_with_layer_cap_and_assets(None, assets)
}

/// `layer_cap = Some(n)`：把单数组层数上限强制为 min(n, adapter 上限)。
/// lavapipe 上限 3907 永远走单数组，GLES 256 层设备才会拆多数组——用它
/// 在 CI 上强制走多数组拆分路径（堵「层数不够就 return」的覆盖洞）。
fn setup_with_layer_cap_and_assets(
    layer_cap: Option<u32>,
    assets: Option<&std::path::Path>,
) -> (wgpu::Device, wgpu::Queue, mcv_render::Renderer) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no adapter: install mesa-vulkan-drivers for lavapipe");
    // 与 app.rs 同款：方块图集 838 层（827 真实 + 1 missing 哨兵 + 10 裂纹）> downlevel 256，
    // 向 adapter 要实际上限（lavapipe 3907 / Metal 2048 / D3D 2048），
    // 否则草地层 336 被 gpu.rs 截尾。
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
    let renderer = mcv_render::Renderer::with_atlas_layer_cap(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        assets,
        layer_cap,
    );
    (device, queue, renderer)
}

/// 工作区 assets/minecraft 资源根（仓库内已提交原版贴图）。测试显式传给
/// Renderer::new，让裂纹层吃到原版 destroy_stage_0..9、天空吃到原版
/// environment/celestial/{sun,moon/*}、染色吃到 colormap/{grass,foliage}
/// ——与 app.rs 桌面路径同源。
fn workspace_assets() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/minecraft")
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
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(&chunk),
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
        hand: None,
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

/// 多数组拆分路径回归（2026-10-10 GLES 图集截断洞）：设备
/// max_texture_array_layers < LAYERS（GLES 保底 256）时方块图集拆 4 个数组、
/// shader 按 layer 区间选数组。lavapipe 上限 3907 永远不会自然走这条路
///（旧 gles 冒烟测试遇到层数不够直接 return——正是 CI 洞），这里用
/// layer_cap=256 强制 cdiv(838,256)=4 数组路径：GRASS_TOP=336 → 数组 1
/// 局部 126（边界映射由 mcv_core::tests::atlas::remap_layer_boundaries
/// 锁定），断言草地仍渲出。
#[test]
fn terrain_multi_array_grass_render() {
    // 必须喂真实素材：nofake 后无素材=全 missingno 品红，草地绿色断言
    // 只可能来自原版贴图（程序化回退已删）。
    let (device, queue, mut renderer) =
        setup_with_layer_cap_and_assets(Some(256), Some(&workspace_assets()));
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
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: day,
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(&chunk),
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
        hand: None,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut encoder);
    queue.submit([encoder.finish()]);
    let rgba = target.read_pixels(&device);
    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        let png = mcv_render::offscreen::encode_png(extent.width, extent.height, &rgba);
        let _ = std::fs::write(
            std::path::Path::new(&dir).join("multi-array-grass.png"),
            png,
        );
    }
    // 阈值与 gles 冒烟一致（像素统计跨后端放宽）。
    let (green, _) = sample_stats(&rgba, extent.width, extent.height);
    assert!(green > 0.15, "多数组路径下草地缺失，green={green}");
}

#[test]
fn mining_crack_and_outline_darken_target() {
    // 同一场景渲两次：overlay（stage3 裂纹 + 描边）应让目标投影区像素
    // 明显变暗（草地亮、裂纹/描边深色系），按像素 diff 计数断言。
    // 喂真实素材：裂纹层走原版 destroy_stage_3（16x16 黑裂纹 + alpha），
    // 验证新 10 档图集接入后暗化不回退。
    let (device, queue, mut renderer) = setup_with_assets(Some(&workspace_assets()));
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
            fog_tint: [1.0, 1.0, 1.0],
            fog_density_mult: 1.0,
            sun_dir: sun,
            moon_phase: 0,
            width: 320.0,
            height: 240.0,
            chunks: std::slice::from_ref(&chunk),
            hud: &hud,
            cloud: None,
            player: None,
            mobs: None,
            overlay,
            underwater: false,
            particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
            hand: None,
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

/// 水 pass 上传契约回归（2026-10-10 审计 H1 的 CI 洞封堵）：bug 存活数月的
/// 根因是整仓没有任何测试驱动水 pass——build_chunk 旧签名只收水索引、水顶点
/// 字节从未上传 GPU，water draw 把基址 0 的水索引绑到 opaque 顶点缓冲上
/// （GLES 越界 INVALID_OPERATION 静默跳过、桌面越界读画垃圾）。本测试
/// **独立构造**水顶点/水索引（索引基址 0，模拟真 mesher 的裸输出——索引
/// 偏移正是 build_chunk 的职责，测试验证的就是这个偏移契约），走
/// build_chunk → draw_frame 全管线，三重断言：
/// (a) 差分（有水帧 − 无水帧）：水面确实画出像素（预期 ~3.2k px，见下方
///     投影仿真）——water_range/水索引缓冲任一环节静默跳过即 diff=0；
/// (b) 差分像素竖直位置落在天空带（y<60，既有约定）与地面带（y≥120）之间
///     ——索引偏移回归时水被画到 opaque 顶点（岸线 y=100，投影带
///     112..163）即越出本带；
/// (c) 全帧无 validation error（push/pop_error_scope）——GLES 越界索引回归
///     必抓（lavapipe 有 robust buffer access 会静默补零画错位，由 (a)/(b)
///     兜底）。
///
/// 阈值定标（非拍脑袋）：解码仓库内原版 water_still.png——16x512 条带、
/// 调色板全灰度 165..216、alpha=180（fs_water 忽略 alpha、且明示水不染色
///），水面片元 = 灰度×shade(1.0) 以 fog≈0.92 与雾色混合 → 屏上 ≈
/// 166..215 的**中性微蓝灰**（b−r≈4）。派单设想的「蓝像素 b>r+30」谓词
/// 对真实素材不成立（biome 水染色未接线是另一码事，不在本测试范围），
/// 改用「亮中性灰」签名：min(rgb)>140 且 g≤r+12 且 |g−b|≤8——草地
/// （g−r≈50+）、泥土（|g−b|≈30）、天空（g−r≈20+）均不落入。
#[test]
fn water_pass_renders_uploaded_water_vertices() {
    let (device, queue, mut renderer) = setup();
    // #80 起上传器持批记账（&mut），且上传经 flush 提交——生产序
    // stream()（flush）→ draw_frame，测试对齐之（见下方 flush 调用）。
    let mut up = mcv_render::gpu::MeshUploader::new(device.clone());
    // validation 范围：覆盖建缓冲（创建期映射写入）+ 两帧全部 pass。
    let guard = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    // 与 terrain 测试同机位：水面投影带 y 86.8..114.5、岸线带 112..163、
    // 天空带 <60（相机 pos+(0,EYE_HEIGHT,0)，pitch −0.62）。
    let camera = Camera {
        pos: Vec3::new(8.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };

    // opaque：近岸草地（y=100，z 8..16）+ 塘底泥土（y=99.5，z 0..8）。
    // 水面盖在塘底上方 0.1、比岸线低 0.4——派单语义「地面下 ~0.4」的塘面。
    let ground = |p: [f32; 3], uv: [u16; 2], layer: u16| Tv {
        pos: p,
        uv,
        layer,
        block_light: 0,
        sky_light: 15,
        ao: 3,
        flags: 2, // face_id +Y
        pad: [0; 2],
    };
    let verts = [
        ground([0.0, 100.0, 8.0], [0, 0], mcv_core::tiles::GRASS_TOP),
        ground([0.0, 100.0, 16.0], [0, 4096], mcv_core::tiles::GRASS_TOP),
        ground(
            [16.0, 100.0, 16.0],
            [4096, 4096],
            mcv_core::tiles::GRASS_TOP,
        ),
        ground([16.0, 100.0, 8.0], [4096, 0], mcv_core::tiles::GRASS_TOP),
        ground([0.0, 99.5, 0.0], [0, 0], mcv_core::tiles::DIRT),
        ground([0.0, 99.5, 8.0], [0, 4096], mcv_core::tiles::DIRT),
        ground([16.0, 99.5, 8.0], [4096, 4096], mcv_core::tiles::DIRT),
        ground([16.0, 99.5, 0.0], [4096, 0], mcv_core::tiles::DIRT),
    ];
    let idx: [u32; 12] = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];

    // 水：塘面 y=99.6、z 0..8，满 tile uv（0..4096），独立构造、索引基址 0。
    // flags = +Y face_id | 波浪位（真 mesher：顶面上是空气 → wave=1，
    // mesher.cpp emit_quad；ao4=0xFF → ao=3 全角全亮）。
    let water_v = |p: [f32; 3], uv: [u16; 2]| Tv {
        pos: p,
        uv,
        layer: mcv_core::tiles::WATER, // water_still，manifest 层 789
        block_light: 0,
        sky_light: 15,
        ao: 3,
        flags: 0x2 | 0x8,
        pad: [0; 2],
    };
    let wverts = [
        water_v([0.0, 99.6, 0.0], [0, 0]),
        water_v([0.0, 99.6, 8.0], [0, 4096]),
        water_v([16.0, 99.6, 8.0], [4096, 4096]),
        water_v([16.0, 99.6, 0.0], [4096, 0]),
    ];
    let widx: [u32; 6] = [0, 1, 2, 0, 2, 3];

    let wet = up.build_chunk(
        [0.0, 0.0, 0.0],
        bytemuck::cast_slice(&verts),
        &idx,
        Some((bytemuck::cast_slice(&wverts), &widx)),
    );
    let dry = up.build_chunk([0.0, 0.0, 0.0], bytemuck::cast_slice(&verts), &idx, None);

    fn frame(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut mcv_render::Renderer,
        target: &OffscreenTarget,
        chunk: &RenderChunk,
        camera: &Camera,
    ) -> Vec<u8> {
        let (sun, day) = mcv_render::sun_state(6000); // noon
        let hud: Vec<HudQuad> = Vec::new();
        let scene = Scene {
            camera,
            time: 0.0,
            day_factor: day,
            fog_tint: [1.0, 1.0, 1.0],
            fog_density_mult: 1.0,
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
            particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
            hand: None,
        };
        let mut enc = device.create_command_encoder(&Default::default());
        renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
        target.enqueue_copy(&mut enc);
        queue.submit([enc.finish()]);
        target.read_pixels(device)
    }

    let dry_px = frame(&device, &queue, &mut renderer, &target, &dry, &camera);
    let wet_px = frame(&device, &queue, &mut renderer, &target, &wet, &camera);

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        for (name, px) in [
            ("water-pass-dry.png", &dry_px),
            ("water-pass-wet.png", &wet_px),
        ] {
            let png = mcv_render::offscreen::encode_png(extent.width, extent.height, px);
            let _ = std::fs::write(std::path::Path::new(&dir).join(name), png);
        }
    }

    let scope_err = pollster::block_on(guard.pop());
    assert!(
        scope_err.is_none(),
        "水 pass 帧产生 validation error: {scope_err:?}——越界索引/缓冲接线回归（GLES 会静默跳 draw 的那类错误）"
    );

    // (a)+(b) 差分：水面像素数 + 竖直位置（天空带与地面带之间）。
    let (diff, bbox) = a_diff_pixels(&dry_px, &wet_px);
    assert!(
        diff > 800,
        "水面差分像素缺失（water pass 被静默跳过？）diff={diff}（bbox {bbox:?}）——塘面投影区 ~3.2k px"
    );
    let (_x0, y0, _x1, y1) = bbox.expect("diff>0 必有包围盒");
    assert!(
        y0 > 60 && y1 < 120,
        "水像素竖直位置越带（y {y0}..{y1}，应落在天空带 <60 与地面带 ≥120 之间）——索引偏移/基址契约回归时水会画到 opaque 顶点位（岸线投影带 112..163）"
    );

    // 水面像素签名：投影水带框（x 90..230，y 84..118）内「亮中性灰」像素。
    // 有水帧必须大量存在；无水帧同框只有草地/泥土/天空（均不落入谓词）。
    let water_gray = |px: &[u8]| -> usize {
        let mut n = 0usize;
        for y in 84..118u32 {
            for x in 90..230u32 {
                let o = ((y * extent.width + x) * 4) as usize;
                let (r, g, b) = (px[o], px[o + 1], px[o + 2]);
                let (r, g, b) = (r as i32, g as i32, b as i32);
                if r.min(g).min(b) > 140 && g <= r + 12 && (g - b).abs() <= 8 {
                    n += 1;
                }
            }
        }
        n
    };
    let wet_gray = water_gray(&wet_px);
    let dry_gray = water_gray(&dry_px);
    assert!(
        wet_gray > 800,
        "水带缺少水面像素（预期 ~3.2k）：wet_gray={wet_gray}——water_still 层采样/水 pass 接线回归"
    );
    assert!(
        dry_gray < 200,
        "无水帧出现水面签名像素 {dry_gray}——背景误判（签名谓词失效）"
    );
}

/// GLES 后端守护（2026-10-10 真机两连 fatal：cloud.wgsl naga 校验、GLES 拒绝
/// 对创建期映射 buffer 做 queue.write_buffer）。CI render-headless 全走
/// lavapipe Vulkan，GLES 回退后端的 buffer 映射/上传语义从未覆盖。本测试
/// 向 adapter 要 fallback：拿到 swrast GL 就用 MeshUploader（出事入口）+ 云 +
/// HUD 渲一整帧；fallback 仍是 Vulkan 或无 GL/EGL 环境则跳过，不为守护引入
/// flake。
///
/// 2026-10-10 修正：不再因「数组层数 < 图集」跳过——那正是 CI 洞（GLES
/// 截断/多数组路径零覆盖）。改用 `with_atlas_layer_cap(Some(256))` 强制
/// 4 数组拆分路径（swrast 256 层成为天然试验田），草地断言照常执行。
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
    let mut renderer = mcv_render::Renderer::with_atlas_layer_cap(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Some(&workspace_assets()),
        // 强制 256 层设备上限：lavapipe 3907 层永不触发的多数组拆分路径
        // 在 GLES 后端下必须真实走到（CI 洞修复——旧版「层数不够就跳过」
        // 让 Mali 截断路径在 CI 永不可见）。
        Some(256),
    );
    let clouds = mcv_render::Clouds::new(&device, &queue);
    // 事故现场复跑：MeshUploader 创建期映射写入路径（GLES 曾在此 fatal）。
    // #80 起上传器持批记账（&mut）。
    let mut up = mcv_render::gpu::MeshUploader::new(device.clone());
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
        mk([0.0, y, 16.0], [0, 4096]),
        mk([16.0, y, 16.0], [4096, 4096]),
        mk([16.0, y, 0.0], [4096, 0]),
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
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(&chunk),
        hud: &hud,
        cloud: Some((&clouds, mcv_render::CloudSettings::default())),
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
        hand: None,
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
    // GL 回退下草地必须仍然画得出（真实素材 + 染色；颜色路径跨后端
    // 一致性弱，阈值放宽）。
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
            fog_tint: [1.0, 1.0, 1.0],
            fog_density_mult: 1.0,
            sun_dir: Vec3::Y,
            moon_phase: 0,
            width: 320.0,
            height: 240.0,
            chunks: &[],
            hud: empty_hud,
            cloud: cloud_on.then_some((&clouds, mcv_render::CloudSettings::default())),
            player: None,
            mobs: None,
            overlay: None,
            underwater: false,
            particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
            hand: None,
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

/// CJK 字形尺寸回归（任务 #40：中文渲染比英文大很多、溢出布局框）。
/// 原版依据：unifont quad 尺寸 = 位图 / oversample（UnihexProvider.java:320
/// getOversample()=2.0、:329 getPixelHeight()=16；GlyphBitmap.java:20-30
/// right/bottom 均除以 oversample），16px 位图只画 8px 高，与 ASCII
/// （BitmapProvider 8x8、oversample 1）同处一行行框（Font.java:37
/// lineHeight = 9，8px 字形 + 1px 阴影行）。修复前 CJK quad 直接用 16px
/// 位图高，纵向范围约 17px，溢出行框近一倍。本测试渲一行中英混排，
/// 断言白色（正文，阈值 >200；阴影 0.25 灰不计数）像素纵向范围 ≤ 9。
#[test]
fn cjk_text_stays_within_line_box() {
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);

    // 中英混排一行：CJK 走 unifont（tex=3），ASCII 走字体图集（tex=0）。
    // 先垫一块深色 HUD 矩形，隔离天空/雾背景色对「白色像素」统计的干扰。
    let mut hud = vec![mcv_render::text::rect(
        10.0,
        80.0,
        80.0,
        50.0,
        [0.05, 0.05, 0.08, 1.0],
    )];
    hud.extend(mcv_render::text::text_quads(
        "中文Aa",
        20.0,
        100.0,
        1.0,
        [1.0, 1.0, 1.0, 1.0],
    ));
    let (sun, day) = mcv_render::sun_state(6000);
    let camera = Camera {
        pos: Vec3::new(8.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: day,
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: &[],
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
        hand: None,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut encoder);
    queue.submit([encoder.finish()]);
    let rgba = target.read_pixels(&device);

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        let png = mcv_render::offscreen::encode_png(extent.width, extent.height, &rgba);
        let _ = std::fs::write(std::path::Path::new(&dir).join("cjk-line-test.png"), png);
    }

    // 扫描文字区（x 10..80，y 80..130）内白色像素的纵向范围。
    let (min_y, max_y, count) = {
        let mut min_y = u32::MAX;
        let mut max_y = 0u32;
        let mut count = 0usize;
        for y in 80..130u32 {
            for x in 10..80u32 {
                let o = ((y * extent.width + x) * 4) as usize;
                if rgba[o] > 200 && rgba[o + 1] > 200 && rgba[o + 2] > 200 {
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                    count += 1;
                }
            }
        }
        (min_y, max_y, count)
    };
    assert!(count > 50, "中英混排文字未渲出白像素：count={count}");
    let span = max_y - min_y + 1;
    assert!(
        span <= 9,
        "文字纵向范围 {min_y}..{max_y}（{span}px）超出一行行框（9px，Font.java:37）——CJK 字形溢出"
    );
}

/// 日月贴图化回归锁（原版 26.1 日月为 celestials 图集贴图 quad，
/// SkyRenderer.java:125-157；CELESTIAL 管线加色混合，RenderPipelines.java:643
/// → BlendFunction.java:9 `dst += src.rgb * src.a`——sun.png 全图不透明、
/// 四周暗色，加色下只亮出中心核）。相机仰视天顶：太阳亮核在屏上应产出
/// 一簇白/黄白像素（蓝天底 b 主导，亮核 r/g 追平 b）。程序化圆盘回退路径
/// （无素材）不会产出该簇 → 双路径可区分。
#[test]
fn celestial_sun_texture_paints_core() {
    let (device, queue, mut renderer) = setup_with_assets(Some(&workspace_assets()));
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    // 仰视天顶（pitch>0 朝上；太阳在 sun_dir=+Y 天顶，quad 半角 atan(15/100)≈8.6°）。
    let camera = Camera {
        pos: Vec3::new(8.0, 110.0, 8.0),
        yaw: 0.0,
        pitch: 1.4,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 512.0,
    };
    let hud: Vec<HudQuad> = Vec::new();
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: 1.0,
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: Vec3::Y,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: &[],
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
        hand: None,
    };
    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut enc);
    queue.submit([enc.finish()]);
    let rgba = target.read_pixels(&device);
    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        let png = mcv_render::offscreen::encode_png(extent.width, extent.height, &rgba);
        let _ = std::fs::write(std::path::Path::new(&dir).join("celestial-test.png"), png);
    }
    // 太阳亮核判别：天顶蓝天 sRGB ≈ (153,195,249)，亮核加色后 r/g/b 全顶满
    // 255——r≥250 即可唯一区分（蓝天 r≈153 差距远超驱动舍入噪声）。
    let mut core = 0usize;
    for px in rgba.chunks(4) {
        if px[0] >= 250 && px[1] >= 250 {
            core += 1;
        }
    }
    assert!(core > 30, "太阳亮核缺失（原版贴图未上屏？）core={core}");
}

/// 素材红线守护（2026-10 任务 #53）：素材缺失路径不得产出任何程序化
/// 假贴图像素（原「程序化回退」分支已删，本测试锁死该语义）：
/// - 地形层：图集只剩原版 missing 标记（MissingTextureAtlasSprite 品红/
///   黑棋盘），草地层采样为品红系——绝不出现绿色假草地；
/// - 天体：纹理数组全透明，天空绝不出现程序化假太阳亮核（对照
///   [`celestial_sun_texture_paints_core`] 的真实素材路径）；
/// - 字体：字体纹理全透明，HUD 字形/实心矩形不上屏——无假字形。
#[test]
fn missing_assets_never_paint_fake_pixels() {
    let (device, queue, mut renderer) = setup_with_assets(None);
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);
    let (sun, day) = mcv_render::sun_state(6000);
    let hud: Vec<HudQuad> = Vec::new();

    // —— 场景 A：俯视草地，应为 missing 品红标记 ——
    let camera = Camera {
        pos: Vec3::new(8.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: day,
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: std::slice::from_ref(&chunk),
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
        hand: None,
    };
    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut enc);
    queue.submit([enc.finish()]);
    let rgba = target.read_pixels(&device);

    // 草地带统计：品红主导（r、b 高且 g 低）必须占多数；绿色主导必须为 0。
    let (mut magenta, mut green) = (0u64, 0u64);
    for y in 120..190u32 {
        for x in 0..320u32 {
            let o = ((y * 320 + x) * 4) as usize;
            let (r, g, b) = (rgba[o] as i32, rgba[o + 1] as i32, rgba[o + 2] as i32);
            if r > 150 && b > 150 && g + 60 < r.min(b) {
                magenta += 1;
            }
            if g > r + 10 && g > b + 10 {
                green += 1;
            }
        }
    }
    let band = 70 * 320; // y 120..190 的地面统计带
    // 红线断言：素材缺失下不得出现绿色假草地（程序化噪声回退的签名色）。
    // 品红象限经 mip 下采样/雾混合后亮度不可控（lavapipe 实测 ~16%像素
    // 命中品红谓词），故只要求品红显著多于绿——标记存在即可，伪装必零。
    assert!(
        magenta * 8 > band as u64,
        "素材缺失下应能看到 missing 品红标记，magenta={magenta}/{band}"
    );
    assert_eq!(green, 0, "素材缺失下出现绿色假草地像素（程序化回退复辟？）");

    // —— 场景 B：仰视天顶 + HUD 字形/实心条，应无假太阳、无假字形 ——
    let camera_up = Camera {
        pos: Vec3::new(8.0, 110.0, 8.0),
        yaw: 0.0,
        pitch: 1.4,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 512.0,
    };
    let hud_b = vec![
        HudQuad {
            x: 100.0,
            y: 20.0,
            w: 16.0,
            h: 16.0,
            uv: font::glyph_uv(65), // 'A'
            color: [1.0, 1.0, 1.0, 1.0],
            tex: 0,
            layer: 0,
            rot: 0.0,
        },
        HudQuad {
            x: 130.0,
            y: 20.0,
            w: 40.0,
            h: 6.0,
            uv: font::glyph_uv(font::SOLID_CELL),
            color: [1.0, 1.0, 1.0, 1.0],
            tex: 0,
            layer: 0,
            rot: 0.0,
        },
    ];
    let scene = Scene {
        camera: &camera_up,
        time: 0.0,
        day_factor: 1.0,
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: Vec3::Y, // 太阳在正天顶（对照真实素材测试的同机位）
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: &[],
        hud: &hud_b,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
        hand: None,
    };
    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut enc);
    queue.submit([enc.finish()]);
    let rgba = target.read_pixels(&device);

    // 无假太阳亮核：真实素材路径该机位 core>30（r、g ≥250），此处必须为 0。
    let mut core = 0usize;
    for px in rgba.chunks(4) {
        if px[0] >= 250 && px[1] >= 250 {
            core += 1;
        }
    }
    assert_eq!(core, 0, "素材缺失下出现程序化假太阳亮核像素 core={core}");

    // 无假字形/假实心矩形：字形与实心条区域（顶部天顶蓝底，r<200）不应
    // 有任何白色像素。
    let mut white = 0usize;
    for y in 16..44u32 {
        for x in 96..176u32 {
            let o = ((y * 320 + x) * 4) as usize;
            if rgba[o] > 200 && rgba[o + 1] > 200 && rgba[o + 2] > 200 {
                white += 1;
            }
        }
    }
    assert_eq!(
        white, 0,
        "素材缺失下出现假字形/假实心矩形像素 white={white}"
    );
}

/// M7b 最小接线验证：鸡/牛/羊/猪四型以原版模型 + 原版贴图（entity/
/// {chicken,cow,sheep,pig}/…temperate 系）在世界里出像素；缺素材路径下
/// mob 必须一像素不出（素材红线：宁缺勿画，draw_mobs 短路）。
#[test]
fn four_mobs_paint_with_vanilla_textures() {
    use mcv_render::gpu::MobInstance;
    use mcv_render::mob_mesh::{MAX_MOB_PARTS, MobModelKind, MobPose, mob_model_matrices};

    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    // 四站位：站在 y=100 草地（ground_chunk 覆盖 0..16），绕视场中心展开。
    let placements = [
        (MobModelKind::Chicken, Vec3::new(4.0, 100.0, 10.0)),
        (MobModelKind::Cow, Vec3::new(8.0, 100.0, 8.0)),
        (MobModelKind::Sheep, Vec3::new(12.0, 100.0, 10.0)),
        (MobModelKind::Pig, Vec3::new(8.0, 100.0, 14.0)),
    ];
    let instances: Vec<MobInstance> = placements
        .iter()
        .map(|(k, p)| {
            let pose = MobPose {
                pos: *p,
                yaw: 0.6,
                phase: 1.0,
                amount: 0.5,
                ..Default::default()
            };
            let mut models = [[[0.0f32; 4]; 4]; MAX_MOB_PARTS];
            for (m, dst) in mob_model_matrices(*k, &pose).iter().zip(models.iter_mut()) {
                *dst = m.to_cols_array_2d();
            }
            MobInstance {
                kind: k.idx() as u32,
                models,
            }
        })
        .collect();

    let camera = Camera {
        pos: Vec3::new(8.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };

    // 同一 renderer 渲两帧对比（闭包只管提交与回读）。
    fn frame(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut mcv_render::Renderer,
        target: &OffscreenTarget,
        chunk: &RenderChunk,
        camera: &Camera,
        mobs: Option<&[MobInstance]>,
    ) -> Vec<u8> {
        let (sun, day) = mcv_render::sun_state(6000); // noon
        let hud: Vec<HudQuad> = Vec::new();
        let scene = Scene {
            camera,
            time: 0.0,
            day_factor: day,
            fog_tint: [1.0, 1.0, 1.0],
            fog_density_mult: 1.0,
            sun_dir: sun,
            moon_phase: 0,
            width: 320.0,
            height: 240.0,
            chunks: std::slice::from_ref(chunk),
            hud: &hud,
            cloud: None,
            player: None,
            mobs,
            overlay: None,
            underwater: false,
            particles: None, // M8a 接线：Some((&runtime.particles, tick_frac)),
            hand: None,
        };
        let mut enc = device.create_command_encoder(&Default::default());
        renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
        target.enqueue_copy(&mut enc);
        queue.submit([enc.finish()]);
        target.read_pixels(device)
    }

    let (device, queue, mut renderer) = setup();
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);
    let base = frame(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        &camera,
        None,
    );
    let with = frame(
        &device,
        &queue,
        &mut renderer,
        &target,
        &chunk,
        &camera,
        Some(&instances),
    );
    let (diff, bbox) = a_diff_pixels(&base, &with);
    assert!(
        diff > 400,
        "四生物应共同遮挡 >400px，实测 {diff}（bbox {bbox:?}）"
    );

    // 逐型单独出场：每种都必须有自己的像素（贴图/网格路由按 kind 正确）。
    for (i, (k, _)) in placements.iter().enumerate() {
        let solo = frame(
            &device,
            &queue,
            &mut renderer,
            &target,
            &chunk,
            &camera,
            Some(&instances[i..i + 1]),
        );
        let (d, bb) = a_diff_pixels(&base, &solo);
        assert!(d > 20, "{k:?} 独应出像素，实测 {d}（bbox {bb:?}）");
    }

    // 「原版贴图」证据：被生物改写的像素必须多彩且有亮部——程序化黑/纯色
    // 占位只能给出色数 ≤2 或全暗的签名。
    let mut colors: std::collections::HashSet<(u8, u8, u8)> = std::collections::HashSet::new();
    let mut max_lum = 0u32;
    for (p, q) in base.chunks(4).zip(with.chunks(4)) {
        let dp: i32 = p[..3].iter().map(|&v| v as i32).sum();
        let dq: i32 = q[..3].iter().map(|&v| v as i32).sum();
        if (dp - dq).abs() > 40 {
            colors.insert((q[0], q[1], q[2]));
            max_lum = max_lum.max(dq as u32);
        }
    }
    assert!(
        colors.len() >= 5,
        "生物像素色数 {}/亮峰 {max_lum}：疑似程序化纯色贴图而非原版素材",
        colors.len()
    );
    assert!(
        max_lum > 150,
        "生物像素全暗（亮峰 {max_lum}）：疑似黑占位贴图"
    );

    // 素材红线：缺素材 renderer 下 mobs 必须与不画逐像素一致（短路 no-op，
    // 无任何程序化假生物）。
    let (device2, queue2, mut renderer2) = setup_with_assets(None);
    let target2 = OffscreenTarget::new(&device2, extent);
    let chunk2 = ground_chunk(&device2);
    let base2 = frame(
        &device2,
        &queue2,
        &mut renderer2,
        &target2,
        &chunk2,
        &camera,
        None,
    );
    let with2 = frame(
        &device2,
        &queue2,
        &mut renderer2,
        &target2,
        &chunk2,
        &camera,
        Some(&instances),
    );
    let (d2, bb2) = a_diff_pixels(&base2, &with2);
    assert_eq!(d2, 0, "缺素材路径画出了假生物像素 {d2}（bbox {bb2:?}）");
}

/// 第一人称手持渲染离屏像素断言（任务板 #93）：
/// (a) 空手帧 vs 无手持帧 diff > 阈值——手臂像素确实上屏（真机投诉「没有手臂」回归锁）；
/// (b) 持方块帧 vs 空手帧 diff > 阈值——手持方块像素上屏；
/// (c) 各帧 diff 的包围盒都落在屏幕右下区域——位置/缩放对标原版观感；
/// (d) 挥臂半程帧 vs 静止帧 diff > 阈值——swing 快照确实被渲染消费（挖掘/攻击挥臂上屏）；
/// (e) 手持物品图标帧 vs 空手帧 diff > 阈值——icon quad 三件接线（uniform 喂帧/
///     quad 索引/视→世界角点换算）缺一即静默只画手臂；
/// (f) 覆盖建缓冲到全部 pass 的 validation 无错误。
#[test]
fn first_person_hand_paints_bottom_right() {
    let (device, queue, mut renderer) = setup_with_assets(Some(&workspace_assets()));
    // 第一人称手臂复用玩家皮肤贴图（entity/player/wide/steve.png）。
    let assets = mcv_assets::AssetManager::new(workspace_assets());
    let steve = assets
        .read_optional("textures/entity/player/wide/steve.png")
        .expect("steve.png missing");
    let alex = assets
        .read_optional("textures/entity/player/slim/alex.png")
        .expect("alex.png missing");
    renderer.load_skins(&steve, &alex).expect("skins upload");

    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let guard = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // 平视空场景（无区块）：世界 pass 只画天空，任何新增像素都来自手持 pass。
    let camera = Camera {
        pos: Vec3::new(0.0, 64.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };

    fn frame(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut mcv_render::Renderer,
        target: &OffscreenTarget,
        camera: &Camera,
        hand: Option<mcv_render::HandRender>,
    ) -> Vec<u8> {
        let (sun, day) = mcv_render::sun_state(6000); // noon
        let hud: Vec<HudQuad> = Vec::new();
        let scene = Scene {
            camera,
            time: 0.0,
            day_factor: day,
            fog_tint: [1.0, 1.0, 1.0],
            fog_density_mult: 1.0,
            sun_dir: sun,
            moon_phase: 0,
            width: 320.0,
            height: 240.0,
            chunks: &[],
            hud: &hud,
            cloud: None,
            player: None,
            mobs: None,
            overlay: None,
            underwater: false,
            particles: None,
            hand,
        };
        let mut enc = device.create_command_encoder(&Default::default());
        renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
        target.enqueue_copy(&mut enc);
        queue.submit([enc.finish()]);
        target.read_pixels(device)
    }

    let none = frame(&device, &queue, &mut renderer, &target, &camera, None);
    let empty = frame(
        &device,
        &queue,
        &mut renderer,
        &target,
        &camera,
        Some(mcv_render::HandRender {
            item: mcv_render::HandItem::Empty,
            swing: 0.0,
        }),
    );
    let block = frame(
        &device,
        &queue,
        &mut renderer,
        &target,
        &camera,
        Some(mcv_render::HandRender {
            item: mcv_render::HandItem::Block(1), // BLOCKS[1] = stone
            swing: 0.0,
        }),
    );

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        for (name, px) in [
            ("hand-none.png", &none),
            ("hand-empty.png", &empty),
            ("hand-block.png", &block),
        ] {
            let png = mcv_render::offscreen::encode_png(extent.width, extent.height, px);
            let _ = std::fs::write(std::path::Path::new(&dir).join(name), png);
        }
    }

    // 带 w 的像素 diff（阈值取通道和 30）：返回包围盒供区域断言。
    let diff_bbox = |a: &[u8], b: &[u8], w: u32| -> (usize, Option<(u32, u32, u32, u32)>) {
        let mut n = 0usize;
        let mut bb: Option<(u32, u32, u32, u32)> = None;
        for (i, (p, q)) in a.chunks(4).zip(b.chunks(4)).enumerate() {
            let dp: i32 = p[..3].iter().map(|&v| v as i32).sum();
            let dq: i32 = q[..3].iter().map(|&v| v as i32).sum();
            if (dp - dq).abs() > 30 {
                n += 1;
                let (x, y) = ((i as u32) % w, (i as u32) / w);
                bb = Some(match bb {
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    None => (x, y, x, y),
                });
            }
        }
        (n, bb)
    };

    // (a) 手臂像素上屏：空手帧必须与空场景不同。
    let (arm_diff, arm_bb) = diff_bbox(&none, &empty, extent.width);
    assert!(
        arm_diff > 100,
        "空手帧缺少手臂像素：diff={arm_diff}（bbox {arm_bb:?}）——第一人称手臂管线被静默跳过？"
    );
    // (b) 手持方块像素上屏：持方块帧与空手帧必须显著不同。
    let (blk_diff, blk_bb) = diff_bbox(&empty, &block, extent.width);
    assert!(
        blk_diff > 100,
        "持方块帧缺少手持方块像素：diff={blk_diff}（bbox {blk_bb:?}）"
    );
    // (c) 原版观感：手臂与手持物的像素都在右下象限。
    let in_bottom_right = |bb: Option<(u32, u32, u32, u32)>| {
        let (x0, y0, ..) = bb.expect("diff>0 必有包围盒");
        x0 >= extent.width / 2 && y0 >= extent.height / 2
    };
    assert!(
        in_bottom_right(arm_bb),
        "手臂像素越出右下象限（bbox {arm_bb:?}）——摆放常数回归"
    );
    assert!(
        in_bottom_right(blk_bb),
        "手持方块像素越出右下象限（bbox {blk_bb:?}）——摆放常数回归"
    );

    // (d) 挥臂消费链上屏：swing 快照必须驱动画面变化（game 层
    // swing_progress → Scene.hand.swing → 臂/手持物矩阵的端到端）。
    // 挖掘中每 tick 重触发的劈砍若只停留在快照字段，本断言 diff=0 即红。
    let swung = frame(
        &device,
        &queue,
        &mut renderer,
        &target,
        &camera,
        Some(mcv_render::HandRender {
            item: mcv_render::HandItem::Block(1),
            swing: 0.5, // 半程 = 劈砍角最大（sin(√0.5·π)）
        }),
    );
    let (swing_diff, swing_bb) = diff_bbox(&block, &swung, extent.width);
    assert!(
        swing_diff > 100,
        "挥臂半程帧与静止帧 diff={swing_diff}（bbox {swing_bb:?}）——swing_progress 快照未被渲染消费"
    );
    // 挥臂运动域 = 下半屏且仍覆盖右半屏（原版劈砍朝准星摆、过中线属正常，
    // 不能按静止姿态的右下象限硬卡——CI 实测 bbox x0≈150 合理）。
    let (sx0, sy0, sx1, _) = swing_bb.expect("diff>100 必有包围盒");
    assert!(
        sy0 >= extent.height / 2 && sx1 >= extent.width / 2 && sx0 >= extent.width / 4,
        "挥臂运动域越界（bbox {swing_bb:?}）——挥臂锚点/旋转常数回归"
    );

    // (e) 手持物品图标（sprite 路径）：icon quad 的 uniform 喂帧 + quad 索引
    // + 视→世界角点换算三件接线缺一即只有手臂（diff=0 红）。
    let sprite = frame(
        &device,
        &queue,
        &mut renderer,
        &target,
        &camera,
        Some(mcv_render::HandRender {
            item: mcv_render::HandItem::Sprite("diamond"),
            swing: 0.0,
        }),
    );
    let (icon_diff, icon_bb) = diff_bbox(&empty, &sprite, extent.width);
    assert!(
        icon_diff > 100,
        "手持物品图标帧缺少图标像素：diff={icon_diff}（bbox {icon_bb:?}）——hand_icon uniform/索引/世界换算接线回归"
    );
    assert!(
        in_bottom_right(icon_bb),
        "手持图标像素越出右下象限（bbox {icon_bb:?}）——图标锚点常数回归"
    );

    let scope_err = pollster::block_on(guard.pop());
    assert!(
        scope_err.is_none(),
        "手持 pass 产生 validation error: {scope_err:?}——臂/立方体缓冲接线回归"
    );
}

/// #80 相邻区块合批（结构性）：空间相邻的同 pass 区块必须合并为更少的
/// 绘制条目、一次 draw 画多个区块。三重断言：
/// (a) 三个相邻块经 MeshUploader 只产出 1 个条目（批次 origin = 最小角、
///     opaque 索引量为三块之和）；
/// (b) 批次条目画出的地形与逐块条目基线逐像素一致（重定基只动顶点字节，
///     世界坐标逐位不变——frustum_cull/草地测试同款绿份额断言兜底）；
/// (c) draw_frame 统计 opaque_draws == 1（3 块 1 draw，收敛 3 倍）。
#[test]
fn adjacent_chunks_merge_into_one_draw_unit() {
    let (device, queue, mut renderer) = setup();
    let mut up = mcv_render::gpu::MeshUploader::new(device.clone());
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
        mk([0.0, y, 16.0], [0, 4096]),
        mk([16.0, y, 16.0], [4096, 4096]),
        mk([16.0, y, 0.0], [4096, 0]),
    ];
    let idx: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let vb = bytemuck::cast_slice(&verts);
    // 三个 x 向相邻块（局部顶点 0..16，原点 0/16/32）——mesher 输出即局部
    // 坐标，重定基由上传器负责。
    let _c0 = up.build_chunk([0.0, 0.0, 0.0], vb, &idx, None);
    let _c1 = up.build_chunk([16.0, 0.0, 0.0], vb, &idx, None);
    let _c2 = up.build_chunk([32.0, 0.0, 0.0], vb, &idx, None);

    let entries = up.entries();
    assert_eq!(
        entries.len(),
        1,
        "三个相邻块必须合并为 1 个批次条目，实测 {}",
        entries.len()
    );
    assert_eq!(entries[0].origin, [0.0, 0.0, 0.0], "批次 origin 必须是最小角");
    assert_eq!(entries[0].opaque_range, 0..18, "批次索引量 = 三块之和");

    // (b)+(c)：批次条目整帧渲染——绿份额与 draw 数。
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let camera = Camera {
        pos: Vec3::new(24.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };
    let hud: Vec<HudQuad> = Vec::new();
    let (sun, day) = mcv_render::sun_state(6000); // noon
    let scene = Scene {
        camera: &camera,
        time: 0.0,
        day_factor: day,
        fog_tint: [1.0, 1.0, 1.0],
        fog_density_mult: 1.0,
        sun_dir: sun,
        moon_phase: 0,
        width: 320.0,
        height: 240.0,
        chunks: up.entries(),
        hud: &hud,
        cloud: None,
        player: None,
        mobs: None,
        overlay: None,
        underwater: false,
        particles: None,
        hand: None,
    };
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    let st = renderer.last_frame_stats();
    assert_eq!(
        st.opaque_draws, 1,
        "三个相邻块合批后 opaque draw 必须是 1 次"
    );
    let mut enc = device.create_command_encoder(&Default::default());
    target.enqueue_copy(&mut enc);
    queue.submit([enc.finish()]);
    let rgba = target.read_pixels(&device);
    let (green, _) = sample_stats(&rgba, extent.width, extent.height);
    assert!(
        green > 0.2,
        "批次条目渲染缺草地像素 green={green}——重定基/索引平移回归"
    );
}

/// #80 重网格批次重建：批次内成员重传新网格后，批次条目必须用**新字节**
/// 原位重建（同批次成员资格不变、条目数不变），画面随新贴图切换。
#[test]
fn batch_rebuild_uses_fresh_member_bytes() {
    let (device, queue, mut renderer) = setup();
    let mut up = mcv_render::gpu::MeshUploader::new(device.clone());
    let y = 100.0f32;
    let mk = |layer: u16, p: [f32; 3], uv: [u16; 2]| Tv {
        pos: p,
        uv,
        layer,
        block_light: 0,
        sky_light: 15,
        ao: 3,
        flags: 2, // face_id +Y
        pad: [0; 2],
    };
    let verts_for = |layer: u16| {
        [
            mk(layer, [0.0, y, 0.0], [0, 0]),
            mk(layer, [0.0, y, 16.0], [0, 4096]),
            mk(layer, [16.0, y, 16.0], [4096, 4096]),
            mk(layer, [16.0, y, 0.0], [4096, 0]),
        ]
    };
    let idx: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let camera = Camera {
        pos: Vec3::new(16.0, 110.0, 26.0),
        yaw: 0.0,
        pitch: -0.62,
        fov_y: 1.2,
        aspect: 320.0 / 240.0,
        near: 0.1,
        far: 256.0,
    };
    let hud: Vec<HudQuad> = Vec::new();
    let (sun, day) = mcv_render::sun_state(6000);

    let frame_green = |renderer: &mut mcv_render::Renderer,
                       chunks: &[RenderChunk]|
     -> f64 {
        let scene = Scene {
            camera: &camera,
            time: 0.0,
            day_factor: day,
            fog_tint: [1.0, 1.0, 1.0],
            fog_density_mult: 1.0,
            sun_dir: sun,
            moon_phase: 0,
            width: 320.0,
            height: 240.0,
            chunks,
            hud: &hud,
            cloud: None,
            player: None,
            mobs: None,
            overlay: None,
            underwater: false,
            particles: None,
            hand: None,
        };
        renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
        let mut enc = device.create_command_encoder(&Default::default());
        target.enqueue_copy(&mut enc);
        queue.submit([enc.finish()]);
        let rgba = target.read_pixels(&device);
        let (green, _) = sample_stats(&rgba, extent.width, extent.height);
        green
    };

    // 两块相邻（x 0 与 16），先全草皮。
    let _a = up.build_chunk(
        [0.0, 0.0, 0.0],
        bytemuck::cast_slice(&verts_for(mcv_core::tiles::GRASS_TOP)),
        &idx,
        None,
    );
    let _b = up.build_chunk(
        [16.0, 0.0, 0.0],
        bytemuck::cast_slice(&verts_for(mcv_core::tiles::GRASS_TOP)),
        &idx,
        None,
    );
    assert_eq!(up.entries().len(), 1, "两块必须合为一批");
    let green_grass = frame_green(&mut renderer, up.entries());

    // 重网格块 A：草 → 泥土。批次条目必须换新字节（成员资格与条目数不变）。
    let _a2 = up.build_chunk(
        [0.0, 0.0, 0.0],
        bytemuck::cast_slice(&verts_for(mcv_core::tiles::DIRT)),
        &idx,
        None,
    );
    assert_eq!(up.entries().len(), 1, "重网格不得拆批");
    let green_dirt = frame_green(&mut renderer, up.entries());
    assert!(
        green_dirt < green_grass * 0.5,
        "重网格后画面必须切换到新贴图（草 green={green_grass} → 泥 green={green_dirt}）——批次重建用了陈旧字节"
    );
}

/// #80 卸载成员：批次随成员卸载收缩/清批，画面不得残留已卸载区块的像素。
#[test]
fn unload_removes_member_from_batch() {
    let (device, queue, mut renderer) = setup();
    let mut up = mcv_render::gpu::MeshUploader::new(device.clone());
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
        mk([0.0, y, 16.0], [0, 4096]),
        mk([16.0, y, 16.0], [4096, 4096]),
        mk([16.0, y, 0.0], [4096, 0]),
    ];
    let idx: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let vb = bytemuck::cast_slice(&verts);
    let _a = up.build_chunk([0.0, 0.0, 0.0], vb, &idx, None);
    let _b = up.build_chunk([16.0, 0.0, 0.0], vb, &idx, None);
    assert_eq!(up.entries().len(), 1, "两块合为一批");
    assert_eq!(up.batching_stats().meshed_chunks, 2);

    // 卸载块 B（x=16）：批次收缩为单块 A，画面只剩 A。
    up.unload(mcv_core::ChunkPos::new(1, 0));
    assert_eq!(up.entries().len(), 1, "批内仍有一块，批次保留");
    assert_eq!(up.batching_stats().meshed_chunks, 1);
    // 卸载块 A：批次清空。
    up.unload(mcv_core::ChunkPos::new(0, 0));
    assert_eq!(up.entries().len(), 0, "批空必须删批");

    // 卸载不存在的区块 = no-op（不炸不建批）。
    up.unload(mcv_core::ChunkPos::new(9, 9));
    assert_eq!(up.entries().len(), 0);
}
