# rustcreate

Minecraft 类体素沙盒，自研引擎重写：**wgpu 30 引擎核心 + 纯 Rust 全栈**（任务板 #77 起 C++17 热路径已整体拆除）。

分层原则：**引擎提供能力，游戏提供规则**。`engine/` 不知道方块/物品/MC 语义，
`game/` 不直接使用 wgpu / winit；依赖只允许 game → engine，反向由编译期阻断。

## 引擎层 `engine/`（可复用能力，无 MC 语义）

| 语言 | crate | 职责 |
|---|---|---|
| Rust | `engine/mcv_render` | wgpu 30 管线、WGSL、昼夜雾、视锥剔除、HUD/字体图集 |
| Rust | `engine/mcv_game` | 体素运行时能力：AABB 扫掠物理、DDA 射线、键位表 |
| Rust | `engine/mcv_save` | 通用体素存档：region 文件 + RLE |
| Rust | `engine/mcv_audio` | 音频混音与解码，按句柄播放 |

## 游戏层 `game/`（MC 规则，含方块/物品/世界语义）

| 语言 | crate | 职责 |
|---|---|---|
| Rust | `game/mcv_core` | 区块布局(16x256x16)、方块注册表、任务池 |
| Rust | `game/mcv_worldgen` | 地形编排（legacy 冻结基线 + vanilla 26.1 管线） |
| Rust | `game/mcv_light` | 双通道 BFS（sky/block）、removal、跨区块同步 |
| Rust | `game/mcv_mesher` | 网格化编排 + 网格缓冲（纯 Rust 贪心网格器） |
| Rust | `game/mcv_entity` / `game/mcv_item` | 生物 AI、战斗、掉落；物品与快捷栏 |
| Rust | `game/mcv_logic` | 游戏运行时编排：区块流式调度、玩家、实体、HUD 装配 |
| Rust | `game/mcv_app` | 平台壳：winit 0.30 窗口/输入/surface + 菜单 UI + 触屏 |
| Rust | `game/mcv_desktop` | 桌面入口 `mcv` 二进制 |

## 云端构建（无本地工具链要求）

push 到 `main` → Actions 自动构建四平台产物（Windows / Linux / macOS arm64 / Android APK）；
打 tag（如 `v0.1.0`）→ 自动创建 Release 附 SHA256。

| 工作流 | 触发 | 内容 |
|---|---|---|
| `ci.yml` | push / PR | fmt + clippy -D warnings + 全量测试 |
| `build.yml` | main / 复用 | 4 平台 release 产物 |
| `release.yml` | tag `v*` | Release 发布 + checksums |

构建产物在 **Actions → 最新运行 → Artifacts**。

## 本地运行（有 Rust 工具链时）

```sh
cargo run -p mcv_app --release
```

桌面：WASD 移动，空格跳，鼠标视角，左键挖，右键放，Esc 释放鼠标。
Android：左半屏虚拟摇杆，右半屏视角，右下按钮跳/放。

## 许可

MIT
