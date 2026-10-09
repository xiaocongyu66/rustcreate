//! 控件集：Button / Slider / Checkbox / Tooltip。
//!
//! 机制对照（26.1）：
//! - `AbstractWidget`：命中 = `areCoordinatesInRectangle`，点击只认
//!   左键（isValidClickButton :133-135），命中即 `playDownSound` +
//!   `onClick`（mouseClicked :106-121）；悬停在 extractRenderState 时
//!   重算（:56-62，含 scissor containsPoint）；
//! - `AbstractButton`：三态贴图 normal/highlighted/disabled
//!   （WidgetSprites :18-22），文字边距 TEXT_MARGIN=2（:17），播放
//!   `ui.button.click`（playButtonClickSound :165-167）；
//! - `AbstractSliderButton`：DEFAULT_HEIGHT=20（:33）、HANDLE_WIDTH=8
//!   （:35），按下拖动取值 `(x - (getX()+4)) / (width-8)`
//!   （setValueFromMouse :136-138），松开播 down sound（onRelease
//!   :172-176），滑杆按下本身不播（playDownSound 覆写为空 :170-172）；
//! - `Checkbox`：盒尺寸 getBoxSize = 9+8 = 17（:88-90）、盒与文字间距
//!   SPACING=4（:27）、宽度 = 盒 + 4 + 文本宽（getDefaultWidth :86-89），
//!   点击翻转并回调 onValueChange（onPress :92-96）；
//! - `Tooltip`：MAX_WIDTH=170（Tooltip.java:17）；定位翻边 =
//!   `DefaultTooltipPositioner.positionTooltip`（:16-30）：偏移
//!   (12,-12)，右侧超界翻到左侧 `max(x-24-w, 4)`，下方超界贴底。

use crate::context::{Rect, UiGraphics};
use crate::text::{self, TOOLTIP_MAX_WIDTH};
use mcv_render::gui::SpriteSheet;
use mcv_render::text::text_width;

/// 原版按钮宽/高（Button.Builder 常用 bounds；WidgetSprites 200x20）。
pub const BUTTON_W: f32 = 200.0;
pub const BUTTON_H: f32 = 20.0;
/// 文字边距（AbstractButton.java:17 TEXT_MARGIN）。
pub const TEXT_MARGIN: f32 = 2.0;
/// 滑杆默认高（AbstractSliderButton.java:33 DEFAULT_HEIGHT）。
pub const SLIDER_H: f32 = 20.0;
/// 滑杆把手宽（AbstractSliderButton.java:34 HANDLE_WIDTH）。
pub const SLIDER_HANDLE_W: f32 = 8.0;
/// 复选框盒尺寸（Checkbox.java:88-90 getBoxSize = 9 + 8）。
pub const CHECKBOX_BOX: f32 = 17.0;
/// 盒与文字间距（Checkbox.java:27 SPACING）。
pub const CHECKBOX_SPACING: f32 = 4.0;
/// tooltip 定位偏移（DefaultTooltipPositioner.java:17 add(12, -12)）。
pub const TOOLTIP_OFFSET: f32 = 12.0;

/// UI 音效事件（mcv_ui 只发事件不发声；游戏层接 mcv_audio）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiSound {
    /// `SoundEvents.UI_BUTTON_CLICK`（AbstractWidget.java:161-167）。
    ButtonClick,
}

/// 控件事件输出（游戏层消费）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiEvent {
    /// 按钮按下 / 复选框按下（应播 [`UiSound::ButtonClick`]）。
    Click { id: usize },
    /// 滑杆值变化（applyValue；value ∈ 0..1）。
    Value { id: usize, value: f32 },
    /// 滑杆松开（onRelease 播 down sound，AbstractSliderButton:172-176）。
    SliderRelease { id: usize },
    /// 复选框翻转后状态。
    Toggle { id: usize, checked: bool },
}

/// 按钮（AbstractButton 对应物：normal/hover/disabled 三态 + 中央文本）。
#[derive(Clone, Debug)]
pub struct Button {
    pub id: usize,
    pub rect: Rect,
    pub label: String,
    pub enabled: bool,
    /// 无素材回退底色（有素材时作 nine_slice 染色）。
    pub tint: [f32; 4],
}

