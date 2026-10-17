# UI / 输入 / 音频 三域差距审计：mcv_ui·mcv_game·mcv_audio vs Bevy 0.20

> 日期：2026-10-17。只读审计（不改游戏代码），姊妹篇：
> `bevy-render.md` / `bevy-ecs.md` / `bevy-events.md` / `bevy-physics-misc.md` /
> `bevy-sys-asset.md`（本章不改动它们）。
>
> 证据基线：
> - 本仓：`engine/mcv_ui`（6 文件 ≈3.55k 行）、`engine/mcv_audio`（5 文件 ≈2.3k
>   行 + 9 测试）、`engine/mcv_game/src/keymap.rs`（按键重映射）、
>   `game/mcv_app/src/app.rs`（3567 行 winit 壳 + 菜单/容器接线）、
>   `game/mcv_app/src/{binds_ui,craft_ui,loading_ui,options,i18n}.rs`、
>   `game/mcv_logic/src/ui.rs`（1207 行容器/合成/熔炉状态机）、
>   `game/mcv_logic/src/game.rs`（touch 镜像）。原版 26.1 规格出处
>   `/root/mc-ref/src-26.1`（不入仓）。
> - Bevy 侧：**本仓不依赖 bevy**；对拍基线为 crates.io 官方源码快照
>   bevy_ui-0.20.0 / bevy_input-0.20.0 / bevy_audio-0.20.0 / bevy_window-0.20.0 /
>   bevy_winit-0.20.0 / bevy_feathers-0.20.0 / bevy_text-0.20.0 / kira-0.10.7
>   （研究用快照，不入库、不编译）；行号以该树为准。
> - 判价原则同姊妹篇：本仓十屏 + 容器 + HUD、单机桌面/Android 触屏优先、
>   原版 26.1 逐屏对齐是第一目标。差距按「现实影响 × 迁移成本」计价，
>   不做「Bevy 有 = 我们该有」的等号；区分「缺能力」与「原版本就无」。

## 0. 本仓三域现状（代码证据）

### 0.1 UI 域（engine/mcv_ui）

机制对照原版 26.1 `net/minecraft/client/gui/**`，纯 CPU 数据结构 +
事件出队（不含游戏逻辑，红线：不碰 wgpu/winit，`lib.rs:5-11`）。

| 模块 | 实现 | 锚点 |
|---|---|---|
| context | `UiContext` GUI scale（320×240 基准整数档自动/封顶 + 逻辑分辨率向上取整）+ `Rect` + `UiGraphics`（HudQuad 收集器：fill/text/item_icon/push_raw + **CPU 侧 scissor 栈求交 + UV 比例裁剪**） | context.rs:27-48, 134-246, 251-293 |
| screen | `Screen` trait（is_pause_screen/should_close_on_esc/on_open/on_resize/on_close/on_key/mouse_*/draw；事件出队靠 `as_any` downcast）+ `ScreenStack`（push/pop/replace/resize、**顶层独占输入**、Esc 关屏协议、暂停信号交引擎发） | screen.rs:59-104, 109-248 |
| widget | `WidgetSet`：Button/Slider/Checkbox/Tooltip（原版常量全锚：200×20、TEXT_MARGIN 2、滑杆高 20/把手 8、checkbox 盒 17/距 4、TOOLTIP_MAX_WIDTH 170、(12,-12) 翻边）；`UiEvent`/`UiSound` 事件出队 | widget.rs:28-61, 130-499 |
| slot | `ContainerScreen` = `AbstractContainerMenu.doClick` 纯数据移植：PICKUP/QUICK_MOVE/THROW/CLONE/QUICK_CRAFT 三段 mask 协议/PICKUP_ALL 双击、picked 跟随 (-8,-8)、面板外 -999、quickMove 路由闭包（分组→区间段表）；`StackRef` 镜像（kind/count/meta 表索引；**组件级相等性未建模**）；触屏 quickdrop/snapback 未移植（自文档 slot.rs:24-25） | slot.rs:29-210, 247-510+ |
| layout | `LinearLayout` 移植（方向/spacing/CellSettings padding+归一化对齐/spacer；f32 逻辑像素为有意差异）；**无** GridLayout 通用形态/锚点组/网格单元跨行跨列 | layout.rs:25-262 |
| text | clip_head/clip_tail（plainSubstrByWidth）+ wrap（170 上限、词断行、CJK 逐字硬断） | text.rs:15-92 |

游戏侧装配（决定差距的现实权重）：

