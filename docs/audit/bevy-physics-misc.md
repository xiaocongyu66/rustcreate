# Bevy 生态差距审计——物理 / 动画 / Picking / 诊断 / 网络（五域）

> 分支 `bevy-gap-work`·审计性质（只读侦察，不改代码）。对拍对象：Bevy 0.1x 生态
> （avian 物理原生 ECS、`bevy_animation`、`bevy_picking`、`bevy_dev/bevy_diagnostics`
> 诊断面板、`bevy_replicon` 网络复制层）vs 本仓手写实现。
> 结论分级约定：**平迁**（直接搬语义，低风险）/ **重构**（需重新设计接线）/
> **远期**（当前不确定做的事）/ **反向**（Bevy 为通用游戏设计、MC 还原不需要的部分——
> 引入反而是负资产）。

## 0. 本仓五域现状（代码证据）

### 0.1 物理域

手写**运动学 CharacterBody**，无刚体/求解器：

- `engine/mcv_game/src/physics.rs`（802 行）：`Aabb` + 单轴扫掠碰撞
  （`move_box`/`move_axis`，子步 ≤0.5 m、SKIN=1e-4 间隙）；轴序对齐原版
  `Entity.collideWithShapes`（Y 恒最先，水平按 |vx|<|vz| 取序）；
  特性：潜行防跌落 `edge_back_off`（Player.java:880-933 对拍）、自动上台阶
  `try_step_up`（Entity.java:1080-1136 候选顶面+位移比较）、梯子攀附
  `on_climbable`（外扩 1e-3 探测）、水中三模式（普通/冲刺游泳视线转向/
  出水 jumpOutOfFluid）、创造飞行、显式 Euler 积分。
- `engine/mcv_game/src/consts.rs`：MC 语义常数全表（g=32 m/s²、JUMP 8.4 m/s、
  WALK 4.317、STEP_HEIGHT 0.6、LADDER_* 等，均带 26.1 反编译行号出处）。
- `engine/mcv_game/src/blockshapes.rs`（441 行）：**碰撞盒 ≠ 拾取盒** 双形状表
  （半砖/楼梯/栅栏按状态盒，火把/花草无碰撞、拾取整格）。
- 消费面：玩家 `step`（physics.rs::step）+ 怪/箭/掉落物 `step_entity`
  （同一 AABB 通路，`gravity_scale` 区分）；`game/mcv_entity/src/components.rs`
  `PhysBody`（pos/vel/on_ground/pos_old）。
- 测试：`tests/physics*.rs`（5 个文件：calib/ladder/movement/sweep_air）、
  `physics_calib.rs::jump_onto_one_block` 峰高校准。

### 0.2 动画域

手写**账本相位 + CPU 关节矩阵**：

- 账本：`game/mcv_entity/src/passive.rs::AnimState`（walk phase/amount、
  鸡扑翼 wing_phase/wing_speed、羊吃草 head_drop/head_eat_angle）+ 行走
  推进 `advance_walk_anim`（60 Hz 步、按 4.317 m/s 满速比值缩放，
  LivingEntity.walkAnimation 等价）。
- 装配：`game/mcv_logic/src/mob_render.rs::collect_mob_instances_in` ——
  渲染帧用 `pos_old→pos × partialTick` 插值 + MobPose →
  `mob_mesh::mob_model_matrices`（per-part 4×4，MAX_MOB_PARTS 上限）
  打包进 `MobInstance`（GPU instancing）。
- 模型：`engine/mcv_render/src/mob_mesh.rs`（1576 行，8 种怪 hard-coded
  关节模型）+ `player_mesh.rs`（599 行，玩家第一/三人称同款摆动账本）。
- 无骨骼骨架资产、无 blend graph、无动画事件（音效帧/触发帧直接由
  游戏逻辑在 tick 边界结算，不走动画系统）。

### 0.3 Picking 域

- 方块：`engine/mcv_game/src/raycast.rs`（155 行）Amanatides & Woo DDA
  网格步进，命中判据 = `blockshapes::pick_boxes`（拾取形状与碰撞解耦：
  火把 column 可拾取、花草整格、水/空气无盒），返回 (BlockPos, 入射法线)；
  REACH 4.5 / CREATIVE_REACH 5.0 常数。
  另有 `game/mcv_logic/src/game.rs::block_hit_t`（攻击遮挡专用 DDA，
  判据=碰撞形状「水/花不挡刀」——刻意与拾取判据不同源）。
- 实体：`game.rs::pick_attack_target`（:8554）ray-AABB vs 脚底中心+half_size
  算盒子（高=2·hy），≤ ENTITY_ATTACK_RANGE 且严格近于方块命中距离才入选，
  取最近；命中位姿 `attack_pick_pos`（:8544）= `lerp(pos_old,pos,partial)`
  **与 mob_render 渲染读数逐字同源**（判定侧与渲染同一插值位——#attack-pick 同源插值）。
- 触屏：`engine/mcv_platform/src/touch.rs` 供 Android 指针事件，
  无独立 Hover/Cursor 抽象。

### 0.4 诊断域

- `game/mcv_app/src/app.rs`（:3229-3280）**~1 Hz 一行结构化日志**：
  `render-stats: draws=… water_draws=… entries=… resident_kb=… pool_kb=…
  evicted=… dispatch_gap=… worker_lost=… | frame-timing[us] mean/max …`。
- 帧预算分段计时（E0，#155）：`game/mcv_logic/src/game.rs::FrameTiming`
  9 段（fixed_step/ecs/stream{init,edge,io,remesh}/flush/draw）+ 1Hz
  mean/max 累计器（`FrameTimingStats::record/take`）；口径表
  `docs/memory-budget.md`、`docs/worker-accounting.md`。
- 纯 log，无屏幕内指标面板/图表、无 CPU/GPU 采样器、无系统级计数器注册面。

### 0.5 网络域

**本仓无此面**（如实标注）：全仓 grep 无 protocol/packet/socket/replicate 类
实现；单机 + Android 目标。`engine/mcv_sync` 名字易误导——实为音频线程
无锁 SPSC 环（Lamport 队列），非网络。存档走 `engine/mcv_save`（region IO，
本地盘）。无客户端/服务器拆分：worldgen/实体/光照全部单进程。

## 1. Bevy 生态侧（docs.rs / 官网侦察，待补）

（第二节由后续轮次填充）

## 2. 五域对拍与分级（待补）

（第三节由后续轮次填充）
