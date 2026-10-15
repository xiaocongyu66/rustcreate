# KNOWN-DIVERGENCE（有意偏差登记簿）

本文件登记「与原版 26.1 行为有意不一致 / 尚未移植」的项，全部为**平台债或
裁剪债**，禁止无产品决策的私自修正（原版先行铁律）。代码内同 tag 检索
`KNOWN-DIVERGENCE`。本簿随修复波滚动更新。

## HAND-1 竖屏（aspect<1）第一人称手臂出画 = 原版行为，不修正

- 原版：手 pass 竖直 FOV 固定 70°（Camera.java:221-223 `calculateHudFov`
  = 70.0F）、投影 aspect = width/height（Projection.java:69-72
  `setPerspective(fov·π/180, width/height, …)`）——**没有任何 aspect<1
  适配**。竖屏（如 Android 竖屏 aspect≈0.45）时水平视场只剩桌面 16:9 的
  ~25%，renderPlayerArm 的摆位常数（ItemInHandRenderer.java:267-281，
  为桌面横屏设计）会把整条手臂推出画面：本机对原版矩阵链逐点复算，
  aspect 0.45 下臂 8 角 0/8 屏内（NDC x∈[1.51,4.56]）。
- 本仓：同原版（camera.rs `hud_proj` 竖直 70° + aspect=w/h；臂链全抄
  renderPlayerArm）。竖屏看不到手臂**不是 bug，是原版行为**——不发明
  水平 FOV 锁定/锚点平移等原版不存在的修正；若产品要求竖屏可见手持，
  需单独立项定夺（自定决策须走 porting-conventions 的偏差流程）。
- 关联：审计 129-E ③-2/④-C（/root/audit/129-E-hand.md）。

## HAND-2 手 pass 裁剪债（随 129-E 波登记，未移植）

按 26.1 原版机制列出、本仓未实现的项（修复波 133 覆盖矩阵链/bob/投影/
挥臂曲线四项，其余如下）：

1. 换持下放动画 inverseArmHeight（ItemInHandRenderer.java:270,344）与
   mainHandHeight 逐 tick 插值（:660-668 tick）——本仓 hand.rs 臂链按
   h=0 走，切槽瞬移。
2. (viewXRot−xBob)·0.1° / (viewYRot−yBob)·0.1° 视差补偿
   （renderHandsWithItems :358-361）——视角快速转动时手持微摆缺失。
3. bobHurt 受伤甩镜（GameRenderer.java:314-334）与死亡 FOV
   （Camera.java:225-240 modifyFovBasedOnDeathOrFluid）未移植——bob 门
   的 `!dying`（AbstractClientPlayer.java:84-87）与
   `!swimming`（本仓无游泳态）因此按可用状态近似。
4. 手 pass 光照硬编码 block_light=15/sky_light=15（gpu.rs draw_hand）——
   原版传玩家实体 packed light（ItemInHandRenderer.java:366-390 传入），
   洞穴/夜里本仓手臂恒全亮（审计 #9）。
5. 臂第二层 sleeve（AvatarRenderer.java:249-265 含可见性门）未画——
   手 pass 只取 player_index_slices[0][P_R_ARM] 基础层。
6. 使用类姿态 EAT/DRINK（:349-360）、BOW/CROSSBOW/TRIDENT 充能摆位
   （:472-577）、SHIELD BLOCK、BRUSH、单/双手地图（:205-254）未移植——
   进食/弓等走引擎自有观感。
7. 手持物为 GUI 精灵平片 quad + ICON_SIZE=0.45 自定（hand.rs）vs 原版
   builtin/generated 拉伸 3D 模型 + firstperson_righthand display 变换。

## TREES-1 树结构纯函数库（#135 T 波）的四面裁剪（接线刀须补）

`game/mcv_worldgen/src/trees.rs` 移植 26.1 `TreeFeature` + trunk/foliage
placer 族为纯函数 [`place_tree`]（不接 generate()/game.rs，接线=T5 合入后
的后续刀）。纯函数无世界读面，随之裁剪四项：

1. 立地裁剪缺失：原版 `getMaxFreeTreeHeight`（TreeFeature.java:96-112）+
   `validTreePos`（:53-55）逐格读世界查空/查可替换；本仓假定立地开阔，
   仅在「树足迹内」用本次调用自己的原木位置集模拟替换门（叶不盖本树
   原木）。接线方须在调用前自查立柱净空，否则树干可切崖/切结构。
2. 基下 dirt 恒放：原版是条件替换——非 `CANNOT_REPLACE_BELOW_TREE_TRUNK`
   标签才给 DIRT（TreeConfiguration.java:23-25）；本仓无条件放。
3. decorators 未移植：树藤/树干藤/可可/蜂巢/苍藓/落叶层（TreeDecorator
   族）不在 #135 面；jungle 因此对齐 `JUNGLE_TREE_NO_VINE` 注册项
   （TreeFeatures.java:469），非带藤版 :461。
