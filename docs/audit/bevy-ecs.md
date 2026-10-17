# ECS 与调度架构差距审计：mcv_ecs vs Bevy

> 日期：2026-10-17。只读审计（不改游戏代码），目标：逐项对拍 `engine/mcv_ecs`
> 与 `bevy_ecs` 的设计差异，给出「可平迁改据 / 纵深重构」两级结论。
>
> 证据基线：
> - 本仓：`engine/mcv_ecs/src/*.rs`（约 860 行 src + 2 个测试文件），
>   `game/mcv_logic/src/game.rs`（ECS 装配 :1744-1848、Fixed 驱动 :4277-4321、
>   事件结算 :4322-4390、fixed_step 双频 :4154-4278）。
> - Bevy 侧：**本仓不依赖 bevy**；对拍基线为开发机参考树
>   `/root/engine-ref/bevy_ecs-0.20.0`（bevy_ecs 0.20.0 官方源码，研究用
>   快照，不入库、不编译），行号以该树为准；bevy_app/bevy_time 的固定步
>   部分以 docs.rs 在线文档补充。
> - 判价原则：本仓实体数百量级、单进程桌面/移动端、60Hz 主循环。差距按
>   「现实影响 × 迁移成本」计价，不做「Bevy 有 = 我们该有」的等号。meta
>   层面：mcv_ecs 的 Godot 取舍是**声明过的有意取向**（lib.rs:6-28「与
>   Bevy 的取舍」），审计区分「缺能力」与「拒绝该能力」。

## 1. 本仓现状快照

`mcv_ecs` 能力清单（全部单线程、零 unsafe）：

| 模块 | 实现 | 关键锚点 |
|---|---|---|
| World | `Entity{idx,generation}` 句柄 + 每 TypeId 一张 `RefCell<Box<dyn Store>>` sparse-set；despawn 级联清表 | lib.rs:74-96, 222-232, 271-287 |
| 组件 | 任意 `'static` 类型即组件（`trait Component: 'static`），无 derive/无注册宏 | lib.rs:94-96 |
| 查询 | 无多组件 join：单表 `iter`/`for_each` 密集迭代 + 跨表按句柄 `get(e)`；整表 `read`/`write` 视图，RefCell 运行期互斥 | lib.rs:148-218, 338-376 |
| 调度 | `Stage::{Fixed,Variable}` 双阶段（Godot `_physics_process`/`_process` 对位）；固定步 1/60 累加器 + `MAX_CATCHUP_STEPS=5` 丢历史；**注册序=执行序**；阶段末统一 `commands.apply` → `events.rotate_all` | schedule.rs:19-21, 100-133 |
| 系统 | `FnMut(&mut SysCtx)`，SysCtx 含 world/resources/events/commands 四字段 disjoint 借用 | schedule.rs:32-55 |
| 资源 | `HashMap<TypeId, Box<dyn Any>>` 单例表；游戏侧用法是**每步插入所有权快照**（区块表 Arc + 玩家位姿） | resources.rs:7-53；game.rs:4293-4312 |
| 事件 | `Events<E>` 双缓冲，`EventBus` 按阶段边界全通道 rotate（写→读翻面，读侧一个阶段后作废） | events.rs:12-97 |
| 命令 | `CommandQueue` 帧末闭包队列：push/spawn_with/despawn/insert/remove，按提交序 apply | commands.rs:9-63 |
| 装配 | `App`（5 字段全 pub）+ `Plugin` trait + `plain_plugin` | app.rs:15-72, plugin.rs |

游戏侧使用面（决定差距的现实权重）：

- ECS **只包裹 mob/掉落物/经验球**三族实体（`mobs_app: mcv_ecs::App`，
  game.rs:1744）；方块/玩家/天气/熔炉/粒子等仍是 `GameRuntime` 裸字段。
- 7 个 Fixed 系统，注册序即执行序：mob_ai → mob_arrows → item_physics →
  item_merge → item_pickup → xp_orb_physics → xp_orb_pickup（game.rs:1813-1848）。
- 分区查表：MobKind/ItemDrop/XpOrb 作**分区标记**（同一 World 多张表互不
  触碰），没有 marker 查询语法——靠系统内 `MobKind::get(e).is_some()` 过滤。
