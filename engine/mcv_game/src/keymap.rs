//! 键盘按键重映射：纯逻辑层，不依赖 winit。
//!
//! `VKey` 是平台无关的物理键，`winit_name()` 用 winit 0.30 `KeyCode`/`MouseButton`
//! 的文档名（"KeyW"、"Space"、"MouseLeft"…），主控在事件入口做一次名称转换。

use std::collections::HashMap;
use std::fmt;

/// 可重映射的动作。name 即持久化文本里的动作名。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Sneak,
    Sprint,
    FlyToggle,
    Inventory,
    Debug,
    PickBlock,
    Screenshot,
    Pause,
    Mine,
    Place,
    HotbarSelect,
}

impl Action {
    pub const ALL: [Action; 16] = [
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Jump,
        Action::Sneak,
        Action::Sprint,
        Action::FlyToggle,
        Action::Inventory,
        Action::Debug,
        Action::PickBlock,
        Action::Screenshot,
        Action::Pause,
        Action::Mine,
        Action::Place,
        Action::HotbarSelect,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Action::Forward => "forward",
            Action::Back => "back",
            Action::Left => "left",
            Action::Right => "right",
            Action::Jump => "jump",
            Action::Sneak => "sneak",
            Action::Sprint => "sprint",
            Action::FlyToggle => "fly_toggle",
            Action::Inventory => "inventory",
            Action::Debug => "debug",
            Action::PickBlock => "pick_block",
            Action::Screenshot => "screenshot",
            Action::Pause => "pause",
            Action::Mine => "mine",
            Action::Place => "place",
            Action::HotbarSelect => "hotbar_select",
        }
    }

    /// UI 显示用（首字母大写，MC 26.1 无法在持久化里放空格，故显示层单独给）。
    pub fn display(self) -> &'static str {
        match self {
            Action::Forward => "Forward",
            Action::Back => "Back",
            Action::Left => "Left",
            Action::Right => "Right",
            Action::Jump => "Jump",
            Action::Sneak => "Sneak",
            Action::Sprint => "Sprint",
            Action::FlyToggle => "Toggle Fly",
            Action::Inventory => "Open/Close Inventory",
            Action::Debug => "Toggle Debug Screen",
            Action::PickBlock => "Pick Block",
            Action::Screenshot => "Take Screenshot",
            Action::Pause => "Inventory and Pause",
            Action::Mine => "Attack / Break Block",
            Action::Place => "Use Item / Place Block",
            Action::HotbarSelect => "Select Hotbar Slot",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 平台无关物理键（仅覆盖会被绑定的键，含鼠标键——MC 同样允许鼠标绑定）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VKey {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Space,
    Comma,
    Period,
    Slash,
    Semicolon,
    Apostrophe,
    Minus,
    Equal,
    LeftBracket,
    RightBracket,
    Backslash,
    Grave,
    Enter,
    Backspace,
    Tab,
    Escape,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowLeft,
    ArrowUp,
    ArrowRight,
    ArrowDown,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    MouseLeft,
    MouseRight,
    MouseMiddle,
}

impl VKey {
    /// 全部变体，用于遍历/校验。
    pub const ALL: [VKey; 83] = [
        VKey::KeyA,
        VKey::KeyB,
        VKey::KeyC,
        VKey::KeyD,
        VKey::KeyE,
        VKey::KeyF,
        VKey::KeyG,
        VKey::KeyH,
        VKey::KeyI,
        VKey::KeyJ,
        VKey::KeyK,
        VKey::KeyL,
        VKey::KeyM,
        VKey::KeyN,
        VKey::KeyO,
        VKey::KeyP,
        VKey::KeyQ,
        VKey::KeyR,
        VKey::KeyS,
        VKey::KeyT,
        VKey::KeyU,
        VKey::KeyV,
        VKey::KeyW,
        VKey::KeyX,
        VKey::KeyY,
        VKey::KeyZ,
        VKey::Digit0,
        VKey::Digit1,
        VKey::Digit2,
        VKey::Digit3,
        VKey::Digit4,
        VKey::Digit5,
        VKey::Digit6,
        VKey::Digit7,
        VKey::Digit8,
        VKey::Digit9,
        VKey::F1,
        VKey::F2,
        VKey::F3,
        VKey::F4,
        VKey::F5,
        VKey::F6,
        VKey::F7,
        VKey::F8,
        VKey::F9,
        VKey::F10,
        VKey::F11,
        VKey::F12,
        VKey::Space,
        VKey::Comma,
        VKey::Period,
        VKey::Slash,
        VKey::Semicolon,
        VKey::Apostrophe,
        VKey::Minus,
        VKey::Equal,
        VKey::LeftBracket,
        VKey::RightBracket,
        VKey::Backslash,
        VKey::Grave,
        VKey::Enter,
        VKey::Backspace,
        VKey::Tab,
        VKey::Escape,
        VKey::Insert,
        VKey::Delete,
        VKey::Home,
        VKey::End,
        VKey::PageUp,
        VKey::PageDown,
        VKey::ArrowLeft,
        VKey::ArrowUp,
        VKey::ArrowRight,
        VKey::ArrowDown,
        VKey::ShiftLeft,
        VKey::ShiftRight,
        VKey::ControlLeft,
        VKey::ControlRight,
        VKey::AltLeft,
        VKey::AltRight,
        VKey::MouseLeft,
        VKey::MouseRight,
        VKey::MouseMiddle,
    ];

    /// winit KeyboardKey / MouseButton 文档名。鼠标键不是 `KeyCode`，
    /// 主控对鼠标事件单独映射（`MouseLeft` → `MouseButton::Left`）。
    pub fn winit_name(self) -> &'static str {
        match self {
            VKey::KeyA => "KeyA",
            VKey::KeyB => "KeyB",
            VKey::KeyC => "KeyC",
            VKey::KeyD => "KeyD",
            VKey::KeyE => "KeyE",
            VKey::KeyF => "KeyF",
            VKey::KeyG => "KeyG",
            VKey::KeyH => "KeyH",
            VKey::KeyI => "KeyI",
            VKey::KeyJ => "KeyJ",
            VKey::KeyK => "KeyK",
            VKey::KeyL => "KeyL",
            VKey::KeyM => "KeyM",
            VKey::KeyN => "KeyN",
            VKey::KeyO => "KeyO",
            VKey::KeyP => "KeyP",
            VKey::KeyQ => "KeyQ",
            VKey::KeyR => "KeyR",
            VKey::KeyS => "KeyS",
            VKey::KeyT => "KeyT",
            VKey::KeyU => "KeyU",
            VKey::KeyV => "KeyV",
            VKey::KeyW => "KeyW",
            VKey::KeyX => "KeyX",
            VKey::KeyY => "KeyY",
            VKey::KeyZ => "KeyZ",
            VKey::Digit0 => "Digit0",
            VKey::Digit1 => "Digit1",
            VKey::Digit2 => "Digit2",
            VKey::Digit3 => "Digit3",
            VKey::Digit4 => "Digit4",
            VKey::Digit5 => "Digit5",
            VKey::Digit6 => "Digit6",
            VKey::Digit7 => "Digit7",
            VKey::Digit8 => "Digit8",
            VKey::Digit9 => "Digit9",
            VKey::F1 => "F1",
            VKey::F2 => "F2",
            VKey::F3 => "F3",
            VKey::F4 => "F4",
            VKey::F5 => "F5",
            VKey::F6 => "F6",
            VKey::F7 => "F7",
            VKey::F8 => "F8",
            VKey::F9 => "F9",
            VKey::F10 => "F10",
            VKey::F11 => "F11",
            VKey::F12 => "F12",
            VKey::Space => "Space",
            VKey::Comma => "Comma",
            VKey::Period => "Period",
            VKey::Slash => "Slash",
            VKey::Semicolon => "Semicolon",
            VKey::Apostrophe => "Quote",
            VKey::Minus => "Minus",
            VKey::Equal => "Equal",
            VKey::LeftBracket => "BracketLeft",
            VKey::RightBracket => "BracketRight",
            VKey::Backslash => "Backslash",
            VKey::Grave => "Backquote",
            VKey::Enter => "Enter",
            VKey::Backspace => "Backspace",
            VKey::Tab => "Tab",
            VKey::Escape => "Escape",
            VKey::Insert => "Insert",
            VKey::Delete => "Delete",
            VKey::Home => "Home",
            VKey::End => "End",
            VKey::PageUp => "PageUp",
            VKey::PageDown => "PageDown",
            VKey::ArrowLeft => "ArrowLeft",
            VKey::ArrowUp => "ArrowUp",
            VKey::ArrowRight => "ArrowRight",
            VKey::ArrowDown => "ArrowDown",
            VKey::ShiftLeft => "ShiftLeft",
            VKey::ShiftRight => "ShiftRight",
            VKey::ControlLeft => "ControlLeft",
            VKey::ControlRight => "ControlRight",
            VKey::AltLeft => "AltLeft",
            VKey::AltRight => "AltRight",
            VKey::MouseLeft => "MouseLeft",
            VKey::MouseRight => "MouseRight",
            VKey::MouseMiddle => "MouseMiddle",
        }
    }

    /// 从 winit 文档名解析；大小写不敏感（"keyw" 亦可），未知返回 None。
    pub fn from_winit_name(name: &str) -> Option<VKey> {
        let name = name.trim();
        VKey::ALL.into_iter().find(|k| {
            let w = k.winit_name();
            w.eq_ignore_ascii_case(name)
        })
    }

    pub fn display(self) -> &'static str {
        self.winit_name()
    }
}