4. 方块状态未建模：叶 distance/persistent（自然消亡机制随之未模拟）与
   原木 axis 一元 id。后者语义无损——这些 placer 的原木恒
   `defaultBlockState()`=axis_y（ForkingTrunkPlacer 的斜枝也是逐 y 竖杆）。

acacia 分支结构已按 ForkingTrunkPlacer.java:34-88 + AcaciaFoliagePlacer
.java:33-54 全文移植（倾斜主杆+反向斜枝+双冠附着点）——**无
acacia_branching_debt**。

## TREES-2 树投影接线（#139 树接线波）的池化模型裁剪

vanilla 后端 pass 2（`vanilla/tree_pass.rs`）把 `place_tree` 接进
`generate()`：5×5 树池世界坐标纯函数 + 群系密度门。密度换算自 26.1
placed feature 的 per-chunk 尝试数（E[count]/10.24，见模块头注），随
池化模型固有六项偏差：

1. **池栅格 vs 独立撒点**：原版每区块 `count` 次独立均匀尝试（可成团、
   可重叠冠），本仓每 5×5 池至多一树（最小间距 ~5 格、期望密度同口径
   = 每区块 E[count]）——密度对了、空间排布规整度不同。
2. **高密度群系池容量截断**：Jungle E=50.1、Dark Forest count=16 超出
   池容量上限 10.24 棵/区块 → 只按 1.0 概率封顶（且 10 群系点表
   （biomes.rs v1）选不出 JUNGLE/DARK_FOREST，两树种当前不可达，表驱动
   已留位——扩群系表即生效，届时刻度差按本条兑现）。
3. **叶下不立树**：原版尝试用 OCEAN_FLOOR heightmap——叶也挡
   motion，邻树冠上的列会在冠顶续放树（森林有「叠生树」）；本仓选址读
   pass-1 体素实顶、拒绝以叶为立地 → 无叠生树。
4. **跨界重放 vs 跨界写**（fix/tree-chunk-clip 改写）：原版 FEATURES 步
   `blockStateWriteRadius=1`（ChunkPyramid.java:30-33）允许特征跨写邻块
   （WorldGenRegion.java:231-237）并经 `ServerChunkCache.blockChanged`
   (:463-468) → `ChunkHolder.blockChanged`（ChunkHolder.java:123-135）标
   邻块脏重网格；本仓 worker 并行无跨界写通道 → 邻块生成时对同一 TreeSpot
   重放补齐伸入部分（`tree_spots` 纯函数，树基经 `column_real_top` 与树干
   所属块逐字节一致；门判据只读本块「pass-1 地形 + 本块已盖章树」，两树冠
   跨界重叠格的物种归属可与原版全局序不同——单格仍确定、无裂缝）。等价性
   锁 `tests/trees_cross_chunk.rs`。历史：#139 初版只在树干落本块时选址，
   本条旧文「由邻区块生成时以同一纯函数补齐」从未成立，跨界树恒缺一侧冠 =
   真机「树少一边树叶」缺陷（2026-10-12 修复）。
5. **树基高度门**：`SEA+1 < base ≤ TREE_MAX_Y(=SEA+52)`（派发既定门，
   legacy rust_terrain.rs TREE_MAX_Y 先例）——vanilla 无此高度上限
   （SnowyPlains 稀疏云杉原版全高度程可长）；本仓高雪原（surface>
   SEA+52）无树、低雪原（≤SEA+52）保留稀疏云杉，观感与原版一致。
6. **树苗/群生后续刀**：placed feature 的 `would_survive(树苗)` 门在本仓
   等价为立地白名单（SUPPORTS_VEGETATION 生成子集）直接判定；原版
   「树苗方块生长成树」「暗橡树 2×2 群生」「巨型丛林木 2×2」的运行时
   语义（BoneMeal/自然衰减/4 苗齐催）不在本波面——苗→树逻辑与 2×2
   群生登记为后续刀。

fancy oak（0.1/0.333）、pine（taiga 1/3）、fallen tree（各处 0.0125）
折回对应主种（#135 六树种之外的 placer 不在本波面）。

## SURF-1 表层规则（#140 地表内核波）的例外群系与参数裁剪

vanilla 后端 pass 1 表层按 26.1 SurfaceRules 语义重写：逐列密度场真实过零面
（`SurfaceSystem.buildSurface` SurfaceSystem.java:106-163 逐列自顶向下扫描、
`ON_FLOOR` = stoneDepthAbove≤1，SurfaceRules.java:680）+ 群系面表（
`SurfaceRuleData.overworldLike`）。10 群系子集与 id 面之下的偏差：