- 资源注入 = 每步 `resources.insert` 两个快照结构（`MobServices`/`DropWorld`，
  E6 起区块表 Arc 计数递增，game.rs:4293-4312）。
- 事件结算外提到宿主：命中（MobMeleeHit/MobArrowHit/MobExplosionHit）与
  PickupReq 在 `run_stage` 旋转后 `channel().take()` 统一结算
  （game.rs:4322-4390）。
- 原版 20Hz tick 语义（天气/熔炉/粒子/自然刷怪等）**不经调度器**，直接在
  `fixed_step` 里 `on_tick` 手工分频（game.rs:4158-4276）。
- `Stage::Variable`、`Plugin`、`App::update` 游戏侧零使用（仅 mcv_ecs 自测）。

## 2. Bevy 0.20 对拍基线

| 机制 | Bevy 实现（源码锚点） | 一句话 |
|---|---|---|
| 存储 | archetype 主存 + per-component 可选 SparseSet：每个组件组合一个 archetype，表组件存储列式 Table，随机访问组件可标注存 SparseSet | archetype.rs:1-7; storage/mod.rs:6-11; README「Component Storage」 |
| 系统模型 | 普通函数 + `SystemParam` 派生（`Query`/`Res`/`ResMut`/`Commands`/`Local`/`ParamSet`/`Single`/`Populated`），参数类型即访问声明，编译期算出 access 集 | system/system_param.rs; README「Systems」 |
| 排序 | `ScheduleLabel`/`Schedules` 命名调度集合；`SystemSet` 分组 + before/after/chain/run_if 声明式约束 | schedule/schedule.rs:321-399 |
| 歧义检测 | 构建期把「访问冲突且无序」的系统对报 warning/error（默认 `Ignore`，层级冗余默认 `Warn`） | schedule/schedule.rs:1839-1912 |
| 同步点 | `ApplyDeferred` 系统： Ordered 依赖边上有 deferred 缓冲时自动插桩（默认开） | schedule/auto_insert_apply_deferred.rs:13-21 |
| 执行器 | 可插拔：默认 `MultiThreadedExecutor`（预计算冲突集+依赖计数+ready 位集，跨线程跑非冲突系统）；可换 `SingleThreadedExecutor` | schedule/executor/mod.rs:47-55; multi_threaded.rs:62-130 |
| 固定步 | bevy_ecs 不管；bevy_app 层 `Main → RunFixedMainLoop → FixedMain`（内含 `FixedUpdate`），bevy_time 层 `Time<Fixed>` overstep 累计 + 有效帧长钳制（`max_delta` 默认 250ms） | docs.rs bevy_app `RunFixedMainLoop` / bevy_time `Time` |
| 资源 | 0.20 起资源即组件：`trait Resource: Component {}`，藏在专用资源实体上（`IsResource` 标记 + `ResourceEntities` 缓存），`Res`/`ResMut` 是带变更追踪的借用 | resource.rs:18-124, 87 |
| 变更检测 | 每 component/resource 一对 tick（added/changed）；系统有 last_run/this_run 游标；`Added`/`Changed` 过滤 + `Ref`/`is_changed()`；`CheckChangeTicks` 周期扫描防 u32 回绕（`MAX_CHANGE_AGE` 钳制） | change_detection/tick.rs:23-80; README「Change Detection」 |
| 消息 | 原 buffered Events 更名 `Messages`：双缓冲 + `MessageReader` 游标（跨两个 update 周期可见）+ `MessageWriter`；`MessageRegistry` 统一 update | message/messages.rs:69-73; message/message_mutator.rs:45-49 |
| 观察者 | push 型即时响应：`Observer` 本身是组件，`world.trigger` 立即运行匹配系统；`EntityEvent` 带目标实体 + 自定义 `Traversal`（可沿关系冒泡）；生命周期事件 `Add`/`Insert`/`Remove`/`Despawn`/`Discard` 都走观察者 | event/mod.rs:89-120, 369-372; observer/mod.rs:1-3; lifecycle.rs |
| 关系 | `Relationship`/`RelationshipTarget` 双向组件（如 `ChildOf`+`Children`），`LINKED_SPAWN` 控制级联 despawn/克隆 | relationship/mod.rs:270- |
| 必需组件 | `#[require]` 声明依赖组件，插组件时自动补默认值（含传递闭包） | component/required.rs:120-134 |
| 实体 | `Entity` = index+generation repr(C,align(8)) 位打包（u64 等价），预留/映射/克隆工具；`Disabled` 类禁用组件让查询隐式跳过 | entity/mod.rs:421-432; entity_disabling.rs:1-6 |

