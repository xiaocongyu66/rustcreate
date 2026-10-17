# 渲染管线 vs Bevy Render Stack 差距审计（mcv_render）

- 分支：`bevy-gap-work`（本文档唯一改动面；只读侦查 + 文档，零代码改动，禁 cargo）
- 日期：2026-10-17
- 对拍基准：**Bevy 0.19**（2026-06-19 发布，[Release Notes](https://bevy.org/news/bevy-0-19)）；部分命名引自 0.20-dev 文档（[Tainted Coders](https://taintedcoders.com/bevy/rendering)），标注为 dev。
- 证据面：本仓所有行号取自 `fluid-flow2@0777b34a` 工作树（与本分支同源）。

---

## 0. 关键结论先行

1. **Bevy 0.19 起 RenderGraph 已被 ECS 调度取代**（PR #22144 "Render Graph as Systems"）：渲染 pass 现在是渲染世界上的普通系统，排在 `Core3d`/`Core2d` 调度的 `Prepass`/`MainPass`/`EarlyPostProcess`/`PostProcess` set 里。本仓"手写 8(11) pass + 硬编码顺序"的对标物不是 Node 图，而是**带依赖边的系统调度**——两者的共同本质是 DAG 依赖调度，差异在于本仓的边写在注释里，Bevy 的边写在 `.after(Core3dSystems::MainPass)` 这种机器可校验的约束里。
2. **三大结构性差距**：① 材质/绑定系统（per-chunk 256B dynamic-offset uniform vs `AsBindGroup` 材质管线 + 0.19 partial bindless + GPU-driven batch sets）；② 资源生命周期（手写池/世代号/债账 vs `AssetServer`/`Handle` 强引用 GC + 依赖图加载）；③ 多线程渲染准备（主线程单线程装配 vs Extract（阻塞同步点）→ Prepare/Queue 并行系统 + 渲染世界分离）。
3. **可复用面意外地大**：E0 九段帧计时纪律、字节账（`BatchingStats` 瞬态峰值口径）、保留环逐出契约（#174 隐形墙）、staging 环分帧上传思想——这些是**方法论资产**，与 Bevy 迁移正交；WGSL 着色器本体可低成本搬家（Bevy 用 naga，同语言）。

---

## 1. 本仓现状（mcv_render，证据行号）

### 1.1 管线清单：11 条 `create_render_pipeline`，无缓存层

| 管线 | 定义处 | shader 入口 | 备注 |
|---|---|---|---|
| terrain / water / crack / outline | gpu.rs:2353/2381/2411/2440 | terrain.wgsl 一文件四个入口组 | 共用同一 bind group layout |
| sky | gpu.rs:2526 | sky.wgsl | 全屏三角 |
| hud | gpu.rs:2560 | hud.wgsl | 无深度 |
| player | gpu.rs:2636 | player.wgsl | mob 复用同管线（gpu.rs:3150 注释） |
| hand_icon | gpu.rs:2791 | hand.wgsl | 顶点布局复用 player |
| cloud | cloud.rs:217 | cloud.wgsl | |
| particle | particle_renderer.rs:172 | particle.wgsl | |
| item_drop | item_render.rs:123 | item_drop.wgsl | GUI 精灵表 + 方块图集双源采样 |

全部在 `Renderer::new`（gpu.rs:1559 起）一次性同步创建。**无 pipeline cache、无 specialization key、无 variant 治理**——变体仅靠"设备 max_texture_array_layers 决定图集数组拆分"（gpu.rs:1513 `atlas_arrays_wgsl` 字符串生成 + terrain.wgsl:130 `@@TERRAIN_ARRAYS@@` 占位行 replace）。

### 1.2 帧装配：draw_frame 手写 DAG

`Renderer::draw_frame`（gpu.rs:3214-3632）单 encoder 三 pass：

1. **world pass**（gpu.rs:3399-3573，LoadOp::Clear）：序 = sky(3424) → clouds(3433, 26.1 RenderPassOrder 注释) → opaque terrain 逐可见条目 draw(3445-3455) → player(3458) → mobs(3464) → item drops(3470) → crack/overlay(3486-3523) → particles(3527) → water **CPU 远→近排序**逐段 draw(3546-3572)。
2. **hand pass**（gpu.rs:3757，独立 pass 清深度，GameRenderer.java:724-729 对照）。
3. **HUD pass**（gpu.rs:3604，LoadOp::Load，无深度）。

- 依赖序的正确性靠**注释**维系（"sky first: writes no depth"(3423)、"26.1 translucent 序"(3485/3525)）——无机器校验的 DAG。
- CPU 侧每帧：视锥剔除（frustum.rs + AABB，gpu.rs:3371-3381）、water `sort_by` distance_squared（gpu.rs:3560）、HUD `flatten_hud`（gpu.rs:3872）、手持方块顶点 CPU 构建（gpu.rs:3671-3711）、各 pass uniform `write_buffer` 即时上传。
- draw 数观测：`FrameStats`（gpu.rs:1266，#80）。

### 1.3 材质与绑定：per-chunk dynamic offset，无材质抽象

- **每 chunk 一个 256B uniform 槽**（origins_buf，gpu.rs:3384-3391 连续打包 + 不变式注释 3368-3370）：`frame_bind` 按 slot 动态偏移，shader 侧 `@group(0) @binding(1) chunk.origin`（terrain.wgsl:123/155）。合批把多块重定基进一个 entry（MeshUploader 批次账本，gpu.rs:270-274），但**绑定粒度仍是"条目=一次 set_bind_group+draw"**。
- 纹理：单一 838 层 `texture_2d_array` 图集（827 贴图 + missing + 裂纹，lib.rs:1-3/47），按设备层数上限拆多数组 + `if` 链选层（terrain.wgsl:124-131 注释，GLES 256 保底）。
- 玩家/生物：`PlayerUniforms` 832B（gpu.rs:1384-1389），按 256B 槽位 dynamic offset 切分，**每实例一次 write_buffer + 一次 draw_indexed**（gpu.rs:3162-3163/3188-3208）——即 CPU 侧逐实例 uniform 轮询，无 instancing buffer、无 bindless。
- 水染色：shader 内常数 `WATER_TINT`（terrain.wgsl:113，群系级色变体登记债 :110-112）；群系染色走顶点色（网格器 CPU 预采样，#191）。

### 1.4 资源生命周期：手写治理面（三件套）

`MeshUploader`（gpu.rs:275-331）是**引擎把 wgpu 挡在游戏层外的唯一入口**（注释 gpu.rs:262-268）：

1. **上传**：staging 环 8×2MiB（gpu.rs:97-98）map_async 写入 + mailbox 回执分帧回收（gpu.rs:145-157/284-289）+ `PendingUpload` 世代号追踪（gpu.rs:159-170，世代替换/死亡摘除 purge_uploads gpu.rs:409-433）+ 背压线 64MiB 提前 flush（gpu.rs:100-106）。
2. **驻留**：CPU pool 96MiB 预算（gpu.rs:124）+ **保留环语义**（锚点+Chebyshev 半径，环内永不逐出——#174 隐形墙契约，gpu.rs:317-323；对照 ClientChunkCache.java:158-160）+ remesh 债账 `drain_evicted` 回灌（gpu.rs:325-329）。
3. **记账**：`BatchingStats` 14 字段（gpu.rs:178-227）：resident/cpu_pool/transient 峰值/live_vertex_peak 等字节账，#115 刀 5a「现算不入热路径」纪律（gpu.rs:173-176）。
4. **资产加载**：`std::fs::read`/`include_bytes` 直载（atlas.rs:596-734、cloud.rs:146、text.rs:21）——同步阻塞、无失败恢复协议之外的状态机、无热重载。素材红线：不产程序化假贴图（lib.rs:3）。

### 1.5 后处理：不存在

雾 = 每 pass 片元内联 `fog_factor`（terrain.wgsl:81-104，双层线性 max，色由 CPU 合成下发）；无 tonemap、无 bloom、无独立雾/天空 RT、无 post-process 链。`ViewTarget`-式"中间 RT + 最终 blit"结构不存在——三个 pass 全部直接写窗口 color/depth（offscreen.rs 仅 CI 抓屏用）。

### 1.6 多线程与观测：单线程装配，E0 九段计时

- mcv_render 内无渲染准备线程（仅 cloud.rs:138 `Mutex` 状态与 staging mailbox 两处锁）。`draw` 段（Scene 装配 + draw_frame）在主线程；`queue.submit` 单次提交（gpu.rs:3631）。
- **E0 已落地**（超出 roadmap 描述）：`FrameTiming` 九段 `fixed_step/ecs/stream(init/edge/io/remesh)/flush/draw`（game.rs:760-800，`SEGMENTS` 表为验收面）+ 1Hz mean/max 日志（app.rs:3241）。**缺口**：`draw` 段内部不细分（cull/sort/encode 混在一个数里）；无 GPU timestamp（world pass `timestamp_writes: None`，gpu.rs:3418）。

---

## 2. Bevy 对拍（0.19 / 0.20-dev）

### 2.1 渲染阶段总序（渲染世界 + 主世界分离）

帧序（[Tainted Coders 0.20-dev](https://taintedcoders.com/bevy/rendering)）：

```
ExtractSchedule（阻塞同步点：主世界 → 渲染世界拷贝）
Render schedule: ExtractCommands → PrepareAssets → PrepareMeshes → CreateViews
  → Specialize → PrepareViews → Queue(QueueMeshes/QueueSweep) → PhaseSort
  → Prepare(PrepareResources…→PrepareBindGroups 六子段) → Render → Cleanup → PostCleanup
Render 内: Begin → Render → Submit → Finish（camera_driver 逐活动相机置 CurrentView 跑 Core3d/Core2d）
Core3d/2d sets: Prepass → MainPass → EarlyPostProcess → PostProcess
```

与本仓逐点映射：Extract ≈ `Scene` 快照结构（gpu.rs:1201-1260，但 Scene 是值拷贝传参，not 双世界）；PhaseSort ≈ water CPU sort（gpu.rs:3560，但只半透明，且在 draw 里而非独立阶段）；camera_driver ≈ 无（单視口写死）。

### 2.2 Render Graph as Systems（0.19 #22144）

- 旧：`Node` trait + `RenderLabel` + 手接边；新：**pass = 普通系统**，`RenderContext` 系统参数 + `command_encoder()`，排序用 `.after(Core3dSystems::MainPass)`（[0.19 notes](https://bevy.org/news/bevy-0-19)）。
- 对本仓意义：本仓 1.2 的注释式 DAG 在 Bevy 里全部变成可并行校验的调度约束；新增 pass（如未来 SSR/SSAO）从"改 draw_frame 名手翻注释"降为"注册一个系统 + 声明两条边"。

### 2.3 材质与绑定

- `AsBindGroup` derive 声明绑定（uniform/texture/binding_array 槽位），`Material`/`Material2d` trait + `MaterialPlugin` 注册；扩展材质 `ExtendedMaterial` 在基材质上叠槽（扩展位建议 index 20 起防碰撞）。
- **0.19 partial bindless（Metal）**：按材质实际需要选 binding_array(仅纹理) 或 buffer 数组；修 sampler 上限口径（`max_binding_array_sampler_elements_per_shader_stage`）；避开未用槽保 31 argument buffer 限额。
- 体素级参照：0.19 "GPU-driven multi-draw-indirect"——mesh 实例分组进 **batch sets**、解包下沉 GPU；batched depth-only prepass 合并 draw；sparse mesh uniform uploads 只传变更段（many_cubes 49.47ms→18.77ms）。
- 差距量化（本仓 vs）：per-chunk 256B 槽 ≈ Bevy "mesh uniform buffer + sparse upload" 的手工雏形，缺的是①绑定声明式治理（换槽不重写 shader管线）②bindless 通道 ③GPU 侧 batch 展开（本仓批次在 CPU 建、rebuild_bytes 全 CPU 付——gpu.rs:186-188）。

### 2.4 Shader imports / pipeline 特化

- Bevy：`#import` 跨文件片段复用（naga_oil 处理器）、`ShaderRef`、shader_defs 编译期开关、`SpecializedRenderPipeline` + 缓存 key（管线按 key 惰性创建并缓存）。
- 本仓：`atlas_arrays_wgsl` 字符串 replace 是同一问题的手写解（gpu.rs:1513-1557），无 import、无 defs、无缓存 key——**加一个位移/水波变体 = 复制整条 create_render_pipeline**。

### 2.5 资产系统

- `AssetServer::load` → 立即 `Handle`（后台 TaskPool 加载）→ `get_load_state`（NotLoaded/Loading/Loaded/Failed）→ `AssetEvent::LoadedWithDependencies`；强引用计数卸载；`.meta` 处理管线 + file watcher 热重载（[Tainted Coders](https://taintedcoders.com/bevy/assets)）。
- 本仓：同步 fs 读（1.4.4）+ 手写"素材红线"失败面。**迁移时最先失守的资产**：838 层图集的数组拆分逻辑（设备上限适配，terrain.wgsl:124-131）需要重写成 Bevy 的 array texture 加载器/自定义 Loader。

### 2.6 后处理栈

- Bevy：`ViewTarget` 中间 RT 链 + `EarlyPostProcess`/`PostProcess` set 中串 `ViewNode/pass 系统`；内置 tonemapping/TAA/bloom；0.19 新增 `Vignette`、`LensDistortion` 全屏材质（`FullscreenMaterial`-系 API，0.18 引入 node_edges 声明）；Solari 实时路径追踪（离本仓远）。
- 本仓：零后处理（1.5）。雾要独立成层（如体积雾/水下后期）必须先引入中间 RT——这恰是 ViewTarget 解的问题。

### 2.7 可见性 / 排序 / 剔除

- Bevy：PhaseItem bin（opaque 按 pipeline/material key bin、transparent 按距离 sort，PhaseSort set）；0.19 skinned mesh culling 用逐帧关节位（`Mesh::generate_skinned_mesh_bounds`）；`NoCpuCulling` 逃生门；GPU-driven culling 面。
- 本仓：仅 CPU frustum-AABB（wall-fade a4cbf366 刚修过 AABB y 域世界带化缺陷）；水排序正确但逐帧 CPU 全量 sort；无遮挡剔除、无 LOD、无 DLOD。**生物/玩家为每实例一 draw**（1.3），与 0.19 batch sets 方向差距最大的就是这里（mob 上限 `MOB_MAX_INSTANCES` 撑不住大怪群）。

### 2.8 多线程渲染准备

- Bevy：渲染世界独立 ECS + 系统自动并行（Prepare/Queue 各系统按依赖并行）；Extract 是唯一阻塞同步点；命令编码分散在各 pass 系统（0.19 后甚至 pass 之间也可由调度器并行无冲突编码）。
- 本仓：全部单线程（1.6）。E0 的 `draw` 段已在 app.rs:3241 单列——**它是量化"装配占总预算多少"的现成探针**，迁不迁 Bevy 都该先读数。

---

## 3. 差距矩阵

| # | 维度 | 本仓现状（证据） | Bevy 0.19 能力 | 对 MC 意义 | 迁移成本 | 风险 |
|---|---|---|---|---|---|---|
| G1 | pass 装配/DAG | 注释维系的手写序（gpu.rs:3399-3628） | Core3d 调度 + `.after` 机器校验边（#22144） | 加 pass 从"改骨架"到"注册系统" | **M**：draw_frame 拆 11 系统，依赖照注释翻译 | 低；序已在注释里自证 |
| G2 | 材质/绑定 | per-chunk 256B offset + 单图集数组 + 手写 wgsl 生成（gpu.rs:1513） | AsBindGroup/Material + partial bindless + batch sets | draw 数与 uniform 带宽；mob 群 | **L**：材质抽象重写绑定面；图集假设渗透全 mesher | 高：像素级回归面巨大（CI 对拍在位） |
| G3 | pipeline 特化/缓存 | new() 一次性 11 条，无 key | SpecializedRenderPipeline + 缓存 | 加变体（水波/LOD/手电） | **S**（局部可先做裸 wgpu 版 key 表） | 低 |
| G4 | 资产生命周期 | fs 直载 + 手写池/世代/债账（gpu.rs:275-331） | AssetServer/Handle/事件/热重载 | 换资源包、开发生态 | **L**：MeshUploader 三件套是资产=劳动力的核心，Bevy 无对应体素驻留治理 | 高：#174 隐形墙契约需原样保留 |
| G5 | 后处理 | 无；雾内联片段（terrain.wgsl:81-104） | ViewTarget + Fullscreen 管线栈 + Vignette 等 | 水下后期/体积雾/tonemap | **M**：先引中间 RT，再谈链 | 中：移动端带宽 |
| G6 | 可见性/排序 | CPU frustum + 水全量 sort + 逐实例 draw | Phase bins + GPU batch/indirect + 关节位剔除 | 大怪群/大视距 | **M**（mob instancing 可独立先行） | 中 |
| G7 | 多线程准备 | 单线程装配；E0 探针在位（app.rs:3241） | 渲染世界 + 并行 Prepare/Queue | 30ms 移动帧窗 | **L**：横切整个渲染层 | 高：决定论纪律（E 波"零重型依赖"哲学冲突点） |
| G8 | shader 复用 | 字符串 replace 占位（gpu.rs:1513） | #import + shader_defs | 去重 8 文件 | **S**（Bevy 内置；不迁移也可引 naga_oil） | 低 |

成本口径：S=天、M=周、L=月级横切。避免虚构：未逐条核 Bevy 源码行号，全部注明二手出处。

## 4. 可复用面（迁移的"带得走"清单）

1. **E0 九段计时 + FrameTimingStats mean/max 汇总**（game.rs:860-900）——迁移验收的探针原样可移植。
2. **字节账口径**（transient/live_vertex peak，gpu.rs:213-227）——Bevy 下等价物是 `BatchingStats` 语义保持，作为 GPU 驻留回归锁。
3. **保留环契约 + remesh 债账**（#174）——无论谁渲染，"环内永不逐出 + drain 回灌"语义必须随迁移走。
4. **staging 环分帧上传思想**——Bevy 渲染世界有自家缓冲管理，但 8×2MiB 分帧回收 + 背压线的参数可直接复用。
5. **8 份 WGSL**——同语言（naga），移植主要是换绑定声明与 import；原版对齐注释（各文件 Java 行号）是合规资产。
6. **offscreen CI 像素对拍**（offscreen.rs）——Bevy 侧以 RenderTarget 等价接法保留，红绿判据不变。

## 5. 若迁 Bevy 的阶段路线（草案，未拍板）

- P0（不引 Bevy）：G3+G8——本仓内先立 pipeline key 表与 wgsl 模块化（naga_oil 单依赖），消除"G2 迁移前的最大静态债"。
- P1（G1）：以 Core3d 模型为蓝本**手工重排** draw_frame 为系统化 pass 结构（仍裸 wgpu），把注释边变数据边——这是 Bevy 迁移的预演，也独立受益。
- P2（G6）：mob 逐实例 draw → 实例 buffer/indirect（对照 0.19 batch sets），E0 读数立项。
- P3（G2/G4/G7）：材质抽象与资产治理上 Bevy——只在 P0-P2 数字支持（movs 数/uniform 带宽/装配毫秒）且移动端内存窗核过 **才立项**；否则 Bevy 仅作对标参数来源（其 0.19 优化数字是免费的对拍基线）。
- 明确不迁的正当性：E 波"零重型框架"哲学（engine-roadmap.md §3 E1 并发选型理由同构）+ 本仓渲染正确性资产（对拍注释、CI 抓屏）全部锚定在手写管线上。

## 6. 未决问题

1. `draw` 段毫秒分布未知（E0 段内不再细分）：G1/G7 收益先要这一格数——建议下一步加 cull/sort/encode 三子列（纯读改 app.rs 日志行，归 E0 延伸，不属本审计改动面）。
2. Bevy 0.20-dev 的 `RenderScheduleOrder` 命名尚在漂移；本文引 0.19 为冻结基线。
3. GLES 保底机（256 层图集拆分）在 Bevy binding_array 下的等价适配无实测——P1 若启动需先过这一关。

---

*来源：本仓代码行号见各节；Bevy 面：[Bevy 0.19 Release Notes](https://bevy.org/news/bevy-0-19)、[Bevy 0.18 Release Notes](https://bevy.org/news/bevy-0-18)、[Tainted Coders: Rendering](https://taintedcoders.com/bevy/rendering)、[Tainted Coders: Assets](https://taintedcoders.com/bevy/assets)、[bevy::render::renderer docs](https://docs.rs/bevy/latest/bevy/render/renderer/index.html)、[Bevy Engine 官网](https://bevy.org)。*