1. **缺失方块 id（千块表登记债）**：原版表层规则表引用的方块中，本仓
   生成面缺——砾石 GRAVEL（海洋/河床 ON_FLOOR 默认
   SurfaceRuleData.java:258-265 gravelOrStoneIfCeiling；砾石补丁另出
   disk feature）、砂岩 SANDSTONE（滩/沙漠 DEEP/VERY_DEEP_UNDER_FLOOR
   :254-255 与洞顶 :72）、各色彩陶与红沙（badlands 带 :202-232 + bandlands）、
   灰化土/粗泥土（老生长针叶林 :158-161、风袭系 :113-156）、菌丝（蘑菇岛
   :164）、泥（红树滩 :123）、雪块/浮冰/细雪/冰（frozen peaks/slopes/grove/
   frozen ocean :96-143,240-252）、方解石（stony peaks :78-79）。以上群系
   多数不在 10 群系点表（biomes.rs v1），可达者（海洋/河流底）以沙近似；
   扩群系表/扩千块表后须按 SurfaceRuleData 逐群系补全，本波只锁「内陆
   草群系顶格 = 草、滩/沙漠 = 沙、水下沉底 ∈ {沙,土}」（tests/surface_rules.rs）。
2. **UNDER_FLOOR 带宽取块级常数 3**〔已销账 #192 主世界地形波〕：实物接线
   `Orchestrator::surface_depth` = getSurfaceDepth 逐列值（噪声·2.75+3+jitter
   ·0.25 截断，SurfaceSystem.java:165-168）；surface 通道（surface.json
   −6 [1,1,1]）与 jitter 均走 RNG-1 同族**原版种子链**
   （`NormalNoise::from_vanilla_name`/根位置随机工厂直传），同 seed 与原版
   同位型（内核求值口径同 perlin.rs 头注）。量化门 = cave_audit.rs
   K-SURFACE-DEPTH（均值带 + 分布塌缩哨兵，histogram 入 CI 日志）。
3. **abovePreliminarySurface 门简化**：原版 = 无 jag 初始密度零点按 16 格
   单元双线性插值 + surfaceDepth − 8（SurfaceRules.java:351-383
   preliminarySurfaceLevel/`HOW_FAR_BELOW…=8` :264）；本仓 = 逐列含 jag
   解析过零面 − 8（无单元插值、surfaceDepth 取 0）。带下回落 STONE 的
   语义一致（深洞底不长草），带缘 ±几格差异属插值近似。
4. **steep() 不实现**：邻列 WORLD_SURFACE_WG 高差 ≥4 判崖
   （SurfaceRules.java:403-427）仅被 FROZEN_PEAKS/SNOWY_SLOPES/
   JAGGED_PEAKS 三门消费（SurfaceRuleData.java:100/108/141）——10 群系
   子集选不出这三群系；内陆草群系悬崖侧面按原版行为保持各列实顶材质
   （每列自己的顶，无独立悬崖换石规则），扩群系后须补 steep 门。
5. **滩/雪位置带覆盖群系带**：估计面 `surface_ours ≤ SEA+1 → 沙`、
   `> SEA+46 → 雪草`（geometry_locks 契约「材质由 offset 估计面分带」，
   tests/geometry_locks.rs:100-121）优先于群系面表；原版全按群系与
   温度门（SNOWY_BEACH/DESERT 沙不依赖海拔，peaks 系雪不依赖固定 46）。
   估计面与真实顶差 ±1-2 格的边缘列按估计面分带，是有意保留的既有契约。
6. **水面置换门（swamp/mangrove puddle、frozen ocean 冰盖）不实现**：
   SurfaceRuleData.java:186-199,240-252 的水/冰置面规则依赖 SWAMP 噪声与
   temperature 门，10 群系子集无 SWAMP/MANGROVE/FROZEN_OCEAN → 不可达，
   扩群系后随 id 债一并补。

## BLOCK-COVERAGE-1 全方块覆盖债量化（#147 审计波，只读登记）

审计波零生产改动，量化「千块表距能用」的缺口（矩阵与波切分见
docs/block-coverage.md；基线哨兵 `mcv_mesher/tests/coverage_audit.rs` +
`mcv_item/tests/block_item_coverage.rs`，白名单只许单调收缩）：

1. **整盒占位 447**：生成表 kind=1（26.1 模型非立方）且引擎 shape=Cube 的方块
   （solid=true 255 / solid=false 192——后者「看着是整盒、走着穿过」）。逐族
   收敛计划：十字档扩名 58 → 染料色变体 99 → 告示牌/头颅/贴墙 77（波切分见文档）。
2. **solid×cross 错位 4**（#138「gen solid 强制」遗留的量化名单）：
   amethyst_cluster + large/medium/small_amethyst_bud——26.1 十字视觉但
   `AmethystClusterBlock` 自带 AABB 碰撞（Blocks.java:5078-5084 无 noCollision）；
   引擎现整盒碰撞 + 整盒渲染。紫水晶族落地时按「cross 视觉 + 保留碰撞盒」收敛
   （白名单 `SOLID_CROSS_DEBT` 锁死防恶化）。
3. **Cross 超集近似 2**：pink_petals / wildflowers 引擎走 Cross，26.1 实为
   flowerbed_*（贴地层分段模型，非 cross）——近似渲染，不修正（同随 flowerbed
   模板落地收敛）。
