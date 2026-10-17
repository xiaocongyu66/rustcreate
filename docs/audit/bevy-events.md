# 游戏事件处理 vs Bevy 0.20 差距审计

> 状态：证据落盘完成（矩阵/分级/衔接建议齐）。只读审计，未改任何生产代码。
> 对象：本仓事件面三件套——`engine/mcv_ecs` 事件总线（events.rs）+ `game/mcv_logic`
> 事件消费面（game.rs）+ 两条**非总线**异步流（worker 死账回执 #204、fluid
> scheduled tick 队列）；对拍 Bevy `bevy_ecs` 0.20.0（GitHub `bevyengine/bevy`
> v0.20.0 实物源 + docs.rs，本地佐证 `/root/engine-ref/bevy_ecs-0.20.0`）。
> 注意 Bevy 0.17+ 定名反转：**缓冲事件 = `Message`**（derive `Message`，
> MessageReader/Writer），**`Event` = 观察者触发**（immediate observer）。

## 0. 结论速览

三大差距：

1. **事件寿命节拍：本仓「阶段」vs Bevy「帧」**。本仓 `rotate_all` 挂在
   `run_stage` 末尾（schedule.rs:112），Fixed+Variable 每阶段翻一次——追帧
   （MAX_CATCHUP_STEPS=5）时一帧最多 6 次 rotate，事件寿命被抖动截短；Bevy
   的 `message_update_system` 固定在 `First` 每帧一次，两帧窗口恒定，且提供
   `signal_message_update_system` 显式对齐 fixed update 周期。→ **平迁**。
2. **观察者路由：本仓零、Bevy 强**。Bevy 0.20 `Event` + `Observer` +
   `EntityEvent`/`Trigger` 提供「事件→目标实体→立即执行」的委托模型；本仓是
   「发送进通道 → 阶段末主线程集中 `take()` 手写结算」，无实体级路由、无
   生命周期事件（Add/Remove patterns）。当前单玩家打怪场景下集中结算够用；
   实体互射/AOE 多目标时是重构点。→ **维持，条件触发重构**。
3. **多消费者契约：本仓 take 即清（单消费者），Bevy per-reader cursor**。
   本仓 `read()`（非消费快照）与 `take()`（整批取走消费）是两种手工协调的
   消费语义（拾取流 = read 快照过滤 → settle `take` 消费 → get_ref 复核三段
   式）；Bevy `MessageReader` 每读者独立 cursor，天然多消费者且两帧窗口内
   不丢。→ **小成本平迁（帧级 rotate 后风险自然消解）**。

分级一览：

| 差距面 | 分级 | 成本 | 见 § |
|---|---|---|---|
| rotate 节拍阶段→帧 | **平迁** | 低（~10 行 + 调用点收敛） | 4.1 |
| per-reader cursor / 多消费者 | 平迁(可选) | 低–中 | 4.2 |
| derive `Message`/`Event` 宏 | 平迁(可选) | 低（proc-macro 或手放 `#[derive]`-less） | 3.5 |
| 观察者/实体路由/生命周期事件 | **重构（暂不）** | 中–高 | 4.3 |
| SystemSet 依赖排序 | 维持（注册序即序，单线程可预测） | — | 3.4 |
| worker 死账回执（#204） | **维持（本仓强项，Bevy 无对应物）** | — | 5.1 |
| fluid scheduled tick 时间序队列 | **维持独立队列，不映射为消息流** | — | 5.2 |

## 1. 本仓事件面（代码证据）

### 1.1 mcv_ecs 事件总线（engine/mcv_ecs/src/events.rs，97 行）

- `Events<E>`：`reading`/`writing` 两个 `Vec<E>` 双缓冲（events.rs:12-15）。
  `send` 压 writing（:26-28）；`read` 返回**非消费**只读迭代（:31-33）；
  `take` 整批取走读侧——「消费即清，适合阶段结束后一次性结算」（:35-38）；
  `rotate` = swap + 清旧 writing（:48-51）。