/// 滑杆（AbstractSliderButton 对应物；value ∈ 0..1）。
#[derive(Clone, Debug)]
pub struct Slider {
    pub id: usize,
    pub rect: Rect,
    pub label: String,
    pub value: f32,
    pub dragging: bool,
    pub enabled: bool,
    pub tint: [f32; 4],
}

/// 复选框（Checkbox 对应物；盒 + 文字，整块命中）。
#[derive(Clone, Debug)]
pub struct Checkbox {
    pub id: usize,
    pub rect: Rect,
    pub label: String,
    pub checked: bool,
    pub enabled: bool,
    pub tint: [f32; 4],
}

/// tooltip 状态（单实例：同屏至多一条，同原版 deferredTooltip）。
#[derive(Clone, Debug, Default)]
struct TooltipState {
    /// id → 文本。
    texts: Vec<(usize, String)>,
    /// 悬停延迟（毫秒；0 = 立即，AbstractWidget.setTooltipDelay 语义）。
    delay_ms: u64,
    /// 悬停起始时刻（换目标时重置）。
    hover_since: u64,
}

impl TooltipState {
    fn text_of(&self, id: usize) -> Option<&str> {
        self.texts
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, t)| t.as_str())
    }
}

/// 控件集：一屏一组；交互纯 CPU、事件出队由屏幕实现转发。
#[derive(Default)]
pub struct WidgetSet {
    pub buttons: Vec<Button>,
    pub sliders: Vec<Slider>,
    pub checkboxes: Vec<Checkbox>,
    tooltip: TooltipState,
    /// 最近鼠标位置（逻辑像素；draw 时算悬停）。
    mouse: (f32, f32),
    /// 当前帧时刻（毫秒；tooltip 延迟判定）。
    now_ms: u64,
}

impl WidgetSet {
    pub fn add_button(&mut self, id: usize, x: f32, y: f32, w: f32, h: f32, label: &str) {
        self.buttons.push(Button {
            id,
            rect: Rect::new(x, y, w, h),
            label: label.to_string(),
            enabled: true,
            tint: [1.0, 1.0, 1.0, 1.0],
        });
    }

    /// 附带回退底色的按钮（无素材时的可读性，对照现网 mc_button accent）。
    #[allow(clippy::too_many_arguments)] // 控件构造参数天然多
    pub fn add_button_accent(
        &mut self,
        id: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: &str,
        tint: [f32; 4],
    ) {
        self.add_button(id, x, y, w, h, label);
        if let Some(b) = self.buttons.last_mut() {
            b.tint = tint;
        }
    }

    pub fn add_slider(&mut self, id: usize, x: f32, y: f32, w: f32, value: f32, label: &str) {
        self.sliders.push(Slider {
            id,
            rect: Rect::new(x, y, w, SLIDER_H),
            label: label.to_string(),
            value: value.clamp(0.0, 1.0),
            dragging: false,
            enabled: true,
            tint: [1.0, 1.0, 1.0, 1.0],
        });
    }

    /// 复选框：宽 = 盒 + SPACING + 文本宽（Checkbox.getDefaultWidth；
    /// 逻辑像素 = 字体像素，见 text_width(s, 1.0)）。
    pub fn add_checkbox(&mut self, id: usize, x: f32, y: f32, checked: bool, label: &str) {
        let w = CHECKBOX_BOX + CHECKBOX_SPACING + text_width(label, 1.0);
        self.checkboxes.push(Checkbox {
            id,
            rect: Rect::new(x, y, w, CHECKBOX_BOX),
            label: label.to_string(),
            checked,
            enabled: true,
            tint: [1.0, 1.0, 1.0, 1.0],
        });
    }

    /// 绑定 tooltip（setTooltip；同 id 重复调用覆盖）。
    pub fn set_tooltip(&mut self, id: usize, text: &str) {
        if let Some(slot) = self.tooltip.texts.iter_mut().find(|(i, _)| *i == id) {
            slot.1 = text.to_string();
        } else {
            self.tooltip.texts.push((id, text.to_string()));
        }
    }

