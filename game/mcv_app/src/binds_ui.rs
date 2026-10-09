//! 按键重映射界面：纯逻辑（捕获状态机 + 双语文案表），不依赖 winit/渲染，
//! 可独立单测；app.rs 只做绘制与事件接线。

use mcv_game::keymap::{Action, KeyMap, VKey};

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
