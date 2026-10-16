#!/usr/bin/env python3
"""biome-window-scan — 树窗重锚的 python 侧选区工具（#207 fork 链切换后）。

背景：seed 链切原版 fork 链（841b562a）后气候窗整体移动，
trees_wiring.rs / trees_cross_chunk.rs 的旧 Forest/Ocean/Desert 窗落空转
（CI run 38650650041：Forest 窗 (x=-336,z=384) 实测 Ocean、Ocean 窗
(x=-384,z=-384) 实测 Beach）。本工具按「换取样区、不改判据」纪律
（tools/fork-chain-mirror-report.md §7）用 python 复刻
`vanilla::biome_at` 的完整判据链选新窗：

  气候通道（bit-exact，java_dupe 复刻 × rust_from_vanilla_name 同
  fork-chain-mirror.py 360/360 位型证据链）
    → ClimateSampler::sample（climate.rs SHIFT 编排）
    → biome_from_sample（mod.rs；TargetPoint 量化 ×10000）
    → biomes::pick_biome（span 平铺点表 + fitness 严格小于序）

树密度预估同源复刻 tree_pass.rs::tree_spots 的池抽取序
（hash01 通道 1..4 + profile 密度门；立地门 column_real_top 依赖密度场，
python 不复刻——全 Forest 且地表在带内的列上 grass 命中是 surface_rules
的结构性推论，风险由 CI 的窗口断言兜底）。

用法: python3 tools/fork-reanchor/biome_window_scan.py [--seed N ...]
只读；无网络；无 cargo。
"""

import argparse
import struct
import sys
from pathlib import Path

sys.path.insert(0, "/root/java-harness/tools")
import java_dupe as jd  # noqa: E402

M64 = (1 << 64) - 1

# ------------------------------------------------------- rust 侧常量镜像
TREE_DOMAIN = 0x7BEE_5501
CELL = 5
# tree_pass.rs profile()（density = E[count]/10.24，无树群系 None）。
FOREST_DENSITY = 10.1 / 10.24

