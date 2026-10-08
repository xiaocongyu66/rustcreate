//! winit 0.30 shell: window + surface + input → [`GameRuntime`].

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

#[cfg(target_os = "android")]
use winit::platform::android::EventLoopBuilderExtAndroid;

use crate::game::GameRuntime;

#[cfg(target_os = "android")]
type AndroidApp = android_activity::AndroidApp;
#[cfg(not(target_os = "android"))]
type AndroidApp = ();

pub async fn run(android: Option<AndroidApp>) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = EventLoop::<()>::with_user_event();
    #[cfg(target_os = "android")]
    {
        if let Some(app) = android {
            builder.with_android_app(app);
        }
    }
    let _ = android;
    let event_loop = builder.build()?;
    let mut state = AppState::default();
    #[cfg(target_os = "android")]
    {
        state.android_data = event_loop
            .android_app()
            .and_then(|a| a.internal_data_path().map(std::path::PathBuf::from));
        log::info!("android internal data: {:?}", state.android_data);
    }
    event_loop.run_app(&mut state)?;
    Ok(())
}

#[derive(Default)]
struct AppState {
    window: Option<Arc<Window>>,
    surface: Option<SurfacePair>,
    runtime: Option<GameRuntime>,
    last_cursor: Option<(f64, f64)>,
    step_accum: f32,
    last_time: Option<std::time::Instant>,
    save_timer: Option<std::time::Instant>,
    /// Android 应用私有目录（internal_data_path），存档放这里
    #[cfg(target_os = "android")]
    android_data: Option<std::path::PathBuf>,
}

