//! winit `KeyCode` → [`VKey`] 转换表。显式逐变体匹配，不依赖 Debug 名字符串
//! （winit 用 `BracketLeft`/`Backquote`，[`VKey`] 用 `LeftBracket`/`Grave`）。
//! 鼠标键（`MouseLeft` 等）不是 KeyCode，由鼠标事件单独喂给 KeyMap。

use mcv_game::keymap::VKey;
use winit::keyboard::KeyCode;

/// 数字键 1..9 → 快捷栏槽位 0..8（MC 中是 9 个独立键位，本引擎键位表
/// 单动作单键容不下，先保持硬编码直绑）。
pub fn hotbar_slot(code: KeyCode) -> Option<usize> {
    let slot = match code {
        KeyCode::Digit1 => 0,
        KeyCode::Digit2 => 1,
        KeyCode::Digit3 => 2,
        KeyCode::Digit4 => 3,
        KeyCode::Digit5 => 4,
        KeyCode::Digit6 => 5,
        KeyCode::Digit7 => 6,
        KeyCode::Digit8 => 7,
        KeyCode::Digit9 => 8,
        _ => return None,
    };
    Some(slot)
}

/// 快捷栏滚动选槽（审计 D-6；26.1 `ScrollWheelHandler.java:10-49`）。
///
/// 原版语义逐条对账：
/// - `onMouseScroll:11-19`：与累计值**符号相反**的新事件先把对应轴累计清零
///   （反向即重开，不做抵消）；
/// - `:21-31`：累计取整后扣掉整数部分，余数留到下次（触控板小步长滚动能攒够
///   一格才换槽）；两轴都取不到整数 → `(0,0)` 表示本次不换槽；
/// - `getNextScrollWheelSelection:33-48`：按 `signum(wheel)` **减**一步，
///   负数加满 limit 回绕、≥limit 减 limit 回绕。
/// - 取轴：`MouseHandler.java:209-210`——y 为 0 时退 x，且 x 取负
///   （横向滚轮方向与纵向相反）。
///
/// 26.1 的旁观者菜单/飞行速度分支（`MouseHandler.java:211-217`）本仓无对应
/// 机制（无旁观模式，见交互审计 I-35），不在此实现。
#[derive(Default)]
pub struct ScrollWheelSelection {
    acc_x: f64,
    acc_y: f64,
}

impl ScrollWheelSelection {
    /// 累计并提取整格滚动量。返回 `(wheel_x, wheel_y)`，`(0,0)` = 不足一格。
    pub fn on_scroll(&mut self, dx: f64, dy: f64) -> (i32, i32) {
        if self.acc_x != 0.0 && dx.signum() != self.acc_x.signum() {
            self.acc_x = 0.0;
        }
        if self.acc_y != 0.0 && dy.signum() != self.acc_y.signum() {
            self.acc_y = 0.0;
        }
        self.acc_x += dx;
        self.acc_y += dy;
        let wheel_x = self.acc_x.trunc() as i32;
        let wheel_y = self.acc_y.trunc() as i32;
        if wheel_x == 0 && wheel_y == 0 {
            return (0, 0);
        }
        self.acc_x -= wheel_x as f64;
        self.acc_y -= wheel_y as f64;
        (wheel_x, wheel_y)
    }

    /// 原版 `getNextScrollWheelSelection`：反向一步 + limit 回绕。
    pub fn next_selection(wheel: i32, current: usize, limit: usize) -> usize {
        let mut cur = current as i64 - wheel.signum() as i64;
        cur = cur.max(-1);
        while cur < 0 {
            cur += limit as i64;
        }
        while cur >= limit as i64 {
            cur -= limit as i64;
        }
        cur as usize
    }