- app.rs 有**自己的** `Screen` enum 十态机（Main/Worlds/Create/Settings/
  InGame/Paused/Death/KeyBinds/Crafting/Creative，app.rs:19-45）；mcv_ui 的
  `ScreenStack` 实际只托管 KeyBinds 一屏（`open_keybinds` app.rs:1528-1539；
  binds_ui.rs:5-6「屏幕骨架迁 mcv_ui」）——**双状态机并存是本仓自有债务**，
  非 Bevy 差距。
- 容器 UI（#158 波）：`craft_menu: Option<mcv_ui::slot::ContainerScreen>`
  （app.rs:198），`make_craft_menu`（app.rs:459-492）构造网格+36 背包槽；
  每次交互 `craft_flush_menu` → `take_cmds` 回写 Hotbar/网格（app.rs:1830,
  1889）；拖拽三段/shift/双击/面板外全走 doClick（app.rs:2565 注释）。
- 暂停/容器不暂停 = `is_pause_screen`（app.rs:43-45）+
  `gameplay_input_allowed`（app.rs:51-53）——原版 `Screen.isPauseScreen()`
  与 KeyMapping 短路的本仓平价。
- 绘制为**每帧重收集**：UI quad 経 `UiGraphics.into_quads` 进 `Scene.hud`
  （context.rs:243-245），无 retained 节点树。
- 既有缺口账本：`docs/ui-parity.md`（EditBox 世界名/种子输入 ⬜、世界列表
  滚动 🔨、声音分类音量 UI ⬜、HUD 血/饿/经验 ⬜、聊天 ⬜ 等完整逐屏表）。

### 0.2 输入域（engine/mcv_game/keymap.rs + game/mcv_app/app.rs）

| 面 | 实现 | 锚点 |
|---|---|---|
| 动作层 | 16 个 `Action`（Forward…HotbarSelect）× `VKey`（字母/数字/F1-F12/方向/修饰 + **鼠标键**）HashMap 单绑 | keymap.rs:11-99 |
| 重映射 | `KeyMap` 单绑 + `conflicting` 冲突检测（保守拒绝双绑）+ `to_text/from_text` 持久化（saves 根 keybindings.txt，options.rs）+ UI 侧 `BindCapture` 捕获状态机（Esc 取消/冲突红标） | binds_ui.rs:16-77 |
| 事件入口 | winit 0.30 `ApplicationHandler`：KeyboardInput/PointerInput/CursorMoved/MouseWheel/ModifiersChanged→shift/ctrl 修饰手持（ACS hasShiftDown 对位）；光标差分 `runtime.look` | app.rs:2500-2521 |
| 指针捕获 | `set_cursor_grab(Confined)` + 光标隐藏（InGame 抓取；非 Locked 模式） | app.rs:236-241, 1473-1474 |
| 触屏 | winit `Touch` 三分支：菜单/合成=点击模拟（TouchPhase::Started 一发）；InGame→`runtime.touch.on_event`（**自写虚拟摇杆+挖/放/跳按钮**，BE 布局取向，ui-parity 认 ✅）；容器触屏拖拽/quickdrop/长按未接（app.rs:2532 注释自列债） | app.rs:2524-2557 |
| 游戏内消费 | `gameplay_input_allowed` 门 + 屏幕非空时输入全归屏（原版 screen==null 短路移植）；死亡界面每步清零（app.rs:398-411） | app.rs:51-53 |

**无手柄**（全仓 grep 无 gilrs/gamepad）；无 IME/软键盘（世界名不允许键入，
自动生成——ui-parity 3 §）；按键 held 态以布尔镜像散置 app.rs。

### 0.3 音频域（engine/mcv_audio）