    /// 悬停延迟（setTooltipDelay）。
    pub fn set_tooltip_delay(&mut self, delay_ms: u64) {
        self.tooltip.delay_ms = delay_ms;
    }

    fn id_at(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons
            .iter()
            .find(|b| b.enabled && b.rect.contains(x, y))
            .map(|b| b.id)
            .or_else(|| {
                self.sliders
                    .iter()
                    .find(|s| s.enabled && s.rect.contains(x, y))
                    .map(|s| s.id)
            })
            .or_else(|| {
                self.checkboxes
                    .iter()
                    .find(|c| c.enabled && c.rect.contains(x, y))
                    .map(|c| c.id)
            })
    }

    /// 当前悬停中的控件 id（按钮/滑杆/复选框统一查）。
    pub fn hovered_id(&self) -> Option<usize> {
        let (x, y) = self.mouse;
        self.id_at(x, y)
    }

    /// 悬停更新（mouse_move）：换目标时重置悬停计时（tooltip 延迟基准）。
    pub fn hover(&mut self, x: f32, y: f32, now_ms: u64) {
        let prev = self.hovered_id();
        self.mouse = (x, y);
        self.now_ms = now_ms;
        if prev != self.hovered_id() {
            self.tooltip.hover_since = now_ms;
        }
    }

    /// 按下（AbstractWidget.mouseClicked：命中 + 左键 → 音效事件 + onClick）。
    pub fn mouse_down(&mut self, x: f32, y: f32, button: u8) -> Vec<UiEvent> {
        let mut events = Vec::new();
        if button != 0 {
            return events;
        }
        self.mouse = (x, y);
        // 按钮：首个命中即触发（原版 children 顺序派发，先到先得）
        if let Some(b) = self
            .buttons
            .iter_mut()
            .find(|b| b.enabled && b.rect.contains(x, y))
        {
            events.push(UiEvent::Click { id: b.id });
        }
        if let Some(c) = self
            .checkboxes
            .iter_mut()
            .find(|c| c.enabled && c.rect.contains(x, y))
        {
            c.checked = !c.checked;
            events.push(UiEvent::Toggle {
                id: c.id,
                checked: c.checked,
            });
            events.push(UiEvent::Click { id: c.id });
        }
        if let Some(s) = self
            .sliders
            .iter_mut()
            .find(|s| s.enabled && s.rect.contains(x, y))
        {
            s.dragging = true;
            events.extend(set_slider_value(s, x));
        }
        events
    }

    /// 拖动（AbstractSliderButton.onDrag → setValueFromMouse）。
    pub fn mouse_drag(&mut self, x: f32, y: f32, button: u8) -> Vec<UiEvent> {
        let mut events = Vec::new();
        if button != 0 {
            return events;
        }
        self.mouse = (x, y);
        for s in self.sliders.iter_mut().filter(|s| s.dragging) {
            events.extend(set_slider_value(s, x));
        }
        events
    }

    /// 松开（AbstractSliderButton.onRelease：dragging=false + 音效）。
    pub fn mouse_up(&mut self, x: f32, y: f32, button: u8) -> Vec<UiEvent> {
        self.mouse = (x, y);
        let mut events = Vec::new();
        for s in self.sliders.iter_mut() {
            if s.dragging && button == 0 {
                s.dragging = false;
                events.push(UiEvent::SliderRelease { id: s.id });
            }
        }
        events
    }

    // ---- 绘制 ----

