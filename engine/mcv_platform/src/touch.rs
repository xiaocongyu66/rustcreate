//! 移动端触屏控件：虚拟摇杆 + 动作按钮 + 拖屏转视角 + 快捷栏点按。
//!
//! 交互设计参照 FoldCraftLauncher 的移动端方案，视觉全部程序化绘制
//! （HUD 半透明方块），不使用任何外部素材。

use winit::event::{TouchPhase, WindowEvent};

/// 摇杆半径（逻辑像素）。
pub const STICK_R: f32 = 56.0;
/// 动作按钮半径。
pub const BTN_R: f32 = 32.0;
/// 摇杆死区。
const DEADZONE: f32 = 8.0;

/// 一帧内消费掉的触控效果。
#[derive(Default)]
pub struct TouchEffects {
    pub look: (f32, f32),
    pub slot: Option<usize>,
    pub place: bool,
    pub attack: bool,
    /// 跳跃键**松开沿**（本次 consume 窗口内松开过）——运行时据此把
    /// input.jump 拉回 false（触屏没有键盘按键释放事件，若只按住置真、
    /// 松开不置假，jump 会永远悬真：落地自动连跳/飞行中永久上升）。
    pub jump_released: bool,
    /// 放置键**松开沿**（同 jump_released 的道理）：进食/弓的按住态依赖
    /// 运行时 input.placing 持续为真，松开沿负责拉回 false。
    pub place_released: bool,
}

#[derive(Default)]
pub struct TouchState {
    /// 收到过触摸事件（据此显示触控 HUD）。
    pub enabled: bool,
    stick_id: Option<u64>,
    stick_anchor: (f32, f32),
    pub stick_vec: (f32, f32),
    look_id: Option<u64>,
    look_last: (f32, f32),
    look_acc: (f32, f32),
    jump_id: Option<u64>,
    mine_id: Option<u64>,
    place_id: Option<u64>,
    pub jump_held: bool,
    pub mine_held: bool,
    /// 放置键按住态（与 place_id 同步维护；运行时镜像 input.placing 用，
    /// 进食按住不松的推进依赖它）。
    pub place_held: bool,
    effects: TouchEffects,
}

impl TouchState {
    /// 各控件中心点（与 `build_hud` 的绘制保持一致）。
    pub fn stick_center(_w: f32, h: f32) -> (f32, f32) {
        (STICK_R * 1.7, h - STICK_R * 1.7)
    }

    pub fn jump_center(w: f32, h: f32) -> (f32, f32) {
        (w - BTN_R * 1.8, h - BTN_R * 1.8)
    }

    pub fn mine_center(w: f32, h: f32) -> (f32, f32) {
        (w - BTN_R * 1.8, h - BTN_R * 4.6)
    }

    pub fn place_center(w: f32, h: f32) -> (f32, f32) {
        (w - BTN_R * 4.6, h - BTN_R * 1.8)
    }

    /// 快捷栏矩形（与 build_hud 相同的布局参数）。
    fn hotbar_rect(w: f32, h: f32) -> (f32, f32, f32, f32) {
        let slot = 40.0;
        let total = slot * 9.0;
        (w * 0.5 - total * 0.5, h - slot - 8.0, total, slot)
    }

    fn hit(cx: f32, cy: f32, r: f32, x: f32, y: f32) -> bool {
        let (dx, dy) = (x - cx, y - cy);
        dx * dx + dy * dy <= r * r
    }

