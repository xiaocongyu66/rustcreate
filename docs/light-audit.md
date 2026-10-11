# 光照保真审计（#153 light-audit）

- 审计对象：`game/mcv_light`（双通道 BFS）→ `mcv_logic/game.rs` 接线 →
  `mcv_mesher` 顶点烘焙 → `mcv_render` lightmap。
- 原版真值：仅锚定反编译 Minecraft 26.1 源码 `/root/mc-ref/src-26.1`
  （旧树/笔记只作旁证，不引行号）。本文所有 `X.java:N` 均指该树。
- 对拍套件：`game/mcv_light/tests/light_fidelity.rs`（新增，10 绿 + 1 病灶红
  按 `#[ignore]` 登记）；既有 `tests/light.rs` 21 用例全部保持绿。

## 判决

**①基本保真（引擎语义与 26.1 等价），无推倒重建必要。**

- 用户质疑「闭门造车、注释引 vanilla 不代表真贴原版」——**证伪**：
  本仓 BFS 语义与 26.1 `LightEngine.propagateIncreases/propagateDecreases`
  的 FIFO + stale-skip + 撤销波回播逐条等价（矩阵 M1–M8）；天光源柱
  判据、水的截断、玻璃穿透与现代「三件套」分工全部对上（M4–M6）。
- 真正的偏差不在引擎核心，而在**形状感知遮光**（引擎+网格器共用缺口，
  病灶#1）与**网格器平滑光照层**（病灶#2，原版默认开启、本仓从未移植），
  以及一条 MINOR 边撤销余量（病灶#3，自愈）。三者都越出「mcv_light 单
  文件小修」边界，按审计铁律不修，列入重建波清单。
- 注释引注核对：`mcv_light` 头注引 `ChunkSkyLightSources.java:140-148`
  与源码逐句一致；`mesher.rs` 引的「mesher.cpp:184-224」是本仓早期
  自写 C++（256 高时代），Rust 是其忠实移植——cpp 的上游不是 Mojang
  源，但其语义恰好落在 26.1 `LevelRenderer.getLightCoords`（平采）的
  语义上（M8），**缺的是原版默认的平滑光照层**（M9）。

## 对拍矩阵