4. **贴图缺清单落点**：827 真实贴图层对 1171 方块六面**零真缺**（部分缺失 0、
   层号越界 0）；missing 哨兵层唯一用户 = 六个无模型方块（air/barrier/cave_air/
   light/structure_void/void_air，原版即不可见、网格器 INVISIBLE_GEOM）——名单
   由测试锁死，新增全哨兵方块必须同步登记。
5. **物品覆盖 98/1171（8.37%）**：ITEMS 快照 152 条（audit 时点 main=abf83707，
   #144 扩表波并行中，合并后按实测更新）；3 个旧表占位别名
   （cobblestone→cobble、iron_ore/diamond_ore→stone，id<14）白名单登记。
6. **勘误**：HANDOFF-SHAPES「原版 ladder 无碰撞」按 26.1 修正——26.1
   LadderBlock 无 `.noCollision()`（Blocks.java:1145-1146），碰撞 = getShape
   薄板（BlockBehaviour.java:333-334）；千块表 ladder solid=true 与 26.1 一致，
   模板表 Collision 薄板本就正确，非债。

## ITEMRENDER-1 掉落物 billboard 简化（#150 通用物品渲染通路，未移植面）

26.1 `ItemEntityRenderer.java`（:41-110）语义逐项对本仓
`engine/mcv_render/src/item_render.rs` 的 billboard 实现登记偏差。已对齐：
AABB 0.25（Item.java SIZE）→ quad 边长 0.25、浮动 bob = sin(ageInTicks/10
+ bobOffs)·0.1+0.1（:48 原式）、自转角 spin = ageInTicks/20 + bobOffs
（ItemEntity.java:431-432 原式）、光照取所在格 lightmap。裁剪/简化项：

1. **方块物品不做 1/4 微缩立方体**（原版 block item 模型 0.25 缩放六面
   `tiles[]`；本仓统一 billboard 图标取 `tiles[2]` 侧面 tile）——3D 微缩
   模型债；材料物品 sprite 挤出体（extruded sprite，FLAT_ITEM_DEPTH_
   THRESHOLD :20 分支）同样不做。
2. **count>1 不画堆叠**（submitMultipleFromCount :58-110 多模型随机散布，
   seed=实体 id）——单枚 billboard 代表整堆。
3. **无阴影**（EntityRenderer shadowRadius=0.15 / shadowStrength=0.75，
   ItemEntityRenderer.java:27-28）——引擎无实体阴影通路（影深债，与
   mob/player 阴影同域）。
4. **自转轴简化**：原版整体模型绕世界 Y 转；billboard = 竖直卡片绕竖轴
   自转（相机右向随 spin 旋转），侧对时投影收窄成线——无挤出厚度可见。
5. **位置不插值**：快照取当前 PhysBody.pos（无 o-pos 外推），原版
   partialTicks 位置插值缺失；引擎 60 Hz 步长下步进抖动 ≤1/60 格。
6. **ITEM_MIN_HOVER_HEIGHT 并入浮动基线**：原版 = −bb.minY + 0.0625
   （:47，bb 依模型盒），本仓取 0.125+0.0625 常数（DROP_CENTER_Y），
   方块/材料两观感在 ±0.03 块内。
7. **手持 3D 模型债**已在 HAND-2 #7 登记（本波手持图标 sprite 简化版
   维持不动）。

## GROWTH-1 生长波（#146，feat/growth）的运行时裁剪

树苗→树（`mcv_logic/src/growth.rs` + `game.rs::growth_tick`）接 #135
`place_tree` 结构库，随机 tick 结构对齐 26.1 `ServerLevel.tickChunk`
（每块 × 24 section × random_tick_speed=3 尝试/tick，GameRules.java:74）。
随运行时无状态位/无跨界通道的既定模型，登记六项偏差：

1. **树形来源**：原版树苗 `growTree` 走 `TreeGrower` 特征参数（特征表+
   placer 采样），本仓树结构 = `trees::place_tree`（#135 六树种结构库，
   同 seed 同字节契约），不消费原版 sapling 特征参数——树形由 #135 锁定，
   fancy oak/mega 系依旧折回（TREES-2 尾注）。
2. **收获掉落物品缺口**：26.1 `loot_table/blocks/wheat.json` 直译为
   `growth::harvest_drops` 的按名规格（age=7 → 1 麦粒 + 0..3 种子、
   age<7 → 1 种子），物品 id 解析走 `mcv_item::item_by_name`——ITEMS 表
   （#144 在扩）现无 wheat/wheat_seeds → 解析 None 即不掉。#144 合入后
   自动接通；届时**勿在 DROP_TABLE 重复登记小麦行**（双重掉落），本断言
   锁在 `tests/growth_runtime.rs::wheat_items_absent_until_items_wave_lands`。
3. **作物面**：仅小麦（`is_crop`）；胡萝卜/土豆/甜菜根等 CropBlock 族
   随 ITEMS 扩表同构接入（速度公式/亮度门已按 CropBlock.java:79-142
   通用移植）。
