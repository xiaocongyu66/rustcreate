//! winit 0.30 shell: window + surface + input → [`GameRuntime`].

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
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
    /// 按键重映射界面（MC KeyBindsScreen）。
    KeyBinds,
    /// 合成界面（随身 2x2 / 工作台 3x3 + 36 格背包），游戏画面做底。
    Crafting,
    /// 创造模式取物页（分页），游戏画面做底。
    Creative,
}

/// 该界面是否停游戏 tick —— 原版 `Screen.isPauseScreen()` 的本仓映射
/// （26.1 Screen.java:426-428 默认 true；PauseScreen 无覆写 → 暂停；
/// DeathScreen.java:149-151 与 AbstractContainerScreen.java:640-642
/// 覆写 false → 死亡/容器界面世界照常跑）。本仓 Screen::Paused 是唯一
/// isPauseScreen=true 的界面；redraw 据此门掉 fixed_step 喂步
/// （Minecraft.java:1300-1308 每帧重算 pause、:1773 tick() `!this.pause` 门）。
fn is_pause_screen(screen: Screen) -> bool {
    matches!(screen, Screen::Paused)
}

/// 玩法输入（移动/跳跃/快捷栏/视角）是否放行 —— 原版 KeyMapping 只在
/// Screen==null 时驱动游戏（有屏时键盘归屏幕，暂停菜单里玩家不能走动；
/// 死亡界面输入由 game.rs 每步清零，此处一并门掉）。
/// 加载态另由 `GamePhase::Loading` 门（LevelLoadingScreen 语义，先于本门）。
fn gameplay_input_allowed(screen: Screen) -> bool {
    matches!(screen, Screen::InGame)
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

/// 快捷栏槽数（26.1 `Inventory.getSelectionSize()` = 9）。
const HOTBAR_SIZE: usize = 9;

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
    /// MeshUploader（#80 staging 异步上传）需要 queue 冲刷拷贝；与 device
    /// 同源缓存（wgpu Queue 是 Arc 型克隆）。
    cached_queue: Option<wgpu::Queue>,
    /// #80 结构性指标日志节拍（~1 Hz 一行，取证不刷屏）。
    stats_log: Option<std::time::Instant>,
    last_cursor: Option<(f64, f64)>,
    step_accum: f32,
    last_time: Option<std::time::Instant>,
    save_timer: Option<std::time::Instant>,
    /// B-4（波#127-3）：窗口失焦时刻。原版 `pauseIfInactive`
    /// （Minecraft.java:1388-1397）：失焦 && pauseOnLostFocus（默认 true，
    /// Options.java:320）&& 距最后活动 >500ms → pauseGame。None=有焦。
    focus_lost_at: Option<std::time::Instant>,
    /// C-2（波#127-3）：Android 挂起前的主增益（后台静音，回前台恢复）。
    gain_before_suspend: Option<f32>,
    /// D-6（波#127-5）：滚轮换槽累计器（原版 `ScrollWheelHandler` 移植，
    /// 见 [`mcv_platform::keybind::ScrollWheelSelection`]）。
    scroll_hotbar: mcv_platform::keybind::ScrollWheelSelection,
    /// D-2（波#127-6）：双击前进冲刺窗（见 [`SprintTapWindow`]）。
    sprint_tap: SprintTapWindow,
    /// I-21（波#127-4）：桌面右键按住态（周期发火的 held 输入之一）。
    right_use_held: bool,
    /// I-21：触摸放置键上一帧按住态（检测按下沿置 rightClickDelay）。
    touch_place_prev: bool,
    /// I-21：rightClickDelay 周期门（见 [`RightUseRepeat`]）。
    use_repeat: RightUseRepeat,
    /// 本启动会话随机 splash 短语
    splash: &'static str,
    /// splash 动画时间基准
    menu_t0: Option<std::time::Instant>,
    /// App 级菜单音频（本波 #143 UI 点击音）：单一输出设备所有权链——
    /// 菜单期由 App 持有（主菜单/设置/键位屏的按钮音），进世界转手给
    /// runtime（enter_game），退世界收回（quit_to_menu）。None=尚未打开
    /// 或已转手；游戏内屏（暂停/死亡）的按钮音走 runtime.audio（同设备）。
    menu_audio: Option<mcv_audio::AudioManager>,
    /// 音频后端打开失败（无声卡/无素材）后的不再重试旗标：点击静默降级。
    menu_audio_dead: bool,
    /// 界面语言：0=English 1=中文
    set_lang: usize,
    /// 渲染云（官方 CloudStatus）：0=OFF 1=FAST 2=FANCY
    set_clouds: usize,
    /// 云资源（GPU 缓冲 + 单元表，启动建一次）
    clouds: Option<mcv_render::Clouds>,
    /// 走路动画状态：(相位, 幅值)
    walk_anim: (f32, f32),
    /// 键位映射（MC KeyBindingRegistry 对应物；持久化 keybindings.txt）
    keymap: mcv_game::keymap::KeyMap,
    /// mcv_ui GUI 上下文（GUI scale 换算；M8b 试点：KeyBinds 屏）
    ui_ctx: mcv_ui::UiContext,
    /// mcv_ui 屏幕栈（KeyBinds 屏已在栈上；其余屏迁移清单见报告）
    ui_stack: mcv_ui::ScreenStack,
    /// 合成界面状态（仅 Screen::Crafting 期间 Some）。
    craft: Option<mcv_logic::ui::CraftScreen>,
    /// 创造取物页状态（仅 Screen::Creative 期间 Some）。
    creative: Option<mcv_logic::ui::CreativePicker>,
    /// 合成/创造界面命中矩形（每帧随 craft_ui 重建）。
    craft_hot: Vec<crate::craft_ui::HotRect>,
    /// 波 #126 接线：合成界面的容器状态机（engine/mcv_ui ContainerScreen，
    /// AbstractContainerMenu.doClick 高保真移植）。Screen::Crafting 期间存在；
    /// items/carried = 网格+结果槽+36 背包+光标镜像（StackRef.meta 令牌无损
    /// 往返耐久/附魔），每次交互经 `craft_flush_menu` 灌入→doClick→回写。
    craft_menu: Option<mcv_ui::slot::ContainerScreen>,
    /// 容器界面按住的按钮位图（bit0 左 / bit1 右）：拖拽 mouseDragged 路由。
    craft_held: u8,
    /// 双击判定基准（ACS:319 lastClickSlot + 250ms；winit CursorInput 无
    /// doubleClick 标志，游戏层自计时）：(槽 id，-999=面板外/-1=无槽, 按钮, 时刻)。
    craft_last_click: Option<(i32, u8, std::time::Instant)>,
    /// 修饰键态（shift=QUICK_MOVE，ctrl=Q 整堆丢）。
    shift_held: bool,
    ctrl_held: bool,
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
    /// HDR 中间链（`hdr` feature，默认关；None = 未启用/GLES 回退，
    /// 直通现状 Rgba8UnormSrgb 路径）。
    #[cfg(feature = "hdr")]
    hdr: Option<mcv_render::hdr::HdrStage>,
}

/// 抓/放鼠标（进游戏 Confined+隐藏，开界面/暂停 None+显示）。
fn set_cursor(window: &Option<Arc<Window>>, grab: bool) {
    let Some(w) = window else { return };
    let _ = w.set_cursor_grab(if grab {
        winit::window::CursorGrabMode::Confined
    } else {
        winit::window::CursorGrabMode::None
    });
    w.set_cursor_visible(!grab);
}

/// 本帧要喂的固定步数（redraw 的 tick 喂入点，抽成纯函数供回归测试）。
/// 暂停 = 0 步（B-1，原版 `!this.pause` 门）；否则累加墙钟差、按 1/60s
/// 出步——追帧预算钳 0.2s（A-3 已登记债：原版 0.5s，本波不动）。
fn feed_steps(paused: bool, step_accum: &mut f32, dt: f32) -> u32 {
    if paused {
        return 0;
    }
    *step_accum = (*step_accum + dt).min(0.2);
    let mut n = 0;
    while *step_accum >= 1.0 / 60.0 {
        *step_accum -= 1.0 / 60.0;
        n += 1;
    }
    n
}

/// 按住 use（右键/放置键）周期触发门 —— I-21（波#127-4）。
/// 26.1 `rightClickDelay` 机器：`startUseItem` 入口置 `rightClickDelay=4`
/// （Minecraft.java:1705）；`tick()` 每 tick 减 1（:1779-1781）；
/// `handleKeybinds` 在 `keyUse.isDown() && rightClickDelay==0 &&
/// !player.isUsingItem()` 再次 `startUseItem`（:2036-2038）——即 fire tick +
/// 4 tick 冷却共 5 个 tick 槽、每 4 tick 一发（5 次/秒，审计 I-21「周期
/// 5 tick」）。本仓按 1/60s 喂 fixed_step（3 步/tick），换算成步数：
/// 冷却 4 tick = 12 步，同 tick「先减后判」= 第 12 步减到 0 当步发火。
#[derive(Default)]
struct RightUseRepeat {
    cooldown_steps: u32,
}

/// 冷却计数换算：原版 rightClickDelay=4 tick（game.rs 进食重触发用的
/// `mcv_item::food::RIGHT_CLICK_DELAY_TICKS` 同源值）× 3 步/tick
/// （本仓 1/60s 喂步，mcv_logic accumulate_ticks `frac += dt*20`）。
const RIGHT_CLICK_DELAY_STEPS: u32 = mcv_item::food::RIGHT_CLICK_DELAY_TICKS * 3;

impl RightUseRepeat {
    /// 是否可立即发火（按下沿走这条：原版 handleKeybinds 的 delay==0 门——
    /// 快速连点同样被 4 tick 节流，原版连点宏存在的原因）。
    fn ready(&self) -> bool {
        self.cooldown_steps == 0
    }
    /// 发火记账（调用方执行 use）：原版 startUseItem :1705 无条件置延迟。
    fn on_fire(&mut self) {
        self.cooldown_steps = RIGHT_CLICK_DELAY_STEPS;
    }
    /// 每固定步推进。`held`=右键或触摸放置键仍按住；`busy`=isUsingItem
    /// （进食中，原版 :2036 第三门——interact 重入会重置 eat_hold 把进食
    /// 从头再来，必须挡）。返回 true = 本步发火。
    fn step(&mut self, held: bool, busy: bool) -> bool {
        if self.cooldown_steps > 0 {
            self.cooldown_steps -= 1;
            if self.cooldown_steps > 0 {
                return false;
            }
        }
        if held && !busy {
            self.on_fire();
            return true;
        }
        false
    }
}

/// 右键/放置键的一次 use（按下沿与周期发火共用）：工作台在准星下 = 开
/// 3x3 合成界面（26.1 CraftingTableBlock 打开容器），拦截在放置之前；
/// 其余走 interact(true)（方块 use 优先后物品 use/进食/放置，game.rs 内）。
fn right_use(
    runtime: &mut GameRuntime,
    screen: &mut Screen,
    craft: &mut Option<mcv_logic::ui::CraftScreen>,
    craft_menu: &mut Option<mcv_ui::slot::ContainerScreen>,
    craft_held: &mut u8,
    craft_last_click: &mut Option<(i32, u8, std::time::Instant)>,
    window: &Option<Arc<Window>>,
) {
    let on_table = runtime
        .look_block()
        .is_some_and(|(_, id)| id == crafting_table_id());
    if on_table {
        *screen = Screen::Crafting;
        *craft = Some(mcv_logic::ui::CraftScreen::new(3));
        // 容器波镜像同步初始化（app.rs make_craft_menu 契约：开屏即建，
        // 每次交互灌回写；漏建 = 工作台开屏无菜单）。
        *craft_menu = Some(make_craft_menu(3, runtime.mode == GameMode::Creative));
        *craft_held = 0;
        *craft_last_click = None;
        set_cursor(window, false);
    } else {
        runtime.interact(true);
    }
}

/// 双击前进冲刺窗（D-2，波#127-6）。26.1 `LocalPlayer.aiStep`：
/// `sprintTriggerTime` 每 tick −1（:764-766）；潜行按住/使用减速/后退清窗
/// （:798-800）；前进侧（:802-810，`hasForwardImpulse` 取 :775 的**上一 tick**
/// 快照，input.tick() :784 在后——净效果即「按下沿开/耗窗」）：窗口内 →
/// `setSprinting(true)`，否则开窗 `options.sprintWindow()`（Options.java:580-587
/// 默认 **7** tick，0=关）。冲刺维持到条件破坏（`shouldStopRunSprinting`
/// :918-922：失去前进冲量即停；潜行清窗 :798）。本仓消费面是逐 tick 布尔
/// `input.sprint`（game.rs:2376），latch 语义等价 setSprinting。
/// 60Hz 步长换算：7 tick = 21 步。
#[derive(Default)]
struct SprintTapWindow {
    window_steps: u32,
    latched: bool,
    key_held: bool,
}

/// 原版 sprintWindow 默认 7 tick × 3 步/tick（与 RIGHT_CLICK_DELAY_STEPS
/// 同一换算）。
const SPRINT_WINDOW_STEPS: u32 = 7 * 3;

impl SprintTapWindow {
    /// 前进键事件：按下沿开窗/耗窗（窗口内二次按下 = 冲刺点火），松开沿
    /// 解除 latch（shouldStopRunSprinting 的失去前进冲量语义在输入层的投影）。
    fn on_forward(&mut self, pressed: bool) {
        if pressed {
            if self.window_steps > 0 {
                self.latched = true;
                self.window_steps = 0;
            } else {
                self.window_steps = SPRINT_WINDOW_STEPS;
            }
        } else {
            self.latched = false;
        }
    }
    /// 潜行/后退按下沿清窗清 latch（LocalPlayer:798-800）。
    fn on_sneak_or_back(&mut self) {
        self.window_steps = 0;
        self.latched = false;
    }
    /// 冲刺键（Ctrl/toggle）状态镜像。
    fn set_key_held(&mut self, held: bool) {
        self.key_held = held;
    }
    /// 世界每 tick 推进窗口（fixed_step 域节拍：暂停冻结、死亡清理由
    /// game.rs 清 input 兜住）。
    fn step(&mut self) {
        self.window_steps = self.window_steps.saturating_sub(1);
    }
    /// 写入 `input.sprint` 的合成值 = 冲刺键按住 ∨ 双击 latch。
    fn sprint(&self) -> bool {
        self.key_held || self.latched
    }
    /// `KeyMapping.releaseAll()`（Minecraft.setScreen :1148）等价重置。
    fn release_all(&mut self) {
        *self = Self::default();
    }
}

