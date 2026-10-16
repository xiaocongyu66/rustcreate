# density/含水层通道切原版 fork 链 — python 镜像对拍报告

日期: 2026-10-14 · 分支 noise-vanilla-fork（基点 main=cdcb233a）
镜像: tools/fork-chain-mirror.py · 原始输出: tools/fork-chain-mirror-run.txt
java 真值: java-harness 官方 26.1 jar `--probe-noise` 探针（Main.java:117-131
`getOrCreateNoise` 实物链）；复刻函数借 /root/java-harness/tools/java_dupe.py
（该组复刻与官方 jar 108/108 位型互证，见 java-harness/logs/first-proof-report.md §3）。

## 1. 镜像对象（切换后 Rust 公式，逐行对位）

`game/mcv_worldgen/src/vanilla/noise.rs::NormalNoise::from_vanilla_name` +
`vanilla/xoroshiro.rs` + `vanilla/perlin.rs::ImprovedNoise::from_xoroshiro`：

```text
root  = XoroshiroRandomSource.from_seed(world_seed).fork_positional()
        # xoroshiro.rs root_positional_factory = RandomState.java:36
rng   = root.fromHashOf("minecraft:{name}")          # Noises.java:78-83
first = PerlinNoise(rng, fo, amps)   # forkPositional 消耗 2 nextLong（:88）
second = PerlinNoise(rng, fo, amps)  # 第二次 fork（NormalNoise.java:43-44）
octave_i（非零振幅） = ImprovedNoise(factory.fromHashOf("octave_{fo+i}"))
        # PerlinNoise.java:90-95；零振幅不建实例不消耗随机数
```

## 2. 对拍结果：**360/360 位型全同，max|d|=0**

3 seed（1 / 123 / 20260114）× 10 通道 × 12 探针点（同首证口径
`(y,x,z) = (k·7−32, 17+3k, 25+5k)`），python f64 与 java double 同 IEEE 语义，
逐位比对（`Double.doubleToLongBits` 位型相等）：

| seed | 通道 | 同点同值率 |
| --- | --- | --- |
| 1 | temperature / vegetation / continentalness / erosion / ridge / offset | **各 12/12** |
| 1 | aquifer_barrier / …_floodedness / …_spread / aquifer_lava | **各 12/12** |
| 123 | 同上 10 通道 | **各 12/12** |
| 20260114 | 同上 10 通道 | **各 12/12** |

合计 **360/360（100%）**，max|d|=0（无一处位型差）。任务目标
「12/12×3 通道」达成并超额（3 气候 + 4 含水层 + offset/continentalness/
vegetation 共 10 通道；terrain 的 21 个洞穴/山脊通道同链同派生式，仅
名字/参数不同，由同式覆盖）。

## 3. 通道名与原版对位表（Noises.java createKey 实名，实读为准）

任务书拟名 `minecraft:barrier/floodedness/spread/lava` 与实物**不符**——
四条含水层通道的注册名带 `aquifer_` 前缀；shift 通道注册名是 `offset`
（Noises.java:20 `SHIFT = createKey("offset")`）。实装按实物：

| Rust 挂载点 | 原版注册名 | firstOctave | amplitudes（含尾部零填充） |
| --- | --- | --- | --- |
| climate.offset（shift_x/shift_z **共用一实例**） | minecraft:offset | −3 | [1,1,1,0] |
| climate.continentalness | minecraft:continentalness | −9 | [1,1,2,2,2,1,1,1,1] |
| climate.erosion | minecraft:erosion | −9 | [1,1,0,1,1] |
| climate.ridge | minecraft:ridge | −7 | [1,2,1,0,0,0] |
| climate.temperature | minecraft:temperature | −10 | [1.5,0,1,0,0,0] |
| climate.vegetation | minecraft:vegetation | −8 | [1,1,0,0,0,0] |
| aquifer.barrier | minecraft:aquifer_barrier | −3 | [1] |
| aquifer.floodedness | minecraft:aquifer_fluid_level_floodedness | −7 | [1] |
| aquifer.spread | minecraft:aquifer_fluid_level_spread | −5 | [1] |
| aquifer.lavan | minecraft:aquifer_lava | −1 | [1] |
| terrain.cave_entrance | minecraft:cave_entrance | −7 | [0.4,0.5,1.0] |
| terrain.sp3d_rarity / sp3d_1 / sp3d_2 / sp3d_thickness | minecraft:spaghetti_3d_rarity(−11) / spaghetti_3d_1(−7) / spaghetti_3d_2(−7) / spaghetti_3d_thickness(−8) | | [1] |
| terrain.sp_roughness_mod / sp_roughness | minecraft:spaghetti_roughness_modulator(−8) / spaghetti_roughness(−5) | | [1] |
| terrain.sp2d_mod / sp2d / sp2d_elevation / sp2d_thickness | minecraft:spaghetti_2d_modulator(−11) / spaghetti_2d(−7) / spaghetti_2d_elevation(−8) / spaghetti_2d_thickness(−11) | | [1] |
| terrain.cave_layer / cave_cheese | minecraft:cave_layer(−8) / cave_cheese(−8) | | [1] / [0.5,1,2,1,2,1,0,2,0] |
| terrain.jagged | minecraft:jagged | −16 | 16×[1] |
| terrain.pillar / pillar_rareness / pillar_thickness | minecraft:pillar(−7) / pillar_rareness(−8) / pillar_thickness(−8) | | [1,1] / [1] / [1] |
| terrain.noodle_noise / noodle_thickness / noodle_ridge_a / noodle_ridge_b | minecraft:noodle(−8) / noodle_thickness(−8) / noodle_ridge_a(−7) / noodle_ridge_b(−7) | | [1] |
| terrain.surface_noise（先例，不动） | minecraft:surface | −6 | [1,1,1] |

