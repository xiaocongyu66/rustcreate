//! 引擎的平台输入层：把 winit 原生事件翻译成引擎内部输入类型。
//!
//! 分层定位（引擎能力 vs 游戏规则）：OS/窗口系统的输入差异（触屏相位、
//! 物理键码、坐标系）全部隔离在本 crate 内——上层（平台壳 mcv_app、
//! 运行时编排 mcv_logic）只面向引擎内部类型（[`keybind::VKey`]、
//! [`touch::TouchState`] 的意图输出），不直接 `use winit`。
//!
//! - [`touch`]：移动端虚拟摇杆/按钮状态机，`WindowEvent::Touch` →
//!   摇杆向量/跳/挖/放/视角增量/快捷栏槽位等意图。
//! - [`keybind`]：winit `KeyCode` → [`keybind::VKey`] 虚拟键转换表，
//!   以及数字键 → 快捷栏槽位的直绑。
//!
//! 本 crate 不知道方块/物品/MC 语义；把 VKey 组合成语义动作是游戏层
//! （mcv_logic/mcv_app）的规则，不在这里。

pub mod keybind;
pub mod touch;