调度哲学差异一句话：Bevy 把「系统跑在什么顺序、能不能并行」变成**构建期
图问题**（声明约束 → 编译 access 集 → 拓扑序 + 并行);mcv_ecs 把它变成
**注册期线性问题**（注册序即执行序，借用冲突靠每表 RefCell 运行时把关）。
小系统数下后者更直白且零构建开销；系统数上去后，前者防止的是「顺序靠
巧合正确」——Bevy 甚至提供 shuffle_seed 随机抽检约束完备性
（schedule/schedule.rs:1893-1906）。

## 3. 差距矩阵

判定：✔ 有 / ◐ 近似（语义等价但收窄或机制不同）/ ✘ 缺失。
成本：低=天级、可局部；中=周级、动面收紧；高=换骨架。
风险：语义回归面 + 对现有 7 系统/宿主结算路径的破坏半径。

| # | 能力 | 本仓 | Bevy 基线 | 状态 | 平迁成本 | 纵深成本 | 风险与影响 |
|---|---|---|---|---|---|---|---|
| 1 | archetype 列存主存 | ✘（全 sparse set，已声明取舍） | archetype+Table | ◐ | — | 高 | 现规模无感；实体进万级且组件组合动态变化才回本。**维持取舍** |
| 2 | 多组件 join 迭代 | ✘（单表迭代+按句柄 cross-get） | `Query<(&A, &mut B), With<C>>` | ✘ | — | 中 | 现实痛点第一名：每个实体每步 `get::<T>()` 都付 TypeId HashMap 查找 + downcast；7 系统普遍 2-3 表/实体 |
| 3 | 查询过滤语法 | ✘（分区标记手工 `is_some` 过滤） | `With/Without/Added/Changed/Has/Or` | ✘ | — | 中 | 与 #2 同bject；marker 语义已存在只是没有语法糖 |
| 4 | per-component 存储标注 | ✘ | `#[component(storage="SparseSet")]` | ✘ | — | 低 | 跟 #1 绑定，单独做无意义 |
| 5 | bundle/批量 spawn | ✘（spawn+逐 insert） | `world.spawn((A,B,C))`，BundleInfo 批量迁移 | ✘ | 低 | 低 | 仅人机工学 |
| 6 | 实体分配 O(1) | ✘（spawn 扫 `occupied` 线性找洞） | 空闲回收 + 原子预留 | ✘ | **低（高收益）** | — | 掉落物风暴时 spawn 是 O(活实体)；修成空闲栈即可 |
| 7 | despawn 定位 | ✘（每表 dense 线性 position，stores×dense 双线性） | 表中已含定位元数据 | ✘ | 中 | 低 | despawn 风暴（箭矢/掉落合并）双倍线性 |
| 8 | 命名多调度 | ◐（enum 双阶段固化） | `ScheduleLabel` 动态集合 | ◐ | 低 | 中 | 够用；若把 20Hz tick 块调度化需 Phase 扩展（见 4.1-P6） |
| 9 | 系统集/声明式排序 | ✘（注册序即执行序） | SystemSet + before/after/chain | ◐ | 低 | 低 | 7 系统规模注册序胜过图排序；声明式约束防止后续加系统时顺序靠巧合 |
| 10 | run_if/条件/States | ✘（`on_tick` 手工分频） | condition + `States` 状态机 | ✘ | 中 | 中 | 游戏已有 Loading/Playing 门——但走的是 if 分支不是调度条件 |
| 11 | 并行执行器 | ✘（单线程，RefCell 互斥） | MultiThreadedExecutor 冲突集 | ✘ | — | 高 | 收益受主循环其余段（stream/remesh/draw）串行压制；`!Sync` World 须换原子借用账本；「视图分区」注释（game.rs:1817-1819）已自证分区可行，届时可按系统簇并行 |
| 12 | 同步点插桩 | ◐（阶段末一次性 apply=唯一同步点） | Ordered 边上自动 `ApplyDeferred` | ◐ | — | — | 语义等价（命令始终阶段末生效）；粒度粗不至于错 |
| 13 | 借用型资源/常驻资源 | ◐（每步所有权快照 insert） | `Res`/`ResMut` 常驻 + 变更追踪 | ◐ | 低 | — | 快照是有意取向（借用进不了 TypeId 表，lib.rs:15-18）；E6 Arc 化已把成本压到计数递增；**不强补** |
| 14 | 变更检测（组件/资源 changed/added） | ✘（无 tick，一切系统每步全表全跑） | `Added/Changed` + tick 滚动扫描 | ✘ | 中 | 中 | 现实痛点第二名：item_merge/pickup 每 60Hz 对全掉落物全量重算；与 #3 联合实现 |
| 15 | 双缓冲消息 | ✔（`Events` 双缓冲，阶段边界 rotate） | `Messages` 双缓冲 per-update | ✔ | 低 | — | 等价；细微差：Bevy 是 per-update，本仓是 per-stage——`schedule.step` 双阶段一帧会转两次（见 4.1-P4 注） |
| 16 | per-reader 游标/多读者 | ◐（`take()` 整批接管=单读者约定） | `MessageReader` 各自游标 | ◐ | 低 | — | 现有结算路径全是单消费者；多读者出现前无感 |
| 17 | 观察者/立即触发/遍历 | ✘ | `Observer`+`On`+`EntityEvent`+`Traversal` | ✘ | — | 中 | 会引入同步重入语义，与「帧批次信号」取向冲突；按需再议 |
| 18 | 生命周期事件/钩子 | ✘（级联清理 despawn 内联） | `Add/Insert/Remove/Despawn/RemovedComponents` 过程 | ✘ | — | 中 | 现无需求（组件生命周期全部显式） |
| 19 | 关系/层级 | ✘ | `Relationship`/`ChildOf` | ✘ | — | 中 | mob 无层级需求；UI 树不在 ECS 管 |
| 20 | 必需组件 | ✘ | `#[require]` | ✘ | — | 中 | spawn_with 闭包装配已覆盖 |
| 21 | 禁用组件 | ✘（手工过滤） | `Disabled` 查询隐式过滤 | ✘ | — | 低 | |
| 22 | 悬挂句柄安全/代校验 | ✔ | ✔（同款 generation） | ✔ | — | — | 对面打平 |
| 23 | 零 unsafe | ✔ | ✘（unsafe_code 有意用于性能） | ✔ | — | — | 本仓反向优势：RefCell 互斥语义=Bevy 冲突检查运行时版（lib.rs:19-21） |
| 24 | 固定步追帧 | ◐（累计器+步数硬上限 5 丢历史） | `Time<Fixed>` overstep+输入钳制（max_delta 250ms 无步数上限） | ◐ | — | — | 语义对位，本仓上限策略更保守；两法结合更稳（见 4.1-P7） |