- `EventBus`：`HashMap<TypeId, Box<dyn Channel>>` 类型擦除通道注册表，通道
  惰性创建（events.rs:69-89）；`rotate_all` 阶段边界统一翻转（:92-97）。
- 头注自述语义（events.rs:1-5）：**「同阶段发送的事件本阶段读不到，下一
  阶段可见，再下一阶段自动清空」**；双缓冲避免「迭代中发消息」改坏读侧。

### 1.2 调度契约（engine/mcv_ecs/src/schedule.rs）

- `Schedule::run_stage`：系统顺序执行 → `commands.apply` → **`rotate_all`**
  （schedule.rs:101-113）。事件窗口边界 = 阶段边界，阶段内 World/事件为
  稳定快照（:109-110 注）。
- `step`：固定步累加 ≤5 次追帧 + 一次 Variable（schedule.rs:117-132）。
  推论：**正常帧 2 次 rotate；卡顿帧最多 6 次**——send 于 Fixed 第 k 步的
  事件最迟在第 k+2 步或 Variable 段被清，事件寿命随帧时长抖动。
- 执行序 = 注册序（`add`，schedule.rs:81-90；Godot「注册在启动、运行期
  冻结」模型，lib.rs:10-12）——本仓无 SystemSet 式声明依赖。

### 1.3 事件类型清单与消费点（game/mcv_logic/src/game.rs + game/mcv_entity）

| 事件 | 生产侧 | 消费侧 | 消费契约 |
|---|---|---|---|
| `MobMeleeHit` | mobs AI（game.rs:7593） | game.rs:4322 `take()` | 消费即清 |
| `MobArrowHit` | 射手 AI（game.rs:7787） | game.rs:4337 `take()` | 消费即清 |
| `MobExplosionHit` | 爆炸（game.rs:7643） | game.rs:4342 `take()` | 消费即清 |
| `PlayerArrowHitMob` | 玩家箭（game.rs:7806） | game.rs:5070 `take()` | 消费即清 |
| `XpOrbPickup` | xp.rs:204 `send` | game.rs:5042 `take()` | 消费即清 |
| `PickupReq` | items.rs:240 `send` | game.rs:4362-4368 `read()` 快照 + items.rs:399 `settle_pickups` 内 `take()` | **read+take 三段式** |

- **拾取三段式**（game.rs:4362-4411）：① `read()` 非消费快照收候选（含句柄/
  位姿/没收前 count；注释明言「同 tick 已被合并/到期的实体在此处 get_ref 即
  None」）；② `settle_pickups`（items.rs:393-399）`take()` 真消费、入栏、
  满收 despawn；③ 主控对 ① 的快照做 `get_ref` 复核——「满收 despawn → None；
  部分入栏 → count 变小（对应原版部分拾取也发 take 包）」。这是**手写的
  peek/ack/verify 读者生命周期**，语义上等价于 Bevy per-reader cursor 加
  事后世界态校验的组合。
- **双消费者风险位**：`PickupReq` 通道同时有 `read()`（game.rs）与 `take()`
  （items.rs）两类读者；当前靠调用顺序（同一同步序列内先 read 后 take）保证
  正确——正是 1.2 无 cursor 面的结构性裸奔点：谁先 take 谁吞全批。

### 1.4 交互事件不走总线

玩家输入/交互（放置/挖掘/挥手）由平台层直调宿主方法（game.rs:5913
`pub fn interact(&mut self, place: bool)`；mcv_game/interact.rs 只供射线/
距离纯函数）。**交互没有任何事件化抽象**——无 Bevy `Trigger`/输入聚合层，
事件总线只承载「ECS 系统→宿主」的回传向（mob→player）。

### 1.5 双 App 双总线

