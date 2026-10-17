# rayon 全量替换自研 TaskPool（用户拍板 2026-10-17）——分支 rayon-switch 收账

状态：施工完成（本文件为终报底稿）。基线：分支即 origin/main（a116c019）。
对照审计：docs/audit/bevy-sys-asset.md（G1 三池分立超订 / G1a 纪律重建）。

## 1. 使用面清单（git grep TaskPool/test_pool 摸底，rayon-switch 波）

| 域 | 位置 | 旧用法 | 新映射 |
|---|---|---|---|
| 池本体 | game/mcv_core/src/pool.rs | Mutex+Condvar 手写 worker 循环 + hi/lo 双级队列 + 三计数 + run_isolated | rayon 执行端 ×2 + 配额门（纪律原位） |
| E2 worldgen（#157） | game/mcv_worldgen/src/lib.rs `TerrainScheduler` | `TaskPool::new` + per-chunk `spawn_lo` + mpsc 回执 + gen_inflight 自记在途 | `TaskPool::shared`（全局池 + 配额门）；回执/在途零改动 |
| E3 mesher（#159） | game/mcv_mesher/src/sched.rs `MeshScheduler` | `TaskPool::new` + `spawn_hi/spawn_lo` + seq/Gate + mpsc 回执 | `TaskPool::shared`；hi/lo/seq/回执零改动 |
| E5 light（#163） | game/mcv_light/src/sched.rs `LightScheduler` | `TaskPool::new` + `spawn_lo` + inflight 门 + 失败回执 | `TaskPool::shared`；记账/回执零改动 |
| 池语义锁 | game/mcv_core/tests/taskpool_panic.rs | dedicated 语义 3 锁 + 新增 shared 锁 | `TaskPool::new` = dedicated 兼容名 |
| 乱序注入锁 | game/mcv_worldgen/tests/parallel.rs | `TaskPool::new(9)`（turnstile 依赖名额=线程） | 保持 `new`（dedicated），语义原样 |

注：E7 palette（#171）与 E4 IO 不在 TaskPool 面（palette 无并行池；IO 专线
std::thread 刻意不入池，game.rs:1350 注释仍准确）。render 快照通道 =
`MeshResults` mpsc（E3），结构未动。

## 2. 替换映射表

| 旧（自研池） | 新（rayon 收口） |
|---|---|
| 手写 `std::thread::spawn` worker 循环 ×n | `rayon::ThreadPool`（dedicated）/ rayon 全局池（shared）出线程 |
| hi/lo 出队由 worker 循环顺序保证 | hi/lo 在派发侧配额门实现（rayon 无优先级语义→记账层语义，grep `pump_locked`） |
| queued/running 原子计数（锁内增减） | 同级锁增减（`stats()` 快照自洽不变） |
| `run_isolated` 包 job（pool 层） | 同一点保留：rayon 派发任务裸 panic 会炸 worker，闭包首行即隔离（G1a） |
| 三 sched catch_unwind 合成失败回执 | **零改动**（DC-3 纪律：mesh 丢失标记/light failed/terrain Err(rc=-7)） |
| `Drop` shutdown+join | 关门弃队 → Condvar 等在途归零 → 等 Arc 归一（防 dedicated 模式闭包侧释放 Shared 在 rayon worker 内析构私有池 = worker 内 join 自身） |
| 三域各建 n=cpu−1 线程（8 核≈21 后台线程） | shared：一套全局池线程；配额=回压上限（`saturated`/`dispatch_budget` 基数语义不变） |

## 3. 红线保障

1. **worldgen 掷骰确定性锁**：每 chunk 纯函数 `content = f(seed, pos)`；
   rayon 完成序不影响输出——新增 `rayon_par_iter_index_backfill_bitwise_both_backends`
   （tests/parallel.rs ③）：par_iter 完成对 `(slot, out)` **按 index 原位
   回填**，输出序=网格序 bit 级不变；既有①②锁（turnstile 乱序注入 +
   两轮互拍）原样绿（走 dedicated 池，名额=线程语义未变）。
2. **mesher 三段式快照-回传契约**：MeshJob/MeshDone/mpsc/seq/Gate 提交地板
   全部零改动；probe 断言池路产物 vs 串行直呼逐字节一致。
3. **`MCV_MESHER_WORKERS=0` 回滚开关**：开关在 game.rs `new_with_workers`
   （workers==0 → 不装配 MeshScheduler → RustMesher 同步直算），池替换
   不触及；`MCV_LIGHT_WORKERS` 同构；`MCV_POOL_WORKERS` 语义（配额上限）
   保持原版 `max.bg.threads` 镜像。
4. **fluid/tint/mob 行为锁**：零触碰（diff 面 = pool.rs + 三 sched 构造行
   + 测试/文档/CI）。

## 4. 性能对比（K-RAYON 数值行，CI `--nocapture`）

口径：**串行直呼 vs 生产池装配**（同一进程同环境双数字；E0 分段计时 #155
的 app 层 1Hz 日志仍是真机口径，本节为 CI 侧可比锚）。
环境：GitHub Actions ubuntu-24.04（run ID 见终报），debug 构建
（worldgen 挂 dev-O3；mesher 纯 debug）。

- K-RAYON-GEN（vanilla，N=32 块批量）：serial_ms=__ / shared_pool_ms=__
- K-RAYON-MESH（N=32 块 opaque+water）：serial_ms=__ / shared_pool_ms=__

> 前账（替换前 TaskPool v2）专项数字未留档（E3/E5 波无 CI 侧探针；本机
> 禁 cargo 零运行），无法同环境重演——按「纪律语义逐锁等价 + 执行端同为
> n 线程池」论证等性，以本次 CI 双数字作为 rayon 收口的基准锚，后续波次
> 直接对 K-RAYON 行对照。

## 5. 回归面（必绿清单）

- mcv_core/tests/taskpool_panic.rs（dedicated 三锁 + shared 新锁）
- mcv_worldgen/tests/parallel.rs（①②③）
- mcv_logic 全量（mesh_parallel/light_parallel/terrain_receipt/streaming…
  ——回执/看门狗/丢失注入三态全走原 API）
- golden 三表（mesher/worldgen/os2s）零改动零漂移
