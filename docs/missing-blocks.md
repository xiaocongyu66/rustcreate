# 方块覆盖审计 — 缺失清单基线

> 由 `ci/check-blocks.py` 自动生成，勿手改。复跑：`python3 ci/check-blocks.py`。

- 基线时间戳：2026-10-09 04:20:55 +0800
- 官方 blockstate（26.1）：1170
- 我方注册：BLOCKS 现表 14 + GEN_BLOCKS 不存在（仅 14 块基线） = 14 条
- 名称匹配（含历史别名映射，如 cobble→cobblestone，命中 8 个别名）：13
- **覆盖率：1.11%**；完全缺失：**1157**

## 按类别缺失统计（按名字关键词粗分，首命中优先）

| 类别 | 缺失数 | 占缺失比例 |
|---|---:|---:|
| 其他 | 333 | 28.8% |
| furniture | 251 | 21.7% |
| stonecutting | 137 | 11.8% |
| metal | 99 | 8.6% |
| crop | 75 | 6.5% |
| stone_variant | 61 | 5.3% |
| log | 39 | 3.4% |
| glass | 35 | 3.0% |
| wool | 34 | 2.9% |
| terracotta | 33 | 2.9% |
| concrete | 32 | 2.8% |
| ore | 18 | 1.6% |
| planks | 10 | 0.9% |

## 完全缺失名单