| # | 机制 | 26.1 原版行为（file:line） | 本仓实现（file:line） | 判定 |
|---|------|---------------------------|----------------------|------|
| M1 | 传播代价 | `getOpacity = max(1, lightDampening)`（LightEngine.java:77-79）；方向集 = `Direction.values()` 全六向（:25）；`propagateIncrease` 每步 `fromLevel - getOpacity(toState)`（BlockLightEngine.java:54-58、SkyLightEngine.java:148-153），`newToLevel > toLevel` 才写 | `spread_target` = `level - nopacity.max(1)`（mcv_light/src/lib.rs:163-172、196）；六向 `NEIGHBORS`（:119-127）；`< target` 才写（:200-203） | 等价 |
| M2 | 块光源 | `checkNode`：`emission < old` → 清零+`decreaseAllDirections(old)`；否则 `PULL_LIGHT_IN_ENTRY`；`emission > 0` → `increaseLightFromEmission(emission, isEmptyShape)`（BlockLightEngine.java:26-43）；发光值经 `getLightEmission`（BlockBehaviour.java:578-579） | `update_block` 回撤旧光（:611-618）+ 播种 `light_emit(new_block)`（:682-686）；`BLOCKS_EMIT` 镜像 `mcv_core::BLOCKS.light_emit`（:90-106）；冷启 `init` 全格扫发射体（:381-387） | 等价 |
| M3 | 增加波 | `propagateIncreases`：FIFO 双长队，头格 `isIncreaseFromEmission && fromLevel < target` → 抬到 target，`fromLevel == target` 才播（LightEngine.java:155-173） | `propagate_channel`：FIFO+head 游标，stale（`cur == 0 || cur > level`）跳过，按 stored 现值继续（lib.rs:177-206） | 等价（vanilla 的 LeveledPriorityQueue 只是按层排序优化，不动点相同，DynamicGraphMinFixedPoint.java:124-150） |
| M4 | 天光源柱 | 直天 15 只由源柱直写：`addSourcesAbove` 从 `lowestSourceY` 到顶逐格 `setStoredLevel(15)`（SkyLightEngine.java:106-136）；`propagateIncrease` **无向下特例**（:152）——源柱之外垂直每步同样 -1 | `init`/`update_block` 以 `column_top`（`opacity > 0`）逐列直写 15（lib.rs:370-379、:633-640）；BFS 垂直步同样 -1（spread_target 无方向特例；direct-down 豁免仅在 level==15 且 damp==0，等价于源柱延展） | 等价（fidelity F2 实证：竖井衰减率与水平一致） |
| M5 | 源柱截断判据 | `isEdgeOccluded = bottomState.getLightDampening() != 0 || faceShapeOccludes(top↓, bottom↑)`（ChunkSkyLightSources.java:140-148）；水 `propagatesSkylightDown=false` → damp 1（LiquidBlock.java:114-116 + BlockBehaviour.java:305-310）→ 海面即源柱底；玻璃 damp 0 + 空遮蔽形 → 不截断（TransparentBlock.java:34-37） | `column_top`：自顶向下首个 `opacity(id) > 0`（lib.rs:272-283），判据表 `mcv_core::OPACITY`（mcv_core/src/lib.rs:323-331：solidRender?15 : propagatesSkylightDown?0:1 + 水/叶/冰 1、玻璃/花/板 0、tinted_glass 15 例外） | 等价（damp 维度；形状维度缺失 → 病灶#1） |
| M6 | 撤销波 | `propagateDecrease`：`toLevel <= old-1` → 清零续波；`toEmission < toLevel` 才续带旧值，`toEmission > 0` → 按发射值回播（BlockLightEngine.java:91-101）；幸存格 `toLevel >= oldFromLevel` → `increaseOnlyOneDirection(toLevel)` 按现值重播（:103-105）；天光无发射体（SkyLightEngine.java:178-199） | `removal_channel`：`ncur < old` → 清零 + 续 `(nidx, ncur)`；`emit > 0` → 清零后回播发射值进 readd；`ncur >= old` → readd 按现值（lib.rs:219-266）；天光通道 emit ≡ 0（:249-253） | 等价 |
| M7 | 挖/放入口 | `LevelChunk.setBlockState`：`hasDifferentLightProperties`（damp/emission/形状遮蔽任一变）→ `skyLightSources.update` + `lightEngine.checkBlock(pos)`（LevelChunk.java:297-302；谓词 LightEngine.java:41-48） | `update_block`：列顶重建（lib.rs:603-604）+ 直天修（:622-640）+ 撤销波/重播（:642-689）+ 边界遮光下降补报（:692-709）；`old == new` 早退（:588） | 等价（补报位是协议增强，对拍 light.rs 用例 4/5） |
| M8 | 网格器取光（平采） | `LevelRenderer.getLightCoords`：单格采 + `emissiveRendering` 直返满亮 + 自发光下限 `block < emission → withBlock(emission)`（LevelRenderer.java:1438-1452）；流体面 `max(自身格, 上格)`（FluidRenderer.java:430-432；顶 :168/底 :204/侧 :234） | `light_at` 单格平采 + 越界语义（y≥顶 → sky15，y<底 → 0，缺格 0）（mcv_mesher/src/mesher.rs:204-233）；水/流面 `max(自身, 上格)`（:925-935，129-L/L1 已锚定） | 等价（≙ vanilla「Smooth Lighting OFF」语义） |
| M9 | 平滑光照（原版默认） | 顶点四格均值 + 借中心规则（`smoothBlend`，LightCoordsUtil.java:61-89）+ AO 形状权重混合（`smoothWeightedBlend` :91-104；BlockModelLighter.java:163-178） | **未移植**：本仓每面单采（mesher.rs:505/:562）+ 仅 AO 走 corner 权重（:235-260） | **缺失 → 病灶#2（重建波）** |
| M10 | 面基数 | `CardinalLighting.DEFAULT = (down .5, up 1.0, north .8, south .8, west .6, east .6)`（CardinalLighting.java:8；消费 BlockModelLighter.java:194-196） | terrain.wgsl `face_shade` 表 (+X,-X,+Y,-Y,+Z,-Z) = 0.60,0.60,1.00,0.50,0.80,0.80（engine/mcv_render/assets/terrain.wgsl:31-40，X/Z 曾互换已修） | 等价 |
| M11 | lightmap 曲线 | `get_brightness = level/(4-3·level)`（Lightmap.java:89-93；lightmap.fsh:20-22）；合成 = `max(Ambient,NV) + SkyLightColor·sky_b + mix(tint,1,0.9·parabolic)·block_b`，再 boss/darkness/notGamma（lightmap.fsh:35-65）；Ambient #0a0a0a、tint 0xFFD88C | terrain.wgsl `get_brightness`/`parabolic_mix`/`light_curve` 逐项同式（terrain.wgsl:42-75）；tint/ambient 常数同值（:51-53）；昼轨色由 CPU 下发（env_color.rs:108-118）；notGamma/boss/darkness 不做（gamma=0 → 混合系数 0，本仓无该效果） | 等价（裁剪项均有据） |
| M12 | 跨区块边 | BFS 仅 6 邻、无对角跳步（LightEngine.java:25）；跨 section 直写 stored（LayerLightSectionStorage） | `extract_edge`/`apply_edge` 两相（REMOVE→ADD）边快照协议，REMOVE 回报边变化供调用方级联反推（lib.rs:416-544；接线 game/mcv_logic/src/game.rs:400-470，512 步预算 :405）；角柱格 (15,y,15) 同时属 ±X/±Z 边（h=15 槽位），两条边同步都覆盖它 | 等价（fidelity F5 实证：角柱槽位 + 双向证成收敛；余量偏差 → 病灶#3） |

