//! 按键重映射界面：纯逻辑（捕获状态机 + 双语文案表），不依赖 winit/渲染，
//! 可独立单测；app.rs 只做接线。M8b 起屏幕骨架/布局/交互/绘制迁到
//! [`mcv_ui`]（屏幕栈 + Button + Tooltip + 等待按键模态）。

use mcv_game::keymap::{Action, KeyMap, VKey};
use mcv_render::gui::SpriteSheet;
use mcv_render::text::text_width;
use mcv_ui::UiContext;

use crate::i18n::Lang;

/// 捕获状态：点击某行进入 Capturing，下一次按键即收敛回 Idle（Esc = 取消）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Capture {
    #[default]
    Idle,
    Capturing(Action),
}

/// 一次按键喂给状态机的结果（UI 侧据此刷新，无需再判内部状态）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureEvent {
    NotCapturing,
    Cancelled,
    Bound,
    Conflict,
}

/// 重映射捕获状态机。
#[derive(Clone, Copy, Debug, Default)]
pub struct BindCapture {
    pub state: Capture,
    /// 上一次因冲突而未生效的动作：该行红底提示，下一次交互（点击/按键）清除。
    pub conflict: Option<Action>,
}

impl BindCapture {
    /// 点击某动作行：开始捕获，并清掉旧的冲突红标。
    pub fn begin(&mut self, a: Action) {
        self.conflict = None;
        self.state = Capture::Capturing(a);
    }

    /// 离开界面等场景直接取消捕获。
    pub fn cancel(&mut self) {
        self.state = Capture::Idle;
    }

    /// 处理一次物理键按下。Esc → 取消并保持原绑定；其他键：若与别的动作
    /// 冲突（[`KeyMap::conflicting`]）则**保持原绑定**并置红标（保守策略：
    /// 允许一键多动作会改变 [`KeyMap::action_for`] 语义），无冲突才真正重绑。
    pub fn key_down(&mut self, map: &mut KeyMap, k: VKey) -> CaptureEvent {
        let Capture::Capturing(a) = self.state else {
            return CaptureEvent::NotCapturing;
        };
        self.state = Capture::Idle;
        if k == VKey::Escape {
            self.conflict = None;
            return CaptureEvent::Cancelled;
        }
        if map.conflicting(a, k).is_some() {
            self.conflict = Some(a);
            return CaptureEvent::Conflict;
        }
        map.set(a, k);
        self.conflict = None;
        CaptureEvent::Bound
    }
}

/// 行按钮 id：下标与 [`Action::ALL`] 一一对应（mc_button/hit 表需要 &'static str）。
pub const BIND_IDS: [&str; Action::ALL.len()] = [
    "bind0", "bind1", "bind2", "bind3", "bind4", "bind5", "bind6", "bind7", "bind8", "bind9",
    "bind10", "bind11", "bind12", "bind13", "bind14", "bind15",
];

/// id → `Action::ALL` 下标（非行按钮 id 返回 None）。
pub fn bind_index_of(id: &str) -> Option<usize> {
    BIND_IDS.iter().position(|s| *s == id)
}

/// 界面文案 key。i18n compact 表受 gen-lang.py 的 40 key 上限约束，
/// 键位界面自带英文字面量 + 中文对照，不走生成表。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    /// 界面标题
    Title,
    /// 恢复默认按钮
    Restore,
    /// 保存按钮
    Save,
    /// 设置页入口按钮
    Entry,
    /// 捕获中提示
    PressKey,
}

/// 界面双语文案（ASCII + CJK，HUD 字体已支持两者）。
pub fn ui_text(lang: Lang, t: Text) -> &'static str {
    use Text::*;
    match (lang, t) {
        (Lang::En, Title) => "Key Binds",
        (Lang::Zh, Title) => "按键绑定",
        (Lang::En, Restore) => "Restore Defaults",
        (Lang::Zh, Restore) => "恢复默认",
        (Lang::En, Save) => "Save",
        (Lang::Zh, Save) => "保存",
        (Lang::En, Entry) => "Key Binds...",
        (Lang::Zh, Entry) => "按键绑定...",
        (Lang::En, PressKey) => "Press key...",
        (Lang::Zh, PressKey) => "请按按键...",
    }
}

