# Bevy 差距审计：任务调度 / 资产 / 场景体系

> 审计日期：2026-10-17。只读侦查 + 本文档，不改游戏代码。
> 仓库撞线备注：原定分支名 `bevy-gap-work` 已被平行线（bevy 五域审计：物理/动画/picking/诊断/网络，见其 `docs/audit/bevy-ecs.md`、`bevy-physics-misc.md`）占用并 force 级翻动中；本审计（调度/资产/场景三域）改发姊妹分支 `bevy-gap-sys-asset`（基线 origin/main a116c019），文档路径按原令 `docs/audit/bevy-sys-asset.md`。
> **终态基线声明**：本仓正执行「自研 TaskPool → rayon 替换」（另一线负责，本审计不动其面）。
> 所有差距矩阵与衔接点均以 **rayon 已引入** 为前提评估；
> `docs/engine-roadmap.md` §3-E1「不引入 rayon」与 §2 末「伪需求」段落已成失效文档，待其收口时一并改写。
> 证据截取自 2026-10-17 ~09:45 的工作副本（rayon-switch == 当时 origin/main 73712828）。

## 0. 勘察面速览（证据）

| 面 | 位置 | 现状一句话 |
|---|---|---|
| TaskPool v2 | `game/mcv_core/src/pool.rs` | Mutex+Condvar 手写池；hi/lo 双优先级队列；`catch_unwind` panic 隔离（worker 不减员，pool.rs:188-197）；`stats()` 三计数遥测；worker 数 = 原版 `clamp(cpu−1,1,max.bg.threads)`（Util.java:187-189 镜像，pool.rs:217-237） |
| 三池分立 | `game/mcv_logic/src/game.rs:1709,1556-1570` | terrain/mesh/light 各建**专属** TaskPool，默认各取 `world_worker_count()` → 8 核机 = 3×7=21 个 worker 线程，外加 E4 IO 专线与音频 RT 线程 |
| E4 IO 线程 | `game/mcv_logic/src/game.rs:1343-1420` | 专属 std::thread（刻意非 TaskPool，注释言明「IO 与生成争 worker 会互饿」）+ mpsc 命令/回执 + seq 配对 + 高水位 64 |
| mcv_sync | `engine/mcv_sync/src/lib.rs` | 手写 Lamport SPSC 有界环（音频 RT 通道 mcv_audio:220）；acquire/release 全配对；`!Sync` 句柄结构性保证单读单写 |
| 手写调度器×3 | `game/mcv_mesher/src/sched.rs`、`game/mcv_light/src/sched.rs`、`game/mcv_worldgen/src/lib.rs`（TerrainScheduler） | 统一「快照-计算-回传」三段式：主线程快照字节进池，worker 纯计算，产物经 mpsc 回传；seq 提交地板拒僵尸/陈旧产物；LossHook 三态（Deliver/Swallow/Panic）注入测试（pool.rs:162-172）；光照边同步刻意留主线程串行（sched.rs:37-44 五项时序假设） |
| 主循环 | `game/mcv_app/src/app.rs`（3579 行） | winit 0.30 ApplicationHandler 直驱；redraw 帧喂 60Hz fixed_step；`mcv_logic::accumulate_ticks`（game.rs:1327-1334）按 `frac += dt*20` 折算 20Hz 原版 tick，单帧补 tick 上限 4，超出即丢（不追帧）；暂停门走原版 `isPauseScreen` 映射 |
| mcv_ecs | `engine/mcv_ecs/src/*`（~1100 行） | Godot 语义小型 ECS：Fixed(1/60)+Variable 双阶段、RefCell 运行期冲突检查、CommandQueue 帧末变更、事件双缓冲**逐阶段**翻转、Resources 逐步所有权快照（E6 起 Arc 快照零表复制）；**仅覆盖 mob 子系统**（`GameRuntime.mobs_app`，game.rs:1753,1820-1858），其余玩法在 game.rs 手工编排 |
| AssetManager | `engine/mcv_assets/src/lib.rs`（490 行） | 同步 `read/read_optional` + `RefCell` 缓存（**!Sync，单线程**）；缺素材硬错误且按路径去重登记 + `summary_report` 一次性上报；**实例分裂**：Renderer 内部（gpu.rs:1587）、皮肤（app.rs:2191）、音效根（app.rs:1433 附近）各持一份管理器，缓存互不相通 |
| 热重载（#58 项） | `engine/mcv_render/src/gpu.rs:3036-3050`、`mcv_assets/src/lib.rs:245-248` | CPU 半边（`rebuild_atlas_payload`）已通，**GPU 纹理数组换绑未接**（注释自认「仍需整体重建 Renderer 才生效」）；**零文件监听**；`read_cache_dedups` 测试（mcv_assets:453-466）反向锁定「盘上改写后仍回旧字节」——缓存语义与热重载互斥。（注：#58 为编排侧内部审计台账号，仓内 issues 无对应项；#54/#152 同风格由代码注释引用） |
| Android 资产 | `game/mcv_app/src/android_assets.rs` | 首启 APK assets → internal data 全量解包；manifest+stamp 校验（失败不落 stamp，防残缺永久缓存死局）；资源根解析顺序 env→Android base→CWD→exe（mcv_assets:129-161） |
| 存档 | `engine/mcv_save/src/lib.rs`（751 行） | 自研格式：`level.meta` 手写二进制（v6：seed/名/day_time/mode/player/天气）+ `.mcrv` region（512 项头 + RLE u16 域 + heightmap 尾）；**仅持久化 地形/玩家/背包/天气/统计**；mob、掉落物、经验、效果、方块实体 NBT 一律不落盘 |
| mcv_nbt | `engine/mcv_nbt/*`（~2500 行） | 完整 NBT/SNBT 读写 + Accounter 栈安全口径（KNOWN-DIVERGENCE NBT-* #177：CESU-8 残余、写侧递归限深等），**当前唯一消费者是原版结构模板 .nbt**（mcv_worldgen::vanilla::structure）——存档侧完全没用它 |
| 平台层 | `engine/mcv_platform/` | 仅输入翻译（VKey/触屏摇杆），winit 事件循环、窗口生命周期、GLES surface（2048 纹理上限缩进）全在 mcv_app 单文件里；无分窗抽象、无渲染线程（draw 段在主线程，FrameTiming game.rs:757-820） |

