#!/usr/bin/env python3
"""reanchor2 — 重锚波② 官方矩阵提取器（十四红重锚的 26.1 对拍数字来源）。

官方翼：tools/fork-reanchor-dumps/（fork-reanchor 波① 产出的官方 26.1 jar
JGH1 dump，java_dupe.parse_jgh1 直读）。
rust 翼：tools/java-harness/rust-dumpout/（CI run 38825194406 @30bf8585
artifact rust-dumpout，与 HEAD 逐字节一致已 cmp 验证）。

输出三段官方矩阵：
  A) cave 洞带 air/fluid 全带官方值 vs rust（16 chunks (2..5)×(-1..-4)，
     = cave_audit_stats 同窗；官方侧恒无流体 dump，air/solid 面）。
  B) deep 列级 best_solid_run（y∈[-64,8]，<24 即坏列）官方值（seeds
     {20261013,1,777,0xDEADBEEF,42} × chunks (0,0),(0,1),(1,0),(1,1)，s42
     官方缺 c1_1，共 19 chunks 4864 列） = deep_columns 锚对拍。
  C) aq3/aq 含水层面（seed 0xA01F1750B chunk (0,0) / (-3,-3)，y -64..64，
     官方 noise 站含水层水已填）：被盖空腔 dry/wet + 海下 air/water 分面
     = aquifer 两锁锚对拍。
  D) forest noise 站 dump（seed 132784305 chunks (15,-31)/(17,-33)）地表
     面参考 = 树窗重锚选点依据。
"""
import sys
from pathlib import Path

sys.path.insert(0, "/root/java-harness/tools")
import java_dupe as jd  # noqa: E402

OFF = Path("/root/noise-fork/tools/fork-reanchor-dumps")
RUST = Path("/root/noise-fork/tools/java-harness/rust-dumpout")
BANDS = [(lo, lo + 16) for lo in range(-64, 96, 16)]


def band_index(y):
    for i, (lo, hi) in enumerate(BANDS):
        if lo <= y < hi:
            return i
    return None


def band_stats(d, want_names=None):
    """per-band air/fluid/solid counts"""
    names = [n.split("[", 1)[0] for n in d["names"]]
    air = [0] * 10
    flu = [0] * 10
    sol = [0] * 10
    ymin = d["ymin"]
    ymax = d["ymax"]
    for y in range(ymin, ymax):
        bi = band_index(y)
        if bi is None:
            continue
        base = (y - ymin) * 256
        for v in d["vox"][base:base + 256]:
            n = names[v]
            if n == "minecraft:air":
                air[bi] += 1
            elif n in ("minecraft:water", "minecraft:lava"):
                flu[bi] += 1
            else:
                sol[bi] += 1
    return air, flu, sol


def part_a_cave():
    print("== A) cave 16-chunk 带矩阵（cave_audit 同窗）==")
    off_chunks, rust_chunks = [], []
    for i in range(4):
        for j in range(4):
            cx, cz = 2 + i, -4 + j
            o = next(OFF.glob(f"*_c{cx}_{cz}.bin"), None)
            r = next(RUST.glob(f"*_{cx}_{cz}.bin"), None)
            assert o and r, f"miss c{cx}_{cz}"
            off_chunks.append(jd.parse_jgh1(o))
            rust_chunks.append(jd.parse_jgh1(r))
    tot_o = [air for air in [0] * 10], [0] * 10, [0] * 10
    sums = [[0, 0, 0] for _ in range(10)]  # off_air, rust_air, rust_fluid
    for d in off_chunks:
        air, flu, sol = band_stats(d)
        for i in range(10):
            sums[i][0] += air[i]
    for d in rust_chunks:
        air, flu, sol = band_stats(d)
        for i in range(10):
            sums[i][1] += air[i]
            sums[i][2] += flu[i]
    print(f"{'band':>12} {'off_air':>8} {'rust_air':>8} {'rust_fluid':>10} "
          f"{'off_void':>9} {'rust_void':>9}")
    for i, (lo, hi) in enumerate(BANDS):
        oa, ra, rf = sums[i]
        ovt = oa + sums[i][2] + 65536 - 0  # solid: per-chunk 16*16*16-per-band
        solid_per_chunk = 256 * 16
        o_solid = 16 * solid_per_chunk - oa - 0  # official has no fluid cells
        r_solid = 16 * solid_per_chunk - ra - rf
        print(f"[{lo:>4},{hi:>4}) {oa:>8} {ra:>8} {rf:>10} "
              f"{oa/(oa+o_solid):>9.4f} {ra/(ra+rf+r_solid):>9.4f}")


def best_seg_col(d, y_lo=-64, y_hi=8, min_seg=24):
    """per-column longest solid run."""
    names = [n.split("[", 1)[0] for n in d["names"]]
    ymin = d["ymin"]
    bad = 0
    runs = []
    for z in range(16):
        for x in range(16):
            seg = best = 0
            for y in range(y_lo, y_hi + 1):
                n = names[d["vox"][(y - ymin) * 256 + z * 16 + x]]
                if n != "minecraft:air":
                    seg += 1
                    if seg > best:
                        best = seg
                else:
                    seg = 0
            if best < min_seg:
                bad += 1
            runs.append(best)
    return bad, runs


