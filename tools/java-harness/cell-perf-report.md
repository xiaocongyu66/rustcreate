# cell-perf — NoiseChunk cell 预积分插值外壳（#210 立项审计）

分支 `noise-vanilla-fork`，基线 803d3a3b。目标：terrain 填充从「final_density
逐体素直算 3D 噪声」切到原版 NoiseChunk 架构——被 `minecraft:interpolated`
包裹的子树只在 cell 角点采样（cell 尺寸 (4,8,4)，cell 内 3D 采样 ~64× ↓），
逐位语义以 26.1 官方为锚（mc-ref `NoiseChunk.java` / `Mth.java` /
`NoiseBasedChunkGenerator.java` 引注见下）。

## §1 final_density 树内 INTERPOLATED 包裹节点清单（官方 JSON 展开）

来源 `/root/mc-ref/src-26.1/data/minecraft/worldgen/noise_settings/overworld.json`
`noise_router.final_density` + `density_function/overworld/**`（python 全递归
walker 展开，引用闭合）。final_density = `min(squeeze(0.64·W1), noodle_device)`，
noodle_device = noodle.json 顶层 range_choice。**全树恰 5 个 INTERPOLATED 节点**：

| # | JSON 路径 | 包裹子树（角点求值函数） | rust 对位（terrain.rs） |
|---|---|---|---|
| W1 | `min.argument1.squeeze.mul.argument2` | `I(blend_density(add(0.1171875, ygrad(-64,-40)·(−0.1171875+(−0.078125+ygrad(240,256)·(0.078125+range_choice(sloped<1.5625 → sloped 组合, 否则 洞穴组合)))))))` —— 地形+slide 全合成 | `density()` 内 `v`（slide 双 lerp 后、`squeeze(0.64·)` 之前）的整段表达式树（含 jag/depth/offset/factor 样条、base_3d、entrances、spaghetti_2d、underground、pillars 门） |
| W2 | `min.argument2.input`（noodle 顶层 range_choice 的 input） | `I(range_choice(y∈[−60,321) → noise(noodle,1,1), 否则 −1))` —— 掩膜通道 | `noodle()` 的 `mask = self.noodle_noise.get_value(bx,y_mc,bz)`（y−60 门分支） |
| W3 | `min.argument2.when_out_of_range.argument1` | `I(range_choice(y∈[−60,321) → add(−0.07500000000000001, −0.025·noise(noodle_thickness,1,1)), 否则 0))` —— 厚度基底 | `noodle()` 的 `-0.075... - 0.025 * t` 段 |
| W4 | `min.argument2.when_out_of_range.argument2.argument2.argument1.argument` | `I(range_choice(y∈[−60,321) → noise(noodle_ridge_a,xz/y 2.6666666666666665), 否则 0))`（**abs 在包裹外侧**，对插值后值取绝对值） | `noodle()` 的 `a` |
| W5 | 同上 `…argument2.argument2.argument2.argument` | 同 W4，ridge_b | `noodle()` 的 `b` |

noodle_device 整体（非包裹、逐体素）：
`range_choice(W2插值 ∈ [−1e6,0) → 64, 否则 add(W3插值, 1.5·max(|W4插值|, |W5插值|)))`；
final_density = `min` ( squeeze(0.64·W1插值), noodle_device )。

**树外包裹=false 的证据**：entrances.json/pillars.json/spaghetti_2d 等
（`resolved refs` 全表）内**无任何** INTERPOLATED 节点（walker 结果
`top refs + INTERPS` 计 5，见上）。它们全部位于 W1 内部 → 角点求值。

## §2 官方插值外壳机制（mc-ref 26.1 实文件）

- cell 尺寸：noise_settings.overworld `noise{min_y:-64,height:384,
  size_horizontal:1,size_vertical:2}` → `cellWidth=4·1=4`、
  `cellHeight=4·2=8`、`cellCountXZ=16/4=4`、`cellCountY=384/8=48`
  （NoiseChunk.java:108-111）。角面值阵列 = `[cellCountXZ+1=5][cellCountY+1=49]`
  双 slice（slice0/1，NoiseInterpolator.java:693-717 allocateSlice）。
- 角面求值（fillSlice，NoiseChunk.java:235-252）：X 向每列 cellX 先填
  slice0（firstCellX），`advanceCellX(k)` 填 slice1（firstCellX+k+1）后回退
  cellStartBlockX；`inCellX=inCellY=inCellZ=0` 直接求值原始树。⇒ 角值 = 单点
  raw 树值 = `--density` 探针值（final-density-mirror.py 已证 raw 全等，
  见 §4）。
