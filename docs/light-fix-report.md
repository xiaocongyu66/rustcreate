# 真机光照波取证报告：天光穿透深板岩（light-fix 分支）

日期：2026-10-16 · 波次：真机光照波 · 分支：`light-fix`（自 main）
指控：用户真机现象「从陆地挖到深板岩层仍有光照到；原版只在洞口几格内
渐弱（侧向 BFS 每步 -1）」。

## 0. 取证方法（可复现实验，非纸面推演）

`rustc` 独立 harness（不触 cargo/仓内构建面）：**当前 main 的真实源码**
逐位编译——`engine/mcv_registry` + `game/mcv_core`（桩掉 atlas/tint 的
image/bytemuck 依赖，`OPACITY`/`face_shape_occludes`/`vidx` 全同源）+
`game/mcv_light/src/lib.rs`（仅去掉 `sched` 模块行）——然后以
`update_block` 生产口径（先写体素、`old.0`/`new.0` 原值传入）逐格挖，
采样天光字节。既有套件同 harness 复跑：`tests/light.rs` 22 绿、
`tests/light_fidelity.rs` 12 绿（含 #153 波已转绿的
`fidelity_top_slab_truncates_direct_sky_column`）。

## 1. 悬疑面①（主嫌疑）`update_block` 挖动后的列重算 —— **证伪**

对照实物：`mc-ref/src-26.1` `ChunkSkyLightSources.java:77-148`
（`update`→`updateEdge` 的 damp+面遮蔽二分支）、`SkyLightEngine.java:48-73`
（`checkNode`）、`:81-136`（`removeSourcesBelow/addSourcesAbove`）、
`LevelChunk.java:297-302`（`hasDifferentLightProperties` 门 + `checkBlock`）。

逐子路径对拍（本仓 `game/mcv_light/src/lib.rs`）：

| 子路径 | 实证结果 |
| --- | --- |
| `column_top` 判据（lib.rs:324-337）= `opacity>0 \|\| face_shape_occludes(above,cur)` | 与 `isEdgeOccluded`（ChunkSkyLightSources.java:140-148）二分支逐条等价；切片面：顶半砖 UP 满格闭柱已由 F9（light_fidelity）绿锁 |
| 挖「本列顶部遮光块」增量直写（lib.rs:687-694）| 写入 `(new_top+1)..=old_top` = vanilla `lowestSourceY+1..=旧顶`；双扫描仅差subst 单格，`new_top<old_top ⇔ y==old_top`（挖到顶块），下方有岩层的列两扫描同值、不开分支——**列重算无误判为全透** |
| 竖井/阶梯井/封顶平洞/1×1 穿透洞腔（真机四形态，standalone 实跑） | 全部 26.1 一致：竖井整柱 15（vanilla 开天柱同值）；阶梯井 15,15,13,11,9,7,5,3,1,0（每阶 -2＝每步 -1 的几何必然）；封顶平洞 14,13,… 到 0；封闭洞腔 0；穿透柱 15+曼哈顿扇，扇外 0 |
| 跨块边（2 块 29 格跨界平洞，push/pull 两相 + REMOVE/ADD 逐位） | 衰减序列穿过边界恒 -1/格，`extract_edge/apply_edge` 收敛场与单一大世界 BFS 等价（fidelity F6 同判） |
| 体素存储往返（`VoxelStore` set_flat/get_flat/decode_column 全块 98304 格） | 0 mismatch——列扫描所见与地形写入逐位一致 |
| worker 路（sched.rs）| 快照-计算-回传三段：writeback `stage!=TerrainReady` 弃置（game.rs:3748）+ 编辑期 relight 门（game.rs:557）⇒ 在途产物回写唯一亮偏窗口不存在；save（game.rs:2757-2782）只存 voxels+heightmap，重载全量 re-init，无陈旧光照字节的存在面 |

**结论：`update_block`（lib.rs:632-765）列重算与 26.1 语义等价，「浅挖
一块→源柱误判全透→15 直灌深地」在本仓源码上不成立。**

## 2. 悬疑面② `VoxelAccess::light()` 缺块满亮 fallback —— **证伪（非泄漏面）**

`game.rs:438-451` 缺块/未就位 → 15 的 fallback，全仓 grep 消费面：
`trait 定义`（engine/mcv_game/src/lib.rs:25）之外 **零调用点**
（粒子走 `ParticleRt::light_at`（game.rs:475-488）同 fallback——
Particle.java:184-187 语义，合法；刷怪走 `chunk_light`（game.rs:7340）
精确读字节；网格器自带 `mesher.rs:223-254` 缺格 `(0,0)`）。fallback 不进
网格顶点烘焙，不可能把暗腔照亮。

## 3. 病灶点名（最终）

- **引擎（game/mcv_light/**）：无新病灶。指控场景引擎级不可复现；
  天光≥亮区的唯一写路径 = 源柱直写（vanilla `addSourcesAbove` 等价）+
  逐格 -1 BFS，两者都是 26.1 语义。已知登记项不构成该现象：
  KNOWN-DIVERGENCE（lib.rs:92-96 注）侧向 BFS 面遮蔽仍是 damp 维——
  石/深板岩 damp 15 主导（L4 判据 `getLightBlockInto` 双空形走
  simpleOpacity，LightEngine.java:50-60），对该现象无贡献。
- **疑点移交（越本波红线，未动）**：现象若在真机仍现，最可能在
  mcv_mesher 平滑光照层顶点采样（病灶#2 波 in_progress，另一实装代理域）
  或 26.1 对拍外的渲染面。给定点探针：对深板岩洞壁面取
  `mesher.rs:223 light_at(面空气格)` 与顶点四角 `corner_smooth`
  （mesher.rs:316）逐值对拍烘焙顶点 light nibble；引擎字节本身按 §1
  已证明在 15 步外恒 0。

## 4. 回归锁（本波落盘，`game/mcv_light/tests/light.rs`）

真机三形态进 CI 矩阵（harness 实测 25 绿）：

1. `single_shallow_dig_deepslate_layer_stays_dark` —— 浅挖一块+
   深板岩层无光断言（挖开格=源柱底 15，其下/邻列全黑）；
2. `deepslate_stairwell_fades_per_vanilla` —— 阶梯井 -2/阶衰减序列
   （15,15,13,…,1,0）+ 井底横厅全黑；
3. `covered_tunnel_in_deepslate_dark_beyond_entry` —— 封顶平洞侧向
   渐弱（14,13）+ 洞顶/洞底/远处全黑。

任何后续重构若让「深板岩仍有光照到」复发，本锁先红。

## 5. 取证环境

- /tmp/lightsim：rustc 单文件 harness（同 docs/light-audit.md「取证环境」
  的 /root/tp_la 模式；不污染仓内）。世界夹具含整柱深板岩
  （y −60..8 = DEEPSLATE id 342，blocks_gen.inc.rs:370）。