4. **ticking 域半径**：原版 random tick 跑在模拟距内全部 ticking 块
   （ServerLevel.java:497-523 遍历 shouldTick 块），本仓取玩家块
   Chebyshev ±2 的 5×5 交互子集——苗/麦的每格节奏不变（尝试数按 section
   体积比例对齐），仅覆盖半径收窄。
5. **尝试流重播种**：原版 `ServerLevel.random` 为世界级长流；本仓按
   `(world_seed ^ GROWTH_DOMAIN, game_tick)` 每 tick 重播种（splitmix 终
   混）——同世界态同 seed 恒同决策（契约面），节奏统计同分布，跨实现
   字节级对拍不适用（同 trees.rs TreesRng 契约先例）。
6. **湿耕地未建模**：farmland MOISTURE 状态无位 → `crop_growth_speed`
   恒取干耕地 1.0 基值，湿耕 3.0 分支不可达（CropBlock.java:104-113）——
   作物节奏偏慢侧（干耕产量下限），状态位波落地后同构接通。

## AQUI-1 含水层（#175，feat/aquifer）的结构复刻裁剪

原版机制 = `NoiseBasedAquifer`（26.1 已并入 Aquifer.java:58-513 inner
class），本仓为**结构复刻**——判据内的四通道噪声与所传 density 的种子链
彼时均为 clean-room 派生；切链刀（noise-vanilla-fork，841b562a）后通道
种子链已切原版 fork 链并逐位同真（下条 #1 销账），端到端 finalDensity
逐位对拍仍待 base_3d（BlendedNoise）实物化与样条组装全展开（首证报告
§6.2 路径）；逐行复刻锁死演进不换一致性。逐项登记：

1. **种子链非原版**〔已销账，切链刀 841b562a + 3812e621〕：四通道噪声
   （barrier/floodedness/spread/lava）原用本仓 NormalNoise 内核 + 自有
   域盐（SALT_*，TREE_DOMAIN 先例），非 Xoroshiro fork 序。销账刀改走
   **原版 fork 链实物接线**：RandomState.java:36 根位置随机工厂 →
   `Noises.instantiate` = `NormalNoise.create(root.fromHashOf(
   "minecraft:aquifer_barrier" 等实名), params)`（Noises.java:23-26
   createKey 实名 + :78-83；RandomState.java:112-114——通道随机源是
   **根工厂 fromHashOf**，非 :38 的 aquiferRandom，后者只喂胞心/位置
   随机 = 本条 #2）。同刀连带：terrain 21 个 density 通道与 climate
   6 实例（noise.rs 旧 v1 域盐派生整体退役）同链切换，并修两处对位差
   （振幅表尾部零填充——表长进 PerlinNoise.lowestFreqValueFactor 归一；
   shift 通道实名 offset 且 shift_x/shift_z 共用一实例 + ShiftB 采样
   x/z 对调，DensityFunctions.java:1000-1043）。**销账证据 = python
   镜像 × 官方 26.1 jar `--probe-noise` 探针 3 seed×10 通道×12 点
   360/360 位型同、max|d|=0**（tools/fork-chain-mirror-report.md +
   tools/fork-chain-mirror-run.txt；noise.rs 金标准测试
   fork_chain_matches_official_jar_probe_bits 锁 3 向量）。随之漂移的
   黄金/统计门重锚清单见对拍报告 §7（CI run 38650650041 实测：6 target
   11 测试红——ores ore_v7 锚定哈希、aquifer/cave_audit/deep_columns/
   trees_cross_chunk/trees_wiring 统计带与窗哨兵；terrain golden 锚
   Legacy 后端零重锚仍绿，quality.rs 全带绿）。
   ImprovedNoise 三坑核对（lerp3 轴序/nextDouble f32 提升/常量精确性）
   随镜像 360/360 端到端通过，零修改。
2. **胞心随机非原版 Xoroshiro 三连 nextInt**：splitmix 链 + Lemire
   无偏归约（同值域 10/9/10、同分布）。
3. **preliminary surface 用密度场解析过零面 crossing**〔已销账，本刀
   find_top_surface 实装〕：旧实现 = 128+128·(offset+jag) 解析式近似，
   原版 = noise_router.preliminary_surface_level =
   **FindTopSurface**（DensityFunctions.java:585-611：专用 surface 探测
   密度树从 upper_bound 树钳定顶向下每 8 格步进找首个 density>0）。
   旧条目「两者同为无洞穴地表高度估计，语义对位」的断言被 java-harness
   对拍**证伪**：腔区 crossing 远低于探测树值 → aquifer under 门
   （surfaceAtCenterIsUnderGlobalFluidLevel）误开 → floodedness factor
   抬高 → 阈值 0.8→−0.3 → 海平面下洞全灌水（#175 残余面；run
   38650650041 K-CAVE-AUDIT [0,16) 带 rust 3921 水/138 空气 vs 官方
   dump 0 水/4751 空气）。销账刀 = terrain.rs 实装探测树全展开
   （offset/factor 列常量 + 双端 y_clamped_gradient 裁剪 +
   quarter_negative + clamp(−64..64)，FindTopSurface 8 格步进语义）
   + upper_bound 树。**销账证据 = python 双复刻对拍：20000 组随机
   (offset, factor) 网格 rust 公式 vs json 树镜像 find_top_surface
   输出 diff=0、密度值 max|d|=0.0**（f64 同构逐位）。