| 面 | 实现 | 锚点 |
|---|---|---|
| 架构 | 游戏线程 `SoundLoader`（sounds.json 表 + event/dictionary **递归展平** + LRU 32 缓存 + 80ms 同变体节流 + 缺失去重 + SplitMix64 加权抽变体）→ **SPSC 命令环 256**（`mcv_sync::Spsc`，满丢保实时）→ tinyaudio 回调线程独占 `Mixer`（24 voice FIFO 上限、线性插值重采样、f32 叠加限幅）——回调热路径零锁零分配 | lib.rs:171-343, loader.rs:39-215, mixer.rs:24-142, mcv_sync/src |
| 后端 | tinyaudio 2.x（**Android AAudio 拉 ndk 无手写 JNI**/Windows DirectSound/Linux ALSA），44100 立体声 f32、1024 帧 ≈23ms 粒度；无声卡退 `silent()` | lib.rs:68-72, 219-241 |
| 语义面 | `play_event`（3D 16 格线性衰减 `distance_attenuation`）+ `play_event_pitched`（行为音高乘子）+ `play_ui`（2D 无衰减）+ `loop_start/loop_stop`（事件名幂等，挖掘持续音）+ `set_master_gain`（AtomicU32 位镜像即时生效） | lib.rs:132-147, 265-333 |
| 素材 | CDN 音效树 #21：`ci/fetch-sounds.sh` piston-meta index 30 按 sha1 内容寻址拉全量（4564 ogg + sounds.json 不入 git，dev-only DEVELOP_ONLY）；`events.rs` 10 事件常量接线 + `tests/events_table.rs` 完备性（CI fetch 后**真校验存在性**，改名对齐 26.1 有账）；`sound_types.inc.rs` 方块 SoundType 数据驱动全表（#136） | fetch-sounds.sh:1-30, events.rs, table.rs |
| 听感现状 | 素材多 22050 单声道 → 混音端**无方位声像**（L=R 平铺），衰减仅按距离（无听者朝向）；`stream=true` 变体仅透传不改链路（music 类大文件若接会整块解码）；无背景音乐/分类音量 | mixer.rs:147-180, table.rs |
| 原版对位 | 26.1 = OpenAL 管线：`SoundSource` 10 分类做**总线**（SoundSource.java:4-13；`refreshCategoryVolume` SoundEngine.java:117-122、`updateCategoryVolume` :152-154 实时刷通道）、3D/声像交给 OpenAL（blaze3d/audio Listener/ListenerTransform；`directionalAudio` 选项即 OpenAL HRTF 开关，SoundEngine.java:106） | mc-ref，不入仓 |

## 1. Bevy 0.20 生态侧基线

### 1.1 bevy_ui 0.20.0

| 机制 | 实现（源码锚点） | 一句话 |
|---|---|---|
| 布局引擎 | **taffy 0.14**（Cargo.toml:147-156，features=flexbox+**grid**+block_layout+content_size+taffy_tree）；`Node` 组件数十布局字段（display/position_type/方向/对齐/gap/网格模板，ui_node.rs:485-816） | Flexbox+CSS Grid 声明式布局 |
| 布局执行 | `UiSurface` 持缓存 TaffyTree，每帧映射实体→节点做**增量 relayout**，measure 回调接文本测量（layout/ui_surface.rs） | 布局树常驻、脏面小 |
| 渲染 | `UiTransform` + gradients（linear/radial 渐变 stops）+ ImageNode 九宫格（widget/image.rs）+ Border/Outline + 3D/2D 相机过滤（UiTargetCamera） | retained 渲染 + change 检测 |
| 交互 | `Interaction` 组件 + interaction_states.rs 新标准状态组件（`Pressed`/`Checked`/`Selected`/`Checked`/`InteractionDisabled`，配 `Checkable`/`Selectable` 标记） | hover/按下/禁用状态即组件 |
| 文本 | bevy_text（**cosmic-text 整形**：复杂文种 shaping/折行/富文本 run）；可编辑文本管线（widget/text_input_layout.rs + bevy_input_focus 集成） | 成熟文本栈 |
| 焦点/导航 | focus.rs（键盘文本焦点）+ **auto_directional_navigation.rs**（手柄/键盘方向焦点自动跳转） | 遥控器级导航 |
| 无障碍 | accessibility.rs **全链 accesskit**（0.24）节点语义自动暴露 | 读屏器可用 |
| 拾取 | picking_backend.rs：bevy_ui 即 bevy_picking 的 UI 后端（pointer 事件统一管） | UI 也走全局指针管线 |
| 事件传播 | Observer `EntityEvent` 沿 UI 树 **Traversal 冒泡 + stop_propagation**（feathers lib.rs:15-18 自述该惯例） | 事件树传播 |

### 1.2 bevy_input / bevy_window / bevy_winit 0.20.0

