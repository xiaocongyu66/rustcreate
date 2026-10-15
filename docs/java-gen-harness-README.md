# java-gen-harness — 26.1 vanilla worldgen 黑盒对拍器

给 mcv-engine 的 Rust 地形实现当终局裁判:同 seed 下输出官方 26.1
语义的体素 dump(`JGH1` 二进制)与 density/噪声探针值,做逐位或分带
差异报告。整条 terrain 路径**从 26.1 反编译源码编译执行**(不是 26.2
jar 跑批),依赖 jar 全部取自 Mojang 官方 26.1 清单。

## 合规铁律

**Mojang 反编译源码绝不入仓**(EULA 禁再分发;本项目纪律同 mc-ref
只读参考树)。入库的只有:424 文件拷贝**清单**
(`bootstrap/copy_manifest.txt`)+ 原创**修型补丁**
(`bootstrap/harness-patches/decompile-fixes.patch`)+ 自研 CLI 与
python 对拍框架。工作区 `src/net/**` 由 `harness/bootstrap.sh` 从
本地参考树生成,`.gitignore` 已锁死——任何人想把反编译源提交进来
都会被 git 拒收。

## 本地前提

- 本机 JDK(java/javac,21 或 25 均可)+ `tools/java_dupe.py` 需要的 python3。
- 数据包 JSON:`--data-root`(或 `-Dharness.data=...`)指向包含
  `data/minecraft/worldgen/` 的目录。默认
  `/root/mc-ref/src-26.1/data`(mc-ref 只读参考树,harness 不拷贝、不入仓)。
- `libs/` 不入仓(见下),首次构建前需生成;`build/` 是产物,均可随时重建。

## 构建

```sh
cd /root/java-harness
harness/bootstrap/bootstrap.sh   # 从本地参考树生成 src/net/** + 应用修型补丁
./build.sh                       # javac 425+ 个源文件 -> build/,零 error 才算过
```

classpath 由 `build.sh` 拼接:`libs/server-26.1.jar`(官方 26.1
server jar,从 piston 下载并 sha1 校验)+ 107 个精确 26.1 依赖 jar
(piston-meta `26.1.json` 的 libraries 清单逐个 sha1 校验后下载)。

> **版本锚定**:本机 `/root/Vanilla/jars/minecraft_server.26.2.bundler.jar`
> 内嵌的是 26.2 的依赖(DFU 10.0.21/guava 33.6.0/...),与 26.1 源码存在
> API 错位(如 DFU 9 的 `RecordCodecBuilder.group` 已上移为
> `com.mojang.datafixers.kinds.Kind1` 默认方法,官方 26.1 字节码走该链)。
> 用 26.2 依赖编译 26.1 源码会得到 154 个假错 + 静默语义风险。26.1 需要
> **DFU 9.0.19 / guava 33.5.0-jre / mojang-logging 1.6.11**,故 harness
> 的 libs 全量来自 piston 26.1 清单,server jar 本体同样来自 piston
> (官方未混淆、mojang-mapped 类名)。一次性获取(幂等,sha1 校验):
>
> ```sh
> # manifest: https://piston-meta.mojang.com/v1/packages/8667c5278895a6ae31e05ab99ba044ef33fca092/26.1.json
> # 遍历 libraries[].downloads.artifact 下载到 libs/<path> 并比对 sha1(共 107 库);
> # downloads.server.url 下载外层 bundler,解出 META-INF/versions/26.1/server-26.1.jar 放 libs/
> ```

## 运行

```sh
CP="$(printf '%s:' build libs/server-26.1.jar; find libs -name '*.jar' ! -name 'server-26.1.jar' | sort | tr '\n' ':' | sed 's/:$//')"

# 1) 体素 dump(stage=noise fill;yMax 为开区间端点)
java -cp "$CP" Main --seed 123 --chunk 0,0 -o out.bin
java -cp "$CP" Main --seed 123 --chunk -3,7 --y-min -64 --y-max 320 -o out.bin

# 2) density 单点探针(router.finalDensity,十六进制 fp 位 + 十进制)
java -cp "$CP" Main --seed 123 --density -32,17,25

# 3) NormalNoise 通道探针(原版种子链,可批量)
java -cp "$CP" Main --seed 123 --probe-noise temperature,-32,17,25 --probe-noise erosion,0,100,-3
```

### JGH1 格式(供 python 映射到 Rust u16)

```
magic "JGH1" | i32 idCount | i64 seed | i32 chunkX | i32 chunkZ | i32 yMin | i32 yMax(开)
idCount x (u16 len + utf8 blockstate 名)
(16*16*(yMax-yMin)) x u16 BE 体素 id;y,z,x 序(y 最外);id 0 = minecraft:air
```
sidecar `out.bin.json` 记录 meta + id→blockstate 名映射
(`block[key=v,...]` 属性按键名排序,确定性)。python 侧解析见
`tools/java_dupe.py::parse_jgh1`。

### dump 语义口径(与 Rust 对拍时必读)