    /// 绘制全部控件（三态贴图 + 文本裁剪；无素材回退纯色块）。
    pub fn draw(&mut self, g: &mut UiGraphics, sprites: Option<&SpriteSheet>) {
        let (mx, my) = self.mouse;
        let s = g.scale;
        for b in self.buttons.iter_mut() {
            let hovered = b.enabled && b.rect.contains(mx, my);
            let sprite = if !b.enabled {
                "button_dis"
            } else if hovered {
                "button_hl"
            } else {
                "button"
            };
            match sprites {
                Some(sprites) => {
                    for q in sprites.nine_slice(
                        sprite,
                        b.rect.x * s,
                        b.rect.y * s,
                        b.rect.w * s,
                        b.rect.h * s,
                        3,
                        s,
                        b.tint,
                    ) {
                        g.push_raw(q);
                    }
                }
                None => {
                    let tint = if b.enabled { b.tint } else { dim(b.tint) };
                    g.fill(b.rect.x, b.rect.y, b.rect.w, b.rect.h, tint);
                }
            }
            draw_clipped_center(g, &b.label, &b.rect, b.enabled);
        }
        for c in self.checkboxes.iter() {
            // 盒（Checkbox:96-118 四态贴图；素材表未收录，程序化盒）
            let box_r = Rect::new(c.rect.x, c.rect.y, CHECKBOX_BOX, CHECKBOX_BOX);
            let border = if c.checked {
                [0.5, 0.75, 0.5, 1.0]
            } else {
                [0.45, 0.45, 0.5, 1.0]
            };
            g.fill(box_r.x, box_r.y, box_r.w, box_r.h, border);
            let inner = if c.checked {
                c.tint
            } else {
                [0.1, 0.1, 0.12, 1.0]
            };
            g.fill(
                box_r.x + 2.0,
                box_r.y + 2.0,
                box_r.w - 4.0,
                box_r.h - 4.0,
                inner,
            );
            // 文字（垂直居中于盒）
            let tx = box_r.x + box_r.w + CHECKBOX_SPACING;
            let ty = c.rect.y + (c.rect.h - 8.0) * 0.5;
            let color = if c.enabled {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.63, 0.63, 0.63, 1.0]
            };
            g.text(&c.label, tx, ty, color);
        }
        for sl in self.sliders.iter_mut() {
            let hovered = sl.enabled && sl.rect.contains(mx, my);
            // 滑轨（getSprite/getHandleSprite :53-61；素材表用按钮三态）
            match sprites {
                Some(sprites) => {
                    let track = if !sl.enabled {
                        "button_dis"
                    } else if hovered || sl.dragging {
                        "button_hl"
                    } else {
                        "button"
                    };
                    for q in sprites.nine_slice(
                        track,
                        sl.rect.x * s,
                        sl.rect.y * s,
                        sl.rect.w * s,
                        sl.rect.h * s,
                        3,
                        s,
                        sl.tint,
                    ) {
                        g.push_raw(q);
                    }
                }
                None => {
                    let tint = if sl.enabled { sl.tint } else { dim(sl.tint) };
                    g.fill(sl.rect.x, sl.rect.y, sl.rect.w, sl.rect.h, tint);
                }
            }
            let hx = sl.rect.x + sl.value * (sl.rect.w - SLIDER_HANDLE_W);
            let handle_hi = sl.enabled && (hovered || sl.dragging);
            g.fill(
                hx,
                sl.rect.y,
                SLIDER_HANDLE_W,
                sl.rect.h,
                if handle_hi {
                    [1.0, 0.98, 0.6, 1.0]
                } else {
                    [0.75, 0.75, 0.8, 1.0]
                },
            );
            draw_clipped_center(g, &sl.label, &sl.rect, sl.enabled);
        }
    }

    /// tooltip 绘制（悬停达标后；返回是否画了）。
    pub fn draw_tooltip(&mut self, g: &mut UiGraphics) -> bool {
        let Some(id) = self.hovered_id() else {
            return false;
        };
        let Some(tip) = self.tooltip.text_of(id).map(|s| s.to_string()) else {
            return false;
        };
        if tip.is_empty() {
            return false;
        }
        if self.now_ms.saturating_sub(self.tooltip.hover_since) < self.tooltip.delay_ms {
            return false;
        }
        let (mx, my) = self.mouse;
        // 折行/量宽用逻辑像素（text_width(_, 1.0) = 字体像素）
        let lines = text::wrap(&tip, TOOLTIP_MAX_WIDTH, 1.0);
        let w = lines
            .iter()
            .map(|l| text_width(l, 1.0))
            .fold(0.0f32, f32::max)
            + 8.0;
        let h = lines.len() as f32 * 10.0;
        let (tx, ty) = tooltip_position(g.width as f32, g.height as f32, mx, my, w, h);
        // 背景 + 边框（TooltipRenderUtil 配色的近似移植：
        // 外沿 0x505000FF、底 0xF0100010）
        g.fill(tx - 1.0, ty - 1.0, w + 2.0, h + 2.0, [0.31, 0.31, 0.0, 1.0]);
        g.fill(tx, ty, w, h, [0.06, 0.0, 0.06, 0.94]);
        for (i, line) in lines.iter().enumerate() {
            g.text(
                line,
                tx + 4.0,
                ty + 1.0 + i as f32 * 10.0,
                [1.0, 1.0, 1.0, 1.0],
            );
        }
        true
    }
}