`mobs_app: mcv_ecs::App`（game.rs:993）独立拥有 World/EventBus/Schedule；
宿主每帧拆字段自建 `SysCtx` 驱动 `run_stage(Fixed)`（game.rs:4313-4319），
随后立刻 `take()` 各通道（game.rs:4322-）——**发送（ECS 阶段内）与消费
（阶段结束 rotate 后）被编排为紧邻的两步**，事件实际不可丢；但这把「两帧
窗口」压缩成了「阶段末一次性 drain」的隐式约定，新通道接入者必须复刻该
顺序，无类型系统保障。

### 1.6 worker 死账回执（#204，docs/worker-accounting.md）——异步回执模型

- 模型：**记账推进必须与产物确认配对；失败路径必须有回执**（DC-3 范本）。
  mesh worker 路产物经 mpsc `MeshDone{pos, seq, opaque, ..}` 回传
  （mcv_mesher/src/sched.rs:55-62），主线程 `flush` 把产物转成
  `MeshFlushEvents{committed, lost}`（game.rs:334/351），看门狗
  `mesh_watchdog_step` 三路合流（game.rs:3874-3882）：提交确认销账 +
  丢失回执即时回滚 + K=10 帧超时兜底，连续 M=3 次丢失断路器降级同步路。
- 回执**不经 EventBus**：`seq` 配对 + 超时 + 断路器的「必达/终态」语义与
  双缓冲「两窗口后静默丢弃」互斥（game.rs:3866-3873 注）。
- 对照 Bevy：无内建回执概念；Bevy 生态把后台任务产物转 `Message` 或
  world.trigger 的做法都无超时/断路面，需自建。**此面是本仓相对 Bevy 的
  超集**，非差距。

### 1.7 fluid scheduled tick 队列（fluid-flow2 波，wt-fluid2/game/mcv_logic/src/fluid.rs）

- 决策核 + **时间序 tick 队列**：`BinaryHeap` + `Reverse` 按「最早到期优先」
  排布，`(格, 流体)` 单账去重（fluid.rs 头注①：原版 LevelTicks 允许同格
  同型多笔按时间归并；本仓单账、迟到重调度不重入，队列规模恒界）。
- 触发面：`onPlace`/`neighborChanged`/`updateShape` 三入口归
  `scheduleTick(pos, fluid, tickDelay)`（water 5 / lava 30 tick，
  fluid.rs 头注、tick_delay:111）。
- 主线（本仓工作分支）**尚未落地**：KNOWN-DIVERGENCE.md:297「引擎无流体
  模拟，全局面在 #148 流体波」。game.rs 目前 fluid 面只有位姿/高度图判读
  （fluid_top:499、eye_in_water:4747），无流动。
- 性质：这是**游戏节奏（游戏拍）事件队列**——按世界时间到期的延迟事件，
  同格去重、可重排，与非 FIFO 的优先序;与帧事件（一次性广播）正交。

### 1.8 固定步节奏事件（growth_tick）

`growth_tick`（game.rs:5146-）固定步驱动的生长 tick：ticking 域圈定
（玩家 ±2 chunk、`stage ≥ LightLocalReady` 门）→ 纯函数 `growth::scan_tick`
→ 结算写入。等价「按节奏到期的域事件」，同样是**宿主手写分频 + 直接结算**，
不经事件总线（同 1.4 的直调风格）。

## 2. Bevy 0.20 事件系统事实（原文引注）

### 2.1 双缓冲与两帧窗口（crates/bevy_ecs/src/message/messages.rs）

- `Messages<B>`：**「represents the messages that occurred within the last
  two `Messages::update` calls」**；「Each message can be consumed by
  multiple systems, in parallel, with consumption tracked by the
  [`MessageReader`] on a per-system basis」——per-reader cursor 多消费者。
- 内核：`update()` = `swap(messages_a, messages_b)` + 清最旧缓冲
  （messages.rs:193-201）；`update_drain()` 返回被清批的迭代器（:208-）
  ——与本仓 `take()` 同形。
- 读者纪律：**「`MessageReader`s that read at least once per update will
  never drop messages」「If messages are not handled by the end of the frame
  after they are updated, they will be dropped silently.」**

