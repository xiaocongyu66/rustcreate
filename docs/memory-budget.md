# 内存预算（v6 世界域）——整区块重构刀 5（#115）

收口刀落地的内存记账口径表：区块 resident、网格域、图集域三本账的算式、
预算门与 CI 断言位置索引。预算数 = 派单 #115；实测值 = CI `K5-MEAS` 行
（ harness=false 测试，stdout 直落 job 日志，绿跑可读）。

## 1. 区块 resident 算式（v6）

v6 世界域：384 高（`WORLD_MIN_Y=-64`，`mcv_core/src/lib.rs:26`），
16×16×384 = 98304 格/块（`CHUNK_VOL`，lib.rs:19）。每块 resident =
`ChunkHandle` 三个数组（`game/mcv_core/src/chunk.rs:134-159`）：

| 数组 | 类型 | 字节 |
|---|---|---|
| voxels | `Box<[BlockId; 98304]>`（u16 平铺） | 98304×2 = 196 608 |
| light | `Box<[u8; 98304]>` | 98 304 |
| heightmap | `Box<[i16; 256]>` | 512 |
| **合计** | | **295 424 B ≈ 288.5 KiB/块** |

算式锁：`memory_budget.rs:40` `PER_CHUNK_BYTES` + 断言 `== 295424`
—— 世界尺寸/类型宽度任何漂移都会先红这里，预算表必须重算。

**稳态口径**：预算门只锁「静止稳态」——在册集 = 请求环 =
`sim_dist = rd+1`（`game.rs` stream 螺旋 `0..=sim_dist`，含 +1 支撑环 =
原版 view distance 语义）；无移动 → 无卸载 → 无迟滞滞留、复活缓存空
（测试断言 `pending_unload_count()==0`）。

### 视距 → 块数 → 字节（静止稳态）

| 视距 rd | 请求环 (2(rd+1)+1)² | resident 块数 | resident 字节 | 预算门 |
|---|---|---|---|---|
| 8（桌面默认） | 19² = 361 | 361 | 361×295424 = **106 648 064 B ≈ 101.7 MiB** | ≤ 120 MiB |
| 6（Android 默认） | 15² = 225 | 225 | 225×295424 = **66 470 400 B ≈ 63.4 MiB** | ≤ 70 MiB |

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
  —— 常见地形 palette 4 bit ≈ 12 KiB/块存储（另加 light）vs 本仓
  u16 平铺 192 KiB。**palette 压缩列待做**（26.1 本就 palette，不算
  发明；本仓 voxels u16 平铺是重构期简化）—— 登记后续刀，收益估算
  ~6-16×/块，届时本表与两道预算门同步重算。

## 2. 网格域（mesh）

记账（刀 5a）：`BatchingStats.resident_bytes` / `cpu_pool_bytes`
（`engine/mcv_render/src/gpu.rs:136-140`），`batching_stats()` 调用时
现算（`gpu.rs:239-263`，~1 Hz 调用；上传热路径零改动、无锁无失败路径）。
镜像到 `MeshStats`（`game/mcv_logic/src/game.rs:77-80`）并由
render-stats 日志带出（`app.rs` ~1 Hz）：

```
render-stats: draws=… water_draws=… entries=… chunks=… rebuilds=… rebuild_kb=…
              staged_uploads=… staged_kb=… fallback=… resident_kb=… pool_kb=…
```

两本小账：

- **GPU 驻留**（resident_bytes）：Σ批次条目 `vertex_buf + index_buf +
  water_index_buf` 缓冲尺寸。批次成员共享缓冲（≤8 块/批、≤4 MiB/批，
  `gpu.rs:76-77`），按整缓冲尺寸计——含 wgpu 4B 对齐放大，即驱动侧
  真实占用量。
- **CPU 留存**（cpu_pool_bytes）：Σ池内 `PoolMesh` 顶点+索引字节
  （与 `batch_joinable` 的 mesh_bytes 同一公式）。#80 起 CPU 池无 LRU
  （合批重建的数据源，常驻不回收）——按派单要求计入预算面。

### 网格域预算与实测

RD=8 生产网格域 = remesh 环 = sim_dist = **19² = 361 块**（比可见环
17²=289 多一圈支撑网格；`game.rs` remesh 环过滤「可见网格半径 ≤
sim_dist」）。CI 断言跑在合成地形上（双频正弦高度场 + 海面水柱，真
mcv_mesher 双 pass + 生产同款 `build_chunk`/`flush`）——它是记账量级
回归门，不是真机内存保证（真实地形树冠/洞穴更大，真机侧看 render-stats）。

实测（CI `K5-MEAS mesh` 行，run 38114958737 / 7a35424，lavapipe）：

| 项 | 实测 | 预算门 |
|---|---|---|
| GPU 驻留（resident_bytes） | 50 690 400 B ≈ 48.34 MiB（~140 KiB/块） | ≤ 96 MiB（= 2× 实测收紧后） |
| CPU 池（cpu_pool_bytes） | 50 690 400 B ≈ 48.34 MiB | ≤ 96 MiB（同上） |
| draw_entries（批次数） | 51（361 块 → 批均 ~7.1，逼近成员上限 8） | — |
| meshed_chunks | 361 | = 网格域块数 |

**收紧纪律**（派单）已执行：首跑暂定派单估值 40 MiB——run 38114958737
实测 48.34 MiB 如约触发取证红（`mesh_memory.rs:166` panic，K5-MEAS 行
先行落日志），随后按 2× 实测收紧为 96 MiB（`MESH_BUDGET_MIB` 注释登记
实测值与 run 号）。真机 Mali 实测另走 render-stats 取证
（`docs/real-machine-checklist.md` §9）。

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
- **palette 压缩列待做**（§1 原版对照）：落地后本表全面重算。