## 红绿清单（tests/light_fidelity.rs）

绿（原版真值断言，每断言引 file:line）：

1. `fidelity_block_light_level_is_emission_minus_manhattan_distance` ——
   火把 14 全六向 `14−曼哈顿距离`，石壳 damp 15 吞光（BlockLightEngine.java:54-58、LightEngine.java:77-79/:25）。
2. `fidelity_sky_bfs_downward_costs_one_per_step_like_horizontal` ——
   平洞→竖井→井底横廊 L 形场：竖直步与水平步同率 -1（SkyLightEngine.java:148-153 无向下特例；源柱豁免仅 :106-136 直写）。**先验疑点 a 的对称性取证**。
3. `fidelity_glass_skylight_feeds_full_bright_column_into_room` ——
   玻璃 damp 0 不截断源柱，天窗下整柱 15（ChunkSkyLightSources.java:140-148 + TransparentBlock.java:34-37 + SkyLightEngine.java:106-136）。
4. `fidelity_remove_one_of_two_lamps_collapses_to_survivor_field` ——
   双灯拆一，全走廊逐格回落幸存源独场（BlockLightEngine.java:91-105）。
5. `fidelity_corner_column_light_crosses_edge_slot_h15` —— 角柱火把经
   ±X 边 h=15 槽位一步进 B（疑点 c：6 邻、无对角跳步，边柱 384×16 够用）。
6. `fidelity_cross_border_tunnel_field_matches_single_world_bfs` ——
   跨界平洞双通道各呈 V 形、跨界差恒 1：两相协议收敛场 = 单一大世界 BFS
   （M12；疑点 b/c 的双向证成）。
7. `fidelity_water_column_and_block_light_decay_one_per_cell` —— 水柱
   垂直 14,13,…（damp 1，LiquidBlock.java:114-116；疑点 a 实锤：无
   「每格 -3」旧语义）+ 方块光穿水同率（LightEngine.java:77-79）+ 气穴。
8. `fidelity_dig_window_into_dark_room_pulls_neighbour_light` ——
   暗室挖窗进光 = `PULL_LIGHT_IN_ENTRY` 语义（LightEngine.java:23；
   BlockLightEngine.java:36 / SkyLightEngine.java:69）。
9. `fidelity_dig_shaft_extends_direct_sky_column_incrementally` ——
   逐格下挖源柱随挖下探、回填即截断（LevelChunk.java:297-302 +
   ChunkSkyLightSources.java:77-96 + SkyLightEngine.java:48-73）。
10. `fidelity_material_axis_water_leaves_tinted_glass` ——
    水/叶 damp 1 vs 遮光玻璃 damp 15 的「透光衰减 vs 整格挡光」材料轴
    （TintedGlassBlock.java:25-27；疑点 f 的非不透明挡光格）。

红（病灶实锤，已 `#[ignore = "light-audit: 病灶#1 待重建波…"]`）：

- `fidelity_top_slab_truncates_direct_sky_column` —— 本机实跑红：
  `顶半砖格应有侧向 14: left 15, right 14`。原版顶半砖
  `useShapeForLightOcclusion=true`（SlabBlock.java:49-51），UP 面遮蔽形
  满格 → 源柱截断（ChunkSkyLightSources.java:140-148 的
  faceShapeOccludes 分支）；悬空底半砖的 DOWN 面同理闭柱。本引擎
  damp(slab)=0 无形状感知 → 整柱 15 穿透。

既有 `tests/light.rs` 21 用例：全绿，无回归。

## 病灶与重建波清单

### 病灶#1（引擎+数据面）：形状感知遮光缺失

