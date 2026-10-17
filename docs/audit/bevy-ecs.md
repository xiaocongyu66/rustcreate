# ECS 与调度架构 vs Bevy 差距审计（WIP 骨架）

> 状态：调研进行中（本骨架先落盘防蒸发，随后逐节充实）。
> 对象：本仓 `engine/mcv_ecs`（Godot 语义自研 ECS）+ `game/mcv_logic` 的调度使用面，
> 对拍 Bevy `bevy_ecs`（本地 `/root/engine-ref/bevy_ecs-0.20.0`，0.20.0 版）。
> 结论矩阵与分级建议随后补齐。

## 现状速记（已核实）

- `mcv_ecs`：sparse-set 存储 + 双阶段调度（Fixed 1/60 追帧上限 5 + Variable）+
  事件双缓冲（阶段边界 rotate_all）+ CommandQueue 帧末 apply + Plugin 壳；
  单线程、零 unsafe、RefCell 运行期借互斥。约 860 行（src）。
- 游戏侧：`GameRuntime` 巨型宿主结构，ECS 只覆盖 mob/掉落物/经验球
  （`mobs_app: mcv_ecs::App`，7 个 Fixed 系统）；玩家/方块/天气/熔炉等仍是
  宿主裸字段，靠 `fixed_step` 内 `on_tick` 手工分频；资源注入走每步所有权
  快照（`MobServices`/`DropWorld`，Arc 计数递增）；命中/拾取走事件
  `channel().take()`。
- `Stage::Variable`、`Plugin`、`App::update` 游戏侧零使用（仅测试）。

## 本节 TODO

- [ ] Bevy 0.20 各模块证据落点（archetype / schedule / change_detection /
      message+event / observer / relationship / resource / query / world）
- [ ] 差距矩阵（能力 × 状态 × 迁移成本/风险）
- [ ] 「可平迁改据」与「纵深重构」分级建议
