# Bevy 生态差距审计——物理 / 动画 / Picking / 诊断 / 网络（五域）

> 分支 `bevy-gap-work`（经 `bevy-audit-physics-misc` fast-forward推送）·审计性质
> （只读侦察，不改代码）。对拍对象：Bevy 0.20 生态（avian3d 物理、`bevy_animation`、
> `bevy_picking`、`bevy::diagnostic`、`bevy_replicon`）vs 本仓手写实现。
> 结论分级约定：**平迁**（直接搬语义，低风险）/ **重构**（需重新设计接线）/
> **远期**（当前不打算做、留记录）/ **反向**（Bevy 为通用游戏设计而 MC 还原不需要
> 的部分——引入反而是负资产，同样如实写清）。

## 0. 本仓五域现状（代码证据）

### 0.1 物理域

手写**运动学 CharacterBody**，无刚体/求解器：

- `engine/mcv_game/src/physics.rs`（802 行）：`Aabb` + 单轴扫掠碰撞
  （`move_box`/`move_axis`，子步 ≤0.5 m、SKIN=1e-4 间隙）；轴序对齐原版
  `Entity.collideWithShapes`（Y 恒最先，水平按 |vx|<|vz| 取序）；
  特性：潜行防跌落 `edge_back_off`（Player.java:880-933 对拍）、自动上台阶
  `try_step_up`（Entity.java:1080-1136 候选顶面+位移比较）、梯子攀附
  `on_climbable`、水中三模式（普通/冲刺游泳视线转向/出水 jumpOutOfFluid）、
  创造飞行、显式 Euler 积分。
- `engine/mcv_game/src/consts.rs`：MC 语义常数全表（g=32 m/s²、JUMP 8.4 m/s、
  WALK 4.317、STEP_HEIGHT 0.6、LADDER_* 等，均带 26.1 反编译行号出处）。
- `engine/mcv_game/src/blockshapes.rs`（441 行）：**碰撞盒 ≠ 拾取盒** 双形状表
  （半砖/楼梯/栅栏按状态盒，火把/花草无碰撞）。
- 消费面：玩家 `step` + 怪/箭/掉落物 `step_entity`（同一 AABB 通路，
  `gravity_scale` 区分）；`game/mcv_entity/src/components.rs::PhysBody`。
- 测试：`tests/physics*.rs` 5 文件（calib/ladder/movement/sweep_air），
  `physics_calib.rs::jump_onto_one_block` 峰高校准。
- 边界：**无实体间碰撞**（怪挤怪/entity push 未实现）；无旋转/扭矩/关节；
  箭矢用 0.2 子步防隧穿（components.rs MobArrow 注）。

### 0.2 动画域

手写**账本相位 + CPU 关节矩阵**：

- 账本：`game/mcv_entity/src/passive.rs::AnimState`（walk phase/amount、
  鸡扑翼、羊吃草 head_drop/head_eat_angle）+ `advance_walk_anim`（60 Hz 步、
  按 4.317 m/s 满速比值缩放，LivingEntity.walkAnimation 等价）。
- 装配：`game/mcv_logic/src/mob_render.rs::collect_mob_instances_in` ——
  渲染帧 `pos_old→pos × partialTick` 插值 + MobPose →
  `mob_mesh::mob_model_matrices`（per-part 4×4，MAX_MOB_PARTS 上限）
  打包 `MobInstance`（GPU instancing）。
- 模型：`engine/mcv_render/src/mob_mesh.rs`（1576 行，8 怪 hard-coded 关节）
  + `player_mesh.rs`（599 行同款摆动账本）。
- 无骨骼骨架资产、无 blend graph、无动画事件（表现帧结算走 20t tick 边界）。
- 已知限制：head_pitch 恒 0（mob_render.rs:92 装配处未接怪实况俯仰）。

### 0.3 Picking 域

- 方块：`engine/mcv_game/src/raycast.rs`（155 行）Amanatides & Woo DDA，
  命中判据 = `blockshapes::pick_boxes`（拾取/碰撞双形状解耦：火把可拾取、
  水/空气无盒），返回 (BlockPos, 入射法线)；REACH 4.5 / 创造 5.0。
  另有 `game.rs::block_hit_t`（攻击遮挡专用 DDA，判据=碰撞形状
  「水/花草不挡刀」——刻意与拾取判据不同源）。