def part_b_deep():
    print("== B) deep 列级 best_solid_run<24 官方对拍（y∈[-64,8]）==")
    total = bad = 0
    per = []
    for seed in [20261013, 1, 777, 0xDEADBEEF, 42]:
        for cx in (0, 1):
            for cz in (0, 1):
                p = OFF / f"deep_s{seed}_c{cx}_{cz}.bin"
                if not p.exists():
                    print(f"  deep_s{seed}_c{cx}_{cz}: 官方无 dump（rust 侧样本存在）")
                    continue
                d = jd.parse_jgh1(p)
                b, runs = best_seg_col(d)
                total += 256
                bad += b
                per.append((seed, cx, cz, b))
    for seed, cx, cz, b in per:
        if b:
            print(f"  seed={seed} c({cx},{cz}) bad={b}/256")
    print(f"  OFFICIAL bad_cols={bad}/{total} = {bad/total:.4f}")


def part_c_aquifer():
    print("== C) aq3/aq 官方含水层面（seed 0xA01F1750B）==")
    for name in ("aq3_s0xA01F1750B_c0_0.bin", "aq_A01F1750B_c-3-3.bin"):
        p = OFF / name
        d = jd.parse_jgh1(p)
        names = [n.split("[", 1)[0] for n in d["names"]]
        ymin = d["ymin"]
        lo, hi = -59, 62  # rust scan_pockets 口径
        # 被盖空腔：非固体 run（air+water）上下皆固体，取 [lo,62]
        dry = wet = 0
        for z in range(16):
            for x in range(16):
                col = [names[d["vox"][(y - ymin) * 256 + z * 16 + x]]
                       for y in range(ymin, d["ymax"])]
                y = max(lo, ymin)
                while y <= hi:
                    n = col[y - ymin]
                    if n not in ("minecraft:air", "minecraft:water"):
                        y += 1
                        continue
                    y0 = y
                    while y <= hi and col[y - ymin] in ("minecraft:air", "minecraft:water"):
                        y += 1
                    y1 = y
                    # 上下界需固体（y0-1 与 y1）
                    below = col[y0 - 1 - ymin] if y0 - 1 >= ymin else "minecraft:air"
                    above = col[y1 - ymin] if y1 <= hi else "minecraft:air"
                    if below not in ("minecraft:air", "minecraft:water") and \
                       above not in ("minecraft:air", "minecraft:water"):
                        for py in range(max(y0, lo), y1):
                            n2 = col[py - ymin]
                            if n2 == "minecraft:air":
                                dry += 1
                            elif n2 == "minecraft:water":
                                wet += 1
        cav = dry + wet
        print(f"  {name}: capped-cav dry={dry} wet={wet} "
              f"dry_share={dry/cav:.3f} n={cav}")
        # 海下三态分面（y∈[-59,63)）
        air = water = lava = solid = 0
        for z in range(16):
            for x in range(16):
                for y in range(lo, 63):
                    n = names[d["vox"][(y - ymin) * 256 + z * 16 + x]]
                    if n == "minecraft:air":
                        air += 1
                    elif n == "minecraft:water" or n == "minecraft:lava":
                        water += 1
                    else:
                        solid += 1
        tot = air + water + lava
        print(f"  {name}: below-sea air={air} fluid={water+lava} solid={solid} "
              f"air_share={air/tot:.3f} (共 {tot} 格)")


def part_d_forest():
    print("== D) forest noise 站官方 dump 地表面（seed 132784305，树窗选点）==")
    for name in ("forest_s132784305_c15 -31_.bin", "forest_s132784305_c17 -33_.bin"):
        p = OFF / name
        if not p.exists():
            print(f"  {name}: 缺文件")
            continue
        d = jd.parse_jgh1(p)
        names = [n.split("[", 1)[0] for n in d["names"]]
        ymin = d["ymin"]
        tops = {}
        hists = [0] * len(names)
        for z in range(16):
            for x in range(16):
                top = None
                for y in range(-64, 96):
                    v = d["vox"][(y - ymin) * 256 + z * 16 + x]
                    hists[v] += 0
                    if names[v] != "minecraft:air" and top is None:
                        top = y
                tops[(x, z)] = top
        from collections import Counter
        c = Counter(tops.values())
        print(f"  {name}: first-solid top 分布（min y 起计）: min={min(c)} "
              f"max={max(c)} 众数={c.most_common(3)}")
        for v, cnt in Counter(d["vox"]).most_common(6):
            print(f"    {names[v]}: {cnt}")


if __name__ == "__main__":
    part_a_cave()
    part_b_deep()
    part_c_aquifer()
    part_d_forest()
