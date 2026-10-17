# 引擎基础框架路线图 —— 多核多线程架构蓝图（#154）

状态：勘察完成（2026-10-12，基座 main=b5369449）。本文是后续引擎增强波的**施工总图**：
每个波独立可验收、可回滚，波与波之间按依赖排序。所有结论给 file:line；性能数字只引
既有预算门与 CI 已有实测，本波零基准运行。

参照源：`/root/mc-ref/src-26.1`（仅机制/常数引注，零 Mojang 内容入仓，见
porting-conventions.md 硬约束）。

---

## 1. 当前线程/并发模型盘点（勘察结论）

全仓 `thread::spawn` 只有 4 处，其中生产代码仅 1 处。并发原语全部为 std
（Mutex/Condvar/Atomic/mpsc/自研 SPSC），**零第三方并发框架**（rayon/tokio/
crossbeam 全仓 0 引用，Cargo.toml 工作区依赖仅 wgpu/winit/pollster/bytemuck/
glam/log/serde_json/image）。

### 1.1 已多线程的部分（别重复造轮子）

| 域 | 机制 | 锚点 |
|---|---|---|
| **世界生成** | `TerrainScheduler`：构造期建 [`TaskPool`](../game/mcv_core/src/pool.rs)（Mutex+Condvar 双级优先队列，hi/lo），每块一个 `spawn_lo` 任务，结果走 `std::sync::mpsc`，主线程 `stream()` 内 `try_recv` 排空并 commit | mcv_worldgen/src/lib.rs:194-230；线程创建 pool.rs:65；主线程提交点 game.rs:2081-2094 |
| | 生成是**逐块纯函数** `generate_terrain_with(backend, seed, pos)`，后端在调度器构造期固定（后台线程不读环境变量） | lib.rs:73-105, 194-213 |
| | 在途节流：`MAX_CHUNKS_IN_EXECUTION = 4`，本帧先收结果释放名额再发新任务（原版 ThrottlingChunkTaskDispatcher 语义） | ticket.rs:65；game.rs:2116-2158；原版锚 DistanceManager.java:55 |
| | 池大小 `world_worker_count()` = `available_parallelism − 2`，clamp(2,6)（给主/渲染留 2 核） | pool.rs:111-115；原版对照 Util.java:187-189（`clamp(cpu−1, 1, max.bg.threads)`） |
| **GPU 上传** | #80 第二刀已达成：staging 环（8 槽 × 2 MiB）map_async + 完成回调 mailbox，flush 在 `stream()` 末尾、同帧 draw 之前 submit；单笔超槽/环耗尽退一次性兜底缓冲 | gpu.rs:91-97, 182-237, 284-425；flush 接线 game.rs:2310-2313 |
| **音频** | RT 回调线程独占 Mixer，命令经自研无锁 SPSC 环（Lamport 队列，acquire/release 配对，句柄 `!Sync` 结构性强制单生产者/消费者） | mcv_audio/src/lib.rs:17, 219-250；mcv_sync/src/lib.rs:1-11, 93-139 |

**结论：世界生成已多线程，GPU 上传已异步化，音频已实时线程化。这三块的任务不是
"并行化"，是"上确定性锁 + 补短板"（见 §3 波 E1/E2/E4）。**

### 1.2 主线程串行的部分（真串行墙，按单帧成本排序）

主循环 = winit 单线程事件循环（app.rs:2671 `redraw()`）：`fixed_step` ×n →
`stream()` → 30s 存盘 → 取帧 → build_hud/Scene → draw。以下按"视距 16、
实体数百"口径的单帧成本排序：

