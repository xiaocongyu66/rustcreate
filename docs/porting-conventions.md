# Java→Rust 机制移植约定（防演绎作品定性）

目标：完整还原 MC Java 26.1 的**行为**；不产生 Mojang 代码的演绎作品。

## 硬约束

1. **只移植机制，不翻译表达。** 从反编译源码提取：常数、公式、执行顺序、
   边界条件、数据结构语义——写成机制规格（笔记仅存开发机
   `/root/mc-ref/NOTES-*.md`，永不入库）。代码表达层 100% 自有：
   不照抄类划分、方法分解、控制流写法、命名、注释。
2. **仓库零 Mojang 内容。** `ci/port-manifest.csv` 只含文件名+状态；
   任何 Java 原文/笔记/反编译产物不进 git，CI 不接触 jar。
3. **验收=行为对拍，不是结构对应。** 每个机制配表驱动测试：原版常数
   断言（值+出处 file:line 注释）、输入→输出对拍。测试注释引用
   Java `file:line` 作机制出处（引用出处≠复制表达）。
4. **落地进现有框架。** 移植产物接进我们的架构缝（WorldView/VoxelAccess、
   GameRuntime tick、mcv_ecs 系统），不镜像 Mojang 的包/类树；
   一个 Rust 模块可覆盖多个 Java 文件，反之亦然。
5. **client/server/network 桶豁免。** 渲染/协议/存档机制由
   mcv_render/mcv_app/mcv_save 承担等价职责，账本标 exempt/adapted。

## 账本

`python3 ci/port-manifest.py` 重新生成 `ci/port-manifest.csv`
（logic=逐文件机制移植目标，adapt/exempt=框架承担）。
agent 完成一个域后把对应行 status 改为 ported/adapted 并填 rust_file，
随同一提交入库。
