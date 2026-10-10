# UI 对标清单：原版 MC 26.1 vs 本引擎

逐屏逐功能对比结果。原版参考仅在本机反编译树 `src-26.1/`（**勿入仓**），
本文只记录类名（作规格参考）与本引擎待办，不引用任何原版代码/文本资产。
数值规格见文末「HUD 像素规格」。

图例：✅ 已对齐 | 🔨 部分实现 | ⬜ 缺失

引擎 UI 现状总评：**六个 Screen（Main/Worlds/Create/Settings/InGame/Paused）+ 程序化 HUD**。
原版核心循环所需的背包、物品栏、合成界面、血量/饥饿数据、死亡、聊天、加载进度均无；
物品数据层（mcv_item 的堆叠/工具/合成逻辑）已有但没有接 UI。

---

## 1. 标题界面（原版 `TitleScreen` / `Panorama` / `LogoRenderer` / `SplashRenderer`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 3D 全景背景（旋转场景 + 暗角遮罩） | ⬜ | 现为 dirt 平铺 ×0.4 亮度背景 |
| Logo（256×44 显示，y=30） | ✅ | `gui.rs` 已按 44/64 行裁剪 |
| Splash 黄字（-20° 脉动旋转，锚点 w/2+123,69） | ✅ | 常数一致；文案自写（合规正确做法） |
| 按钮：单人游戏 | ✅ | 宽 200/高 20 九宫格贴图一致 |
| 按钮：多人游戏 / Realms | ⬜ | 无网络层；触摸端也可先搁置（波5 或永久裁剪） |
| 左下角语言图标按钮（`CommonButtons.language`） | 🔨 | 语言入口在 Settings 里，无图标按钮 |
| 无障碍图标按钮（`AccessibilityOptionsScreen`） | ⬜ | 无 |
| 退出游戏 | ✅ | — |
| 按钮渐入动画 / 键盘焦点导航 | ⬜ | 触摸端可降级为仅渐入 |

## 2. 选择世界（`SelectWorldScreen` / `WorldSelectionList`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 可滚动世界列表 | 🔨 | 固定 5 行、无滚动、超出即不可见 |
| 条目：图标/名字/模式/上次游玩日期 | 🔨 | 只有 名字+模式；无日期与图标 |
| 选中后：游玩 / 编辑（`EditWorldScreen`）/ 删除（确认框 `ConfirmScreen`） | ⬜ | 只能点行进入；**无法删除世界** |
| 新建世界入口 | ✅ | — |

## 3. 创建世界（`CreateWorldScreen` / `DifficultyButtons` / `ExperimentsScreen`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 世界名输入框（`EditBox`） | ⬜ | 名字自动生成；触摸端需要自绘软键盘或系统 IME |
| 游戏模式三选 | ✅ | 循环按钮等价实现 |
| 难度四档（`DifficultyButtons`） | ⬜ | 引擎无难度系统（关联波4 饥饿/伤害） |
| 更多选项：种子/结构/作弊（`MoreWorldOptionsScreen`） | ⬜ | 无种子输入 → 无法复现地图 |
| 世界类型（普通/平坦 buffet，`CreateFlatWorldScreen`） | ⬜ | 仅普通 |

## 4. 选项族（`OptionsScreen` + `options/*` 子菜单）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 列表式选项页（滑条/循环钮，`OptionsList`） | 🔨 | 现为 +/− 按钮堆叠，无滑条控件（`AbstractOptionSliderButton`） |
| 视频子菜单（`VideoSettingsScreen`：FOV/粒子/阴影/实体距离/垂直同步/帧率上限/云雾距离…） | 🔨 | 只有渲染距离 + 云三态 |
| 鼠标子菜单（灵敏度/Y 反转/滚动） | 🔨 | 只有灵敏度 |
| 按键绑定（`ControlsScreen`/`KeyBindsScreen`：改键/搜索/重置） | 🔨 | KeyBindsScreen 已迁 `mcv_ui` 屏幕栈（改键捕获/冲突提示/恢复默认/保存）；搜索与逐项重置未做 |
| 声音子菜单（`SoundOptionsScreen` 分类音量） | ⬜ | mcv_audio 有混音器但无音量 UI |
| 语言列表（`LanguageSelectScreen`：可搜索全量列表） | 🔨 | EN↔CN 二态切换；34 个 key 待扩充 |
| 聊天设置/无障碍设置 | ⬜ | 依赖聊天/字体缩放功能 |