- 服务（真 chunk 生成环 = NoiseBasedChunkGenerator.java:392-453）：
  cellX 外层→cellZ→cellY **自顶向下**（47→0）→ yInCell **高→低**
  （7→0）→ xInCell 0..3 → zInCell 0..3；每体素
  `updateForY/X/Z` 系数重算（NoiseInterpolator.java:749-766）：

```java
valueXZ00 = lerp(fy, n000, n010);  valueXZ10 = lerp(fy, n100, n110);
valueXZ01 = lerp(fy, n001, n011);  valueXZ11 = lerp(fy, n101, n111);
valueZ0   = lerp(fx, valueXZ00, valueXZ10);  valueZ1 = lerp(fx, valueXZ01, valueXZ11);
value     = lerp(fz, valueZ0, valueZ1);
// Mth.lerp(t, lo, hi) = lo + t*(hi-lo)（Mth.java:540-542）；fx=inCellX/4, fy=inCellY/8, fz=inCellZ/4
```

  即 **Y 最内 → X → Z 最外** 的关联序（填写 fillingCell 的 `Mth.lerp3`
  是 X 最内（Mth.java:544-562）——只服务 `cache_all_in_cell` 通道，
  overworld final_density 不使用，取增量序为准）。
- n000..n111 角命名（selectCellYZ，NoiseChunk.java:732-745）：
  nXYZ 中 x∈{0,1}=slice0/1（X 位相差 cellWidth），y∈{0,1}=cellYIndex 底/顶
  （相差 cellHeight），z 同 x。角世界坐标 = (cellX·4, (cellYIndex−8)·8,
  cellZ·4)（cellNoiseMinY=floorDiv(−64,8)=−8）。
- fill 期间所有 BlockStateFiller（aquifer substance :160、OreVein filler
  ：:161-163）和 final_density 都走 NoiseChunk context → 包裹子树吃**插值值**；
  未包裹通道（vein_toggle/vein_ridged/vein_gap 等）逐体素 raw。
- 任意点（非 fill 环）求值：`Interpolated.compute` context≠NoiseChunk →
  直透 wrapped raw（NoiseChunk.java:757-760）。⇒ ore_pass 任意点/官方
  `--density` 探针 = raw。

## §3 rust 消费面分工（本次改动范围）

| 消费点 | vanilla 对应 | 本次处理 |
|---|---|---|
| terrain.rs `generate()` 填充环（:1205 dval） | NoiseBasedChunkGenerator fill 环 | **接 cell 壳**（`density_cell`） |
| terrain.rs `column_real_top`（tree_pass 实顶，:729 dval） | 树 feature 读已生成块（cell 语义面） | **接 cell 壳**（新入参 shell） |
| lake_pass `Replay::density`（:97，重放判据） | feature 级读已生成块 | **接 cell 壳**（缓冲↔重放构造一致保持） |
| ore_pass `density`（:229） + 任意点 scan（:220/:316） | OreVeinifier filler = vein 噪声逐体素 raw + fill 环块真值 | **保持 raw**（ore_v7 WANT 红线零改动；solid-scan raw 口径差异记 KNOWN-DIVERGENCE 残余） |
| aquifer tests / aquifer field 入参 dval | fill 环内 substance 吃插值 dval | 入参 dval 随填充切 cell（本方天然过渡）；tests 用 raw 直调不触碰 |
| 行为锁 cave_audit/deep_columns/geometry_locks/lakes/trees* | —— | 期望值零改动；红=预期，终报列新值 |

实装文件：新 `vanilla/cell_interp.rs`（一文件一职责）：角面缓存
（HashMap(角列 xz) → [[f64;49];5]）+ 增量序插值核 + shell 合成
（`squeeze(0.64·W1)`、noodle 门/绝对值/1.5/max 合成、`min`）——与 §2
逐行对位；terrain.rs 只抽 `terrain_density()`（= 现 density() 到 slide 为
止的 v，位序不变）+ 接线。cellW=4/cellH=8、gy=(y−(−64))/8∈0..48、
角列 xz ≡ 0 (mod 4)。

## §4 对拍（raw 面 = 官方 --density）

现况（本审计实测，tools/final-density-mirror.py --quick，seed 0x7EA20B1）：
6/6 点 `max|d|=0` bit_eq=True —— rust raw 树与官方逐位一致 ⇒
**角面值可直接与官方对拍。**

