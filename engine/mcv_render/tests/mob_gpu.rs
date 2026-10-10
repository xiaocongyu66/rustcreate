#![cfg(feature = "gpu-tests")]

//! 生物渲染离屏验证（lavapipe / CI render-headless job，与 particles.rs
//! 同款基座：像素统计 + base/with diff，不做黄金像素）。
//!
//! 审计 D P0 的端到端锁：`Scene.mobs` 喂入 [`MobInstance`] 后画面必须
//! 出现 entity 贴图像素（diff 簇 = mob 部位，非天空/地面色——diff 语义
//! 天然保证：帧间唯一变量是 mob 实例）。

use glam::Vec3;
use mcv_render::gpu::{RenderChunk, Scene};
use mcv_render::mob_mesh::{MobModelKind, MobPose, mob_model_matrices};
use mcv_render::{Camera, MobInstance, OffscreenTarget};
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
    // 16x16 草皮 y=100（particles.rs 同款：顶面天光草）。
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
            label: Some("mob-gpu-test-vbuf"),
            contents: bytemuck::bytes_of(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        index_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mob-gpu-test-ibuf"),
            contents: bytemuck::bytes_of(&idx),
            usage: wgpu::BufferUsages::INDEX,
        }),
        opaque_range: 0..6,
        // #80 合批：单块条目无水分段（旧 water_range: 0..0 的等价形态）。
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
        label: Some("mob-gpu-test"),
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

fn diff_pixels(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(p, q)| p[0..3] != q[0..3])
        .count()
}

fn camera() -> Camera {
    // 俯视草皮，画面中心落在 (8,100,6.4) 附近；四只怪沿视线一字排开。
    Camera {
        pos: Vec3::new(8.0, 103.5, 16.0),
        yaw: 0.0,
        pitch: -0.35,
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
    mobs: &[MobInstance],
) -> Vec<u8> {
    let cam = camera();
    let hud: Vec<mcv_render::gpu::HudQuad> = Vec::new();
    let (sun, day) = mcv_render::sun_state(6000);
    let scene = Scene {
        camera: &cam,
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
        mobs: (!mobs.is_empty()).then_some(mobs),
        overlay: None,
        underwater: false,
        particles: None,
        hand: None,
    };
    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw_frame(&target.color_view(), &target.depth_view(), &scene);
    target.enqueue_copy(&mut enc);
    queue.submit([enc.finish()]);
    target.read_pixels(device)
}

fn instance(kind: MobModelKind, pos: Vec3, yaw: f32, phase: f32, amount: f32) -> MobInstance {
    let pose = MobPose {
        pos,
        yaw,
        phase,
        amount,
        ..Default::default()
    };
    let mut models = [[[0.0f32; 4]; 4]; mcv_render::mob_mesh::MAX_MOB_PARTS];
    for (m, dst) in mob_model_matrices(kind, &pose)
        .iter()
        .zip(models.iter_mut())
    {
        *dst = m.to_cols_array_2d();
    }
    MobInstance {
        kind: kind.idx() as u32,
        models,
    }
}

#[test]
fn four_hostile_mobs_paint_entity_pixels() {
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);

    let base = render(&device, &queue, &mut renderer, &target, &chunk, &[]);

    // 敌对四怪沿视线排开（各自贴图层 5..8）：投影核算都在画面内
    // （像素 x ≈ 97/143/198/265、各 20-32px 高），站上草皮无遮挡。
    let mobs = [
        instance(
            MobModelKind::Zombie,
            Vec3::new(4.0, 100.0, 6.0),
            0.0,
            0.0,
            0.0,
        ),
        instance(
            MobModelKind::Skeleton,
            Vec3::new(7.0, 100.0, 7.0),
            0.0,
            0.0,
            0.0,
        ),
        instance(
            MobModelKind::Creeper,
            Vec3::new(10.0, 100.0, 8.0),
            0.0,
            0.0,
            0.0,
        ),
        instance(
            MobModelKind::Spider,
            Vec3::new(13.0, 100.0, 9.0),
            0.0,
            0.0,
            0.0,
        ),
    ];
    let with = render(&device, &queue, &mut renderer, &target, &chunk, &mobs);

    if let Ok(dir) = std::env::var("MCV_SCREENSHOT_DIR") {
        let png = mcv_render::offscreen::encode_png(extent.width, extent.height, &with);
        let _ = std::fs::write(std::path::Path::new(&dir).join("mob-gpu-test.png"), png);
    }

    // diff 像素 = mob 部位（贴图色非天空/草皮色，帧间唯一变量是 mobs）。
    // 阈值宽松：四怪屏占合计数百像素起，只有 mob 全部未画出才可能低于。
    let changed = diff_pixels(&base, &with);
    assert!(
        changed > 400,
        "敌对四怪应画出 entity 贴图像素，changed={changed}"
    );
}

#[test]
fn zombie_walk_anim_changes_drawn_pixels() {
    // 行走相位端到端：同一只僵尸 静止 vs 满幅摆腿 → 画面必须变化
    // （WalkLeg 通路在 GPU 上生效；yaw=0 排除朝向因素）。
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);
    let pos = Vec3::new(8.0, 100.0, 9.0);
    let still = [instance(MobModelKind::Zombie, pos, 0.0, 0.0, 0.0)];
    let walking = [instance(MobModelKind::Zombie, pos, 0.0, 0.9, 0.88)];

    let a = render(&device, &queue, &mut renderer, &target, &chunk, &still);
    let b = render(&device, &queue, &mut renderer, &target, &chunk, &walking);
    let changed = diff_pixels(&a, &b);
    assert!(changed > 30, "摆腿应改变画面，changed={changed}");
}

#[test]
fn zombie_walk_anim_changes_pixels_at_nonzero_yaw() {
    // yaw≠0 回归（验收发现：此前全部实例 yaw=0，旧矩阵缺陷
    // T·ry²·Rx·ry⁻¹ 的腿摆侧翻在 yaw=0 下不可见）。侧向/背向两档
    // 各跑 静止 vs 满幅摆腿——WalkLeg 通路必须在任意朝向下生效；
    // 矩阵代数锁见 mob_mesh.rs 的刚性协变测试。
    let (device, queue, mut renderer) = setup();
    let extent = wgpu::Extent3d {
        width: 320,
        height: 240,
        depth_or_array_layers: 1,
    };
    let target = OffscreenTarget::new(&device, extent);
    let chunk = ground_chunk(&device);
    let pos = Vec3::new(8.0, 100.0, 9.0);
    for yaw in [std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
        let still = [instance(MobModelKind::Zombie, pos, yaw, 0.0, 0.0)];
        let walking = [instance(MobModelKind::Zombie, pos, yaw, 0.9, 0.88)];
        let a = render(&device, &queue, &mut renderer, &target, &chunk, &still);
        let b = render(&device, &queue, &mut renderer, &target, &chunk, &walking);
        let changed = diff_pixels(&a, &b);
        assert!(changed > 30, "yaw {yaw} 摆腿应改变画面，changed={changed}");
    }
}
