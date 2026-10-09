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

use mcv_logic::game::{GameMode, GameRuntime};

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
    /// 死亡界面（26.1 deathScreen）：游戏画面 + 红罩 + 重生/标题按钮。
    Death,
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

/// 主界面 splash 短语(自写;MC 原版 splashes.txt 是 Mojang 资产)。
const SPLASHES: [&str; 10] = [
    "100% Rust 打造!",
    "也支持安卓!",
    "小心苦力怕!",
    "挖到钻石了吗?",
    "试着睡一觉?",
    "别敲末影龙!",
    "方块无限好!",
    "现在就能玩!",
    "你好,方块世界!",
    "纯 Rust 引擎!",
];

/// GUI 整数缩放(统一走 mcv_render::gui_scale)。
use mcv_render::gui_scale;

const SETTINGS_MIN_DIST: i32 = 4;
const SETTINGS_MAX_DIST: i32 = 16;

#[cfg(target_os = "android")]
type AndroidApp = android_activity::AndroidApp;
#[cfg(not(target_os = "android"))]
type AndroidApp = ();

pub async fn run(android: Option<AndroidApp>) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = EventLoop::<()>::with_user_event();
    let mut state = AppState::default();
    #[cfg(target_os = "android")]
    {
        state.android_data = android
            .as_ref()
            .and_then(|a| a.internal_data_path().map(std::path::PathBuf::from));
        log::info!("android internal data: {:?}", state.android_data);
        if let Some(app) = android.as_ref()
            && let Some(data) = state.android_data.as_ref()
        {
            crate::android_assets::extract_assets(app, data);
        }
        if let Some(app) = android {
            builder.with_android_app(app);
        }
    }
    let _ = android;
    let event_loop = builder.build()?;
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
    /// 本启动会话随机 splash 短语
    splash: &'static str,
    /// splash 动画时间基准
    menu_t0: Option<std::time::Instant>,
    /// 界面语言：0=English 1=中文
    set_lang: usize,
    /// 渲染云（官方 CloudStatus）：0=OFF 1=FAST 2=FANCY
    set_clouds: usize,
    /// 云资源（GPU 缓冲 + 单元表，启动建一次）
    clouds: Option<mcv_render::Clouds>,
    /// 走路动画状态：(相位, 幅值)
    walk_anim: (f32, f32),
    /// 键位映射（MC KeyBindingRegistry 对应物；重映射 UI 属波4）
    keymap: mcv_game::keymap::KeyMap,
    /// Android 应用私有目录（internal_data_path），存档放这里
    #[cfg(target_os = "android")]
    android_data: Option<std::path::PathBuf>,
}

/// 将 surface 尺寸等比缩进设备纹理上限（GLES 常见 2048），present 时自动拉伸。
fn fit_surface_size(w: u32, h: u32, max: u32) -> (u32, u32) {
    let m = w.max(h);
    if max == 0 || m <= max || m == 0 {
        (w.max(1), h.max(1))
    } else {
        let k = max as f64 / m as f64;
        (
            ((w as f64 * k) as u32).max(1),
            ((h as f64 * k) as u32).max(1),
        )
    }
}

struct SurfacePair {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    max_extent: u32,
}

impl AppState {
    /// MC 风格按钮：真 MC button 三态贴图九宫格（缺素材回退纯色矩形）+
    /// 白色阴影文字居中；悬停换高亮贴图并染黄（MC AbstractButton 行为）。
    /// 逻辑 id 与旧版一致，只换皮。
    #[allow(clippy::too_many_arguments)]
    fn mc_button(
        q: &mut Vec<mcv_render::HudQuad>,
        hot: &mut Vec<MenuButton>,
        gui: Option<&mcv_render::gui::SpriteSheet>,
        hover: Option<(f32, f32)>,
        id: &'static str,
        x: f32,
        y: f32,
        bw: f32,
        bh: f32,
        label: &str,
        scale: f32,
        accent: [f32; 4],
        align_left: bool,
    ) {
        use mcv_render::text;
        let hovered =
            hover.is_some_and(|(mx, my)| mx >= x && mx <= x + bw && my >= y && my <= y + bh);
        match gui {
            Some(g) => {
                let sprite = if hovered { "button_hl" } else { "button" };
                q.extend(g.nine_slice(sprite, x, y, bw, bh, 3, scale, [1.0, 1.0, 1.0, 1.0]));
            }
            None => q.push(text::rect(x, y, bw, bh, accent)),
        }
        let color = if hovered && gui.is_some() {
            [1.0, 0.98, 0.6, 1.0] // MC 悬停黄
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };
        let tw = text::text_width(label, scale);
        let max_w = (bw - 4.0 * scale).max(8.0);
        let ts = if tw > max_w {
            scale * max_w / tw
        } else {
            scale
        };
        let tx = if align_left {
            x + 4.0 * scale
        } else {
            x + bw * 0.5 - text::text_width(label, ts) * 0.5
        };
        let ty = y + (bh - 8.0 * ts) * 0.5;
        q.extend(text::text_quads(label, tx, ty, ts, color));
        hot.push(MenuButton {
            id,
            x,
            y,
            w: bw,
            h: bh,
        });
    }