- 实体：`game.rs::pick_attack_target`（:8554）ray-AABB（脚底中心+half_size），
  ≤ ENTITY_ATTACK_RANGE 且严格近于方块命中距离，取最近；命中位姿
  `attack_pick_pos`（:8544）= `lerp(pos_old,pos,partial)` **与 mob_render
  渲染读数逐字同源**（判定侧与渲染同一插值位——攻击拾取同源插值）。
- 触屏：`engine/mcv_platform/src/touch.rs`（Android 指针事件），
  无独立 Hover/Cursor 抽象、无 UI/world 双通道拾取。

### 0.4 诊断域

- `game/mcv_app/src/app.rs`（:3229-3280）**~1 Hz 一行结构化日志**：
  `render-stats: draws=… resident_kb=… pool_kb=… evicted=… dispatch_gap=…
  worker_lost=… | frame-timing[us] mean/max …`。
- 帧预算分段计时（E0，#155）：`game.rs::FrameTiming` 9 段
  （fixed_step/ecs/stream{init,edge,io,remesh}/flush/draw）+ 1Hz mean/max
  累计器（`FrameTimingStats::record/take`）；口径表 `docs/memory-budget.md`、
  `docs/worker-accounting.md`。
- 纯 log：无屏内指标面板/曲线图、无统计量注册表（注册/打点/消费三段解耦）、
  无 GPU 时间戳、无外部 profiler 帧标记。

### 0.5 网络域

**本仓无此面**（如实标注）：全仓无 protocol/packet/socket/replicate 实现；
单机 + Android 目标。`engine/mcv_sync` 名字易误导——实为音频线程无锁 SPSC
环（Lamport 队列），非网络。存档走 `engine/mcv_save`（region IO，本地盘）。
无客户端/服务器拆分：worldgen/实体/光照全部单进程（worldgen 种子确定性
已在——将来拆服务端有地基）。

## 1. Bevy 生态侧侦察（docs.rs / 官网，2026-10-17）

| 域 | crate/模块 | 版本 | 核心事实 |
|---|---|---|---|
| 物理 | avian3d（`avian` 名为占位保留 crate） | 0.7 | XPBD 求解（`xpbd_joints` feature + 自定义 XPBD 约束）；collider 后端 = Parry（`parry-f32/f64`）+ `ColliderConstructor(Hierarchy)` 从 mesh 生成；`RigidBody`（Dynamic/Kinematic/Static 三态）、`Friction/Restitution/CollisionLayers/Sensor/CollidingEntities`、`CollisionHooks`；`Gravity/GravityScale/LinearDamping/LockedAxes/Dominance`；speculative + swept CCD、`SubstepCount`、物理插值；joints：Fixed/Distance/Prismatic/Revolute/Spherical（**motor/articulation 尚无**）；空间查询 `SpatialQuery`/`RayCaster`/`ShapeCaster`/`SpatialQueryFilter`；角色控制器**不含**，给 `MoveAndSlide` 系统参数 + 第三方 `bevy_tnua`/`bevy_ahoy` |
| 动画 | bevy_animation | 0.20.0 | `AnimationClip/AnimationPlayer/AnimationTargetId`；`AnimationGraph`（clip 节点+blend 节点加权混合，RFC51 composition）+ `AnimationTransitions`；additive blending（#15631）；**masks**（#15013，上下半身分层互不干扰）；`AnimationEvent`（随动画时间轴触发事件）；`AnimatableProperty/AnimatableCurve/animated_field!`（可动画任意组件字段）；骨骼蒙皮 = bones-as-entities + SkinnedMesh（Gltf 导入通路同 API） |
| Picking | bevy_picking | 0.20.0 | 四段管线：`pointer`（发 `PointerInput`）→ `backend`（读 `PointerLocation` 产 `PointerHits`）→ `hover`（合并成 `HoverMap`，`Pickable` 组件定归属）→ `events`（`PointerOver/PointerClick/PointerDrag` 等）；**crate 自带零 backend**（「mix and match」），`bevy_mod_raycast` 是 mesh raycast 参考实现；多 backend 可并发混跑 |
| 诊断 | bevy::diagnostic | 0.20.0 | `Diagnostic/ DiagnosticMeasurement/`+注册路径、`LogDiagnosticsPlugin`（打日志）、FrameCount；内建 no 屏内面板/曲线——面板是第三方（bevy_dev_tools、bevy-inspector-egui）；Tracy 集成走 `bevy/trace_tracy` feature（Bevy 0.16 起 GPU 时间戳进 Tracy `RenderQueue` 行，#18490；`trace_tracy_memory` 加内存采样） |
| 网络 | bevy_replicon | （对 Bevy 0.20） | **server-authoritative 复制**：`ServerTick`（默认 64 Hz、挂 `FixedMain`）驱动 server→client 单向组件/实体复制（`Replicated`/`Remote`/`ServerEntityMap`/`AppRuleExt::replicate*`）；RPC 类 = client/server/shared messages（`FromClient/ToClients`）；`Channel` 定传递语义；**transport 可插拔**（例 `bevy_replicon_renet`）；插值/预测只给低层 API + `Signature` 预测生成，无内建 snapshot 插值 |