4. **水流更新调度不建模**：shouldScheduleFluidUpdate/
   markPosForPostprocessing（Aquifer.java:160,246-294）无对应存储/
   tick 面——原版靠它把边界水交给随机刻流动，本仓生成期水静态
   （引擎无流体模拟，全局面在 #148 流体波）。
5. **岩浆渲染最小可用面**：LAVA id=516（blocks_gen.inc.rs:537，liquid、
   光 15、六面贴图 402）。mesher `liquid` 规则出贴图整盒（无流动动画、
   无 0.9 水面下沉、非水 pass 半透）——流体保真归 #148 E2 流体波债；
   光照面 lava 光 15 经 blocks_gen 表进 mcv_light 自动生效（tests 锁
   「注岩浆格邻近方块光 >0」）。
6. **开放柱门 = 本列实顶扫描位**（#175 终修，geometry_locks 裁决）：
   原版「修正 preliminary 面以上恒 globalFluid」由区块 skip 线
   （max prelim，Aquifer.java:136-138/:159-161）+ 胞体 pokes/global 段
   （:416-429）两段合成；本仓以本列首个实心位为门（`aquifer::
   open_column`，generate/column_real_top/lake_pass Replay 同型）——
   skip 线的跨列取 max 与本列近面的压力带细节不复刻（锁面等价：洋面
   恒灌 + 胞格噪声限岩层内空腔两结论不变）。液面含 y=SEA 为本仓几何
   契约（geometry_locks 锁死；原版最高水行 = seaLevel−1，
   FluidStatus.at y<fluidLevel）。

## LAKE-1 岩浆湖特征（#175，feat/aquifer 刀C）的接线近似

原版 `LakeFeature` + placed 链（实证见 lake_pass.rs 头注）为本仓首个
非树特征；接线为纯函数重放模型，逐项登记：

1. **种子链非原版**：WorldgenRandom `setLargeFeatureSeed(seed, cx, cz)`
   大特征链 → splitmix 域盐流（LAKE_DOMAIN，TREE_DOMAIN 先例）；形状
   分布参数逐字抄 LakeFeature.java:38-66，由 tests/lakes.rs 频度带 +
   确定性锁替代字节对拍。
2. **判据环读 pass-1 重放态**（密度/含水层/开放柱门，非 WorldGenRegion
   实况）：邻块=噪声填充态与 vanilla CHUNK 阶一致，但本块内**后续湖
   不看见先前湖的写入**（原版后湖看见前湖）；origin 块内湖间重叠为
   低频边缘。
3. **biome placement filter 不逐群系建模**：65 群系 json 中 53 个在
   features[1] 注册 lake_lava_pair（缺席 12 = 下界 5 + 末地 5 +
   the_void + deep_dark）——本仓主世界全区块统一跑，仅 deep_dark 区
   候选多出的低频偏差。
4. **canReplaceBlock 面**：FEATURES_CANNOT_REPLACE 六成员本仓生成面
   仅存在 bedrock → 只排 BEDROCK；LAVA_POOL_STONE_CANNOT_REPLACE 的
   leaves/logs 成员在湖阶（features[1] < 植被）尚不存在 → barrier 门
   等价退化为「非空气非流体即铺石」。
5. **height_range 上沿**：`below_top 0`（max_inclusive=320 出界行）
   取 [0,320) 均匀——单点边界差，扫描/阈值面自动消化。

## NBT-* mcv_nbt 底座（#177，feat/nbt-foundation）的登记偏差

1. **NBT-UTF8-CESU（残余，需用户裁决）**：CESU-8 读写主体已对齐原版
   `writeUTF`/`DataInput.readUTF`——补充平面写 surrogate 对 2×3 字节、
   读侧合法对合成单码位、4 字节标准 UTF-8 同拒（utf8.rs）。唯一残余：
   **孤立 surrogate**（Java `String` 可持、Rust `String` 类型不可表示）
   解码报 BadUtf8——平台类型系统限制，非取舍；若要 byte 级互读恶意/
   损坏样本需改用 WTF-8/`Vec<u16>` 字符串表示，属产品决策，**需用户
   裁决**。（原登记的标准 UTF-8 4 字节读写偏差已销账：ec2f1d8f 前波
   662478d9。）

已销账（原 2–5 条）：配额逐 tag self-size 对齐（8614361f：标量
9/10/12/16/12/16、数组 24+宽×n、String 36+2×UTF-16 char、compound
48+新键 36+键 28+2×char、list 36+4×count，边界用例锁死）；写侧超长串
回退原版 StringFallbackDataOutput 静默空串（c9e61924，NbtIo.java:197-
205）；SNBT hex/二进制整数、数字下划线（首尾非法）、`[B;]`/`[I;]`/
`[L;]` 空数组声明、数组域内非法元素拒绝、无后缀超 i32 报错（ec2f1d8f，
SnbtGrammar/NumberRunParseRule 实物）。