/// 进暂停（波#127 B-1）：对齐 26.1 `Minecraft.setScreen`（:1119-1150）——
/// `KeyMapping.releaseAll()`（:1148）在开屏瞬间释放全部按压态。我们若只门
/// 分发不清账：暂停期间松开事件被屏幕栈挡掉，`input.forward/mining/placing`
/// 悬真，恢复即“没人碰键盘也往前走/挖”。清 input 同时覆盖触摸按动态
/// （game.rs 每步从 touch 镜像，恢复后以实际触摸态为准，无残留）。
/// `screen`/`sprint`/`right_use_held` 是 app 层的按压态镜像（game.rs 的
/// `input` 之外），须随 releaseAll 一同清，否则恢复后悬真。
fn enter_pause(
    runtime: &mut GameRuntime,
    screen: &mut Screen,
    window: &Option<Arc<Window>>,
    sprint: &mut SprintTapWindow,
    right_use_held: &mut bool,
) {
    runtime.input = Default::default();
    // 触摸放置键按住态同释：暂停期间触摸被菜单层接管，place_held 悬真会
    // 让恢复后的周期 use（I-21）幽灵连发。
    runtime.touch.place_held = false;
    sprint.release_all();
    *right_use_held = false;
    *screen = Screen::Paused;
    set_cursor(window, false);
}

/// 关合成界面（原版 `removed` 语义，审计 #52 数据丢失修复）：
/// - carried 先塞背包、装不下的余量 drop 到世界（dropOrPlaceInInventory，
///   ACM:588-596）；`to_inventory=false`（死亡路径）直接掉世界——fixed_step
///   死亡丢弃刚清完 36 格，塞回去 = 免死漏洞；
/// - 网格物品**逐堆掉世界**，不塞背包（CM:96-99 / IM:87-94 clearContainer）。
///
/// 旧实现全部塞背包、满栏 log 后静默丢弃 = 双重偏离 + 吞物，禁。
/// ContainerScreen 镜像随屏丢弃：真数据在 craft/hotbar，每次交互已回写。
fn close_craft(
    craft: &mut Option<mcv_logic::ui::CraftScreen>,
    craft_menu: &mut Option<mcv_ui::slot::ContainerScreen>,
    to_inventory: bool,
    rt: &mut GameRuntime,
) {
    *craft_menu = None;
    let Some(cs) = craft.take() else { return };
    let (cursor, grid) = cs.close_to_world();
    let mut spills: Vec<mcv_item::ItemStack> = Vec::new();
    if let Some(st) = cursor {
        if to_inventory {
            if let Some(rest) = rt.hotbar.add(rt.player.sel_slot, st) {
                log::debug!("背包已满，carried 余量掉世界");
                spills.push(rest);
            }
        } else {
            spills.push(st);
        }
    }
    spills.extend(grid);
    if !spills.is_empty() {
        mcv_logic::ui::drop_container_items(rt, &spills);
    }
}

/// 容器界面构造（波 #126 接线）：槽位表 = 原版容器身份序——结果 0、网格
/// 1..n²+1、主背包 27、热栏 9（盔甲 5..8 / 副手 45 整段缺 = 已登记债
/// 审计 #55，索引紧凑排布）。分组路由对照 quickMoveStack 重载：
/// 3x3 = CraftingMenu.java:110-129（结果→背包倒序、背包→网格段优先
/// 失败再另一段、网格→背包正向）；2x2 = InventoryMenu.java:110-136
/// （无网格路由；装备路由依赖物品内容分支，router 签名不可表达 =
/// 审计 #24 债）。
fn make_craft_menu(cw: usize, creative: bool) -> mcv_ui::slot::ContainerScreen {
    use mcv_ui::slot::{ContainerScreen, SlotDef};
    let n = cw * cw;
    let inv = 1 + n;
    let mut slots = vec![SlotDef::result(0.0, 0.0, 0)];
    slots.extend((0..n).map(|_| SlotDef::new(0.0, 0.0, 1)));
    slots.extend((0..27).map(|_| SlotDef::new(0.0, 0.0, 2)));
    slots.extend((0..9).map(|_| SlotDef::new(0.0, 0.0, 3)));
    let (inv_end, hot) = (inv + 36, inv + 27);
    let router = Box::new(move |g: u16| -> Vec<(usize, usize, bool)> {
        match g {
            // 结果槽 → 玩家背包，倒序（CM:111 / IM:111）
            0 => vec![(inv, inv_end, true)],
            // 网格 → 背包正向（CM:129 else 段 / IM:114-116）
            1 => vec![(inv, inv_end, false)],
            // 主/热栏 → 3x3 先试网格段（CM:118），再另一段（CM:120-126）；
            // 2x2 只有另一段（IM:127-134）
            2 => {
                if cw == 3 {
                    vec![(1, 1 + n, false), (hot, inv_end, false)]
                } else {
                    vec![(hot, inv_end, false)]
                }
            }
            _ => {
                if cw == 3 {
                    vec![(1, 1 + n, false), (inv, hot, false)]
                } else {
                    vec![(inv, hot, false)]
                }
            }
        }
    });
    let mut cs = ContainerScreen::new(
        mcv_ui::context::Rect::new(0.0, 0.0, 176.0, 166.0),
        slots,
        |kind| mcv_item::Hotbar::max_stack(kind as u16) as u32,
        router,
    );
    cs.infinite_materials = creative;
    cs
}

/// ItemStack → StackRef 镜像：damage/enchants 走侧表，meta = 表索引+1
/// （0=无令牌）；引擎契约保证整堆往返（mcv_ui slot.rs StackRef.meta）。
fn stack_to_ref(
    st: &mcv_item::ItemStack,
    table: &mut Vec<mcv_item::ItemStack>,
) -> mcv_ui::slot::StackRef {
    if st.is_empty() {
        return mcv_ui::slot::StackRef::default();
    }
    let meta = if st.damage > 0 || !st.enchants.is_empty() {
        table.push(st.clone());
        table.len() as u32
    } else {
        0
    };
    mcv_ui::slot::StackRef {
        kind: st.item as u32,
        count: st.count as u32,
        meta,
    }
}

/// StackRef → ItemStack 回写：meta>0 从侧表克隆再刷实时 count。
fn ref_to_stack(r: &mcv_ui::slot::StackRef, table: &[mcv_item::ItemStack]) -> mcv_item::ItemStack {
    if r.is_empty() {
        return mcv_item::ItemStack::empty();
    }
    let mut st = if r.meta > 0 {
        table[(r.meta - 1) as usize].clone()
    } else {
        mcv_item::ItemStack::new(r.kind as u16, 1)
    };
    st.count = r.count.min(u8::MAX as u32) as u8;
    st
}

/// 工作台方块 id：按注册名查（blocks_gen），查不到回退已知值 252。
fn crafting_table_id() -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == "crafting_table")
        .unwrap_or(252) as u16
}

/// 悬停槽 tooltip 首行（#152）：26.1 AbstractContainerScreen.extractTooltip
/// （ACS:195-204）→ Screen.getTooltipFromItem（Screen.java:237-242）首行 =
/// 物品显示名。数据源 i18n::item_name（表内 modern 官方名；legacy 名回退
/// 注册名，i18n.rs:91-105 回退链）；空堆 = None（ACS:196 hasItem 门）。
fn tooltip_title(lang: crate::i18n::Lang, st: &mcv_item::ItemStack) -> Option<String> {
    if st.is_empty() {
        return None;
    }
    let name = crate::i18n::item_name(lang, st.item);
    (!name.is_empty()).then(|| name.to_string())
}

