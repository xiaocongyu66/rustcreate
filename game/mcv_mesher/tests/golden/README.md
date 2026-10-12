# 黄金数据（回归锁，非正确性标准）

> **重锚记录（2026-10-12，#153 light-audit 病灶#2 平滑光照层落地重锚，
> LightCoordsUtil.java:61-89）**：顶点光从每面单采（≈ vanilla Smooth
> Lighting OFF）切换为暴露层四格均值 + 借中心规则（原版默认 Smooth
> Lighting ON，见 src/mesher.rs 头注），黄金件按「迁移后行为重锚」
> 通道重写——非静默改表，差异全部可解释：
> - `light_gradient_k0.bin`：长度/顶点数/索引数/AO/flags/索引流逐位不变，
>   仅顶点光照字节（24 B 布局偏移 18/19）变为四格均值；
> - `mesh_random_batch.tsv`：恰 64/128 行变化（随机批每区块 50% 概率
>   0xF0 均匀光——均匀光四格均值=原值，行不变；其余随机光行 vc/ic 因
>   合并键纳入四角光场而合法分裂）；`in_hash` 全部未变（输入规格无漂移）；
> - 其余 12 个场景件逐字节不变（均匀光下平滑层恒等于单采）。
> 重写经 rustc 独立 harness 逐字节复刻 `tests/golden.rs` 的
> `GOLDEN_UPDATE=1` 通道（本机禁 cargo，与 CI 同源编译）。回落开关
> `MCV_SMOOTH_LIGHT=0` 的输出仍与重锚前的黄金件逐位一致（回滚面）。

由任务板 #77 在删除 `cpp/`（C++ frozen oracle：mesher.cpp + blocks_gen.inc）
前，于本机用 g++ 直编 oracle + 独立驱动全量落盘固化（基线 1dd3470，
aarch64-linux-gnu-g++ -O2 -ffp-contract=off）。删除前已用 rustc 独立
harness 对 Rust 网格器全量对拍验证 142/142 用例（14 场景 + 128 随机批）
逐字节一致。

**黄金值是 frozen oracle 的输出，即旧贪心网格路径的近似语义，不是 26.1
权威语义，更不是「原版应该如此」的依据**（平滑光照重锚后基准面更新为
「当前 Rust 输出」，职责不变：防重构手滑）。26.1 正确性门在 vanilla
后端 + tests/quality.rs（对标 src-26.1 反编译源码）。黄金文件的唯一
职责：C++ 删除后，Rust 网格器仍与删除前 oracle 输出逐字节一致（防重构
手滑）。

输入规格唯一来源：`../golden/cases.rs`（场景构造/splitmix64/行序/种子）。

| 文件 | 格式 |
|---|---|
| `mesh_scenes/<scene>_k<kind>.bin` | 14 个场景输出：u32 顶点数 LE + u32 索引数 LE + 顶点字节（24 B/顶点）+ u32 索引 LE；kind 0=opaque 1=water |
| `mesh_random_batch.tsv` | 128 行：`idx name kind in_hash vc ic vhash ihash`；in_hash 锁输入规格，vhash/ihash 为 FNV-1a 64（顶点/索引字节流） |
| `blocks_gen.inc` | 删除前 C++ 生成表逐字节副本（geom 规则逐 id 表锁数据源） |