/// 动作标签：英文用 `Action::display()`，中文在此自带对照。
pub fn action_label(lang: Lang, a: Action) -> &'static str {
    if lang == Lang::En {
        return a.display();
    }
    match a {
        Action::Forward => "前进",
        Action::Back => "后退",
        Action::Left => "左移",
        Action::Right => "右移",
        Action::Jump => "跳跃",
        Action::Sneak => "潜行",
        Action::Sprint => "疾跑",
        Action::FlyToggle => "切换飞行",
        Action::Inventory => "开关背包",
        Action::Debug => "切换调试信息",
        Action::PickBlock => "选取方块",
        Action::Screenshot => "截图",
        Action::Pause => "暂停菜单",
        Action::Mine => "攻击/挖掘",
        Action::Place => "使用/放置",
        Action::HotbarSelect => "快捷栏选择",
    }
}

/// WidgetSet id 空间：`0..Action::ALL.len()` = 动作行，其后是底部按钮。
pub const ID_RESET: usize = 100;
pub const ID_SAVE: usize = 101;
pub const ID_BACK: usize = 102;

/// 按键重映射屏（MC KeyBindsScreen 对应物）：布局/命中/三态贴图/tooltip
/// 全部经 mcv_ui 控件集，本结构只保留游戏侧语义（KeyMap 改写、冲突提示、
/// 等待按键模态）。
pub struct KeyBindsScreen {
    /// 捕获状态机（等待按键模态本体）。
    pub binds: BindCapture,
    widgets: mcv_ui::WidgetSet,
    /// 框架控件事件缓冲（mouse_down 累积，take_events 取走）。
    events: Vec<mcv_ui::widget::UiEvent>,
    /// 右列键名（refresh_rows 按 KeyMap 刷新；draw 只读）。
    key_labels: Vec<String>,
    /// 双语文案缓存（构造时定，切换语言 = 重建屏幕）。
    conflict_hint: String,
    press_key: String,
    lang: Lang,
    /// 悬停计时基准（draw 时从 UiGraphics.now_ms 同步）。
    now_ms: u64,
}

impl KeyBindsScreen {
    pub fn new(lang: Lang) -> Self {
        let conflict_hint = if lang == Lang::En {
            "Key conflict, binding kept".to_string()
        } else {
            "按键冲突，保持原绑定".to_string()
        };
        let press_key = ui_text(lang, Text::PressKey).to_string();
        Self {
            binds: BindCapture::default(),
            widgets: mcv_ui::WidgetSet::default(),
            events: Vec::new(),
            key_labels: Action::ALL.iter().map(|_| String::new()).collect(),
            conflict_hint,
            press_key,
            lang,
            now_ms: 0,
        }
    }

    /// 布局（on_open/on_resize 共用；逻辑像素，不再手乘 GUI scale）。
    /// 26.1 KeyBindsScreen：标题顶部，行列表 + 底部三按钮。
    fn layout(&mut self, ctx: &UiContext) {
        let w = ctx.width as f32;
        let h = ctx.height as f32;
        let btn_w = 200.0_f32.min(w * 0.9);
        let btn_h = 20.0_f32;
        let gap = 4.0_f32;
        let top = 26.0_f32;
        let y_btn = h - (btn_h + 4.0);
        let avail = (y_btn - 8.0 - top).max(16.0);
        let row_h = (avail / Action::ALL.len() as f32).min(16.0);
        let x = w * 0.5 - btn_w * 0.5;
        self.widgets = mcv_ui::WidgetSet::default();
        for (i, _) in Action::ALL.iter().enumerate() {
            // 行文本（动作名 + 键名两列）由 draw 自绘（联合缩号防溢出），
            // 框架按钮负责贴图三态/命中/悬停。
            self.widgets
                .add_button(i, x, top + i as f32 * row_h, btn_w, row_h, "");
        }
        let bw3 = (btn_w - gap * 2.0) / 3.0;
        // 底部三按钮走原版白皮 button 贴图（对齐任务 #53 素材红线后
        // 的现网 mc_button 风格：不染色、不回退纯色）。
        self.widgets.add_button(
            ID_RESET,
            x,
            y_btn,
            bw3,
            btn_h,
            ui_text(self.lang, Text::Restore),
        );
        self.widgets.add_button(
            ID_SAVE,
            x + bw3 + gap,
            y_btn,
            bw3,
            btn_h,
            ui_text(self.lang, Text::Save),
        );
        self.widgets.add_button(
            ID_BACK,
            x + (bw3 + gap) * 2.0,
            y_btn,
            bw3,
            btn_h,
            crate::i18n::t(self.lang, "gui.back"),
        );
    }

