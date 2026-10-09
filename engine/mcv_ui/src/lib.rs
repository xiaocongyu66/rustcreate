//! 统一 GUI 框架（M8b 任务 #56）：机制对照原版 26.1
//! `net/minecraft/client/gui/**`（Screen / GuiGraphics / AbstractWidget /
//! AbstractContainerScreen / Slot / Font）。
//!
//! 设计红线（任务派单）：
//! - 只依赖 `mcv_render` 的 `HudQuad`/字体度量/精灵表类型，不碰
//!   wgpu/winit（CI 分层守卫：mcv_ui 属 engine 层）；
//! - 全部纯 CPU 数据结构 + 事件输出：游戏层拿事件改背包/按键表，
//!   mcv_ui 不含游戏逻辑（物品注册表、玩家、音效播放都在游戏层）；
//! - 坐标体系：逻辑像素（GUI 单位）+ GUI scale 缩放语义，对照
//!   `Window.calculateScale`/`setGuiScale`（Window.java:445-472）。
//!
//! 模块一览：
//! - [`context`]：`UiContext`（GUI scale 换算）+ `UiGraphics`（HudQuad
//!   收集器：文本/矩形/精灵/物品图标 + scissor 裁剪栈，对照
//!   GuiGraphicsExtractor.enableScissor / ScissorStack）。
//! - [`screen`]：`Screen` trait + `ScreenStack`（push/pop/replace、
//!   关闭时输入穿透规则、暂停语义对照 `Screen.isPauseScreen`）。
//! - [`widget`]：Button / Slider / Checkbox / Tooltip（悬停延迟与
//!   翻边对照 DefaultTooltipPositioner）。
//! - [`slot`]：容器槽位状态机（Slot 矩形表 + picked 物品跟随光标 +
//!   左键收/放 + 右键分半 + shift 快移 + 三类拖拽分发；内部含
//!   `AbstractContainerMenu.doClick` 的纯数据移植，对镜像
//!   `StackRef` 槽位表生效并同步发 [`slot::MenuCmd`] 事件——游戏层
//!   拿事件改背包，容器关闭时读回镜像）。
//! - [`text`]：文本裁剪/折行（对照 `Font.plainSubstrByWidth` 与
//!   `Tooltip.MAX_WIDTH` 170）。

pub mod context;
pub mod screen;
pub mod slot;
pub mod text;
pub mod widget;

pub use context::{Rect, UiContext, UiGraphics, calculate_scale, scaled_len};
pub use screen::{KeyEvent, Screen, ScreenStack, key};
pub use slot::{MenuCmd, StackRef};