## 2. 五域对拍与分级

### 2.1 物理 vs avian3d ——【反向为主，一小重构点】

MC Java 26.1 的实体物理**不是刚体物理**：它是「格子碰撞上的确定性运动学」
（AABB vs 体素网格单轴扫掠 + 固定步积分），与 XPBD 刚体求解器的语义差：

| 维度 | 本仓（MC 语义） | avian3d 0.7 | 适配成本 |
|---|---|---|---|
| 碰撞体 | 体素网格逐轴扫掠（blockshapes 状态盒，内存零增长） | Parry 凸形状/trimesh；体素世界需 collider 层级，十万格级内存/构建成本爆炸 | 不可平迁 |
| 手感校准 | physics.rs 逐行对拍反编译（轴序/上台阶/潜行回缩/水/梯子），峰高差 <0.15 m 量级有测试锁 | 要重写全部为力/速度/约束表达，「走着上台阶」「贴梯子慢滑」这类阶梯语义会变成约束补丁 | 极高 |
| 确定性 | 固定步显式 Euler，行为可重放/可测 | XPBD 多子步迭代 + 睡眠/倍频，浮点迭代序平台敏感 | 语义冲突 |
| 实用面 | 单轴扫掠 + 子步防隧穿 | swept CCD、`MoveAndSlide`、`SpatialQuery` | 思想可借鉴 |

**结论（物理域）**
- 采用 avian 全套求解器：**反向**——MC 还原不需要摩擦/恢复/关节/CCD 刚体
  世界；引入即负资产（确定性丢失 + 手感重写）。
- 可平迁思想 1：`RayCaster/ShapeCaster`（把查询器挂成组件、系统统一跑）——
  本仓 raycast 是自由函数、每次现算；但 DDA 网格步进本身快于 Parry
  broadphase，**不必改**，登记不动。
- 可平迁思想 2（真正的差距点）：**实体间推挤（entity push）**——原版
  LivingEntity push/挤怪本仓未实现（0.1 节边界）；MC 的 push 是简单 AABB
  分离位移，不是刚体接触求解；补法 = 每 tick 对相邻实体对做 AABB overlap
  推开（O(n²) 或 grid 桶），量小（数百实体）——**平迁（小）/待排期**。
- swept CCD 思想：箭矢 0.2 子步已覆盖速域，不引。

### 2.2 动画 vs bevy_animation ——【反向为主，远期一处】

- vanilla 生物动画本体 = **程序化账本**（walkAnimation 相位/幅值 + 部件三角
  函数摆动），不是关键帧资产——本仓 AnimState/advance_walk_anim 是忠实直译。
  blend graph/masks/AnimationTransitions 解决的是「多关键帧 clip 混合」问题，
  vanilla 地面八怪不存在该问题 → **反向**。
- 骨骼蒙皮管线（bones-as-entities + SkinnedMesh）：本仓 per-part 4×4 关节矩阵
  instancing 与 vanilla「cuboid 部件平移旋转」完全同构，渲染面更省
  （无蒙皮顶点权重）；引骨骼 = 把硬编码模型改成 Gltf 资产 = **远期**
  （前提：模组生态/高模需求出现；届时 AnimationGraph 才有意义）。
- 可平迁思想 1：`AnimatableProperty/animated_field!`（把「哪个分量被动画」
  声明成曲线可寻址数据）。本仓现全部硬编码 `MobPose` 字段；若后续加
  creeper 膨胀 swell/slime 压扁/箭矢插抖等曲线动画，可按此思想把
  AnimState 扩成「命名曲线账本」——**平迁（小）**，触发条件 = 出现首个
  无法用单相位公式表达的动画。
