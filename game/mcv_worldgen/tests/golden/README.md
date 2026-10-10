# 黄金数据（回归锁，非正确性标准）

由任务板 #77 在删除 `cpp/`（C++ frozen oracle：terrain.cpp + noise.h +
mempool.cpp）前，于本机用 g++ 直编 oracle + 独立驱动全量落盘固化
（基线 1dd3470，aarch64-linux-gnu-g++ -O2 -ffp-contract=off，与 x86 CI 的
cc-crate 构建在所覆盖数值上逐字节一致——删除前已用 rustc 独立 harness 对
Rust 移植全量对拍验证 400/400 哈希 + 4/4 全量落盘一致）。

**黄金值是 frozen oracle 的输出，即 legacy 常数体系的旧近似语义，不是
26.1 权威语义，更不是「原版应该如此」的依据**。26.1 正确性门在 vanilla
后端 + tests/quality.rs（对标 src-26.1 反编译源码）。黄金文件的唯一职责：
C++ 删除后，Rust 移植仍与删除前 oracle 输出逐字节一致（防重构手滑）。

输入规格唯一来源：`../golden/cases.rs`（seed 表/网格/落盘清单）。

## v6 重锚（384 / min_y=-64 / 海平面 63 基线）

C++ oracle 已删除，#77 的「与删除前 oracle 一致」职责随之终止。本目录现由
**Rust legacy 自生成重锚**：黄金值 = v6 几何（16×16×384、y ∈ [-64,320)、
海平面 63）下 legacy 内核的输出，语义 = 回归锁 —— 只保证「迁移后行为不再
漂移」，不是正确性标准（26.1 门仍在 vanilla 后端 + tests/quality.rs）。
重生成走 CI：`GOLDEN_UPDATE=1` 跑 `--test golden`（Linux x86_64 runner），
输入仍严格取自 cases.rs。

| 文件 | 格式 |
|---|---|
| `terrain_parity.tsv` | 400 行：`seed(016x) cx cz vox_hash(016x) hm_hash(016x)`（FNV-1a 64；体素按 u16 LE 字节流哈希） |
| `terrain/terrain_s<seed>_c<cx>_<cz>.bin` | 4 个全量区块：98304×u16 体素（LE）+ 256×i16 绝对 y 高度图（LE），共 197,120 B |