### 2.2 时序契约与系统集（message/update.rs、bevy_app/src/sub_app.rs）

- `message_update_system` 由 `SubApp::default` 装配：**schedule 在 `First`，
  系统集 `MessageUpdateSystems`，run_if `message_update_condition`**
  （sub_app.rs:91-100 注：「We expect all `SubApp` implementations to
  schedule `First` regularly」）。
- `signal_message_update_system`：**「Signals the `message_update_system` to
  run after `FixedUpdate` systems」**（update.rs:15-17）——固定步对齐翻转的
  官方开关（`ShouldUpdateMessages::{Always, Ready, Waiting}` 状态机）。
- 竞态警告（messages.rs 模块文档）：**「If no ordering is applied between
  writing and reading systems, there is a risk of a race condition… whether
  the messages arrive before or after the next `Messages::update` is
  unpredictable.」** ——Bevy 用 SystemSet 声明序缓解，但契约更弱（窗口内
  竞态合法），本仓注册序 + 阶段 rotate 反而更确定。

### 2.3 Event = 观察者触发（immediate，非缓冲）（crates/bevy_ecs/src/event/mod.rs）

- **「To make an `Event` "happen", you "trigger" it on a `World`… This causes
  any `Observer` watching for that `Event` to run _immediately_, as part of
  the `World::trigger` call.」** ——与缓冲 Message 完全正交的第三条事件流。
- `Trigger` 关联类型决定**「which observers will run, what data will be
  passed to them, and the order they will be run in」**（Event::Trigger）；
  derive 默认 `GlobalTrigger`（全局观察者全体运行）。
- `EntityEvent`：**「For events that "target" a specific [`Entity`]」**——
  实体级路由;生命周期 `EventPattern`s（`Add`/`Insert`/`Discard`/`Remove`/
  `Despawn`）声明式订阅组件增删毁。
- Observer 是**组件**（`world.add_observer` 即 spawn；observer/mod.rs:28-），
  可被实体级 `observe()` 绑定到目标实体，含运行条件（condition.rs）。

### 2.4 Derive 与体系划分

- 缓冲事件 `#[derive(Message)]` + `add_message` 自动装 `message_update_system`
  （messages.rs 模块注：**typically initialized automatically using
  `add_message`**）；观察者事件 `#[derive(Event)]` / `#[derive(EntityEvent)]`。
- 三条流分工：Message=帧窗口广播（拉），Event/Observer=即发即执（推），
  SystemSet=系统间声明排序。

## 3. 对拍矩阵