    /// MC 主界面 splash：logo 右下，-20° 旋转 + 脉动缩放 + 逐字符正弦
    /// 摆动，黄色带阴影（SplashRenderer 常数，见 NOTES-ui.md）。
    fn splash_quads(&self, w: f32, h: f32, s: f32, t: f32) -> Vec<mcv_render::HudQuad> {
        use mcv_render::font::{advance, glyph_uv};
        let mut q = Vec::new();
        if self.splash.is_empty() {
            return q;
        }
        let _ = h;
        let text_w: f32 = self.splash.chars().map(|c| advance(c as u32)).sum();
        if text_w <= 0.0 {
            return q;
        }
        let phase = 1.8 - ((t * std::f32::consts::TAU).sin().abs() * 0.1);
        let scale_u = phase * 100.0 / (text_w + 32.0);
        let px = scale_u * s;
        let rot = -std::f32::consts::PI / 9.0;
        let (cs, sn) = (rot.cos(), rot.sin());
        let anchor = (w * 0.5 + 123.0 * s, 69.0 * s);
        let yellow = [1.0, 1.0, 0.33, 1.0]; // MC 0xFFFFFF55
        let yshadow = [0.25, 0.25, 0.08, 1.0];
        let mut lx = -text_w * 0.5;
        for (i, ch) in self.splash.chars().enumerate() {
            let adv = advance(ch as u32);
            if ch != ' ' {
                let code = ch as u32;
                let cell = if (32..256).contains(&code) && adv > 1.0 {
                    code
                } else {
                    b'?' as u32
                };
                // 逐字符正弦摆动（任务规定，Bedrock 风格）
                let wave = (i as f32 * 0.5 + t * 3.0).sin() * 0.8;
                let gx = lx + adv * 0.5;
                let gy = -4.0 + wave; // 局部中心（顶 -8 + 半高 4）
                for (ox, oy, col) in [(1.0, 1.0, yshadow), (0.0, 0.0, yellow)] {
                    let wx = anchor.0 + ox + px * (gx * cs - gy * sn);
                    let wy = anchor.1 + oy + px * (gx * sn + gy * cs);
                    q.push(mcv_render::HudQuad {
                        x: wx - 4.0 * px,
                        y: wy - 4.0 * px,
                        w: 8.0 * px,
                        h: 8.0 * px,
                        uv: glyph_uv(cell),
                        color: col,
                        tex: 0,
                        layer: 0,
                        rot,
                    });
                }
            }
            lx += adv;
        }
        q
    }