## 1. 与 Bevy（0.17/0.18 现状）的模型对照

### 1.1 调度模型：fixed tick vs FixedUpdate

- **本仓**：60Hz 物理步（app 层逐帧喂步）与 20Hz 逻辑 tick（`accumulate_ticks` 折算）双轨并存；tick 语义散在 game.rs `fixed_step` 各处（`on_tick` 门 + 步数换算注释「4 tick = 12 步」）。**mcv_ecs 的 `DEFAULT_FIXED_DT=1/60` 注释称「与原版 tick 率一致」（schedule.rs:18-19）——事实错误**（原版 20 TPS），真实折算另在 accumulate_ticks；两套追帧策略也不一致（mcv_ecs schedule.rs:117-132 丢历史 acc=0；game.rs accumulate_ticks 裁 `n.min(4)`，即单帧最多补 4 tick、超出静默丢）。
- **Bevy**：`Update`（可变帧）/`FixedUpdate`（独立运行表，`Time<Fixed>` 累加器 + `max_delta_steps` 有界追帧，0.15+）清晰分立；tick 语义 = 固定内积，插值走 `Time<Virtual>`。本仓 60/20 双轨是 FixedUpdate 的手工版，但分散、有注释漂移、追帧策略两处不一。
- **多线程系统执行**：Bevy 系统经访问冲突推导自动并行（0.16→0.17 迁移指南明载「系统并行策略变更」），确定性靠显式 ordering；本仓系统单线程串行，确定性由构造保证（mcv_ecs lib.rs:35-37 明记取舍：RefCell 运行期冲突检查语义等同 Bevy query 冲突检查）。
- **池模型**：Bevy 是按类分池（ Io/AsyncCompute/Compute）+ 统一调度；本仓是三个子系统池 + IO 专线，**无统一入口、无负载均衡**（仅 stats() 遥测）——这正是 rayon 迁移（终态基线）要收的口。