## 5. 游戏暂停（`PauseScreen`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 游戏内透明遮罩 + 标题 | ✅ | — |
| 返回游戏 / 选项 / 保存并退出 | ✅ | — |
| 成就 / 统计（`AdvancementsScreen`/`StatsScreen`） | ⬜ | 无成就系统；波5 |
| 对局域网开放（`ShareToLanScreen`） | ⬜ | 无网络；触摸端建议永久裁剪 |
| 反馈/报告按钮 | ⬜ | 建议永久裁剪（Public 仓库也不宜挂 Mojang 链接） |
| **Esc/暂停入口（触摸端）** | 🔨 | 桌面 Esc 可用；**触屏 HUD 无暂停按钮**，手机进游戏出不来（仅系统返回可救） |

## 6. HUD（`Gui` + `contextualbar/*` + `components/toasts/*`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 准星 15×15 + 攻击冷却指示（`crosshair_attack_indicator_*`） | 🔨 | 准星贴图已对；无攻击冷却环/条 |
| 快捷栏 182×22 + 选中框 24×23 | 🔨 | 布局像素级一致；**但物品是写死的 9 格方块表**（`game.rs HOTBAR` 常量 + 单把铁剑），无数量/耐久/真实物品 |
| 手持物品名悬浮（切手时 h-59 淡出） | ⬜ | — |
| 副手槽 29×24（左右两侧） | ⬜ | 引擎无副手概念 |
| 心血条（9×9、半心、多行、受伤闪烁） | ⬜ | **玩家无血量字段**，现画假满血 |
| 饥饿条（右侧镜像、饥饿闪烁变体） | ⬜ | 同上，假满值 |
| 盔甲条（9×9，血条上方 -10） | ⬜ | 无盔甲；贴图也未进 `gui.rs` SPRITES |
| 经验条 182×5 + 等级数字（`ExperienceBarRenderer`） | ⬜ | `player_xp` 已累计但**从不显示** |
| 骑乘心 / 山羊角条 | ⬜ | 低优先 |
| F3 调试（`DebugScreenOverlay` + `components/debug/*` 全套条目） | 🔨 | 现为一行常驻 XYZ；原版是 F3 开/关 + 左右两栏数十条目 |
| 损伤红晕/水下/创造蓝灰等全屏效果 | ⬜ | 依赖血量 |
| 弹窗 toast（成就/进度，`components/toasts/*`） | ⬜ | 波5 |
| Boss 栏（`BossHealthOverlay`）/ 计分板 / Tab 列表 / 字幕 | ⬜ | 无对应系统；波5 |
| 触屏摇杆 + 跳/挖/放按钮（本引擎自加） | ✅ | 原版触屏对应 BE 布局，我们的占位可接受 |

## 7. 死亡界面（`DeathScreen`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 红色渐入遮罩 + 标题 | ⬜ | 无（玩家本来不会死） |
|  respawn 按钮（延迟可点）/ 回标题 | ⬜ | `game.rs` 仅 `hardcore_death` 标志 + 传送出生点 |
| 极限模式变体（只能回标题） | 🔨 | 有极限标志无界面 |
| 死亡时展示计分板 | ⬜ | 波5 |

## 8. 聊天与命令（`ChatScreen` / `ChatComponent` / `CommandSuggestions`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 输入框（4, h-12, w-8, 12）+ 历史翻阅 + Tab 补全 | ⬜ | 单机也需命令（gamemode/tp/time） |
| 聊天记录淡出/展开、动作栏文本 | ⬜ | — |
| 触摸端降级 | — | BE 方案：屏幕角落小聊天按钮 + 系统 IME；命令建议可砍 |

