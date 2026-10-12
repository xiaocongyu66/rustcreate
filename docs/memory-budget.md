# 内存预算（v6 世界域）——整区块重构刀 5（#115）

收口刀落地的内存记账口径表：区块 resident、网格域、图集域三本账的算式、
预算门与 CI 断言位置索引。预算数 = 派单 #115；实测值 = CI `K5-MEAS` 行
（ harness=false 测试，stdout 直落 job 日志，绿跑可读）。

## 1. 区块 resident 算式（v6）

v6 世界域：384 高（`WORLD_MIN_Y=-64`，`mcv_core/src/lib.rs:26`），
16×16×384 = 98304 格/块（`CHUNK_VOL`，lib.rs:19）。每块 resident =
`ChunkHandle` 三个数组（`game/mcv_core/src/chunk.rs:134-159`）：

| 数组 | 类型 | 字节（最坏上界） |
|---|---|---|
| voxels | `VoxelStore`（E7a palette 压缩，24×16³ section；档位 0/4/5-8/16bit 对齐原版 Strategy.java:33-48） | 24×4096×2 = 196 608（global 16bit 顶档；典型地形 ≪ 此，见下） |
| light | `Box<[u8; 98304]>` | 98 304 |
| heightmap | `Box<[i16; 256]>` | 512 |
| **合计** | | **≤ 295 424 B ≈ 288.5 KiB/块（最坏上界；平铺同值）** |

E7a 起 voxels 驻留 = `VoxelStore::resident_bytes()`（打包 long 数据 +
palette 表实测口径）：全空气 section 0bit（0 B 数据）、2..16 值 section
4bit（2 KiB）、17..256 值 5-8bit（2.7-4 KiB）、>256 值才回退 16bit
（8 KiB）——多数真实生成柱 ≈ 4-16 KiB vs 平铺 192 KiB。

算式锁（E7a 改版）：`memory_budget.rs` `PER_CHUNK_BYTES == 295424`
仍锁最坏上界；新增实测口径 = Σ `voxels.resident_bytes()` + light +
heightmap，配套两道新门：全空气柱 voxels ≤ 1024 B（0bit 生效硬证）
与 vanilla 后端典型生成柱 ≤ 32 KiB。

**稳态口径**：预算门只锁「静止稳态」——在册集 = 请求环 =
`sim_dist = rd+1`（`game.rs` stream 螺旋 `0..=sim_dist`，含 +1 支撑环 =
原版 view distance 语义）；无移动 → 无卸载 → 无迟滞滞留、复活缓存空
（测试断言 `pending_unload_count()==0`）。

### 视距 → 块数 → 字节（静止稳态）

| 视距 rd | 请求环 (2(rd+1)+1)² | resident 块数 | 平铺基线（E7a 前实测） | E7a 预算门 |
|---|---|---|---|---|
| 8（桌面默认） | 19² = 361 | 361 | 106 648 064 B ≈ 101.7 MiB | **≤ 60 MiB** |
| 6（Android 默认） | 15² = 225 | 225 | 66 470 400 B ≈ 63.4 MiB | **≤ 35 MiB** |

E7a 收紧依据：voxels palette 化后 light（96 KiB/块）成为主体，
361×(~106 KiB) ≈ 37 MiB、225×~106 KiB ≈ 23 MiB，门留 ~1.6× 地形
波动余量。实测落 CI `K5-MEAS`/`K5-MEAS-VOX` 行（voxels 分项单列）。

实测（CI `K5-MEAS` 行，run 38114958737 / 7a35424，test job）：

| 档 | resident_chunks | resident_bytes | 折算 | 预算余量 |
|---|---|---|---|---|
| vd8 | 361（= 请求环，无 ε 毛刺；steady 1165 帧） | 106 648 064 B | 101.7 MiB | 15.2% |
| vd6 | 225（同上；steady 772 帧） | 66 470 400 B | 63.4 MiB | 9.4% |

实测与算式逐字节吻合（在册数恒 = 请求环）。

### 原版对照

- 视距：档位 4/12/16/32（`net/minecraft/client/Options.java:81-84`）；
  `IntRange(2, maxMemory≥1GB ? 32 : 16)`、**默认 12**、无平台分支
  （`Options.java:1405-1409`），弱机靠玩家自调。本仓桌面 8 / Android 6、
  持久化钳制 4..16 —— 内存预算主动裁剪，登记为裁剪债（刀 5b，
  `options.rs` Default 文档）。
- 块存储 palette：原版 `PalettedContainer.java:25-29`（`Data` =
  Configuration + BitStorage + Palette）+ `Strategy.java:8-16`（0 bit
  单值 / 1-4 bit 线性 / 5-8 bit HashMap / 全局）与 `:33-47` 块态档位
  —— 常见地形 palette 4 bit ≈ 12 KiB/块存储（另加 light）。
  **E7a 已落地**（`mcv_core::VoxelStore`）：档位表/copyFrom 换档/只升
  不降逐项对齐原版；偏差登记=global 档位宽 16（值域全域 u16）、盘
  格式仍平铺 u16（save/load 一次编解码，盘 palette 化 = 债）、无 env
  回滚开关（存储表示无法双路，回退 = 分支 revert）。