| # | 能力面 | 本仓现状 | Bevy 0.20 | 差距/风险 | 分级 |
|---|---|---|---|---|---|
| G1 | 缓冲双缓冲 | `Events<E>` 两 Vec，rotate 挂阶段边界（schedule.rs:112） | `Messages<B>` 两缓冲，update 挂 `First` 每帧 | 本仓事件寿命=1 阶段且随追帧抖动（卡顿帧最多 6 rotate/帧）；Bevy 恒两帧 | **平迁** |
| G2 | 多消费者 | read/take 二态，全通道共享一份读侧；PickupReq 双读者靠调用顺序 | per-reader cursor，读者互不干扰，两帧窗内不丢 | take 吞批风险（ whoever-takes-first）；无读者记账 | 平迁(可选) |
| G3 | 丢失语义 | rotate 即丢旧批（writing.clear），无静默丢警告 | 「dropped silently」有明文契约与读者纪律 | 本仓契约靠头注口口相传，新通道易写错节奏 | 文档化平迁 |
| G4 | 消费后对账 | 拾取三段式 read→take→get_ref 复核（手写 cursor+verify） | MessageId/cursor + 世界态，无内建 verify | 本仓靠代码纪律，可复用性差 | 维持（已有范本） |
| G5 | 观察者/路由 | 无。发送→阶段末宿主集中 take 结算 | trigger 立即执行、EntityEvent 实体路由、Trigger 定序 | 实体互射/AOE 多目标结算需手写过滤链（如今 PlayerArrowHitMob 已显形） | **重构（暂不）** |
| G6 | 生命周期事件 | despawn 各表级联清（lib.rs:271-287），无事件 | Add/Insert/Remove/Despawn patterns + observer | 掉落物/经验球到期销毁现为静默，无横切挂钩面 | 随 G5 |
| G7 | Derive/注册 | 手写 struct + TypeId 通道，新事件零样板但无编译期校验面 | derive 宏 + add_message 自动装配更新系统 | 糖差，非语义差 | 平迁(可选) |
| G8 | 执行序契约 | 注册序即执行序（启动期冻结） | SystemSet 声明依赖，缺序=合法竞态警告 | 本仓更强；Bevy 迁移会**丢确定性**（回执对账依赖序） | 维持 |
| G9 | 固定步对齐 | rotate 随每次 run_stage（追帧多次） | `signal_message_update_system` 显式对齐 FixedUpdate | 本仓无帧级统一翻转位 → G1 根因 | 平迁（同 G1） |
| G10 | 回执/超时/断路 | seq 配对 + K 帧看门狗 + M 次断路（独立于总线） | 无对应物 | **本仓超集**，Bevy 迁移反需自建 | 维持 |
| G11 | 时间序节奏事件 | fluid BinaryHeap（到期优先、同格单账）+ growth_tick 手写分频 | Timer/固定步系统（无同键去重的内建队列） | 原版 LevelTicks 多笔归并语义两侧都缺，本仓 fluid 队列更近 | 维持独立队列 |

## 4. 分级与迁移成本

### 4.1 平迁 A（推荐，成本≈10 行）：rotate 从阶段边界提到帧边界

- 做法：`Schedule::step` 在**帧末**（Variable 段后）追加一次 `events.rotate_all()`
  （或新增 `Stage` 尾钩子），`run_stage` 内的 rotate 移除或仅保留帧级调用；
  语义从「1 阶段窗口」改为「1 帧窗口」（卡顿追帧时保真为 Bevy 两帧窗的下界）。
- 依据：Bevy 0.20 `signal_message_update_system` 正是为「不要让翻转跟固定步
  数量抖动」而生（update.rs:15-24）——本仓追帧 MAX_CATCHUP_STEPS=5 是同一
  问题的现成触发器。
- 连带：`take()` 语义不变;`read()` 快照在帧内多阶段系统间从「下阶段不可见」
  变「同帧可见」，需核对 PickupReq 双读者顺序（1.3 风险位）——mesh 事件
  （MeshFlushEvents）不在 EventBus，不受影响。
- 风险：低。唯一行为变化是事件可见窗变宽（多消费者竞态面变宽），靠 §4.2
  收口。

### 4.2 平迁 B（可选，成本中）：per-reader cursor 最小面

- `Events<E>` 增加 `u64` 单调计数 + `Cursor { last_read: usize }`，`read_from`
  按 cursor 切片;`take` 保留为「ack 全部」快路径。读者（mob 伤害结算、拾取
  结算）各自持 cursor，天然解除 PickupReq 双读者的 take 吞批风险。
- 判据：只有当同一通道出现第 2 个稳定消费者（如回放/调试事件打印）时才值得；
  当前仅 PickupReq 一处，靠调用顺序注释钉住亦可。

### 4.3 重构（暂缓，条件触发）：观察者与实体路由

- 触发条件：引入**实体对实体**战斗/AOE（多目标命中路由）、掉落物需要
  「被拾起/被合并」的横切生命周期挂钩、或实体数从数百升到需要解耦结算
  链时。
