#!/usr/bin/env python3
"""fork-chain-mirror — density/aquifer 通道切原版 fork 链后的 python 镜像对拍。

镜像对象 = game/mcv_worldgen/src/vanilla/noise.rs `NormalNoise::from_vanilla_name`
（切链后全部 router 通道的构造路径）+ vanilla/xoroshiro.rs +
vanilla/perlin.rs::from_xoroshiro。派生链逐行对位：

  root = XoroshiroRandomSource.from_seed(world_seed).fork_positional()
         # xoroshiro.rs:236 root_positional_factory = RandomState.java:36
  rng  = root.fromHashOf("minecraft:{name}")
         # xoroshiro.rs:220 random_from_hash = RandomState.java:112-114
         # getOrCreateNoise → Noises.java:78-83 instantiate
  first/second = PerlinNoise(rng, fo, amps) ×2
         # noise.rs:126-146 build×2 = NormalNoise.java:43-44 两次 create，
         # 每次开首 forkPositional 消耗 2 nextLong（PerlinNoise.java:88）
  octave_i（非零振幅）= ImprovedNoise(factory.fromHashOf("octave_{fo+i}"))
         # PerlinNoise.java:90-95，零振幅不建实例不消耗随机数

复刻函数借 java-harness/tools/java_dupe.py 的 Xoroshiro/ImprovedNoise/
PerlinNoise/NormalNoiseVanilla——该组复刻与官方 26.1 jar 已 108/108 位型互证
（/root/java-harness/logs/first-proof-report.md §3）。java 真值 = 同 harness
官方 jar `--probe-noise` 探针（Main.java:117-131 getOrCreateNoise 实物链），
任意通道名可探（ResourceKey.create(Registries.NOISE, "minecraft:{name}")）。

对拍口径（同首证）：3 seed × N 通道 × 12 探针点，python f64=IEEE double 与
java double 同语义，目标逐位同（max|d|=0）。通道表 = 切链后 Rust 常量
（climate.rs/aquifer.rs），并与 data/minecraft/worldgen/noise/*.json 逐通道
比对（表长含尾部零填充——PerlinNoise.lowestFreqValueFactor 依赖表长）。

用法: python3 tools/fork-chain-mirror.py [--seeds 1 123 20260114] [--points 12]
只读 /root/java-harness 与 /root/mc-ref；不写 java-harness。
"""

import argparse
import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, "/root/java-harness/tools")
import java_dupe as jd  # noqa: E402  (借 java-harness 复刻函数，见模块头注)

MC_DATA = Path("/root/mc-ref/src-26.1/data")  # load_params 自带 minecraft/worldgen/noise/ 前缀