- 可平迁思想 2：`AnimationEvent`（动画时间轴发事件）。本仓表现帧（如羊吃草
  音效、扑翼声）由 20t tick 边界结算，语义本就 tick 对齐——**不需要**，反向。
- 顺手差距登记：`head_pitch` 装配处恒 0（0.2 节），修它不需要 Bevy——
  归域内补课，不入对本仓外差距。

### 2.3 Picking vs bevy_picking ——【反向；装置抽象远期】

- `bevy_picking` 是多指针/多后端/ix 通用探测装置（HoverMap、Pickable、
  Over/Out/Click/Drag 事件、UI+world 混合 hover）。MC 的 pick 语义是
  **中心准星单射线 + 双判据形状**（outline 拾取盒 vs collision 判挡盒——
  本仓 blockshapes 已解耦两表，-DDA 按判据分别步进）。Bevy picking 没有这个
  「同射线两套形状判据」概念，套上后仍要自己在 backend 里写 DDA——装置
  层（多指针/拖拽/悬停事件）对 MC 反是负担 → **反向**。
- 本仓实缺的反而是 MC 语义面：无 hover 持续高亮帧同步的独立系统（现在的
  outline/select 走 game.rs 内聚）；实体拾取仅攻击路径）。——即
  「本仓缺 MC 的部分」很小；「Bevy 多出的部分」很大。
- 触屏 Android：若未来做「拖拽畅选/分屏双指针」，`bevy_picking` 的
  pointer 抽象是现成参考骨架——**远期**（登记指针事件抽象可复用其四段
  管线命名）。
- 独立对照：本仓 wgpu-vs-MC 补丁不需要 bevy_picking 的 mesh raycast——
  DDA O(穿越格数) 已最优。

### 2.4 诊断 vs bevy::diagnostic + Tracy ——【平迁一小；Tracy 重构（小）/可选】

- Bevy 方案分层：①计数器注册面（`Diagnostic` 按路径注册，系统打点、
  消费面独立）②`LogDiagnosticsPlugin`（打日志）③第三方面板
  （bevy_dev_tools/inspector egui，桌面 egui 途经）④Tracy（`trace_tracy`
  feature，系统级 span + GPU `RenderQueue` 时间戳 + `trace_tracy_memory`）。
- 本仓 E0 分段计时（9 段 + 1Hz mean/max + render-stats 单行带 worker 账本）
  **粒度不输** Bevy 内建（内建只有帧时长/实体数/cpu/mem），但形态面差距：
  - 无**注册/打点/消费解耦**：字段硬编码进 log 行（口径表靠 md 文档维护），
    加一段要改三处；按 `DiagnosticPath` 思想抽注册表 → **平迁（小）**
    （新观察项只需 `PATH.record(v)`，log/面板/未来导出共享一份）。
  - 无屏内面板：真机 Android 现场看不到曲线，要 adb logcat；mcv_ui 自绘
    HUD 数字行/迷你曲线（贴 render-stats 数据源）即可，Bevy 的 egui 面板
    桌面途经不适配本仓 GLES/自绘栈 → **平迁（中）**，随手可做。
  - Tracy：系统级 span + GPU 时间戳 + 内存采样的重型取证，对 #155 帧预算
    目标（分段微秒账）属过度配置，且 Android 真机 overhead 与 feature 门
    成本不小——**重构（小）/可选**：若把 E0 各段换成 tracing span 并加
    `trace_tracy` feature 门，桌面 CI 深度取证与真机 log 双轨并存；
    **当下不排**（0.4 节自制分段够用），登记触发条件 = E0 显示的段位在
    本仓内无法再归因（需要跨系统/线程级归因）时。
- 结论：诊断域本仓**无实质落后**；补「注册面 + 屏内面板」两平迁、
  Tracy 登记为可选。

### 2.5 网络 vs bevy_replicon ——【远期；本仓无此面】

