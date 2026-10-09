# 方块表生成管道报告（ci/gen-blocks.py）

- 日期：2026-10-09。生成器：`ci/gen-blocks.py`（DEVELOP_ONLY，同 assets/ 素材政策）。
- 输入：官方 26.1 素材树 `assets/minecraft/`（历史输入 = `src-26.1/assets/minecraft/`，
  1170 blockstates / 2392 block models）+ 反编译 `net/minecraft/world/level/block/Blocks.java`。
- 产物：
  - `game/mcv_core/src/blocks_gen.inc.rs` —— `GEN_BLOCKS: [(&str, bool,bool,bool, u8, [u16;6], f32, u8); 1171]`
  - `game/mcv_core/tiles_manifest.json` —— tile 层索引（按贴图名字典序）→ 文件名 + 虚拟路径
  - `cpp/src/blocks_gen.inc` —— C++ mesher kBlocks 同步行
  - `game/mcv_core/tests/vanilla_blocks_gen.inc` —— 官方注册表名字清单（覆盖率测试数据源）
- 贴图**不拷贝**：运行时从 `assets/minecraft/textures/block/` 按 manifest 虚拟路径加载（主控指令，
  不访问 Mojang CDN；沙箱内 Mojang CDN 亦实测整体 404）。
- lib.rs 未改动（等主控合并 BlockId u8→u16 后 `include!` 本表）。
  - **2026-10-09 更新（接线已完成）**：`mcv_core::BLOCKS` 已改为 `include!`
    本表（1171 项；前 14 项回归锁 `mcv_core::tests::first_14_match_legacy_table`）。
    827 张贴图源为 `assets/minecraft/textures/block/`（历史上曾拷入 texturepack/blocks/，现已退役）
    （DEVELOP_ONLY 体系），atlas 运行时按 manifest 层号读盘（层 0..826 =
    真实贴图，层 827 = missing 哨兵，裂纹特殊层移至 828..838，
    `atlas::LAYERS = 838`）。（素材红线，2026-10 任务 #53：缺文件/解码失败
    的层固定显示原版 missingno 品红标记并 log::error，程序化噪声回退已全删。）
    （历史注：曾以 texturepack/blocks/ 作为运行时贴图层，2026-10
    目录统一后退役，现一律从资源根 assets/minecraft/textures/block/ 读取，
    见 assets/DEVELOP_ONLY.md。）

## 规模与分类统计

| 项 | 数 |
|---|---|
| blockstates 总数 | **1170** |
| `model_kind=0` 纯立方（全支持） | **410** |
| `model_kind=1` 非立方（占位） | **760**（cross 68 / multipart 96 / 其他楼梯·板·栅栏·柱等 596） |
| 表内条目（含旧 14 方块） | **1171**（snow_grass 为独立 id，官方无独立 blockstate） |
| 唯一贴图 = 纹理数组层数 | **827**（< 2048 预算，GLES 上限抬到 adapter 值即可容纳） |
| 缺贴图失败 | **0**（本机素材树 1112 张 block PNG 全覆盖所有引用） |
| 全空贴图方块 | 5：air/cave_air/void_air/barrier/light/structure_void 类（语义即无形，tiles 全 0 合理） |
| Blocks.java 名称未解析 | 2（仅引擎自建 item_frame/glow_item_frame 占位；26.1 铜系 24 变体 2026-10-10 已按 WeatheringCopperBlocks.create 解析，见下） |

默认属性回退（缺数据时）：solid=true opaque=true liquid=false light=0 hardness=0.0
（原版 `Properties.destroyTime` 字段默认 0.0F，BlockBehaviour.java:976；2026-10-10
审计修正，此前误用 2.0 兜底）。

## id 兼容（硬约束，已机器校验）

- id 0..13 = air,stone,dirt,grass,sand,water,log,leaves,planks,cobble,bedrock,snow_grass,flower_red,flower_yellow —— 顺序/名字/属性与旧表逐字节一致（含 water=100、bedrock=inf）。
- 官方映射：grass→grass_block、snow_grass→grass_block[snowy=true]（独立 id，注明于表注释）、log→oak_log、planks→oak_planks、cobble→cobblestone、leaves→oak_leaves、flower_red→poppy、flower_yellow→dandelion。
- 新 1157 方块 id 14+ 按官方名字典序（已断言 sorted + 无重名）；旧 14 引用的官方名不重复出现。

## 抽查 15 方块（逐面贴图 [+X,-X,+Y,-Y,+Z,-Z]）

模型 JSON 解析结果 + PIL 像素校验（原版原图）：