## STRU-* 结构模板运行时（#177）的接线近似

1. **STRU-STATE**：mirror/rotation 只变换 block pos；palette Name 的
   facing 等 blockstate 属性旋转（BlockRotationUtil 全表）未实装。
2. **STRU-BE**：block `nbt`（方块实体）与 `entities` 解析并保留在
   Template（STRU-BE 收窄为「放置面」：本仓暂无方块实体/实体运行时，
   保真解析数据在 `Template::entities`/`has_nbt` 待消费）。

已销账（原 STRU-1/STRU-RNG/STRU-LOOKUP，4eed60da+2432de80）：
`palettes` 多调色板逐变体展开 + `getRandomPalette` 按
`Mth.getSeed(anchor)` 播种 `nextInt(n)` 选取（StructureTemplate.java:
708-721、StructurePlaceSettings.java:138-143）；state 越界回落实物
`SimplePalette.stateFor` 的 `DEFAULT_BLOCK_STATE`=AIR（StructureTemplate.
java:852-870，非任务书所说「palette 首元素」）；integrity 掷骰位对齐
实物——26.1 `RandomSource.create(long)`=`LegacyRandomSource`（java LCG，
RandomSource.java:23-25；Xoroshiro128PlusPlusRandomSource/RandomSharedConstants 不存在，Xoroshiro128PlusPlus 仅余 levelgen 内部组件、不挂 create 路径），每块
全新播种 `Mth.getSeed(世界坐标)`（Mth.java:314-318）掷一次 `nextFloat`
（BlockRotProcessor.java:42-53），`mcv_core::rand::{Lcg, mth_get_seed}`
向量锁死；Name→BlockId OnceLock 建表 O(1)。旧世代 BlockIndices/
width/height/length 格式经全树 grep 实物不存在（DataFixer 面，数据
资产无此形态），按实物不复刻。

## NBT-STACK mcv_nbt 栈安全口径（#177 追加,CI run 38324618498 实测驱动）

1. **读侧零递归**：原版 Java 递归下降靠 JVM 栈扛 depth 512;Rust debug 帧
   大,同深度在测试线程栈上直接 SIGABRT(CI 实测)。本仓二进制读改显式栈
   状态机(栈外语义——深度上限、记账、错误类型——与原版一致)。
2. **SNBT 读限深 512**：parser 仍递归但显式限深(与 NbtAccounter 对齐),
   深嵌套文本报 BadSnbt 不炸栈。
3. **写侧仍递归(NBT-WRITE-DEPTH)**：write_raw/write_snbt 递归深度=树深;
   消费方从本仓 parser 拿到的树天然 ≤512 层安全,程序化手构超深树属调用方
   责任,未设防。

## RNG-1 vertical_gradient 位置随机种子链（#192 深板岩/基岩波）〔已销账〕

overworld.json surface_rule 的两条 `vertical_gradient`（bedrock_floor /
deepslate，SurfaceRules.java:735-781）原版本仓以 hash01 通道域替代
（登记时口径）。#192 RNG-1 销账刀改为**实物种子链逐位接线**：
`vanilla/xoroshiro.rs` 移植 Xoroshiro128++（Xoroshiro128PlusPlus.java:24-36）、
`upgradeSeedTo128bit`/`mixStafford13`/`seedFromHashOf`=MD5-128
（RandomSupport.java:17-42）、`Mth.getSeed(x,y,z)`（Mth.java:314-318，
int 域溢出乘细节保留）、`fromHashOf(name)→forkPositional→at().nextFloat`
链（XoroshiroRandomSource.java:38-41,113-155）——同 seed 同坐标与原版
**同位型**（金标准向量锁在 xoroshiro.rs tests，python hashlib 独立复算
生成）。残余（非本条义务）：位级对拍的另一个前置——defaultBlock 位的
洞形与原版逐格一致——density **通道种子链**已切原版 fork 链（AQUI-1 #1
销账，360/360 位型证据）；洞形逐位一致的残余 = base_3d（BlendedNoise
legacy 初始化路径实物化）与 finalDensity 样条组装全展开，随地形保真
闭环刀统一收敛；tests/deepslate_bedrock.rs 保留统计带门。

## CAVE-1 洞穴雕刻通道的未移植面（#192 对照审计登记）

对照 `noise_settings/overworld.json` noise_router.final_density +
`density_function/overworld/*` 实物逐项清点（terrain.rs density() 引注为
已移植面：sloped_cheese 全链、entrances、spaghetti_2d、
spaghetti_roughness、underground 奶酪、slide/夹逼常数——数值对照
tests/quality.rs + tests/cave_audit.rs K-CAVE-AUDIT 日志）：

