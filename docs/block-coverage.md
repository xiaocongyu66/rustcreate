# 全方块覆盖矩阵（审计波 #147）

> 审计时点：2026-10-12，基座 main=abf83707。本文件是「全方块计划」的**只读审计**：
> 零生产代码改动，产出 = 本文档 + 基线哨兵测试（`mcv_mesher/tests/coverage_audit.rs`、
> `mcv_item/tests/block_item_coverage.rs` + 数据件 `vanilla_cross_models.inc`）+ KNOWN-DIVERGENCE 债条目。
> **items 快照 = audit 时点**（ITEMS 152 条；任务板 #144 物品扩表波并行进行中，合并后数字按其落地实测更新）。

## 数据源（三表交叉，全表未动）

| 表 | 位置 | 规模 |
|---|---|---|
| 千块表 | `game/mcv_core/src/blocks_gen.inc.rs`（生成件） | 1171 id（id 0..13 旧表 + 14 起官方字典序） |
| 贴图清单 | `game/mcv_core/tiles_manifest.json` | 827 真实贴图层（0..826）+ 1 missing 哨兵（827）；atlas 另有裂纹层 828..837 |
| 形状模板 | `game/mcv_core/src/shape.rs`（名字派生 + 模板盒） | 16 档（Cube + 15 非立方档） |
| 物品表 | `game/mcv_item/src/lib.rs`（只读快照） | ITEMS 152 条，其中 ItemKind::Block 100 条 → 指向 98 个不同方块 |

原版对照（26.1）：`/root/mc-ref/src-26.1`（blockstates/models/Blocks.java，**严禁旧树**；
对照结论入本文，blockstates JSON 不入库）。

## 总量结论

| 轴 | 结论 |
|---|---|
| 贴图 | **1165/1171 方块六面贴图零缺失**；部分缺失 0；层号越界 0。哨兵层唯一合法用户 = 六个无模型方块：air / barrier / cave_air / light / structure_void / void_air（原版即不可见，网格器 INVISIBLE_GEOM 出零几何）。 |
| 形状 | 非 Cube 档 311（Cross 27 / Torch 4 / Fence 13 / Slab 62 / Stairs 58 / Carpet 18 / Trapdoor 21 / Pane 17 / Wall 26 / FenceGate 12 / Door 21 / Button 14 / PressurePlate 16 / Ladder 1 / SnowLayer 1），Cube 860。 |
| 整盒占位债 | 生成表 kind=1（原版模型非立方）且引擎 shape=Cube = **447 块**（其中 solid=true 255、solid=false 192——后者「看着是整盒、走着穿过」观感最差）。反向（kind=0 但引擎非 Cube）= 0，无反向误分类。 |
| solid 列 | 千块表 solid 与 26.1 `Blocks.java` noCollision 逐项一致（用 ci/gen-blocks.py 同源解析器复验：0 错位）。 |
| 物品 | 可放置物品覆盖 **98/1171 = 8.37%**；物品名与方块名一致 97/100（3 个旧表占位别名：`cobblestone`→`cobble`、`iron_ore`/`diamond_ore`→`stone`，均 id<14）。 |

## 族级矩阵

「kind=1」= 原版 26.1 默认态模型非整立方（生成表字段）；「引擎非 Cube」= shape.rs 名字派生分档
已走出整盒渲染；「十字占位债」= 原版十字视觉但引擎整盒占位（白名单锁定于
`mcv_mesher/tests/coverage_audit.rs` 的 `CROSS_DEBT`，58 名，只许收缩）。