| 方块 | kind | 逐面贴图 | 校验 |
|---|---|---|---|
| grass_block(id3 grass) | 0 | side×4 / **up=grass_block_top** / down=dirt | ✅ 官方 grass_block.json 即 top/side/bottom；注：原版顶图为灰度、靠生物群系 colormap 染色（引擎已实现：mcv_core::tint + terrain.wgsl plains 基线） |
| snow_grass(id11) | 0 | **grass_block_snow**×4 / up=grass_block_top / down=dirt | ✅ 官方 grass_block_snow.json（cube_bottom_top: bottom=dirt, side=grass_block_snow, top=grass_block_top） |
| oak_log(id6 log) | 0 | side=oak_log×4，**up/down=oak_log_top（年轮）** | ✅ 顶/侧 md5 不同，像素均值明显偏浅（151,121,73 vs 109,85,50）= 年轮面 |
| poppy(id12 flower_red) | 1(cross) | 六面=poppy | ✅ cross 模板；PIL alpha_min≈34（透明背景正确）；旧表 6 面同贴图保持不变 |
| dandelion(id13) | 1(cross) | 六面=dandelion | ✅ 同上 |
| glass | 0 | 六面=glass | ✅ PIL 74.6% 像素 alpha<255；noOcclusion→opaque=false |
| water(id5) | 0 | 六面=water_still | ✅ 静态水贴图；liquid=true hardness=100 |
| bedrock(id10) | 0 | 六面=bedrock | ✅ strength(-1)→f32::INFINITY |
| crafting_table | 0 | N=front S=side E=side W=front（按官方模型 north=front,west=side 直译） up=crafting_table_top down=oak_planks | ✅ 四侧不同贴图正确 |
| furnace | 0 | up/down=furnace_top，north=furnace_front，其余 furnace_side | ⚠️ 26.1 furnace.json 默认朝向 front 在 north 面；朝向语义（facing 状态旋转）待网格器支持，默认态正确 |
| sandstone_stairs | 1 | up=sandstone_top down=sandstone_bottom 四周=sandstone | ✅ 楼梯给足三面代表贴图（占位渲染） |
| oak_slab | 1 | 六面=oak_planks | ✅ slab 模型=半高 cube_bottom_top，贴图同源 |
| oak_fence | 1 | 六面=oak_planks | ✅ multipart 首部件 fence_post 贴图 |
| glass_pane | 1 | 六面=glass_pane_top | ✅ multipart 首部件 pane_post；opaque=false（noOcclusion+透明像素） |
| torch | 1 | 六面=torch | ✅ 26.1 起 torch 用 template_torch；alpha≈19 正确；light_emit=14（26.1 实测值） |

另抽样属性：glowstone=15 光、sea_lantern=15、lava=liquid+15 光（任务书写 14，反编译实测 15，沿用 NOTES-blocks.md 结论）、ice/leaves/glass opaque=false（noOcclusion 或透明像素）、ladder/torch/sign 类 noCollision→solid=false、candle=3 光。

## 已知限制（均写入生成文件头注释）

1. `tintindex` 生物群系染色已由渲染管线按 tile 层实现（草顶/羊齿/树叶三族，mcv_core::tint 注册表 + terrain.wgsl LUT 乘 plains 基线色）；草侧面 grass_block_side（未配 overlay 双层模型）按原版无 tint 贴图显示。
2. `model_kind=1` 暂按全方块渲染（网格器未实现 model_kind）；任务书建议 cross 顶底留空，但现网格器渲染 6 面且 tile 0 为可见品红调试层，留空会闪品红，故按"做不到就六面同贴图占位"处理。
3. 朝向/半高/多部件几何（楼梯、板、栅栏、furnace facing 等）需要网格器按 model_kind+state 扩展后才能正确显示，本表已保留分类信息。
4. ~~26 个铜栏杆/链/灯笼 weathering 变体与 item_frame 属性走默认回退~~ 已修复
   （2026-10-10）：`WeatheringCopperBlocks.create` 按 8 变体展开（栏杆/链 strength=5.0、
   灯笼 strength=3.5+15 光，全部 noOcclusion）；`buttonProperties()`→0.5+noCollision、
   `pistonProperties()`→1.5、无 `strength()` 的方块按原版默认 destroyTime=0.0
   （air 族/虫蚀石/蜂蜜/史莱姆/绊线等 40+ 块从 2.0 修正）；屏障/光源/空气族/
   结构空位/气泡柱 geom=false（原版不可见，此前顶着层 0 贴图出全盒）。
   id 0-13 未动。
