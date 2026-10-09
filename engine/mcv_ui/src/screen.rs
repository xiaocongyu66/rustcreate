//! 屏幕栈：`Screen` trait + `ScreenStack`（push/pop/replace、输入路由、
//! 暂停语义）。
//!
//! 机制对照（26.1）：
//! - `Screen`（screens/Screen.java）：`shouldCloseOnEsc`（:191-193，默认
//!   true）→ Esc 即 `onClose`（keyPressed :119-125 先给子类，吃掉才轮
//!   到默认关屏）；`removed()`（:368-370）= 关屏回调；`isPauseScreen`
//!   （:426-428，默认 true；AbstractContainerScreen 覆写 false
//!   :640-642）；`init/resize`（:323-340、:446-450）= 布局入口；
//! - 单机暂停（Minecraft.java:1300-1302）：
//!   `pause = hasSingleplayerServer && (screen != null && screen.isPauseScreen())`
//!   ——屏幕栈在顶层屏上给出同样的信号，**由引擎（游戏层）发暂停**；
//! - 输入穿透规则：屏幕非 null 期间所有鼠标/键盘事件都进 Screen，
//!   游戏输入不派发（MouseHandler/KeyHandler 的 screen==null 短路）；
//!   屏幕为空时事件原样透传（本栈方法返回 false）；
//! - 关闭即消费：Esc 关屏的那次按键不再传给游戏（keyPressed 返回
//!   true）。

use crate::context::{UiContext, UiGraphics};
use mcv_render::gui::SpriteSheet;

/// 键码约定：沿用 GLFW 键值（Escape=256、Enter=257、Tab=258…），与
/// winit `PhysicalKey::Code` 的 GLFW 同源；游戏层映射自有键表。
pub mod key {
    pub const ESC: u32 = 256;
    pub const RETURN: u32 = 257;
    pub const TAB: u32 = 258;
    pub const LEFT: u32 = 263;
    pub const RIGHT: u32 = 262;
    pub const DOWN: u32 = 264;
    pub const UP: u32 = 265;
}

/// 键盘事件（mcv_ui 不依赖 winit；游戏层在事件入口换算）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: u32,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl KeyEvent {
    pub fn new(code: u32) -> Self {
        Self {
            code,
            shift: false,
            ctrl: false,
            alt: false,
        }
    }

    pub fn escape() -> Self {
        Self::new(key::ESC)
    }
}

/// 一屏之实现（对照 Screen.java）。坐标一律逻辑像素。
pub trait Screen {
    /// 暂停语义（Screen.java:426-428；容器屏覆写 false，见
    /// AbstractContainerScreen.java:640-642）。
    fn is_pause_screen(&self) -> bool {
        true
    }

    /// Esc 关屏许可（Screen.java:191-193）。
    fn should_close_on_esc(&self) -> bool {
        true
    }

    /// 打开/首次布局（Screen.init :323-340）。
    fn on_open(&mut self, _ctx: &UiContext) {}

    /// 窗口/scale 变化重建布局（Screen.resize → repositionElements）。
    fn on_resize(&mut self, _ctx: &UiContext) {}

    /// 关屏回调（Screen.removed :368-370）。
    fn on_close(&mut self) {}

    /// 键盘：返回 true = 已消费（不再走默认 Esc 关屏）。
    fn on_key(&mut self, _e: KeyEvent) -> bool {
        false
    }

    fn mouse_down(&mut self, _x: f32, _y: f32, _button: u8) -> bool {
        false
    }

    fn mouse_up(&mut self, _x: f32, _y: f32, _button: u8) -> bool {
        false
    }

    fn mouse_drag(&mut self, _x: f32, _y: f32, _button: u8) -> bool {
        false
    }

    fn mouse_move(&mut self, _x: f32, _y: f32) {}

    /// 绘制（extractRenderState；sprites 缺失时自选回退画法）。
    fn draw(&mut self, _g: &mut UiGraphics, _sprites: Option<&SpriteSheet>) {}

    /// 事件/状态出队通道（游戏层 downcast 取具体屏，见 as_any 调用方）。
    fn as_any(&mut self) -> &mut dyn std::any::Any;
}

/// 屏幕栈：顶层独占输入（栈非空时游戏键不派发），draw 只画顶层
/// （vanilla setScreen 是单槽，栈语义 = 顶层屏完全遮蔽下层）。
#[derive(Default)]
pub struct ScreenStack {
    screens: Vec<Box<dyn Screen>>,
}

