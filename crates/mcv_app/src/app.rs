//! winit 0.30 application shell + minimal wgpu 30 surface loop.
//!
//! M1 scope: window, clear-color render loop, resize, Android suspend/resume.
//! Game integration lands in M3+.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const SKY_CLEAR: [f64; 4] = [0.47, 0.65, 1.0, 1.0];

#[cfg(target_os = "android")]
type AndroidApp = android_activity::AndroidApp;
#[cfg(not(target_os = "android"))]
type AndroidApp = ();

pub async fn run(android: Option<AndroidApp>) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = EventLoop::<()>::with_user_event();
    #[cfg(target_os = "android")]
    if let Some(app) = android {
        builder = builder.with_android_app(app);
    }
    let _ = android;
    let event_loop = builder.build()?;
    let mut app_state = AppState::default();
    event_loop.run_app(&mut app_state)?;
    Ok(())
}

#[derive(Default)]
struct AppState {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("MCV")
            .with_inner_size(LogicalSize::new(1280.0f32, 720.0f32));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("create window failed: {e}");
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(Gpu::new(window.clone())) {
            Ok(gpu) => {
                self.window = Some(window);
                self.gpu = Some(gpu);
                window.request_redraw();
            }
            Err(e) => {
                log::error!("gpu init failed: {e}");
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // Drop surface + window; device/pipeline state dies with Gpu and is
        // rebuilt on the next `resumed` (Android lifecycle requirement).
        self.gpu = None;
        self.window = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.render();
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

impl Gpu {
    async fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("mcv-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await?;

        let caps = surface.get_capabilities(&adapter);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: caps.formats[0],
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        Ok(Self {
            device,
            queue,
            surface,
            config,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn render(&self) {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::OutOfMemory) => {
                log::error!("surface OOM");
                return;
            }
            Err(_) => {
                // Lost / outdated / timeout: reconfigure and retry next frame.
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SKY_CLEAR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
    }
}