## 9. 加载/进度（`ProgressScreen` / `LevelLoadingScreen` / `ConnectScreen`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 建世界进度条 + 阶段文案（`menu.working`） | ⬜ | 现在点创建后直接进世界，首帧前黑屏 |
| 地表加载绿色进度条 + 标题 | ⬜ | 区块边流边渲，可接受但**首进无反馈** |
| 触摸端 | — | 必须做：移动端读条是"没卡死"的唯一信号 |

## 10. 背包 / 创造 / 合成（`inventory/*` + `recipebook/*`）

| 原版元素 | 状态 | 差距 |
|---|---|---|
| 生存背包 `InventoryScreen`：盔甲 4 格 + 副手 + 2×2 快合成 + 主背包 27 + 快捷栏 | ⬜ | E 键在 keymap 里已定义 `Action::Inventory`，无界面 |
| 创造物品栏 `CreativeModeInventoryScreen`（195×136 网格 + 分页页签 + 销毁槽 (173,112) + 搜索） | ⬜ | 现为固定 9 方块，选不到其他方块 |
| 工作台 `CraftingScreen` 3×3 + 输出格（`mcv_item::crafting` 逻辑已有！） | ⬜ | 只差 UI 粘合层 |
| 配方书（`recipebook/*`，知识解锁 + 一键合成） | ⬜ | 波5 |
| 方块容器 UI（箱子/熔炉 `ContainerScreen` 176×(114+18×行)） | ⬜ | 等箱子/熔炉方块存在再说 |
| 拖拽/拆分堆叠/Shift 快速移动（`AbstractContainerScreen` 交互模型） | 🔨 | `mcv_ui::slot` 容器槽位状态机已建（doClick 纯数据移植 + 三类拖拽分发 + quickcraft）；背包屏接线待做（现 craft_ui 临时接线迁过去） |
| 掉落物拾取（挖掘产物进背包，`ItemEntity`） | ⬜ | 现在挖掉方块直接消失，无掉落逻辑接物品 |

## 11. 其它原版 Screen（对照后建议处理）

| 原版类 | 建议 |
|---|---|
| `WinScreen`（终龙） | 波5+，等末地 |
| `DisconnectedScreen`/`DirectJoinServerScreen`/multiplayer/* | 无网络，永久裁剪 |
| `AbstractSignEditScreen` | 等有告示牌方块；触摸端要软键盘 |
| `AdvancementsScreen`/`StatsScreen`/`social/*`/`telemetry/*` | 裁剪或波5 |
| `CreditsAndAttributionScreen` | 波5 顺手（署名页，合规上还加分） |

---

# 优先级

## 波3 —— 视觉可感知大缺口（玩家一眼看出"不像 MC"）

| # | 项 | 原版类参考 | 本引擎改动文件 | 量 | 触摸端处理 |
|---|---|---|---|---|---|
| 3.1 | 玩家血量/饥饿/伤害结算 + 真心/半心/盔甲/饥饿渲染（含受伤闪烁） | `Gui.extractPlayerHealth`、`LivingHurtDuration` 规格 | `mcv_app/src/game.rs`（Player 字段+build_hud）、`mcv_entity/src/combat.rs`（伤害打到玩家）、`mcv_render/src/gui.rs`（补 armor_*/heart_half 已有一半） | L | 无特殊；心在摇杆上方不冲突 |
| 3.2 | 经验条 + 等级数字（xp 已有数据） | `ExperienceBarRenderer` | `mcv_app/src/game.rs`、`mcv_render/src/gui.rs`（exp_bar 两贴图） | S | 无 |
| 3.3 | 死亡界面（红雾、重生/回标题、极限变体、重生点） | `DeathScreen` | `mcv_app/src/app.rs`（新 Screen::Death）、`game.rs` | M | 按钮加大命中区（≥44dp） |
| 3.4 | 加载进度屏（创建世界绿色进度条 + 阶段文案） | `ProgressScreen`/`LevelLoadingScreen` | `app.rs`、`game.rs::stream`（进度回报） | M | 必做；纯展示无输入 |
| 3.5 | 暂停按钮上触屏 HUD + 手持物品名 + 准星攻击冷却环 | `PauseScreen`、`Gui.extractSelectedItemName` | `app.rs`、`game.rs`、`touch.rs`、`gui.rs` | M | 右上角 44dp 暂停钮 |
| 3.6 | 调试面板 F3 开关化（默认关，多条目分栏） | `DebugScreenOverlay` | `game.rs::build_hud`、`app.rs` 接线 | S | 改成设置页开关 |

