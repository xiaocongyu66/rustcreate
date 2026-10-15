# java-gen-harness 首证对拍报告

日期: 2026-10-14 · harness: /root/java-harness (branch java-gen-harness)
mc: 26.1 (dataVersion 4786, piston manifest 8667c527…092) · chunk (0,0) · y∈[-64,320)
dump 口径: stage=noise fill(Beardifier.EMPTY,无 surface/carver/feature),JGH1 格式

## 1. 构建证据

- `./build.sh` → `BUILD OK: 698 classes`,javac **0 error**(425 份 26.1 源拷贝
  + Main/HarnessFill;仅 3 条 deprecation 警告, vanilla 26.1 自带)。
- 修型 10 文件全部带 `// HARNESS-PATCH` 头注(清单见 docs/java-gen-harness-README.md)。
- 官方 jar 运行时自证: `SharedConstants+Bootstrap` 后官方 NoiseSettings codec
  在 DFU 9.0.19 上往返解析 -64/384/1/2 成功(`Optional[NoiseSettings[minY=-64…]]`)。

## 2. dump 样本(3 seed × chunk 0,0)

| 文件 | 大小 | head hex(前 32B) |
| --- | --- | --- |
| dumps/seed1_c00.bin | 196698 B | `4a47 4831 0000 0003 0000 0000 0000 0001 0000 0000 0000 0000 ffff ffc0 0000 0140` |
| dumps/seed123_c00.bin | 196672 B | `4a47 4831 0000 0002 0000 0000 0000 007b 0000 0000 0000 0000 ffff ffc0 0000 0140` |
| dumps/seed20260114_c00.bin | 196672 B | `4a47 4831 0000 0002 0000 0000 0135 2512 0000 0000 0000 0000 ffff ffc0 0000 0140` |

(魔数 JGH1 / idCount / i64 seed / chunkX,Z=0,0 / yMin=-64(0xffc0) / yMax=320(0x140);
体素数 98304;seed1 出现第 3 个状态 = aquifer 水。)
确定性门: 同 seed 跑两遍逐位相同 = **True**。

## 3. NormalNoise 倍频引擎门(首证核心)

探针 12 点/通道(`y,X,Z` 网格),java 侧 = 官方 jar 原值(位型 0x…16 hex);
py-vanilla = 本仓 python 复刻的**原版种子链**(splitmix→Xoroshiro128++→
MD5 fromHashOf("minecraft:{name}")→forkPositional→octave_{n}→ImprovedNoise);
rust-v1 = python 复刻的 noise.rs `NormalNoise::new` v1 种子方案。

| seed | channel | py-vanilla 同点同值率 | max abs diff | rust-v1 同值率 | max abs diff |
| --- | --- | --- | --- | --- | --- |
| 1 | temperature | **12/12 位型全同** | 0 | 0/12 | 0.244 |
| 1 | erosion | **12/12** | 0 | 0/12 | 0.301 |
| 1 | ridge | **12/12** | 0 | 0/12 | 0.350 |
| 123 | temperature | **12/12** | 0 | 0/12 | 0.477 |
| 123 | erosion | **12/12** | 0 | 0/12 | 0.229 |
| 123 | ridge | **12/12** | 0 | 0/12 | 0.935 |
| 20260114 | temperature | **12/12** | 0 | 0/12 | 0.656 |
| 20260114 | erosion | **12/12** | 0 | 0/12 | 0.270 |
| 20260114 | ridge | **12/12** | 0 | 0/12 | 0.418 |

**读法**:同一原版种子链的 python 复刻与官方 26.1 jar 在 108/108 个探针位型
逐位一致(max|d|=0)——harness 的 java 探针与"原版机制"互证成立;任何走
`from_vanilla_name`(原版 fork 链)的实现(如 mcv_worldgen 的 surface_noise
路径)具备直接到逐位的通道。rust-v1 全 0/12、|d|≈0.23~0.94 = noise.rs
30-33 行自述的 v1 域盐种子差异的实际数量级(density/aquifer 通道当前走此路径)。

density(router.finalDensity) 探针同口径可对(如 seed123 @(-32,17,25):
decimal=0.054062531617338900 bits=0x3fabae158a788b24),Rust 侧按
NoiseRouterData 全展开后即可逐位对拍。

## 4. 体素差异分类机器(voxel-diff)

工具: `tools/java_dupe.py voxel-diff -a A.bin -b B.bin`(id 表按名重映射,
总同点率 + 首差坐标(x,y,z+两侧 blockstate)+ 逐 32m 分带差异率)。
机制自检(同 seed 门:两次运行全同;异 seed 门见 voxel-demo-seed1-vs-seed123.md):
seed1 vs seed123 → 总同点率 85.6%,首差 (7,-41,15) air↔stone(aquifer 空腔边界),
深部分带率 2.0%~19.7%、高空 0%——管线工作正常。

**真正的 Rust 对拍**(同 seed 逐位)本机不可跑(禁 cargo):Rust 侧在
mcv_worldgen 挂 `#[test] dump_for_java`(env RUST_DUMPOUT=path 写 JGH1,
否则 no-op),CI artifact 下载后 `voxel-diff -a java.bin -b rust.bin`。

## 5. 差异定性(首证口径)

| 差异类别 | 归因 | 证据/量级 |
| --- | --- | --- |
| NormalNoise 通道值漂 | rust density/aquifer 走 v1 splitmix 域盐种子(非原版 fork 链) | 本报告第 3 节:|d|≈0.23~0.94,同点同值率 0/12 |
| 表层噪声(surface) | rust 已用 from_vanilla_name(原版链);python 证明该链可 12/12 位型同 | 具备逐位通道,待 rust 侧自证 |
| y<0 全 stone(无深板岩) | dump 口径=noise fill,无 surface 阶段,非 bug | band 表高频 0% 差异区 |
| 空腔/水带差异 | aquifer 决策(原版)vs rust 洞口/含水算法 + 原版 carver 不在 dump 口径 | 首差落在空腔边界 (7,-41,15);CAVE-1③(carver)在 stage=noise 口径下**不产生**差异——它属于原版 buildSurface 之后的 carving 阶段,Rust 侧未实现属既知缺口 |
| 结构 beard | dump 双方都关(BEARDIFIER.EMPTY 口径一致) | 0(同口径) |

推论:目前预期差异 100% 归因"v1 种子派生 + density 组装未全展开",
carver 差异在体素对拍中只会以"原版含 carver 的 dump(待做 stage=carver)"
对比时出现;本 harness 现口径已把 carver 变量隔离出对拍面。

## 6. 终局路径(逐位一致闭环)

1. rust `NormalNoise::new` 的种子派生切到原版 fork 链(或全量改用
   from_vanilla_name)→ 第 3 节探针应 12/12 位型同。
2. terrain.rs 的 finalDensity 组装按 26.1 NoiseRouterData 全展开
   (splines/intercepts/移位),以 `--density` 探针逐位对。
3. CI `dump_for_java` 产 JGH1 → `voxel-diff` 同点同值率 100%、
   first_diff=null → 地形保真闭环,此后任何地形改动跑同门。
4. (可选)harness 增 stage=carver/surface 口径,把 CAVE-1③ 差异也纳入量化。

原始输出: logs/first-proof-run.txt;差异机器演示: logs/voxel-demo-seed1-vs-seed123.md