| 机制 | 实现 | 一句话 |
|---|---|---|
| 按键状态 | `ButtonInput<T>`：pressed/just_pressed/just_released/any_/all_ + 每帧 `clear` 双半沿集合（button_input.rs:125-265） | 键鼠手柄统一状态资源 |
| 键盘 | keyboard.rs(KeyCode/ScanCode/KeyboardInput)；IME 事件在 bevy_window（Ime 组：Preedit/Commit） | IME 一等面 |
| 鼠标 | mouse.rs：原始事件 + **AccumulatedMouseMotion/AccumulatedMouseScroll** 原生累积资源（:264,:285） | 相机控制直读 |
| 手柄 | gamepad.rs **gilrs 全套**：连接热插/按钮轴原始+状态事件、`GamepadSettings`（deadzone/灵敏度 per-button-per-axis，:744-1420）、**震动**（GamepadRumbleIntensity/Request :1696,:1784）；axis.rs 通用双轴死区 | 全设备含 rumble |
| 触摸 | touch.rs：`TouchInput`(id/force/TouchPhase) + `Touches` 资源池（按 id 追踪 start/previous/delta，:53-289） | 多指状态机 |
| 手势 | gestures.rs（0.20 新）：PinchGesture/RotateGesture/双击——**platform 注记：仅 macOS/iOS**（:21-23，Android 无此通道） | 触控板/iOS 平级 |
| 光标 | bevy_window CursorOptions（grab Locked/Confined/None per-window + vis/icon）+ winit 壳 | 与本仓 Confined 对价 |

### 1.3 bevy_audio 0.20.0（+ kira 0.10.7）

| 机制 | 实现 | 一句话 |
|---|---|---|
| 后端 | **rodio**（cpal 打底，audio_output.rs:9）；解码 feature 矩阵 mp3/vorbis/wav/flac/aac（Cargo.toml:38-45） | 各平台 OS 混音器上桌 |
| 声部 | `AudioPlayer<R>`**组件** + PlaybackMode Once/Loop/Despawn/Remove（audio.rs:11-21）——声部生命周期与实体解耦；系统负担 sink 池 | ECS 化声部 |
| 音量 | `Volume` dB/线性（27 档查表 volume.rs:305）+ `fade_towards`（:240）+ `GlobalVolume` 全局资源 | 分贝语义 |
| 空间 | `SpatialAudioSink`：set_ears_position（双耳）+ set_emitter_position（sinks.rs:338-352）；rodio spatial = **pan+距离衰减，非 HRTF** | 声像无耳形 |
| 合成 | pitch.rs `Pitch` 正弦振荡器源 | 代码发声 |
| — kira | manager + **子轨道总线**（add_sub_track + 效果链 + send 路由）、effect（Reverb/Echo/Filter/Distortion）、modulator（LFO/Tween 参数自动化）、**clock/Sequence**（节拍时钟=自适应音乐切换点）、spatial track（pan/rotate/distance model） | 中间件级三条件 |

## 2. 三域差距矩阵

判定 ✔/◐/✘ 同姊妹篇；成本：低=天级局部、中=周级动面、高=换骨架。

### 2.1 UI 域

| # | 能力 | 本仓 | Bevy 基线 | 状态 | 平迁成本 | 纵深成本 | 风险与影响 |
|---|---|---|---|---|---|---|---|
| U1 | 声明式布局引擎 | ✘（LinearLayout 单向手排 + MenuButton 绝对坐标） | taffy 0.14 Flexbox/Grid 增量树 | ✘ | — | 中 | 十屏×<30 控件手排够用且像素级测试锁得住；引 taffy=重排全部屏的构造面，**重构非平迁** |
| U2 | 指针命中/交互状态 | ◐（per-widget rect contains + 顶层独占） | Interaction 组件 + 状态组件族 | ◐ | 低 | — | 语义等价、组织不同；WidgetSet 已测试锁路 |
| U3 | 焦点系统/方向导航 | ✘（键盘只有 Esc 与自绘屏内硬编码） | focus + auto_directional_navigation | ✘ | — | 中 | 触屏-first 可降级；桌面键盘导航诉求出现再立项 |
| U4 | 无障碍 | ✘ | accesskit 全链 | ✘ | — | — | 远期（桌面合规诉求触发） |
| U5 | 文本栈 | ◐（8×8 位图字体 + wrap/clip；CJK 逐字硬断**对齐原版字体模型**） | cosmic-text 整形/富文本 | ◐ | — | — | 原版同是位图字体+advance 折行——本仓取向**更贴原版**；cosmetic 差距不立项 |
| U6 | EditBox 文本输入 | ✘（世界名自动生成；种子不可输入，ui-parity §3 ⬜） | text_input_layout + IME | ✘ | 中 | 中 | **本仓第一 UX 缺口**（无法复现地图）；连带 IME/软键盘（见 I6，Android 触屏一族） |
| U7 | 滚动/列表容器 | ✘（世界列表固定 5 行） | Grain 网格/滚轮裁剪标准件 | ✘ | 中 | 低 | 世界多了直接不可用；配 UiGraphics 既有 scissor 平迁 |
| U8 | 容器槽位/拖拽 | **✔ doClick 高保真移植**（mask 协议/双击/路由/守恒） | Bevy 无此层（bevy_ui 无 inventory 概念；生态需自拼） | ✔ | — | — | 本仓反向优势；残余债=触屏 quickdrop/snapback、SWAP、StackRef 组件级相等（见 3.1-P3） |
| U9 | 屏幕状态机 | ◐（ScreenStack 真栈 + app Screen enum **双轨并存**） | States + UI 树常驻 | ◐ | 低 | — | 是本仓债务不是 Bevy 差距：合流后十态机收口，见 3.1-P8 |
| U10 | retained 渲染 | ✘（每帧重收集 quad，无缓存） | UiTree 渲染系 + change 检测 | ◐ | — | 中 | 屏简单时无感；滚动大列表时 quad 线性爆（触发条件=列表 > 数百项） |
| U11 | 事件传播模型 | ◐（UiEvent/MenuCmd 出队到游戏层显式消费） | Observer TreePropagation 冒泡 | ◐ | — | — | events 审计已裁定取向：帧批次 vs 同步重入；引 Feathers 类组件前需先有冒泡层（依赖项，见 4.1） |
| U12 | 主题/token | ◐（调色板硬编码 + nine_slice 染色） | Color 统一 + 渐变 + Feathers token | ◐ | 低 | — | 纯工程糖；九宫格/三态贴图机制已在 |