stage = **noise fill only**(`NoiseBasedChunkGenerator.doFill` 的逐体素
镜像,由 `HarnessFill` 在 levelgen 包内桥接 protected 访问器):
- 值 = `NoiseChunk.blockStateRule` = aquifer(finalDensity + Beardifier)
  (+ 矿脉开启时的 ore vein 项);null 落 default_block。
- **不含** surface 规则 → 原版在此阶段 y<0 也是 `minecraft:stone`
  (deepslate/表土来自 buildSurface 阶段),不含草地/沙/雪。
- `Beardifier.EMPTY`:有意关闭结构 beard(与"无结构"的 Rust 基线同口径)。
  差异报告里若出现"原版实、Rust 空"的小范围山丘状实体隆起,先查结构 beard。
- 无 carver/feature(生成阶段 raw 之前)。

## 对拍(python)

```sh
python3 tools/java_dupe.py dump --seeds 1 123 20260114 --chunk 0,0
python3 tools/java_dupe.py report --seeds 1 123 20260114 --chunk 0,0
python3 tools/java_dupe.py voxel-diff -a dumps/seed1_c00.bin -b <rust-dump.bin>
```

`report` 三节:
1. dump 清单 + 确定性门(同 seed 跑两遍逐位相同)。
2. NormalNoise 通道探针:java(官方 jar 原值)vs python 原版种子链复刻 vs
   python 复刻的 noise.rs v1 种子方案。第一条是**倍频引擎门**(位型或
   1e-12 同值),第二条量化 noise.rs:30-33 自述的"自有 splitmix 域盐派生"
   与原版 fork 链的差距。
3. 差异定性说明 + `voxel-diff` 用法。

`voxel-diff` 输出:总同点率、首个差异坐标(x,y,z + 两侧 blockstate)、
逐 32m 分带差异率。Rust 侧 dump 用 CI 产物(本机禁 cargo):
在 mcv_worldgen 挂 `#[test] dump_for_java`(env `RUST_DUMPOUT=path` 时写
JGH1,否则 no-op),CI artifact 下载后作为 `-b` 传入即可。

## 修型清单(全部机械修,不改语义,文件头带 `// HARNESS-PATCH` 注)

| 文件(levelgen 相对路径) | 修法 |
| --- | --- |
| `NoiseSettings.java` | RecordCodecBuilder lambda 显式泛型 |
| `structure/placement/RandomSpreadStructurePlacement.java` | 同上(mapCodec) |
| `presets/WorldPreset.java` | 同上 |
| `blending/BlendingData.java` | 同上(内嵌 record Packed) |
| `structure/structures/JigsawStructure.java` | 同上(settingsCodec 推断) |
| `flat/FlatLevelGeneratorSettings.java` | 同上(8 字段 group) |
| `structure/pieces/PiecesContainer.java` | guava builder 显式类型见证 |
| `structure/StructurePiece.java` | 同上 |
| `structure/structures/OceanMonumentPieces.java` | 同上 |
| `structure/structures/MineshaftPieces.java` | codec read 显式类型见证 |
| `synth/…` 等 414 文件 | 无修型(bootstrap 按清单生成,**不入仓**) |

根因两族:CFR 反编译丢 lambda 目标类型(DFU 9 的 `Kind1.group` 重载
族在 javac 下推断崩溃);CFR 丢 builder 泛型见证。修型均为
补显式类型/实参,零行为改动(与官方 jar 字节码对照过)。

另:harness 自有文件(非 vanilla,头注标明)
`net/minecraft/world/level/levelgen/HarnessFill.java`(protected 桥接 +
doFill 循环镜像)、默认包 `Main.java`(CLI/引导)。

## 已知差异门口径

| 差异类别 | 归因 | 首证量法 |
| --- | --- | --- |
| 深板岩/表土/草地缺失 | dump 口径 = noise fill(无 surface 阶段),非 bug | band 表里 y<0 全 stone 属预期 |
| 空腔带 | aquifer 决策(原版) vs Rust 洞口算法;原版 carver 阶段洞穴**不在本 dump** | band 差异率 + 首差坐标 |
| 结构 beard | dump 关(beardifier=EMPTY);真实原版会在结构处隆起 | 若 Rust CI dump 同口径则同为 0 |
| NormalNoise 值漂 | noise.rs v1 自述:splitmix 域盐种子 ≠ 原版 fork 链 | report 第 2 节数值 |
| 浮点末位 | ImprovedNoise 纯 Java double,与 Rust f64 位型应同;不一致即引擎 bug | probe bits 对照 |

## 终局路径(逐位一致闭环)

1. report 第 2 节把 rust `NormalNoise::from_vanilla_name`(xoroshiro fork 链)
   对到 java 探针位型一致(引擎已按原版机制转写,预期直接过)。
2. Rust 侧把 density 通道种子从 splitmix 域盐切换为原版 fork 链
   (或接受该差异并进文档),然后 terrain.rs 的 finalDensity 组装按
   26.1 `NoiseRouterData` 全展开(连续 splines + intercepts),与
   `--density` 探针逐位对。
3. 体素级:CI `dump_for_java` 产出 JGH1 → `voxel-diff` 归零
   (同点同值率 100%,first_diff=null)。此后任何地形改动都跑该门,
   这就是地形保真闭环。