- 本仓集中结算 vs Bevy 观察者的真实差：Bevy `EntityEvent` 把「谁被命中」的
  路由下沉到触发器，`Trigger` 控序;本仓在宿主手写 for + get_ref 过滤
  （game.rs:4322-4353）。单玩家场景成本差可忽略，实体互射后每加一类事件
  都要重复手写过滤链——届时按 `world.trigger(MobArrowHit { target })` +
  实体观察者形态重构（新增面，非改写面）。
- 明确不建议：为重构而把 `MeshFlushEvents`/fluid 队列迁到观察者（§5）。

### 4.4 维持项（不迁）

- **注册序调度**（G8）：单线程 + 启动期冻结使回执对账时序可证明;换 SystemSet
  反引入「缺序竞态」合法域。
- **worker 回执体系**（G10）：Bevy 无对应物，迁移即倒退。
- **fluid 时间序队列**（G11）：见 §5.2。
- derive 宏（G7）：事件类型个位数，宏糖无杠杆;登记即可。

## 5. 衔接建议（fluid tick 队列 × 死账回执 × 事件总线）

### 5.1 worker 回执与 Message 缓冲的接口纪律

- **禁止**：把 `MeshDone`/`LightDone`/`GenResult` 等需必达终态的回执放入
  `Events`/`Message` 双缓冲——两窗口静默丢弃 + rotate 换批与「每派发恰一
  笔回执」（DC-3）互斥;#204 三次真机事故的教训面即「回执通道不丢」。
- 若事件逻辑统一（如改用 Bevy `Message` 承载回执）：必须走 `update_drain`
  等价消费（本仓 `take()` 同形,Bevy 有现成 `Messages::update_drain`）且
  消费侧保留 seq 配对/超时/断路三件套——即**缓冲只当搬运工，对账不进缓冲**。

### 5.2 fluid scheduled tick 队列的挂点

- 保持队列**独立资源**（BinaryHeap + (格,流体) 单账），按固定步
  `drain 到期项 → mcv_logic::fluid::tick 纯函数决策 → 世界写` 的宿主直调形
  （比照 growth_tick:5146 先例）。不要映射为 Message 流：同格去重与迟到
  重排（fluid.rs 头注①「迟到的重调度不重入」）在 FIFO 双缓冲上无法表达。
- 事件总线只承载**表现向广播**（spread 时的粒子/音效、石化黑曜石的视效），
  与 1.3 现有 6 通道同构——新通道接入按 §4.1 的帧级 rotate 节奏走。
- lava 30 tick / water 5 tick 与 lava 上升延迟 ×4（spread_delay:393）是
  「游戏拍」常量,应 expressive 于驱动侧固定步（1/60 之外原版 20 TPS 的
  分频位）与本仓 `on_tick` 手工分频风格对齐（KNOWN-DIVERGENCE GROWTH-1 ④
  同款登记法）。

### 5.3 一句话原则（沿用 worker-accounting 措辞风格）

> **节奏事件走时间序队列、回执走配对通道、广播走帧窗口缓冲——三者不同
> 队，混则丢账。**

## 6. 证据文件清单

- 本仓：engine/mcv_ecs/src/{events,schedule,app,lib}.rs;game/mcv_logic/src/
  game.rs（993/1744/1803-1844/334-363/3866-3894/4313-4411/5042-5070/5146-/
  5913/7593-7643/7787-7807）;game/mcv_entity/src/{items,xp}.rs;
  game/mcv_mesher/src/sched.rs;docs/worker-accounting.md;
  docs/KNOWN-DIVERGENCE.md:297;wt-fluid2 (fluid-flow2) game/mcv_logic/src/fluid.rs。
- Bevy（v0.20.0）：crates/bevy_ecs/src/message/{messages,mod,update}.rs;
  crates/bevy_ecs/src/event/{mod,trigger}.rs;crates/bevy_ecs/src/observer/
  mod.rs;crates/bevy_app/src/sub_app.rs。
- 前波审计：docs/audit/bevy-ecs.md（ECS/调度骨架）、bevy-physics-misc.md、
  bevy-render.md——本文只覆盖事件/回执/节奏队列三面，与其不重叠。