/// 动作 → 按键 映射。同一键允许多动作（MC 行为），冲突由 [`KeyMap::conflicting`] 报告。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMap {
    map: HashMap<Action, VKey>,
}

impl Default for KeyMap {
    /// MC 26.1 默认 + 本引擎既有约定（F 切飞行、数字键选物品栏、鼠标左右键挖/放）。
    /// sprint 取 MC 的左 Ctrl；现有 app.rs 里 ShiftRight 的冗余绑定丢弃。
    fn default() -> Self {
        let mut map = HashMap::new();
        map.insert(Action::Forward, VKey::KeyW);
        map.insert(Action::Back, VKey::KeyS);
        map.insert(Action::Left, VKey::KeyA);
        map.insert(Action::Right, VKey::KeyD);
        map.insert(Action::Jump, VKey::Space);
        map.insert(Action::Sneak, VKey::ShiftLeft);
        map.insert(Action::Sprint, VKey::ControlLeft);
        map.insert(Action::FlyToggle, VKey::KeyF);
        map.insert(Action::Inventory, VKey::KeyE);
        map.insert(Action::Debug, VKey::F3);
        map.insert(Action::PickBlock, VKey::MouseMiddle);
        map.insert(Action::Screenshot, VKey::F2);
        map.insert(Action::Pause, VKey::Escape);
        map.insert(Action::Mine, VKey::MouseLeft);
        map.insert(Action::Place, VKey::MouseRight);
        map.insert(Action::HotbarSelect, VKey::Digit1);
        Self { map }
    }
}

