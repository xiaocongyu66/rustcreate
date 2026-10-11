# 真机验证清单——整区块重构刀 5（#115 收口）

Android 真机（Mali GPU）手测清单。执行环境：`adb logcat -s RustMcv`
（Debug 级已放行，流式/光照/网格埋点带区块坐标）。每项跑完在结果列记
PASS/FAIL + 取证（截图/日志行/视频）。基线 = 刀 1-4 交付机（f1de441 构建）
同场景数据。

## 1. 进世界内存：≤ 基线 + 20%

- [ ] 冷启动 → 创建新世界 → 进世界加载完成，记录加载完成时刻 RSS
      （`adb shell dumpsys meminfo <pkg> | grep TOTAL`，或 procrss 常驻
      采样）。对比基线同口径值，增幅 ≤20%。
- [ ] 网格域期望：render-stats 行 `resident_kb` 稳定后 ≈ CI 实测同量级
      （合成 19²=361 块 ~15 MiB；真实地形树冠/洞穴更大，量级突增即记账
      异常，回看 `BatchingStats` 口径）。

## 2. 持续跨块 5 分钟：无隐形墙 / 无冻结

- [ ] 直线朝一个方向连续行走 5 分钟（跨越 ≥15 个区块边界），全程可自由
      移动、无「撞看不见的墙」（刀 4 clamp_to_sim_area 票环派生 +
      debug_assert 前哨，debug 构建下越界即 panic——logcat 无 PANIC）。
- [ ] 每 30s 记一次 RSS：应围绕稳态值波动，无单调增长（增长 = 迟滞环/
      复活缓存泄漏嫌疑，对照 `docs/memory-budget.md` §5 移动足迹口径）。
- [ ] logcat 无 `border pair` 同一对反复出现（静止期重复 = 记账重置乒乓
      实证埋点）；无 `mesh upload` 同块反复（重网格循环实证）。

## 3. 视距切换不崩

- [ ] 设置界面视距 4→16→4 往返各两次（每次落盘 options.txt），切换与
      回世界均不崩；切低视距后 logcat render-stats 行 `chunks=` 收敛
      下降、无残留条目（GPU 驻留 `resident_kb` 同步下降）。

## 4. 后台 5 分钟回来不崩

- [ ] 进世界后 Home 键退后台 5 分钟 → 回前台：不崩、世界可用、tick 恢复
      （D-6 挂起路径 + C-2 主增益恢复）。logcat 无 PANIC / 无 GPU 报错。

## 5. 30 分钟 RSS ≤ 180 MB

- [ ] 进世界后正常游玩（走动+挖放）30 分钟，RSS 峰值 ≤180 MB。超出即
      对照 `docs/memory-budget.md` 口径分解（resident + 网格域 + 图集
      ~1 MiB + 迟滞足迹）定位增长域。

## 6. 移动端默认视距 6 首屏内存观察（刀 5b）

- [ ] 清除 app 数据（首跑无 options.txt）→ 启动 → 进世界：首屏视距
      应为 6（画面可辨环数；或 options.txt 落盘值 `render_dist=6`）。
- [ ] 首屏（加载完成 + 30 s）RSS 记录：与视距 8 旧版同场景对比，期望
      resident 差 ≈ (361−225)×288.5 KiB ≈ 38 MiB（算式见
      docs/memory-budget.md §1）。
- [ ] 设置内改视距并重启 app：持久化往返生效（8/6 不被覆盖回写）。

## 7. 光黑交替抓 logcat（#119）

- [ ] 构造明暗交替场景（洞穴口进出 / 火把放置撤除 / 昼夜线），持续
      2 分钟抓 logcat：无光照边乒乓（`border pair` 同对反复）、无
      `sync_light_edges` 风暴、画面无整块黑闪/明暗跳动。
- [ ] 存证：logcat 文件 + 屏录。#119 关闭依据。

## 8. 水色 / 雾 / 无波动复验（#118）

- [ ] 海面/湖面：水色为原版蓝绿染色（非纯色/非缺色），水下雾生效、
      离水恢复正常雾距。
- [ ] 水面顶点无异常波动/波纹（WATER_TOP_SINK 之外无漂浮碎面）；
      水下看水面无双绘/闪烁。
- [ ] 存证：wet/dry 截图各一张（对照 CI render-screenshot 工件）。
      #118 关闭依据。

## 9. Mali 真机 render-stats 取证（#80 遗留 + 刀 5a）

- [ ] 稳态（进世界静止 1 分钟）抓 ≥3 行 render-stats：
      `draws / water_draws / entries / chunks / rebuilds / rebuild_kb /
      staged_uploads / staged_kb / fallback / resident_kb / pool_kb`。
- [ ] 判读：`fallback=0`（staging 环容量足够）、`rebuilds` 稳态不涨、
      `entries ≈ chunks/8` 量级（合批收敛）。数据回填
      docs/memory-budget.md §2 真机行，#80 观测债关闭依据。
- [ ] 行走 1 分钟再取 3 行：`rebuilds` 增速与新加载块数同量级（无空转
      重建）。

## 执行备注

- 全部项在 debug 构建跑（debug_assert 前哨生效）；第 1/5 项内存数字
  另跑一次 release 构建复核（debug 分配器放大 ~1.5-2×，以 release 为
  预算口径、debug 为回归口径）。
- logcat 抓取统一 `adb logcat -c && adb logcat -s RustMcv > capture.txt`。
- 任一 FAIL：先对照本仓 CI 断言（docs/memory-budget.md §4）判「记账
  错」还是「真超支」，前者修断言口径、后者开性能刀。
