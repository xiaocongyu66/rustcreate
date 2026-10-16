#!/usr/bin/env python3
"""cave_audit java 对照 — 官方 jar noise-fill dump 的同口径带统计。

对拍面：tests/cave_audit.rs::cave_audit_stats 的 K-CAVE-AUDIT 行
（seed 0x7EA20B1、chunks (2,-4)..(5,-1)、y∈[-64,96)、16 格带 air/fluid/
solid/bedrock → void=air/total）。java 真值 = /root/java-harness Main
JGH1 dump（stage=noise：noise fill + aquifer + oreVeins，无 surface/
carver/features——本仓 vanilla 后端同为无 carver 面，口径相容；surface
规则不改 y<48 的 air/fluid 分类，bedrock 带差异单列）。
"""
import sys
from pathlib import Path

sys.path.insert(0, "/root/java-harness/tools")
import java_dupe as jd

D = Path("/root/noise-fork/tools/fork-reanchor-dumps")


def load_window(prefix, cx0, cx1, cz0, cz1):
    chunks = {}
    for cx in range(cx0, cx1 + 1):
        for cz in range(cz0, cz1 + 1):
            p = D / f"{prefix}_c{cx}_{cz}.bin"
            if not p.exists():
                print("MISSING", p.name)
                continue
            chunks[(cx, cz)] = jd.parse_jgh1(p)
    return chunks


def stats(chunks, ymin_all, ymax_all):
    LO, HI = -64, 96
    bands = {}
    for lo in range(LO, HI, 16):
        hi = min(lo + 16, HI)
        air = fluid = solid = bedrock = 0
        for d in chunks.values():
            names = d["names"]
            a = names.index("minecraft:air")
            w = names.index("minecraft:water[level=0]") if "minecraft:water[level=0]" in names else -1
            l = names.index("minecraft:lava") if "minecraft:lava" in names else -1
            # bedrock 不出现在 noise dump；逐格分类
            for z in range(16):
                for x in range(16):
                    for y in range(max(lo, d["ymin"]), min(hi, d["ymax"])):
                        v = d["vox"][(y - d["ymin"]) * 256 + z * 16 + x]
                        if v == a:
                            air += 1
                        elif v == w or v == l:
                            fluid += 1
                        else:
                            solid += 1
        tot = air + fluid + solid
        void = air / tot if tot else 0.0
        bands[(lo, hi)] = (air, fluid, solid, tot, void)
        print(f"K-CAVE-AUDIT-java band=[{lo},{hi}) air={air} fluid={fluid} "
              f"solid={solid} bedrock={bedrock} void={void:.4f}")
    return bands


if __name__ == "__main__":
    chunks = load_window("cave_s0x7EA20B1", 2, 5, -4, -1)
    print(f"chunks loaded: {len(chunks)}")
    stats(chunks, -64, 96)
