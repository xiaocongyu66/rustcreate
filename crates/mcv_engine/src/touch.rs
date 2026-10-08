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
                }
                if self.mine_id == Some(id) {
                    self.mine_id = None;
                    self.mine_held = false;
                }
                if self.place_id == Some(id) {
                    self.place_id = None;
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
}