- 原版：`useShapeForLightOcclusion` 方块（板 SlabBlock.java:49-51、梯
  StairBlock.java:69-71 等）按 (方块, 状态, 方向) 面遮蔽形参与
  BFS 面遮蔽（LightEngine.java:81-85）与源柱截断
  （ChunkSkyLightSources.java:140-148）。半影效果：顶半砖顶棚/悬空板
  投整柱阴影，底半砖砖格自身保 15。
- 本仓：`mcv_core::OPACITY` 只到 damp 数值维度（板/梯 = 0），
  `column_top`/`spread_target` 无方向感知（mcv_light/src/lib.rs:272-283、
  :163-172；lib.rs:75-78 已登记 KNOWN-DIVERGENCE）。
- 为何不修：需 (方块, 状态, 方向) → 面遮蔽形 的全表数据（网格器形状
  域数据），非 mcv_light 单文件小修；且引擎状态位当前按基础方块掩蔽
  （lib.rs:80-86），改状态感知是行为切换。
- 重建波切分建议：
  1. `mcv_core`：为板/梯生成 `occludes_face(id, state, dir) -> bool`
     查表（板：top 态 UP 满格、bottom 态 DOWN 满格、侧向恒半格不相接
     → 不遮；梯同构；DOUBLE 板恒满）——数据面小波；
  2. `mcv_light`：`column_top` 与 `update_block` 列修接该表（顶棚
     阴影 + 底板砖格 15/下方 14）——红测试 =
     `fidelity_top_slab_truncates_direct_sky_column`（已写好，去 ignore
     即红/转绿）+ 补悬空底半砖用例；
  3. `mcv_mesher`：面遮蔽（非 AO）走同表（半砖侧面透光语义不变）。
- 重锚通道需求：`mcv_mesher` golden `light_gradient` 场景
  （game/mcv_mesher/tests/golden/cases.rs:255）如涉及半砖烘焙需重锚
  `mesh_scenes/light_gradient.tsv`；render headless 像素样本重截。

### 病灶#2（网格器/渲染面）：平滑光照层未移植

- 原版默认 Smooth Lighting ON：顶点四格均值 + 借中心
  （LightCoordsUtil.java:61-89）+ AO 权重混合（BlockModelLighter.java:
  163-178）。本仓只有平采（M8）+ AO 角权（mesher.rs:235-260）——面内
  无光影渐变、阴影边呈硬台阶。
- 为何不修：mesher/env_color 在本波禁改文件面。
- 重建波切分建议：`mcv_mesher` 增设每顶点 light 通道（Cell 按 4 角
  展开，先 REMOVE+ADD 收敛域内取样），键入 `same_key` 时仅比较平采
  值、顶点光独立编码；golden `light_gradient` 场景 + `mesh_random_batch`
  64 块批全量重锚；render 像素样本（terrain 光影渐变）重截。

### 病灶#3（MINOR，自愈）：边撤销余量

- `apply_edge` REMOVE 判据 `s > n + 1`（lib.rs:480-496 注释）：协议拿
  不到 26.1 撤销波的编辑前波前等级，残余 ≤ +2，随下次编辑自愈；贴边
  y=319 直天源柱可净降 1 档后由上方空气柱 readd 掩盖（lib.rs:486-496
  A5 纠偏注）。幂等门（#119 乒乓门）与 512 步预算兜底均在
  （light.rs ocean_pair 用例 + game.rs:405）。
- 处置：保留登记，不切波；若未来把 BorderSeed 升级为「携带编辑前
  边快照」可一次消掉（与病灶#1/2 的重建波同批做协议 v2）。

### 明确不需要重建的（审计结论）

- 双通道 BFS 主循环、源柱播种/截断（damp 维度）、撤销波 + 自发光回播
  + PULL、跨块两相边协议、OPACITY 全表、lightmap 曲线与面基数——
  以上 M1–M8/M10–M12 全部等价，推倒重建只会引入回归。

## 取证环境

- 本地实跑：`/root/tp_la`（rustc 单文件 harness，/root/tp_harness 模式）：
  mcv_core 以 symlinks + atlas/tint 桩编 rlib（注册表/OPACITY 代码逐位
  同源），mcv_light 编 rlib，两套件 `rustc --test` 实跑取证；
  `clippy-driver -D warnings` 口径复查通过；rustfmt 已过。
- 本仓文件面改动：`game/mcv_light/tests/light_fidelity.rs`（新增）+
  本文档；引擎/mesher/render 零改动。
