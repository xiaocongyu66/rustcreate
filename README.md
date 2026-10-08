# rustcreate

Minecraft 类体素沙盒，自研引擎重写：**wgpu 30 引擎核心 + Rust 主体 + C++17 热路径**。

| 层 | 语言 | crate / 目录 | 职责 |
|---|---|---|---|
| 公共底座 | Rust | `crates/mcv_core` | 区块布局(16x256x16)、方块注册表、任务池 |
| FFI 边界 | Rust+cc | `crates/mcv_ffi` | 唯一 C ABI：repr(C) 镜像、静态断言、RAII 内存契约 |
| 复杂地形算法 | **C++17** | `cpp/src/terrain.cpp` | 三层噪声、双阈值 3D 洞穴、树投影 |
| 自定义渲染后端 | **C++17** | `cpp/src/mesher.cpp` | 贪心网格化 + 逐顶点 AO + 顶点打包（wgpu 直读） |
| 内存回收 | Rust+C++ | `cpp/src/mempool.cpp` | 尺寸分级 freelist 池 + canary，Rust RAII Drop 归还 |
| 光照引擎 | Rust | `crates/mcv_light` | 双通道 BFS（sky/block）、removal、跨区块同步 |
| 渲染器 | Rust | `crates/mcv_render` | wgpu 30 管线、WGSL、昼夜雾、视锥剔除 |
| 游戏逻辑 | Rust | `crates/mcv_game` | AABB 扫掠物理、DDA 射线、输入 |
| 存档 | Rust | `crates/mcv_save` | region 文件 + RLE |
| 平台壳 | Rust | `crates/mcv_app` | winit 0.30（桌面三平台 + Android NativeActivity） |

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