1. **pillars**〔已销账 #192 修复刀 494a7b43，CI run 38431595459 三 job
   绿〕：`caves/pillars.json` 实物外层为 **add** 结构
   `mul{ add{ mul{2.0, P}, add{−1.0, mul{−1.0, R}} },
   cube{ add{0.55, mul{0.55, T}} } }`（pillars.json:4-48），即
   `(2P − 1 − R)·cube(0.55+0.55·T)`；首版误作 mul `2P·(−1−R)·T³`——场
   符号翻转、激活域镜像（python 双向对拍：P=0.9/R=0 实物 +0.8·c³ 门开 vs
   旧 −1.8·c³ 门关；P=−0.9/R=0 实物 −2.8·c³ 门关 vs 旧 +1.8·c³ 门开，
   修后逐点=实物，对拍数值在 494a7b43 提交信息存档）。现实现
   terrain.rs:911 `((2.0*p)+(-1.0-r))*(c*c*c)` 逐节点对位 + final_density
   out_of_range 外层 max `range_choice(<0.03 → −10⁶)` 口径不变。销账
   证据链：run 38431595459（tip 865310d5）lint/test/render-headless 全绿、
   --no-fail-fast 下 mcv_worldgen 整包实跑（湖锁 K-LAKES-BUF
   gate_survivors=13/lava_in_band=1116、deepslate 三锁、cave_audit 全
   ok）；golden 双测试**零重锚仍绿**（golden 锁 Legacy 后端，本刀只动
   vanilla 路径，无字节漂移）；前后量化对照在本分支 CI K-CAVE-AUDIT 日志
   （ci.yml --nocapture 步，跨提交对照）。
2. **noodle**〔已销账 #192 洞穴改进波〕：`caves/noodle.json` 实物接线
   （terrain.rs `Orchestrator::noodle`——掩膜 range_choice y∈[−60,321) +
   `−0.075−0.025·thickness + 1.5·max(|ridgeA|,|ridgeB|)`，final_density
   最外层 min）。残余口径：各通道 `minecraft:interpolated` 外壳按模块既定
   取舍跳过（与 final_density 外层 interpolated/blend_density 同一口径，
   只影响 8×8×4 单元管壁平滑度）；noodle rice/thickness 特征接线随特征
   波另立项（非 density 面）。
3. **管状 carver**（`configured_carver/{cave,cave_extra_underground,
   canyon}.json`：probability 0.15/0.07/0.01、y 窗 above_bottom 8..180 /
   8..47 / absolute 10..67、floor_level uniform(−1,−0.4)、
   radius/vertical 乘子表）：**整体未移植**——1.18+ 密度雕刻已有
   （意面/奶酪/entrances），carver 是叠加的样条路径管状洞与峡谷层；
   缺口=缺直管状穿行洞与 canyon 裂谷。子系统规模（CubicSpline 路径 +
   TagKey replaceable + 流体面 + 跨块确定性重放）独立成波，非本任务
   参数级修补；tests/cave_audit.rs 量化现状基线（空洞率随深度分布、
   最大连通体）。
4. **vein_toggle/vein_ridged/vein_gap**（OreVeinifier，noise_router 三
   通道）：矿脉编织——未移植（本仓无矿石生成面，整体随矿脉波立项）。

## MESH-WD-1 无模型技术方块（barrier/light）放置路径的可见性语义（mesh 看门狗波顺手登记）

`blocks_gen.inc.rs:96`（barrier）与 `:549`（light）均为 solid=true（引擎
整盒碰撞）+ 六面 missing 哨兵层 827 → 网格器 `INVISIBLE_GEOM` 恒不渲染
（mesher.rs `INVISIBLE_GEOM` 名单，BLOCK-COVERAGE-1 #4 已锁名单本身）。
放置路径（物品表有定义：mcv_item/src/lib.rs:880/:2934，手持右键即可落格）
的可见性语义逐方块对照 26.1：

1. **barrier**：碰撞语义与原版一致（Blocks.java:2563-2573 全盒碰撞、
   无 `noCollision`）——放置产生「看不见但撞得动」的隐形墙**是原版行为**；
   差异仅在视觉：原版手持 barrier 时显示线框粒子（BarrierBlock 持有门），
   本仓恒不渲染。低风险（创造/调试用途与原版观感差异 ≤ 一层线框）。
2. **light**：碰撞语义与原版**不一致**——原版 `getShape` 持有 light 物品
   之外恒 `Shapes.empty()`（LightBlock.java:62-66，无碰撞、不可拾取命中、
   `replaceable()` 可被放置直接替换，Blocks.java:2575-2582）；本仓
   solid=true 整盒碰撞 + 不可替换——放置产生原版不存在的隐形实体墙，
   且不能再往该格放方块。低风险（light 属技术方块，常规玩法不触达；
   生存无获取路径，仅手持调试）。收敛方向：light 随状态位/碰撞形状表
   波改 no-collision + replaceable，barrier 维持原版碰撞语义。

（净零口径：本条经 mesh 看门狗修复波主线程派发批准登记，登记面 =
放置路径可见性语义 2 项；无旧条目同步销账。）