## 2. 网格域（mesh）

记账（刀 5a）：`BatchingStats.resident_bytes` / `cpu_pool_bytes`
（`engine/mcv_render/src/gpu.rs:136-140`），`batching_stats()` 调用时
现算（`gpu.rs:239-263`，~1 Hz 调用；上传热路径零改动、无锁无失败路径）。
镜像到 `MeshStats`（`game/mcv_logic/src/game.rs:77-80`）并由
render-stats 日志带出（`app.rs` ~1 Hz）：

```
render-stats: draws=… water_draws=… entries=… chunks=… rebuilds=… rebuild_kb=…
              staged_uploads=… staged_kb=… fallback=… resident_kb=… pool_kb=…
              trimmed=… evicted=… debt=…
```
（`trimmed`=环外修剪累计、`evicted`=环内盲逐出警报恒 0、`debt`=在途
remesh 债；#174 观测面，真机隐形墙回归时 evicted>0 或 debt 常态堆积。）

两本小账：

- **GPU 驻留**（resident_bytes）：Σ批次条目 `vertex_buf + index_buf +
  water_index_buf` 缓冲尺寸。批次成员共享缓冲（≤8 块/批、≤4 MiB/批，
  `gpu.rs:76-77`），按整缓冲尺寸计——含 wgpu 4B 对齐放大，即驱动侧
  真实占用量。
- **CPU 留存**（cpu_pool_bytes）：Σ池内 `PoolMesh` 顶点+索引字节
  （与 `batch_joinable` 的 mesh_bytes 同一公式）。池是合批重建的数据源，
  在用批次成员不可回收。**#174 隐形墙修复后的治理口径**：越
  `CPU_POOL_BUDGET_BYTES`（60→96 MiB 重标定）只修剪**保留环外**的远场
  滞留（`set_retention(anchor, unload_dist)` 每帧随玩家迁移；环内绝对
  保护、永不静默逐出——旧 LRU 逐「最早加载的近场块」且游戏层零通知 =
  真机整墙透明/打掉一块全显形/走近偶愈），被逐块转入 **remesh 债账**
  （`drain_evicted` 每帧回灌 `dirty::MESH` 重建，`game.rs` stream 网格段；
  统计面 `trimmed_chunks` 正常治理 / `evicted_chunks` 环内警报恒 0 /
  `debt_chunks` 在途债）。`meshed_chunks` 口径 = 池 ∪ 债（committed 未
  卸载），「游戏零 unload 则 meshed == committed」恒成立。
  原版对照：本仓「池」的等价物 = 客户端渲染保留区——
  `ClientChunkCache.java:158-160`（`calculateStorageRange =
  max(2, viewRange) + 3`，**只按距离不按 LRU**）+
  `ChunkMap.java:477-496`（processUnloads 距离驱动卸载迟滞）；
  `PlayerChunkSender.java:92,103`（发送队列按离玩家距离排序，无时间序
  淘汰）。原版无 CPU 网格缓存这一层，「逐出」在本仓只许发生在游戏层
  流式环（`unload_dist`，= 保留半径）之外。

### 网格域预算与实测

RD=8 生产网格域 = remesh 环 = sim_dist = **19² = 361 块**（比可见环
17²=289 多一圈支撑网格；`game.rs` remesh 环过滤「可见网格半径 ≤
sim_dist」）。CI 断言跑在合成地形上（双频正弦高度场 + 海面水柱，真
mcv_mesher 双 pass + 生产同款 `build_chunk`/`flush`）——它是记账量级
回归门，不是真机内存保证（真实地形树冠/洞穴更大，真机侧看 render-stats）。

实测（CI `K5-MEAS mesh` 行，run 38114958737 / 7a35424，lavapipe）：

| 项 | 实测（合成高度场） | 预算门 |
|---|---|---|
| GPU 驻留（resident_bytes） | 50 690 400 B ≈ 48.34 MiB（~140 KiB/块） | ≤ 96 MiB（= 2× 实测收紧后） |
| CPU 池（cpu_pool_bytes） | 50 690 400 B ≈ 48.34 MiB | ≤ 96 MiB（同上） |
| draw_entries（批次数） | 51（361 块 → 批均 ~7.1，逼近成员上限 8） | — |
| meshed_chunks | 361 | = 网格域块数 |

