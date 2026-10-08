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

use crate::game::{GameMode, GameRuntime};

/// 应用界面状态机。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Screen {
    #[default]
    Main,
    Worlds,
    Create,
    Settings,
    InGame,
    Paused,
}

/// 菜单按钮：命中测试用。
struct MenuButton {
    id: &'static str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct WorldEntry {
    dir: std::path::PathBuf,
    name: String,
    mode: &'static str,
}

const SETTINGS_MIN_DIST: i32 = 4;
const SETTINGS_MAX_DIST: i32 = 16;

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
    renderer: Option<mcv_render::Renderer>,
    screen: Screen,
    menu_hot: Vec<MenuButton>,
    world_hot: Vec<(usize, f32, f32, f32, f32)>,
    quit_requested: bool,
    worlds: Vec<WorldEntry>,
    create_mode: usize, // index into [Survival, Creative, Hardcore]
    set_dist: i32,
    set_sens: f32,
    cached_device: Option<wgpu::Device>,
    cached_queue: Option<wgpu::Queue>,
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

impl AppState {
    /// 构建菜单界面（HUD quad + 按钮命中表）。
    fn menu_ui(&mut self, w: f32, h: f32) -> Vec<mcv_render::HudQuad> {
        use mcv_render::text;
        let mut q: Vec<mcv_render::HudQuad> = Vec::new();
        self.menu_hot.clear();
        self.world_hot.clear();
        let btn_w = (w * 0.36).clamp(180.0, 320.0);
        let btn_h = 44.0;
        let gap = 14.0;
        let x = w * 0.5 - btn_w * 0.5;
        let row = |q: &mut Vec<mcv_render::HudQuad>,
                   hot: &mut Vec<MenuButton>,
                   y: f32,
                   label: &str,
                   id: &'static str,
                   accent: [f32; 4]| {
            q.push(text::rect(x, y, btn_w, btn_h, accent));
            let scale = 2.0;
            let tw = label.chars().count() as f32 * 16.0 * scale * 0.62;
            q.extend(text::text_quads(
                label,
                w * 0.5 - tw * 0.5,
                y + btn_h * 0.5 - 16.0,
                scale,
                [1.0, 1.0, 1.0, 0.95],
            ));
            hot.push(MenuButton {
                id,
                x,
                y,
                w: btn_w,
                h: btn_h,
            });
        };
        match self.screen {
            Screen::Main => {
                // 标题
                q.extend(text::text_quads(
                    "MCV",
                    w * 0.5 - 96.0,
                    h * 0.22,
                    6.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let y0 = h * 0.42;
                for (i, (label, id)) in [
                    ("单人游戏", "single"),
                    ("设置", "settings"),
                    ("退出", "quit"),
                ]
                .iter()
                .enumerate()
                {
                    row(
                        &mut q,
                        &mut self.menu_hot,
                        y0 + i as f32 * (btn_h + gap),
                        label,
                        id,
                        [0.15, 0.16, 0.2, 0.82],
                    );
                }
            }
            Screen::Worlds => {
                q.extend(text::text_quads(
                    "选择世界",
                    w * 0.5 - 80.0,
                    h * 0.1,
                    3.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                // 世界列表（最多 5 行）
                for (i, entry) in self.worlds.iter().take(5).enumerate() {
                    let y = h * 0.2 + i as f32 * (btn_h + 10.0);
                    q.push(text::rect(
                        x,
                        y,
                        btn_w * 1.7,
                        btn_h,
                        [0.12, 0.13, 0.17, 0.85],
                    ));
                    let label = format!("{}  [{}]", entry.name, entry.mode);
                    q.extend(text::text_quads(
                        &label,
                        x + 16.0,
                        y + 10.0,
                        2.0,
                        [0.95, 0.95, 0.95, 1.0],
                    ));
                    self.world_hot.push((i, x, y, btn_w * 1.7, btn_h));
                }
                let y0 = h - (btn_h + gap) * 2.0 - 24.0;
                row(
                    &mut q,
                    &mut self.menu_hot,
                    y0,
                    "创建新世界",
                    "create",
                    [0.13, 0.3, 0.16, 0.85],
                );
                row(
                    &mut q,
                    &mut self.menu_hot,
                    y0 + btn_h + gap,
                    "返回",
                    "back",
                    [0.15, 0.16, 0.2, 0.82],
                );
            }
            Screen::Create => {
                q.extend(text::text_quads(
                    "创建新世界",
                    w * 0.5 - 100.0,
                    h * 0.14,
                    3.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let mode_names = ["生存", "创造", "极限"];
                let y0 = h * 0.34;
                // 模式选择按钮
                q.push(text::rect(x, y0, btn_w, btn_h, [0.2, 0.14, 0.3, 0.85]));
                let label = format!("模式：{}", mode_names[self.create_mode]);
                q.extend(text::text_quads(
                    &label,
                    w * 0.5 - 80.0,
                    y0 + 12.0,
                    2.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                self.menu_hot.push(MenuButton {
                    id: "mode",
                    x,
                    y: y0,
                    w: btn_w,
                    h: btn_h,
                });
                // 模式说明
                let desc = match self.create_mode {
                    1 => "飞行、瞬间挖掘、不受伤害",
                    2 => "同生存，死亡即删除世界",
                    _ => "收集资源、建造、冒险",
                };
                q.extend(text::text_quads(
                    desc,
                    w * 0.5 - 176.0,
                    y0 + btn_h + 6.0,
                    1.5,
                    [0.8, 0.8, 0.8, 0.9],
                ));
                let y1 = y0 + btn_h + 48.0;
                row(
                    &mut q,
                    &mut self.menu_hot,
                    y1,
                    "创建世界",
                    "go",
                    [0.13, 0.3, 0.16, 0.85],
                );
                row(
                    &mut q,
                    &mut self.menu_hot,
                    y1 + btn_h + gap,
                    "返回",
                    "back",
                    [0.15, 0.16, 0.2, 0.82],
                );
            }
            Screen::Settings => {
                q.extend(text::text_quads(
                    "设置",
                    w * 0.5 - 40.0,
                    h * 0.12,
                    3.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let y0 = h * 0.28;
                // 渲染距离
                q.push(text::rect(x, y0, btn_w, btn_h, [0.15, 0.16, 0.2, 0.82]));
                let dlabel = format!("渲染距离：{} 区块", self.set_dist);
                q.extend(text::text_quads(
                    &dlabel,
                    w * 0.5 - 96.0,
                    y0 + 12.0,
                    2.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                self.menu_hot.push(MenuButton {
                    id: "dist-",
                    x,
                    y: y0,
                    w: btn_w,
                    h: btn_h,
                });
                self.menu_hot.push(MenuButton {
                    id: "dist+",
                    x: x + btn_w,
                    y: y0,
                    w: 48.0,
                    h: btn_h,
                });
                q.extend(text::text_quads(
                    "+",
                    x + btn_w + 12.0,
                    y0 + 14.0,
                    2.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                // 灵敏度
                let y1 = y0 + btn_h + gap;
                q.push(text::rect(x, y1, btn_w, btn_h, [0.15, 0.16, 0.2, 0.82]));
                let slabel = format!("灵敏度：{:.2}x", self.set_sens);
                q.extend(text::text_quads(
                    &slabel,
                    w * 0.5 - 84.0,
                    y1 + 12.0,
                    2.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                self.menu_hot.push(MenuButton {
                    id: "sens-",
                    x,
                    y: y1,
                    w: btn_w,
                    h: btn_h,
                });
                self.menu_hot.push(MenuButton {
                    id: "sens+",
                    x: x + btn_w,
                    y: y1,
                    w: 48.0,
                    h: btn_h,
                });
                q.extend(text::text_quads(
                    "+",
                    x + btn_w + 12.0,
                    y1 + 14.0,
                    2.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let y2 = y1 + btn_h + gap * 2.0;
                row(
                    &mut q,
                    &mut self.menu_hot,
                    y2,
                    "返回",
                    "back",
                    [0.15, 0.16, 0.2, 0.82],
                );
            }
            Screen::Paused => {
                // 半透明遮罩
                q.push(text::rect(0.0, 0.0, w, h, [0.0, 0.0, 0.0, 0.55]));
                q.extend(text::text_quads(
                    "游戏暂停",
                    w * 0.5 - 80.0,
                    h * 0.2,
                    3.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let y0 = h * 0.36;
                row(
                    &mut q,
                    &mut self.menu_hot,
                    y0,
                    "回到游戏",
                    "resume",
                    [0.13, 0.3, 0.16, 0.85],
                );
                row(
                    &mut q,
                    &mut self.menu_hot,
                    y0 + btn_h + gap,
                    "设置",
                    "settings",
                    [0.15, 0.16, 0.2, 0.82],
                );
                row(
                    &mut q,
                    &mut self.menu_hot,
                    y0 + (btn_h + gap) * 2.0,
                    "保存并退出",
                    "savequit",
                    [0.3, 0.15, 0.13, 0.85],
                );
                // Paused -> settings 回来要回 Paused：借用 back 逻辑不行，改为在 on_menu_click 处理
                self.menu_hot.push(MenuButton {
                    id: "pause-settings-back",
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                });
                self.menu_hot.pop();
            }
            Screen::InGame => {}
        }
        q
    }

    /// 存档根目录（saves/）。
    fn saves_root(&self) -> std::path::PathBuf {
        #[cfg(target_os = "android")]
        {
            return self
                .android_data
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from("/data/local/tmp"))
                .join("saves");
        }
        #[cfg(not(target_os = "android"))]
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("saves")))
            .unwrap_or_else(|| std::path::PathBuf::from("saves"))
    }

    fn refresh_worlds(&mut self) {
        self.worlds.clear();
        let root = self.saves_root();
        let Ok(rd) = std::fs::read_dir(&root) else {
            return;
        };
        let mut dirs: Vec<_> = rd.flatten().collect();
        dirs.sort_by_key(|e| e.file_name());
        for d in dirs {
            let path = d.path();
            if !path.is_dir() {
                continue;
            }
            let Ok(bytes) = std::fs::read(path.join("level.meta")) else {
                continue;
            };
            let Ok(meta) = mcv_save::LevelMeta::decode(&bytes) else {
                continue;
            };
            let mode = match meta.mode {
                1 => "创造",
                2 => "极限",
                _ => "生存",
            };
            self.worlds.push(WorldEntry {
                dir: path,
                name: meta.name.clone(),
                mode,
            });
        }
    }

    /// 创建新世界并进入。
    fn create_world(&mut self) {
        let root = self.saves_root();
        let mut n = 1;
        while root.join(format!("world{n}")).exists() {
            n += 1;
        }
        let dir = root.join(format!("world{n}"));
        if std::fs::create_dir_all(&dir).is_err() {
            log::error!("cannot create world dir");
            return;
        }
        let mode = [GameMode::Survival, GameMode::Creative, GameMode::Hardcore][self.create_mode];
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 | (d.as_secs() << 8))
            .unwrap_or(12345)
            | 1;
        let device = self.runtime_device();
        let queue = self.runtime_queue();
        let (Some(device), Some(queue)) = (device, queue) else {
            log::error!("gpu not ready");
            return;
        };
        let mut runtime = GameRuntime::new(seed, device, queue, dir, mode);
        if mode == GameMode::Creative {
            runtime.player.flying = true;
        }
        runtime.save_meta();
        self.enter_game(runtime);
    }

    fn enter_game(&mut self, mut runtime: GameRuntime) {
        runtime.render_dist = self.set_dist;
        runtime.sens = self.set_sens;
        runtime.load_meta();
        runtime.save_meta();
        self.save_timer = Some(std::time::Instant::now());
        self.runtime = Some(runtime);
        self.screen = Screen::InGame;
        if let Some(w) = self.window.clone() {
            let _ = w.set_cursor_grab(winit::window::CursorGrabMode::Confined);
            w.set_cursor_visible(false);
            w.request_redraw();
        }
    }

    /// 退出到主菜单（保存并释放世界）。
    fn quit_to_menu(&mut self, delete: bool) {
        if let Some(mut runtime) = self.runtime.take() {
            runtime.save_dirty(None);
            runtime.save_meta();
            let dir = runtime.save_dir.clone();
            drop(runtime);
            if delete {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        self.screen = Screen::Main;
        self.refresh_worlds();
        if let Some(w) = self.window.clone() {
            let _ = w.set_cursor_grab(winit::window::CursorGrabMode::None);
            w.set_cursor_visible(true);
            w.request_redraw();
        }
    }

    /// 从运行时借出 device/queue（创建世界时 GameRuntime 还不存在）。
    fn runtime_device(&self) -> Option<wgpu::Device> {
        self.surface.as_ref()?;
        // GameRuntime 需要独立 device/queue；wgpu Device 是 Arc 型克隆。
        self.cached_device.clone()
    }

    fn runtime_queue(&self) -> Option<wgpu::Queue> {
        self.cached_queue.clone()
    }

    /// 菜单指针点击（桌面鼠标/触屏统一入口）。
    fn handle_menu_pointer(&mut self, x: f64, y: f64) {
        if self.screen == Screen::Worlds {
            let (px, py) = (x as f32, y as f32);
            let hit = self.world_hot.iter().find(|(_, bx, by, bw, bh)| {
                px >= *bx && px <= bx + bw && py >= *by && py <= by + bh
            });
            if let Some((idx, ..)) = hit {
                let idx = *idx;
                self.on_world_pick(idx);
                return;
            }
        }
        if let Some(id) = self.hit_menu(x, y) {
            if id == "quit" {
                self.quit_requested = true;
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
                return;
            }
            self.on_menu_click(id);
            // 设置实时应用到运行中的世界
            if self.screen == Screen::Paused || self.screen == Screen::InGame {
                if let Some(rt) = self.runtime.as_mut() {
                    rt.render_dist = self.set_dist;
                    rt.sens = self.set_sens;
                }
            }
        }
    }

    /// 点击命中测试（桌面鼠标 + 触屏共用）。返回命中的按钮 id。
    fn hit_menu(&self, x: f64, y: f64) -> Option<&'static str> {
        let (x, y) = (x as f32, y as f32);
        self.menu_hot
            .iter()
            .find(|b| x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h)
            .map(|b| b.id)
    }

    fn on_menu_click(&mut self, id: &'static str) {
        match (self.screen, id) {
            (Screen::Main, "single") => {
                self.refresh_worlds();
                self.screen = Screen::Worlds;
            }
            (Screen::Main, "settings") => self.screen = Screen::Settings,
            (Screen::Main, "quit") => {}
            (Screen::Worlds, "back") => self.screen = Screen::Main,
            (Screen::Worlds, "create") => self.screen = Screen::Create,
            (Screen::Create, "mode") => self.create_mode = (self.create_mode + 1) % 3,
            (Screen::Create, "go") => self.create_world(),
            (Screen::Create, "back") => self.screen = Screen::Worlds,
            (Screen::Settings, "dist+") => {
                self.set_dist = (self.set_dist + 2).min(SETTINGS_MAX_DIST)
            }
            (Screen::Settings, "dist-") => {
                self.set_dist = (self.set_dist - 2).max(SETTINGS_MIN_DIST)
            }
            (Screen::Settings, "sens+") => self.set_sens = (self.set_sens + 0.25).min(3.0),
            (Screen::Settings, "sens-") => self.set_sens = (self.set_sens - 0.25).max(0.25),
            (Screen::Settings, "back") => self.screen = Screen::Main,
            (Screen::Paused, "resume") => self.screen = Screen::InGame,
            (Screen::Paused, "savequit") => self.quit_to_menu(false),
            _ => {}
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Worlds 界面：点击某个世界进入。idx 由 UI 构建时写进 menu_hot 之外的旁路表。
    fn on_world_pick(&mut self, idx: usize) {
        let Some(entry) = self.worlds.get(idx) else {
            return;
        };
        let dir = entry.dir.clone();
        let device = self.runtime_device();
        let queue = self.runtime_queue();
        let (Some(device), Some(queue)) = (device, queue) else {
            return;
        };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(42);
        let mode = {
            let Ok(bytes) = std::fs::read(dir.join("level.meta")) else {
                return;
            };
            match mcv_save::LevelMeta::decode(&bytes) {
                Ok(m) => GameMode::from_u8(m.mode),
                Err(_) => GameMode::Survival,
            }
        };
        let runtime = GameRuntime::new(seed, device, queue, dir, mode);
        self.enter_game(runtime);
    }
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
                // 纹理包目录：<exe>/texturepack/（桌面）
                let pack_dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("texturepack")));
                let pack = pack_dir.filter(|d| d.is_dir());
                self.cached_device = Some(device.clone());
                self.cached_queue = Some(queue.clone());
                self.renderer = Some(mcv_render::Renderer::new(
                    device.clone(),
                    queue.clone(),
                    config.format,
                    pack.as_deref(),
                ));
                self.set_dist = 8;
                self.set_sens = 1.0;
                self.screen = Screen::Main;
                self.refresh_worlds();
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
                    KeyCode::Escape => match self.screen {
                        Screen::InGame => {
                            self.screen = Screen::Paused;
                            if let Some(w) = self.window.as_ref() {
                                let _ = w.set_cursor_grab(winit::window::CursorGrabMode::None);
                                w.set_cursor_visible(true);
                            }
                        }
                        Screen::Paused => self.screen = Screen::InGame,
                        Screen::Worlds | Screen::Create | Screen::Settings => {
                            self.screen = Screen::Main
                        }
                        Screen::Main => event_loop.exit(),
                    },
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
                if self.screen == Screen::InGame {
                    if let (Some(runtime), Some(last)) = (self.runtime.as_mut(), self.last_cursor) {
                        runtime.look(position.x - last.0, position.y - last.1);
                    }
                }
                self.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::Touch { .. } => {
                if self.screen != Screen::InGame {
                    // 菜单：触摸按下 = 点击
                    if let WindowEvent::Touch(t) = &event {
                        if t.phase == winit::event::TouchPhase::Started {
                            self.handle_menu_pointer(t.location.x, t.location.y);
                        }
                    }
                    return;
                }
                if let (Some(runtime), Some(sp)) = (self.runtime.as_mut(), self.surface.as_ref()) {
                    runtime
                        .touch
                        .on_event(&event, sp.config.width as f32, sp.config.height as f32);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.screen != Screen::InGame {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        if let Some(cur) = self.last_cursor {
                            self.handle_menu_pointer(cur.0, cur.1);
                        }
                    }
                    return;
                }
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
    /// 菜单帧：天空底色 + HUD。
    fn redraw_menu(&mut self) {
        {
            let Some(sp) = self.surface.as_mut() else {
                return;
            };
            let size = self
                .window
                .as_ref()
                .map(|w| w.inner_size())
                .unwrap_or(winit::dpi::PhysicalSize::new(0, 0));
            if size.width > 0
                && size.height > 0
                && (sp.config.width != size.width || sp.config.height != size.height)
            {
                sp.config.width = size.width;
                sp.config.height = size.height;
                sp.surface
                    .configure(self.cached_device.as_ref().unwrap(), &sp.config);
                sp.depth = Self::depth_view(
                    self.cached_device.as_ref().unwrap(),
                    size.width,
                    size.height,
                );
            }
        }
        let (mw, mh) = {
            let Some(sp) = self.surface.as_ref() else {
                return;
            };
            (sp.config.width as f32, sp.config.height as f32)
        };
        let hud = self.menu_ui(mw, mh);
        let camera = mcv_render::Camera {
            pos: glam::Vec3::new(0.0, 200.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 1.25,
            aspect: mw / mh,
            near: 0.1,
            far: 256.0,
        };
        let (sun, day) = mcv_render::sun_state(6_000);
        let scene = mcv_render::Scene {
            camera: &camera,
            time: 0.0,
            day_factor: day,
            sun_dir: sun,
            width: mw,
            height: mh,
            chunks: &[],
            hud: &hud,
        };
        let Some(sp) = self.surface.as_mut() else {
            return;
        };
        use wgpu::CurrentSurfaceTexture as Tex;
        let frame = match sp.surface.get_current_texture() {
            Tex::Success(f) | Tex::Suboptimal(f) => f,
            _ => {
                sp.surface
                    .configure(self.cached_device.as_ref().unwrap(), &sp.config);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let renderer = self.renderer.as_mut().unwrap();
        renderer.draw_frame(&view, &sp.depth, &scene);
        renderer.queue().present(frame);
    }

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
        if self.quit_requested {
            return; // window_event 在 RedrawRequested 后处理退出
        }
        if self.screen != Screen::InGame && self.screen != Screen::Paused {
            self.redraw_menu();
            return;
        }
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
            let device = self.cached_device.clone().unwrap();
            sp.surface.configure(&device, &sp.config);
            sp.depth = Self::depth_view(&device, size.width, size.height);
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
                    .configure(self.cached_device.as_ref().unwrap(), &sp.config);
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
        let renderer = self.renderer.as_mut().unwrap();
        renderer.draw_frame(&view, &sp.depth, &scene);
        renderer.queue().present(frame);
    }
}
