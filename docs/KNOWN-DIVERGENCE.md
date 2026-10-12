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
2. **UNDER_FLOOR 带宽取块级常数 3**：原版带宽 = 1 + surfaceDepth（surface
   噪声 0..4 + 0.25 随机抖动，SurfaceSystem.java:165-168）逐列变化；本仓
   恒 `SUBSOIL_DEPTH=3`（terrain.rs）。
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