| 族 | 方块数 | 贴图齐 | kind=1（原版非立方） | 引擎非 Cube 档 | 有物品 | 十字占位债 |
|---|---:|---:|---:|---:|---:|---:|
| 石系（含深板岩/凝灰岩/磨制/砖） | 208 | 208 | 114 | 109 | 12 | 0 |
| 金属/矿/资源块 | 148 | 148 | 86 | 35 | 1 | 5 |
| 木构件/石构件（板梯门窗） | 100 | 100 | 100 | 100 | 48 | 0 |
| 染料色变体（床/旗/盒/烛） | 99 | 99 | 99 | 0 | 0 | 0 |
| 羊毛/地毯/混凝土/陶瓦 | 99 | 99 | 18 | 18 | 0 | 0 |
| 木材系（原木/板/菌丝体/架） | 81 | 81 | 21 | 2 | 12 | 0 |
| 功能/容器/红石 | 70 | 70 | 44 | 0 | 2 | 0 |
| 花草/苗 | 63 | 63 | 62 | 27 | 6 | 25 |
| 告示牌/悬挂牌 | 48 | 48 | 48 | 0 | 0 | 0 |
| 珊瑚 | 40 | 40 | 30 | 0 | 0 | 10 |
| 盆栽（flower_pot_cross） | 39 | 39 | 39 | 0 | 0 | 0 |
| 地形/土壤/冰雪 | 27 | 27 | 4 | 1 | 11 | 0 |
| 有机/杂项（蜂巢/海绵/刷怪笼…） | 26 | 26 | 13 | 0 | 5 | 1 |
| 作物/藤蔓/水草 | 25 | 25 | 23 | 0 | 0 | 17 |
| 光源/火 | 20 | 20 | 14 | 2 | 0 | 0 |
| 玻璃（全盒：glass/stained/tinted） | 18 | 18 | 0 | 0 | 0 | 0 |
| 玻璃板/铜栏 | 17 | 17 | 17 | 17 | 0 | 0 |
| 生物头颅 | 15 | 15 | 15 | 0 | 0 | 0 |
| 空气/技术不可见 | 12 | 6 | 11 | 0 | 0 | 0 |
| 树叶 | 11 | 11 | 0 | 0 | 1 | 0 |
| 流体/技术（water/snow_grass/test…） | 5 | 5 | 0 | 0 | 0 | 0 |
| **合计** | **1171** | **1165** | **758** | **311** | **98** | **58** |

读法：**贴图轴已饱和**（各族贴图齐 = 族规模，唯一缺口是 6 个语义不可见块）——后续波的工作量
几乎全部在**形状轴**（758 kind=1 中 447 还在整盒占位）与**物品轴**（8.37%）。

## 抽查对照（26.1 blockstates/models，抽查法）

全量逐值对 1000+ 方块 × 逐 blockstate 变体不在本波；以下为已抽查结论：

- **玻璃族全 35 块**：全盒玻璃（glass / tinted_glass / 16 色 stained_glass）原版模型
  = `cube_all`（视觉整盒）→ 引擎 Cube + solid=true（有碰撞）+ opaque=false ✓；
  17 块玻璃板/铜栏 → Pane 档 + solid=true ✓（`glass_family_spot_check` 锁定）。
- **十字/作物/海草类全量扫（默认态 parent 链）**：26.1 默认放置态渲染为 cross/crop/
  tinted_cross/template_seagrass/cane 类的方块共 **83** 名（数据件
  `mcv_mesher/tests/vanilla_cross_models.inc`）；引擎 Cross 档 27 = 18 花草 + 9 苗，
  其中 25 名与原版对上、4 名为超集近似（见下债表）。**multipart 植物不在该清单**：
  藤蔓/发光苔/幽匿脉/树叶堆/红树根等（blockstate 为 multipart，模型 vine_*/lichen_*）
  归入「作物/藤蔓/水草」族，走 multipart 模板或 cross 近似，均不进 58 白名单。
- **紫水晶族**：amethyst_cluster（Blocks.java:5078-5084）无 `.noCollision()` →
  原版十字视觉**带**簇状碰撞；引擎现整盒碰撞 + 整盒渲染（4 名，见债表）。
- **梯子**：26.1 `LadderBlock` 无 `.noCollision()`（Blocks.java:1145-1146 仅
  forceSolidOff+noOcclusion），碰撞 = getShape 薄板（BlockBehaviour.java:333-334
  `hasCollision ? getShape : empty`）→ 千块表 solid=true 与 26.1 **一致**
  （HANDOFF-SHAPES 中「原版 ladder 无碰撞」按 26.1 修正，模板表 Collision=薄板本就正确）。