## 波4 —— 玩法闭环（没有就"不是游戏"）

| # | 项 | 原版类参考 | 改动文件 | 量 | 触摸端处理 |
|---|---|---|---|---|---|
| 4.1 | 通用容器/背包 UI 框架：18px 槽网格、悬停提示、点选-放置两段式拖拽、Shift 等效钮 | `AbstractContainerScreen`/`Slot`/`Tooltip` | `app.rs`（新 Screen::Container）、`mcv_render/src/gui.rs`（container_background/slot 九宫格）、新 `mcv_app/src/ui/container.rs` | L | 点选代替拖拽（BE 同款） |
| 4.2 | 生存背包 E：盔甲 4 + 副手 + 2×2 合成 | `InventoryScreen` | 同上 + `mcv_item/src/crafting.rs` 接入 | L | E 键→背包钮上 HUD |
| 4.3 | 创造物品栏：分页网格 + 当前手持置顶 + 销毁槽；替换 `game.rs HOTBAR` 写死表 | `CreativeModeInventoryScreen` | `game.rs`、容器 UI 复用、`assets/minecraft/textures` 方块/物品图标源 | L | 网格加大格距；页签横滑 |
| 4.4 | 挖掘掉落 → 物品实体 → 拾取入包 → 快捷栏显示数量/耐久条 | `ItemEntity`、`Gui` 耐久条规格 | `mcv_game`、`mcv_item`、`game.rs` | L | 自动吸附拾取（BE 默认开） |
| 4.5 | 工作台 3×3 界面（合成逻辑已有） | `CraftingScreen` | 容器 UI 复用 | M | 同点选式 |
| 4.6 | 命令/聊天（单机命令：gamemode/time/tp/give） | `ChatScreen`/`Commands` | `app.rs`（EditBox 自绘）、`mcv_item` give | M | 按钮呼出 + 系统 IME；Tab 补全裁剪 |
| 4.7 | 饥饿消耗/吃食物/难度 | `FoodData` 规格、`DifficultyButtons` | `game.rs`、`mcv_item` 食物表 | M | 食物放快捷栏点按 |

## 波5 —— 打磨补齐

| # | 项 | 原版类参考 | 改动文件 | 量 | 备注 |
|---|---|---|---|---|---|
| 5.1 | 标题全景背景 + 遮罩 | `Panorama`/`PanoramaOverlay` | `mcv_render`、`app.rs` | M | 低模旋转场景即可 |
| 5.2 | 世界列表完善：日期/图标/删除(确认框)/编辑(改名+种子) | `SelectWorldScreen`/`ConfirmScreen` | `app.rs` | M | 删除确认在触摸端必做 |
| 5.3 | 选项子菜单族：视频(FOV/粒子/阴影/帧率上限)/声音/按键(改键 UI)/语言搜索列表；滑条控件 | `VideoSettingsScreen` 等、`AbstractOptionSliderButton` | `app.rs` 或新 `ui/options.rs`、`keymap.rs` 接线 | L | 改键在触摸端裁剪为预设布局 |
| 5.4 | 创建世界：名字输入 + 种子 + 难度 | `CreateWorldScreen`/`EditBox` | `app.rs` | M | EditBox 软键盘是共同依赖 |
| 5.5 | 通用控件库：EditBox/滑条/复选/滚动列表/确认弹窗 | `EditBox`/`ScrollLayout`/`PopupScreen` | 新 `mcv_app/src/ui/widgets.rs` | L | 3.x/5.x 多项的前置，可与 4.1 合并排期 |
| 5.6 | toast 成就弹窗 + Boss 栏 + Tab/计分板 | `components/toasts/*`、`BossHealthOverlay` | `game.rs`/`gui.rs` | M | 随对应系统 |
| 5.7 | 配方书、旗帜/染色等长尾界面、署名页 | `recipebook/*`、`CreditsAndAttributionScreen` | — | S~M | 署名页合规加分 |
| — | 裁剪清单（建议永久不做）：Realms、多人、反馈/举报、局域网开放、皮肤定制 | — | — | — | 与"移动端优先 + Public 仓库"定位一致 |