### 2.2 输入域

| # | 能力 | 本仓 | Bevy 基线 | 状态 | 平迁成本 | 纵深成本 | 风险与影响 |
|---|---|---|---|---|---|---|---|
| I1 | 按键半沿状态资源 | ✘（winit 事件直译 + held 布尔散置 + game.rs 清零） | ButtonInput 双缓冲集合（just_pressed/just_released 帧语义可测） | ✘ | **中（高收益）** | — | 现实痛点第一名：输入语义无法无头测试且跨帧沿须手维护；薄资源层不动事件入口即可（见 3.1-P1） |
| I2 | 鼠标 delta 累积 | ◐（CursorMoved 差分 last_cursor） | AccumulatedMouseMotion 资源 | ◐ | 低 | — | 一帧多鼠标事件时本仓靠单 last 丢样（快速甩视会跳）；随 P1 一并 |
| I3 | 按键重映射 | **✔ 16 Action+冲突保守+持久化+捕获机**（对标原版 KeyMapping 真-基线而非 Bevy——Bevy 无内建绑层） | ✘（生态散件） | ✔ | — | — | 反向优势；对原版残余债=分类分组列/逐项重置/搜索（ui-parity §4 🔨） |
| I4 | 手柄 | ✘ | gilrs 全套 + deadzone + **rumble** | ✘ | — | 中 | Android 触屏优先，桌面手柄=增强；远期，触发行=桌面覆盖率需求 |
| I5 | 触摸 | ◐（**自写语义层**：虚拟摇杆/按钮/点击模拟；多指仅 Started 一发） | Touches 多指状态池 + force/delta | ◐ | 低 | — | Bevy 只给原始流，本仓语义更深=优势；缺口=多指跟踪与容器触屏拖拽（3.1-P2 清偿债） |
| I6 | IME/软键盘 | ✘ | Ime 组 + winit IME | ✘ | 中 | — | 与 U6 绑死（EditBox 无 IME 不可用 Android） |
| I7 | 手势 | ✘ | Pinch/Rotate（仅 macOS/iOS；**Android 无通道**） | ◐ | — | 低 | 平台注记：Bevy 该面对本仓 Android 主战场无效；缩合可自写两指距离 |
| I8 | 光标捕获 | ✔ Confined+隐藏 | CursorOptions 等价 | ✔ | — | — | 对面打平；Locked 模式差异不触感 |

### 2.3 音频域