/// setValueFromMouse：(x - (getX() + 4)) / (width - 8)，clamp 0..1；
/// 值变化才发事件（AbstractSliderButton.setValue 的 oldValue != value 分支）。
fn set_slider_value(s: &mut Slider, x: f32) -> Vec<UiEvent> {
    let inner = (s.rect.w - SLIDER_HANDLE_W).max(1.0);
    let v = ((x - (s.rect.x + SLIDER_HANDLE_W * 0.5)) / inner).clamp(0.0, 1.0);
    if (v - s.value).abs() > f32::EPSILON {
        s.value = v;
        vec![UiEvent::Value { id: s.id, value: v }]
    } else {
        Vec::new()
    }
}

fn dim(c: [f32; 4]) -> [f32; 4] {
    [c[0] * 0.5, c[1] * 0.5, c[2] * 0.5, c[3]]
}

/// 居中文本 + 超宽裁剪（extractScrollingStringOverContents 的静态版：
/// 原版超宽走滚动字幕，本框架先落静态裁剪，滚动列入迁移清单）。
fn draw_clipped_center(g: &mut UiGraphics, label: &str, rect: &Rect, enabled: bool) {
    let color = if enabled {
        [1.0, 1.0, 1.0, 1.0]
    } else {
        [0.63, 0.63, 0.63, 1.0] // 原版 disabled 文本灰 0xA0A0A0
    };
    let max_w = (rect.w - TEXT_MARGIN * 2.0).max(1.0);
    let w = text_width(label, 1.0); // 逻辑像素 = 字体像素
    let ty = rect.y + (rect.h - 8.0) * 0.5;
    if w <= max_w {
        g.text(label, rect.x + rect.w * 0.5 - w * 0.5, ty, color);
    } else {
        // 超宽：截头左对齐（text::clip_head，中文/长标签防溢出）
        let clipped = text::clip_head(label, max_w, 1.0);
        g.text(clipped, rect.x + TEXT_MARGIN, ty, color);
    }
}