## 4. 分级建议

### 4.1 可平迁改据（不换 Godot 骨架，吸收 Bevy 语义糖/工程经验）

- **P1 实体空闲分配 O(1)**：`occupied: Vec<bool>` 扫描找洞 → 维护空闲栈。
  掉落物高频生灭场景的先行安全垫；纯局部改动（lib.rs:251-267）。
- **P2 despawn 直寻**：给 `SparseSet` 增加 idx→dense 位置映射（或接受
  swap_remove 后在稀疏侧存 dense 位），消掉每表 `position()` 线性扫描
  （lib.rs:128-134）。与 P1 同属「实体风雨面的力学修补」。
- **P3 register 自动化**：`read`/`write` 遇未注册类型改为惰性建表（对标
  Bevy 组件访问即注册）。拆除「防止零怪时 read panic」的仪式性注册
  （game.rs:1803-1811 的注释可见该坑已实际咬人）；`register::<T>` 保留
  为显式别名，单测断言面不破。
- **P4 事件 reader 记号化**：`EventBus::channel::<E>` 保持，另给 `reader::<E>`
  返回带游标的只读视图（阶段内已读不重读），`take()` 降为「整批消费」糖。
  注：若未来启用 Variable 阶段，须先裁决「本仓 rotate 是 per-stage 而
  Bevy 是 per-update」——`schedule.step` 下事件在 Fixed 发、Variable 读、
  下帧 Fixed 前作废，与 Bevy 双 update 周期可见对齐，但文档必须写死。