### 1.2 资产异步模型与热重载

- **Bevy**：`AssetServer` 同步 facade 背后是异步 IO（任务池 + futures），`Handle<T>`/`AssetId` 生命周期 + 依赖图 + `AssetEvent`（Added/Modified/Removed）广播；`file_watcher` 特性做全资产热重载（0.17/0.18 持续修补 reload 事件语义）；`AssetProcessor` 离线加工（meta 稳定 UUID）；加载态可查（LoadState/RecursiveDependencyLoadState）。
- **本仓**：加载 = 主线程阻塞 `std::fs::read`，启动期一次性吃满；无句柄生命周期（`Arc<[u8]>` 即引用计数，但无人发布「资产已换」事件）；缓存永不清除且**多实例碎片化**；热重载仅图集 CPU 半边 stub，纹理数组重建绑在「渲染线程重构」上未排期；重载后果面（mip/层重映射、材质重绑）无事件契约；无加载态查询（loading_ui 是进度特例，非通用门）。
- **Android 特例**：APK 内资产不可原地改，热重载在 Android 事实上无法走文件监听路线（解包目录可以，但现有 stamp 机制按 manifest 全量重解 = 「重装」非「重载」）。

### 1.3 序列化 / 场景（Scene）

- **Bevy**：Reflect + DynamicScene ↔ serde ↔ RON；组件/资源可架构无关地落盘与恢复，是无头编辑器与存档复用的基石。
- **本仓**：ECS 完全无序列化面（组件无 derive、World 与渲染/存档状态耦合在 GameRuntime 手写字段）；存档是逐子系统手写二进制（RLE 域、`CHUNK_VERSION=3`、字段追加式演进 v1→v6），**与原版 .mca/level.dat NBT 零互通**（mcv_nbt 栈已号而不用）；实体态不持久化（重进档 mob/掉落物全蒸发——原版行为是 entities/*.mca 落盘）。
- **结论**：本仓没有「场景」概念；一切存取 = 逐案手写。新增可持久化玩法状态的成本 = 改 mcv_save + 抬版本号 + 双侧解码，无反射红利。

### 1.4 Windowing/平台

- **Bevy**：bevy_winit 封装 winit 0.30（与本仓同代），窗口事件经 internal 再分发，多窗、渲染分线程（PipelinedRendering render_app）。
- **本仓**：winit 0.30 直驱 + wgpu 30 直驱，Android native-activity 与 Bevy 同特性；生命周期处理（resumed/suspended 后台节流、30s autosave 墙钟冻结）已真机化但**埋在 app.rs 3500+ 行内**，与 UI/玩法/资产解析纠缠；`mcv_platform` 只收输入，未收窗口/渲染平台差异，GLES 适配无跨壳复用位。

## 2. 差距矩阵 + 成本/风险 + rayon 衔接

成本（S/M/L/XL）与风险（低/中/高）按「动到的行为面」估；「衔接」= rayon 迁移线的直接受益/重叠点。

| # | 维度 | 差距 | 成本 | 风险 | rayon 衔接 |
|---|---|---|---|---|---|
| G1 | 调度池 | 三池+IO 线程 → 统一全局池；各自 `world_worker_count()` 超订（8 核 ≈ 21+ worker 线程） | M | 低-中：回滚开关（`MCV_*_WORKERS=0`）已体系化 | **直接**：rayon 全局池即收口；hi/lo 优先级需迁为记账层语义（rayon 无优先级队列）；`stats()` 遥测面需重建 |
| G1a | panic 隔离 | TaskPool 的 worker 存活 + 失败回执纪律（DC-3，docs/worker-accounting.md）在 rayon 下无对应物（rayon 任务 panic 上抛 to scope） | S | 中：纪律丢失会复发黑色三案（隐形墙/黑块/生成停摆） | **直接**：迁移线必须建 catch_unwind 包装层，保 pool.rs:174-197 语义；LossHook 测试面随行 |
| G1b | tick 语义 | 60/20 双轨散落 + mcv_ecs 固定步注释错误 + 两套追帧策略不一 | S | 低：纯常量/文档收口 | 无 |
| G1c | 系统并行 | mob 系统串行；Bevy 式并行系统需求缺证据（engine-roadmap E6 自署搁置条款） | XL | 高：确定性 golden 面重划 | 间接：rayon 到位后才有 per-system 并行的可选路径，仍维持 E6 搁置 |
| G2 | 资产异步 | 阻塞式单线程加载、无 Handle/事件契约、缓存碎片化（三实例三缓存） | L | 中：!Sync RefCell 结构锁死单线程，改共享需换结构 | 间接：资产 IO 可投 rayon blocking 面，**先决** = AssetManager 去单线程化（RefCell→并发缓存） |
| G2a | 热重载 | GPU 换绑未接、无文件监听、缓存反语义（dedup 测试锁死旧字节）、Android 无重载路线 | L | 中：渲染线程重构是前置（已在排队） | 无 |
| G3 | 场景/序列化 | 无反射、ECS 不可落盘、实体不持久化、存档与原版 NBT 不通 | XL | 高：格式迁移 + 旧档兼容；特定分叉已在 KNOWN-DIVERGENCE 登记体系内 | 无 |
| G3a | mcv_nbt 复用 | NBT 栈已号（#177）但存档未接——最小互通路径 = level.meta 换 NBT 编码或 region 对齐 .mca，二选一 | L-M | 中：双格式期需版本路由 | 无 |
| G4 | 平台面 | app.rs 单文件巨型编排、无渲染线程、无多窗抽象 | XL | 高（渲染线程重构已另列在排） | 无 |

## 3. 三大差距（终报底稿）

1. **调度：池分立 + 纪律无宿主**——三池超订与 panic/回执纪律全锁在 TaskPool v2 实现里。rayon 迁移是换「池」不能丢「纪律」：catch_unwind 包装 + DC-3 失败回执是迁移线的第一接缝（G1/G1a 直接受益：线程数、负载均衡、观测重锚）。
2. **资产：同步单线程 + 半成品热重载**——Bevy 式 Handle/异步/事件契约全缺；#58 收口先决是渲染线程纹理重绑；AssetManager 单线程化（RefCell 结构）是后续任何并行 IO 的结构性债务。
3. **场景：零反射零实体落盘**——存取全是手写特例（.mcrv + level.meta），mcv_nbt 号而不用；对拍 Bevy Reflect/Scene 是三差中迁移成本最高档，建议按 G3a 最小路径（存档接 NBT 栈）起步，而非先扯 ECS 反射。

## 4. 建议收口顺序（供编排参考，本审计不动手）

rayon 收口（含 G1a 纪律重建）→ G1b 文档/常量修正（S 级顺手）→ G3a 存档接 NBT 栈 → G2/G2a 等渲染线程重构窗口合并处理 → G1c 维持 E6 搁置条款。

## 5. 补强证据（二次核查）

- **ECS 零序列化面**：`engine/mcv_ecs/` 全 crate 无 `serde` 引用；组件仅 `'static` 约束无 derive（lib.rs:95-98）——G3「无反射」坐实。
- **RNG 非确定种子**：`spawn_rng()`（game.rs:7994）以 `fast_rand()`（进程熵）作种子逐调用点独立起链——单线程现状下 mob 掉落/自然刷怪/天气本就不可复现；G1c 若开并行需先给确定性 RNG 通道（对比 Bevy 生态惯例：入口注入种子化 Rng 资源），否则 golden 面无法延伸到 mob 域。
- **确定性锁域窄**：worldgen 的 E2 golden 锁只覆盖地形生成域（tests/os2s_golden 等），调度域（tick 折算/追帧）、资产域（缓存语义）、存档域（v6 解码默认）无同规格对拍——Bevy FixedUpdate 的可测性（Time<Fixed> 直接驱动 headless 运行表）是收口 G1b 时可顺带引入的测试面。
