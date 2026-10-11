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
4. **跨界写裁剪**：原版 `blockStateWriteRadius=1` 的 CHUNK_FEATURES 语义
   允许特征写邻区块；`generate()` 签名无跨界通道 → 树冠伸出本块的部分
   由邻区块生成时以同一纯函数补齐（树干恒由其落点所属块负责），仅当
   邻块尚未生成时视口边缘暂缺冠——流式加载下自愈，非持久缺块。
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