| # | 热点 | 现状 | 单帧成本量级 | 锚点 |
|---|---|---|---|---|
| 1 | **网格化** | 主线程串行，进世界后 2 块/帧、装载期 8 块/帧；每块先拷 3×3 邻域快照（9×98304×2B 体素 + 9×98304B 光 ≈ **2.53 MiB/块**）再跑 greedy mesh（opaque+water） | golden.rs 预算门 = release 10ms/**块** → 满预算时 2 块 = 20ms/帧，单帧即超 60fps；装载期 8 块是最尖峰 | 预算 game.rs:2288-2292；拷贝 game.rs:126-141；哨兵 mcv_mesher/tests/golden.rs:312-340（**只有预算门 10ms/200ms 可引，CI 捕获 stdout 无既有实测值**） |
| 2 | **光照 init + 跨块边同步** | init 主线程串行 2 块/帧（全块双通道 BFS，98304 格×2 通道）；边同步每帧两次消化段（stream 头 + 成对标记后），LIFO push→pull 序、REMOVE-before-ADD 两相、in-flight 去重、**512 步/帧预算耗尽不丢** | init 每块 ~O(CHUNK_VOL) 次队列操作；级联收敛期 512 步常打满 | init game.rs:2195-2220；边同步 game.rs:400-499（预算 :405，两端就绪门 :432-439）；成对标记 :2237-2280 |
| 3 | **存/读盘 IO（同步在主线程）** | 读盘 ≤4/帧、卸载写盘常态 ≤2/帧（积压门 48 追加）、脏块写回 ≤2/帧、30s 全量兜底扫描——每次 open+read/write 一个 region 文件，同步阻塞 | 一次 region IO = ms 级尖刺；30s 全量存盘是登记过的周期掉帧源（game.rs:2053 注释） | 读 :2406-2431；卸载落盘 :2010-2051；脏写回 :2061-2076；30s 全量 app.rs:2815-2819 → game.rs:1791-1818 |
| 4 | **ECS 资源快照克隆** | 每个固定步 `MobServices{chunks.clone()}` + `DropWorld{chunks.clone()}` 两次**整表 HashMap 克隆**（N=活动块数，Arc 计数级但表节点全重分配）；rd8=361 键×120 步/s，rd16=1225 键×120 步/s | rd16 时 ≈30 万表节点分配/s——实体系统尚未成为墙，这个克隆先成为 | game.rs:2670-2699（注释自称"Arc 计数级克隆"——表本身仍是深克隆，登记为认知修正） |
| 5 | **streaming O(N) 每帧重建** | `refresh_player_tickets` 全量重建票表；`level_map` 每帧算 2 次（卸载候选 + status 推进）；`sorted_keys` 全表克隆+排序每帧 2 次（布光/建网格流共用）；请求环每帧全环扫描 | rd16 N=1225、rd32 N=4489——每帧 O(N log N)×2 + O(N)×3；绝对值 ~百 µs 级，是 rd32 的最后一道墙而非第一道 | 票表 :1285, 1938；level_map :1957, 2231（实现 ticket.rs:166-194）；排序 :2386-2394；环扫描 :2118-2158 |
| 6 | ECS 系统 | 单线程顺序执行（FnMut(&mut SysCtx)，无冲突声明），mob_ai/arrow/item×3 五系统 | 数百实体下未成为墙（无计时数据支撑立项） | schedule.rs:101-113；注册 game.rs:1019-1040 |

### 1.3 锁与共享状态盘点

- `ChunkHandle`：`voxels`/`heightmap`/`light` 各一把 `RwLock`，`stage`/`status`/`dirty`
  原子（CAS 单调只进不退）。**锁粒度 = 单数组整块**，读端一次一锁、不持跨锁
  （game.rs:127-135 逐块 read；commit 一次写 game.rs:108-119；worker 只持
  `Arc<ChunkHandle>`，从不触 `GameRuntime`）。mcv_core/src/chunk.rs:134-159。
- `GameRuntime.chunks: HashMap<…, Arc<ChunkHandle>>`：**仅主线程可变**——这是当前
  一切并发的安全基石，也是 E3/E4/E5 快照纪律必须绕开的对象（worker 不许持有
  `&GameRuntime`）。
- `TerrainScheduler`：`Arc<Shared>` 池 + mpsc 双端；`request()` 克隆 tx，无共享可变。
- 音频：`SoundLoader` 在 Mutex 后（加载慢路径），RT 路径零锁（SPSC）。
- stage 流水线单向：`Empty → TerrainReady → LightLocalReady → Uploaded`
  （Lit/MeshReady 为预留死态，比较请用 `>= LightLocalReady` / `== Uploaded`）
  chunk.rs:1-9, 20-31。

---

## 2. 主线程帧预算结构

`redraw()`（app.rs:2671）一个渲染帧依次跑：

```
[暂停门] → fixed_step(1/60) ×n(追帧≤5)
    ├─ 20Hz tick 域：weather.tick + particles.tick + 雨粒子 + bob
    ├─ 输入应用 / 挖掘步进 / 自然生成（20 tick 一轮）
    ├─ ECS Fixed 阶段（mob_ai → mob_arrows → item_physics → item_merge →
    │   item_pickup）——每步先 insert 两次 chunks 全表克隆（§1.2-4）
    ├─ 事件结算（近战/箭/爆炸/拾取）
    └─ 玩家物理（sim_safe 半径门 + 钳制）
→ stream()（帧驱动，非 tick）
    ├─ 边同步消化段（512 步预算）→ 卸载 ≤4 → 卸载落盘 ≤2+积压
    ├─ 脏块写回 ≤2 → drain 生成结果 → 请求环（读盘 ≤4 / 生成在途 ≤4）
    ├─ 出生投放 → 光照 init ≤2 → status 推进 → 成对边标记 + 同步
    └─ remesh ≤2（装载期 8）→ mesher.flush()（staging submit）
→ 30s 存盘兜底 → get_current_texture → HUD/Scene → draw
```

**"渲染距离拉满 32、实体数百时主线程在哪爆"的定性排序：**

1. **内存墙（最先爆，且不是线程问题）**：请求环 (2×33+1)² = 4489 块 ×
   295,424 B/块 ≈ **1.26 GiB** 常驻（算式与 vd8=101.7 MiB 实测口径见
   memory-budget.md §1；palette 压缩已在 §1 登记"待做"）。当前 `DIST_MAX=16`
   （options.rs:9 设置 UI 步进域裁剪）——rd32 前必须先过 palette + 预算门重算。
2. **网格化**：2 块/帧 × 10ms 预算上限 = 20ms，高吞吐期（移动/编辑级联）稳定吃满；
   且每块 2.53 MiB 快照拷贝是纯带宽税。**这是并行化收益最大的一块**。
3. **光照级联**：边同步 512 步/帧预算在加载洪峰期常打满（每次 init 席位 + 成对
   标记都产生新边任务）；init 2 块/帧串行 BFS。
4. **IO 尖刺**：读盘 4 + 写盘 2-4 次同步 region IO；30s 全量扫描叠加。
5. **ECS 资源克隆**（§1.2-4）与 streaming O(N) 重建（§1.2-5）：rd32 规模下从
   微成本升级为可测成本。
6. ECS 系统本身与玩家物理：数百实体量级仍安全（无数据支撑立项，见 §3 E6 判断）。

---

## 3. 波切分（风险低→高，依赖前→后；每波独立可验收）

依赖图：E1 →（E2、E3、E4、E5 可并行推进）；E7 依赖 E4 + E6；E6 依赖 E0 计时。

### E0 —— 帧预算分段计时（观测先行，半天量级）

- **目标**：把 §2 的预算模型变成实测。`render-stats` 1Hz 日志（app.rs 已有，
  memory-budget.md §2）扩展为分段耗时：`fixed_step`（ECS 阶段单列）、`stream`
  （init/edge/remesh/io 各列）、`flush`、`draw`。零行为变化。
- **文件面**：game/mcv_logic/src/game.rs（计时字段）、game/mcv_app/src/app.rs（日志行）。
- **验收**：log 字段出现且 headless 流式回归不变红；无 golden 影响。
- **为什么是波**：E3-E6 的立项/验收标准（"mob_ai 是否 >2ms""remesh 实测多少"）
  全部靠它，#115 的 K5-MEAS 先例同构。

### E1 —— TaskPool v2：panic 隔离 + 池策略对齐（低风险）

- **目标**：① 任务 panic 隔离——现状 `job()` 裸调用（pool.rs:79），panic 会炸死
  worker 线程且无任何告警，池**永久缩编**（Drop join 返回 Err 被忽略 :105）；
  包 `catch_unwind` 记 log，worker 存活。② 池大小策略对齐原版：
  `clamp(cpu−1, 1, max.bg.threads)`（Util.java:187-189）替代现有
  `cpu−2, clamp(2,6)`（pool.rs:111-115）——移动端 6 核上限在大核机上过保守；
  保留 env 覆盖位。③ 在途/排队/worker 占用计数（E0 的任务侧数据源）。
- **并发原语选型（本文总决策，写在最前）**：**不引入 rayon/tokio-util**。
  理由：本仓并发单位是"一个区块一个任务"（~10ms 粒度），不是 µs 级细粒度——
  work-stealing 的优势场景不成立；手写 Mutex+Condvar 池 + mpsc + 自研 SPSC
  已覆盖三种已并行域，且"无重型框架"是 Cargo.toml 的既定哲学（§1：零第三方
  并发依赖）。需要补的是**确定性纪律与可观测性**，不是调度器换代。
  **【2026-10-17 失效更新】**本决策已被用户拍板覆盖：「rayon 全量替换自研
  TaskPool」（分支 rayon-switch）。执行端换代、纪律不动摇：rayon `ThreadPool`
  出线程（生产三域走 `TaskPool::shared` = 全局池 + 配额门，收口三池分立
  超订，docs/audit/bevy-sys-asset.md G1 收口），hi/lo 优先队列、三计数、
  任务级 `catch_unwind`（G1a）与三 sched 失败回执（DC-3）全部保留为记账层
  语义；`TaskPool::new`（专属池）保留给「名额=线程」强并发契约测试面
  （worldgen turnstile 乱序注入等）。
- **文件面**：game/mcv_core/src/pool.rs、mcv_core/tests（新增）。
- **golden/确定性影响**：无（不改任何生成/渲染语义）。
- **红测试**：panic 任务后池仍满编（后续 hi/lo 任务正常执行）；hi 先于 lo 排空
  （既有语义补锁）。
- **回滚面**：单文件回退，无行为耦合。

### E2 —— 世界生成并行确定性锁（低风险：已并行，这波是"上锁"）

- **目标**：把"同 seed 字节不变"从**事实**升格为**测试锁**，并行化才敢再动：
  ① 同 seed 下 N 块并行（多 worker 池、提交序打乱、结果到达序乱序）
  vs 单线程串行逐块生成——`TerrainOutput.voxels`/`heightmap` **逐位对拍**
  （legacy + vanilla 双后端都要）；② 多次运行互拍（杀掉 mpsc 到达序影响——
  commit 顺序本就不影响内容，因为每块是纯函数，但这从未被测试锁住）。
- **纯函数性依据（并行安全的地基，勘察确认）**：树跨块冠由邻域池扫描按同
  seed 重算，"树干在本块的池负责全树，邻池只贡献伸入本块的冠"
  （vanilla/tree_pass.rs:227-234）；表内核/vanilla 管线均无共享可变状态
  （terrain.rs 全文仅 `'static` 只读常数表）。**落库顺序确定性已由设计达成：
  内容 = f(seed, pos)，与到达序无关**——E2 用测试把它钉死，未来任何 worldgen
  波（表层/树/结构）回红即知破坏纯度。
- **顺带（同文件面）**：在途名额调参位——现有硬上限 4（对齐原版
  DistanceManager.java:55）小于池 worker 数时喂不满池（Cargo.toml
  `[profile.dev.package.mcv_worldgen]` 注释的实测口径：dev 无优化单块数百
  ms、O3 后才赶得上流式回归帧窗——worker 单任务耗时以 E0 计时为准）；
  给 `max(4, workers)` 的调参常量并登记与原版的裁剪差异。
- **文件面**：game/mcv_worldgen/src/lib.rs + tests/golden.rs（或新 tests/parallel.rs）、
  game/mcv_logic/src/ticket.rs（名额常量）。
- **红测试**：上述逐位对拍；到达序乱序注入（受控延迟包装 tx）。
- **回滚面**：测试-only + 常量调参，零运行时风险。

### E3 —— 网格化并行：worker 跑 greedy mesh，主线程只做快照与提交（中风险）

- **目标**：remesh 预算流的"每块 ~10ms 纯计算"移出主线程，2 块/帧预算的实际
  帧成本从 ~20ms 降到 ~1-2ms（快照拷贝 + 结果入池）。
- **方案（快照-计算-回传三段式）**：
  1. **快照（主线程）**：维持现状的 3×3 拷贝（game.rs:126-141），拷完即发——
     拷贝是唯一碰 RwLock 的点，~0.5-1ms，留在主线程使锁纪律**保持平凡**；
  2. **计算（TaskPool）**：`Mesher::build`（opaque+water）纯计算，输入
     `Slot` 只读切片，无共享状态——移到 worker（spawn_hi：编辑源块优先
     通道 mesh_priority 走 hi，预算流走 lo）；
  3. **回传（主线程）**：worker 返回顶点/索引字节，主线程
     `uploader.build_chunk` 进 staging（MeshUploader 状态机与 wgpu 句柄
     **不出主线程**，绕开 GLES/adapter 线程亲和与 #80 状态机的时序锁）。
- **跨界边同步纪律（本波最难点，方案已含）**：网格把 3×3 邻域边面光烘焙进
  顶点，正确性门槛 = 邻域全部 `≥ LightLocalReady` 且边同步收敛（现状谓词
  `neighbors_ready` game.rs:2438-2451 + build_chunk_mesh 门槛 :2320-2341）。
  并行不改变门槛——**快照仍由主线程在固定帧序点做**（fixed_step 编辑之后、
  本帧边同步消化之后），脏位捕获时点不变，所以"拿到快照的那一刻邻域自洽"
  与现状同构，无新增时序面。真正的新增风险是**快照撕裂**（主线程拷 3×3 期间
  无编辑可打断——主线程内串行天然成立，worker 只见字节——维持该结构即无撕裂面）。
- **golden/确定性影响**：mesher golden 三表与 perf 哨兵（golden.rs:312-340）
  必须逐位不动（mesher 本体零改动，只换执行位置）；新增红测试 = 同输入
  worker 产物 vs 主线程产物**逐字节相同**（确定性 = 纯函数 + 无共享）。
- **红测试设计**：① 逐字节等价锁（上）；② 玩家编辑当帧：mesh_priority hi
  通道先于 lo 预算流到达上传（顺序锁）；③ 流式回归（streaming.rs 帧窗不变红）。
- **回滚面**：env 开关 `MCV_MESHER_WORKERS=0` 回落同步路径（对齐
  `MCV_TERRAIN_BACKEND` 先例，mcv_worldgen/src/lib.rs:38-40）；ChunkMesher
  trait 不动，RustMesher 内部分叉。

### E4 —— 存/读盘 IO 异步化：独立 IO 线程 + 两段式落盘（中风险）

- **目标**：region open/read/write 全部移出主线程；帧预算（读 4/写 2+积压/脏
  2）保留为**投递预算**，不再是执行预算。
- **方案**：
  - **独立单 IO 线程**（不是 TaskPool lo——IO 与生成争 worker 会互饿；原版
    同构：IO 走独立 executor，"原版 IO 走独立 executor 不入节流器"已是
    game.rs:2110-2112 的既有机制笔记）。通道用 mpsc（命令低频，锁环收益无）。
  - **写两段式（保住"失败保脏位"与 FIFO 序，game.rs:2054-2060 既有语义）**：
    主线程锁内快照 voxels+heightmap（~197 KiB）→ 投递 (pos, 快照) → worker
    写盘 → 完成信道回主线程，**成功且该块 SAVE 未被再次置位时才清位**；
    编辑再置位则下轮以新快照重写。写盘顺序无关紧要（per-position region 槽位，
    同块最新快照覆盖）；失败保脏位语义逐字保留。
  - **读异步化**：投递读请求 → worker 读 region → 结果回主线程走既有
    `advance_to(TerrainReady)` 接线；在途记账区分生成/读盘两类（req_frames
    扩展 io_frames 或 tag 位），复用"Empty 在途 300 帧驻留"防乒乓机制
    （game.rs:1975-1990）。
  - 30s 全量兜底（app.rs:2815-2819）自然退化为投递扫描（既有注释方向：
    "30s 全量存盘退化为兜底扫描"）。
- **vanilla 锚**：存盘编码在后台线程池——`CompletableFuture.supplyAsync
  (data::write, Util.backgroundExecutor())`（ChunkMap.java:773），解析同
  （:566、:924）；写失败报告路径 `reportChunkSaveFailure`（:775-777）。
- **golden/确定性影响**：存档内容 = 体素快照，与写盘时序无关；读回内容 =
  盘上内容。存档格式零改动 → mcv_save golden/读写回归不变。
- **红测试**：① 写失败（测试替身 IO）→ SAVE 保留 → 重试成功清位；
  ② 同帧编辑 + 落盘 → 最终盘上内容 = 最后编辑（竞态窗行为锁）；③
  在途读期间该块 Empty 驻留不卸载（防乒乓锁）。
- **回滚面**：env 开关回落同步 IO 路径；mcv_save 零改动。

### E5 —— 光照 init 并行（边同步保持串行）（高风险，最后动）

- **目标**：init 2 块/帧串行 BFS → worker 并行，加载期布光波推进速度 ≈
  worker 数倍。
- **方案**：与 E3 同构——主线程快照 voxels+light+heightmap（~295 KiB），
  worker 跑 `mcv_light::init` 纯计算，回传光字节，主线程写回 +
  `advance_to(LightLocalReady)`。**写回必须按玩家近优先的环序生效**
  （`sorted_keys` 序，game.rs:2381-2385 的既有确定性破平规则），保持
  "布光以玩家为圆心成波推进"的既有加载门时序。
- **铁纪律：跨块边同步（`sync_light_edges` + 成对标记）保持主线程串行，不并行**。
  理由（时序假设清单，全是审计 A1/A3/F2 用真机事故换来的）：
  LIFO push→pull 序（game.rs:358-381）、REMOVE-before-ADD 两相
  （:384-395）、in-flight 同键去重（:397-399）、512 步预算耗尽不丢
  （:405-414）、两端 `LightLocalReady` 门（:432-439）。这些假设建立在
  "边任务按确定序在同一线程消化"之上，并行化的正确性证明成本 >> 收益
  （边任务本身是 K 级字节操作，不是 ms 级）。
- **golden/确定性影响**：光照 golden 表 + 跨块火把测试
  （game.rs tests:6326-6361）+ 边同步幂等锁（streaming.rs:384-424）全保；
  新增红测试 = 并行 init vs 串行 init 逐位对拍（含同帧多块回传序）。
- **回滚面**：env 开关；init 调用点单处（stream 内 :2195-2220），回退面最小。

### E6 —— ECS 资源快照去克隆（低成本高杠杆）+ 系统并行**明确不做**

- **目标**：消除每固定步两次整表 HashMap 克隆（§1.2-4）：`chunks` 改
  `Arc<HashMap<ChunkPos, Arc<ChunkHandle>>>`（game.rs:538），主线程写路径
  写时复刻（COW 语义：stream 的插入/删除频率 ≈ 流式事件级，不是每步），
  ECS Resources 持 Arc 快照零拷贝。行为零变化，rd16 下省 ~30 万表节点/s。
- **系统并行：登记为伪需求，不做**。理由（诚实判断）：① 架构是零 unsafe
  RefCell sparse-set，明确"系统本就单线程顺序执行"（mcv_ecs/src/lib.rs:19-21），
  无冲突声明机制，并行化 = 重写调度器 + 给全部系统加读写集声明；
  ② 收益前提是系统耗时 >1ms/步——E0 计时之前无证据；数百实体 × 现有
  五系统在 §2 排序里进不了前四。**重开条件**：E0 显示 Fixed 阶段 >2ms 且
  mob_ai 占比 >50%，届时先做"系统级数据并行 for_each"（SparseSet::for_each
  内部分块）而非调度器并行。
- **红测试**：既有 mob 行为锁全保（mob_inside/outside ring、arrow freeze 等
  game.rs tests:6663-6826）；新增 = COW 后资源快照与主表一致性（写时复刻
  时机锁）。
- **回滚面**：字段类型改动集中在 GameRuntime + ecs 消费侧，单波可逆。

### E7 —— 视距天花板工程（rd 16/32 前置，依赖 E4/E6）

- **目标**：把"渲染距离拉满"从内存不可能变成可选项。三件事：
  1. **palette 压缩**——**E7a 已落地**（`mcv_core::VoxelStore`，main
     f4cdec80；memory-budget.md §1 已按 palette 口径重算）。债登记：
     盘格式仍平铺 u16（save/load 一次编解码，盘 palette 化 = 债）、
     global 档位宽 16、无 env 回滚开关（回退 = 分支 revert）；
  2. **streaming 增量化**：`level_map`×2、`sorted_keys`×2、票表重建从每帧
     O(N) 改事件驱动/脏标记（票变更沿 + chunks 集合变更沿触发）；
  3. `DIST_MAX` 解禁路径：options.rs:9 的 4..16 → 4..32 与设置 UI 步进域、
     预算门、far 平面（game.rs:1873 `rd*16*1.6`）联动重算。
- **风险**：改 GameRuntime 核心容器类型（与 E6 同型）；**依赖 E4**（IO 不再
  占主线程）与 E6（Arc 化先行，同一容器的两次改造合流）。
- **golden/确定性影响**：streaming 门谓词与帧窗回归是回归锁（streaming.rs
  全表）；rd32 下装载吞吐预算（8 块/帧）与在途节流需重验。

---

## 4. 风险与不变量清单（多线程最容易破的本仓硬约束）

每条给"并行化后如何保住"的机制。**本节是蓝图的核心**：数据竞争/时序依赖
破坏是引擎波最大风险。

| # | 不变量 | 现状锚点 | 并行化怎么破 | 保住机制 |
|---|---|---|---|---|
| 1 | **同 seed 确定性**：同 seed 世界逐位相同 | 生成纯函数（terrain.rs；树邻域重算 tree_pass.rs:227-234）；后端构造期固定（lib.rs:194-213） | worldgen 波若引入共享噪声状态/缓存（如跨块复用采样器）即破；乱序 commit 本身无害但从未被锁 | E2 双后端逐位对拍锁：并行/乱序/串行三态同拍；新增跨块缓存的波必须先证明"缓存对 f(seed,pos) 透明" |
| 2 | **mesher golden 三表 + perf 哨兵** | golden.rs 三表 + rust_mesher_perf_single_chunk（release 10ms 预算门） | E3 只换执行位置；若顺手"优化"烘焙序/AO 遍历序即破 | mesher 本体零改动原则；逐字节等价红测试（worker vs 主线程）；golden 表 CI 绿是合并门 |
| 3 | **光照跨块边时序**：LIFO push/pull、REMOVE-before-ADD、in-flight 去重、512 步不丢、两端就绪门 | game.rs:358-399, 400-439（审计 A1/A3/F2） | 任何把边任务移出主线程/乱序消化/丢任务的改动 → 黑缝与乒乓（真机事故复现） | **边同步永久串行**（E5 铁纪律）；预算-不-丢语义单测保留；幂等锁（streaming.rs:384-424）CI 绿是合并门 |
| 4 | **快照一致性（3×3 邻域 / 布光输入）**：网格与 init 的输入必须是自洽瞬间 | 现状由"全部主线程串行"平凡保证 | E3/E5 若让 worker 自行碰 RwLock 拷贝，会与编辑/边写回交错 → 撕裂快照 → 陈旧网格/错光 | **快照只在主线程做**（锁内一次一锁、无编辑可交错），worker 只见不可变字节；该结构写入 E3/E5 方案红线 |
| 5 | **streaming 门谓词**：加载门=status≥Surface（stage 位）、出生门=出生列邻域 TerrainReady、在途驻留防乒乓 | chunk.rs:107-119（commit 只在请求环内）；game.rs:1975-1990, 2160-2189 | E4 读异步化把"读盘完成"从同步点变事件点，若在途记账不区分生成/读盘，驻留/撤回判定错位 | 在途记账分类（io_frames/生成 tag）；门谓词读 stage 位不变；streaming.rs 帧窗回归是合并门 |
| 6 | **渲染帧差**：headless GPU 回归（lavapipe + GLES 观察门） | .github/workflows/ci.yml render-headless job | E3/E5 改动上传/回传时序 → 同帧 draw 可见性窗口错位（#80 时序：staging submit 先于同帧 draw） | 保持"flush 于 stream 末、draw 之前"的帧序不变量（game.rs:2310-2313 注释）；渲染测试三绿是合并门；GLES 观察行转硬门禁随 E3 落地评估 |
| 7 | **stage/status 单调 CAS**：只进不退、跨档直进 | chunk.rs:173-207（含单测） | 并行回写（init 光字节写回、读盘 advance）若乱序，"回退"会被 CAS 静默吞掉——当前正确；但**跨档直进**意味着乱序写入低档无效、高档提前——语义安全，唯"写回覆盖"要防（光字节是非原子的 RwLock 覆盖写） | 光字节写回只在主线程、且仅在 stage 未达 LightLocalReady 的块上（已 init 的块不许被过期 worker 结果覆盖——回传带快照代次校验） |
| 8 | **RNG 消费序**：weather.tick(spawn_rng()) 与 try_natural_spawn 的主线程单点消费 | game.rs:2564, 2660-2666 | 任何把 tick 域系统并行的尝试都会重排 RNG 消费序 → 同 seed 回放破（对齐 #1） | tick 域系统永久串行（本蓝图所有波不动 fixed_step 的 tick 域结构）；E6 也只改数据布局不改执行序 |
| 9 | **任务 panic ≠ 池缩编**（新增不变量，现状是洞） | pool.rs:79 裸调用，:105 join Err 被忽略 | E2-E5 把更多计算放进池，panic 概率随面积扩大；缩编后加载洪峰静默变慢、无任何告警 | E1 catch_unwind + 告警 + 满编红测试 |
| 10 | **主线程独占 `GameRuntime.chunks` 可变权** | 全仓事实（worker 只持 Arc<ChunkHandle>） | 任何 worker 回调闭包捕获 `&mut GameRuntime`/`&GameRuntime` 即死锁/竞争 | 快照-计算-回传三段式红线（E3/E4/E5 统一模式）：闭包只捕获克隆数据与回传 tx；由 code review + 类型签名（worker 入口收 owned 数据）保证 |

---

## 5. 诚实判断：哪些已多线程 / 哪些是伪需求

**已多线程（本文勘实，勿重复立项）**：世界生成（TaskPool+mpsc，在途 4 节流，
2024 年即已落地）；GPU 网格上传（#80 staging 环异步化，兜底路径在案）；音频
（RT 回调线程 + 无锁 SPSC）。

**伪需求（明确不做，理由如上）**：rayon/work-stealing 换代（任务粒度 10ms，
手写池够用，依赖哲学不合）【2026-10-17 失效更新：用户拍板 rayon 全量替换，见 §3 E1 内注】；ECS 系统并行调度器（无冲突声明架构 + 无热点
证据，重开条件见 E6）；主循环渲染线程拆分（winit 单线程模型 + #80 已消掉
主线程 GPU 阻塞，拆线程引入的帧同步复杂度 >> 收益；移动端 GLES 上下文亲和
另加一票反对）。

**真串行墙（值得立项，按收益/风险比）**：网格化 worker 化（E3，收益最大）；
存读盘 IO（E4，掉帧尖刺根因，既有登记）；光照 init（E5，收益明确但纪律最重）；
ECS 资源克隆（E6，rd16 起才可测）；streaming O(N) 增量化 + palette（E7，
rd32 的真正前置——第一堵墙是内存不是线程）。