impl AppState {
    /// MC 风格按钮：真 MC button 三态贴图九宫格 + 白色阴影文字居中；
    /// 悬停换高亮贴图并染黄（MC AbstractButton 行为）。逻辑 id 与旧版一致，
    /// 只换皮。素材红线（2026-10 任务 #53）：精灵表缺失时**不画纯色矩形
    /// 假按钮**，仅保留文字标签与命中矩形（缺贴图应当显眼，不伪装）。
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
        align_left: bool,
    ) {
        use mcv_render::text;
        let hovered =
            hover.is_some_and(|(mx, my)| mx >= x && mx <= x + bw && my >= y && my <= y + bh);
        if let Some(g) = gui {
            let sprite = if hovered { "button_hl" } else { "button" };
            q.extend(g.nine_slice(sprite, x, y, bw, bh, 3, scale, [1.0, 1.0, 1.0, 1.0]));
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
    /// 背景、原版 logo + splash、button 三态贴图；素材缺失显示失败提示
    /// （素材红线：无程序化回退）。
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
                    // 素材红线（2026-10 任务 #53）：不画程序化标题假 logo，
                    // 显示原版素材加载失败提示。
                    q.extend(text::text_quads_centered(
                        "missing textures: gui/title/minecraft.png",
                        w * 0.5,
                        h * 0.22,
                        s,
                        [1.0, 0.3, 0.3, 1.0],
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
                            // 素材红线（2026-10 任务 #53）：缺精灵不画纯色
                            // 假按钮行，仅保留文字。
                            q.extend(text::text_quads(
                                &label,
                                x + 4.0 * s,
                                y + (btn_h - 8.0 * s) * 0.5,
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
                    false,
                );
                // 键位重映射入口（追加于 arm 末尾，最小化并行分支冲突面）
                Self::mc_button(
                    &mut q,
                    &mut self.menu_hot,
                    gui,
                    hover,
                    "keybinds",
                    x,
                    y4 + btn_h + gap,
                    btn_w,
                    btn_h,
                    crate::binds_ui::ui_text(lang, crate::binds_ui::Text::Entry),
                    s,
                    false,
                );
            }
            Screen::KeyBinds => {
                // M8b 试点：按键重映射屏迁入 mcv_ui 屏幕栈——布局/命中/
                // 三态贴图/tooltip 全在框架内，此处只做接线（逻辑像素）。
                self.ui_ctx.update(w as i32, h as i32, 0);
                if let Some(top) = self.ui_stack.top_mut()
                    && let Some(kb) = top
                        .as_any()
                        .downcast_mut::<crate::binds_ui::KeyBindsScreen>()
                {
                    kb.refresh_rows(&self.keymap);
                }
                let mut g = mcv_ui::UiGraphics::new(&self.ui_ctx, (t * 1000.0) as u64);
                self.ui_stack.draw(&mut g, gui);
                q.extend(g.into_quads());
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
                    false,
                );
            }
            // 游戏内画面由 redraw 路径渲染；合成/创造界面由 craft_ui 叠加。
            Screen::InGame | Screen::Crafting | Screen::Creative => {}
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

    /// UI 按钮按下音（本波 #143）。26.1：`AbstractWidget.mouseClicked`
    /// （AbstractWidget.java:106-118）命中即 `playDownSound` →
    /// `playButtonClickSound`（:165-167）`forUI(UI_BUTTON_CLICK, 1.0F)`
    /// （SoundEvents.java:1636 "ui.button.click"）；滑杆松开同音
    /// （AbstractSliderButton.onRelease :153-155）。世界列表行同音
    /// （WorldSelectionList.java:579/:594）。
    ///
    /// 设备路由：有 runtime（暂停/死亡屏）复用 `runtime.audio`（与游戏
    /// 行为音同设备同主增益）；纯菜单屏（runtime 已退出/未进世界）用
    /// App 级 `menu_audio`——未开则惰性开一次，开失败（无声卡/无素材）
    /// 置 dead 不再重试，点击静默。
    fn play_ui_click(&mut self) {
        if let Some(rt) = self.runtime.as_mut() {
            rt.audio.play_ui(mcv_audio::events::UI_BUTTON_CLICK, 1.0);
            return;
        }
        if self.menu_audio.is_none() && !self.menu_audio_dead {
            match self.sounds_dir().map(mcv_audio::AudioManager::open) {
                Some(Ok(a)) => self.menu_audio = Some(a),
                Some(Err(e)) => {
                    self.menu_audio_dead = true;
                    log::warn!("audio unavailable (menu ui): {e}");
                }
                None => {
                    self.menu_audio_dead = true;
                    log::warn!("audio unavailable (menu ui): sounds dir not found");
                }
            }
        }
        if let Some(a) = self.menu_audio.as_mut() {
            a.play_ui(mcv_audio::events::UI_BUTTON_CLICK, 1.0);
        }
    }

    /// 当前界面语言。
    fn lang(&self) -> crate::i18n::Lang {
        if self.set_lang == 1 {
            crate::i18n::Lang::Zh
        } else {
            crate::i18n::Lang::En
        }
    }

    /// 资源根（`assets/minecraft`，布局镜像原版 jar）。M8c：解析收编到
    /// mcv_assets——`MCV_ASSETS_DIR` env →（Android）internal 解包目录 →
    /// CWD → exe 同级（桌面 bundle 分发形态）；有效性门 = 目录存在且含
    /// `textures/`。Android 解包目录由 `android_data` 传入。
    fn assets_dir(&self) -> Option<std::path::PathBuf> {
        #[cfg(target_os = "android")]
        {
            mcv_assets::AssetManager::resolve_root(self.android_data.as_deref())
        }
        #[cfg(not(target_os = "android"))]
        {
            mcv_assets::AssetManager::resolve_root(None)
        }
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

    /// options.txt：saves 根目录下，与世界文件夹同级。
    fn options_path(&self) -> std::path::PathBuf {
        self.saves_root().join("options.txt")
    }

    /// keybindings.txt：与 options.txt 同目录。
    fn keybinds_path(&self) -> std::path::PathBuf {
        self.saves_root().join("keybindings.txt")
    }

    /// 从磁盘读入设置与键位并应用；文件缺失 → 保持默认值。
    fn load_persisted(&mut self) {
        let o = crate::options::Options::load(&self.options_path());
        self.set_dist = o.render_dist;
        self.set_sens = o.sens;
        self.set_clouds = o.clouds as usize;
        self.set_lang = o.lang as usize;
        if let Ok(text) = std::fs::read_to_string(self.keybinds_path()) {
            self.keymap = mcv_game::keymap::KeyMap::from_text(&text);
        }
    }

    /// 落盘 options.txt（设置按钮改动、退出时均触发）。
    fn save_options(&self) {
        let o = crate::options::Options {
            render_dist: self.set_dist,
            sens: self.set_sens,
            clouds: (self.set_clouds % 3) as u8,
            lang: (self.set_lang % 2) as u8,
        };
        if let Err(e) = o.save(&self.options_path()) {
            log::warn!("options.txt save failed: {e}");
        }
    }

    /// 落盘 keybindings.txt（退出时或键位界面手动保存）。
    fn save_keybinds(&self) {
        let text = self.keymap.to_text();
        if let Err(e) = crate::options::write_text(&self.keybinds_path(), &text) {
            log::warn!("keybindings.txt save failed: {e}");
        }
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
        let Some(device) = device else {
            log::error!("gpu not ready");
            return;
        };
        // #80 staging 异步上传：冲刷拷贝需要 queue（与 device 同源）。
        let Some(queue) = self.runtime_queue() else {
            log::error!("gpu queue not ready");
            return;
        };
        let uploader = mcv_render::gpu::MeshUploader::new(device, queue);
        let mut runtime = GameRuntime::new(seed, uploader, dir, mode);
        // 新世界：关屏延迟 500ms（26.1 Minecraft.java:2083 doWorldLoad
        // `new LevelLoadTracker(newWorld ? 500L : 0L)`）。
        runtime.begin_load(true);
        if mode == GameMode::Creative {
            runtime.player.flying = true;
        }
        runtime.save_meta();
        self.enter_game(runtime);
    }

    fn enter_game(&mut self, mut runtime: GameRuntime) {
        runtime.render_dist = self.set_dist;
        runtime.sens = self.set_sens;
        // #191 群系染色素材：colormap PNG（原版 GrassColorReloadListener
        // 运行时加载同一张图）壳层经 mcv_assets 装载 → 解码 → 注入
        // runtime。任一缺失 = 染色禁用红线（草/树叶按灰度贴图原样，
        // 不伪造颜色）。天空底色不吃素材，仅群系表，runtime 内自带。
        if let Some(root) = self.assets_dir() {
            let am = mcv_assets::AssetManager::new(root);
            let dec = |f: &str| -> Option<Vec<u8>> {
                am.colormap(f)
                    .ok()
                    .and_then(|b| mcv_core::tint::decode_colormap(&b))
            };
            match (dec("grass.png"), dec("foliage.png")) {
                (Some(g), Some(fl)) => runtime.attach_biome_tint(g, fl),
                _ => log::error!(
                    "colormap 素材缺失（textures/colormap/{{grass,foliage}}.png）\
——生物群系染色禁用，草/树叶按灰度贴图原样显示（不伪造颜色）"
                ),
            }
        }
        // 音频后端：无声卡/无素材时保持 silent 降级，不阻塞进游戏。
        // 设备所有权链（本波 #143）：菜单期 App 级 menu_audio 若已开，
        // 直接转手给 runtime（避免双输出设备）；否则此处新开，失败置
        // dead 不再重试（菜单点击音一并静默降级）。
        if let Some(dir) = self.sounds_dir() {
            let opened = match self.menu_audio.take() {
                Some(a) => Some(a),
                None => match mcv_audio::AudioManager::open(&dir) {
                    Ok(a) => Some(a),
                    Err(e) => {
                        self.menu_audio_dead = true;
                        log::warn!("audio unavailable ({dir:?}): {e}");
                        None
                    }
                },
            };
            if let Some(a) = opened {
                runtime.set_audio(a);
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
            // E4：退出安全落盘 = 全量投递 + 阻塞排空在途 IO（delete 时
            // remove_dir_all 前必须保证写盘收尾）。
            runtime.flush_save_blocking();
            runtime.save_meta();
            // 设备所有权链（本波 #143）：把 AudioManager 收回 App 级菜单
            // 音频（主菜单/设置按钮音续用同一输出设备）；runtime 留
            // silent 空壳走到 drop。收回后再取 save_dir，避免借用交织。
            self.menu_audio = Some(std::mem::replace(
                &mut runtime.audio,
                mcv_audio::AudioManager::silent(mcv_audio::default_sounds_dir()),
            ));
            let dir = runtime.save_dir.clone();
            drop(runtime);
            if delete {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        // 退出到菜单：设置与键位一并落盘
        self.save_options();
        self.save_keybinds();
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

    /// #80 staging 上传冲刷所需的 queue（与 device 同源）。
    fn runtime_queue(&self) -> Option<wgpu::Queue> {
        self.surface.as_ref()?;
        self.cached_queue.clone()
    }

    // ---- mcv_ui 屏幕栈接线（M8b 试点：KeyBinds 屏）----

    /// 打开按键重映射屏：压 mcv_ui 屏幕栈（对照 minecraft setScreen）。
    fn open_keybinds(&mut self) {
        let (w, h) = match self.surface.as_ref() {
            Some(sp) => (sp.config.width as i32, sp.config.height as i32),
            None => (0, 0),
        };
        self.ui_ctx.update(w, h, 0);
        self.ui_stack.push(
            Box::new(crate::binds_ui::KeyBindsScreen::new(self.lang())),
            &self.ui_ctx,
        );
        self.screen = Screen::KeyBinds;
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// 关闭按键重映射屏：pop 触发 on_close（取消捕获）后回设置页。
    fn close_keybinds(&mut self) {
        self.ui_stack.pop();
        self.screen = Screen::Settings;
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// mcv_ui 控件事件 → 键位屏动作（行=捕获，底部=恢复/保存/返回）。
    fn handle_binds_events(&mut self) {
        let events = {
            let Some(top) = self.ui_stack.top_mut() else {
                return;
            };
            let Some(kb) = top
                .as_any()
                .downcast_mut::<crate::binds_ui::KeyBindsScreen>()
            else {
                return;
            };
            kb.take_events()
        };
        for ev in events {
            // 按钮按下 / 滑杆松开都播 UI 点击音（AbstractWidget.java:106-118
            // → playButtonClickSound :165-167；滑杆松开同音
            // AbstractSliderButton.onRelease :153-155）。
            match ev {
                mcv_ui::widget::UiEvent::Click { .. }
                | mcv_ui::widget::UiEvent::SliderRelease { .. } => self.play_ui_click(),
                _ => {}
            }
            let mcv_ui::widget::UiEvent::Click { id } = ev else {
                continue;
            };
            if id < mcv_game::keymap::Action::ALL.len() {
                if let Some(top) = self.ui_stack.top_mut()
                    && let Some(kb) = top
                        .as_any()
                        .downcast_mut::<crate::binds_ui::KeyBindsScreen>()
                {
                    kb.begin_row(id, &mut self.keymap);
                }
            } else if id == crate::binds_ui::ID_RESET {
                if let Some(top) = self.ui_stack.top_mut()
                    && let Some(kb) = top
                        .as_any()
                        .downcast_mut::<crate::binds_ui::KeyBindsScreen>()
                {
                    kb.restore_defaults(&mut self.keymap);
                }
            } else if id == crate::binds_ui::ID_SAVE {
                self.save_keybinds();
            } else if id == crate::binds_ui::ID_BACK {
                self.close_keybinds();
            }
        }
    }

    /// 菜单指针点击（桌面鼠标/触屏统一入口）。
    fn handle_menu_pointer(&mut self, x: f64, y: f64) {
        if self.screen == Screen::KeyBinds {
            // mcv_ui 屏幕栈路由：物理 → 逻辑像素；栈非空即消费（关闭时
            // 输入穿透规则：屏幕打开期间游戏输入不派发）
            let (lx, ly) = self.ui_ctx.to_logical(x as f32, y as f32);
            self.ui_stack.mouse_down(lx, ly, 0);
            self.handle_binds_events();
            return;
        }
        if self.screen == Screen::Worlds {
            let (px, py) = (x as f32, y as f32);
            let hit = self.world_hot.iter().find(|(_, bx, by, bw, bh)| {
                px >= *bx && px <= bx + bw && py >= *by && py <= by + bh
            });
            if let Some(&(idx, ..)) = hit {
                // 世界行按下音（WorldSelectionList.java:579：forUI
                // (UI_BUTTON_CLICK, 1.0F)，先音后交互同 mouseClicked 序）。
                // 先按值拷出 idx 再播（hit 借 world_hot，播音需 &mut self）。
                self.play_ui_click();
                self.on_world_pick(idx);
                return;
            }
        }
        if let Some(id) = self.hit_menu(x, y) {
            // 按下沿音（AbstractWidget.mouseClicked :106-118：命中即
            // playDownSound，先音后 onClick；:165-167 音量 1.0F）。
            self.play_ui_click();
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
            (Screen::Settings, "keybinds") => self.open_keybinds(),
            // KeyBinds 屏的点击已改走 mcv_ui 屏幕栈（handle_binds_events）
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
        // 设置按钮改动 → 立即落盘 options.txt
        if self.screen == Screen::Settings
            && matches!(
                id,
                "dist+" | "dist-" | "sens+" | "sens-" | "clouds" | "lang"
            )
        {
            self.save_options();
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// 合成/创造帧：构建面板 quad 并刷新命中表 `craft_hot`。
    /// 必须在 runtime 可变借用之前调用。
    fn craft_ui(&mut self, w: f32, h: f32) -> Vec<mcv_render::HudQuad> {
        let gui = self.renderer.as_ref().and_then(|r| r.gui());
        let hover = self.last_cursor.map(|(x, y)| (x as f32, y as f32));
        let Some(rt) = self.runtime.as_ref() else {
            self.craft_hot.clear();
            return Vec::new();
        };
        let (q, rects) = match self.screen {
            Screen::Crafting => match &self.craft {
                Some(c) => {
                    let (mut q, hot) =
                        crate::craft_ui::craft_quads(c, &rt.hotbar, gui, w, h, hover);
                    // #152：悬停物品名 tooltip（ACS:195-204 extractTooltip）。
                    q.extend(self.craft_tooltip_quads());
                    (q, hot)
                }
                None => (Vec::new(), Vec::new()),
            },
            Screen::Creative => match &self.creative {
                Some(cp) => crate::craft_ui::creative_quads(cp, &rt.hotbar, gui, w, h),
                None => (Vec::new(), Vec::new()),
            },
            _ => (Vec::new(), Vec::new()),
        };
        self.craft_hot = rects;
        q
    }

    /// 创造取物页点击路由（波 #126 后仅 Screen::Creative 使用；合成界面的
    /// 槽位交互已整体切到 ContainerScreen）：id 前缀路由（c + 序号、
    /// close/prev/next）。创造页无光标持物，点击直接发物。
    fn handle_craft_pointer(&mut self, x: f64, y: f64, left: bool) {
        let (px, py) = (x as f32, y as f32);
        let Some(hit) = self
            .craft_hot
            .iter()
            .find(|r| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h)
            .map(|r| r.id)
        else {
            return;
        };
        match hit {
            "close" => self.close_craft_ui(),
            "prev" | "next" => {
                if let Some(cp) = self.creative.as_mut() {
                    let step = if hit == "next" { 1 } else { -1 };
                    let p = cp.page as isize + step;
                    cp.set_page(p.max(0) as usize, crate::craft_ui::ALL_ITEMS.len());
                }
            }
            id => {
                // 注意 "close" 也带 c 前缀，但已在上面分支消费；"lose" 非数字。
                if let Some(i) = id.strip_prefix('c').and_then(|s| s.parse::<usize>().ok()) {
                    let give = self
                        .creative
                        .as_ref()
                        .and_then(|cp| cp.click(i, left, &crate::craft_ui::ALL_ITEMS));
                    if let Some((item, n)) = give
                        && let Some(rt) = self.runtime.as_mut()
                    {
                        let st = mcv_item::ItemStack::new(item, n);
                        if let Some(rest) = rt.hotbar.add(rt.player.sel_slot, st) {
                            mcv_logic::ui::drop_container_items(rt, std::slice::from_ref(&rest));
                        }
                    }
                }
            }
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// 关屏按钮命中（引擎自有控件，桌面/触屏共用；craft_hot 物理像素域）。
    fn craft_close_hit(&self, x: f64, y: f64) -> bool {
        let (x, y) = (x as f32, y as f32);
        self.craft_hot
            .iter()
            .any(|r| r.id == "close" && x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h)
    }

    /// 关合成/创造界面：原版归还语义（close_craft）+ 光标释放回游戏。
    fn close_craft_ui(&mut self) {
        if let Some(rt) = self.runtime.as_mut() {
            close_craft(&mut self.craft, &mut self.craft_menu, true, rt);
        } else {
            self.craft = None;
            self.craft_menu = None;
        }
        self.creative = None;
        self.craft_held = 0;
        self.craft_last_click = None;
        self.screen = Screen::InGame;
        set_cursor(&self.window, true);
    }

    /// 容器坐标换算：(gui scale, 面板逻辑像素原点)。面板 = 176x166 居中
    /// （AbstractContainerScreen.java:89-90 leftPos/topPos），与 craft_quads
    /// 物理像素布局同基准（同 ÷gui_scale 即命中一致）。
    fn craft_geom(&self) -> Option<(f32, f32, f32)> {
        let sp = self.surface.as_ref()?;
        let s = gui_scale(sp.config.height as f32);
        let lw = sp.config.width as f32 / s;
        let lh = sp.config.height as f32 / s;
        Some((s, (lw - 176.0) * 0.5, (lh - 166.0) * 0.5))
    }

    /// 容器交互执行器（双轨合一：doClick 是唯一裁决）——
    /// ① 槽坐标按当前窗口重写（resize 跟随 craft_quads 布局）；
    /// ② 灌镜像：网格 + 结果槽（实时按配方重算 = vanilla ResultContainer/
    ///    slotChangedCraftingGrid）+ 36 背包 + 光标，耐久/附魔经 meta 令牌；
    /// ③ 执行事件（mouse_down/up/drag、key_throw 皆经 doClick）；
    /// ④ 结果槽计数减少 = 消耗一次配方（ResultSlot.onTake 的逐格 −1，
    ///    Q 丢/shift 带走的也一并消耗，堵结果槽白取重算漏洞）；
    /// ⑤ 回写 craft.grid/cursor/hotbar，take_drops → 世界掉落实体。
    fn craft_flush_menu(&mut self, ev: impl FnOnce(&mut mcv_ui::slot::ContainerScreen)) {
        if self.craft.is_none() || self.craft_menu.is_none() {
            return;
        }
        let cw = self.craft.as_ref().map_or(2, |c| c.width);
        let (_, px, py) = self.craft_geom().unwrap_or((1.0, 0.0, 0.0));
        // 面板内槽位原点：与 craft_quads 的 craft_panel/inv_panel 常量同源。
        let (gx, gy, rx, ry) = if cw == 3 {
            (30.0f32, 17.0f32, 124.0f32, 35.0f32)
        } else {
            (98.0, 18.0, 154.0, 28.0)
        };
        let mut extra: Vec<mcv_item::ItemStack> = Vec::new();
        let mut items: Vec<mcv_ui::slot::StackRef> = Vec::new();
        let carried;
        {
            let craft = self.craft.as_ref().unwrap();
            items.push(
                craft
                    .result()
                    .map(|r| stack_to_ref(&r, &mut extra))
                    .unwrap_or_default(),
            );
            for g in &craft.grid {
                items.push(stack_to_ref(g, &mut extra));
            }
            if let Some(rt) = self.runtime.as_ref() {
                // 灌入序 = 槽位表序（主背包 27 → 热栏 9），回写同序。
                for st in rt.hotbar.main.iter().chain(rt.hotbar.slots.iter()) {
                    items.push(stack_to_ref(st, &mut extra));
                }
            }
            carried = stack_to_ref(&craft.cursor, &mut extra);
        }
        let (consumed, drop_stacks);
        {
            let cs = self.craft_menu.as_mut().unwrap();
            cs.panel = mcv_ui::context::Rect::new(px, py, 176.0, 166.0);
            cs.slots[0].x = px + rx;
            cs.slots[0].y = py + ry;
            for i in 0..cw * cw {
                cs.slots[1 + i].x = px + gx + (i % cw) as f32 * 18.0;
                cs.slots[1 + i].y = py + gy + (i / cw) as f32 * 18.0;
            }
            let base = 1 + cw * cw;
            for j in 0..27 {
                cs.slots[base + j].x = px + 8.0 + (j % 9) as f32 * 18.0;
                cs.slots[base + j].y = py + 84.0 + (j / 9) as f32 * 18.0;
            }
            for j in 0..9 {
                cs.slots[base + 27 + j].x = px + 8.0 + j as f32 * 18.0;
                cs.slots[base + 27 + j].y = py + 142.0;
            }
            cs.set_items(items);
            cs.carried = carried;
            let before = cs.items[0].count;
            ev(cs);
            consumed = cs.items[0].count < before;
            // 单机镜像自洽：MenuCmd 只是 screen→server 记账，不重放。
            cs.take_cmds();
            drop_stacks = cs
                .take_drops()
                .iter()
                .map(|r| ref_to_stack(r, &extra))
                .collect::<Vec<_>>();
            let cs = self.craft_menu.as_ref().unwrap();
            let craft = self.craft.as_mut().unwrap();
            for i in 0..cw * cw {
                craft.grid[i] = ref_to_stack(&cs.items[1 + i], &extra);
            }
            craft.cursor = ref_to_stack(&cs.carried, &extra);
        }
        if let Some(rt) = self.runtime.as_mut() {
            let base = 1 + cw * cw;
            let cs = self.craft_menu.as_ref().unwrap();
            for j in 0..27 {
                rt.hotbar.main[j] = ref_to_stack(&cs.items[base + j], &extra);
            }
            for j in 0..9 {
                rt.hotbar.slots[j] = ref_to_stack(&cs.items[base + 27 + j], &extra);
            }
            if consumed {
                let craft = self.craft.as_mut().unwrap();
                mcv_item::crafting::consume_grid(&mut craft.grid);
            }
            if !drop_stacks.is_empty() {
                mcv_logic::ui::drop_container_items(rt, &drop_stacks);
            }
        }
    }

    /// 容器界面按下（物理像素入）：双击自计时（ACS:319 lastClickSlot +
    /// GUI 双击窗 250ms；winit 无 doubleClick 标志）后喂 mouse_down。
    fn craft_menu_down(&mut self, x: f64, y: f64, button: u8) {
        let Some((s, _, _)) = self.craft_geom() else {
            return;
        };
        let (lx, ly) = (x as f32 / s, y as f32 / s);
        let slot = self
            .craft_menu
            .as_ref()
            .and_then(|cs| cs.hovered_at(lx, ly))
            .map_or(-1, |i| i as i32);
        let now = std::time::Instant::now();
        let dc = self.craft_last_click.is_some_and(|(ls, lb, t)| {
            ls == slot && lb == button && now.duration_since(t).as_millis() < 250
        });
        self.craft_last_click = Some((slot, button, now));
        let shift = self.shift_held;
        self.craft_flush_menu(|cs| cs.mouse_down(lx, ly, button, shift, dc));
        if button <= 1 {
            self.craft_held |= 1 << button;
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// 容器界面松开：三段拖拽收尾 / 双击收拢 / 面板外丢弃皆在
    /// ContainerScreen::mouse_up 内（ACS:444-539）。
    fn craft_menu_up(&mut self, x: f64, y: f64, button: u8) {
        if button <= 1 {
            self.craft_held &= !(1 << button);
        }
        let Some((s, _, _)) = self.craft_geom() else {
            return;
        };
        let (lx, ly) = (x as f32 / s, y as f32 / s);
        let shift = self.shift_held;
        self.craft_flush_menu(|cs| cs.mouse_up(lx, ly, button, shift));
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// 容器拖拽收集（按住键移动 = ACS:408-441 quickCraftSlots 收集段）。
    fn craft_menu_drag(&mut self, x: f64, y: f64) {
        let Some((s, _, _)) = self.craft_geom() else {
            return;
        };
        let (lx, ly) = (x as f32 / s, y as f32 / s);
        self.craft_flush_menu(|cs| cs.mouse_drag(lx, ly, 0));
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Q/Ctrl+Q 丢弃悬停槽（Minecraft.java keyDrop/keyDropStack → THROW
    /// button0/1；门槛 carried 空在 ContainerScreen 内）。keymap Action 无
    /// Drop = 审计 #54 债，暂硬编码；非容器屏的世界丢弃同债未接。
    fn craft_key_throw(&mut self, whole_stack: bool) {
        let Some((s, _, _)) = self.craft_geom() else {
            return;
        };
        let Some((cx, cy)) = self.last_cursor else {
            return;
        };
        let Some(h) = self
            .craft_menu
            .as_ref()
            .and_then(|cs| cs.hovered_at(cx as f32 / s, cy as f32 / s))
        else {
            return;
        };
        self.craft_flush_menu(|cs| cs.key_throw(h, whole_stack));
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// 悬停槽物品名 tooltip quads（#152）：光标悬停有物槽 + 光标空手时画
    /// （26.1 AbstractContainerScreen.extractTooltip，ACS:195-204：
    /// hoveredSlot.hasItem() + carried 空门；carried 非空的 image-tooltip
    /// 例外不建模 = 像素级债）。文案 = Screen.getTooltipFromItem（Screen.
    /// java:237-242）首行 → i18n::item_name（modern 官方名 / legacy 注册
    /// 名）。锚点 = 鼠标右下（setTooltipForNextFrame 锚定，偏移像素级债）。
    /// 槽序同 craft_flush_menu 灌入序：0 = 结果槽、1..n²+1 = 网格、其后
    /// 27 主背包 + 9 快捷栏。
    fn craft_tooltip_quads(&self) -> Vec<mcv_render::HudQuad> {
        let Some((mx, my)) = self.last_cursor else {
            return Vec::new();
        };
        let Some((s, _, _)) = self.craft_geom() else {
            return Vec::new();
        };
        let Some(craft) = self.craft.as_ref() else {
            return Vec::new();
        };
        if !craft.cursor.is_empty() {
            return Vec::new();
        }
        let Some(idx) = self
            .craft_menu
            .as_ref()
            .and_then(|cs| cs.hovered_at(mx as f32 / s, my as f32 / s))
        else {
            return Vec::new();
        };
        let Some(rt) = self.runtime.as_ref() else {
            return Vec::new();
        };
        let base = 1 + craft.width * craft.width;
        let st = if idx == 0 {
            craft.result()
        } else if idx < base {
            craft.grid.get(idx - 1).cloned()
        } else if idx < base + 27 {
            rt.hotbar.main.get(idx - base).cloned()
        } else {
            rt.hotbar.slots.get(idx - base - 27).cloned()
        };
        let Some(st) = st.filter(|st| !st.is_empty()) else {
            return Vec::new();
        };
        let Some(title) = tooltip_title(self.lang(), &st) else {
            return Vec::new();
        };
        mcv_render::text::tooltip_quads(&title, mx as f32 + 12.0 * s, my as f32 + 2.0 * s, s)
    }

    /// 数字键 1-9 直换执行器（#152）：门槛 carried 空 + 有悬停槽（ACS:
    /// 608-611 checkHotbarKeyPressed）。槽路由（序同 craft_flush_menu）：
    /// 0 = 结果槽（只允许空热栏槽整份取件，ResultSlot mayPlace=false 门，
    /// 取到消耗一次网格）；1..n²+1 = 网格槽（ACM:471-507 SWAP 全语义）；
    /// 其后 36 = 背包槽（主背包 27 → 热栏 9）。mcv_ui 未建模 SWAP（其
    /// 差异表债），本仓走 mcv_logic 状态机直换，下次 craft_flush_menu
    /// 灌镜像自愈；结果槽直换不触发 consumed 记账（未走 doClick），网格
    /// 消耗已就地执行，语义自洽。
    fn craft_key_swap(&mut self, k: usize) {
        let Some((s, _, _)) = self.craft_geom() else {
            return;
        };
        let Some((cx, cy)) = self.last_cursor else {
            return;
        };
        let Some(h) = self
            .craft_menu
            .as_ref()
            .and_then(|cs| cs.hovered_at(cx as f32 / s, cy as f32 / s))
        else {
            return;
        };
        if self.craft.as_ref().is_some_and(|c| !c.cursor.is_empty()) {
            return;
        }
        let cw = self.craft.as_ref().map_or(2, |c| c.width);
        let base = 1 + cw * cw;
        let Some(rt) = self.runtime.as_mut() else {
            return;
        };
        if h == 0 {
            let craft = self.craft.as_mut().unwrap();
            if rt.hotbar.slots[k].is_empty()
                && let Some(res) = craft.result()
            {
                mcv_item::crafting::consume_grid(&mut craft.grid);
                rt.hotbar.slots[k] = res;
            }
        } else if h < base {
            let i = h - 1;
            let craft = self.craft.as_mut().unwrap();
            if let Some(rest) =
                mcv_logic::ui::swap_with_hotbar(&mut craft.grid[i], &mut rt.hotbar, k)
            {
                // 劈堆换余量（ACM:498-500 inventory.add 失败 → drop）。
                mcv_logic::ui::drop_container_items(rt, std::slice::from_ref(&rest));
            }
        } else {
            let j = h - base;
            let gi = if j < 27 {
                mcv_item::HOTBAR_SLOTS + j
            } else {
                j - 27
            };
            if let Some(rest) = mcv_logic::ui::swap_inv_with_hotbar(&mut rt.hotbar, gi, k) {
                mcv_logic::ui::drop_container_items(rt, std::slice::from_ref(&rest));
            }
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
        let Some(device) = device else {
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
        // #80 staging 异步上传：冲刷拷贝需要 queue（与 device 同源）。
        let Some(queue) = self.runtime_queue() else {
            return;
        };
        let uploader = mcv_render::gpu::MeshUploader::new(device, queue);
        let mut runtime = GameRuntime::new(seed, uploader, dir, mode);
        // 载入存档：closeDelay = 0（26.1 `new LevelLoadTracker()` 默认值，
        // 与新世界的 500ms 区分，Minecraft.java:2083）。
        runtime.begin_load(false);
        self.enter_game(runtime);
    }
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // C-2（波#127-3）：回前台解除后台静音（与 suspended 的主增益归零配对）。
        if let Some(g) = self.gain_before_suspend.take()
            && let Some(rt) = self.runtime.as_ref()
        {
            rt.audio.set_master_gain(g);
        }
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
            Ok((surface, config, depth, device, queue, max_extent, hdr_ok)) => {
                // 资源根（M8c）：mcv_assets 统一解析（env → Android 解包 →
                // CWD → exe 同级，含 textures/ 才生效）。缺根 = 硬错误显式 log。
                let pack = self.assets_dir();
                self.cached_device = Some(device.clone());
                self.cached_queue = Some(queue.clone());
                // HDR 链（#223 续；feature 默认关）：交换链/sRGB 编码仍由
                // surface 格式承担，仅渲染器管线族 + draw 目标换 Rgba16Float
                // 主纹理，ACES tone map 收尾到交换链。MCV_HDR=0 运行期强关。
                #[cfg(feature = "hdr")]
                let hdr_active = hdr_ok && !std::env::var("MCV_HDR").is_ok_and(|v| v == "0");
                // 非 hdr 构建：判定位不进参与（默认路编译面 zero-read）。
                #[cfg(not(feature = "hdr"))]
                let _hdr_active = {
                    let _ = hdr_ok;
                    false
                };
                // 颜色目标格式：hdr 特性外恒 = surface 格式（默认路字节级
                // 不变）；特性内按 hdr_active 判定（Rgba16Float 主纹理）。
                // 常量本身 cfg-gated（mcv_render::HDR_COLOR_FORMAT 只在
                // feature = "hdr" 编译面内存在）——引用必须同 gate。
                let color_format = {
                    #[cfg(feature = "hdr")]
                    let fmt = if hdr_active {
                        log::info!("hdr: RGBA16F 主纹理 + ACES tone mapping 启用");
                        mcv_render::HDR_COLOR_FORMAT
                    } else {
                        config.format
                    };
                    #[cfg(not(feature = "hdr"))]
                    let fmt = config.format;
                    fmt
                };
                self.renderer = Some(mcv_render::Renderer::new(
                    device.clone(),
                    queue.clone(),
                    color_format,
                    pack.as_deref(),
                ));
                self.clouds = Some(mcv_render::Clouds::new(&device, &queue, color_format));
                #[cfg(feature = "hdr")]
                let hdr_stage = hdr_active.then(|| {
                    mcv_render::hdr::HdrStage::new(
                        &device,
                        config.format,
                        config.width,
                        config.height,
                    )
                });
                // 玩家皮肤：原版 entity/player/{wide/steve,slim/alex}.png
                // （M8c 走 AssetManager；缺皮肤/缺音效索引 = 显式 log，
                // 不再静默跳过。纹理/字体/GUI 等缺失由 Renderer 内部
                // AssetManager 构造尾部一次性汇总上报。）
                match &pack {
                    Some(dir) => {
                        let assets = mcv_assets::AssetManager::new(dir.clone());
                        let steve = assets.read_optional("textures/entity/player/wide/steve.png");
                        let alex = assets.read_optional("textures/entity/player/slim/alex.png");
                        match (steve, alex) {
                            (Some(s), Some(a)) => {
                                if let Err(e) = self.renderer.as_mut().unwrap().load_skins(&s, &a) {
                                    log::warn!("skin load failed: {e}");
                                }
                            }
                            _ => log::error!(
                                "玩家皮肤缺失（{dir:?}/textures/entity/player/\
                                 {{wide/steve,slim/alex}}.png）——玩家皮肤不上屏"
                            ),
                        }
                        // 音效索引状态一次性上报（缺 sounds.json → 播放侧
                        // no-op 降级，但错误必须可见；开发树拉取见 fetch-sounds.sh）。
                        if !assets
                            .sounds_dir()
                            .is_some_and(|d| d.join(mcv_assets::SOUNDS_INDEX).is_file())
                        {
                            log::error!(
                                "原版音效缺失（sounds 根或 sounds.json 不在）——\
                                 音效静默降级（不影响画面）"
                            );
                        }
                    }
                    None => log::error!(
                        "资源根未解析（assets/minecraft 未找到，MCV_ASSETS_DIR 未设）\
                         ——贴图/字体/GUI 全部按素材红线降级"
                    ),
                }
                // 设置与键位：从 options.txt / keybindings.txt 读入（缺失 → 默认）
                self.load_persisted();
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
                    #[cfg(feature = "hdr")]
                    hdr: hdr_stage,
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
        // C-3（波#127-2）：Android 进程随时可被杀（onPause 后无删除保证），
        // 自动保存的 30s 墙钟节拍（redraw 内）挂起后不再推进——进后台首件事
        // 同步落盘（CloseRequested 同款通路），被杀丢档上限 30s → 0。
        if let Some(runtime) = self.runtime.as_mut() {
            // E4：进后台首件事 = 投递 + 阻塞排空（原 save_dirty 同点，语义
            // 升级为等在途写盘收尾——被杀丢档上限 30s → 0 不回退）。
            runtime.flush_save_blocking();
            runtime.save_meta();
        }
        // C-2（波#127-3）：后台音频挂起——cpal 回调线程独立于事件循环，
        // 不处理则回后台继续出声（发热/耗电+基线「后台=暂停」违和）。
        // mcv_audio 无流级 suspend API（登记债），主增益归零 = 全静音近似；
        // resumed 恢复挂起前增益。
        if let Some(rt) = self.runtime.as_ref()
            && self.gain_before_suspend.is_none()
        {
            self.gain_before_suspend = Some(rt.audio.master_gain());
            rt.audio.set_master_gain(0.0);
        }
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
                    // E4：关窗 = 全量投递 + 阻塞排空在途 IO 后再落 meta。
                    runtime.flush_save_blocking();
                    runtime.save_meta();
                }
                self.save_options();
                self.save_keybinds();
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
            WindowEvent::Focused(focused) => {
                // B-4（波#127-3）：只记失焦时刻，判定在 redraw 每帧做
                // （原版 pauseIfInactive 是每帧检查：失焦 && 距最后活动
                // >500ms → pauseGame，Minecraft.java:1388-1397）。回焦清账
                // ——暂停屏不随回焦自动关闭（原版同样停在 PauseScreen）。
                if focused {
                    self.focus_lost_at = None;
                } else if self.focus_lost_at.is_none() {
                    self.focus_lost_at = Some(std::time::Instant::now());
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
                // 键位界面：键盘只服务重映射捕获（等待按键模态），不回灌
                // 游戏/暂停逻辑；未捕获时 Esc 关屏（走屏幕栈 on_close）
                if self.screen == Screen::KeyBinds {
                    if state == ElementState::Pressed
                        && let Some(vk) = mcv_platform::keybind::vkey_of(code)
                    {
                        let esc = vk == mcv_game::keymap::VKey::Escape;
                        let mut close = false;
                        if let Some(top) = self.ui_stack.top_mut()
                            && let Some(kb) = top
                                .as_any()
                                .downcast_mut::<crate::binds_ui::KeyBindsScreen>()
                        {
                            let consumed = kb.key_input(Some(vk), &mut self.keymap);
                            if esc && !consumed {
                                close = true;
                            }
                        }
                        if close {
                            self.close_keybinds();
                        }
                        if let Some(w) = self.window.as_ref() {
                            w.request_redraw();
                        }
                    }
                    return;
                }
                // Q/Ctrl+Q = 容器界面丢弃悬停槽 1 件/整堆（Minecraft.java
                // keyDrop/keyDropStack → THROW button0/1，门槛 carried 空在
                // ContainerScreen 内；非容器屏的世界丢弃 = 审计 #54 债）。
                // 先于 runtime 借用处理：craft_key_throw 需要 &mut self。
                if state == ElementState::Pressed
                    && code == KeyCode::KeyQ
                    && self.screen == Screen::Crafting
                {
                    let whole = self.ctrl_held;
                    self.craft_key_throw(whole);
                    return;
                }
                // 数字键 1-9 = 悬停槽 ↔ 快捷栏直换（#152；ACS:592-627
                // keyPressed → checkHotbarKeyPressed → slotClicked(hovered,
                // i, SWAP)，门槛 carried 空 + 有悬停槽在执行器内）。mcv_ui
                // 未建模 SWAP（其差异表债），走 mcv_logic 状态机直换，
                // 下次 craft_flush_menu 灌镜像自愈。
                if state == ElementState::Pressed
                    && self.screen == Screen::Crafting
                    && let Some(k) = mcv_platform::keybind::hotbar_slot(code)
                {
                    self.craft_key_swap(k);
                    return;
                }
                let Some(runtime) = self.runtime.as_mut() else {
                    return;
                };
                let pressed = state == ElementState::Pressed;
                use mcv_game::keymap::Action;
                // 加载态（26.1 LevelLoadingScreen 活动）：快捷栏/背包/切视角/
                // 移动/暂停全部不响应——原版 Screen 非 null 时游戏键不派发，
                // 且 LevelLoadingScreen.shouldCloseOnEsc=false（:64-67）。
                let loading = runtime.phase == mcv_logic::game::GamePhase::Loading;
                // 波#127 B-1/D-5：屏幕栈门——原版 KeyMapping 只在 Screen==null
                // 时驱动玩法（暂停菜单里 WASD/数字键不写 input、不切槽）。
                // `playing` = 游玩屏幕 + 非加载态。
                let playing = !loading && gameplay_input_allowed(self.screen);
                // 快捷栏：MC 是 9 个独立键位，本引擎键位表单动作单键，暂直绑
                if pressed
                    && playing
                    && let Some(slot) = mcv_platform::keybind::hotbar_slot(code)
                {
                    runtime.player.sel_slot = slot;
                }
                // F5 切视角不在 MC 键位表内(GT引擎扩展),保持硬编码
                if pressed && playing && code == KeyCode::F5 {
                    runtime.cycle_camera();
                }
                // E 开/关背包(MC E=inventory)：生存/极限 = 随身 2x2 合成页，
                // 创造 = 取物页。
                if pressed && !loading && code == KeyCode::KeyE {
                    match self.screen {
                        Screen::InGame => {
                            if runtime.mode == GameMode::Creative {
                                self.screen = Screen::Creative;
                                self.creative = Some(mcv_logic::ui::CreativePicker::default());
                            } else {
                                self.screen = Screen::Crafting;
                                self.craft = Some(mcv_logic::ui::CraftScreen::new(2));
                                self.craft_menu = Some(make_craft_menu(2, false));
                                self.craft_held = 0;
                                self.craft_last_click = None;
                            }
                            set_cursor(&self.window, false);
                        }
                        Screen::Crafting | Screen::Creative => {
                            close_craft(&mut self.craft, &mut self.craft_menu, true, runtime);
                            self.creative = None;
                            self.craft_held = 0;
                            self.craft_last_click = None;
                            self.screen = Screen::InGame;
                            set_cursor(&self.window, true);
                        }
                        _ => {}
                    }
                }
                if let Some(vk) = mcv_platform::keybind::vkey_of(code) {
                    match self.keymap.action_for(vk) {
                        // 波#127 B-1：玩法动作只认 `playing`（暂停/死亡界面里
                        // 键盘不得再写 runtime.input——原版 KeyMapping 门）。
                        Some(Action::Forward) if playing => {
                            runtime.input.forward = pressed;
                            // D-2（波#127-6）：前进键两侧驱动 7 tick 双击窗。
                            self.sprint_tap.on_forward(pressed);
                            runtime.input.sprint = self.sprint_tap.sprint();
                        }
                        Some(Action::Back) if playing => {
                            runtime.input.back = pressed;
                            if pressed {
                                self.sprint_tap.on_sneak_or_back();
                                runtime.input.sprint = self.sprint_tap.sprint();
                            }
                        }
                        Some(Action::Left) if playing => runtime.input.left = pressed,
                        Some(Action::Right) if playing => runtime.input.right = pressed,
                        Some(Action::Jump) if playing => runtime.input.jump = pressed,
                        Some(Action::Sneak) if playing => {
                            runtime.input.sneak = pressed;
                            // 潜行按下沿清窗清 latch（LocalPlayer:798-800）。
                            if pressed {
                                self.sprint_tap.on_sneak_or_back();
                                runtime.input.sprint = self.sprint_tap.sprint();
                            }
                        }
                        Some(Action::Sprint) if playing => {
                            self.sprint_tap.set_key_held(pressed);
                            runtime.input.sprint = self.sprint_tap.sprint();
                        }
                        Some(Action::FlyToggle) if pressed && playing => {
                            runtime.player.flying = !runtime.player.flying;
                        }
                        Some(Action::Pause) if pressed && !loading => match self.screen {
                            Screen::InGame => {
                                // 进暂停：releaseAll 语义清按压态（26.1 setScreen
                                // :1148），否则暂停期间松开事件被门挡掉、恢复后
                                // 输入悬真。
                                enter_pause(
                                    runtime,
                                    &mut self.screen,
                                    &self.window,
                                    &mut self.sprint_tap,
                                    &mut self.right_use_held,
                                );
                            }
                            Screen::Paused => {
                                self.screen = Screen::InGame;
                                set_cursor(&self.window, true);
                            }
                            // 关合成/创造界面：carried 先背包后掉、网格掉世界
                            // （close_craft，原版 removed 语义）后回游戏。
                            Screen::Crafting | Screen::Creative => {
                                close_craft(&mut self.craft, &mut self.craft_menu, true, runtime);
                                self.creative = None;
                                self.craft_held = 0;
                                self.craft_last_click = None;
                                self.screen = Screen::InGame;
                                set_cursor(&self.window, true);
                            }
                            Screen::Worlds | Screen::Create | Screen::Settings => {
                                self.screen = Screen::Main
                            }
                            Screen::Death => {} // MC：死亡界面 Esc 无效
                            Screen::Main => event_loop.exit(),
                            // 键位界面的按键在入口已被拦截，此为穷尽性兜底
                            // （走 close_keybinds 保证栈同步弹出）
                            Screen::KeyBinds => self.close_keybinds(),
                        },
                        // Inventory(F)/Debug(F3)/Screenshot(F2)/PickBlock：
                        // 引擎侧功能属波4（背包/F3/截图），先接分发留位
                        _ => {}
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.screen == Screen::KeyBinds {
                    // mcv_ui 悬停路由（物理 → 逻辑像素；tooltip 延迟基准）
                    let (lx, ly) = self.ui_ctx.to_logical(position.x as f32, position.y as f32);
                    self.ui_stack.mouse_move(lx, ly);
                } else if self.screen == Screen::Crafting && self.craft_held != 0 {
                    // 容器按住拖动 = quickCraftSlots 收集（ACS:408-441）。
                    self.craft_menu_drag(position.x, position.y);
                } else if self.screen == Screen::InGame
                    && let (Some(runtime), Some(last)) = (self.runtime.as_mut(), self.last_cursor)
                {
                    runtime.look(position.x - last.0, position.y - last.1);
                }
                self.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::ModifiersChanged(m) => {
                // 容器交互修饰键：shift=QUICK_MOVE/双击副序、ctrl=Q 整堆丢
                // （ACS event.hasShiftDown / Minecraft.java keyDropStack）。
                self.shift_held = m.state().shift_key();
                self.ctrl_held = m.state().control_key();
            }
            WindowEvent::Touch { .. } => {
                if matches!(self.screen, Screen::Crafting | Screen::Creative) {
                    // 合成/创造界面：触摸按下 = 左键单击（按下+松手一发；
                    // 触屏拖拽分发未接 = 与触屏 quickdrop 同列遗留债）。
                    if let WindowEvent::Touch(t) = &event
                        && t.phase == winit::event::TouchPhase::Started
                    {
                        let (tx, ty) = (t.location.x, t.location.y);
                        if self.craft_close_hit(tx, ty) {
                            self.close_craft_ui();
                        } else if self.screen == Screen::Crafting {
                            self.craft_menu_down(tx, ty, 0);
                            self.craft_menu_up(tx, ty, 0);
                        } else {
                            self.handle_craft_pointer(tx, ty, true);
                        }
                    }
                    return;
                }
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
                // I-21：右键松开沿无条件清按住态——开屏（工作台）后的松开
                // 走界面分支提前返回，不清会让按住态悬真、关屏后幽灵连放。
                if button == MouseButton::Right && state == ElementState::Released {
                    self.right_use_held = false;
                }

                // 合成界面：ContainerScreen 完整鼠标路径（按下/松开/拖动
                // 三段分发、shift 快移、双击收拢、面板外丢弃全在 doClick；
                // ACS:312-539 对应）。创造取物页：无光标持物，保持旧式
                // 按下即发。关屏按钮（引擎自有控件）命中表优先。
                if matches!(self.screen, Screen::Crafting | Screen::Creative) {
                    let cur = self.last_cursor;
                    let on_close = cur.is_some_and(|(cx, cy)| self.craft_close_hit(cx, cy));
                    match state {
                        ElementState::Pressed => {
                            if on_close {
                                self.close_craft_ui();
                            } else if let Some((cx, cy)) = cur {
                                let b = match button {
                                    MouseButton::Left => 0,
                                    MouseButton::Right => 1,
                                    _ => 2,
                                };
                                if self.screen == Screen::Crafting {
                                    self.craft_menu_down(cx, cy, b);
                                } else {
                                    self.handle_craft_pointer(cx, cy, button == MouseButton::Left);
                                }
                            }
                        }
                        ElementState::Released => {
                            if self.screen == Screen::Crafting
                                && !on_close
                                && let Some((cx, cy)) = cur
                            {
                                let b = match button {
                                    MouseButton::Left => 0,
                                    MouseButton::Right => 1,
                                    _ => 2,
                                };
                                self.craft_menu_up(cx, cy, b);
                            }
                        }
                    }
                    return;
                }
                // 键位界面：按下/松开都路由给 mcv_ui 屏幕栈（滑杆松开
                // 音效等依赖 release 事件）。
                if self.screen == Screen::KeyBinds {
                    if let Some(cur) = self.last_cursor {
                        let (lx, ly) = self.ui_ctx.to_logical(cur.0 as f32, cur.1 as f32);
                        let b = if button == MouseButton::Left { 0 } else { 1 };
                        match state {
                            ElementState::Pressed => {
                                self.ui_stack.mouse_down(lx, ly, b);
                                self.handle_binds_events();
                            }
                            ElementState::Released => {
                                self.ui_stack.mouse_up(lx, ly, b);
                            }
                        }
                    }
                    return;
                }
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
                // 加载态：不派发攻击/放置（26.1 Screen 非 null 时
                // MouseHandler 不作用于游戏，continueAttack 亦然）。
                if runtime.phase == mcv_logic::game::GamePhase::Loading {
                    return;
                }
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
                        self.right_use_held = pressed;
                        // I-21（波#127-4）：按下沿即时一发，但过原版 delay==0
                        // 轮询门（Minecraft.java:2036——快速连点同样被 4 tick
                        // 节流）；按住后的周期发火在喂步点由 use_repeat 接手。
                        // 右键工作台=开 3x3 合成界面（26.1 CraftingTableBlock）
                        // 的拦截收进 right_use，按下沿与周期发火共用。
                        if pressed && self.use_repeat.ready() {
                            self.use_repeat.on_fire();
                            right_use(
                                runtime,
                                &mut self.screen,
                                &mut self.craft,
                                &mut self.craft_menu,
                                &mut self.craft_held,
                                &mut self.craft_last_click,
                                &self.window,
                            );
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // D-6（波#127-5）：滚轮换快捷栏（26.1 MouseHandler.java:202-221
                // → ScrollWheelHandler）。原版只有 screen==null 走换槽分支
                // （:204-206 有屏时滚轮归屏幕）→ 本仓门 InGame；旁观/飞行
                // 调速分支无对应机制不实现（I-35）。
                if self.screen != Screen::InGame {
                    return;
                }
                let Some(runtime) = self.runtime.as_mut() else {
                    return;
                };
                if runtime.phase == mcv_logic::game::GamePhase::Loading || runtime.dead {
                    return;
                }
                // 原版 mouseWheelSensitivity 默认 1.0（Options.java:445-451
                // logMouse(0)=10^0）→ 一格换一槽；行滚动按原版离散语义取
                // 整格值，像素滚动（触控板）按 100px/格折算（winit 不定义
                // 离散格宽，自设换算，登记注记）。
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x as f64, y as f64),
                    MouseScrollDelta::PixelDelta(p) => (p.x / 100.0, p.y / 100.0),
                };
                if let Some(slot) =
                    self.scroll_hotbar
                        .select(dx, dy, runtime.player.sel_slot, HOTBAR_SIZE)
                {
                    runtime.player.sel_slot = slot;
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
    // HDR 启用判定（`hdr` feature 下 = 桌面且非 GLES 后端；默认恒 false）。
    bool,
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
            bob: glam::Mat4::IDENTITY,
            exposure: 1.0,
        };
        let (sun, day) = mcv_render::sun_state(6_000);
        let scene = mcv_render::Scene {
            camera: &camera,
            time: 0.0,
            day_factor: day,
            sun_dir: sun,
            // 加载画面无天气（晴基线）：env 雾端点 = 属性默认 0/1024；
            // rd 雾层按默认视距 8 chunks（空场景无几何，仅天空）。
            fog_tint: [1.0, 1.0, 1.0],
            // #191 无世界（菜单/加载）→ overworld 默认天底色回退。
            biome_sky: None,
            fog_env: [0.0, 1024.0],
            fog_rd_end: 128.0,
            water_vision: 0.0,
            moon_phase: mcv_render::celestial::moon_phase(6_000),
            width: mw,
            height: mh,
            chunks: &[],
            hud: &hud,
            sprites: &[],
            sprite_fence: 0,
            cloud: None,
            player: None,
            // 菜单/加载画面无 GameRuntime（无 ECS World 可收集）——恒空，
            // 生物实例只在 in-game 场景装配（见下方 draw 内的 collect）。
            mobs: None,
            overlay: None,
            underwater: false,
            particles: None, // 菜单场景无世界粒子
            hand: None,      // 菜单场景无第一人称手持
            drops: None,     // 菜单场景无掉落物可画
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
        // HDR 分流（#223 续，`hdr` feature 默认关 → 本块编译消除）：
        // draw 目标 = RGBA16F 主纹理（随 config 尺寸同步重建），曝光取
        // Camera::exposure（默认 1.0 = 恒等）；None = 未启用/GLES 回退。
        #[cfg(feature = "hdr")]
        let hdr_view: Option<wgpu::TextureView> = sp.hdr.as_mut().map(|st| {
            st.resize(
                self.cached_device.as_ref().unwrap(),
                sp.config.width,
                sp.config.height,
            );
            st.set_exposure(self.cached_queue.as_ref().unwrap(), scene.camera.exposure);
            st.color_view().clone()
        });
        // 借域结束（sp.hdr 可变借用已还），渲染器独占 &mut 可用。
        let renderer = self.renderer.as_mut().unwrap();
        #[cfg(feature = "hdr")]
        let draw_view: &wgpu::TextureView = hdr_view.as_ref().unwrap_or(&view);
        #[cfg(not(feature = "hdr"))]
        let draw_view: &wgpu::TextureView = &view;
        renderer.draw_frame(draw_view, &sp.depth, &scene);
        // tone map 收尾（独立 submit，先于 present；bevy tonemapping node
        // 同序——主纹理读出、渲染到交换链视图）。
        #[cfg(feature = "hdr")]
        if let Some(stage) = sp.hdr.as_ref() {
            stage.tonemap(
                self.cached_device.as_ref().unwrap(),
                renderer.queue(),
                &view,
            );
        }
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
        // 图集容量取证（真机 Mali GLES 定位「平色面」）：error 级常显，
        // logcat 直接 grep `atlas-cap` 看设备真实上限是否容得下全部图集层（LAYERS=838）。
        log::error!(
            "atlas-cap: adapter({:?} {:?}) max_texture_array_layers={} max_texture_dimension_2d={} atlas::LAYERS={}",
            adapter.get_info().backend,
            adapter.get_info().name,
            adapter.limits().max_texture_array_layers,
            adapter.limits().max_texture_dimension_2d,
            mcv_core::atlas::LAYERS,
        );
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
        // 验证错误必须进日志：本次「水从未被绘制」事故的掩盖因素就是
        // GLES 越界 draw 被静默吞掉、全仓零痕迹（2026-10-10 审计 H1）。
        device.on_uncaptured_error(std::sync::Arc::new(|err: wgpu::Error| {
            log::error!("wgpu uncaptured error: {err}");
        }));
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
        // HDR 判定（#223 续）：`hdr` feature（默认关）+ 桌面 + 非 GLES
        // 后端才启用 RGBA16F 链；GLES/Noop（真机 Mali、Android native
        // activity）恒 false = 现状 sRGB 链原样。
        #[cfg(feature = "hdr")]
        let hdr_ok =
            !cfg!(target_os = "android") && mcv_render::is_hdr_capable(adapter.get_info().backend);
        #[cfg(not(feature = "hdr"))]
        let hdr_ok = false;
        Ok((surface, config, depth, device, queue, max_extent, hdr_ok))
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
        if !matches!(
            self.screen,
            Screen::InGame | Screen::Paused | Screen::Death | Screen::Crafting | Screen::Creative
        ) {
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
        // 合成/创造：面板 quad + 命中表（同样须在 runtime 可变借用之前）
        let craft_menu = if matches!(self.screen, Screen::Crafting | Screen::Creative) {
            match self.surface.as_ref() {
                Some(sp) => self.craft_ui(sp.config.width as f32, sp.config.height as f32),
                None => Vec::new(),
            }
        } else {
            self.craft_hot.clear();
            Vec::new()
        };
        // 加载画面标题文案（lang() 借 &self，须在 runtime 可变借用之前取好）
        let lang = self.lang();
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

        // B-4（波#127-3）：失焦自动暂停 = 原版 pauseIfInactive
        // （Minecraft.java:1388-1397，每帧检查）：失焦 && pauseOnLostFocus
        // （默认 true）&& 距最后活动 >500ms → pauseGame(false)（:1595 只在
        // screen==null 开屏 → 本仓门 InGame；加载/死亡对应 LevelLoading/
        // DeathScreen 有屏态，不叠暂停）。pauseOnLostFocus 入 options.txt
        // 登记债（options.rs 红线外）。
        if self.screen == Screen::InGame
            && runtime.phase != mcv_logic::game::GamePhase::Loading
            && !runtime.dead
            && self
                .focus_lost_at
                .is_some_and(|t| t.elapsed().as_millis() >= 500)
        {
            enter_pause(
                runtime,
                &mut self.screen,
                &self.window,
                &mut self.sprint_tap,
                &mut self.right_use_held,
            );
            self.focus_lost_at = None;
        }

        // simulation ticks
        let now = std::time::Instant::now();
        let dt = self
            .last_time
            .replace(now)
            .map_or(0.016, |t| now.duration_since(t).as_secs_f32());
        // 波#127 B-1：暂停 = 停 tick（26.1 Minecraft.java:1300-1308 每帧重算
        // pause = screen.isPauseScreen()；:1773 tick() 内 `!this.pause` 门住
        // level/gameMode.tick）。死亡/容器界面 isPauseScreen=false，世界照常
        // 跑（B-3 正例，本门保持其现状）。last_time 照常刷新：恢复后不拿暂停
        // 时长追帧（追帧预算另有 0.2s 钳，A-3）。
        let paused = is_pause_screen(self.screen);
        let n = feed_steps(paused, &mut self.step_accum, dt);
        // I-21（波#127-4）：按住右键/放置键的周期 use —— 原版
        // handleKeybinds 只在无屏时跑（Minecraft.java:1828），周期门
        // delay==0 && isDown && !isUsingItem（:2036）。触摸按下沿那发由
        // game.rs fx.place→interact（fixed_step 内）完成，这里只置延迟；
        // 桌面按下沿在 MouseInput 即时发，周期续发走本循环。
        let touch_held = runtime.touch.place_held;
        if touch_held && !self.touch_place_prev {
            self.use_repeat.on_fire();
        }
        self.touch_place_prev = touch_held;
        for _ in 0..n {
            runtime.fixed_step(1.0 / 60.0);
            // D-2：冲刺窗只在世界 tick 域递减（暂停冻结/死亡由 game.rs 清
            // input 兜底，同 LocalPlayer.aiStep 的 sprintTriggerTime--）。
            self.sprint_tap.step();
            let use_gate = self.screen == Screen::InGame
                && runtime.phase == mcv_logic::game::GamePhase::Playing
                && !runtime.dead;
            // busy = isUsingItem（Minecraft.java:2036 第三门）：进食与弓蓄力
            // 都算在账使用态（#128-1——旧只查进食，蓄力中周期 use 会打穿
            // right_use 中途开工作台/熔炉屏）。
            let busy = runtime.using_item();
            if self
                .use_repeat
                .step(use_gate && (self.right_use_held || touch_held), busy)
            {
                right_use(
                    runtime,
                    &mut self.screen,
                    &mut self.craft,
                    &mut self.craft_menu,
                    &mut self.craft_held,
                    &mut self.craft_last_click,
                    &self.window,
                );
            }
        }
        // 开着背包死亡：面板收起 + 网格/光标走世界掉落实体
        // （drop_container_items 通道已接；fixed_step 死亡丢弃刚清完 36 格，
        // to_inventory=false 让 carried 也直落世界，不许塞回空手）。
        if runtime.dead && matches!(self.screen, Screen::Crafting | Screen::Creative) {
            close_craft(&mut self.craft, &mut self.craft_menu, false, runtime);
            self.craft_held = 0;
            self.craft_last_click = None;
            self.creative = None;
            self.screen = Screen::InGame;
        }
        // 死亡 → 切死亡界面（显示 26.1 deathScreen，需鼠标点按钮）
        if runtime.dead && self.screen == Screen::InGame {
            self.screen = Screen::Death;
            if let Some(w) = self.window.as_ref() {
                let _ = w.set_cursor_grab(winit::window::CursorGrabMode::None);
                w.set_cursor_visible(true);
            }
        }
        // 时间推进已收进 fixed_step 的 tick 累加器（(dt*20) as u64 在
        // 60 fps 下截断恒 0，昼夜曾因此冻结）。
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

        // E0：draw 段计时起点（HUD/Scene 装配 + draw_frame 提交；帧界归并
        // 在下方日志行前统一做。纯计时记账，见 mcv_logic::game::FrameTiming）。
        let draw_t0 = std::time::Instant::now();
        let camera = runtime.camera(sp.config.width as f32 / sp.config.height as f32);
        let (sun, day) = mcv_render::sun_state(runtime.time_ticks);
        // 天气视觉（WeatherAttributes SKY_LIGHT_FACTOR/雾修饰；雾端点 =
        // AtmosphericFogEnvironment.java:68-72 雨偏移后的 env 层 0..1024）。
        let (day, weather_tint, (env_start, env_end)) = runtime.weather_visual(day);
        // 进世界加载画面（26.1 LevelLoadingScreen）：盖在游戏画面上的
        // 独立 HUD（不画游戏 HUD/触屏按钮，玩家此刻不应看到血条快捷栏）。
        let loading = runtime.phase == mcv_logic::game::GamePhase::Loading;
        // #222 批 A：build_hud 返回 HudBatch（quads + 精灵命令 + 栅栏）。
        let mut batch = if loading {
            let (grid_radius, grid) = runtime.loading_grid();
            let model = crate::loading_ui::LoadScreenModel {
                progress: runtime.loading_progress_smoothed(),
                grid_radius,
                grid,
                title: crate::i18n::t(lang, "multiplayer.downloadingTerrain").to_string(),
            };
            mcv_render::sprite2d::HudBatch {
                quads: crate::loading_ui::quads(
                    sp.config.width as f32,
                    sp.config.height as f32,
                    &model,
                    self.renderer.as_ref().and_then(|r| r.gui()),
                ),
                sprites: Vec::new(),
                sprite_fence: 0,
            }
        } else {
            runtime.build_hud(
                sp.config.width as f32,
                sp.config.height as f32,
                self.renderer.as_ref().and_then(|r| r.gui()),
                self.screen != Screen::Death,
                // 合成/创造面板自带 36 格（含快捷栏行），不再重复画底部快捷栏
                !matches!(self.screen, Screen::Crafting | Screen::Creative),
            )
        };
        let sprite_cmds = std::mem::take(&mut batch.sprites);
        let sprite_fence = batch.sprite_fence;
        let mut hud = batch.quads;
        if matches!(self.screen, Screen::Paused | Screen::Death) {
            // 暂停/死亡：游戏画面之上叠 MC 风格菜单（按钮贴图 + 阴影字体）
            hud.extend(pause_menu);
        }
        if matches!(self.screen, Screen::Crafting | Screen::Creative) {
            hud.extend(craft_menu);
        }
        let chunks: Vec<mcv_render::RenderChunk> = runtime.render_chunks().to_vec();
        // 云（官方 CloudStatus 映射到模块设置；云距跟随渲染距离，MC renderDistance 语义）
        let cloud_settings = mcv_render::CloudSettings {
            enabled: self.set_clouds != 0,
            fast: self.set_clouds == 1,
            distance: (runtime.render_dist * 16) as f32,
            // 云色夜轨（Timelines.java:136-143，夜 ×(0.1,0.1,0.15)）。
            tint: mcv_render::env_color::cloud_multiplier(runtime.time_ticks).to_array(),
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
            // 暂停冻结走路动画（B-1 伴生：原版 limbSwing 在 tick 域推进，
            // 暂停=不 tick=腿停摆；我们渲染帧驱动，需显式门）。
            if !paused {
                self.walk_anim = mcv_render::update_walk_animation(
                    self.walk_anim.0,
                    self.walk_anim.1,
                    speed,
                    dt,
                );
            }
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
        // 生物实例收集（审计 D P0 接线）：mobs ECS 逻辑早已在固定步运转，
        // 但场景装配恒喂 None → 渲染端从未画过怪。此处只读 World 拼装
        // MobInstance，视距裁剪 = 渲染距离（与区块流同界）。
        // partialTick（26.1 LivingEntity 渲染插值 lerp(xOld, x, partialTick)
        // 的残差面）：喂步累加器余量归一化——本帧已喂 0..n 个 60 Hz 步，
        // 余量 ∈ [0, 1/60) 即"距上一完成步的墙钟占比"；怪渲染位置取
        // pos_old→pos 的该占比插值，消除步边界瞬移（与粒子 tick_frac 插值
        // 同思路，粒子在 20 Hz tick 域、mob 在 60 Hz 物理步域）。
        let mob_partial = (self.step_accum * 60.0).clamp(0.0, 1.0);
        // 攻击拾取同源（26.1 客户端 pick 用插值盒）：准星打的是什么，判定
        // 就得读什么——喂给本帧攻击按沿 try_attack 的 partial 与此处渲染的
        // 完全同值（事件在帧间到达时最多滞后一帧，残差 ∈ [0,1] 仍与屏幕
        // 位姿同界，见 GameRuntime::attack_partial 注释）。
        runtime.attack_partial = mob_partial;
        let mob_instances = mcv_logic::mob_render::collect_mob_instances_in(
            &runtime.mobs_app.world,
            Some((camera.pos, runtime.render_dist as f32 * 16.0)),
            mob_partial,
        );
        // 掉落物渲染快照（#150 刀 1）：ECS ItemDrop → billboard 批
        // （图标解析/光照/相位换算在 game 层 item_drops 内聚）。
        let drop_draws = runtime.item_drops();
        let scene = mcv_render::Scene {
            camera: &camera,
            time: (runtime.time_ticks % 24_000) as f32 / 20.0,
            day_factor: day,
            sun_dir: sun,
            fog_tint: weather_tint,
            // #191 相机处群系天空底色（26.1 GaussianSampler 环境属性
            // 混合，EnvironmentAttributeProbe.java:30-34 实物口径）。
            biome_sky: Some(runtime.biome_sky_srgb(camera.pos.x, camera.pos.z)),
            fog_env: [env_start, env_end],
            fog_rd_end: (runtime.render_dist * 16) as f32,
            water_vision: runtime.water_vision(),
            // 月相：26.1 月相周期 192000 tick（timeline/moon.json）。
            moon_phase: mcv_render::celestial::moon_phase(runtime.time_ticks),
            width: sp.config.width as f32,
            height: sp.config.height as f32,
            chunks: &chunks,
            hud: &hud,
            sprites: &sprite_cmds,
            sprite_fence,
            cloud: clouds.map(|c| (c, cloud_settings)),
            player: has_player.then_some((&models, 0)),
            mobs: Some(&mob_instances[..]),
            overlay,
            // 水下雾：眼位在水块内（Player.isEyeInFluid(WATER)）。
            underwater: runtime.eye_under_water(),
            // M8a 主接线：粒子池 + 帧内插值余量（Scene 生命周期借用）。
            particles: Some((&runtime.particles, runtime.tick_frac())),
            // 第一人称手持：仅游玩态第一人称画（26.1 renderItemInHand 的
            // isFirstPerson && screen==null 门；加载/死亡/第三人称不画）。
            hand: (!loading
                && !runtime.dead
                && runtime.cam_type == mcv_logic::game::CameraType::FirstPerson)
                .then_some(mcv_render::HandRender {
                    item: runtime.hand_item(),
                    swing: runtime.swing_progress(),
                }),
            drops: Some(&drop_draws[..]),
        };
        // HDR 分流（#223 续，`hdr` feature 默认关 → 本块编译消除）：
        // draw 目标 = RGBA16F 主纹理（随 config 尺寸同步重建），曝光取
        // Camera::exposure；None = 未启用/GLES 回退 = 原直达路。
        #[cfg(feature = "hdr")]
        let hdr_view: Option<wgpu::TextureView> = sp.hdr.as_mut().map(|st| {
            st.resize(
                self.cached_device.as_ref().unwrap(),
                sp.config.width,
                sp.config.height,
            );
            st.set_exposure(self.cached_queue.as_ref().unwrap(), scene.camera.exposure);
            st.color_view().clone()
        });
        // 借域结束（sp.hdr 可变借用已还），渲染器独占 &mut 可用。
        let renderer = self.renderer.as_mut().unwrap();
        #[cfg(feature = "hdr")]
        let draw_view: &wgpu::TextureView = hdr_view.as_ref().unwrap_or(&view);
        #[cfg(not(feature = "hdr"))]
        let draw_view: &wgpu::TextureView = &view;
        renderer.draw_frame(draw_view, &sp.depth, &scene);
        // tone map 收尾（独立 submit，先于 present；bevy tonemapping node
        // 同序——主纹理读出、渲染到交换链视图）。
        #[cfg(feature = "hdr")]
        if let Some(stage) = sp.hdr.as_ref() {
            stage.tonemap(
                self.cached_device.as_ref().unwrap(),
                renderer.queue(),
                &view,
            );
        }
        // E0：draw 段归账（HUD/Scene 装配 + draw_frame 提交）。
        runtime.timing_frame.draw_us += draw_t0.elapsed().as_micros() as u64;
        // E0：帧界归并——本帧分段窗口并入 1Hz 均值/峰值累计器（取帧失败
        // 提前 return 的帧不归并，帧数略欠计，纯观测口径无行为影响）。
        runtime.end_frame_timing();
        // #80 结构性指标（真机取证通路，不进 HUD 像素）：~1 Hz 一行，
        // 合批 draw 收敛（entries/chunks）与 staging 上传带宽（staged KB、
        // fallback 笔数恒 0 = 环容量足够）。#115 刀 5a 起带网格域字节
        // 记账（resident_kb = GPU 驻留、pool_kb = CPU 池留存、evicted =
        // 越预算 LRU 逐出笔数、live_peak_kb = GPU 顶点账峰值——#168 起后
        // 两者即池 LRU/重建瞬态放大的真机取证面）——真机 Mali 预算取证
        // 行，口径表见 docs/memory-budget.md。worker 路死账观测面（三路
        // 合并一套字段）：dispatch_gap = mesh 在途账超看门狗窗的块数
        // （>0 = 丢失疑点在窗）、worker_lost = mesh 丢失事件 + light/
        // terrain 失败回执累计（自愈后仍累计）——「隐形墙/黑块/生成停摆」
        // 复现的一条日志定位面，口径表见 docs/worker-accounting.md。
        // E0（蓝图 §3-E0）：同行扩展帧预算分段耗时——帧内执行序
        // fixed_step（含 ecs 单列）→ stream（init/edge/io/remesh 子段）→
        // flush → draw；mean = 窗口内帧均值、max = 单帧峰值（微秒）。
        let log_due = self
            .stats_log
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1));
        if log_due {
            self.stats_log = Some(std::time::Instant::now());
            let st = renderer.last_frame_stats();
            let ms = runtime.mesh_stats();
            let tt = runtime.take_frame_timing_stats();
            let mean = tt.mean_us();
            log::info!(
                "render-stats: draws={} water_draws={} entries={} chunks={} rebuilds={} rebuild_kb={} staged_uploads={} staged_kb={} fallback={} resident_kb={} pool_kb={} trimmed={} evicted={} debt={} live_peak_kb={} dispatch_gap={} worker_lost={} | frame-timing[us] mean: fixed_step={} ecs={} stream={} (init={} edge={} io={} remesh={}) flush={} draw={} | max: fixed_step={} ecs={} stream={} (init={} edge={} io={} remesh={}) flush={} draw={} frames={}",
                st.opaque_draws,
                st.water_draws,
                ms.draw_entries,
                ms.meshed_chunks,
                ms.batch_rebuilds,
                ms.rebuild_bytes / 1024,
                ms.staged_uploads,
                ms.staged_bytes / 1024,
                ms.fallback_uploads,
                ms.resident_bytes / 1024,
                ms.cpu_pool_bytes / 1024,
                ms.trimmed_chunks,
                ms.evicted_chunks,
                ms.debt_chunks,
                ms.live_vertex_peak_bytes / 1024,
                ms.dispatch_gap,
                ms.worker_lost,
                mean.fixed_step_us,
                mean.ecs_us,
                mean.stream_us,
                mean.init_us,
                mean.edge_us,
                mean.io_us,
                mean.remesh_us,
                mean.flush_us,
                mean.draw_us,
                tt.max.fixed_step_us,
                tt.max.ecs_us,
                tt.max.stream_us,
                tt.max.init_us,
                tt.max.edge_us,
                tt.max.io_us,
                tt.max.remesh_us,
                tt.max.flush_us,
                tt.max.draw_us,
                tt.frames,
            );
        }
        renderer.queue().present(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 唯一临时存档目录（照 mcv_logic/tests/runtime.rs 的做法，测试并行安全）。
    fn tmp_world(tag: &str) -> std::path::PathBuf {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
        std::env::temp_dir().join(format!("mcv-app-{}-{:x}", tag, h.finish()))
    }

    /// B-1（暂停停 tick，行为级）：用真实 GameRuntime 走 redraw 的喂步路径
    /// （feed_steps + fixed_step），InGame 下 1 秒墙钟 = 20 tick；进暂停后
    /// 同样喂 1 秒，time_ticks 一格不动（昼夜/饥饿/mob AI 全在 fixed_step
    /// 域内，随之冻结）；死亡/容器界面（isPauseScreen=false，26.1
    /// DeathScreen.java:149 / AbstractContainerScreen.java:640）保持走时。
    #[test]
    fn paused_screen_freezes_world_ticks() {
        let mut rt = GameRuntime::new_headless(11, tmp_world("pause-tick"), GameMode::Survival);
        let mut acc = 0.0f32;
        let feed = |rt: &mut GameRuntime, acc: &mut f32, paused: bool| {
            for _ in 0..60 {
                let n = feed_steps(paused, acc, 1.0 / 60.0);
                for _ in 0..n {
                    rt.fixed_step(1.0 / 60.0);
                }
            }
        };
        feed(&mut rt, &mut acc, false);
        assert_eq!(rt.time_ticks, 20, "InGame 喂 1s 墙钟 = 20 tick（20 TPS）");

        // 进暂停（走真实入口 enter_pause）：tick 冻结 + 按压态释放。
        rt.input.forward = true;
        rt.input.mining = true;
        let mut screen = Screen::InGame;
        let mut sprint = SprintTapWindow {
            window_steps: 5,
            latched: true,
            key_held: true,
        };
        let mut right_held = true;
        enter_pause(&mut rt, &mut screen, &None, &mut sprint, &mut right_held);
        assert_eq!(screen, Screen::Paused);
        assert!(
            !rt.input.forward,
            "进暂停须 releaseAll（setScreen :1148 语义）"
        );
        assert!(!rt.input.mining, "挖/放按压态一并释放，恢复后无悬真");
        assert!(
            !sprint.sprint() && sprint.window_steps == 0,
            "冲刺窗/latch/按键态同释"
        );
        assert!(!right_held, "右键按住态同释");
        let t0 = rt.time_ticks;
        feed(&mut rt, &mut acc, is_pause_screen(screen));
        assert_eq!(
            rt.time_ticks, t0,
            "暂停界面喂 1s 世界 0 tick（原版 !pause 门）"
        );

        // 正例保持：死亡/合成界面不停 tick（每喂 1s 墙钟走 20 tick）。
        let mut baseline = rt.time_ticks;
        for s in [Screen::Death, Screen::Crafting, Screen::Creative] {
            assert!(!is_pause_screen(s), "{s:?} 非 isPauseScreen，世界照常跑");
            feed(&mut rt, &mut acc, is_pause_screen(s));
            baseline += 20;
            assert_eq!(rt.time_ticks, baseline, "{s:?} 界面下 1s 应再走 20 tick");
        }
    }

    /// I-21（右键连放周期门，纯节拍）：26.1 `rightClickDelay` 机器——
    /// startUseItem 置延迟 4 tick（Minecraft.java:1705）、tick 每 tick 减 1
    /// （:1779-1781）、`isDown && delay==0 && !isUsingItem` 再发（:2036-2038）。
    /// 60Hz 步长换算（3 步/tick）：按住时首发 12 步后来、周期 12 步；
    /// `busy`（进食=isUsingItem）与松开只走冷却不发火。
    #[test]
    fn right_use_repeat_fires_every_four_ticks_while_held() {
        let mut u = RightUseRepeat::default();
        assert!(
            u.ready(),
            "冷启动 delay=0 可立即发火（按下沿即时一发的前提）"
        );
        u.on_fire();
        assert!(!u.ready(), "发火即置 delay=4 tick（:1705 无条件）");
        let mut fires = Vec::new();
        for i in 0..40 {
            if u.step(true, false) {
                fires.push(i);
            }
        }
        assert_eq!(
            fires,
            vec![11, 23, 35],
            "按住周期 = 12 步 = 4 tick（0..40 内三跳）"
        );
        // 松开：冷却照减（原版 tick 无条件 `rightClickDelay--`），不发火。
        let mut u2 = RightUseRepeat::default();
        u2.on_fire();
        for _ in 0..12 {
            assert!(!u2.step(false, false));
        }
        assert!(u2.ready(), "松开后冷却自然走尽，下次按下沿可再发");
        // busy=进食中（:2036 !isUsingItem 门）：冷却走尽也不发，解除当步发。
        let mut u3 = RightUseRepeat::default();
        u3.on_fire();
        for _ in 0..12 {
            assert!(!u3.step(true, true), "isUsingItem 期间禁止 startUseItem");
        }
        assert!(u3.step(true, false), "解除使用态后当步恢复发火");
    }

    /// I-21（按住右键连放，行为级）：真实 GameRuntime 上重现 redraw 喂步点
    /// 结构（按下沿即时 + step 周期发火），1s 墙钟按住 = 6 次 use；石头墙前
    /// 至少落地 ≥2 块（修复前按下沿只发 1 次，砌墙需逐块点击）。
    #[test]
    fn hold_right_button_repeats_use_every_four_ticks() {
        let mut rt = GameRuntime::new_headless(31, tmp_world("i21-hold"), GameMode::Survival);
        rt.phase = mcv_logic::game::GamePhase::Playing;
        // 手动一块地形：chunk(0,0) 石头墙 x=11（y66..72, z9），玩家 yaw=π/2
        // 朝 +x。Uploaded 阶段体素才对 WorldView 可见（stage 门）。
        let h = std::sync::Arc::new(mcv_core::ChunkHandle::new(mcv_core::ChunkPos::new(0, 0)));
        for y in 66..=72 {
            h.voxels
                .write()
                .unwrap()
                .set_flat(mcv_core::vidx(11, y, 9), mcv_core::BlockId(1));
        }
        h.advance_to(mcv_core::Stage::Uploaded);
        rt.chunks_mut().insert(mcv_core::ChunkPos::new(0, 0), h);
        rt.player.pos = glam::Vec3::new(8.5, 68.0, 9.5);
        rt.player.vel = glam::Vec3::ZERO;
        rt.player.yaw = std::f32::consts::FRAC_PI_2;
        rt.player.pitch = 0.0;
        rt.hotbar.slots[0] = mcv_item::ItemStack::new(mcv_item::COBBLESTONE, 64);
        rt.player.sel_slot = 0;
        assert!(rt.look_block().is_some(), "准星须命中墙");

        let mut u = RightUseRepeat::default();
        // 按下沿（MouseInput ready() 即时发通路）。
        assert!(u.ready());
        u.on_fire();
        rt.interact(true);
        let mut uses = 1usize;
        for _ in 0..60 {
            rt.fixed_step(1.0 / 60.0);
            if u.step(true, rt.using_item()) {
                rt.interact(true);
                uses += 1;
            }
        }
        assert_eq!(
            uses, 6,
            "按住 1s = 按下沿 + 每 4 tick 一发（Minecraft:2036 周期）"
        );
        let placed = 64 - rt.hotbar.slots[0].count as usize;
        assert!(
            placed >= 2,
            "连放至少落 2 块圆石（生存逐次消耗；旧行为仅按下沿 1 块），placed={placed}"
        );
    }

    /// I-21+#128-1（isUsingItem 第三门，行为级）：按住右键**蓄弓**期间，
    /// 周期 use 必须被 `!player.isUsingItem()` 门挡住（Minecraft.java:2036——
    /// isUsingItem 覆盖进食与弓蓄力两种在账使用态）。修复前 busy 只查进食，
    /// 蓄力中周期发火会打穿 right_use（准星下是工作台/熔炉时中途开屏）。
    #[test]
    fn use_repeat_blocked_while_bow_charging() {
        let mut rt = GameRuntime::new_headless(31, tmp_world("i21-bow"), GameMode::Survival);
        rt.phase = mcv_logic::game::GamePhase::Playing;
        let bow = mcv_item::item_by_name("bow").expect("bow 物品在档");
        rt.hotbar.slots[0] = mcv_item::ItemStack::new(bow, 1);
        rt.player.sel_slot = 0;
        rt.input.placing = true;
        rt.fixed_step(1.0 / 60.0);
        assert!(rt.using_item(), "按住右键持弓 = isUsingItem（蓄力在账）");
        let mut u = RightUseRepeat::default();
        u.on_fire();
        let mut fires = 0usize;
        for _ in 0..60 {
            rt.fixed_step(1.0 / 60.0);
            if u.step(true, rt.using_item()) {
                fires += 1;
            }
        }
        assert_eq!(
            fires, 0,
            "蓄力期间周期 use 全被 isUsingItem 门挡住（Minecraft.java:2036）"
        );
    }

    /// D-2（双击前进冲刺窗，行为级）：26.1 LocalPlayer:764-766/798-810 +
    /// Options sprintWindow 默认 7 tick。第一次按下开窗；窗口内第二次按下
    /// latch 冲刺（=setSprinting）；第 8 tick（21 步）后窗口过期，二次按下
    /// 只重开窗不冲刺；松开解除 latch（shouldStopRunSprinting 失去前进冲量）；
    /// 潜行/后退按下沿清窗清 latch（:798-800）。
    #[test]
    fn double_tap_forward_opens_seven_tick_sprint_window() {
        let mut w = SprintTapWindow::default();
        assert!(!w.sprint());
        w.on_forward(true); // 第一次按下：开窗
        assert!(!w.sprint(), "单击不开冲刺");
        w.on_forward(false);
        w.on_forward(true); // 窗口内第二次按下：点火
        assert!(w.sprint(), "双击必须 latch 冲刺（原版 setSprinting）");
        w.on_forward(false);
        assert!(!w.sprint(), "松开前进=失去冲量，冲刺解除");
        // 窗口过期：21 步（7 tick）后再按只重开窗。
        let mut w = SprintTapWindow::default();
        w.on_forward(true);
        for _ in 0..21 {
            w.step();
        }
        assert_eq!(w.window_steps, 0, "7 tick 窗口整步走尽");
        w.on_forward(false);
        w.on_forward(true);
        assert!(!w.sprint(), "窗口过期的第二次按下不得冲刺");
        // 窗口有效到最后一格：20 步后仍 >0。
        let mut w = SprintTapWindow::default();
        w.on_forward(true);
        for _ in 0..20 {
            w.step();
        }
        w.on_forward(false);
        w.on_forward(true);
        assert!(w.sprint(), "第 7 tick 内的双击仍须冲刺");
        // 潜行/后退清窗清 latch；Ctrl 冲刺键与 latch 正交（key_held ∨ latch）。
        let mut w = SprintTapWindow::default();
        w.on_forward(true);
        w.on_sneak_or_back();
        assert_eq!(w.window_steps, 0);
        w.on_forward(false);
        w.on_forward(true);
        assert!(!w.sprint(), "潜行清窗后再按=新窗口，不冲刺");
        w.set_key_held(true);
        assert!(w.sprint(), "Ctrl 长按冲刺（既有语义）不受窗影响");
        w.set_key_held(false);
        assert!(!w.sprint());
    }

    /// B-1/D-5（暂停拦输入）：玩法输入门只认 InGame——暂停菜单里 WASD/跳/
    /// 潜行/数字键不得再写 runtime.input（原版 KeyMapping 在 Screen!=null
    /// 时不驱动游戏；键盘分发的 `playing` 门直接用本函数）。
    #[test]
    fn pause_menu_blocks_gameplay_input() {
        assert!(gameplay_input_allowed(Screen::InGame));
        for s in [
            Screen::Paused,
            Screen::Death,
            Screen::Crafting,
            Screen::Creative,
            Screen::KeyBinds,
        ] {
            assert!(
                !gameplay_input_allowed(s),
                "{s:?} 界面有屏，玩法键须被屏幕栈拦截"
            );
        }
    }

    /// #152 tooltip 物品名查询（26.1 Screen.getTooltipFromItem:237-242 首
    /// 行 = 物品显示名）：modern 官方键 + legacy 无键回退注册名两档 +
    /// 空堆不显（ACS:196 hoveredSlot.hasItem() 门）。
    #[test]
    fn tooltip_title_modern_legacy_and_empty() {
        let modern = mcv_item::ItemStack::new(mcv_item::OAK_STAIRS, 1);
        assert_eq!(
            tooltip_title(crate::i18n::Lang::En, &modern),
            Some("Oak Stairs".to_string()),
            "modern：i18n 表内官方名"
        );
        let legacy = mcv_item::ItemStack::new(mcv_item::LOG, 1);
        assert_eq!(
            tooltip_title(crate::i18n::Lang::En, &legacy),
            Some("log".to_string()),
            "legacy：无官方 key 回退注册名（不伪造翻译）"
        );
        assert_eq!(
            tooltip_title(crate::i18n::Lang::En, &mcv_item::ItemStack::empty()),
            None,
            "空堆不显（ACS:196 门）"
        );
    }
}