/// tooltip 定位（DefaultTooltipPositioner.positionTooltip 移植）：
/// 偏移 (12,-12)；右侧超界翻到光标左侧 `max(x-24-w, 4)`；
/// 下方超界 `y = 屏高 - (h + 3)`。
pub fn tooltip_position(
    screen_w: f32,
    screen_h: f32,
    x: f32,
    y: f32,
    tooltip_w: f32,
    tooltip_h: f32,
) -> (f32, f32) {
    let mut result = (x + TOOLTIP_OFFSET, y - TOOLTIP_OFFSET);
    if result.0 + tooltip_w > screen_w {
        result.0 = (result.0 - 2.0 * TOOLTIP_OFFSET - tooltip_w).max(4.0);
    }
    let padded_h = tooltip_h + 3.0;
    if result.1 + padded_h > screen_h {
        result.1 = screen_h - padded_h;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_graphics;

    fn set() -> WidgetSet {
        let mut ws = WidgetSet::default();
        ws.add_button(1, 0.0, 0.0, BUTTON_W, BUTTON_H, "Test");
        ws
    }

    #[test]
    fn button_clicks_emit_once() {
        let mut ws = set();
        let ev = ws.mouse_down(10.0, 10.0, 0);
        assert_eq!(ev, vec![UiEvent::Click { id: 1 }]);
        // 右键不触发（isValidClickButton 只认左键）
        assert!(ws.mouse_down(10.0, 10.0, 1).is_empty());
        // 空白处不触发
        assert!(ws.mouse_down(150.0, 150.0, 0).is_empty());
    }

    #[test]
    fn hover_tracks_widgets() {
        let mut ws = set();
        ws.hover(10.0, 10.0, 5);
        assert_eq!(ws.hovered_id(), Some(1));
        ws.hover(150.0, 150.0, 6);
        assert_eq!(ws.hovered_id(), None);
    }

    #[test]
    fn tooltip_position_flips_at_edges() {
        // 屏 320x240；光标 (300, 100)，tooltip 宽 60 → 右侧超界翻左边
        let (x, y) = tooltip_position(320.0, 240.0, 300.0, 100.0, 60.0, 20.0);
        assert_eq!(x, (300.0 + 12.0 - 24.0 - 60.0).max(4.0)); // 228
        assert_eq!(y, 100.0 - 12.0);
        // 底部超界 → 贴底
        let (x, y) = tooltip_position(320.0, 240.0, 100.0, 235.0, 60.0, 20.0);
        assert_eq!(x, 112.0);
        assert_eq!(y, 240.0 - 23.0);
        // 窄屏：翻边后 clamp 4
        let (x, _) = tooltip_position(30.0, 240.0, 2.0, 2.0, 60.0, 20.0);
        assert_eq!(x, 4.0);
    }

    #[test]
    fn slider_value_from_mouse_and_events() {
        let mut ws = WidgetSet::default();
        ws.add_slider(7, 0.0, 0.0, 108.0, 0.0, "Sens");
        // setValueFromMouse：(x-4)/(108-8)；x=54 → 0.5、x=104 → 1.0
        let ev = ws.mouse_down(54.0, 10.0, 0);
        assert_eq!(ev, vec![UiEvent::Value { id: 7, value: 0.5 }]);
        let ev = ws.mouse_drag(104.0, 10.0, 0);
        assert_eq!(ev, vec![UiEvent::Value { id: 7, value: 1.0 }]);
        let ev = ws.mouse_up(104.0, 10.0, 0);
        assert_eq!(ev, vec![UiEvent::SliderRelease { id: 7 }]);
        // 松开后拖动不再更新
        assert!(ws.mouse_drag(10.0, 10.0, 0).is_empty());
    }

    #[test]
    fn slider_clamps() {
        let mut ws = WidgetSet::default();
        ws.add_slider(7, 0.0, 0.0, 108.0, 0.5, "Sens");
        ws.mouse_down(-50.0, 10.0, 0);
        assert!((ws.sliders[0].value - 0.0).abs() < 1e-6);
        ws.mouse_up(-50.0, 10.0, 0);
        ws.mouse_down(999.0, 10.0, 0);
        assert!((ws.sliders[0].value - 1.0).abs() < 1e-6);
    }

    #[test]
    fn checkbox_toggles_and_reports() {
        let mut ws = WidgetSet::default();
        ws.add_checkbox(3, 0.0, 0.0, false, "Clouds");
        let ev = ws.mouse_down(5.0, 5.0, 0);
        assert!(ev.contains(&UiEvent::Toggle {
            id: 3,
            checked: true
        }));
        assert!(ev.contains(&UiEvent::Click { id: 3 }));
        let ev = ws.mouse_down(5.0, 5.0, 0);
        assert!(ev.contains(&UiEvent::Toggle {
            id: 3,
            checked: false
        }));
        assert!(!ws.checkboxes[0].checked);
    }

    #[test]
    fn tooltip_respects_delay() {
        let mut ws = set();
        ws.set_tooltip(1, "冲突：已绑定 后退");
        ws.set_tooltip_delay(300);
        // 未悬停 → 不画
        assert!(!ws.draw_tooltip(&mut test_graphics(0)));
        // 悬停但未到延迟 → 不画
        ws.hover(10.0, 10.0, 0);
        assert!(!ws.draw_tooltip(&mut test_graphics(299)));
        // 到延迟 → 画
        assert!(ws.draw_tooltip(&mut test_graphics(300)));
    }

    #[test]
    fn tooltip_without_binding_not_drawn() {
        let mut ws = set();
        ws.hover(10.0, 10.0, 0);
        assert!(!ws.draw_tooltip(&mut test_graphics(10)));
    }
}