    pub fn on_event(&mut self, event: &WindowEvent, w: f32, h: f32) {
        let WindowEvent::Touch(touch) = event else {
            return;
        };
        let phase = touch.phase;
        let id = touch.id;
        let (x, y) = (touch.location.x as f32, touch.location.y as f32);
        self.enabled = true;
        match phase {
            TouchPhase::Started => {
                let (jx, jy) = Self::jump_center(w, h);
                let (mx, my) = Self::mine_center(w, h);
                let (px, py) = Self::place_center(w, h);
                let (hx, hy, hw, hh) = Self::hotbar_rect(w, h);
                if Self::hit(jx, jy, BTN_R * 1.4, x, y) {
                    self.jump_id = Some(id);
                    self.jump_held = true;
                } else if Self::hit(mx, my, BTN_R * 1.4, x, y) {
                    self.mine_id = Some(id);
                    self.mine_held = true;
                } else if Self::hit(px, py, BTN_R * 1.4, x, y) {
                    self.place_id = Some(id);
                    self.place_held = true;
                    self.effects.place = true;
                } else if x >= hx && x <= hx + hw && y >= hy - 12.0 && y <= hy + hh + 12.0 {
                    let idx = ((x - hx) / 40.0) as usize;
                    if idx < 9 {
                        self.effects.slot = Some(idx);
                    }
                } else if x < w * 0.45 && y > h * 0.4 && self.stick_id.is_none() {
                    self.stick_id = Some(id);
                    self.stick_anchor = (x, y);
                    self.stick_vec = (0.0, 0.0);
                } else if self.look_id.is_none() {
                    self.look_id = Some(id);
                    self.look_last = (x, y);
                }
            }
            TouchPhase::Moved => {
                if self.stick_id == Some(id) {
                    let (ax, ay) = self.stick_anchor;
                    let (dx, dy) = (x - ax, y - ay);
                    let len = (dx * dx + dy * dy).sqrt();
                    let v = if len > STICK_R {
                        let k = STICK_R / len;
                        (dx * k, dy * k)
                    } else {
                        (dx, dy)
                    };
                    self.stick_vec = v;
                } else if self.look_id == Some(id) {
                    let (lx, ly) = self.look_last;
                    self.look_acc.0 += x - lx;
                    self.look_acc.1 += y - ly;
                    self.look_last = (x, y);
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if self.stick_id == Some(id) {
                    self.stick_id = None;
                    self.stick_vec = (0.0, 0.0);
                }
                if self.look_id == Some(id) {
                    self.look_id = None;
                }
                if self.jump_id == Some(id) {
                    self.jump_id = None;
                    self.jump_held = false;
                    self.effects.jump_released = true;
                }
                if self.mine_id == Some(id) {
                    self.mine_id = None;
                    self.mine_held = false;
                }
                if self.place_id == Some(id) {
                    self.place_id = None;
                    self.place_held = false;
                    self.effects.place_released = true;
                }
            }
        }
    }

    /// 每帧取走累计效果并清零。
    pub fn consume(&mut self) -> TouchEffects {
        let look = std::mem::replace(&mut self.look_acc, (0.0, 0.0));
        let mut fx = std::mem::take(&mut self.effects);
        fx.look = look;
        fx
    }

    /// 程序化松开跳跃键：等价 TouchPhase::Ended 落在跳跃键上（置松开沿）。
    /// 供无头测试/输入回放驱动，不经 winit 事件也能走完整按下-松开沿。
    pub fn release_jump(&mut self) {
        if self.jump_held {
            self.jump_held = false;
            self.effects.jump_released = true;
        }
    }

    /// 程序化按下挖掘键：等价 TouchPhase::Started 落在挖按钮上（mine_held
    /// 置真）。供无头测试/输入回放驱动——app 层对 runtime.touch.on_event 的
    /// 事件翻译只落在这两个布尔字段上，headless 侧驱动同字段即等价复现
    /// 「触摸 → fixed_step 内 apply_touch_input → 挖掘状态机」全链。
    pub fn press_mine(&mut self) {
        self.mine_held = true;
    }

    /// 程序化松开挖掘键：等价 TouchPhase::Ended 落在挖按钮上（mine_held
    /// 置假）。与 [`Self::press_mine`] 配对。
    pub fn release_mine(&mut self) {
        self.mine_held = false;
    }

    /// 摇杆偏移换算成移动方向（供运行时合成 wish_dir）。
    pub fn stick_direction(&self) -> Option<(f32, f32)> {
        let (dx, dy) = self.stick_vec;
        let len = (dx * dx + dy * dy).sqrt();
        if len < DEADZONE {
            None
        } else {
            Some((dx / len.max(1.0), dy / len.max(1.0)))
        }
    }

    /// 冲刺门：摇杆物理偏移超过 85% 半径（满杆推 = 冲刺，Bedrock 语义）。
    /// 必须用**像素**模长判定——`stick_direction` 返回的是归一化单位向量，
    /// 对它再求模恒为 1，任何阈值都会失效。
    pub fn stick_sprint(&self) -> bool {
        let (dx, dy) = self.stick_vec;
        (dx * dx + dy * dy).sqrt() > STICK_R * 0.85
    }

    /// 摇杆模拟量幅度 0..=1：死区线性重标定到满偏半径（死区内 0，
    /// STICK_R 封顶 1）。供运行时按幅度缩放移动速度（模拟量摇杆）。
    pub fn stick_analog(&self) -> f32 {
        let (dx, dy) = self.stick_vec;
        let len = (dx * dx + dy * dy).sqrt();
        if len <= DEADZONE {
            0.0
        } else {
            ((len - DEADZONE) / (STICK_R - DEADZONE)).min(1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::event::{DeviceId, Force, WindowEvent};

    /// 合成 winit 触摸事件（真机 app 层把 WindowEvent::Touch 原样转发给
    /// TouchState::on_event；测试喂同一事件形态即覆盖同一翻译路径）。
    fn touch_event(phase: TouchPhase, x: f64, y: f64) -> WindowEvent {
        WindowEvent::Touch(winit::event::Touch {
            device_id: DeviceId::dummy(),
            phase,
            location: PhysicalPosition::new(x, y),
            force: Some(Force::Normalized(1.0)),
            id: 7,
        })
    }

    /// 真机「不能挖掘」链路第 1 段回归锁：挖按钮物理坐标命中 → mine_held
    /// 沿置真/假。按钮中心与 build_hud 绘制共用同一 center 常数，坐标空间
    /// 均为物理像素（winit Touch.location = PhysicalPosition，app 层喂的
    /// surface config 尺寸同为物理像素，无 DPI 错位）。
    #[test]
    fn mine_button_touch_sets_mine_held() {
        let (w, h) = (1080.0, 2340.0); // 典型手机物理分辨率
        let (mx, my) = TouchState::mine_center(w, h);
        let mut ts = TouchState::default();
        assert!(!ts.mine_held);
        ts.on_event(
            &touch_event(TouchPhase::Started, mx as f64, my as f64),
            w,
            h,
        );
        assert!(ts.mine_held, "挖按钮按下沿必须置 mine_held");
        assert!(ts.enabled, "任意触摸事件先点亮触控 HUD");
        // 手指未抬起的 Moved 不得误清（真机长按挖掘时手指抖动）。
        ts.on_event(
            &touch_event(TouchPhase::Moved, mx as f64, my as f64 + 3.0),
            w,
            h,
        );
        assert!(ts.mine_held, "挖按钮长按中的微移不得松开挖掘");
        ts.on_event(&touch_event(TouchPhase::Ended, mx as f64, my as f64), w, h);
        assert!(!ts.mine_held, "挖按钮松开沿必须清 mine_held");
    }

    /// 程序化 press_mine/release_mine 与 winit 事件路径产出同一状态
    /// （headless 端到端测试据此前提驱动 runtime.touch 字段）。
    #[test]
    fn programmatic_mine_drivers_mirror_touch_events() {
        let (w, h) = (800.0, 600.0);
        let (mx, my) = TouchState::mine_center(w, h);
        let mut event = TouchState::default();
        event.on_event(
            &touch_event(TouchPhase::Started, mx as f64, my as f64),
            w,
            h,
        );
        let mut direct = TouchState {
            enabled: true,
            ..Default::default()
        };
        direct.press_mine();
        assert_eq!(event.mine_held, direct.mine_held);
        assert_eq!(event.enabled, direct.enabled);
        event.on_event(&touch_event(TouchPhase::Ended, mx as f64, my as f64), w, h);
        direct.release_mine();
        assert!(!direct.mine_held);
        assert!(!event.mine_held);
    }
}