    /// 一次滚动事件 → 新槽位；不足一格返回 `None`。
    /// `wheel_x == 0` 时取 y，否则取 `-wheel_x`（原版取轴次序）。
    pub fn select(&mut self, dx: f64, dy: f64, current: usize, limit: usize) -> Option<usize> {
        let (wx, wy) = self.on_scroll(dx, dy);
        if wx == 0 && wy == 0 {
            return None;
        }
        let wheel = if wy == 0 { -wx } else { wy };
        Some(Self::next_selection(wheel, current, limit))
    }
}

pub fn vkey_of(code: KeyCode) -> Option<VKey> {
    let vk = match code {
        KeyCode::KeyA => VKey::KeyA,
        KeyCode::KeyB => VKey::KeyB,
        KeyCode::KeyC => VKey::KeyC,
        KeyCode::KeyD => VKey::KeyD,
        KeyCode::KeyE => VKey::KeyE,
        KeyCode::KeyF => VKey::KeyF,
        KeyCode::KeyG => VKey::KeyG,
        KeyCode::KeyH => VKey::KeyH,
        KeyCode::KeyI => VKey::KeyI,
        KeyCode::KeyJ => VKey::KeyJ,
        KeyCode::KeyK => VKey::KeyK,
        KeyCode::KeyL => VKey::KeyL,
        KeyCode::KeyM => VKey::KeyM,
        KeyCode::KeyN => VKey::KeyN,
        KeyCode::KeyO => VKey::KeyO,
        KeyCode::KeyP => VKey::KeyP,
        KeyCode::KeyQ => VKey::KeyQ,
        KeyCode::KeyR => VKey::KeyR,
        KeyCode::KeyS => VKey::KeyS,
        KeyCode::KeyT => VKey::KeyT,
        KeyCode::KeyU => VKey::KeyU,
        KeyCode::KeyV => VKey::KeyV,
        KeyCode::KeyW => VKey::KeyW,
        KeyCode::KeyX => VKey::KeyX,
        KeyCode::KeyY => VKey::KeyY,
        KeyCode::KeyZ => VKey::KeyZ,
        KeyCode::Digit0 => VKey::Digit0,
        KeyCode::Digit1 => VKey::Digit1,
        KeyCode::Digit2 => VKey::Digit2,
        KeyCode::Digit3 => VKey::Digit3,
        KeyCode::Digit4 => VKey::Digit4,
        KeyCode::Digit5 => VKey::Digit5,
        KeyCode::Digit6 => VKey::Digit6,
        KeyCode::Digit7 => VKey::Digit7,
        KeyCode::Digit8 => VKey::Digit8,
        KeyCode::Digit9 => VKey::Digit9,
        KeyCode::F1 => VKey::F1,
        KeyCode::F2 => VKey::F2,
        KeyCode::F3 => VKey::F3,
        KeyCode::F4 => VKey::F4,
        KeyCode::F5 => VKey::F5,
        KeyCode::F6 => VKey::F6,
        KeyCode::F7 => VKey::F7,
        KeyCode::F8 => VKey::F8,
        KeyCode::F9 => VKey::F9,
        KeyCode::F10 => VKey::F10,
        KeyCode::F11 => VKey::F11,
        KeyCode::F12 => VKey::F12,
        KeyCode::Space => VKey::Space,
        KeyCode::Comma => VKey::Comma,
        KeyCode::Period => VKey::Period,
        KeyCode::Slash => VKey::Slash,
        KeyCode::Semicolon => VKey::Semicolon,
        KeyCode::Quote => VKey::Apostrophe,
        KeyCode::Minus => VKey::Minus,
        KeyCode::Equal => VKey::Equal,
        KeyCode::BracketLeft => VKey::LeftBracket,
        KeyCode::BracketRight => VKey::RightBracket,
        KeyCode::Backslash => VKey::Backslash,
        KeyCode::Backquote => VKey::Grave,
        KeyCode::Enter => VKey::Enter,
        KeyCode::Backspace => VKey::Backspace,
        KeyCode::Tab => VKey::Tab,
        KeyCode::Escape => VKey::Escape,
        KeyCode::Insert => VKey::Insert,
        KeyCode::Delete => VKey::Delete,
        KeyCode::Home => VKey::Home,
        KeyCode::End => VKey::End,
        KeyCode::PageUp => VKey::PageUp,
        KeyCode::PageDown => VKey::PageDown,
        KeyCode::ArrowLeft => VKey::ArrowLeft,
        KeyCode::ArrowUp => VKey::ArrowUp,
        KeyCode::ArrowRight => VKey::ArrowRight,
        KeyCode::ArrowDown => VKey::ArrowDown,
        KeyCode::ShiftLeft => VKey::ShiftLeft,
        KeyCode::ShiftRight => VKey::ShiftRight,
        KeyCode::ControlLeft => VKey::ControlLeft,
        KeyCode::ControlRight => VKey::ControlRight,
        KeyCode::AltLeft => VKey::AltLeft,
        KeyCode::AltRight => VKey::AltRight,
        _ => return None,
    };
    Some(vk)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-6（行为）：一格滚动换一槽且**反向**步进（getNextScrollWheelSelection
    /// `current - signum`）：上滚 0→8、下滚 8→0 回绕（原版 :33-48 while 回绕）。
    #[test]
    fn wheel_steps_backwards_and_wraps() {
        let mut s = ScrollWheelSelection::default();
        assert_eq!(
            s.select(0.0, 1.0, 0, 9),
            Some(8),
            "0 槽上滚 → 8（负方向回绕）"
        );
        let mut s = ScrollWheelSelection::default();
        assert_eq!(
            s.select(0.0, -1.0, 8, 9),
            Some(0),
            "8 槽下滚 → 0（正方向回绕）"
        );
        // 连续下滚 9 格回原点。
        let mut cur = 4;
        for _ in 0..9 {
            cur = ScrollWheelSelection::next_selection(-1, cur, 9);
        }
        assert_eq!(cur, 4);
    }

    /// 触控板亚单位滚动：不足一格不换槽，余量累计到整格才换（onMouseScroll
    /// :21-31 取整扣余数）；**反向**事件把累计清零重开（:11-19 signum 门），
    /// 不做抵消。
    #[test]
    fn fractional_accumulates_until_whole_notch_then_resets_on_reversal() {
        let mut s = ScrollWheelSelection::default();
        assert_eq!(s.select(0.0, 0.4, 0, 9), None, "0.4 格不足");
        assert_eq!(s.select(0.0, 0.4, 0, 9), None, "0.8 格仍不足");
        assert_eq!(s.select(0.0, 0.4, 0, 9), Some(8), "1.2 格 → 换一槽，余 0.2");
        // 反向：+0.6 后接 -1.0 —— 不清累计应得 -0.4（不足格不换）；
        // 原版清累计后 -1.0 整格照换。
        let mut s = ScrollWheelSelection::default();
        assert_eq!(s.select(0.0, 0.6, 0, 9), None);
        assert_eq!(
            s.select(0.0, -1.0, 0, 9),
            Some(1),
            "反向清零后 -1.0 整格生效"
        );
    }

    /// 横向滚轮取轴语义（MouseHandler.java:209-210）：y==0 时取 `-x`，
    /// 即横向滚动的方向与纵向相反。
    #[test]
    fn horizontal_axis_is_negated() {
        let mut s = ScrollWheelSelection::default();
        assert_eq!(s.select(1.0, 0.0, 0, 9), Some(1), "x=+1 → wheel=-1 → 槽 +1");
        let mut s = ScrollWheelSelection::default();
        assert_eq!(
            s.select(-1.0, 0.0, 0, 9),
            Some(8),
            "x=-1 → wheel=+1 → 槽 -1"
        );
        // y 优先于 x。
        let mut s = ScrollWheelSelection::default();
        assert_eq!(s.select(1.0, 1.0, 4, 9), Some(3), "两轴同滚取 y");
    }
}