- **P5 `on_tick` 分频声明化（调度集外挂）**：给 `Schedule::add` 增加
  `every_n_steps(20)` 类元数据，把 fixed_step 里四处手工 `if self.on_tick`
  的 20Hz 块（天气/熔炉/粒子/刷怪，game.rs:4158-4276）声明为带频律的系统。
  这是 Bevy `RunFixedMainLoopSystems` 意图的平迁版：不动 SysCtx、不引
  多调度框架，先让「原版 tick 语义」从散落 if 变成注册面可见的一等条目。
- **P6 排序防巧合**：调度器加一个 debug 开关——注册序顺序执行前打乱
  Fixed 系统次序再跑若干次断言（Bevy shuffle_seed 的单线程平价版）。
  7 系统规模下面检查的是「mob_ai 先于 item_physics 是否真有数据依赖」，
  防后续加系统的顺序回归。
- **P7 追帧双保险**：保留 `MAX_CATCHUP_STEPS=5` 步数上限，同时把
  accumulate_ticks 的输入 dt 钳制到 ~250ms（对标 `max_delta` 默认），
  防一次超大 delta 单独触发丢历史路径。

### 4.2 纵深重构（换/扩骨架，按需立项、单独测算收益）

- **D1 多组件 join 查询（建议首个立项）**：本仓化最小版——不抄
  `#[derive(SystemParam)]`，在 `World` 层增加两表/三表 join 迭代器
  （`for_each2::<A,B>` / `for_each3`，密集表逐主从、从表按句柄取），系统
  层面保持 `SysCtx` 收口。收益测算：7 系统平均每实体省 2 次
  TypeId+downcast+RefCell borrow；pos+body+brain 类热循环（mob_ai）,
  掉落物全量循环（item_merge）改善最直接。风险点：可变双表 join 的
  RefCell 互斥天然禁止（`write::<A>() && write::<B>()` 可行但 join 内
  `get_mut` 自表冲突）——需要 `ParamSet` 式单表可变出口或「主表可变+
  从表只读」约束，这正是 Bevy 图检查解决的那类问题，先做静态断言。
- **D2 变更检测**：与 D1 联合——表级 u32 change tick + 系统级 last_run
  游标，`for_each_changed` 只跑脏行。触发条件立项：实体规模进万级
  （掉落物风暴）或出现「想跳过未变重算」的实际 profile 证据。
- **D3 簇级并行**：不做全 Bevy 执行器；按已声明分区（mob 簇 / item 簇 /
  xp 簇天然 disjoint，game.rs:1817-1819 注释为证）把 Fixed 系统分组,
  组间跨任务池并行、组内保序。前置依赖：D1 + World 可并发表读取
  （`read` 是 `&self` 本可 `Send`，卡点是 RefCell borrow 需换原子计数
  或分区借用账本）。收益上限仍受 stream/draw 串行段压制，先测 ECS 子段
  在 FrameTiming 的 `ecs_us` 占比（E0 计时已具备）再立项。
- **D4 archetype 存储：维持不做**。实体组合三类骨架且年内不会动态化，
  lib.rs 头注的论证（数百实体下迭代顺序优势不可感知）在 0.20 对拍下
  依然成立。本行入档，条件触发再翻案：实体万级 + 组件组合高频漂移。
- **D5 观察者/关系：维持不做**。与「帧批次信号」取向冲突（观察者=同步
  重入，事件批=阶段内积攒），且生命周期全显式、无层级需求。

## 5. 结论

三大结构性差距（现实痛感排序）：**①多组件 join 查询缺失**（每实体每表
TypeId 查找税 + 手工分区过滤）、**②变更检测缺失**（60Hz 无差别全量重跑）、
**③并行执行器缺失**（单线程收口，簇级并行可行但被 stream/draw 串行段
稀释）。其余差距集中在工程糖（bundle/required/关系/观察者）与已声明的
有意取舍（archetype/借用资源）。

两级建议摘要：
- **平迁（4.1）**：七项全是局部改动——实体分配/despawn 力学修补 ×2、
  register 消坑、事件 reader 游标、20Hz 分频声明化、排序防巧合、追帧
  双保险；不破 SysCtx 契约，估计净语义零回归。
- **纵深（4.2）**：D1 两/三表 join 为唯一推荐立项，D2/D3 挂条件触发，
  D4/D5（archetype、观察者/关系）显式归档为「有意取舍」，给后者写了
  翻案条件。