- 如实标注：**本仓完全无网络层**（0.5 节），五域中唯一整体空白。
- bevy_replicon 形态（server-authoritative、`ServerTick` 挂 FixedMain、
  规则式组件复制、可插拔 transport、预测/插值低层 API）与 MC 单机还原
  的当前目标正交：
  - 本仓定位 = MC Java 单机 + Android；无服务器进程、无协议面；
  - 若未来做多机：①需先做 server/client 拆分（FCS/worldgen/实体逻辑
    已纯函数化，拆分面尚可）②replicon 只给 Bevy ECS 复制框架，本仓
    自研 ECS（mcv_ecs）+ 快照（MobInstance 等纯数据 POD）不兼容，
    移植成本高于用其思想自写（tick 标定 + 规则式 delta + ENet/ UDP
    transport）③MC-Java 协议不适用（这是 Rust 重写自定协议）。
- 结论：**远期**。登记两条排序约束：a) 先做进程拆分再谈 crate 选型；
  b) 届时评估表以 replicon 输送层（renet）+ 自研复制规则为对照项。
- 单机相关的一点**当下可复用**事实：固定步 60 Hz ≈ replicon 默认 64 Hz
  tick 本就同构，本仓帧分段（E0）对未来「服务器 tick 计时」形态可直接
  继承——零动作，仅记录。

## 3. 追加核对（协调者补点）

### 3.1 GPU 粒子（bevy_hanabi / Sprinkles）vs 本仓 CPU 粒子池 ——【反向；远期一对】

- 本仓：CPU 粒子池（`engine/mcv_render/src/particles.rs` 1557 行 +
  `particle_renderer.rs` 405 行），Scene.particles + `tick_frac` 帧间插值
  （app.rs:3210），量级按块内事件粒子（挖掘/水中/火焰）预算设计。
- bevy_hanabi 0.19：`EffectAsset`（序列化效果体）+ `ParticleEffect` 组件 +
  **compute shader 模拟**；SpawnerSettings/各类 Modifier（球面位置/速度/
  属性/加速度/生命周期着色）。关键限制（官方页）：**旧 WebGL2 渲染器无
  compute——不可用；WASM 需 WebGPU**。本仓 Android 真机 = GLES/Mali
  驱动（docs/memory-budget.md 真机预算语境）：compute 可用性依赖
  wgpu-on-Vulkan，驱动兼容性是大问号 → **GLES 兼容性结论：不可依赖**。
- Sprinkles：同为 GPU 方案（compute），限制同级，且生态位重叠，不另评。
- 结论：**反向**（MC vanilla 粒子量级小、CPU 池预算内且已带插值余量）；
  若未来涌现渲染几万粒子场景（tnt 连爆/彩虹羊群）再评 GPU 路，且必须带
  CPU fallback 双路——**远期**。

### 3.2 脚本接入（bevy_mod_scripting）vs Rust 数据表驱动 ——【远期】

- 本仓：Rust 数据表驱动（registry/recipes/drops 全编译期表），
  改配方/掉落 = 改代码 + 重编。
- bevy_mod_scripting 0.23（对 Bevy 0.20）：Lua（5.1-5.4/Luajit/Luau）+
  Rhai 双后端，`IntoScript/TypedThrough` 派生、`ScriptFunctionsPlugin`
  注册宿主函数、on_init/on_update 生命周期挂接；官方自述「WIP、尚未
  优化/完整」+ 持续基准。
- 增值空间仅在「用户侧数据包/行为自定义」（MC 数据包/Orbis 等价物）：
  配方数据化本仓已有 JSON 面（assets/minecraft/lang 及 registry 结构），
  行为脚本化则引入 VM 调度/沙箱/热重载三面成本，与「单机还原优先」
  不匹配 → **远期**（触发条件 = 出现用户内容生态诉求）。

### 3.3 行为树/HTN（bevy_behave、bevy_bae）vs A*+Brain 状态机 ——【反向；远期一处】

- 本仓（#38）：`mcv_entity::ai::Brain`（20 Hz 状态机）+ `pathfinding.rs`
  A*（`MobPath` 账本）+ 被动怪 RandomStroll/Panic 平行路线——对应
  vanilla Goal 系统的直译层。
- bevy_behave 0.5（对 Bevy 0.18）：`Tree<Behave>` + `behave!` 宏，
  Sequence/Fallback/Invert/Wait、动态 spawn 任务实体、`Behave::trigger(T)`
  观察者回调（ctx.success/failure）、超时、日志；**无 HTN**、无资产格式
  （asset 加载是 PRs welcome 占位）。bevy_bae 同为 BT 类，采用面更小，
  细节不另评。