- **工作台/熔炉**（kind=0）：逐面贴图直译正确（见 docs/blocks-report.md 抽查表）；
  furnace 朝向语义（facing 旋转）为已知网格器债，不重复登记。

## 已知债量化（KNOWN-DIVERGENCE 同步登记）

| 债 | 规模 | 落点 |
|---|---|---|
| 整盒占位（kind=1 ∧ shape=Cube） | 447（solid 255 / 无碰撞 192） | 后续「逐族补方块」波按族收敛 |
| 原版十字视觉 ∧ 整盒占位 | 58（白名单 `CROSS_DEBT`，单调收缩） | `mcv_mesher/tests/coverage_audit.rs` |
| solid/cross 错位（#138 gen solid 强制债量化：原版十字视觉带碰撞 ∧ 引擎整盒碰撞） | 4：amethyst_cluster + large/medium/small_amethyst_bud（白名单 `SOLID_CROSS_DEBT`） | 同上 + KNOWN-DIVERGENCE |
| Cross 超集近似（引擎 Cross，原版 flowerbed 贴地层） | 2：pink_petals、wildflowers | flowerbed 模板未建，登记不修正 |
| 贴图缺清单落点 | 0 真缺；哨兵用户 6 名单锁死（全哨兵=不可见类） | `all_sentinel_tiles_are_invisible_class` |
| 物品覆盖 | 98/1171（8.37%）+ 3 个旧表占位别名 | `mcv_item/tests/block_item_coverage.rs`；#144 合并后按实测更新 |

## 建议波切分（性价比排序，给「逐族补方块」波当导航）

1. **十字档扩名波**（收益 ~117 块，成本最低）：`shape.rs::CROSS_PLANTS` 名单按
   `VANILLA_CROSS_VISUAL` 扩 58 名（作物 wheat/carrots/potatoes/beetroots/nether_wart/pitcher_crop、
   蘑菇/珊瑚 16、藤蔓/孢子/垂根 14、浆果丛/菌索/其他）+ 珊瑚扇 20（fan 模板，十字的旋转近似）+
   盆栽 39（pot 小盒 + 复用 cross 的花体）。**纯名单/模板扩充，不动网格器 golden**；
   紫水晶 4 名须「cross 视觉 + 保留碰撞盒」特例处理（消 SOLID_CROSS_DEBT）。
2. **染料色变体波**（收益 ~99 块，同模板×16 色一次成型）：蜡烛+烛蛋糕 34（新 Candle
   档）/ 旗帜 32 / 潜影盒 17 / 床 16。床/潜影盒兼有容器交互债（随 engine/mcv_game 后续波）。
3. **告示牌/头颅/贴墙族波**（收益 ~77 块）：告示牌+悬挂牌 48（4 模板：post/wall/hanging/wall_hanging，
   需 facing 状态位）/ 生物头颅 15 / 贴墙火把 4 / 珊瑚墙扇 10。

再往后（无同模板红利，按需排）：箱子族 11、铁轨 4+红石线/中继器等 thin-tail（~145 块
unique 模板）、金属族铜栏杆/链/灯笼 52。物品轴（#144）独立推进，不依赖形状波。

## 基线 bump 协议（后续波必读）

- 哨兵测试位置：`game/mcv_mesher/tests/coverage_audit.rs`（贴图/形状/债白名单）、
  `game/mcv_item/tests/block_item_coverage.rs`（物品快照）。
- 改善落地 → 更新 `SHAPE_BASELINE`（分档计数，各档走向原版应然：非 Cube 增、Cube 减）；
  债白名单（`CROSS_DEBT` 58 / `SOLID_CROSS_DEBT` 4）⊆ 断言自动放行收缩，收缩后建议回写
  名单防回潮；**任何恶化当场红**（新增占位/贴图缺失/形状误分类）。
- 黄金三表（blocks_gen.inc.rs / tiles_manifest.json / shape.rs 模板盒）本波零改动。