impl KeyMap {
    pub fn get(&self, a: Action) -> VKey {
        self.map[&a]
    }

    pub fn set(&mut self, a: Action, k: VKey) {
        self.map.insert(a, k);
    }

    /// 恢复全部动作到默认绑定。
    pub fn restore_default(&mut self) {
        *self = Self::default();
    }

    /// 快捷键查询：键 → 动作。冲突时按 [`Action::ALL`] 顺序取第一个，保证确定性。
    pub fn action_for(&self, k: VKey) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|a| self.map.get(a) == Some(&k))
    }

    /// 与 `k` 冲突的既有动作（排除自身绑定），供 UI 显示警告。
    pub fn conflicting(&self, a: Action, k: VKey) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|o| *o != a && self.map.get(o) == Some(&k))
    }

    /// 持久化文本：每行 `动作名:按键名`，按 [`Action::ALL`] 顺序。
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for a in Action::ALL {
            if let Some(k) = self.map.get(&a) {
                s.push_str(&format!("{}:{}\n", a.as_str(), k.winit_name()));
            }
        }
        s
    }

    /// 解析持久化文本。容错：空行、`#` 注释、格式错误、未知动作/按键 → 跳过该行。
    /// 未出现的动作取默认值。
    pub fn from_text(text: &str) -> Self {
        let mut this = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, key)) = line.split_once(':') else {
                continue;
            };
            let Ok(a) = parse_action(name.trim()) else {
                continue;
            };
            let Some(k) = VKey::from_winit_name(key.trim()) else {
                continue;
            };
            this.map.insert(a, k);
        }
        this
    }
}

fn parse_action(name: &str) -> Result<Action, ()> {
    Action::ALL
        .into_iter()
        .find(|a| a.as_str().eq_ignore_ascii_case(name))
        .ok_or(())
}