# ---------------------------------------------------------------- 对位表
# Rust 侧切链后的通道常量（climate.rs / aquifer.rs / terrain.rs）——
# 通道名 = Noises.java:11-52 createKey 实名；firstOctave/振幅表逐字抄
# 对应 noise json（含尾部零填充）。
CHANNELS: dict[str, tuple[str, int, list[float]]] = {
    # key: (rust 常量名, firstOctave, amplitudes)
    "temperature": ("minecraft:temperature", -10,
                    [1.5, 0.0, 1.0, 0.0, 0.0, 0.0]),
    "vegetation": ("minecraft:vegetation", -8,
                   [1.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
    "continentalness": ("minecraft:continentalness", -9,
                        [1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0]),
    "erosion": ("minecraft:erosion", -9, [1.0, 1.0, 0.0, 1.0, 1.0]),
    "ridge": ("minecraft:ridge", -7, [1.0, 2.0, 1.0, 0.0, 0.0, 0.0]),
    "offset": ("minecraft:offset", -3, [1.0, 1.0, 1.0, 0.0]),
    "aquifer_barrier": ("minecraft:aquifer_barrier", -3, [1.0]),
    "aquifer_fluid_level_floodedness": ("minecraft:aquifer_fluid_level_floodedness", -7, [1.0]),
    "aquifer_fluid_level_spread": ("minecraft:aquifer_fluid_level_spread", -5, [1.0]),
    "aquifer_lava": ("minecraft:aquifer_lava", -1, [1.0]),
}

# Rust 公式镜像（noise.rs from_vanilla_name）——与 java_dupe.vanilla_channel
# 同链，此处显式按 Rust 结构走一遍以便审计：
def rust_from_vanilla_name(world_seed: int, name: str, first_octave: int,
                           amplitudes: list[float]) -> jd.NormalNoiseVanilla:
    # xoroshiro.rs:236-238 root_positional_factory(world_seed as i64)
    root = jd.XoroshiroRandomSource.from_seed(world_seed).fork_positional()
    # xoroshiro.rs:220-223 random_from_hash：MD5(name) ⊕ 工厂态 → 生成器
    #（不 fork；Noises.instantiate 直取 fromHashOf）
    rng = jd.factory_from_hash(root, name)
    # noise.rs:126-146 build×2：每次 fork_positional（2 nextLong）+
    # 非零振幅 octave_{fo+i} fromHashOf（零振幅占位不消耗随机数）
    return jd.NormalNoiseVanilla(rng, first_octave, amplitudes)


def check_tables_against_json() -> list[str]:
    """Rust 常量表 ↔ noise json 逐通道一致（firstOctave + 全表含零）。"""
    problems = []
    for short, (_name, fo, amps) in CHANNELS.items():
        jfo, jams = jd.load_params(MC_DATA, short)
        if jfo != fo or list(jams) != amps:
            problems.append(
                f"{short}: rust=({fo}, {amps}) json=({jfo}, {list(jams)})")
    return problems


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--seeds", type=int, nargs="+", default=[1, 123, 20260114])
    ap.add_argument("--points", type=int, default=12)
    ap.add_argument("--channels", nargs="*", default=list(CHANNELS))
    ap.add_argument("--golden", action="store_true",
                    help="额外打印 Rust 侧 golden 测试向量（decimal+bits）")
    args = ap.parse_args()

    problems = check_tables_against_json()
    print(f"[0] rust 常量表 ↔ noise json：{len(CHANNELS)} 通道，"
          f"不一致 {len(problems)} 项")
    for p in problems:
        print("    MISMATCH", p)
    if problems:
        sys.exit(2)

    # 探针点（同首证 first-proof-report §3：y,X,Z 伪网格 + 深部点）
    pts = [(k * 7 - 32, 17 + k * 3, 25 + k * 5) for k in range(args.points)]

    total = same = 0
    max_d = 0.0
    rows = []
    for seed in args.seeds:
        # 一次 JVM bootstrap 探完该 seed 全部通道×点位
        specs = []
        for ch in args.channels:
            _name, fo, _amps = CHANNELS[ch]
            _ = fo
            specs += [f"{ch},{y},{x},{z}" for (y, x, z) in pts]
        probes = jd.java_noise_probes(seed, specs)
        for ch in args.channels:
            name, fo, amps = CHANNELS[ch]
            noise = rust_from_vanilla_name(seed, name, fo, amps)
            n = 0
            ok = 0
            for (y, x, z) in pts:
                jdec, jbits = jd.java_probe_value(probes, ch, y, x, z)
                v = noise.get_value(float(x), float(y), float(z))
                vbits = int(struct.unpack(">Q", struct.pack(">d", v))[0])
                n += 1
                total += 1
                if vbits == jbits:
                    ok += 1
                    same += 1
                else:
                    max_d = max(max_d, abs(v - jdec))
                if args.golden and (ch, seed, y) in {
                    ("temperature", 1, -32),
                    ("ridge", 123, 3),
                    ("aquifer_barrier", 20260114, 45),
                }:
                    print(f"GOLDEN seed={seed} {ch} ({x},{y},{z}) "
                          f"decimal={v:.17g} bits=0x{vbits:016x} "
                          f"java_bits=0x{jbits:016x} eq={vbits == jbits}")
            rows.append((seed, ch, n, ok))
            flag = "OK " if ok == n else "RED"
            print(f"  seed {seed:>10} {ch:<32} {flag} same={ok}/{n}")

    print(f"\nTOTAL bit-exact {same}/{total}"
          + (f"  max|d|(mismatches)={max_d:.3g}" if same != total else "  max|d|=0"))


if __name__ == "__main__":
    main()
