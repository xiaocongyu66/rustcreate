# 黄金数据（回归锁，非正确性标准）

由任务板 #77 在删除 `cpp/`（C++ frozen oracle：mesher.cpp + blocks_gen.inc）
前，于本机用 g++ 直编 oracle + 独立驱动全量落盘固化（基线 1dd3470，
aarch64-linux-gnu-g++ -O2 -ffp-contract=off）。删除前已用 rustc 独立
harness 对 Rust 网格器全量对拍验证 142/142 用例（14 场景 + 128 随机批）
逐字节一致。

**黄金值是 frozen oracle 的输出，即旧贪心网格路径的近似语义，不是 26.1
权威语义，更不是「原版应该如此」的依据**。26.1 正确性门在 vanilla 后端 +
tests/quality.rs（对标 src-26.1 反编译源码）。黄金文件的唯一职责：C++ 删除
后，Rust 网格器仍与删除前 oracle 输出逐字节一致（防重构手滑）。

输入规格唯一来源：`../golden/cases.rs`（场景构造/splitmix64/行序/种子）。

| 文件 | 格式 |
|---|---|
| `mesh_scenes/<scene>_k<kind>.bin` | 14 个场景输出：u32 顶点数 LE + u32 索引数 LE + 顶点字节（24 B/顶点）+ u32 索引 LE；kind 0=opaque 1=water |
| `mesh_random_batch.tsv` | 128 行：`idx name kind in_hash vc ic vhash ihash`；in_hash 锁输入规格，vhash/ihash 为 FNV-1a 64（顶点/索引字节流） |
| `blocks_gen.inc` | 删除前 C++ 生成表逐字节副本（geom 规则逐 id 表锁数据源） |
