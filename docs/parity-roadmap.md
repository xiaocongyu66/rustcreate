# MC 26.1 → mcv-engine 逐模块对齐路线图

参照源：本机完整反编译（6882 个 .java，仅机制/常数参考，源码绝不入仓）。
原则：**逐模块主动对齐**，每模块 = 参照阅读 → 机制笔记（mc-ref/NOTES-*.md）→ 再实现 → 表驱动测试 → CI 绿。
状态：✅ 已对齐 | 🔨 进行中 | ⬜ 未开始 | ➖ 超出范围（红石/村民/维系统等按里程碑裁剪）

## 世界与生成

| 参照包（文件数） | 机制主题 | 引擎落点 | 状态 |
|---|---|---|---|
| world/level/levelgen (425) | 噪声列/密度函数/洞穴/树/装饰物 | mcv_worldgen + cpp/terrain | 🔨 波1-B 常数校准；Rust 地形内核已落地（rust_terrain/rust_noise，oracle 对拍锁定，默认切换待任务板） |
| world/level/chunk (54) | 区块分段存储、heightmap、section 序列化 | mcv_core Chunk + mcv_save | 🔨 波1-E |
| world/level/lighting (15) | 天光柱/衰减/发光表/removal 队列 | mcv_light | 🔨 波1-C |
| world/level/block (461) | 方块属性（硬度/爆炸抗性/发光/遮挡/tick） | mcv_core BLOCKS | 🔨 波1-E |
| world/level/material (11) | 流体流动/源判定 | mcv_game 或新模块 | ⬜ 波4 |
| nbt (43) + 存档格式 | 结构/实体持久化 | mcv_save（现为自研 RLE，评估是否补 NBT 读结构） | ⬜ |

## 实体与玩法

| 参照 | 主题 | 落点 | 状态 |
|---|---|---|---|
| world/entity/LivingEntity+player (708 中物理相关) | 重力/阻力/步进/游泳/坠落伤害 | mcv_game 物理 | 🔨 波1-D |
| player 挖掘 (getDestroySpeed/digProgress) | 硬度公式/工具/挖掘阶段 crack 0-3 | mcv_game 交互 | 🔨 波1-D |
| world/entity 敌对生物 (zombie/skeleton/creeper/spider…) | AI 状态机（游走/索敌/攻击/受击退）、刷怪规则（亮度/距离上限） | mcv_entity | ⬜ 波3 |
| item 投掷物/掉落物 (item/entity) | 掉落物实体、拾取范围、堆叠 | mcv_entity + mcv_item | ⬜ 波3 |
| inventory (player 内) | 36 槽快捷栏+背包、合成网格 | mcv_item + UI | ⬜ 波3 |
| food/health | 饥饿、血量、死亡重生 | mcv_game 生存面 | ⬜ 波3 |

## 渲染与表现

| 参照 (client/renderer 682) | 主题 | 落点 | 状态 |
|---|---|---|---|
| 区块网格/烘焙 | 贪心+AO+面明暗 | mcv_mesher/cpp | ✅ |
| fog/sky/光照曲线 | 雾、昼夜、gamma | mcv_render | ✅ 基础；🔨 曲线常数核对（波2） |
| CloudRenderer | 体素云层 | mcv_render 新 pass | ⬜ 波2 |
| entity/PlayerRenderer | 六盒体玩家模型+行走动画、steve/alex 皮肤 | mcv_render + mcv_game | ⬜ 波2（任务 #12） |
| Camera | 第一/第三前/第三后 距离与偏移 | mcv_game 相机 | ⬜ 波2（任务 #11） |
| particle (81) | 破坏粒子、方块破裂 overlay | mcv_render + mcv_game | ⬜ 波3 |
| screeneffect（受伤红/水下蓝） | 全屏叠加 | mcv_render HUD | ⬜ 波3 |
| textures 原版材质 | 开发期贴图 | assets/minecraft/ | 🔨 波1-A（发布前移除） |

## 界面与交互

| 参照 (client/gui 425) | 主题 | 落点 | 状态 |
|---|---|---|---|
| Screen 栈（主菜单/世界列表/设置/暂停） | 菜单状态机 | mcv_app menu_ui | ✅ 基础版 |
| Options（视角/距离/FOV/敏感性/云） | 设置持久化 | mcv_app settings.json | 🔨 部分；⬜ 波2 扩展 |
| 语言本地化 | i18n 表 | mcv_app | ⬜ 波2（中文字体为前置问题） |
| 键位重映射 | KeyMapping | mcv_app | ⬜ 波2 |
| F3 debug 页 | 坐标/chunk 统计/帧时 | mcv_app HUD | ⬜ 波2 |
| 聊天/命令 | 裁剪 ➖（单机无服务器指令面） | — | ➖ |

## 声音（client/sound + sounds.json）

| 主题 | 落点 | 状态 |
|---|---|---|
| 方块脚步/挖掘循环/放置/跳跃落地 | mcv_audio（新 crate，依赖选型待定） | ⬜ 波2（任务 #8） |
| 音乐/环境声 | mcv_audio | ⬜ 波5（低优） |

## 移动端

| 主题 | 落点 | 状态 |
|---|---|---|
| 摇杆/按钮/拖屏视角 | mcv_app touch | ✅ |
| 视角切换按钮、云开关进设置 | touch + menu_ui | ⬜ 波2 |
| 高分屏 surface 钳制（本次闪退根因） | app.rs fit_surface_size | ✅ 24c947c |

## 里程碑绑定

- 波1（当前并行）：常数/材质对齐 → 数据面可信
- 波2：视角+模型+云+F3+设置/键位+声音 → 表现面可信（M5 一部分）
- 波3：生物 AI、掉落物、背包 UI、饥饿血量 → 玩法闭环（M5 完成判据）
- 波4：流体、NBT 结构、更多方块族
- 波5：性能优化与发布（M6）