| # | 能力 | 本仓 | Bevy 基线 | 状态 | 平迁成本 | 纵深成本 | 风险与影响 |
|---|---|---|---|---|---|---|---|
| A1 | 后端 | ✔ tinyaudio（AAudio/DirectSound/ALSA，f32 直通） | rodio（同代 OS 打底） | ✔ | — | — | Android 落点一致（AAudio/ndk 无 JNI）；tinyaudio 依赖面更小=有意取舍 |
| A2 | 解码 | ✔ symphonia（ogg/vorbis/wav/pcm） | rodio feature 矩阵 | ✔ | — | — | symphonia 面广一档（含 mp3 开匣） |
| A3 | 声部管理 | ✔ **24 voice 上限 FIFO + 循环幂等 + 80ms 节流** | 无上限（每 sink 一 rodio 通道，爆音靠调用方自律） | ✔ | — | — | 本仓「有界+游戏纪律」是反向优势（挖矿雨不打爆） |
| A4 | 分类音量/总线 | ✘（单 master 一档） | bevy 单 GlobalVolume（仍平坦无树）；**原版=SoundSource 10 分类总线**（SoundEngine.java:117-154 实时刷） | ✘ | **低（高收益）** | — | ui-parity §4 声音选项 ⬜ 的前半：AtomicU32 per-分类乘子表+滑条 UI（见 3.1-P4） |
| A5 | 立体声/空间感 | ✘（L=R 平铺，纯距离衰减，无听者朝向） | SpatialAudioSink ears/emitter（rodio=pan+衰减非 HRTF）；**原版=OpenAL 3D+directionalAudio(HRTF 开关)**（SoundEngine.java:106） | ✘ | 中 | 中 | 现实痛点：mob 无方向感（身后的雨/脚下的岩浆分不开）；mixer 加 listener 朝向→pan 即达原版体感（3.1-P5）；OpenAL-soft 级复杂度不引 |
| A6 | 循环/BGM | ◐（loop 幂等=挖掘持续音已用） | PlaybackMode::Loop + 无缝尾循环 | ✘ | 中 | 中 | **BGM 缺失**（原版 music/record 面全缺）：stream=true 流式解码边播 + 交叉过门；整块解码会爆内存（单机 Android 红线） |
| A7 | 效果/混响 | ✘ | bevy 无内建；kira 效果链（Reverb/Echo/Filter）+ modulator 自动化 | ✘ | — | 中 | 原版主路无混响（BE 亦无）→ 非对齐债；记远期（触发=洞穴声学验收） |
| A8 | 自适应音乐 | ✘ | kira clock/Sequence 节拍切换 | ✘ | — | 中 | 原版本就无自适应音乐系统（MusicManager 单曲排程）；原版先行=曲库+定时器（随 A6） |
| A9 | HRTF/遮挡 | ✘ | bevy/kira 均无；Steam Audio 独立 SDK | ✘ | — | 高 | 超-原版增强（原版 HRTF 是 OpenAL 开关）；Android 可行性见 4.3 |
| A10 | 资产管线/防呆 | **✔ CDN sha1+校验+事件完备性测试** | bevy asset server 通用件 | ✔ | — | — | 仅对音效域，本仓纪律更强（防死音：events_table CI 真校验） |

## 3. 分级建议

### 3.1 平迁（不换骨架，局部改据，按现实痛点排序）

- **P1 输入双缓冲资源（I1+I2）**：新增 `mcv_input` 薄层——`ButtonState<VKey>`
  `press/release/just_pressed/just_released`（帧轮次清沿），app.rs 事件入口
  写入，game.rs/gameplay 门读出；held 布尔镜像逐步收编到该层。收益：输入
  语义无头可测（test 直发 press/release 断言动作），语义等价原版
  `KeyMapping.isDown`（held）+ `KeyMapping.click`（沿）。风险低：不改 winit
  流，只改消费端。
- **P2 触屏容器债清偿（U8/I5）**：slot.rs 触屏 quickdrop/snapback 两分支
  （原版 ACS:411-434/:489-519 直译待做，文档已列）+ app.rs 触屏 drag 收集；
  与 #16 同条任务线。
- **P3 StackRef 元数据收口**：组件级相等性（`isSameItemSameComponents`）
  游戏层下发一致性哈希位到 meta，或 menu 侧比较回调——修「附魔物品误合并」
  的隐性正确性债。
- **P4 分类音量（A4）**：`Mixer` voice 增 `category: u8`，master 增 10 分类
  AtomicU32 位镜像表（SoundSource.java:4-13 对位）；SoundOptionsScreen 平台
  （ui-parity §4）给 Music/Sound/Voice 四滑条起步。平迁成本最低的现实收益项。
- **P5 立体声 pan（A5 前半）**：mixer 接入 listener 朝向（yaw 向量点乘→
  [-1,1] pan，对位原版 OpenAL 声像手感），`mix_voice` 按声道增益差分。
  无听者朝向时回退现距衰减——达原版「身后知敌」的方向体感。
- **P6 EditBox/IME（U6+I6）**：mcv_ui 增 `EditBox`（光标/选区/撤回，裁剪归
  位图字体 advance；对照原版 EditBox.java，已在 docs/ui-parity 挂账），
  Android 先接 winit 的 Ime 事件（Preedit 展示，Commit 入框）——若 winit
  在 Android 的 IME 通道不通则自绘软键盘（可参考 4.1 的 feathers
  virtual_keyboard 纯布局语义）。成本中、收益硬（复现地图的种子输入解锁）。
