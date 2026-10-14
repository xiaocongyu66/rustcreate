# Worker 路记账一致性（worker-accounting）

> 给未来贡献者：任何把工作移进后台线程/线程池的改动，先读完本文再动手。
> 本仓已因「记账与回执不一致」付过三次真机事故的学费。

## 背景：一桩真机事故引出的全路审计

**症状**：真机海上航行出现「隐形墙」——看得见的水面走不过去，碰撞/pick
行为与体素一致，但 GPU 上没有该块的网格（水雾吞掉水面洞 + 邻块按真实
体素剔除边界面，视觉上无任何痕迹）。海上高发。

**取证定根因**（以 /root/mc-ref/src-26.1 反编译实物核对过时序）：

1. `build_chunk_mesh` worker 路（game/mcv_logic/src/game.rs:3660-3699）在
   快照提交的同一主线程瞬时完成 `meshed.insert(pos)` + `clear_dirty(MESH)`
   + `advance_to(Uploaded)`；此后 worker 产物丢失**无任何回路重置**。
2. 任务 panic 被池隔离吞掉只打 error 无重试（game/mcv_core/src/pool.rs
   `run_isolated`，`AssertUnwindSafe`，注释自认"不是任务重试"）。
3. 回传产物 `opaque==None` 时 flush 跳过，注释自认"MESH 位已清除，下一笔
   编辑恢复"（旧 game.rs:234-240）——下一笔编辑可能是很久以后。
4. `mesh_fail` 计数+退避是同步路专属，worker 路无对应物。

**结果态** = 体素真实（碰撞/pick 读它）+ meshed 记账在场（永不重派）+
stage=Uploaded（门全放行）+ GPU 无网格 = 隐形墙。

顺着同一根因做全 worker 路审计，发现四条同构病路（三路调度器 + 一处
门谓词），一并修复——见下文模式与现状表。

## 四变体模式：记账推进 × 产物确认的配对失败

所有病路都是同一个形状：**派发时推进了某种记账，但记账的撤回依赖一个
可能不发生的「产物确认」**。按配对缺失的形态分四变体：

### DC-1 记账即门（记账在账 = 重派门恒闭）

派发时记「已做」账，重派逻辑以「账不在」为资格——账永不撤销 = 该块
永不重派。

- **本案**：网格 meshed 记账（上节 1/3/4）。
- **修复**：① worker 闭包内 catch_unwind 合成丢失标记（mcv_mesher/src/
  sched.rs `submit`，`opaque: None`）；② flush 对丢失标记登记事件（game.rs
  `RustMesher::flush`）→ 游戏层即时回滚三笔账（`mesh_worker_lost` →
  `mesh_rollback`：meshed 删账 + MESH 置脏 + stage 回退）；③ 主线程每帧
  K 帧看门狗超时兜底（`mesh_watchdog_scan`，K=10，实测依据见 game.rs
  `MESH_WATCHDOG_FRAMES` 注）；④ 同块连续丢失 M=3 次 → 断路器降级同步路
  （`mesh_sync_fallback`，防无限重派风暴）。

### DC-2 记账即配额（记账占坑 = 配额/吞吐泄漏）

派发时占用一个名额/在途位，回执才释放——任务蒸发 = 永久占坑，泄漏满
额后整条流水停摆。

- **光照 inflight 死账**：`LightScheduler::submit` 派发即 `inflight.insert`
  （mcv_light/src/sched.rs）；worker `init` panic → 无 LightDone → inflight
  永久滞留 → 重派门（game.rs `light_dispatch` 的 `in_flight→continue`）恒
  拒 → 黑块（光照全 0）+ `neighbors_ready` 门连锁冻结 3×3 邻域网格、
  Light/Full 永不推进。
  - **修复**：worker 闭包 catch_unwind，panic 时 send `LightDone { light:
    vec![], failed: true }`；drain 对失败回执**只解 inflight 不写回不推进**
    （空缓冲写回会把已布光清零）。
- **世界生成 req_frames 泄漏**：请求环 `req_frames.insert`（game.rs:3131）
  = 配额记账；worker panic（generate_terrain_with）→ 无 GenResult → 名额
  滞留；泄漏满 `in_flight_quota = max(4, workers)`（ticket.rs）后
  `pops = quota − len ≡ 0` 全图生成停摆。且 Err 产物路径虽释放名额，Empty
  手柄留册 → 请求环 `contains_key→continue` 永不重请求（死块）。
  - **修复**：request 闭包 catch_unwind，panic 时 send
    `GenResult::Terrain(Err((pos, ERR_PANIC)))`（drain 端既有 Err 分支释放
    名额，零新增记账）；Err 分支追加移除仍为 Empty 的手柄（打开重请求
    回路，pops 名额门自限防风暴）。

### DC-3 记账即清位（失败路径有回执）——本仓范本

失败不是「没有产物」，而是「产物 = 失败标记」：每笔派发恰好一笔回执
（成功产物或失败标记），记账推进与回执消化严格配对。**范本 = IO 线程**
（game/mcv_logic/src/game.rs `spawn_io_worker` + `failed_io_done`）：
命令 panic 时合成 `ok: false` / `found: None` 失败回执，保脏位/回退生成的
语义不因线程侧异常失效。三个调度器（mesh/light/terrain）的失败回执均
按此范本补齐。