| 官方名 | 类别 | 是否有贴图引用 | 备注 |
|---|---|---|---|
| `acacia_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `acacia_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `acacia_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `acacia_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `acacia_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `acacia_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `acacia_log` | log | 是 | 官方贴图 2/2 张存在 |
| `acacia_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `acacia_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `acacia_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `acacia_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `acacia_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `acacia_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `acacia_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `acacia_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `acacia_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `acacia_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `acacia_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `activator_rail` | metal | 是 | 官方贴图 2/2 张存在 |
| `allium` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `amethyst_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `amethyst_cluster` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `ancient_debris` | metal | 是 | 官方贴图 2/2 张存在 |
| `andesite` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `andesite_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `andesite_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `andesite_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `anvil` | furniture | 是 | 官方贴图 2/2 张存在 |
| `attached_melon_stem` | crop | 是 | 官方贴图 2/2 张存在 |
| `attached_pumpkin_stem` | crop | 是 | 官方贴图 2/2 张存在 |
| `azalea` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `azalea_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `azure_bluet` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `bamboo` | crop | 是 | 官方贴图 3/3 张存在 |
| `bamboo_block` | crop | 是 | 官方贴图 2/2 张存在 |
| `bamboo_button` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `bamboo_fence` | crop | 是 | 官方贴图 2/2 张存在 |
| `bamboo_fence_gate` | crop | 是 | 官方贴图 2/2 张存在 |
| `bamboo_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `bamboo_mosaic` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_mosaic_slab` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_mosaic_stairs` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_planks` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_pressure_plate` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `bamboo_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `bamboo_slab` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_stairs` | crop | 是 | 官方贴图 1/1 张存在 |
| `bamboo_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `bamboo_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `bamboo_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `barrel` | furniture | 是 | 官方贴图 4/4 张存在 |
| `barrier` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `basalt` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `beacon` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: glass |
| `bee_nest` | furniture | 是 | 官方贴图 5/5 张存在 |
| `beehive` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `beetroots` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `bell` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: stone |
| `big_dripleaf` | 其他 | 是 | 官方贴图 5/5 张存在 |
| `big_dripleaf_stem` | crop | 是 | 官方贴图 1/1 张存在 |
| `birch_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `birch_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `birch_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `birch_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `birch_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `birch_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `birch_log` | log | 是 | 官方贴图 2/2 张存在 |
| `birch_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `birch_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `birch_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `birch_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `birch_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `birch_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `birch_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `birch_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `birch_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `birch_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `birch_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `black_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `black_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `black_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `black_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `black_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `black_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `black_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `black_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `black_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `black_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `black_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `black_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `black_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `black_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `blackstone` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `blackstone_slab` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `blackstone_stairs` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `blackstone_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `blast_furnace` | furniture | 是 | 官方贴图 4/4 张存在 |
| `blue_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `blue_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `blue_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `blue_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `blue_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `blue_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `blue_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `blue_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `blue_ice` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `blue_orchid` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `blue_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `blue_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `blue_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `blue_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `blue_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `blue_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `bone_block` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `bookshelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `brain_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `brain_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `brain_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `brain_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `brewing_stand` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在；本地已有 1 张: bricks |
| `brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在；本地已有 1 张: bricks |
| `brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在；本地已有 1 张: bricks |
| `bricks` | stone_variant | 是 | 官方贴图 1/1 张存在；本地已有 1 张: bricks |
| `brown_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `brown_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `brown_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `brown_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `brown_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `brown_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `brown_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `brown_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `brown_mushroom` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `brown_mushroom_block` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `brown_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `brown_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `brown_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `brown_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `brown_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `brown_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `bubble_column` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `bubble_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `bubble_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `bubble_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `bubble_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `budding_amethyst` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `bush` | crop | 是 | 官方贴图 1/1 张存在 |
| `cactus` | crop | 是 | 官方贴图 3/3 张存在 |
| `cactus_flower` | crop | 是 | 官方贴图 1/1 张存在 |
| `cake` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `calcite` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `calibrated_sculk_sensor` | 其他 | 是 | 官方贴图 7/7 张存在 |
| `campfire` | furniture | 是 | 官方贴图 3/3 张存在 |
| `candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `carrots` | crop | 是 | 官方贴图 4/4 张存在 |
| `cartography_table` | furniture | 是 | 官方贴图 5/5 张存在 |
| `carved_pumpkin` | crop | 是 | 官方贴图 3/3 张存在 |
| `cauldron` | furniture | 是 | 官方贴图 4/4 张存在 |
| `cave_air` | 其他 | 引用缺贴图 | 引用 1 张贴图均不在官方 textures/ 下 |
| `cave_vines` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `cave_vines_plant` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `chain_command_block` | metal | 是 | 官方贴图 4/4 张存在 |
| `cherry_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cherry_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `cherry_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cherry_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cherry_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `cherry_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cherry_log` | log | 是 | 官方贴图 2/2 张存在 |
| `cherry_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `cherry_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cherry_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `cherry_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `cherry_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `cherry_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `cherry_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `cherry_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `cherry_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `cherry_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `cherry_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `chipped_anvil` | furniture | 是 | 官方贴图 2/2 张存在 |
| `chiseled_bookshelf` | furniture | 是 | 官方贴图 4/4 张存在 |
| `chiseled_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `chiseled_deepslate` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `chiseled_nether_bricks` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `chiseled_polished_blackstone` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `chiseled_quartz_block` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `chiseled_red_sandstone` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `chiseled_resin_bricks` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `chiseled_sandstone` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `chiseled_stone_bricks` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `chiseled_tuff` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `chiseled_tuff_bricks` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `chorus_flower` | crop | 是 | 官方贴图 3/3 张存在 |
| `chorus_plant` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `clay` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `closed_eyeblossom` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `coal_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `coal_ore` | ore | 是 | 官方贴图 1/1 张存在；本地已有 1 张: coal_ore |
| `coarse_dirt` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cobbled_deepslate` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `cobbled_deepslate_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `cobbled_deepslate_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `cobbled_deepslate_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `cobblestone_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `cobblestone_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `cobblestone_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `cobweb` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cocoa` | crop | 是 | 官方贴图 3/3 张存在 |
| `command_block` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `comparator` | 其他 | 是 | 官方贴图 5/5 张存在 |
| `composter` | furniture | 是 | 官方贴图 5/5 张存在 |
| `conduit` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `copper_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `copper_block` | metal | 是 | 官方贴图 1/1 张存在 |
| `copper_bulb` | metal | 是 | 官方贴图 4/4 张存在 |
| `copper_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `copper_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `copper_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `copper_golem_statue` | metal | 是 | 官方贴图 1/1 张存在 |
| `copper_grate` | metal | 是 | 官方贴图 1/1 张存在 |
| `copper_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `copper_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `copper_torch` | metal | 是 | 官方贴图 1/1 张存在 |
| `copper_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `copper_wall_torch` | metal | 是 | 官方贴图 1/1 张存在 |
| `cornflower` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cracked_deepslate_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `cracked_deepslate_tiles` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `cracked_nether_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `cracked_polished_blackstone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `cracked_stone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `crafter` | 其他 | 是 | 官方贴图 14/14 张存在 |
| `crafting_table` | furniture | 是 | 官方贴图 4/4 张存在；本地已有 1 张: crafting_table_front |
| `creaking_heart` | 其他 | 是 | 官方贴图 6/6 张存在 |
| `creeper_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `creeper_wall_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `crimson_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `crimson_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `crimson_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `crimson_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `crimson_fungus` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `crimson_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `crimson_hyphae` | log | 是 | 官方贴图 1/1 张存在 |
| `crimson_nylium` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `crimson_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `crimson_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `crimson_roots` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `crimson_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `crimson_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `crimson_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `crimson_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `crimson_stem` | crop | 是 | 官方贴图 2/2 张存在 |
| `crimson_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `crimson_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `crimson_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `crying_obsidian` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `cut_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `cut_copper_slab` | metal | 是 | 官方贴图 1/1 张存在 |
| `cut_copper_stairs` | metal | 是 | 官方贴图 1/1 张存在 |
| `cut_red_sandstone` | stone_variant | 是 | 官方贴图 2/2 张存在 |
| `cut_red_sandstone_slab` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `cut_sandstone` | stone_variant | 是 | 官方贴图 2/2 张存在 |
| `cut_sandstone_slab` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `cyan_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `cyan_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `cyan_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `cyan_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `cyan_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `cyan_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `cyan_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `cyan_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `cyan_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `cyan_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `cyan_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `cyan_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `cyan_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `cyan_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `damaged_anvil` | furniture | 是 | 官方贴图 2/2 张存在 |
| `dark_oak_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `dark_oak_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_log` | log | 是 | 官方贴图 2/2 张存在 |
| `dark_oak_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `dark_oak_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `dark_oak_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `dark_prismarine` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dark_prismarine_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `dark_prismarine_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `daylight_detector` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `dead_brain_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_brain_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_brain_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_brain_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_bubble_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_bubble_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_bubble_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_bubble_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_bush` | crop | 是 | 官方贴图 1/1 张存在 |
| `dead_fire_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_fire_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_fire_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_fire_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_horn_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_horn_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_horn_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_horn_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_tube_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_tube_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_tube_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dead_tube_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `decorated_pot` | furniture | 是 | 官方贴图 1/1 张存在 |
| `deepslate` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `deepslate_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `deepslate_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `deepslate_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `deepslate_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `deepslate_coal_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `deepslate_copper_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `deepslate_diamond_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `deepslate_emerald_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `deepslate_gold_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `deepslate_iron_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `deepslate_lapis_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `deepslate_redstone_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `deepslate_tile_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `deepslate_tile_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `deepslate_tile_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `deepslate_tiles` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `detector_rail` | metal | 是 | 官方贴图 2/2 张存在 |
| `diamond_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `diamond_ore` | ore | 是 | 官方贴图 1/1 张存在；本地已有 1 张: diamond_ore |
| `diorite` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `diorite_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `diorite_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `diorite_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `dirt_path` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `dispenser` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `dragon_egg` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dragon_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `dragon_wall_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `dried_ghast` | 其他 | 是 | 官方贴图 28/28 张存在 |
| `dried_kelp_block` | crop | 是 | 官方贴图 3/3 张存在 |
| `dripstone_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `dropper` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `emerald_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `emerald_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `enchanting_table` | furniture | 是 | 官方贴图 3/3 张存在 |
| `end_gateway` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `end_portal` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `end_portal_frame` | furniture | 是 | 官方贴图 4/4 张存在 |
| `end_rod` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `end_stone` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `end_stone_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `end_stone_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `end_stone_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `end_stone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `ender_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `exposed_chiseled_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_copper_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_copper_bulb` | metal | 是 | 官方贴图 4/4 张存在 |
| `exposed_copper_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_copper_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `exposed_copper_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `exposed_copper_golem_statue` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_copper_grate` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_copper_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `exposed_copper_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `exposed_cut_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_cut_copper_slab` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_cut_copper_stairs` | metal | 是 | 官方贴图 1/1 张存在 |
| `exposed_lightning_rod` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `farmland` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `fern` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `fire` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `fire_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `fire_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `fire_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `fire_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `firefly_bush` | crop | 是 | 官方贴图 2/2 张存在 |
| `fletching_table` | furniture | 是 | 官方贴图 4/4 张存在 |
| `flower_pot` | furniture | 是 | 官方贴图 2/2 张存在；本地已有 1 张: dirt |
| `flowering_azalea` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `flowering_azalea_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `frogspawn` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `frosted_ice` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `furnace` | furniture | 是 | 官方贴图 4/4 张存在；本地已有 1 张: furnace_front_on |
| `gilded_blackstone` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `glow_item_frame` | furniture | 是 | 官方贴图 2/2 张存在 |
| `glow_lichen` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `glowstone` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `gold_block` | metal | 是 | 官方贴图 1/1 张存在 |
| `gold_ore` | ore | 是 | 官方贴图 1/1 张存在；本地已有 1 张: gold_ore |
| `golden_dandelion` | metal | 是 | 官方贴图 1/1 张存在 |
| `granite` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `granite_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `granite_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `granite_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `gravel` | 其他 | 是 | 官方贴图 1/1 张存在；本地已有 1 张: gravel |
| `gray_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `gray_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `gray_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `gray_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `gray_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `gray_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `gray_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `gray_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `gray_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `gray_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `gray_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `gray_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `gray_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `gray_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `green_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `green_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `green_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `green_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `green_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `green_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `green_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `green_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `green_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `green_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `green_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `green_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `green_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `green_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `grindstone` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `hanging_roots` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `hay_block` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `heavy_core` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `heavy_weighted_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `honey_block` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `honeycomb_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `hopper` | metal | 是 | 官方贴图 3/3 张存在 |
| `horn_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `horn_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `horn_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `horn_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `ice` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `infested_chiseled_stone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `infested_cobblestone` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `infested_cracked_stone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `infested_deepslate` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `infested_mossy_stone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `infested_stone` | stone_variant | 是 | 官方贴图 1/1 张存在；本地已有 1 张: stone |
| `infested_stone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `iron_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `iron_block` | metal | 是 | 官方贴图 1/1 张存在 |
| `iron_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `iron_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `iron_ore` | ore | 是 | 官方贴图 1/1 张存在；本地已有 1 张: iron_ore |
| `iron_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `item_frame` | furniture | 是 | 官方贴图 2/2 张存在 |
| `jack_o_lantern` | furniture | 是 | 官方贴图 3/3 张存在 |
| `jigsaw` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `jukebox` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `jungle_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `jungle_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `jungle_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `jungle_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `jungle_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `jungle_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `jungle_log` | log | 是 | 官方贴图 2/2 张存在 |
| `jungle_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `jungle_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `jungle_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `jungle_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `jungle_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `jungle_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `jungle_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `jungle_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `jungle_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `jungle_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `jungle_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `kelp` | crop | 是 | 官方贴图 1/1 张存在 |
| `kelp_plant` | crop | 是 | 官方贴图 1/1 张存在 |
| `ladder` | furniture | 是 | 官方贴图 1/1 张存在 |
| `lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `lapis_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `lapis_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `large_amethyst_bud` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `large_fern` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `lava` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `lava_cauldron` | furniture | 是 | 官方贴图 5/5 张存在 |
| `leaf_litter` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `lectern` | furniture | 是 | 官方贴图 5/5 张存在 |
| `lever` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `light` | 其他 | 是 | 官方贴图 16/16 张存在 |
| `light_blue_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `light_blue_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `light_blue_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `light_blue_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `light_blue_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `light_blue_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `light_blue_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `light_blue_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `light_blue_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `light_blue_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `light_blue_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `light_blue_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `light_blue_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `light_blue_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `light_gray_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `light_gray_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `light_gray_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `light_gray_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `light_gray_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `light_gray_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `light_gray_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `light_gray_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `light_gray_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `light_gray_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `light_gray_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `light_gray_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `light_gray_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `light_gray_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `light_weighted_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `lightning_rod` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `lilac` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `lily_of_the_valley` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `lily_pad` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `lime_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `lime_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `lime_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `lime_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `lime_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `lime_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `lime_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `lime_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `lime_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `lime_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `lime_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `lime_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `lime_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `lime_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `lodestone` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `loom` | furniture | 是 | 官方贴图 4/4 张存在 |
| `magenta_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `magenta_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `magenta_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `magenta_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `magenta_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `magenta_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `magenta_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `magenta_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `magenta_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `magenta_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `magenta_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `magenta_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `magenta_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `magenta_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `magma_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `mangrove_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `mangrove_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `mangrove_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `mangrove_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `mangrove_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `mangrove_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `mangrove_log` | log | 是 | 官方贴图 2/2 张存在 |
| `mangrove_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `mangrove_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `mangrove_propagule` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `mangrove_roots` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `mangrove_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `mangrove_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `mangrove_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mangrove_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mangrove_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `mangrove_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `mangrove_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `mangrove_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `medium_amethyst_bud` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `melon` | crop | 是 | 官方贴图 2/2 张存在 |
| `melon_stem` | crop | 是 | 官方贴图 1/1 张存在 |
| `moss_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `moss_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `mossy_cobblestone` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `mossy_cobblestone_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mossy_cobblestone_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mossy_cobblestone_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mossy_stone_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mossy_stone_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mossy_stone_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mossy_stone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `moving_piston` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `mud` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `mud_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mud_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mud_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `mud_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `muddy_mangrove_roots` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `mushroom_stem` | crop | 是 | 官方贴图 2/2 张存在 |
| `mycelium` | 其他 | 是 | 官方贴图 5/5 张存在；本地已有 2 张: dirt, grass_block_top |
| `nether_brick_fence` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `nether_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `nether_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `nether_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `nether_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `nether_gold_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `nether_portal` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `nether_quartz_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `nether_sprouts` | crop | 是 | 官方贴图 1/1 张存在 |
| `nether_wart` | crop | 是 | 官方贴图 3/3 张存在 |
| `nether_wart_block` | crop | 是 | 官方贴图 1/1 张存在 |
| `netherite_block` | metal | 是 | 官方贴图 1/1 张存在 |
| `netherrack` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `note_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `oak_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `oak_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `oak_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `oak_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `oak_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `oak_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `oak_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `oak_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `oak_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `oak_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `oak_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `oak_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `oak_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `oak_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `oak_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `observer` | 其他 | 是 | 官方贴图 5/5 张存在 |
| `obsidian` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `ochre_froglight` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `open_eyeblossom` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `orange_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `orange_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `orange_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `orange_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `orange_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `orange_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `orange_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `orange_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `orange_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `orange_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `orange_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `orange_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `orange_tulip` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `orange_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `orange_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `oxeye_daisy` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `oxidized_chiseled_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_copper_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_copper_bulb` | metal | 是 | 官方贴图 4/4 张存在 |
| `oxidized_copper_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_copper_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `oxidized_copper_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `oxidized_copper_golem_statue` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_copper_grate` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_copper_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `oxidized_copper_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `oxidized_cut_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_cut_copper_slab` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_cut_copper_stairs` | metal | 是 | 官方贴图 1/1 张存在 |
| `oxidized_lightning_rod` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `packed_ice` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `packed_mud` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `pale_hanging_moss` | stone_variant | 是 | 官方贴图 2/2 张存在 |
| `pale_moss_block` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `pale_moss_carpet` | wool | 是 | 官方贴图 3/3 张存在 |
| `pale_oak_button` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `pale_oak_fence` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_fence_gate` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_leaves` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_log` | log | 是 | 官方贴图 2/2 张存在 |
| `pale_oak_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_pressure_plate` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `pale_oak_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pale_oak_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `pearlescent_froglight` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `peony` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `petrified_oak_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `piglin_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `piglin_wall_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pink_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pink_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pink_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `pink_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `pink_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `pink_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `pink_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `pink_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `pink_petals` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `pink_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `pink_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `pink_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `pink_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `pink_tulip` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `pink_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `pink_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `piston` | 其他 | 是 | 官方贴图 6/6 张存在 |
| `piston_head` | furniture | 是 | 官方贴图 3/3 张存在 |
| `pitcher_crop` | crop | 是 | 官方贴图 9/9 张存在 |
| `pitcher_plant` | crop | 是 | 官方贴图 2/2 张存在 |
| `player_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `player_wall_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `podzol` | 其他 | 是 | 官方贴图 5/5 张存在；本地已有 2 张: dirt, grass_block_top |
| `pointed_dripstone` | 其他 | 是 | 官方贴图 10/10 张存在 |
| `polished_andesite` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_andesite_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_andesite_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_basalt` | stone_variant | 是 | 官方贴图 2/2 张存在 |
| `polished_blackstone` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_button` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_pressure_plate` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_blackstone_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_deepslate` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_deepslate_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_deepslate_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_deepslate_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_diorite` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_diorite_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_diorite_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_granite` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_granite_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_granite_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_tuff` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `polished_tuff_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_tuff_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `polished_tuff_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `potatoes` | crop | 是 | 官方贴图 4/4 张存在 |
| `potted_acacia_sapling` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_allium` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_azalea_bush` | crop | 是 | 官方贴图 5/5 张存在；本地已有 1 张: dirt |
| `potted_azure_bluet` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_bamboo` | crop | 是 | 官方贴图 4/4 张存在；本地已有 1 张: dirt |
| `potted_birch_sapling` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_blue_orchid` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_brown_mushroom` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_cactus` | crop | 是 | 官方贴图 3/3 张存在 |
| `potted_cherry_sapling` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_closed_eyeblossom` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_cornflower` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_crimson_fungus` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_crimson_roots` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_dandelion` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_dark_oak_sapling` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_dead_bush` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_fern` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_flowering_azalea_bush` | crop | 是 | 官方贴图 5/5 张存在；本地已有 1 张: dirt |
| `potted_golden_dandelion` | metal | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_jungle_sapling` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_lily_of_the_valley` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_mangrove_propagule` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_oak_sapling` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_open_eyeblossom` | 其他 | 是 | 官方贴图 4/4 张存在；本地已有 1 张: dirt |
| `potted_orange_tulip` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_oxeye_daisy` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_pale_oak_sapling` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_pink_tulip` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_poppy` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_red_mushroom` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_red_tulip` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_spruce_sapling` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_torchflower` | crop | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_warped_fungus` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_warped_roots` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_white_tulip` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `potted_wither_rose` | 其他 | 是 | 官方贴图 3/3 张存在；本地已有 1 张: dirt |
| `powder_snow` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `powder_snow_cauldron` | furniture | 是 | 官方贴图 5/5 张存在 |
| `powered_rail` | metal | 是 | 官方贴图 2/2 张存在 |
| `prismarine` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `prismarine_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `prismarine_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `prismarine_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `prismarine_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `prismarine_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `prismarine_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `pumpkin` | crop | 是 | 官方贴图 2/2 张存在 |
| `pumpkin_stem` | crop | 是 | 官方贴图 1/1 张存在 |
| `purple_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `purple_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `purple_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `purple_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `purple_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `purple_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `purple_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `purple_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `purple_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `purple_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `purple_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `purple_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `purple_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `purple_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `purpur_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `purpur_pillar` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `purpur_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `purpur_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `quartz_block` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `quartz_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `quartz_pillar` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `quartz_slab` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `quartz_stairs` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `rail` | metal | 是 | 官方贴图 2/2 张存在 |
| `raw_copper_block` | metal | 是 | 官方贴图 1/1 张存在 |
| `raw_gold_block` | metal | 是 | 官方贴图 1/1 张存在 |
| `raw_iron_block` | metal | 是 | 官方贴图 1/1 张存在 |
| `red_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `red_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `red_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `red_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `red_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `red_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `red_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `red_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `red_mushroom` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `red_mushroom_block` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `red_nether_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `red_nether_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `red_nether_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `red_nether_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `red_sand` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `red_sandstone` | stone_variant | 是 | 官方贴图 3/3 张存在 |
| `red_sandstone_slab` | stonecutting | 是 | 官方贴图 3/3 张存在 |
| `red_sandstone_stairs` | stonecutting | 是 | 官方贴图 3/3 张存在 |
| `red_sandstone_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `red_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `red_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `red_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `red_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `red_tulip` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `red_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `red_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `redstone_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `redstone_lamp` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `redstone_ore` | ore | 是 | 官方贴图 1/1 张存在 |
| `redstone_torch` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `redstone_wall_torch` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `redstone_wire` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `reinforced_deepslate` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `repeater` | 其他 | 是 | 官方贴图 6/6 张存在；本地已有 1 张: bedrock |
| `repeating_command_block` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `resin_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `resin_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `resin_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `resin_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `resin_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `resin_clump` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `respawn_anchor` | 其他 | 是 | 官方贴图 8/8 张存在 |
| `rooted_dirt` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `rose_bush` | crop | 是 | 官方贴图 2/2 张存在 |
| `sandstone` | stone_variant | 是 | 官方贴图 3/3 张存在 |
| `sandstone_slab` | stonecutting | 是 | 官方贴图 3/3 张存在 |
| `sandstone_stairs` | stonecutting | 是 | 官方贴图 3/3 张存在 |
| `sandstone_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `scaffolding` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `sculk` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `sculk_catalyst` | 其他 | 是 | 官方贴图 5/5 张存在 |
| `sculk_sensor` | 其他 | 是 | 官方贴图 5/5 张存在 |
| `sculk_shrieker` | 其他 | 是 | 官方贴图 5/5 张存在 |
| `sculk_vein` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `sea_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `sea_pickle` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `seagrass` | crop | 是 | 官方贴图 1/1 张存在 |
| `short_dry_grass` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `short_grass` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `shroomlight` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `skeleton_skull` | furniture | 是 | 官方贴图 1/1 张存在 |
| `skeleton_wall_skull` | furniture | 是 | 官方贴图 1/1 张存在 |
| `slime_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `small_amethyst_bud` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `small_dripleaf` | 其他 | 是 | 官方贴图 5/5 张存在 |
| `smithing_table` | furniture | 是 | 官方贴图 4/4 张存在 |
| `smoker` | 其他 | 是 | 官方贴图 5/5 张存在 |
| `smooth_basalt` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `smooth_quartz` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `smooth_quartz_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `smooth_quartz_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `smooth_red_sandstone` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `smooth_red_sandstone_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `smooth_red_sandstone_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `smooth_sandstone` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `smooth_sandstone_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `smooth_sandstone_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `smooth_stone` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `smooth_stone_slab` | stonecutting | 是 | 官方贴图 2/2 张存在 |
| `sniffer_egg` | 其他 | 是 | 官方贴图 18/18 张存在 |
| `snow` | 其他 | 是 | 官方贴图 2/2 张存在；本地已有 1 张: snow |
| `snow_block` | 其他 | 是 | 官方贴图 1/1 张存在；本地已有 1 张: snow |
| `soul_campfire` | furniture | 是 | 官方贴图 3/3 张存在 |
| `soul_fire` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `soul_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `soul_sand` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `soul_soil` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `soul_torch` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `soul_wall_torch` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `spawner` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `sponge` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `spore_blossom` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `spruce_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `spruce_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `spruce_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `spruce_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `spruce_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `spruce_leaves` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `spruce_log` | log | 是 | 官方贴图 2/2 张存在 |
| `spruce_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `spruce_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `spruce_sapling` | crop | 是 | 官方贴图 1/1 张存在 |
| `spruce_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `spruce_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `spruce_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `spruce_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `spruce_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `spruce_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `spruce_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `spruce_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `sticky_piston` | 其他 | 是 | 官方贴图 6/6 张存在 |
| `stone_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `stone_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `stone_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `stone_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `stone_button` | stone_variant | 是 | 官方贴图 1/1 张存在；本地已有 1 张: stone |
| `stone_pressure_plate` | stone_variant | 是 | 官方贴图 1/1 张存在；本地已有 1 张: stone |
| `stone_slab` | stonecutting | 是 | 官方贴图 1/1 张存在；本地已有 1 张: stone |
| `stone_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在；本地已有 1 张: stone |
| `stonecutter` | stonecutting | 是 | 官方贴图 4/4 张存在 |
| `stripped_acacia_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_acacia_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_bamboo_block` | crop | 是 | 官方贴图 2/2 张存在 |
| `stripped_birch_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_birch_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_cherry_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_cherry_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_crimson_hyphae` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_crimson_stem` | crop | 是 | 官方贴图 2/2 张存在 |
| `stripped_dark_oak_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_dark_oak_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_jungle_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_jungle_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_mangrove_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_mangrove_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_oak_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_oak_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_pale_oak_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_pale_oak_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_spruce_log` | log | 是 | 官方贴图 2/2 张存在 |
| `stripped_spruce_wood` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_warped_hyphae` | log | 是 | 官方贴图 1/1 张存在 |
| `stripped_warped_stem` | crop | 是 | 官方贴图 2/2 张存在 |
| `structure_block` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `structure_void` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `sugar_cane` | crop | 是 | 官方贴图 1/1 张存在 |
| `sunflower` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `suspicious_gravel` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `suspicious_sand` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `sweet_berry_bush` | crop | 是 | 官方贴图 4/4 张存在 |
| `tall_dry_grass` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `tall_grass` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `tall_seagrass` | crop | 是 | 官方贴图 2/2 张存在 |
| `target` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `test_block` | 其他 | 是 | 官方贴图 4/4 张存在 |
| `test_instance_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `tinted_glass` | glass | 是 | 官方贴图 1/1 张存在 |
| `tnt` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `torch` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `torchflower` | crop | 是 | 官方贴图 1/1 张存在 |
| `torchflower_crop` | crop | 是 | 官方贴图 2/2 张存在 |
| `trapped_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `trial_spawner` | 其他 | 是 | 官方贴图 11/11 张存在 |
| `tripwire` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `tripwire_hook` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `tube_coral` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `tube_coral_block` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `tube_coral_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `tube_coral_wall_fan` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `tuff` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `tuff_brick_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `tuff_brick_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `tuff_brick_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `tuff_bricks` | stone_variant | 是 | 官方贴图 1/1 张存在 |
| `tuff_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `tuff_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `tuff_wall` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `turtle_egg` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `twisting_vines` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `twisting_vines_plant` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `vault` | 其他 | 是 | 官方贴图 16/16 张存在 |
| `verdant_froglight` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `vine` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `void_air` | 其他 | 引用缺贴图 | 引用 1 张贴图均不在官方 textures/ 下 |
| `wall_torch` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `warped_button` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `warped_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `warped_fence` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `warped_fence_gate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `warped_fungus` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `warped_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `warped_hyphae` | log | 是 | 官方贴图 1/1 张存在 |
| `warped_nylium` | 其他 | 是 | 官方贴图 3/3 张存在 |
| `warped_planks` | planks | 是 | 官方贴图 1/1 张存在 |
| `warped_pressure_plate` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `warped_roots` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `warped_shelf` | furniture | 是 | 官方贴图 2/2 张存在 |
| `warped_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `warped_slab` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `warped_stairs` | stonecutting | 是 | 官方贴图 1/1 张存在 |
| `warped_stem` | crop | 是 | 官方贴图 2/2 张存在 |
| `warped_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `warped_wall_hanging_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `warped_wall_sign` | furniture | 是 | 官方贴图 1/1 张存在 |
| `warped_wart_block` | crop | 是 | 官方贴图 1/1 张存在 |
| `water_cauldron` | furniture | 是 | 官方贴图 5/5 张存在 |
| `waxed_chiseled_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_copper_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_copper_block` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_copper_bulb` | metal | 是 | 官方贴图 4/4 张存在 |
| `waxed_copper_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_copper_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_copper_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `waxed_copper_golem_statue` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_copper_grate` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_copper_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_copper_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_cut_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_cut_copper_slab` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_cut_copper_stairs` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_chiseled_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_copper_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_copper_bulb` | metal | 是 | 官方贴图 4/4 张存在 |
| `waxed_exposed_copper_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_copper_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_copper_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `waxed_exposed_copper_golem_statue` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_copper_grate` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_copper_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_copper_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_cut_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_cut_copper_slab` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_cut_copper_stairs` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_exposed_lightning_rod` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `waxed_lightning_rod` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `waxed_oxidized_chiseled_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_copper_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_copper_bulb` | metal | 是 | 官方贴图 4/4 张存在 |
| `waxed_oxidized_copper_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_copper_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_copper_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `waxed_oxidized_copper_golem_statue` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_copper_grate` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_copper_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_copper_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_cut_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_cut_copper_slab` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_cut_copper_stairs` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_oxidized_lightning_rod` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `waxed_weathered_chiseled_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_copper_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_copper_bulb` | metal | 是 | 官方贴图 4/4 张存在 |
| `waxed_weathered_copper_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_copper_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_copper_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `waxed_weathered_copper_golem_statue` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_copper_grate` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_copper_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_copper_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_cut_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_cut_copper_slab` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_cut_copper_stairs` | metal | 是 | 官方贴图 1/1 张存在 |
| `waxed_weathered_lightning_rod` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `weathered_chiseled_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_copper_bars` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_copper_bulb` | metal | 是 | 官方贴图 4/4 张存在 |
| `weathered_copper_chain` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_copper_chest` | furniture | 是 | 官方贴图 1/1 张存在 |
| `weathered_copper_door` | furniture | 是 | 官方贴图 2/2 张存在 |
| `weathered_copper_golem_statue` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_copper_grate` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_copper_lantern` | furniture | 是 | 官方贴图 1/1 张存在 |
| `weathered_copper_trapdoor` | furniture | 是 | 官方贴图 1/1 张存在 |
| `weathered_cut_copper` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_cut_copper_slab` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_cut_copper_stairs` | metal | 是 | 官方贴图 1/1 张存在 |
| `weathered_lightning_rod` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `weeping_vines` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `weeping_vines_plant` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `wet_sponge` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `wheat` | crop | 是 | 官方贴图 8/8 张存在 |
| `white_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `white_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `white_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `white_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `white_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `white_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `white_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `white_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `white_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `white_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `white_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `white_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `white_tulip` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `white_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `white_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `wildflowers` | 其他 | 是 | 官方贴图 2/2 张存在 |
| `wither_rose` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `wither_skeleton_skull` | furniture | 是 | 官方贴图 1/1 张存在 |
| `wither_skeleton_wall_skull` | furniture | 是 | 官方贴图 1/1 张存在 |
| `yellow_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `yellow_bed` | furniture | 是 | 官方贴图 1/1 张存在 |
| `yellow_candle` | furniture | 是 | 官方贴图 2/2 张存在 |
| `yellow_candle_cake` | furniture | 是 | 官方贴图 5/5 张存在 |
| `yellow_carpet` | wool | 是 | 官方贴图 1/1 张存在 |
| `yellow_concrete` | concrete | 是 | 官方贴图 1/1 张存在 |
| `yellow_concrete_powder` | concrete | 是 | 官方贴图 1/1 张存在 |
| `yellow_glazed_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `yellow_shulker_box` | 其他 | 是 | 官方贴图 1/1 张存在 |
| `yellow_stained_glass` | glass | 无 | 模型无贴图（占位/空气类） |
| `yellow_stained_glass_pane` | glass | 无 | 模型无贴图（占位/空气类） |
| `yellow_terracotta` | terracotta | 是 | 官方贴图 1/1 张存在 |
| `yellow_wall_banner` | furniture | 是 | 官方贴图 1/1 张存在 |
| `yellow_wool` | wool | 是 | 官方贴图 1/1 张存在 |
| `zombie_head` | furniture | 是 | 官方贴图 1/1 张存在 |
| `zombie_wall_head` | furniture | 是 | 官方贴图 1/1 张存在 |