impl ScreenStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        !self.screens.is_empty()
    }

    pub fn len(&self) -> usize {
        self.screens.len()
    }

    /// 栈是否为空（clippy::len_without_is_empty 配对）。
    pub fn is_empty(&self) -> bool {
        self.screens.is_empty()
    }

    /// 暂停信号（Minecraft.java:1300-1302 的 screen 部分）：顶层屏
    /// isPauseScreen() → 单机暂停由引擎发。
    pub fn paused(&self) -> bool {
        self.top().is_some_and(|s| s.is_pause_screen())
    }

    pub fn top(&self) -> Option<&dyn Screen> {
        // match 形式保证 as_ref 在面向返回类型的 coercion site 上
        //（闭包内会让 trait object 生存期误定为 'static）
        match self.screens.last() {
            Some(b) => Some(b.as_ref()),
            None => None,
        }
    }

    /// 顶层可变借用（`+ '_`：trait object 绑定 self 借用而非 'static；
    /// &mut 不可协变缩界，须显式标注）。
    pub fn top_mut(&mut self) -> Option<&mut (dyn Screen + '_)> {
        match self.screens.last_mut() {
            Some(b) => Some(b.as_mut()),
            None => None,
        }
    }

    /// 压栈（on_open 布局一次）。
    pub fn push(&mut self, mut s: Box<dyn Screen>, ctx: &UiContext) {
        s.on_open(ctx);
        self.screens.push(s);
    }

    /// 弹栈（on_close 回调）。
    pub fn pop(&mut self) -> Option<Box<dyn Screen>> {
        let mut s = self.screens.pop()?;
        s.on_close();
        Some(s)
    }

    /// 顶替顶层（子菜单返回场景：pop + push，不闪空帧）。
    pub fn replace(&mut self, s: Box<dyn Screen>, ctx: &UiContext) {
        self.pop();
        self.push(s, ctx);
    }

    /// 清空（退出游戏/回主菜单）。
    pub fn clear(&mut self) {
        while self.pop().is_some() {}
    }

    /// 窗口/scale 变化 → 顶层重建布局。
    pub fn resize(&mut self, ctx: &UiContext) {
        if let Some(t) = self.top_mut() {
            t.on_resize(ctx);
        }
    }

    /// 键盘路由：先顶层 on_key，未吃且为 Esc 且允许 → on_close + pop；
    /// 栈非空时恒消费（不透传游戏）。
    pub fn key_down(&mut self, e: KeyEvent) -> bool {
        let Some(top) = self.top_mut() else {
            return false;
        };
        if top.on_key(e) {
            return true;
        }
        if e.code == key::ESC && top.should_close_on_esc() {
            self.pop();
            return true;
        }
        true
    }

    pub fn mouse_down(&mut self, x: f32, y: f32, button: u8) -> bool {
        match self.top_mut() {
            Some(t) => {
                t.mouse_down(x, y, button);
                true
            }
            None => false,
        }
    }

    pub fn mouse_up(&mut self, x: f32, y: f32, button: u8) -> bool {
        match self.top_mut() {
            Some(t) => {
                t.mouse_up(x, y, button);
                true
            }
            None => false,
        }
    }

    pub fn mouse_drag(&mut self, x: f32, y: f32, button: u8) -> bool {
        match self.top_mut() {
            Some(t) => {
                t.mouse_drag(x, y, button);
                true
            }
            None => false,
        }
    }

    pub fn mouse_move(&mut self, x: f32, y: f32) -> bool {
        match self.top_mut() {
            Some(t) => {
                t.mouse_move(x, y);
                true
            }
            None => false,
        }
    }

    /// 绘制顶层（栈空为无操作）。
    pub fn draw(&mut self, g: &mut UiGraphics, sprites: Option<&SpriteSheet>) {
        if let Some(t) = self.top_mut() {
            t.draw(g, sprites);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_graphics;

    /// 记录生命周期的测试屏：可选吃 Esc、可选暂停、可记录输入。
    struct Probe {
        name: &'static str,
        log: Vec<&'static str>,
        eat_esc: bool,
        eat_key: bool,
        pause: bool,
    }

    impl Probe {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                log: Vec::new(),
                eat_esc: false,
                eat_key: false,
                pause: true,
            }
        }
    }

    impl Screen for Probe {
        fn is_pause_screen(&self) -> bool {
            self.pause
        }
        fn should_close_on_esc(&self) -> bool {
            !self.eat_esc
        }
        fn on_open(&mut self, _ctx: &UiContext) {
            self.log.push("open");
        }
        fn on_resize(&mut self, _ctx: &UiContext) {
            self.log.push("resize");
        }
        fn on_close(&mut self) {
            self.log.push("close");
        }
        fn on_key(&mut self, _e: KeyEvent) -> bool {
            if self.eat_key {
                self.log.push("key");
                true
            } else {
                false
            }
        }
        fn mouse_down(&mut self, _x: f32, _y: f32, _b: u8) -> bool {
            self.log.push("click");
            true
        }
        fn draw(&mut self, _g: &mut UiGraphics, _s: Option<&SpriteSheet>) {
            self.log.push("draw");
        }
        fn as_any(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn push_pop_replace_calls_lifecycle() {
        let ctx = UiContext::default();
        let mut st = ScreenStack::new();
        st.push(Box::new(Probe::new("a")), &ctx);
        st.replace(Box::new(Probe::new("b")), &ctx);
        assert_eq!(st.len(), 1);
        let mut popped = st.pop().unwrap();
        let p = popped.as_any().downcast_ref::<Probe>().unwrap();
        assert_eq!(p.name, "b");
        // b: open；replace 时 a close、b open
        assert_eq!(p.log, vec!["open"]);
        assert!(st.pop().is_none());
    }

    #[test]
    fn esc_closes_unless_should_close_on_esc_false() {
        let ctx = UiContext::default();
        let mut st = ScreenStack::new();
        st.push(Box::new(Probe::new("a")), &ctx);
        assert!(st.key_down(KeyEvent::escape()));
        assert!(!st.is_open());

        let mut p = Probe::new("b");
        p.eat_esc = true; // shouldCloseOnEsc = false（如加载画面）
        st.push(Box::new(p), &ctx);
        assert!(st.key_down(KeyEvent::escape())); // 被消费但不关
        assert!(st.is_open());
    }

    #[test]
    fn empty_stack_passes_through() {
        let mut st = ScreenStack::new();
        assert!(!st.key_down(KeyEvent::escape()));
        assert!(!st.mouse_down(1.0, 1.0, 0));
        assert!(!st.mouse_move(1.0, 1.0));
        assert!(!st.paused());
    }

    #[test]
    fn pause_semantics_follow_top_screen() {
        let ctx = UiContext::default();
        let mut st = ScreenStack::new();
        let mut container = Probe::new("container");
        container.pause = false; // AbstractContainerScreen.isPauseScreen = false
        st.push(Box::new(container), &ctx);
        assert!(!st.paused());
        let mut menu = Probe::new("menu");
        menu.pause = true;
        st.push(Box::new(menu), &ctx);
        assert!(st.paused()); // 顶层（KeyBinds/暂停菜单）→ 引擎发暂停
    }

    #[test]
    fn input_routes_to_top_only() {
        let ctx = UiContext::default();
        let mut st = ScreenStack::new();
        st.push(Box::new(Probe::new("a")), &ctx);
        st.push(Box::new(Probe::new("b")), &ctx);
        assert!(st.mouse_down(5.0, 5.0, 0));
        // 只顶层收到 click
        let mut b = st.pop().unwrap();
        let p = b.as_any().downcast_ref::<Probe>().unwrap();
        assert_eq!(p.log, vec!["open", "click"]);
        let mut a = st.pop().unwrap();
        let p = a.as_any().downcast_ref::<Probe>().unwrap();
        assert_eq!(p.log, vec!["open", "close"]);
    }

    #[test]
    fn draw_goes_to_top_and_resize_notifies() {
        let ctx = UiContext::default();
        let mut st = ScreenStack::new();
        st.push(Box::new(Probe::new("a")), &ctx);
        let mut g = test_graphics(0);
        st.draw(&mut g, None);
        let mut ctx2 = UiContext::new(1280, 960, 0);
        ctx2.update(1280, 960, 0);
        st.resize(&ctx2);
        let mut a = st.pop().unwrap();
        let p = a.as_any().downcast_ref::<Probe>().unwrap();
        assert_eq!(p.log, vec!["open", "draw", "resize", "close"]);
    }

    #[test]
    fn on_key_consumed_prevents_close() {
        let ctx = UiContext::default();
        let mut st = ScreenStack::new();
        let mut p = Probe::new("capture");
        p.eat_key = true; // 等待按键模态：吃掉 Esc（取消捕获）
        st.push(Box::new(p), &ctx);
        assert!(st.key_down(KeyEvent::escape()));
        assert!(st.is_open()); // Esc 被捕获逻辑消费，不关屏
    }
}