- **P7 世界列表滚动（U7）**：`UiGraphics.scissor` 机制已在，加
  `ScrollArea`（滚轮/惯量）+ 世界列表行即界。与 ui-parity §2 挂账同探。
- **P8 屏幕栈合流（U9）**：app.rs `Screen` enum 十态 → ScreenStack push/pop
  逐屏迁移（KeyBinds 已是模板）；容器/死亡/暂停照原版 isPauseScreen 语义
  顺路挂对（清「双状态机」漂移债，属工程卫生非 Bevy 补齐）。

### 3.2 重构/纵深（触发条件立项，单独测算）

- **D1 taffy 布局引擎（U1/U3 依赖）**：taffy 0.14 纯 Rust、无原生依赖，
  引入成本低但其**换的是全部屏的构造面**（控件注册进树+测点）。触发条件：
  屏数 > 十五或控件动态化（宽度自适应/整数网格/响应不同宽高比）出现手的
  布局维护痛；届时 focus/auto_directional_navigation 才连带值得做（U3）。
- **D2 retained UI 渲染（U10）**：UI 节点树变更检测 + quad 缓存。触发条件：
  世界列表/成就大列表滚动帧耗时进 profile；HUD 管线 quad 数量过万。
- **D3 BGM 流式播放器 + 效果链（A6/A7/A8）**：先做原版对齐面（BGM=曲库 +
  stream=true 流式解码边播 + 定时排程，MusicManager 语义）；kira 级效果链
  两选一：a) 引 kira 全 manager（丢自 Voice/SPSC 架构，shim 成本高）
  b) 零散自研（简易 comb filter 混响）。触发条件：BGM 体验面被显性差评
  （先手）/ 洞穴声学验收缺口（后手）；Android 内存红线（memory-budget.md）
  全程卡流式（禁止整块解码音乐）。
- **D4 手柄（I4）**：gilrs 直引（桌面/部分 Android 外设）+ 手柄导航
  （依赖 D1 focus）远期；触发条件=桌面覆盖率反馈。

### 3.3 远期（登记不排）

- accesskit 无障碍（U4）：等桌面主体玩家反馈；
- Steam Audio HRTF/遮挡（A9）：见 4.3 Android 可行性注记，属超原版增强；
- bevy_feathers 类控件族（4.1）：语义参考不引码（取向不符）；
- 手势层（I7）：Bevy 的 Pinch/Rotate 只覆盖 macOS/iOS，Android 需要
  自研两指间距/旋转——归入触屏波，不排期。

## 4. 追加核对（用户补丁）

### 4.1 bevy_feathers 0.20.0 控件清单 vs mcv_ui

源码盘点（bevy_feathers-0.20.0/src）：