**洞穴灌水口径（#174，真机隐形墙的字节化实证）**：同 RD=8 环 361 块，
换「4³ 粗格凿空 ~15% + 海平面以下腔体灌水」断面（洞壁面 + 水空/水岩
界面双 pass 全负载），单块网格字节 **avg_chunk_bytes = 463 KB**（vs
合成场 ~140 KiB，3.3×），池稳态需求 **≈ 167 MB（160 MiB）**（run 38255019997
`K-INVIS cave` 行，`engine/mcv_render/tests/inviswall_eviction.rs`，
render-headless）。旧 `CPU_POOL_BUDGET_BYTES = 60 MiB`（48.34 +25%，
按无洞穴合成场标定）在此口径下稳态即越界 → 逐出风暴；该值已重标定
为 **96 MiB**（同「2× 合成实测」纪律）。96 < 167 是**有意权衡**：预算
只修剪保留环外远场（见上），洞穴环整体在保留环内不受预算门逐出，
池实际上界 = 保留环面积 × 单块字节——rd8 洞穴最坏 23²×463 KB ≈
245 MB（234 MiB）量级，超移动内存盘可承受线；**洞真机 rd 自适应降档 = 债**
（§5）。目标语义：内存压力下失明只可能发生在远方流式环边缘（且经
drain 回灌 1-2 帧重建），视野中心永不瞎。**#175 含水层落地注**：生成器
「海下全灌」平铺模型已被原版含水层判据取代（地下空腔默认干、注水囊袋
噪声门控，`game/mcv_worldgen/src/vanilla/aquifer.rs`），真机水双 pass
占比随之回落；本段 463 KB/块 口径保留为**最坏标定**——K-INVIS fixture
的灌水是手工注入断面（不读生成器判据），数值不受含水层落地影响，不许
以「生成器变干」为由下调该口径或改动 inviswall_eviction.rs。

**收紧纪律**（派单）已执行：首跑暂定派单估值 40 MiB——run 38114958737
实测 48.34 MiB 如约触发取证红（`resident_bytes ≤ MESH_BUDGET_MIB` 断言
panic，K5-MEAS 行先行落日志），随后按 2× 实测收紧为 96 MiB
（`MESH_BUDGET_MIB` 注释登记实测值与 run 号）。真机 Mali 实测另走
render-stats 取证（`docs/real-machine-checklist.md` §9）。

## 3. 图集域（atlas）

已在 gpu.rs 落账（刀 5 前）：`atlas::LAYERS = 828+10 = 838` 层（827 真实
+ 1 missing 哨兵 + 10 裂纹，`mcv_core/src/atlas.rs:19-35`）、
`MIP_LEVELS=2`（:36）、`TILE_PX=16`（:15）。每层 mip0 16×16×4B = 1024 B
+ mip1 8×8×4B = 256 B → **838×1280 = 1 072 640 B ≈ 1.02 MiB**（固定
开销，与视距无关）。布局断言在 `engine/mcv_render/src/gpu.rs:1229-1235`
（payload 尺寸 == LAYERS×(mip0+mip1)，漂移即红）。

## 4. CI 断言位置索引

| 预算 | 断言位置 | 数值依据 |
|---|---|---|
| resident vd8 ≤ 120 MiB | `game/mcv_logic/tests/memory_budget.rs:136` `run_case("vd8", 8, 120)` | 361×295424 = 101.7 MiB + 15% 余量（派单 #115） |
| resident vd6 ≤ 70 MiB | `memory_budget.rs:137` `run_case("vd6", 6, 70)` | 225×295424 = 63.4 MiB + 10% 余量（派单 #115） |
| 单块算式 295 424 B | `memory_budget.rs:40` + 尺寸/算式双断言 | v6 算式锁 |
| 网格域 GPU/CPU ≤ 96 MiB | `engine/mcv_render/tests/mesh_memory.rs`（`MESH_BUDGET_MIB`） | 2× 实测 48.34 MiB（run 38114958737；首跑 40 MiB 暂定门已按纪律取证+收紧） |
| 图集布局 1 072 640 B | `gpu.rs:1229-1235` assert | LAYERS×mip 布局 |

两个 resident 测试的驱动模式与超时预算：稳态 = 在册数 400 帧不变且在途
生成清零（`STEADY_FRAMES=400`）；帧硬顶 60000（flap 测 81 块 ~900 帧实测
×4.5 倍块数 ×2 倍争用余量）——timeout 类断言按派单纪律留 2 倍余量。

## 5. 口径外足迹（债，不在预算门内）

- **移动足迹**：迟滞环（卸载边界 Chebyshev rd+4 = `unload_dist()`，
  `game.rs:1218`；票级别场边界语义）+ 复活缓存（≤64 块硬顶 + 积压门
  48，`game.rs:566`/UNLOAD_BACKLOG_GATE）。直线行走 rd=8 在册包络
  ~19×21 ≈ 399 块 ≈ 115 MiB，叠加复活缓存最坏 ≈ 137 MiB，超 120 MiB
  静态门——预算门只锁静止稳态；移动足迹的回收策略（迟滞环按方向裁剪
  /复活缓存提前落盘释放）留后续刀。
- **worker 侧在途副本**：worldgen 后台线程每块临时体素缓冲（生成期
  短命，≤4 并发）；不在 resident 口径内。