    /// 构建菜单界面（HUD quad + 按钮命中表）。MC 26.1 风格：dirt 铺贴
    /// 背景、原版 logo + splash、button 三态贴图；素材缺失自动回退。
    fn menu_ui(&mut self, w: f32, h: f32) -> Vec<mcv_render::HudQuad> {
        use mcv_render::text;
        let mut q: Vec<mcv_render::HudQuad> = Vec::new();
        self.menu_hot.clear();
        self.world_hot.clear();
        let s = gui_scale(h);
        let gui: Option<&mcv_render::gui::SpriteSheet> =
            self.renderer.as_ref().and_then(|r| r.gui());
        let hover = self.last_cursor.map(|(x, y)| (x as f32, y as f32));
        let t = self
            .menu_t0
            .map(|t0| t0.elapsed().as_secs_f32())
            .unwrap_or(0.0);

        // 背景：dirt 平铺 × 0.4 亮度（暂停/死亡界面保留游戏画面 + 遮罩）
        if !matches!(self.screen, Screen::Paused | Screen::Death) {
            let tile = 16.0 * s;
            let mut y = 0.0;
            let mut n = 0u32;
            while y < h && n < 1400 {
                let mut x = 0.0;
                while x < w {
                    q.push(mcv_render::HudQuad {
                        x,
                        y,
                        w: tile,
                        h: tile,
                        uv: [[0.0, 0.0], [1.0, 1.0]],
                        color: [0.4, 0.4, 0.4, 1.0],
                        tex: 1,
                        layer: u32::from(mcv_core::tiles::DIRT),
                        rot: 0.0,
                    });
                    x += tile;
                    n += 1;
                }
                y += tile;
            }
        }

        // 通用按钮布局常数（MC：宽 200、高 20、间距 4，GUI 单位）
        let btn_w = (200.0 * s).min(w * 0.9);
        let btn_h = 20.0 * s;
        let gap = 4.0 * s;
        let x = w * 0.5 - btn_w * 0.5;

        match self.screen {
            Screen::Main => {
                if let Some(g) = gui {
                    // logo：256x44 显示（纹理上 44/64 行），居中，保比例
                    let lw = (256.0 * s).min(w * 0.9);
                    let lh = lw * (mcv_render::gui::LOGO_VISIBLE_H as f32 / 256.0);
                    if let Some(qd) = g.sprite(
                        "logo",
                        0.0,
                        0.0,
                        1.0,
                        mcv_render::gui::LOGO_VISIBLE_H as f32 / 64.0,
                        (w - lw) * 0.5,
                        30.0 * s,
                        lw,
                        lh,
                        [1.0, 1.0, 1.0, 1.0],
                    ) {
                        q.push(qd);
                    }
                    q.extend(self.splash_quads(w, h, s, t));
                } else {
                    // 回退：程序化标题
                    q.extend(text::text_quads_centered(
                        "MCV",
                        w * 0.5,
                        h * 0.22,
                        4.0 * s,
                        [1.0, 1.0, 1.0, 1.0],
                    ));
                }
                let lang = self.lang();
                let y0 = (h * 0.48).max(30.0 * s + 44.0 * s + 24.0 * s);
                for (i, (label, id)) in [
                    (crate::i18n::t(lang, "menu.singleplayer"), "single"),
                    (crate::i18n::t(lang, "menu.options"), "settings"),
                    (crate::i18n::t(lang, "menu.quit"), "quit"),
                ]
                .iter()
                .enumerate()
                {
                    Self::mc_button(
                        &mut q,
                        &mut self.menu_hot,
                        gui,
                        hover,
                        id,
                        x,
                        y0 + i as f32 * (btn_h + gap),
                        btn_w,
                        btn_h,
                        label,
                        s,
                        [0.15, 0.16, 0.2, 0.82],
                        false,
                    );
                }
            }
            Screen::Worlds => {
                let lang = self.lang();
                q.extend(text::text_quads_centered(
                    crate::i18n::t(lang, "mcv.selectWorld.title"),
                    w * 0.5,
                    12.0 * s,
                    s,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                // 世界列表（最多 5 行）：button 贴图行，文字左对齐
                for (i, entry) in self.worlds.iter().take(5).enumerate() {
                    let y = 30.0 * s + i as f32 * (btn_h + gap);
                    let label = format!("{}  [{}]", entry.name, entry.mode);
                    match gui {
                        Some(g) => {
                            let hovered = hover.is_some_and(|(mx, my)| {
                                mx >= x && mx <= x + btn_w && my >= y && my <= y + btn_h
                            });
                            let sprite = if hovered { "button_hl" } else { "button" };
                            q.extend(g.nine_slice(sprite, x, y, btn_w, btn_h, 3, s, [1.0; 4]));
                            q.extend(text::text_quads(
                                &label,
                                x + 4.0 * s,
                                y + (btn_h - 8.0 * s) * 0.5,
                                s,
                                [1.0, 1.0, 1.0, 1.0],
                            ));
                        }
                        None => {
                            q.push(text::rect(x, y, btn_w, btn_h, [0.12, 0.13, 0.17, 0.85]));
                            q.extend(text::text_quads(
                                &label,
                                x + 8.0,
                                y + btn_h * 0.5 - 4.0 * s,
                                s,
                                [0.95, 0.95, 0.95, 1.0],
                            ));
                        }
                    }
                    self.world_hot.push((i, x, y, btn_w, btn_h));
                }
                let y0 = h - (btn_h + gap) * 2.0 - 8.0 * s;
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "create",
                    x,
                    y0,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "selectWorld.create"),
                    s,
                    [0.13, 0.3, 0.16, 0.85],
                    false,
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "back",
                    x,
                    y0 + btn_h + gap,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "gui.back"),
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
            }
            Screen::Create => {
                let lang = self.lang();
                q.extend(text::text_quads_centered(
                    crate::i18n::t(lang, "selectWorld.newWorld"),
                    w * 0.5,
                    12.0 * s,
                    s,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let mode_keys = [
                    "mcv.mode.survival",
                    "mcv.mode.creative",
                    "mcv.mode.hardcore",
                ];
                let y0 = h * 0.34;
                let label = format!(
                    "{}: {}",
                    crate::i18n::t(lang, "selectWorld.gameMode"),
                    crate::i18n::t(lang, mode_keys[self.create_mode])
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "mode",
                    x,
                    y0,
                    btn_w,
                    btn_h,
                    &label,
                    s,
                    [0.2, 0.14, 0.3, 0.85],
                    false,
                );
                // 模式说明
                let desc_key = match self.create_mode {
                    1 => "mcv.createWorld.creativeDesc",
                    2 => "mcv.createWorld.hardcoreDesc",
                    _ => "mcv.createWorld.survivalDesc",
                };
                q.extend(text::text_quads_centered(
                    crate::i18n::t(lang, desc_key),
                    w * 0.5,
                    y0 + btn_h + 6.0 * s,
                    s,
                    [0.8, 0.8, 0.8, 0.9],
                ));
                let y1 = y0 + btn_h + 30.0 * s;
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "go",
                    x,
                    y1,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "mcv.createWorld.create"),
                    s,
                    [0.13, 0.3, 0.16, 0.85],
                    false,
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "back",
                    x,
                    y1 + btn_h + gap,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "gui.back"),
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
            }
            Screen::Settings => {
                let lang = self.lang();
                q.extend(text::text_quads_centered(
                    crate::i18n::t(lang, "options.title"),
                    w * 0.5,
                    12.0 * s,
                    s,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let y0 = h * 0.22;
                // 渲染距离：主按钮 = 减，右侧 + 小按钮（逻辑不变）
                let dlabel = format!(
                    "{}: {}",
                    crate::i18n::t(lang, "options.renderDistance"),
                    self.set_dist
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "dist-",
                    x,
                    y0,
                    btn_w,
                    btn_h,
                    &dlabel,
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "dist+",
                    x + btn_w + gap,
                    y0,
                    20.0 * s,
                    btn_h,
                    "+",
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
                // 灵敏度
                let y1 = y0 + btn_h + gap;
                let slabel = format!(
                    "{}: {:.2}x",
                    crate::i18n::t(lang, "options.sensitivity"),
                    self.set_sens
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "sens-",
                    x,
                    y1,
                    btn_w,
                    btn_h,
                    &slabel,
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "sens+",
                    x + btn_w + gap,
                    y1,
                    20.0 * s,
                    btn_h,
                    "+",
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
                // 渲染云：官方 CloudStatus 三态循环 OFF→流畅→高品质（点击主按钮循环）
                let y2 = y1 + btn_h + gap;
                let cloud_cap = match self.set_clouds {
                    0 => crate::i18n::t(lang, "options.off"),
                    1 => crate::i18n::t(lang, "options.clouds.fast"),
                    _ => crate::i18n::t(lang, "options.clouds.fancy"),
                };
                let clabel = format!(
                    "{}: {}",
                    crate::i18n::t(lang, "options.renderClouds"),
                    cloud_cap
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "clouds",
                    x,
                    y2,
                    btn_w,
                    btn_h,
                    &clabel,
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
                // 语言：English ↔ 中文
                let y3 = y2 + btn_h + gap;
                let llabel = format!(
                    "{}: {}",
                    crate::i18n::t(lang, "mcv.options.languageTitle"),
                    if self.set_lang == 1 {
                        "中文"
                    } else {
                        "English"
                    }
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "lang",
                    x,
                    y3,
                    btn_w,
                    btn_h,
                    &llabel,
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
                let y4 = y3 + btn_h + gap * 2.0;
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "back",
                    x,
                    y4,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "gui.back"),
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
            }
            Screen::Paused => {
                let lang = self.lang();
                // 半透明遮罩（MC 暂停界面）
                q.push(text::rect(0.0, 0.0, w, h, [0.0, 0.0, 0.0, 0.55]));
                q.extend(text::text_quads_centered(
                    crate::i18n::t(lang, "menu.paused"),
                    w * 0.5,
                    h * 0.2,
                    s,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let y0 = h * 0.36;
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "resume",
                    x,
                    y0,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "menu.returnToGame"),
                    s,
                    [0.13, 0.3, 0.16, 0.85],
                    false,
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "settings",
                    x,
                    y0 + btn_h + gap,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "menu.options"),
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "savequit",
                    x,
                    y0 + (btn_h + gap) * 2.0,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "menu.returnToMenu"),
                    s,
                    [0.3, 0.15, 0.13, 0.85],
                    false,
                );
            }
            Screen::Death => {
                let lang = self.lang();
                // 26.1 deathScreen：红罩 + 大字标题（2x）+ 重生/标题屏按钮；
                // 极限模式隐藏重生（世界删除提示）。
                q.push(text::rect(0.0, 0.0, w, h, [0.45, 0.0, 0.0, 0.45]));
                q.extend(text::text_quads_centered(
                    crate::i18n::t(lang, "deathScreen.title"),
                    w * 0.5,
                    h * 0.22,
                    s * 2.0,
                    [1.0, 1.0, 1.0, 1.0],
                ));
                let hardcore = self
                    .runtime
                    .as_ref()
                    .is_some_and(|r| r.mode == GameMode::Hardcore);
                if hardcore {
                    q.extend(text::text_quads_centered(
                        crate::i18n::t(lang, "mcv.death.hardcoreInfo"),
                        w * 0.5,
                        h * 0.22 + 28.0 * s,
                        s,
                        [1.0, 0.6, 0.6, 1.0],
                    ));
                }
                let y0 = h * 0.45;
                let mut y = y0;
                if !hardcore {
                    Self::mc_button(
                        &mut q,
                        &mut self.menu_hot,
                        gui,
                        hover,
                        "respawn",
                        x,
                        y,
                        btn_w,
                        btn_h,
                        crate::i18n::t(lang, "deathScreen.respawn"),
                        s,
                        [0.13, 0.3, 0.16, 0.85],
                        false,
                    );
                    y += btn_h + gap;
                }
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "death_title",
                    x,
                    y,
                    btn_w,
                    btn_h,
                    crate::i18n::t(lang, "deathScreen.titleScreen"),
                    s,
                    [0.15, 0.16, 0.2, 0.82],
                    false,
                );
            }
            Screen::InGame => {}
        }
        q
    }

    /// 音效素材目录：Android 优先 internal 数据目录下 sounds/（APK 解包产物），
    /// 否则交给 mcv_audio::default_sounds_dir（env → workspace → ./sounds）。
    fn sounds_dir(&self) -> Option<std::path::PathBuf> {
        #[cfg(target_os = "android")]
        if let Some(d) = self.android_data.as_ref() {
            let p = d.join("sounds");
            if p.is_dir() {
                return Some(p);
            }
        }
        let p = mcv_audio::default_sounds_dir();
        p.is_dir().then_some(p)
    }

    /// 当前界面语言。
    fn lang(&self) -> crate::i18n::Lang {
        if self.set_lang == 1 {
            crate::i18n::Lang::Zh
        } else {
            crate::i18n::Lang::En
        }
    }

    /// 资源根（`assets/minecraft`，布局镜像原版 jar）。Android 优先 internal
    /// 数据目录解包出的 assets/minecraft/，否则开发期 workspace 根（CWD），
    /// 最后退回 <exe 目录>/assets/minecraft/（桌面 bundle 分发形态）。
    fn assets_dir(&self) -> Option<std::path::PathBuf> {
        const REL: &str = "assets/minecraft";
        #[cfg(target_os = "android")]
        if let Some(data) = &self.android_data {
            let dir = data.join(REL);
            if dir.is_dir() {
                return Some(dir);
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            let cwd = std::path::PathBuf::from(REL);
            if cwd.is_dir() {
                return Some(cwd);
            }
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join(REL)))
        }
        #[cfg(target_os = "android")]
        None
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
        let uploader = mcv_render::gpu::MeshUploader::new(device, queue);
        let mut runtime = GameRuntime::new(seed, uploader, dir, mode);
        if mode == GameMode::Creative {
            runtime.player.flying = true;
        }
        runtime.save_meta();
        self.enter_game(runtime);
    }

    fn enter_game(&mut self, mut runtime: GameRuntime) {
        runtime.render_dist = self.set_dist;
        runtime.sens = self.set_sens;
        // 音频后端：无声卡/无素材时保持 silent 降级，不阻塞进游戏
        if let Some(dir) = self.sounds_dir() {
            match mcv_audio::AudioManager::open(&dir) {
                Ok(a) => runtime.set_audio(a),
                Err(e) => log::warn!("audio unavailable ({dir:?}): {e}"),
            }
        }
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
            if (self.screen == Screen::Paused || self.screen == Screen::InGame)
                && let Some(rt) = self.runtime.as_mut()
            {
                rt.render_dist = self.set_dist;
                rt.sens = self.set_sens;
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
            // 官方 CloudStatus 点击循环：OFF→FAST→FANCY
            (Screen::Settings, "clouds") => self.set_clouds = (self.set_clouds + 1) % 3,
            (Screen::Settings, "lang") => self.set_lang = (self.set_lang + 1) % 2,
            (Screen::Settings, "back") => self.screen = Screen::Main,
            (Screen::Paused, "resume") => self.screen = Screen::InGame,
            (Screen::Paused, "savequit") => self.quit_to_menu(false),
            (Screen::Death, "respawn") => {
                if let Some(rt) = self.runtime.as_mut() {
                    rt.respawn();
                }
                self.screen = Screen::InGame;
                if let Some(w) = self.window.clone() {
                    let _ = w.set_cursor_grab(winit::window::CursorGrabMode::Confined);
                    w.set_cursor_visible(false);
                }
            }
            (Screen::Death, "death_title") => self.quit_to_menu(false),
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
        let uploader = mcv_render::gpu::MeshUploader::new(device, queue);
        let runtime = GameRuntime::new(seed, uploader, dir, mode);
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
            Ok((surface, config, depth, device, queue, max_extent)) => {
                // 资源根:Android 解包目录优先,桌面开发期 CWD(workspace 根),
                // bundle 形态 exe 同级(仅当目录存在时生效)
                let pack = self.assets_dir().filter(|d| d.is_dir());
                self.cached_device = Some(device.clone());
                self.cached_queue = Some(queue.clone());
                self.renderer = Some(mcv_render::Renderer::new(
                    device.clone(),
                    queue.clone(),
                    config.format,
                    pack.as_deref(),
                ));
                self.clouds = Some(mcv_render::Clouds::new(&device, &queue));
                // 玩家皮肤：原版 entity/player/{wide/steve,slim/alex}.png
                if let Some(dir) = self.assets_dir() {
                    let read = |p: &str| std::fs::read(dir.join("textures").join(p)).ok();
                    if let (Some(s), Some(a)) = (
                        read("entity/player/wide/steve.png"),
                        read("entity/player/slim/alex.png"),
                    ) && let Err(e) = self.renderer.as_mut().unwrap().load_skins(&s, &a)
                    {
                        log::warn!("skin load failed: {e}");
                    }
                }
                self.set_dist = 8;
                self.set_sens = 1.0;
                self.set_lang = 0;
                self.set_clouds = 2;
                // splash：本会话随机一条 + 动画时钟
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos())
                    .unwrap_or(7);
                self.splash = SPLASHES[(seed as usize) % SPLASHES.len()];
                self.menu_t0 = Some(std::time::Instant::now());
                self.screen = Screen::Main;
                self.refresh_worlds();
                self.surface = Some(SurfacePair {
                    surface,
                    config,
                    depth,
                    max_extent,
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
                if let (Some(sp), Some(_)) = (self.surface.as_mut(), self.runtime.as_ref())
                    && size.width > 0
                    && size.height > 0
                {
                    let (cw, ch) = fit_surface_size(size.width, size.height, sp.max_extent);
                    sp.config.width = cw;
                    sp.config.height = ch;
                    // reconfigure happens lazily in redraw
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
                use mcv_game::keymap::Action;
                // 快捷栏：MC 是 9 个独立键位，本引擎键位表单动作单键，暂直绑
                if pressed && let Some(slot) = mcv_platform::keybind::hotbar_slot(code) {
                    runtime.player.sel_slot = slot;
                }
                // F5 切视角不在 MC 键位表内（本引擎扩展），保持硬编码
                if pressed && code == KeyCode::F5 {
                    runtime.cycle_camera();
                }
                if let Some(vk) = mcv_platform::keybind::vkey_of(code) {
                    match self.keymap.action_for(vk) {
                        Some(Action::Forward) => runtime.input.forward = pressed,
                        Some(Action::Back) => runtime.input.back = pressed,
                        Some(Action::Left) => runtime.input.left = pressed,
                        Some(Action::Right) => runtime.input.right = pressed,
                        Some(Action::Jump) => runtime.input.jump = pressed,
                        Some(Action::Sneak) => runtime.input.sneak = pressed,
                        Some(Action::Sprint) => runtime.input.sprint = pressed,
                        Some(Action::FlyToggle) if pressed => {
                            runtime.player.flying = !runtime.player.flying;
                        }
                        Some(Action::Pause) if pressed => match self.screen {
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
                            Screen::Death => {} // MC：死亡界面 Esc 无效
                            Screen::Main => event_loop.exit(),
                        },
                        // Inventory(F)/Debug(F3)/Screenshot(F2)/PickBlock：
                        // 引擎侧功能属波4（背包/F3/截图），先接分发留位
                        _ => {}
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.screen == Screen::InGame
                    && let (Some(runtime), Some(last)) = (self.runtime.as_mut(), self.last_cursor)
                {
                    runtime.look(position.x - last.0, position.y - last.1);
                }
                self.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::Touch { .. } => {
                if self.screen != Screen::InGame {
                    // 菜单：触摸按下 = 点击
                    if let WindowEvent::Touch(t) = &event
                        && t.phase == winit::event::TouchPhase::Started
                    {
                        self.handle_menu_pointer(t.location.x, t.location.y);
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
                    if state == ElementState::Pressed
                        && button == MouseButton::Left
                        && let Some(cur) = self.last_cursor
                    {
                        self.handle_menu_pointer(cur.0, cur.1);
                    }
                    return;
                }
                let Some(runtime) = self.runtime.as_mut() else {
                    return;
                };
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        // 按下 = 攻 mob/开始挖掘，松开 = STOP 补判（26.1 语义，
                        // 与触摸挖按钮共用 on_left_press/release 入口）。
                        let was = runtime.input.mining;
                        runtime.input.mining = pressed;
                        if pressed {
                            runtime.on_left_press();
                        } else if was {
                            runtime.on_left_release();
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
    u32,
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
            if size.width > 0 && size.height > 0 {
                let (cw, ch) = fit_surface_size(size.width, size.height, sp.max_extent);
                if sp.config.width != cw || sp.config.height != ch {
                    sp.config.width = cw;
                    sp.config.height = ch;
                    sp.surface
                        .configure(self.cached_device.as_ref().unwrap(), &sp.config);
                    sp.depth = Self::depth_view(
                        self.cached_device.as_ref().unwrap(),
                        sp.config.width,
                        sp.config.height,
                    );
                }
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
            cloud: None,
            player: None,
            overlay: None,
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
        // downlevel_defaults 把 max_texture_dimension_2d 限到 2048；高刷屏
        // （如 2640 宽）configure 会直接验证失败 panic，这里放开到设备实际上限。
        let mut limits = wgpu::Limits::downlevel_defaults();
        limits.max_texture_dimension_2d = limits
            .max_texture_dimension_2d
            .max(adapter.limits().max_texture_dimension_2d);
        // 方块图集 831 层：Vulkan 桌面（≥2048）吃满，GLES 保持 256 由
        // gpu.rs 钳制兜底（ lavapipe 3907 / Metal 2048 / D3D 2048 均抬升 ）。
        limits.max_texture_array_layers = limits
            .max_texture_array_layers
            .max(adapter.limits().max_texture_array_layers);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("mcv-device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            }))?;
        let caps = surface.get_capabilities(&adapter);
        let size = window.inner_size();
        let max_extent = device.limits().max_texture_dimension_2d;
        let (sw, sh) = fit_surface_size(size.width, size.height, max_extent);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *caps.formats.first().ok_or("no surface formats")?,
            width: sw,
            height: sh,
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
        Ok((surface, config, depth, device, queue, max_extent))
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
        if !matches!(self.screen, Screen::InGame | Screen::Paused | Screen::Death) {
            self.redraw_menu();
            return;
        }
        // 暂停/死亡：先构建 MC 风格菜单 quad(借用 self，须在 runtime 借用之前)
        let pause_menu = if matches!(self.screen, Screen::Paused | Screen::Death) {
            match self.surface.as_ref() {
                Some(sp) => self.menu_ui(sp.config.width as f32, sp.config.height as f32),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let (Some(sp), Some(runtime), Some(window)) = (
            self.surface.as_mut(),
            self.runtime.as_mut(),
            self.window.clone(),
        ) else {
            return;
        };
        let size = window.inner_size();
        if size.width > 0 && size.height > 0 {
            let (cw, ch) = fit_surface_size(size.width, size.height, sp.max_extent);
            if sp.config.width != cw || sp.config.height != ch {
                sp.config.width = cw;
                sp.config.height = ch;
                let device = self.cached_device.clone().unwrap();
                sp.surface.configure(&device, &sp.config);
                sp.depth = Self::depth_view(&device, sp.config.width, sp.config.height);
            }
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
        // 死亡 → 切死亡界面（显示 26.1 deathScreen，需鼠标点按钮）
        if runtime.dead && self.screen == Screen::InGame {
            self.screen = Screen::Death;
            if let Some(w) = self.window.as_ref() {
                let _ = w.set_cursor_grab(winit::window::CursorGrabMode::None);
                w.set_cursor_visible(true);
            }
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
        let mut hud = runtime.build_hud(
            sp.config.width as f32,
            sp.config.height as f32,
            self.renderer.as_ref().and_then(|r| r.gui()),
            self.screen != Screen::Death,
        );
        if matches!(self.screen, Screen::Paused | Screen::Death) {
            // 暂停/死亡：游戏画面之上叠 MC 风格菜单（按钮贴图 + 阴影字体）
            hud.extend(pause_menu);
        }
        let chunks: Vec<mcv_render::RenderChunk> = runtime.render_chunks().to_vec();
        // 云（官方 CloudStatus 映射到模块设置；云距跟随渲染距离，MC renderDistance 语义）
        let cloud_settings = mcv_render::CloudSettings {
            enabled: self.set_clouds != 0,
            fast: self.set_clouds == 1,
            distance: (runtime.render_dist * 16) as f32,
            ..Default::default()
        };
        let clouds = self.clouds.as_ref();
        // 第三人称玩家：走路动画推进 + 12 部位矩阵
        let third_person = runtime.cam_type != mcv_logic::game::CameraType::FirstPerson;
        let has_player = third_person
            && self.renderer.as_ref().is_some_and(|r| r.has_skins())
            && !runtime.hardcore_death;
        let mut models = [glam::Mat4::IDENTITY; 12];
        if has_player {
            let speed = glam::Vec3::new(runtime.player.vel.x, 0.0, runtime.player.vel.z).length();
            self.walk_anim =
                mcv_render::update_walk_animation(self.walk_anim.0, self.walk_anim.1, speed, dt);
            let pose = mcv_render::PlayerPose {
                pos: runtime.player.pos,
                yaw: runtime.player.yaw,
                pitch: runtime.player.pitch,
                phase: self.walk_anim.0,
                amount: self.walk_anim.1,
            };
            models = mcv_render::model_matrices(&pose);
        }
        let overlay = runtime.mining_overlay();
        let scene = mcv_render::Scene {
            camera: &camera,
            time: (runtime.time_ticks % 24_000) as f32 / 20.0,
            day_factor: day,
            sun_dir: sun,
            width: sp.config.width as f32,
            height: sp.config.height as f32,
            chunks: &chunks,
            hud: &hud,
            cloud: clouds.map(|c| (c, cloud_settings)),
            player: has_player.then_some((&models, 0)),
            overlay,
        };
        let renderer = self.renderer.as_mut().unwrap();
        renderer.draw_frame(&view, &sp.depth, &scene);
        renderer.queue().present(frame);
    }
}
