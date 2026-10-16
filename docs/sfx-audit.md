# 音效审查矩阵——移动族优先（sfx-walk 波）

批量重写对照：`/root/mc-ref/src-26.1` 实物（音 26.1 SoundEvents/SoundType 均为
原版全称），核对日 2026-10-16，分支 sfx-walk（基线 origin/main=67ea1fc8，
盘点点位 1d809aaf——中途 main 快进含隐形墙波退役，本波 rebase 跟进、
音效面无交集）。

## 0. 基建现状（不重考古，只列消费面）

- `mcv_audio`：`AudioManager`（SPSC 命令环 + Mixer；`play_event`/`play_event_pitched`
  3D 16 格线性衰减、`play_ui` 无衰减、`loop_start/stop` 挖掘循环；静音模式
  no-op——见 `engine/mcv_audio/src/lib.rs`）。
- 方块族：`SoundType` 全表 123 型（`sound_types.inc.rs`，gen-soundtypes.py
  生成），千块表 1171 名归属覆盖测试 `sound_types_coverage.rs`。
- 非方块事件中心表：`engine/mcv_audio/src/events.rs`（WIRED + 完备性测试
  `events_table.rs`，CI fetch sounds.json 后真校验事件名存在）。

移动主循环接线点：`game/mcv_logic/src/game.rs` 行走步进段（:4590-4677）。

## 1. 移动族（P0）diff 矩阵

| # | 音效 | 26.1 实物锚 | 本仓现状 | 判定 |
|---|------|-------------|----------|------|
| M1 | 行走脚步 | `Entity.java:820` 门（moveDist > nextStep ∧ 支撑格非空）、`:1389-1392` playStepSound（vol=SoundType.volume×0.15）；水下行替换 `Player.java:1455-1472` | ✅ :4646-4677（步距门 `step_gate` + 千块表材质） | 已接（偏差 J1 登记） |
| M2 | 水中游泳行进声 PLAYER_SWIM | 独占面：`Entity.java:831-835`（步门 else-分支 → waterSwimSound）+ `:1360-1370` 音量 = min(1,√(vx²·0.2+vy²+vz²·0.2))×0.35 + `:1407-1409` 音高 1.0±0.4；趟水步门内 `Player.java:1468-1471` swim+闷步 | ❌ 全缺（入水播的只有溅落 M4；行进中无声） | **P0 缺口 → 本波实装** |
| M3 | 淌水闷步音（water 中 step 替换压低） | `Player.java:1468-1471` + `Entity.java:1384-1387`（vol×0.05、pitch×0.8） | ❌ | **P0 缺口 → 本波实装**（随 M2 同门） |
| M4 | 入水溅落 splash | `Entity.java:1570-1622` doWaterSplashEffect（自身 0.2 档；<0.25 splash/high_speed + `Player.java:382-390`） | ✅ :4481-4526 | 已接 |
| M5 | 摔落落地音（带伤） | `LivingEntity.java:1768-1778`（dmg>0 → fall 分档 :1772 + playBlockFallSound :1827-1838（vol×0.5、pitch×0.75）） | ✅ :4595-4641 | 已接 |
| M6 | 摔落落地音（无伤） | `LivingEntity.java:1776-1777` else 直返——原版无伤不发声 | ✅ 与原版一致（无伤静默） | 已接（一致） |
| M7 | 跳跃声 | 26.1 无玩家通用 jump 事件（马/跳虫专属 getJumpSound） | — | N/A（原版无此音） |
| M8 | 氛围水下循环/气泡柱/生物群系氛围 | `LocalPlayer.java:176-178` Underwater/BubbleColumn/BiomeAmbient 三 handler | ❌ | P2（ambient 域，另波） |
| M9 | 骑乘声（矿车内外/ghast/nautilus） | `LocalPlayer.java:194-204` | ❌ | N/A（无骑乘实体） |
| M10 | 走过晶石 amethyst chime 叠音 | `Entity.java:1353-1358/:1394-1405` CRYSTAL_SOUND_BLOCKS + 强度态机 | ❌ | P2（叠音面，另波） |
| M11 | 闷步/组合步（雪层/羊毛 INSIDE/COMBINATION_STEP_SOUND） | `Player.java:1473-1484` + `Entity.java:1372-1387` | ❌ | P2（方块族覆盖细节） |

傑出偏差登记（本波不改，防回归面扩大）：

- **J1**：步音音高在本仓叠加了 [0.8,1.2) jitter（:4668，fast_rand 近似）；
  26.1 call 点无随机（`:1391` 音高直传 SoundType.pitch），变化全部来自
  sounds.json 变体自带 pitch/权重（本表已如实保留）→ 叠加后变化幅度大于
  原版。登记不改。
- **J2**：步距累计只加水平分量（:4594）；26.1 攀附（climbable）加全长
  （`Entity.java:818`）。攀爬域挂点（爬梯步音）P2 一并欠账（门还依赖
  支撑格非空，爬梯中支撑为空气，原版行注同死——千层门见 :820）。

## 2. 交互族（P1）

| # | 音效 | 26.1 实物锚 | 本仓现状 | 判定 |
|---|------|-------------|----------|------|
| I1 | 进食咀嚼/完食 | `Consumable.java:104-117`（shouldEmit 门 :119-124）+ `ItemStack.java:1094-1098` | ✅ :6199（咀嚼拍）/ :6220（完食拍） | 已接 |
| I2 | 完食打嗝 PLAYER_BURP | `FoodProperties.java:39-48`（完成音后播，vol 0.5、pitch 0.9-1.0 均匀）| ❌ | **P1 缺口 → 本波实装** |
| I3 | 饮用（喝奶桶/药水） | Consumable GENERIC_DRINK | ❌ | N/A（本仓无饮物品类） |
| I4 | UI 按钮点击 | `AbstractWidget.java:106-118/:165-167` | ✅ app.rs:1246/:1265/:1572 | 已接 |
| I5 | 掉落物拾取 | `ClientPacketListener.java:1002-1013` | ✅ :4400 | 已接 |
| I6 | 弓放箭 | `BowItem.java:46-55` | ✅ :4966 | 已接 |
| I7 | 耐久破碎 | `LivingEntity.java:1412-1420`（vol 0.8、pitch 0.8+0.4r） | ✅ :5687 / :6439 | 已接 |
| I8 | 玩家受伤 | `LivingEntity.java:2019` | ✅ :1979 | 已接 |
| I9 | 装备穿脱 equipped 音 | Equippable component / collectEquipmentChanged | ❌ | N/A（本仓无盔甲槽装备系统） |

## 3. 本波实装清单

| 项 | 文件:行 | 26.1 引注 |
|----|---------|-----------|
| P0 游泳行进声 + 淌水闷步（M2+M3） | `events.rs` PLAYER_SWIM 常量；`game.rs` 步门重排 | Entity.java:820/831-835/1360-1370/1384-1387/1407-1409；Player.java:1455-1472 |
| P1 完食打嗝（I2） | `events.rs` PLAYER_BURP 常量；`game.rs` finish_eating | FoodProperties.java:39-48；SoundEvents.java:1284 |

事件表完备性（`events_table.rs` MAPPING/WIRED 集合锁）同步扩面。
