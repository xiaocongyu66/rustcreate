//! 游戏引擎运行时：世界流式调度、区块状态机、网格上传预算、物理/实体
//! 步进、昼夜、音频接线的**编排层**（MC 特化，不是通用引擎核）。
//!
//! 分层定位（平台壳 → 本 crate → 子系统 crate）：
//! - 上层 [`mcv_app`]：winit 事件循环、surface 生命周期、屏幕状态机、
//!   菜单/HUD 排版、平台路径——不碰世界数据。
//! - 本 crate：[`game::GameRuntime`] 是唯一入口，持有 chunks/player/mobs/
//!   mesher/save；外部只调 `fixed_step / stream / camera / render_chunks /
//!   build_hud / interact / attack / look`。GPU 资源由上层创建后注入
//!   （device/queue 是 Arc 型句柄）。
//! - 下层：mcv_worldgen / mcv_light / mcv_mesher（纯数据管线）、
//!   mcv_render（纯 wgpu）、mcv_game（物理/输入）、mcv_entity / mcv_item /
//!   mcv_audio / mcv_save。
//!
//! 移动端触屏事件翻译（winit `WindowEvent::Touch` → 输入意图）属于平台
//! 能力，已移入引擎层 [`mcv_platform::touch`]；本 crate 的 GameRuntime
//! 持有其 `TouchState`。
//!
//! 将来若上插件化（EnginePlugin/事件总线），注册与路由加在本 crate 边界
//! 之上即可，子系统 crate 不需感知。

pub mod game;