路由缩放复核（NoiseRouterData.java:319-322）：barrier (x, 0.5y, z)、
floodedness (x, 0.67y, z)、spread (x, 0.7142857142857143y, z)、lava (x, y, z)
——与 aquifer.rs 调用点一致，未动。气候 ShiftedNoise xzScale 0.25 +
ShiftA/ShiftB 采样点见下。

### 3b. 实装中发现并一并修正的两个非种子链对位差

1. **振幅表尾部零填充**：vanilla json 的 temperature/ridge/vegetation 表长
   6（Rust 旧常量截短为 3）。表长进
   `PerlinNoise.lowestFreqValueFactor = 2^(n−1)/(2^n−1)`（PerlinNoise.java:128），
   截短把首倍频权重系统性抬高 ×1.125（32/63→36/63）——expectedDeviation
   只按**非零**倍频跨度计（两级归一各管各的）。已按 json 补零。
2. **shift_x/shift_z 共用同一 offset 实例 + ShiftB 轴对调**：
   NoiseRouterData.java:81/84 两次 `getOrThrow(Noises.SHIFT)` 同一 Holder →
   visitNoise computeIfAbsent 同一 NormalNoise；ShiftA =
   `compute(blockX, 0, blockZ)`、ShiftB = `compute(blockZ, blockX, 0)`
   （DensityFunctions.java:1000-1043）——shift_z 采样点是 (z·0.25, x·0.25, 0)
   而非 (x·0.25, 0, z·0.25)，且不再是两个独立噪声。climate.rs 已改。

## 4. ImprovedNoise 三坑核对（结论：三坑全过，零修改）

1. **lerp3 轴序**：perlin.rs `lerp2(a1,a2,·) = lerp(a2, lerp(a1,·), lerp(a1,·))`
   = Mth.java:544-546（先 x 后 y）、`lerp3 = lerp(a3, plane0, plane1)` =
   Mth.java:548-561、sample_and_lerp 传参序 = ImprovedNoise.java:85-104。
   一致。
2. **long×float 提升**：xoroshiro.rs `next_double =
   ((next_bits(53) as f32) * 1.110_223e-16_f32) as f64` = XoroshiroRandomSource
   .java:92-94（`nextBits(53) * 1.110223E-16F`——**内联 float 字面量**，
   long×float 二元提升到 f32 域舍入再拓宽；f32 quirk 可返回恰 1.0）。一致。
3. **常量精确性**：GRADIENT 16 项 = SimplexNoise.java:7-24 逐项；
   smoothstep = Mth.java:574-576（`x*6.0` 与 `6.0*x` IEEE 可交换，位型同）；
   GOLDEN/SILVER/stafford 常量与 RandomSupport.java 一致（xoroshiro.rs
   金标准向量锁）；SHIFT_UP_EPSILON 1.000_000_011_686_097_4e-7 =
   (double)1.0E-7F。一致。

端到端证据即第 2 节 360/360——任一坑有差，位型不可能全同。

## 5. 位型金标准向量（noise.rs `fork_chain_matches_official_jar_probe_bits`）

```text
seed=1        minecraft:temperature (17,-32,25)  decimal=0.00088038869423281985  bits=0x3f4cd93c5322c100
seed=123      minecraft:ridge       (32,  3,50)  decimal=0.033012555245622938   bits=0x3fa0e7058a4651ec
seed=20260114 minecraft:aquifer_barrier (50,45,80) decimal=-0.34926483143247972  bits=0xbfd65a5ae1294f1b
```

（向量 = python 镜像值，与 java 探针位型 eq=True；生成口径同
xoroshiro.rs 金标准先例。）

## 6. 残余（不在本刀面）

- **BlendedNoise（base_3d）**：26.1 走 legacy 初始化路径
  （PerlinNoise.createLegacyForBlendedNoise，RandomState.java:69-72 注入
  `fromHashOf("minecraft:terrain")`）——与 NormalNoise 的 new 路径完全不同
  的种子语义（zeroOctave 直取 + 262 计数跳倍频），实物化随地形保真后续刀；
  本刀不动，noise.rs 头注已注记。
- **含水层胞心随机**（AQUI-1 #2）与 preliminary surface 口径（#3）维持
  结构复刻登记。