    /// 每帧刷新：捕获/冲突态着色 + 右列键名 + 冲突行 tooltip。
    pub fn refresh_rows(&mut self, map: &KeyMap) {
        let mut conflict_rows = Vec::new();
        for (i, b) in self.widgets.buttons.iter_mut().enumerate() {
            if i >= Action::ALL.len() {
                break;
            }
            let a = Action::ALL[i];
            let capturing = self.binds.state == Capture::Capturing(a);
            let conflict = self.binds.conflict == Some(a);
            b.tint = if conflict {
                [1.0, 0.5, 0.5, 1.0]
            } else if capturing {
                [0.6, 0.85, 1.0, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            let key = if capturing {
                self.press_key.clone()
            } else {
                map.get(a).display().to_string()
            };
            self.key_labels[i] = key;
            if conflict {
                conflict_rows.push(i);
            }
        }
        for i in 0..Action::ALL.len() {
            self.widgets.set_tooltip(i, "");
        }
        for i in conflict_rows {
            self.widgets.set_tooltip(i, &self.conflict_hint);
        }
    }

    /// 点击动作行：进入捕获（等待按键模态）。`map` 预留（行点击不改表）。
    pub fn begin_row(&mut self, i: usize, _map: &mut KeyMap) {
        if i < Action::ALL.len() {
            self.binds.begin(Action::ALL[i]);
        }
    }

    /// 恢复默认并取消捕获。
    pub fn restore_defaults(&mut self, map: &mut KeyMap) {
        map.restore_default();
        self.binds.cancel();
    }

    /// 键盘输入（等待按键模态）：捕获中的键喂给状态机。
    /// 返回 true = 已消费（app.rs 不再走 Esc 关屏）。
    pub fn key_input(&mut self, vk: Option<VKey>, map: &mut KeyMap) -> bool {
        let Some(vk) = vk else {
            return false;
        };
        if matches!(self.binds.state, Capture::Capturing(_)) {
            let _ = self.binds.key_down(map, vk);
            return true;
        }
        false
    }

    /// 取走框架控件事件（Click/Value/...；app.rs 据此路由动作）。
    pub fn take_events(&mut self) -> Vec<mcv_ui::widget::UiEvent> {
        std::mem::take(&mut self.events)
    }
}

/// mcv_ui 屏幕栈接入：布局/输入/绘制全走框架；键位语义在键输入路径
/// （app.rs 把 winit → VKey 喂 [`KeyBindsScreen::key_input`]，等待按键
/// 模态吃掉即消费）。
impl mcv_ui::Screen for KeyBindsScreen {
    fn on_open(&mut self, ctx: &mcv_ui::UiContext) {
        self.layout(ctx);
    }

    fn on_resize(&mut self, ctx: &mcv_ui::UiContext) {
        self.layout(ctx);
    }

    fn on_close(&mut self) {
        // 离屏取消捕获（原 Screen::removed 语义）
        self.binds.cancel();
    }

    fn mouse_down(&mut self, x: f32, y: f32, button: u8) -> bool {
        let ev = self.widgets.mouse_down(x, y, button);
        self.events.extend(ev);
        true // 屏幕打开即消费（输入穿透规则）
    }

    fn mouse_up(&mut self, x: f32, y: f32, button: u8) -> bool {
        self.widgets.mouse_up(x, y, button);
        true
    }

    fn mouse_drag(&mut self, x: f32, y: f32, button: u8) -> bool {
        self.widgets.mouse_drag(x, y, button);
        true
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        self.widgets.hover(x, y, self.now_ms);
    }

    fn draw(&mut self, g: &mut mcv_ui::UiGraphics, sprites: Option<&SpriteSheet>) {
        self.now_ms = g.now_ms;
        let w = g.width as f32;
        g.text_centered(
            ui_text(self.lang, Text::Title),
            w * 0.5,
            12.0,
            [1.0, 1.0, 1.0, 1.0],
        );
        self.widgets.draw(g, sprites);
        // 行内两列文本：左动作名 + 右键名，联合缩号防溢出（沿用现网
        // 策略；框架负责的行按钮 label 留空避免双画）。
        for (i, b) in self.widgets.buttons.iter().enumerate() {
            if i >= Action::ALL.len() {
                break;
            }
            let a = Action::ALL[i];
            let capturing = self.binds.state == Capture::Capturing(a);
            let conflict = self.binds.conflict == Some(a);
            let label = action_label(self.lang, a);
            let key_text = self.key_labels[i].as_str();
            let max_w = (b.rect.w - 8.0).max(8.0);
            // 逻辑像素量宽（text_width(_, 1.0) = 字体像素）
            let lw = text_width(label, 1.0);
            let kw = text_width(key_text, 1.0);
            let k = if lw + kw > max_w {
                max_w / (lw + kw)
            } else {
                1.0
            };
            let ty = b.rect.y + (b.rect.h - 8.0 * k) * 0.5;
            let label_col = if capturing {
                [0.7, 0.9, 1.0, 1.0]
            } else if conflict {
                [1.0, 0.55, 0.55, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            g.text_scaled(label, b.rect.x + 4.0, ty, k, label_col);
            g.text_scaled(
                key_text,
                b.rect.x + b.rect.w - 4.0 - kw * k,
                ty,
                k,
                [1.0, 1.0, 1.0, 1.0],
            );
        }
        self.widgets.draw_tooltip(g);
    }

    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_ignores_keys() {
        let mut c = BindCapture::default();
        let mut map = KeyMap::default();
        assert_eq!(c.key_down(&mut map, VKey::KeyT), CaptureEvent::NotCapturing);
        assert_eq!(map.get(Action::Forward), VKey::KeyW);
        assert_eq!(c.state, Capture::Idle);
    }

    #[test]
    fn esc_cancels_and_keeps_binding() {
        let mut c = BindCapture::default();
        let mut map = KeyMap::default();
        c.begin(Action::Forward);
        assert_eq!(c.state, Capture::Capturing(Action::Forward));
        assert_eq!(c.key_down(&mut map, VKey::Escape), CaptureEvent::Cancelled);
        assert_eq!(c.state, Capture::Idle);
        assert_eq!(map.get(Action::Forward), VKey::KeyW);
        assert_eq!(c.conflict, None);
    }

    #[test]
    fn free_key_rebinds() {
        let mut c = BindCapture::default();
        let mut map = KeyMap::default();
        c.begin(Action::Forward);
        assert_eq!(c.key_down(&mut map, VKey::KeyT), CaptureEvent::Bound);
        assert_eq!(map.get(Action::Forward), VKey::KeyT);
        assert_eq!(c.conflict, None);
        // 改绑后旧键不再触发动作，新键生效
        assert_eq!(map.action_for(VKey::KeyW), None);
        assert_eq!(map.action_for(VKey::KeyT), Some(Action::Forward));
    }

    #[test]
    fn same_key_rebind_ok() {
        // conflicting 排除自身：把动作绑回它当前的键不算冲突
        let mut c = BindCapture::default();
        let mut map = KeyMap::default();
        c.begin(Action::Jump);
        assert_eq!(c.key_down(&mut map, VKey::Space), CaptureEvent::Bound);
        assert_eq!(map.get(Action::Jump), VKey::Space);
    }

    #[test]
    fn conflict_keeps_previous_binding() {
        let mut c = BindCapture::default();
        let mut map = KeyMap::default();
        c.begin(Action::Forward);
        // KeyS 已绑 Back → 冲突：保持 Forward=KeyW，红标 Forward
        assert_eq!(c.key_down(&mut map, VKey::KeyS), CaptureEvent::Conflict);
        assert_eq!(map.get(Action::Forward), VKey::KeyW);
        assert_eq!(map.get(Action::Back), VKey::KeyS);
        assert_eq!(c.conflict, Some(Action::Forward));
        assert_eq!(c.state, Capture::Idle);
        // 下一次 begin 清除红标
        c.begin(Action::Forward);
        assert_eq!(c.conflict, None);
    }

    #[test]
    fn bind_ids_align_with_action_all() {
        assert_eq!(BIND_IDS.len(), Action::ALL.len());
        for (i, id) in BIND_IDS.iter().enumerate() {
            assert_eq!(bind_index_of(id), Some(i));
        }
        assert_eq!(bind_index_of("back"), None);
        assert_eq!(bind_index_of("bind16"), None);
    }

    #[test]
    fn remap_survives_text_roundtrip() {
        let mut map = KeyMap::default();
        map.set(Action::Forward, VKey::KeyT);
        let mut restored = KeyMap::from_text(&map.to_text());
        assert_eq!(restored, map);
        // 从文件恢复的表继续参与捕获/冲突判定
        let mut c = BindCapture::default();
        c.begin(Action::Forward);
        assert_eq!(
            c.key_down(&mut restored, VKey::KeyS),
            CaptureEvent::Conflict
        );
        assert_eq!(restored.get(Action::Forward), VKey::KeyT);
    }
}

/// 屏幕骨架测试（mcv_ui 迁移试点）：布局/命中/栈接入/绘制冒烟。
#[cfg(test)]
mod screen_tests {
    use super::*;
    use mcv_ui::widget::UiEvent;
    use mcv_ui::{KeyEvent, Screen, ScreenStack, UiContext, UiGraphics};

    fn ctx() -> UiContext {
        // 640x480 → scale 2 → 逻辑 320x240（MC 基准分辨率）
        UiContext::new(640, 480, 0)
    }

    fn row_center(kb: &KeyBindsScreen, i: usize) -> (f32, f32) {
        let r = kb.widgets.buttons[i].rect;
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    }

    #[test]
    fn layout_builds_rows_and_bottom_buttons() {
        let mut kb = KeyBindsScreen::new(Lang::En);
        kb.on_open(&ctx());
        assert_eq!(kb.widgets.buttons.len(), Action::ALL.len() + 3);
        assert_eq!(kb.widgets.buttons[0].id, 0);
        assert_eq!(
            kb.widgets.buttons[Action::ALL.len() - 1].id,
            Action::ALL.len() - 1
        );
        assert_eq!(kb.widgets.buttons[Action::ALL.len()].id, ID_RESET);
        assert_eq!(kb.widgets.buttons[Action::ALL.len() + 1].id, ID_SAVE);
        assert_eq!(kb.widgets.buttons[Action::ALL.len() + 2].id, ID_BACK);
    }

    #[test]
    fn row_click_captures_and_key_rebinds() {
        let mut kb = KeyBindsScreen::new(Lang::En);
        kb.on_open(&ctx());
        let mut map = KeyMap::default();
        // 点击第 0 行（Forward）
        let (x, y) = row_center(&kb, 0);
        kb.mouse_down(x, y, 0);
        assert_eq!(kb.take_events(), vec![UiEvent::Click { id: 0 }]);
        kb.begin_row(0, &mut map);
        assert_eq!(kb.binds.state, Capture::Capturing(Action::Forward));
        // 等待按键模态：下一个键重绑（消费）
        assert!(kb.key_input(Some(VKey::KeyT), &mut map));
        assert_eq!(map.get(Action::Forward), VKey::KeyT);
        // 未捕获时按键不消费（app.rs 据此走 Esc 关屏）
        assert!(!kb.key_input(Some(VKey::KeyY), &mut map));
        assert!(!kb.key_input(None, &mut map));
    }

    #[test]
    fn refresh_rows_marks_conflict_with_tint() {
        let mut kb = KeyBindsScreen::new(Lang::Zh);
        kb.on_open(&ctx());
        let mut map = KeyMap::default();
        kb.refresh_rows(&map);
        assert_eq!(kb.key_labels[0], "KeyW");
        assert_eq!(kb.widgets.buttons[0].tint, [1.0, 1.0, 1.0, 1.0]);
        // Forward 改绑 KeyS（Back 的键）→ 冲突红标 + tooltip 文本挂上
        kb.binds.begin(Action::Forward);
        assert_eq!(
            kb.binds.key_down(&mut map, VKey::KeyS),
            CaptureEvent::Conflict
        );
        kb.refresh_rows(&map);
        assert_eq!(kb.widgets.buttons[0].tint, [1.0, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn close_cancels_pending_capture() {
        let mut kb = KeyBindsScreen::new(Lang::En);
        kb.binds.begin(Action::Jump);
        kb.on_close();
        assert_eq!(kb.binds.state, Capture::Idle);
    }

    #[test]
    fn draw_smoke_without_sprites() {
        let mut kb = KeyBindsScreen::new(Lang::Zh);
        kb.on_open(&ctx());
        let map = KeyMap::default();
        kb.refresh_rows(&map);
        let mut g = UiGraphics::new(&ctx(), 42);
        kb.draw(&mut g, None);
        assert!(!g.quads.is_empty());
        assert_eq!(kb.now_ms, 42);
    }

    #[test]
    fn screen_stack_routes_and_esc_closes() {
        let mut st = ScreenStack::new();
        let c = ctx();
        st.push(Box::new(KeyBindsScreen::new(Lang::En)), &c);
        assert!(st.is_open());
        // 菜单屏默认 isPauseScreen → 引擎发单机暂停
        assert!(st.paused());
        // 点击行 → 栈路由到顶层 → 事件可取
        let (x, y) = {
            let top = st.top_mut().unwrap();
            let kb = top.as_any().downcast_mut::<KeyBindsScreen>().unwrap();
            row_center(kb, 1)
        };
        assert!(st.mouse_down(x, y, 0));
        let top = st.top_mut().unwrap();
        let kb = top.as_any().downcast_mut::<KeyBindsScreen>().unwrap();
        assert_eq!(kb.take_events(), vec![UiEvent::Click { id: 1 }]);
        // Esc（未捕获）→ 栈统一关屏（on_close 取消捕获）
        assert!(st.key_down(KeyEvent::escape()));
        assert!(!st.is_open());
    }
}