---

# HUD 像素规格（1× GUI 单位；来源 `Gui.java`/`ContextualBarRenderer.java`/`AbstractContainerScreen.java`，26.1 实测数值，非代码）

坐标约定：w/h = GUI 缩放后宽高；y 向下为正。

## 快捷栏
- 面板贴图 `hud/hotbar.png`：**182×22**，位 (w/2−91, h−22)。
- 槽距 **20**（槽内可视 18）；物品图标 **16×16**，位 (w/2−90+i·20+2, h−19)。
- 选中框 `hotbar_selection.png`：**24×23**，位 (w/2−92+sel·20, h−23)。
- 副手槽 **29×24**：左位 (w/2−91−29, h−23)，右位 (w/2+91, h−23)。
- 攻击冷却指示（快捷栏模式）：选中槽上 **18×18** 背景+进度，进度自下向上（高 p 时 y=槽底−p）。
- 手持物品名：水平居中，y = h−59（创造 +14 → h−45）；按稀有度着色；10 tick 线性淡出（alpha=timer·256/10）。

## 生命/饥饿/盔甲
- 心：精灵 **9×9**，横向步进 **8**；基准 xLeft = w/2−91，yLineBase = h−39。
- 行数 rows = ceil(最大血量 ÷ 20)（每行 10 心）；行高 = **max(10−(rows−2), 3)**；第 r 行 y = yLineBase − r·行高。
- 半心/空心分层绘制；受伤闪烁：受击后 10~20 tick 内每 3 tick 交替（先画空心容器再画心）。
- 盔甲条：9×9 步进 8，y = yLineBase − (rows−1)·行高 − **10**。
- 饥饿：右起镜像，x = xRight − i·8 − 9，xRight = w/2+91，行基线与心相同；饱和度为 0 且饥饿 ≤ 半程时按 `tickCount % (food·3+1) == 0` 用 `*_hunger` 闪烁变体。
- 骑乘心：同样 9×9/步进 8，上限 30 颗。

## 经验
- 经验条精灵 **182×5**，位 ((w−182)/2, h−29)；进度宽 = round(progress·183) 裁剪绘制；等级 >0 且近期获得经验时才显示（保持 **100 tick**）。
- 等级数字：居中，y = h−35；绿色前景 + 四向 1px 黑色描边。

## 准星
- `hud/crosshair.png` **15×15**，位 ((w−15)/2, (h−15)/2)。
- 准星攻击指示：满圆 **16×16**；充能条 **16×4**（背景+进度），画于准星中心偏移处；攻击间隔 > 5 tick 的武器才显示满圆提示。

## 容器界面（波4 背包用）
- 面板：`leftPos=(w−imgW)/2, topPos=(h−imgH)/2`；通用箱 **176×(114+18·行数)**；槽网格步进 **18**；"物品栏"标题位 (8, imgH−94)；面板外点击即关闭。
- 创造物品栏：**195×136**（快捷栏区另接下方）；销毁槽位于面板内 (173, 112)；顶部页签行。

## 字体/按钮（既有 NOTES-ui.md 数值，已验证）
- ascii.png 128×128，16×16 格、每格 8×8；ascent 7，行高 9，空格宽 4；阴影偏移 (1,1) 亮度 >>2。
- 按钮 `button*.png` 200×20，九宫格 border=3；文字边距 2，高 20；悬停 0xFFFFA0、常态 0xA0A0A0、禁用 0x7F7F7F。
- 通用按钮宽 200（本引擎 `menu_ui` 常数一致）；GUI auto scale = max(2, floor(h/240))。

---

*维护方式：每波完成后回表把 ⬜/🔨 改 ✅ 并在行尾追加 commit 摘要；新增 key 同步 `data/lang/extra.json` 后跑 `ci/gen-lang.py`。*