# climate.rs 振幅表（逐字抄，含尾部零填充；同 fork-chain-mirror.py CHANNELS）。
CHANNELS = {
    "offset": (-3, [1.0, 1.0, 1.0, 0.0]),
    "continentalness": (-9, [1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0]),
    "erosion": (-9, [1.0, 1.0, 0.0, 1.0, 1.0]),
    "ridge": (-7, [1.0, 2.0, 1.0, 0.0, 0.0, 0.0]),
    "temperature": (-10, [1.5, 0.0, 1.0, 0.0, 0.0, 0.0]),
    "vegetation": (-8, [1.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
}
XS = 0.25


def splitmix64(x: int) -> int:
    x = (x + 0x9E3779B97F4A7C15) & M64
    z = x
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
    return z ^ (z >> 31)


def channel_seed(world_seed: int, domain: int) -> int:
    """mod.rs channel_seed：splitmix64(seed ^ domain) as i64（按位重解释）。"""
    return splitmix64((world_seed ^ domain) & M64)


def hash01(seed: int, x: int, y: int, z: int) -> float:
    """mod.rs hash01 → f32（(h>>40)/2^24）。"""
    h = splitmix64(
        (seed & M64)
        ^ ((x & M64) * 0x9E3779B1 & M64)
        ^ ((y & M64) * 0x85EBCA77 & M64)
        ^ ((z & M64) * 0xC2B2AE3D & M64)
    )
    return struct.unpack("<f", struct.pack("<f", (h >> 40) * (1.0 / 16777216.0)))[0]


def peaks_and_valleys(r: float) -> float:
    return -3.0 * (abs(abs(r) - 2.0 / 3.0) - 1.0 / 3.0)


# ------------------------------------------------------- 噪声通道（fork 链）
def build_channels(world_seed: int) -> dict:
    root = jd.XoroshiroRandomSource.from_seed(world_seed).fork_positional()
    out = {}
    for name, (fo, amps) in CHANNELS.items():
        rng = jd.factory_from_hash(root, name)
        out[name] = jd.NormalNoiseVanilla(rng, fo, amps)
    return out


class Climate:
    """climate.rs ClimateSampler::sample 的 python 镜像。"""

    def __init__(self, world_seed: int):
        self.n = build_channels(world_seed)

    def sample(self, bx: int, bz: int):
        n = self.n
        fbx, fbz = float(bx), float(bz)
        sx = n["offset"].get_value(fbx * XS, 0.0, fbz * XS) * 4.0
        sz = n["offset"].get_value(fbz * XS, fbx * XS, 0.0) * 4.0
        x, z = fbx * XS + sx, fbz * XS + sz
        return (
            n["continentalness"].get_value(x, 0.0, z),
            n["erosion"].get_value(x, 0.0, z),
            n["ridge"].get_value(x, 0.0, z),
            n["temperature"].get_value(x, 0.0, z),
            n["vegetation"].get_value(x, 0.0, z),
        )


# ------------------------------------------------------- biome 分类（biomes.rs）
Q = 10_000


def q(v: float) -> int:
    """(v * Q as f64) as i64 —— Rust 截断向零语义。"""
    return int(v * float(Q))


class Param:
    __slots__ = ("lo", "hi")

    def __init__(self, lo: float, hi: float):
        self.lo = int(lo * Q)
        self.hi = int(hi * Q)

    def distance(self, t: int) -> int:
        above = t - self.hi
        if above > 0:
            return above
        below = self.lo - t
        return below if below > 0 else 0


FULL = Param(-1.0, 1.0)
UNFROZEN = Param(-0.45, 1.0)
RIVER_WEIRD = Param(-0.05, 0.05)
WEIRD_OUTSIDE = [Param(-1.0, -0.05), Param(0.05, 1.0)]
T_SPANS = [(-1.0, -0.45), (-0.45, -0.15), (-0.15, 0.2), (0.2, 0.55), (0.55, 1.0)]
H_SPANS = [(-1.0, -0.35), (-0.35, -0.1), (-0.1, 0.1), (0.1, 0.3), (0.3, 1.0)]

# MIDDLE 矩阵（10 群系收窄；None = 未注册）。
MIDDLE = [
    ["SnowyPlains", "SnowyPlains", "SnowyPlains", None, "Taiga"],
    ["Plains", "Plains", "Forest", "Taiga", None],
    [None, "Plains", "Forest", "BirchForest", None],
    ["Savanna", "Savanna", "Forest", None, None],
    ["Desert", "Desert", "Desert", "Desert", None],
]


def build_point_table():
    """biome_point_table 镜像：注册序即平局序（ocean → river → beach → 内陆）。"""
    v = []

    def add(t, h, c, e, w, d, biome):
        # climate_point：6 参数 + offset=100（surface 类全表同值）。
        v.append(((t, h, c, e, Param(d, d), w, 100), biome))

    for (t_lo, t_hi) in T_SPANS:
        for (c_lo, c_hi) in [(-1.05, -0.455), (-0.455, -0.19)]:
            for d in (0.0, 1.0):
                add(Param(t_lo, t_hi), FULL, Param(c_lo, c_hi), FULL, FULL, d, "Ocean")
    for d in (0.0, 1.0):
        add(UNFROZEN, FULL, Param(-0.19, 1.0), FULL, RIVER_WEIRD, d, "River")
    for (t_lo, t_hi) in T_SPANS[1:4]:
        for w in WEIRD_OUTSIDE:
            for d in (0.0, 1.0):
                add(Param(t_lo, t_hi), FULL, Param(-0.19, -0.11), FULL, w, d, "Beach")
    for ti, row in enumerate(MIDDLE):
        for hi, cell in enumerate(row):
            if cell is None:
                continue
            for w in WEIRD_OUTSIDE:
                for d in (0.0, 1.0):
                    add(
                        Param(*T_SPANS[ti]),
                        Param(*H_SPANS[hi]),
                        Param(-0.11, 1.0),
                        FULL,
                        w,
                        d,
                        cell,
                    )
    return v


TABLE = build_point_table()


def pick_biome(continents: float, erosion: float, ridges: float, temperature: float,
               vegetation: float) -> str:
    t = (
        q(temperature),
        q(vegetation),
        q(continents),
        q(erosion),
        0,
        q(ridges),
    )
    best = None
    want = None
    for (point, biome) in TABLE:
        tpar, hpar, cpar, epar, dpar, wpar, off = point
        f = 0
        for par, val in ((tpar, t[0]), (hpar, t[1]), (cpar, t[2]), (epar, t[3]),
                         (dpar, t[4]), (wpar, t[5])):
            d = par.distance(val)
            f += d * d
        f += off * off
        if best is None or f < best:
            best = f
            want = biome
    return want


# ------------------------------------------------------- 窗口扫描
def chunk_column_biomes(cl: Climate, cx: int, cz: int) -> list:
    out = []
    for z in range(16):
        for x in range(16):
            out.append(pick_biome(*cl.sample(cx * 16 + x, cz * 16 + z)))
    return out


def est_trees_in_chunk(cl: Climate, seed: int, cx: int, cz: int):
    """tree_pass.rs 树池的 python 预估：biome 门 + gate 门（立地门不复刻）。"""
    ts = channel_seed(seed, TREE_DOMAIN)
    base_x, base_z = cx * 16, cz * 16
    c0x = (base_x - 6) // CELL
    c1x = (base_x + 15 + 6) // CELL
    c0z = (base_z - 6) // CELL
    c1z = (base_z + 15 + 6) // CELL
    n_in = 0
    for cell_z in range(c0z, c1z + 1):
        for cell_x in range(c0x, c1x + 1):
            jx = hash01(ts, cell_x, cell_z, 1)
            jz = hash01(ts, cell_x, cell_z, 2)
            wx = cell_x * CELL + int(jx * float(CELL))
            wz = cell_z * CELL + int(jz * float(CELL))
            b = pick_biome(*cl.sample(wx, wz))
            if b not in ("Forest", "BirchForest"):
                continue
            gate = hash01(ts, cell_x, cell_z, 3)
            if gate >= FOREST_DENSITY:
                continue
            if base_x <= wx < base_x + 16 and base_z <= wz < base_z + 16:
                n_in += 1
    return n_in


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, nargs="+", default=[132784305, 99])
    ap.add_argument("--range", type=int, default=64, help="chunk 扫描半径")
    args = ap.parse_args()

    for seed in args.seed:
        cl = Climate(seed)
        # 粗扫：chunk 中心列群系 → 找 Forest/Ocean/Desert 候选区。
        forest, ocean, desert = [], [], []
        for cz in range(-args.range, args.range):
            for cx in range(-args.range, args.range):
                b = pick_biome(*cl.sample(cx * 16 + 8, cz * 16 + 8))
                if b == "Forest":
                    forest.append((cx, cz))
                elif b == "Ocean":
                    ocean.append((cx, cz))
                elif b == "Desert":
                    desert.append((cx, cz))
        print(f"seed={seed} 粗扫 ±{args.range} chunk: forest={len(forest)} "
              f"ocean={len(ocean)} desert={len(desert)}")

        def fully(biome, cands, size=1):
            """找 size×size chunk 全列 biome 的窗口。"""
            cand = set(cands)
            for (cx, cz) in cands:
                ok = True
                for dz in range(size):
                    for dx in range(size):
                        if (cx + dx, cz + dz) not in cand:
                            ok = False
                            break
                    if not ok:
                        break
                if ok:
                    # 逐列验证
                    good = True
                    for dz in range(size):
                        for dx in range(size):
                            cols = chunk_column_biomes(cl, cx + dx, cz + dz)
                            if any(c != biome for c in cols):
                                good = False
                                break
                        if not good:
                            break
                    if good:
                        return (cx, cz)
            return None

        f1 = fully("Forest", forest, 1)
        print(f"  全列 Forest 单块窗: {f1}")
        f3 = fully("Forest", forest, 3)
        print(f"  3×3 全列 Forest 区: {f3}")
        o1 = fully("Ocean", ocean, 1)
        print(f"  全列 Ocean 单块窗: {o1}")
        d1 = fully("Desert", desert, 1)
        print(f"  全列 Desert 单块窗: {d1}")
        if f1:
            cx, cz = f1
            print(f"  Forest 中心块树预估(干在块内): {est_trees_in_chunk(cl, seed, cx, cz)}")
        if f3:
            cx, cz = f3
            print(f"  3×3 区中心块树预估: {est_trees_in_chunk(cl, seed, cx + 1, cz + 1)}")


if __name__ == "__main__":
    main()