计划（step 3）：扩 mirror 出 cell 版公式（§2 增量序 + W1..W5 角面），
8-12 点矩阵（c2_-2 洞区 / 实心区，y −64..96）：角点 = 插值恒等点
（t=0/1 → lerp=lo/hi 精确）直接对官方 --density，max|d|=0 目标；
cell 内点 = python 镜像 vs rust 公式（CI dump_for_java 面）间接收敛。

**实测（tools/final-density-mirror.py 已扩 cell 段：`Orch.terrain_density`
（W1 raw）+ `corner_column/interp_slot/cell_density` = cell_interp.rs
逐行镜像）**：

- raw 基线 6/6 `max|d|=0` bit_eq=True（复证）；
- **cell 角点 12/12 bit_eq=True，`max|d|=0`（0 目标达成）**——角点 =
  插值恒等点，证明 c2_-2 洞带周边的角面选点/槽位布局（W1..W5）/
  外壳合成与官方语义一致；
- in-cell 参考（官方单点探针无外壳 → 直透 raw，差异 = 预期插值平滑）：
  (36,4,-4) cell−raw = 6.94e−4、(44,12,-12) = 1.76e−3，其余（角点）= 0。

| corner (x,y,z) | cell（镜像） | 官方 \|d\| | bit_eq |
|---|---|---|---|
| (40,8,-8) | +0.0036803662079781386 | 0 | True |
| (40,8,-4) | +0.011403389107623989 | 0 | True |
| (40,16,-8) | +0.0097463000857975438 | 0 | True |
| (40,16,-4) | +0.018265978810026576 | 0 | True |
| (44,8,-8) | +0.00046401605828555161 | 0 | True |
| (44,8,-4) | +0.0064803636771429531 | 0 | True |
| (44,16,-8) | +0.0056020547614269722 | 0 | True |
| (44,16,-4) | +0.01231851666098473 | 0 | True |
| (36,0,-4) | +0.013635527693837062 | 0 | True |
| (36,0,0) | +0.021570257624630849 | 0 | True |
| (36,8,-4) | +0.013660416446115153 | 0 | True |
| (36,8,0) | +0.017275322394550756 | 0 | True |

（全量模式：c3_-3/c3_-4 带、窗东半 c5 带、通道 ladder 全展开——补录于
/tmp/cell-mirror-full.log，数字随 CI 刀更新。）

## §5 循环节与性能备注

- 填充环维持现有「列层序 + 扫顶」（scan_top 收口），cell 壳按列懒建
  （4 角列 × 49 层 × 5W）；每体素成本 ~50 flops（12 lerp + shell 合成）
  替代 base_3d(BlendedNoise)+3 洞穴噪声+样条的 ~5-10k flops 直算。
- 角面构建 25 列×49 层×5W（ lazily 4 角列起步）≈1225 次 raw 树求值/块,
  对比现逐体素 ~98k 次求值/块（16×16×384）——3D 噪声调用 ~64× ↓
  （cell 4×8×4=128 体素/角 8 点）。


## §6 首发塌方事故与根因（CI run 38798627796 → 38812058727 探针实锤）

实装首刀（42b7e097）CI dump 全线「塌方」：c2..c5 窗 4096 列逐列
surf_eq=4096 但流体面崩——chunk 内基本全水/全岩浆（老 raw dump 深层
deepslate vs 新 dump lava/water），行为锁新增红 16 只。

**根因（CI 探针 38812058727 实锤）**：`corner()` 的
`for (gy, level) in w.iter_mut()` 把 `Box<[[f64; 49]; 5]>` 的**外维
（slot，5）**误标为 gy——槽位常数 `W_TERRAIN..=W_RIDGE_B` 被当行号写入
`level[0..5]`，row5..48 恒 **0.0**。修复前探针值：

```
P x=32 z=-32 y=-63 cell=-0.007187 raw=+0.039671
P x=32 z=-32 y=-56 cell=-0.309077 raw=+0.048135
P x=32 z=-32 y=-48 cell=+0.000000 raw=+0.045290   ← 精确零（零矩阵插值）
P x=32 z=-32 y=  0 cell=+0.000000 raw=+0.127537
```

cell=+0.000000 恰为零矩阵三线（所有角=0 ⇒ interp=0 ⇒ squeeze=0，
mask=0 ⇒ noodle=0，min(0,0)=0）——dval≤0 ⇒ 全柱进 open_column/aquifer
流体面。修复（ffa57cb0）：`for gy in 0..GY_LEVELS { w[slot][gy] = … }`
双下标直写。

教训入册：`w[slot][gy]` 维序 = 外 slot 内 gy；「iter_mut 首循环变量」
语义上是**外维**，与 gy 无关——凡二维数组必须下标常量落笔，禁
`(misnomer, level) in iter_mut()`。