- **controls/**（19）：button、checkbox、toggle_switch、slider、radio、
  select、number_input、text_input、color_input、color_wheel、color_plane、
  color_slider、color_swatch、color_swatch_grid、scrollbar、listview、
  menu、dialog、disclosure_toggle、**virtual_keyboard**；
- **containers/**：group、pane、subpane、flex_spacer；
- **display/**：label、icon；**theme/token 侧**：tokens、palette、
  dark_theme、font_styles、rounded_corners、focus。

对拍结论：

- 覆盖面本仓缺的三块：**text_input（=U6 EditBox）、scrollbar/listview
  （=U7）、dialog（=ConfirmScreen/删除确认，ui-parity §2 ⬜）**；Feathers
  的 virtual_keyboard 对 P6 的 Android 软键盘是**纯语义参考**（键位布局/
  长按重复），不是代码移植源；
- **官方自述不推荐给游戏 UI**：lib.rs:1-12「styled for editors/inspectors…
  deliberately not intended for game's UI」——配置为编辑器观感+一致性；
- **事件模型不匹配**：Feathers 全部组件靠 `EntityEvent` 树冒泡 +
  stop_propagation（lib.rs:15-18），与本仓「事件出队到游戏层」（U11）相反
  ——**引组件必须先建冒泡层 = 重构门槛归档远期**；
- 判定：控件**语义参考价值高、代码移植价值低**；对 U6/U7 的实现顺序
  建议（光标/选区/滚轮裁剪语义）予以采纳。

### 4.2 音频中间件（分层总线/自适应音乐/混响快照）

- **kira 0.10.7 源码盘点**（对照 bevy_audio 缺的三处）：
  - 分层总线：`manager.add_sub_track` 子轨道自组 + track send 双向路由 +
  每 track 独立效果链（其 lib.rs:76-84 例）；
  - 自适应音乐：clock（节拍时钟）+ modulator（LFO/Tween 参数自动化）
  ——**原版没有自适应音乐**（MusicManager 单曲定时排程，mc-ref
    MusicManager.java），BE 才有分层音乐；对齐原版只需「曲库 + 定时 +
    交叉过门」（D3 前半）；
  - 混响快照：kira 无 FMOD 式 snapshot 概念，等价物是「track 效果链参数
    自动化」（tween Reverb mix）；原版 26.1 主音路（OpenAL 基面）
    **无混响总线**（BE 有 cave ambience）；
- **生态桥接**：bevy_kira_audio 是 kira 之上的 bevy 集成层——本仓无 bevy，
  直接引 kira 才是真对照物（bevy_audio 的分贝/sink 语义已列 1.3）；
- **FMOD/Wwise 官方/社区 Rust 绑定**：FMOD 有 fmod-oxide 等社区绑定、
  Wwise 有官方 Rust 声明（Wwise SDK 独立 so）——Android so 体积（FMOD
  ~2-4MB/声道库）+ 实时内存分配 + 许可（Wwise 按收入分成 / FMOD 需商业
  授权）超单机 MC 还原价位 → **远期外翻，不入盘**；
- 判定：本仓 SPSC+Mixer 架构对「音效纪律」已够；总线三件套平迁首点=P4
  分类音量（自 issue 不引 kira）；kira 引入=换声部管理器（D3），触发
  条件见 3.2。

### 4.3 Steam Audio（HRTF/遮挡）——Android 可行性单独注记

- 能力线：HRTF 双耳渲染（Binaural）+ 几何遮挡/散射（rays/simulation）+
  Hybrid 混响——都是原版 26.1 **没有**的层级（原版立体声位感=OpenAL
  HRTF 开关 `directionalAudio`，SoundEngine.java:106；遮挡原版无）；
- **Android 平台合规**：Steam Audio 官方 SDK 提供 Android（arm64）预编译
  so + C API 自定义后端挂法（官方平台支持表含 Android/iOS）——技术可行
  ✅；但：(a) Rust 无官方绑定，社区 sys 薄绑定须随 SDK 版本手追；(b)
  **许可**：Steam Audio SDK 免许可费（Parsita 条款）但 SDK 二进制再分发
  与随包条款必须逐条核对（发行红线同 Mojang 资产原则：DEVELOP_ONLY 域
  照样处置）；(c) CPU 预算：rays/卷积在移动端 CPU 的占用未过谱（E0 计时
  尚无音频段，先证 2ms 级余量再考虑）；
- 判定：**远期**，行位同 D3 后置——先过 P5 立体声 pan（=原版 OpenAL
  平价体感），Steam Audio 触发条件=「洞穴/室内遮挡声学」验收被单机用户
  显性提出 + Android 帧预算谱面（E0 计时）证有余量。

## 5. 参考来源

- crates.io 官方源码快照（本机 /tmp/bevyref，不入库）：bevy_ui-0.20.0 /
  bevy_input-0.20.0 / bevy_audio-0.20.0 / bevy_window-0.20.0 /
  bevy_winit-0.20.0 / bevy_feathers-0.20.0 / bevy_text-0.20.0 / kira-0.10.7
- 原版 26.1 反编译规格参考（/root/mc-ref/src-26.1，不入仓）：
  SoundSource.java（10 分类总线）、SoundEngine.java:106/117-154（HRTF 开关
  与分类音量实时刷）、net/minecraft/client/sounds/*（MusicManager 流），
  net/minecraft/client/gui/**（ui-parity.md 既有账）
- https://docs.rs/bevy_ui/latest/bevy_ui/ （0.20，taffy/Interaction/accesskit）
- https://docs.rs/bevy_input/latest/bevy_input/ （ButtonInput/Touches/gestures）
- https://docs.rs/bevy_audio/latest/bevy_audio/ （PlaybackMode/Volume/Spatial）
- https://docs.rs/kira/latest/kira/ （track/effect/clock/modulator）
- https://github.com/bevyengine/bevy （bevy_feathers 0.20 官方 workspace crate）
- 本仓既有账：docs/ui-parity.md（逐屏缺口 ⬜/🔨 面板主力）、
  docs/memory-budget.md（Android 内存红线，BGM 流式裁定关联）
