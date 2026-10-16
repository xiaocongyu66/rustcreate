#!/usr/bin/env python3
"""par-water — #209 含水层/灌水面 逐列矩阵对拍（rust JGH1 vs 官方 JGH1）。

输入：
  OFFICIAL_DIR（默认 /root/noise-fork/tools/fork-reanchor-dumps）——
    harness 官方 jar dump：cave_s0x7EA20B1_c{cx}_{cz}.bin（16 chunks）
  RUST_DIR（默认 ./rust-dumpout）——CI artifact：rust_seed0x..._c{cx}_{cz}.bin
统计（逐 chunk × 逐列 x,z，公式对齐java_dupe 的归一化——取 name 前
"minecraft:xxx" 去 [属性]）：
  - surf：首实心 y（自 -64 向上首个 solid）
  - wtop°/wcount：首流体 y 与流体计数（water/lava）
  - per-band（[0,16)/[-16,0)…）air/fluid 计数总量
报告：逐 chunk 的带统计表 + 全窗「首实心差>8 的列数 / 流体差列数 /
      worst 列（差值与坐标）」点名，供 #209 残余分歧定位。
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import java_dupe as jd  # noqa: E402

OFF = Path("/root/noise-fork/tools/fork-reanchor-dumps")
RUST = Path(__file__).parent / "rust-dumpout"
SEED = "0x7EA20B1"
WIN = [(2 + i, -4 + j) for j in range(4) for i in range(4)]


def norm(n: str) -> str:
    return n.split("[", 1)[0]


def classify(d: dict):
    # 返回 per-column: (surf_first_solid, first_fluid, fluid_count,
    #                   band_air[10], band_fluid[10])
    names = [norm(n) for n in d["names"]]
    a = names.index("minecraft:air")
    bands = [(lo, lo + 16) for lo in range(-64, 96, 16)]
    cols = {}
    bair = [0] * 10
    bflu = [0] * 10
    for z in range(16):
        for x in range(16):
            surf = None
            ftop = None
            fcnt = 0
            for y in range(-64, 96):
                v = names[d["vox"][(y + 64) * 256 + z * 16 + x]]
                bi = next(i for i, (lo, hi) in enumerate(bands) if lo <= y < hi) if y < 96 else 9
                if v == "minecraft:air":
                    bair[bi] += 1
                elif v in ("minecraft:water", "minecraft:lava"):
                    bflu[bi] += 1
                    fcnt += 1
                    if ftop is None:
                        ftop = y
                else:
                    if surf is None:
                        surf = y
            cols[(x, z)] = (surf, ftop, fcnt)
    return cols, bair, bflu


def find(off_dir: Path, rust_dir: Path, cx: int, cz: int):
    o = None
    for p in off_dir.glob(f"*c{cx}_{cz}.bin"):
        o = p
    r = None
    for p in rust_dir.glob(f"*c{cx}_{cz}.bin"):
        r = p
    return o, r


def main():
    tot_worst = []
    print("== par-water 逐列矩阵（首实心/首流体/流体计数对拍）==")
    for cx, cz in WIN:
        o, r = find(OFF, RUST, cx, cz)
        if o is None or r is None:
            print(f"c{cx}_{cz}: MISS off={o} rust={r}")
            continue
        do, dr = jd.parse_jgh1(o), jd.parse_jgh1(r)
        co, ao, fo = classify(do)
        cr, ar, fr = classify(dr)
        dsurf = dfl = 0
        worst = None
        for k in co:
            so, ft, fc = co[k], co[k][1], co[k][2]
            sr, frt, frc = cr[k], cr[k][1], cr[k][2]
            if so is not None and sr is not None and abs(so - sr) > 8:
                dsurf += 1
                dv = abs(so - sr)
                if worst is None or dv > worst[0]:
                    worst = (dv, k, so, sr)
            fto = ft if ft is not None else 999
            ftr = frt if frt is not None else 999
            if abs(fto - ftr) > 8:
                dfl += 1
        print(f"c{cx}_{cz}: surf_diff(>8)={dsurf} fluidtop_diff(>8)={dfl} "
              f"band_air[0,16) off={ao[4]} rust={ar[4]} "
              f"band_fluid[0,16) off={fo[4]} rust={fr[4]} "
              f"worst={worst}")
        if worst:
            tot_worst.append((worst, cx, cz))
    if tot_worst:
        tot_worst.sort(reverse=True)
        print("\n== 全窗 worst 列 top5 ==")
        for (dv, (x, z), so, sr), cx, cz in tot_worst[:5]:
            print(f"c{cx}_{cz}（wx={cx*16+x}, wz={cz*16+z}）surf off={so} rust={sr} Δ={dv}")


if __name__ == "__main__":
    main()