struct SurfacePair {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("MCV — rustcreate")
            .with_inner_size(LogicalSize::new(1280.0f32, 720.0f32));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("create window failed: {e}");
                event_loop.exit();
                return;
            }
        };
        match Self::init_gpu(window.clone()) {
            Ok((surface, config, depth, device, queue)) => {
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(42);
                // Android 上 current_exe() 指向 APK（不可写），用应用私有目录
                #[cfg(target_os = "android")]
                let save_dir = {
                    let base = self
                        .android_data
                        .clone()
                        .unwrap_or_else(|| std::path::PathBuf::from("/data/local/tmp"));
                    base.join("saves/world")
                };
                #[cfg(not(target_os = "android"))]
                let save_dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("saves/world")))
                    .unwrap_or_else(|| std::path::PathBuf::from("saves/world"));
                let mut runtime = GameRuntime::new(seed, device, queue, config.format, save_dir);
                runtime.load_meta();
                self.save_timer = Some(std::time::Instant::now());
                self.runtime = Some(runtime);
                let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Confined);
                window.set_cursor_visible(false);
                self.surface = Some(SurfacePair {
                    surface,
                    config,
                    depth,
                });
                self.window = Some(window.clone());
                window.request_redraw();
            }
            Err(e) => {
                log::error!("gpu init failed: {e}");
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.surface = None;
        self.window = None;
        // runtime persists; its GPU buffers die with the device on Android —
        // for M6 we rebuild runtime on resume if the device was lost.
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.save_dirty(None);
                    runtime.save_meta();
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let (Some(sp), Some(_)) = (self.surface.as_mut(), self.runtime.as_ref()) {
                    if size.width > 0 && size.height > 0 {
                        sp.config.width = size.width;
                        sp.config.height = size.height;
                        // reconfigure happens lazily in redraw
                    }
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state,
                        physical_key: PhysicalKey::Code(code),
                        ..
                    },
                ..
            } => {
                let Some(runtime) = self.runtime.as_mut() else {
                    return;
                };
                let pressed = state == ElementState::Pressed;
                match code {
                    KeyCode::KeyW => runtime.input.forward = pressed,
                    KeyCode::KeyS => runtime.input.back = pressed,
                    KeyCode::KeyA => runtime.input.left = pressed,
                    KeyCode::KeyD => runtime.input.right = pressed,
                    KeyCode::Space => runtime.input.jump = pressed,
                    KeyCode::ShiftLeft => runtime.input.sneak = pressed,
                    KeyCode::ControlLeft | KeyCode::ShiftRight => runtime.input.sprint = pressed,
                    KeyCode::KeyF => {
                        if pressed {
                            runtime.player.flying = !runtime.player.flying;
                        }
                    }
                    KeyCode::Escape => event_loop.exit(),
                    code @ (KeyCode::Digit1
                    | KeyCode::Digit2
                    | KeyCode::Digit3
                    | KeyCode::Digit4
                    | KeyCode::Digit5
                    | KeyCode::Digit6
                    | KeyCode::Digit7
                    | KeyCode::Digit8
                    | KeyCode::Digit9)
                        if pressed =>
                    {
                        runtime.player.sel_slot = match code {
                            KeyCode::Digit1 => 0,
                            KeyCode::Digit2 => 1,
                            KeyCode::Digit3 => 2,
                            KeyCode::Digit4 => 3,
                            KeyCode::Digit5 => 4,
                            KeyCode::Digit6 => 5,
                            KeyCode::Digit7 => 6,
                            KeyCode::Digit8 => 7,
                            _ => 8,
                        };
                    }
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (Some(runtime), Some(last)) = (self.runtime.as_mut(), self.last_cursor) {
                    runtime.look(position.x - last.0, position.y - last.1);
                }
                self.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::Touch { .. } => {
                if let (Some(runtime), Some(sp)) = (self.runtime.as_mut(), self.surface.as_ref()) {
                    runtime
                        .touch
                        .on_event(&event, sp.config.width as f32, sp.config.height as f32);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(runtime) = self.runtime.as_mut() else {
                    return;
                };
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        runtime.input.mining = pressed;
                        if pressed {
                            runtime.interact(false);
                        }
                    }
                    MouseButton::Right => {
                        runtime.input.placing = pressed;
                        if pressed {
                            runtime.interact(true);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

type GpuInit = (
    wgpu::Surface<'static>,
    wgpu::SurfaceConfiguration,
    wgpu::TextureView,
    wgpu::Device,
    wgpu::Queue,
);

impl AppState {
    fn init_gpu(window: Arc<Window>) -> Result<GpuInit, Box<dyn std::error::Error>> {
        // Android: 默认 PRIMARY 不含 GL —— 无 Vulkan 的设备会直接拿不到
        // adapter 然后被当作闪退退出。显式加入 GLES。
        #[cfg(target_os = "android")]
        let backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        #[cfg(not(target_os = "android"))]
        let backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("mcv-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            }))?;
        let caps = surface.get_capabilities(&adapter);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: caps.formats[0],
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let depth = Self::depth_view(&device, config.width, config.height);
        Ok((surface, config, depth, device, queue))
    }

    fn depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth24Plus,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn redraw(&mut self) {
        let (Some(sp), Some(runtime), Some(window)) = (
            self.surface.as_mut(),
            self.runtime.as_mut(),
            self.window.clone(),
        ) else {
            return;
        };
        let size = window.inner_size();
        if size.width > 0
            && size.height > 0
            && (sp.config.width != size.width || sp.config.height != size.height)
        {
            sp.config.width = size.width;
            sp.config.height = size.height;
            sp.surface
                .configure(runtime.renderer().device(), &sp.config);
            sp.depth = Self::depth_view(runtime.renderer().device(), size.width, size.height);
        }

        // simulation ticks
        let now = std::time::Instant::now();
        let dt = self
            .last_time
            .replace(now)
            .map_or(0.016, |t| now.duration_since(t).as_secs_f32());
        self.step_accum = (self.step_accum + dt).min(0.2);
        while self.step_accum >= 1.0 / 60.0 {
            runtime.fixed_step(1.0 / 60.0);
            self.step_accum -= 1.0 / 60.0;
        }
        runtime.time_ticks += (dt * 20.0) as u64; // 20 ticks/s
        runtime.stream();

        // periodic world save (30 s)
        if self.save_timer.is_some_and(|t| t.elapsed().as_secs() >= 30) {
            self.save_timer = Some(std::time::Instant::now());
            runtime.save_dirty(None);
            runtime.save_meta();
        }

        use wgpu::CurrentSurfaceTexture as Tex;
        let frame = match sp.surface.get_current_texture() {
            Tex::Success(f) | Tex::Suboptimal(f) => f,
            _ => {
                sp.surface
                    .configure(runtime.renderer().device(), &sp.config);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let camera = runtime.camera(sp.config.width as f32 / sp.config.height as f32);
        let (sun, day) = mcv_render::sun_state(runtime.time_ticks);
        let hud = runtime.build_hud(sp.config.width as f32, sp.config.height as f32);
        let chunks: Vec<mcv_render::RenderChunk> = runtime.render_chunks().to_vec();
        let scene = mcv_render::Scene {
            camera: &camera,
            time: (runtime.time_ticks % 24_000) as f32 / 20.0,
            day_factor: day,
            sun_dir: sun,
            width: sp.config.width as f32,
            height: sp.config.height as f32,
            chunks: &chunks,
            hud: &hud,
        };
        runtime.renderer().draw_frame(&view, &sp.depth, &scene);
        runtime.renderer().queue().present(frame);
    }
}