- 判定：vanilla AI 本就「Goal 优先级状态机」，BT 是额外间接层，直译
  Brain 比套 BT 更近原版语义 → **反向**。唯一真差距点 = 本仓 Brain 与
  bevy_behave 均无的 **HTN/规划式 AI**——vanilla 也没有，两者打平；
  若未来 mob 目录扩到几十种目标组合行为，可再评（**远期**）。

### 3.4 Tracy profiler vs E0 自制分段计时 ——【重构（小）/可选，见 2.4】

要点已在 2.4：Bevy `trace_tracy`/`trace_tracy_memory` feature（系统 span +
GPU RenderQueue 行 + 内存采样，Bevy 0.16 起 GPU 时间戳入 Tracy；Rust 侧
另有 tracy_full 绑定 1.14.1）。对真机 Android（Mali + 自绘 wgpu 栈）
overhead 与集成成本不匹配 #155 的「帧预算分段观测」目标；登记为选择性
增强（桌面 CI 深度取证双轨），触发条件同 2.4。**当前不排**。

## 4. 五域分级汇总

| 域 | 结论 | 一句话 |
|---|---|---|
| 物理 | **反向**（引求解器不采用）；entity push = **平迁（小）待排期** | MC 是格子运动学不是刚体；avian 语义不匹配且破坏确定性/手感 |
| 动画 | **反向**（graph/骨骼不采用）；曲线账本思想 = **平迁（小）**；骨骼资产化 = **远期** | vanilla 本就是账本程序动画；本仓 per-part instancing 已同构 |
| Picking | **反向**（装置抽象不采用）；本仓 DDA+双形状判据更贴 MC；指针四段管线 = **远期参考** | MC 中心单射线语义，bevy_picking 是通用 hover/drag 装置 |
| 诊断 | 注册面 + 屏内面板 = **平迁**；Tracy = **重构（小）/可选不排** | Bevy 内建比 E0 更粗；差距在面板与注册解耦，不在框架 |
| 网络 | **远期**（本仓无此面，如实空白；先进程拆分再选型） | 单机定位；replicon 形态登记为未来对照项 |
| 粒子 | **反向/远期** | GLES 不可依赖 compute；CPU 池够 vanilla 量级 |
| 脚本 | **远期** | 数据包诉求出现再评；现 Rust 表驱动 |
| AI/BT/HTN | **反向；HTN 远期（vanilla 亦无）** | Brain 状态机即 vanilla Goal 直译 |
| Tracy | **重构（小）/可选不排** | E0 分段已满足；桌面双轨留触发条件 |

## 5. 参考来源

- https://docs.rs/avian3d/latest/avian3d/ （0.7，XPBD/Parry/SpatialQuery/MoveAndSlide）
- https://docs.rs/avian/latest/avian/ （保留占位 crate，实体引擎在 avian2d/3d）
- https://docs.rs/bevy_animation/latest/bevy_animation/ （0.20.0）
- https://bevy.org/news/bevy-0-15 （masks #15013、additive #15631、AnimationGraph 用法）
- https://thisweekinbevy.com/issue/2024-03-11-animation-graphs-cheatbooks-and-googly-eyes （AnimationGraph = RFC51 落地 #11989）
- https://bevy.org/examples/animation/animation-graph （clip/blend 节点 + 权重拖拽示例）
- https://docs.rs/bevy_picking/latest/bevy_picking/ （0.20.0，四段管线/零内建 backend）
- https://docs.rs/bevy/latest/bevy/diagnostic/index.html （Diagnostic/LogDiagnosticsPlugin，无内建面板）
- https://docs.rs/bevy_replicon/latest/bevy_replicon/ （server-authoritative/ServerTick 64Hz/Channel/ transport 可插）
- https://docs.rs/bevy_hanabi/latest/bevy_hanabi/ （0.19.0，compute shader/WebGL2 不可用）
- https://docs.rs/bevy_mod_scripting/latest/bevy_mod_scripting/ （0.23.0，Lua/Rhai，WIP 自述）
- https://docs.rs/bevy_behave/latest/bevy_behave/ （0.5.0，对 Bevy 0.18，无 HTN/无资产格式）
- https://github.com/bevyengine/bevy/blob/main/docs/profiling.md （trace_tracy 用法/Tracy 版本配对）
- https://bevy.org/news/bevy-0-16 （GPU 时间戳入 Tracy #18490）