- **端到端 finalDensity 逐位对拍**：需样条/intercept 全展开（首证报告 §6.2），
  本刀只保证通道级位型。

<<<<<<< HEAD
## 7. 黄金漂移清单（本刀只登记不重锚）

**确定红**（字节级黄金，输出必变）：

| 件 | 测试 | 重锚流程预估 |
| --- | --- | --- |
| game/mcv_worldgen/tests/golden.rs | terrain_golden_hash_4_seeds_x_25_chunks、terrain_golden_full_dumps_byte_identical | `GOLDEN_UPDATE=1 cargo test --test golden`（必须 CI Linux x86_64 runner，tests/golden/README 口径）重写 terrain_parity.tsv + terrain/*.bin；cases.rs 输入规格不动 |
| game/mcv_worldgen/tests/ores.rs | ore_v7_byte_golden | 失败输出直接打印六例新 fnv 哈希 → 填入 WANT（#192 surface 波同流程先例，run 38472795386 双 run 复核） |

**统计带，可能漂**（换链后分布重抽，带宽内可能仍绿）：

| 件 | 门 | 重锚流程预估 |
| --- | --- | --- |
| game/mcv_worldgen/tests/quality.rs | plains_share/σ 带/base_3d 谱带 | 跑 CI 取新分布，越带则按原口径复定带（climate 表补零使 σ 口径归真，方向可预期） |
| game/mcv_worldgen/tests/aquifer.rs | 干占比/三态/囊袋并存带 | 同上（四通道换链） |
| game/mcv_worldgen/tests/cave_audit.rs | K-CAVE-AUDIT 量化基线 | 新基线日志入 CI 对照（空洞率/连通体分布重抽） |
| game/mcv_worldgen/tests/lakes.rs | 湖频度带 | 湖位判据依赖地形实顶，重抽后越带再收带 |

**预期绿**（机制/语义锁，非字节锁）：geometry_locks（材质↔估计面自洽）、
surface_rules、deep_columns、deepslate_bedrock（vertical_gradient RNG 链
未动）、trees_cross_chunk（跨界一致性）、tests/terrain.rs（语义哨兵）、
os2s_golden/os2s_parity（vendored 内核未动）、mcv_mesher/tests/golden.rs
（几何规则表，非 worldgen 输入）。
=======
## 7. 黄金漂移清单——CI 实测回填（run 38650650041，head 48e797ed）

预测先行（本节上半），CI 实测随后修正——**terrain golden 预测有误**：它锚
Legacy 后端（CAVE-1 #1 前例口径），本刀只动 vanilla 路径 → 预测确定红、
实测绿。quality.rs 全部统计带亦实测绿（气候 σ 归一后仍在带内）。

**实测红（6 target / 11 测试，`--no-fail-fast` 全量跑完）**：

| 件 | 测试 | 诊断（CI 日志原文） | 重锚流程预估 |
| --- | --- | --- | --- |
| tests/ores.rs | ore_v7_byte_golden | 六例新 fnv 哈希已由断言打印（case0 b688092216ca91e1 / case1 1694df7b329415c1 / case2 6868c030cc34e823 / case3 e2337296a9636e56 / case4 ebe41767c4cf9363 / case6 略） | 填入 WANT（#192 surface 波同流程先例，双 run 复核） |
| tests/aquifer.rs | sub_sea_caves_majority_dry_with_wet_and_lava_pockets | 湿+岩浆占比 0.386 距上界 | 四通道换链后 floodedness/spread 分布重抽 → 复定统计带 |
| tests/aquifer.rs | generated_chunks_have_dry_caves_below_sea | K-AQUIFER-BUF dry_share=0.123 ≤0.30 | 同上 |
| tests/cave_audit.rs | cave_audit_stats | 带 [0,16) 空洞率 0.002 ≤0.005 | 洞穴通道换链 → K-CAVE-AUDIT 新基线（量化对照日志跨提交存档，#192 前例口径） |
| tests/deep_columns.rs | deep_columns_are_never_hollow | 无实心底列 127/5120（>1%） | density 换链后深层基座重抽 → 复定列级判据窗 |
| tests/trees_cross_chunk.rs | boundary_trees_complete_on_both_sides / tree_spots_agree_across_chunk_views / trunk_bases_land_on_real_surface | 「区域无树/无跨界树——测试空转」 | 气候链切换后 biome 窗移动 → 换取样区（cases 内区域表）而非改判据 |
| tests/trees_wiring.rs | forest_window_tree_count_in_band / ocean_and_desert_windows_treeless / tree_bases_above_sea_on_grassy_ground | 树窗/密度带越带 | 同上（复定频度带） |

**实测绿（含预期外）**：tests/golden.rs 双测试（Legacy 后端锚，零重锚）、
quality.rs 全带（plains_share 0.206/谱 p90 0.0138 域内）、lakes、
surface_rules、geometry_locks、deepslate_bedrock、os2s_golden/os2s_parity、
mcv_mesher/tests/golden.rs、mcv_logic streaming、render-headless 双后端。

>>>>>>> origin/noise-vanilla-fork