### DC-4 记账即推进（乐观推进无确认配对）

派发即推进状态机终点（如 stage=Uploaded），把「已派出」当「已到位」。
单独看不是 bug——**前提是失败路径能把记账撤回**。缺了撤回回路，DC-4
就坍缩成 DC-1/DC-2。修复不是取消乐观推进（加载语义依赖它），而是补齐
「确认/丢失 → 撤回/重派」回路（即 DC-1 修复的 ①②③）。

## 逐路径现状表

| # | 路径 | 记账 | 状态 | 证据 |
|---|------|------|------|------|
| 1 | mesh worker 路（派发→回传） | meshed/stage/pending | **修复**：失败回执 + 即时回滚 + K 帧看门狗 + 断路器 | `mesh_parallel.rs::watchdog_redispatches_after_silent_worker_loss` / `::loss_marker_rolls_back_immediately_and_degrades_after_repeat` |
| 2 | mesh 同步路（回滚开关） | meshed/mesh_fail | OK（原有退避） | game.rs `build_chunk_mesh` 同步分支 |
| 3 | mesh 僵尸/陈旧过滤 | floor 地板 | OK | `mesh_parallel.rs::killed_position_result_never_committed` / `::stale_result_cannot_override_newer_commit` |
| 4 | light worker init | inflight | **修复**：failed 失败回执 + drain 拒收档 | `light_parallel.rs::panicking_init_sends_failure_receipt_and_reopens_inflight_gate` / `::failed_receipt_never_admitted_by_drain` / `::panicking_light_init_redispatches_and_advances_stage` |
| 5 | light 僵尸过滤 | floor 地板 | OK | `light_parallel.rs::killed_position_result_never_admitted` |
| 6 | terrain request | req_frames + Empty 手柄 | **修复**：Err 失败回执 + 手柄移除 | `terrain_receipt.rs::panicking_generate_releases_quota_and_recommits` |
| 7 | 生成池静默门（孤儿回收前置） | gen_inflight 自记计数 | **修复**（B 修）：计数判据替换池内判据 | `terrain_receipt.rs::undrained_receipt_window_does_not_evict_empty_chunk` |
| 8 | IO 线程命令 | outstanding/脏位 | OK（范本） | game.rs `spawn_io_worker`/`failed_io_done` |
| 9 | IO 写失败 | SAVE 脏位 | OK | game.rs `process_io_done` |

## 新增 worker 路径检查清单（合入门槛）

派发任何任务到后台线程前，回答三问——**任何一问答不出就不许合入**：

1. **我的记账在派发时推进吗？**（在途表/配额槽/网格记账/状态机档位——
   写下它在哪个 file:line、谁消费它。）
2. **失败（panic / Err / 回执丢失）有回执吗？**（闭包内必须自带
   catch_unwind 合成失败标记——池层 `run_isolated` 只保 worker 线程存活，
   **不保任务产物**；被吞的 panic 不产生任何回执。）
3. **回滚回路在哪？**（谁在收到失败标记时把第 1 问的记账撤回/重派？
   回执通道本身丢失的极端面，有没有超时兜底？兜底重派有没有断路器，
   不会无限重派？）

一句话原则（写进每个调度器模块头）：**记账推进必须与产物确认配对；
失败路径必须有回执（DC-3 范本）。**

## 真机验证判据

**logcat 对账**（派发/回执事件均带区块坐标与 seq）：

```sh
adb logcat -d -s RustMcv | grep -E 'mesh dispatch|mesh worker commit|mesh watchdog|light dispatch|light worker commit|terrain gen failed'
# 回传延迟对账：同区块 dispatch 行与 commit 行的 seq 配对，帧数差即可观测回传延迟
# 丢失面：dispatch 有、commit 长期无 → 看门狗行（warn=回滚重派 / error=降级同步路）
```

**render-stats 新字段**（~1 Hz 一行）：

- `dispatch_gap`：mesh 派发超 K 帧未见产物确认的在途块数（>0 = 丢失疑点
  在窗，回滚重派在即；静止期常态 >0 = K 过小或持续丢失）。
- `worker_lost`：worker 产物丢失事件累计（mesh 丢失标记/超时 + light/
  terrain 失败回执；自愈后仍累计——真机复现的一条日志定位面）。

**静置 2 秒实验**（隐形墙判据）：进世界后停止一切操作静置 2 秒
（≈120 stream 帧 ≫ K=10），取 render-stats 数行：

- 通过判据：`chunks`（记账）与实际渲染网格一致、`dispatch_gap=0`、
  `worker_lost` 不增长；logcat 内同一区块的 dispatch 与 commit 一一配对，
  无 watchdog 行。
- 失败判据（复现）：`dispatch_gap` 反复 >0、`worker_lost` 增长、logcat
  出现 `mesh worker产物缺失`/`worker build panicked` —— 失败回执/看门狗
  已接住，按坐标对账定位丢失源。
