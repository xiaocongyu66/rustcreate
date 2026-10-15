# par-water 首版矩阵报告（#209 后半：含水层水面对拍）

日期 2026-10-15 · 分支 noise-vanilla-fork a065b90d 后 · 窗=cave_audit 同窗
（chunks (2,-4)..(5,-1) 4×4，y∈[-64,96)，seed 0x7EA20B1）
rust 翼=dump_for_java artifact（run 38749575664）· 官方翼=harness 官方 jar
dump（fork-reanchor-dumps/cave_s0x7EA20B1_*，java_dupe.parse_jgh1 直读）

## 结论
1. **首实心面：全窗 0 列差 >8**——find_top_surface 实装后实心骨架与
   官方列列对齐（#209 prelim 刀有效性实证）。
2. **水语义：cell-center 原 Xoroshiro 切换后，窗东半（c5_*, c4_-1）对齐**
   （c5_-1 水/气一致列 219/256，fluid 带计数双侧归零）——AQUI-1 #2
   销账刀生效。
3. **窗西半（c2_*, c3_*, c4_-4）rust 侧「无洞全实心」vs 官方大洞**
   （c2_-2 [0,16) air：官方 2335 vs rust 0；c2_-4 152/0；c3_-4
   1036/0）——**非水语义，是 final_density 洞腔面差异**：官方在该区
   生成负 density 空腔，rust 同列实心。此面=KNOWN-DIVERGENCE
   「finalDensity 端到端逐位对拍仍待 splines/intercepts 全展开」
   （首证报告 §6.2）的第一次体素级定位实证；首实心面口径对本类
   差异天然盲（双侧「自底向上首个实心」在洞上下同位）。

## 数值表
| chunk | 水/气一致列 | [0,16)air off/rust | [-32,-16)air off/rust |
|---|---|---|---|
| c2_-4 | 0/256 | 152/0 (rust 水 469) | 297/0 |
| c2_-2 | 0/256 | 2335/0 | 0/1618(洞在-32~-16 带) |
| c2_-1 | 0/256 | 754/0 | 25/513 |
| c3_-4 | 0/256 | 1036/0 (rust 水 1123) | 0/0 |
| c3_-1 | 0/256 | 67/0 | 434/26 |
| c5_-1 | 219/256 | 0/0 | 5/0 |
| c4_-3..c5_-3 等 | 双侧无水无洞 | 0/0 | — |

## 下一刀定位
final_density 全展开（terrain.rs 终态树 vs noise_router.final_density
逐节点——spline/intercepts 端）与逐位对拍 = 保真闭环路径 §6.2 的
第 2 步；水门侧（AQUI-1）已净（东半对齐 + 西半洞差属上游 density）。
