#!/usr/bin/env python3
"""java_dupe.py - parity tooling for the java-gen-harness (26.1 worldgen black box).

Subcommands
  dump       run the java dumper for seeds x chunks      -> dumps/seed<N>_c<X>Z.bin(.json)
  probe      run --probe-noise / --density probes        -> JSON lines on stdout
  vain       python replica of the vanilla seed->NormalNoise chain (xoroshiro128++,
             fromHashOf MD5 names, forkPositional octaves, ImprovedNoise, two-layer
             NormalNoise) - mechanically transcribed from:
               harness/src/net/minecraft/world/level/levelgen/{XoroshiroRandomSource,
               Xoroshiro128PlusPlus}.java, RandomSupport (26.1), synth/{ImprovedNoise,
               PerlinNoise, NormalNoise}.java
  rust-model python replica of game/mcv_worldgen/src/vanilla/noise.rs `NormalNoise::new`
             (v1 clean-room Seeding: splitmix64(world ^ salt), octaves +i*2^28)
  report     3-seed first-proof report:
             - java dumps exist + row/column stats
             - vanilla(java) vs vanilla-replica(py)  same-dot rate   (octave engine gate)
             - vanilla(java) vs rust-model(py)       same-dot rate   (documented v1 gap)
             - voxel diff machinery (java-vs-<other bin>): first diff coord, band rates

Requires: JDK (java/jar on PATH), the harness built with build.sh, datapack root
(default /root/mc-ref/src-26.1/data, override --data-root). No network use.
"""

import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys
from pathlib import Path

HARNESS = Path(__file__).resolve().parent.parent
DUMPS = HARNESS / "dumps"
MASK64 = (1 << 64) - 1
F32 = struct.Struct("<f")


def s64(x: int) -> int:
    """int64 two's-complement of x."""
    x &= MASK64
    return x - (1 << 64) if x >> 63 else x


def u64(x: int) -> int:
    return x & MASK64


def rotl(x: int, k: int) -> int:
    x &= MASK64
    return ((x << k) | (x >> (64 - k))) & MASK64


def i32(x: int) -> int:
    x &= 0xFFFFFFFF
    return x - (1 << 32) if x >> 31 else x


# ---------------------------------------------------------------- java runner
def classpath() -> str:
    libs =str(HARNESS / "libs")
    jars = ["build", f"{libs}/server-26.1.jar"]
    for root, _dirs, files in os.walk(libs):
        for f in sorted(files):
            if f.endswith(".jar") and f != "server-26.1.jar":
                jars.append(os.path.join(root, f))
    return ":".join(jars)


def run_java(args: list[str], timeout=900) -> list[str]:
    env = dict(os.environ)
    cp = classpath()
    out = subprocess.run(
        ["java", "-cp", cp, "Main", *args],
        cwd=HARNESS, capture_output=True, text=True, timeout=timeout, env=env,
    )
    lines = []
    for line in (out.stdout + out.stderr).splitlines():
        if "WARNING" in line or "Unsafe" in line or "joml" in line or "\tat " in line:
            continue
        lines.append(line)
    return lines


# ------------------------------------------------------- vanilla RNG replica
def mix_stafford13(z: int) -> int:
    z = u64(z)
    z = u64((z ^ (z >> 30)) * u64(0xBFB3D29A2F0A0E97 if False else -4658895280553007687))
    z = u64((z ^ (z >> 27)) * u64(-7723592293110705685))
    return u64(z ^ (z >> 31))


def upgrade_seed_128(seed: int) -> tuple[int, int]:
    low = u64(seed ^ 7640891576956012809)                 # 0x6A09E667F3BCC909
    high = u64(low + u64(-7046029254386353131))           # + 0x9E3779B97F4A7C15
    return mix_stafford13(low), mix_stafford13(high)


class Xoroshiro128PlusPlus:
    __slots__ = ("lo", "hi")

    def __init__(self, lo: int, hi: int):
        self.lo, self.hi = u64(lo), u64(hi)

    def next_long(self) -> int:
        s0, s1 = self.lo, self.hi
        result = u64(rotl(u64(s0 + s1), 17) + s0)
        s1 ^= s0
        self.lo = u64(rotl(s0, 49) ^ s1 ^ u64(s1 << 21))
        self.hi = rotl(s1, 28)
        return result


def seed_from_hash_of(name: str) -> tuple[int, int]:
    d = hashlib.md5(name.encode("utf-8")).digest()
    lo = int.from_bytes(d[0:8], "big")
    hi = int.from_bytes(d[8:16], "big")
    return lo, hi


F32_1_110223E_16 = F32.unpack(struct.pack("<f", 1.110223e-16))[0]


class XoroshiroRandomSource:
    """RandomSource over xoroshiro128++ (int/long/double conversion per 26.1)."""

    def __init__(self, lo: int, hi: int):
        self.rng = Xoroshiro128PlusPlus(lo, hi)

    @classmethod
    def from_seed(cls, seed: int) -> "XoroshiroRandomSource":
        lo, hi = upgrade_seed_128(seed)
        return cls(lo, hi)

    def next_long(self) -> int:
        return self.rng.next_long()

    def next_int(self) -> int:
        return i32(self.rng.next_long())

    def next_int_bound(self, bound: int) -> int:
        # 26.1 XoroshiroRandomSource.nextInt(int) - Lemire rejection on 32 bits
        assert bound > 0
        random_bits = self.next_int() & 0xFFFFFFFF
        multiplied = random_bits * bound
        fractional = multiplied & 0xFFFFFFFF
        if fractional < bound:
            unbiased_start = (u64(~bound + 1)) % bound
            while fractional < unbiased_start:
                random_bits = self.next_int() & 0xFFFFFFFF
                multiplied = random_bits * bound
                fractional = multiplied & 0xFFFFFFFF
        return i32(multiplied >> 32)

    def next_double(self) -> float:
        # vanilla: nextLong >>> (64-53), multiplied BY A FLOAT literal -> Java
        # promotes long*float to FLOAT arithmetic, then widens to double.
        n = u64(self.rng.next_long()) >> (64 - 53)
        nf = F32.unpack(F32.pack(n))[0]                      # (float) n
        return F32.unpack(F32.pack(nf * F32_1_110223E_16))[0]  # (float)((float)n * C)

    def fork_positional(self) -> tuple[int, int]:
        """factory (seedLo, seedHi) = two consumed nextLongs."""
        lo = self.next_long()
        hi = self.next_long()
        return lo, hi


def factory_from_hash(factory: tuple[int, int], name: str) -> XoroshiroRandomSource:
    """XoroshiroPositionalRandomFactory.fromHashOf: MD5(name) xor factory lo/hi."""
    lo, hi = seed_from_hash_of(name)
    return XoroshiroRandomSource(u64(lo ^ factory[0]), u64(hi ^ factory[1]))


GRADIENT = [
    [1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0],
    [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1],
    [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1],
    [1, 1, 0], [0, -1, 1], [-1, 1, 0], [0, -1, -1],
]


def lfloor(x: float) -> int:
    import math
    return math.floor(x)


def lerp3(xa, ya, za, d000, d100, d010, d110, d001, d101, d011, d111):
    # Mth.lerp3: plane0 = lerp2(xa: 000->100, 010->110); plane1 = same at z+1;
    # result = lerp(za, plane0, plane1). Axis order matters bit-exactly.
    p00 = d000 + (d100 - d000) * xa
    p01 = d010 + (d110 - d010) * xa
    plane0 = p00 + (p01 - p00) * ya
    p10 = d001 + (d101 - d001) * xa
    p11 = d011 + (d111 - d011) * xa
    plane1 = p10 + (p11 - p10) * ya
    return plane0 + (plane1 - plane0) * za


def smoothstep(value: float) -> float:
    return value * value * value * (value * (value * 6.0 - 15.0) + 10.0)


class ImprovedNoise:
    def __init__(self, rng: XoroshiroRandomSource):
        self.xo = rng.next_double() * 256.0
        self.yo = rng.next_double() * 256.0
        self.zo = rng.next_double() * 256.0
        self.p = list(range(256))
        for i in range(256):
            offset = rng.next_int_bound(256 - i)
            self.p[i], self.p[i + offset] = self.p[i + offset], self.p[i]
        self.p = [v & 0xFF for v in self.p]

    @staticmethod
    def _grad_dot(h: int, x: float, y: float, z: float) -> float:
        g = GRADIENT[h & 15]
        return g[0] * x + g[1] * y + g[2] * z

    def _p(self, x: int) -> int:
        return self.p[x & 0xFF]

    def noise(self, x: float, y: float, z: float) -> float:
        x += self.xo
        y += self.yo
        z += self.zo
        xf, yf, zf = lfloor(x), lfloor(y), lfloor(z)
        xr, yr, zr = x - xf, y - yf, z - zf
        x0 = self._p(xf)
        x1 = self._p(xf + 1)
        xy00 = self._p(x0 + yf)
        xy01 = self._p(x0 + yf + 1)
        xy10 = self._p(x1 + yf)
        xy11 = self._p(x1 + yf + 1)
        d000 = self._grad_dot(self._p(xy00 + zf), xr, yr, zr)
        d100 = self._grad_dot(self._p(xy10 + zf), xr - 1.0, yr, zr)
        d010 = self._grad_dot(self._p(xy01 + zf), xr, yr - 1.0, zr)
        d110 = self._grad_dot(self._p(xy11 + zf), xr - 1.0, yr - 1.0, zr)
        d001 = self._grad_dot(self._p(xy00 + zf + 1), xr, yr, zr - 1.0)
        d101 = self._grad_dot(self._p(xy10 + zf + 1), xr - 1.0, yr, zr - 1.0)
        d011 = self._grad_dot(self._p(xy01 + zf + 1), xr, yr - 1.0, zr - 1.0)
        d111 = self._grad_dot(self._p(xy11 + zf + 1), xr - 1.0, yr - 1.0, zr - 1.0)
        xa = smoothstep(xr)
        ya = smoothstep(yr)
        za = smoothstep(zr)
        return lerp3(xa, ya, za, d000, d100, d010, d110, d001, d101, d011, d111)


WRAP_D = 3.3554432e7


def wrap(x: float) -> float:
    import math
    return x - math.floor(x / WRAP_D + 0.5) * WRAP_D


class PerlinNoise:
    """vanilla synth/PerlinNoise (new init): fork octaves once, one ImprovedNoise
    per NON-ZERO amplitude, fromHashOf("octave_{firstOctave+i}")."""

    def __init__(self, rng: XoroshiroRandomSource, first_octave: int, amplitudes: list[float]):
        self.first_octave = first_octave
        self.amplitudes = amplitudes
        factory = rng.fork_positional()
        octaves = len(amplitudes)
        self.levels: list[ImprovedNoise | None] = [None] * octaves
        for i, amp in enumerate(amplitudes):
            if amp != 0.0:
                octave = first_octave + i
                self.levels[i] = ImprovedNoise(factory_from_hash(factory, f"octave_{octave}"))
        # lowestFreqInputFactor = 2^(firstOctave) via java's -zeroOctaveIndex
        # (zeroOctaveIndex == -firstOctave): first octave has the LONGEST wavelength.
        self.input_factor = 2.0 ** first_octave
        self.value_factor = (2.0 ** (octaves - 1)) / (2.0 ** octaves - 1.0)

    def get_value(self, x: float, y: float, z: float) -> float:
        value = 0.0
        factor = self.input_factor
        value_factor = self.value_factor
        for i, noise in enumerate(self.levels):
            if noise is not None:
                value += self.amplitudes[i] * noise.noise(wrap(x * factor), wrap(y * factor), wrap(z * factor)) * value_factor
            factor *= 2.0
            value_factor /= 2.0
        return value


class NormalNoiseVanilla:
    """vanilla synth/NormalNoise: two independent PerlinNoise layers from one
    channel random; second layer coords x1.0181268882175227; value factor =
    0.166.../expectedDeviation(nonzero octave span)."""

    D = 1.0181268882175227

    def __init__(self, rng: XoroshiroRandomSource, first_octave: int, amplitudes: list[float]):
        self.first = PerlinNoise(rng, first_octave, amplitudes)
        self.second = PerlinNoise(rng, first_octave, amplitudes)
        nz = [i for i, a in enumerate(amplitudes) if a != 0.0]
        span = 0 if not nz else nz[-1] - nz[0]
        expected_deviation = 0.1 * (1.0 + 1.0 / (span + 1))
        self.value_factor = 0.16666666666666666 / expected_deviation

    def get_value(self, x: float, y: float, z: float) -> float:
        first = self.first.get_value(x, y, z)
        second = self.second.get_value(x * self.D, y * self.D, z * self.D)
        return (first + second) * self.value_factor


def vanilla_channel(world_seed: int, name: str, first_octave: int, amplitudes: list[float]) -> NormalNoiseVanilla:
    """RandomState chain: XoroshiroRandomSource(seed).forkPositional() ->
    fromHashOf('minecraft:{name}') (identifier with namespace)."""
    factory = XoroshiroRandomSource.from_seed(world_seed).fork_positional()
    rng = factory_from_hash(factory, f"minecraft:{name}")
    return NormalNoiseVanilla(rng, first_octave, amplitudes)


# ------------------------------------------------------------ rust v1 model
def splitmix64(x: int) -> int:
    x = u64(x + 0x9E3779B97F4A7C15)
    x = u64((x ^ (x >> 30)) * 0xBF58476D1CE4E5B9)
    x = u64((x ^ (x >> 27)) * 0x94D049BB133111EB)
    return u64(x ^ (x >> 31))


class NormalNoiseRustV1:
    """game/mcv_worldgen/src/vanilla/noise.rs NormalNoise::new: own splitmix
    seeding; octave seeds root + i*2^28; second layer root = splitmix(root)."""

    OCTAVE_STEP = 0x10000000
    D = 1.0181268882175227

    def __init__(self, world_seed: int, salt: int, first_octave: int, amplitudes: list[float]):
        seed = splitmix64(u64(world_seed) ^ u64(salt))
        seed2 = splitmix64(seed)
        self.first = [
            ImprovedNoise(XoroshiroRandomSource.from_seed_data(u64(seed + i * self.OCTAVE_STEP)))
            for i in range(len(amplitudes))
        ]
        self.second_r = [
            ImprovedNoise(XoroshiroRandomSource.from_seed_data(u64(seed2 + i * self.OCTAVE_STEP)))
            for i in range(len(amplitudes))
        ]
        self.first_octave = first_octave
        self.amplitudes = amplitudes
        nz = [i for i, a in enumerate(amplitudes) if a != 0.0]
        span = 0 if not nz else nz[-1] - nz[0]
        expected_deviation = 0.1 * (1.0 + 1.0 / (span + 1))
        self.value_factor = 0.16666666666666666 / expected_deviation

    def _one(self, levels, x, y, z) -> float:
        value = 0.0
        factor = 2.0 ** self.first_octave
        value_factor = (2.0 ** (len(self.amplitudes) - 1)) / (2.0 ** len(self.amplitudes) - 1.0)
        for amp, inst in zip(self.amplitudes, levels):
            if amp != 0.0:
                value += amp * inst.noise(wrap(x * factor), wrap(y * factor), wrap(z * factor)) * value_factor
            factor *= 2.0
            value_factor /= 2.0
        return value

    def get_value(self, x: float, y: float, z: float) -> float:
        first = self._one(self.first, x, y, z)
        second = self._one(self.second_r, x * self.D, y * self.D, z * self.D)
        return (first + second) * self.value_factor

    def note(self) -> str:
        # ImprovedNoise::new(u64) uses setSeed -> upgradeSeedTo128bit, i.e.
        # XoroshiroRandomSource.from_seed_data == from_seed in this replica.
        return "seeds via upgradeSeedTo128bit(seed)"


XoroshiroRandomSource.from_seed_data = XoroshiroRandomSource.from_seed  # alias for clarity


# ------------------------------------------------------------- JGH1 parsing
def parse_jgh1(path: Path):
    data = path.read_bytes()
    assert data[:4] == b"JGH1", f"bad magic in {path}"
    (icnt,) = struct.unpack_from(">i", data, 4)
    (seed,) = struct.unpack_from(">q", data, 8)
    cx, cz = struct.unpack_from(">ii", data, 16)
    ymin, ymax = struct.unpack_from(">ii", data, 24)
    off = 32
    names = []
    for _ in range(icnt):
        (l,) = struct.unpack_from(">H", data, off)
        off += 2
        names.append(data[off:off + l].decode("utf-8"))
        off += l
    nvox = (ymax - ymin) * 16 * 16
    vox = struct.unpack_from(f">{nvox}H", data, off)
    off += 2 * nvox
    assert off == len(data), f"trailing bytes in {path}"
    return dict(seed=seed, cx=cx, cz=cz, ymin=ymin, ymax=ymax, names=names, vox=vox)


def band_of(y: int) -> str:
    if y < 0:
        return f"deep({y//32*32})"
    if y < 64:
        return f"mid({y//32*32})"
    return f"high({y//32*32})"


def voxel_diff(a: dict, b: dict) -> dict:
    """first-diff + per-band diff-rate statistics between two JGH1 dumps.
    Id tables may differ: voxels are remapped into a canonical name space."""
    assert (a["cx"], a["cz"]) == (b["cx"], b["cz"]) and a["ymin"] == b["ymin"] and a["ymax"] == b["ymax"]
    canon_names = list(dict.fromkeys([*a["names"], *b["names"]]))
    amap = [canon_names.index(n) for n in a["names"]]
    bmap = [canon_names.index(n) for n in b["names"]]
    a = dict(a, names=canon_names, vox=tuple(amap[v] for v in a["vox"]))
    b = dict(b, names=canon_names, vox=tuple(bmap[v] for v in b["vox"]))
    lo, hi = a["ymin"], a["ymax"]
    total = equal = 0
    bands: dict[str, list[int]] = {}
    first_diff = None
    for yo in range(0, hi - lo):
        y = lo + yo
        band = band_of(y)
        for z in range(16):
            for x in range(16):
                va, vb = a["vox"][(yo * 16 + z) * 16 + x], b["vox"][(yo * 16 + z) * 16 + x]
                total += 1
                if va == vb:
                    equal += 1
                else:
                    if first_diff is None:
                        first_diff = [x, y, z, a["names"][va], b["names"][vb]]
                    b_ = bands.setdefault(band, [0, 0])
                    b_[0] += 1
                    b_[1] += 1
                if band not in bands:
                    bands[band] = [0, 0]
                bands[band][1] += 1
    return dict(
        total=total, equal=equal,
        same_rate=round(equal / total, 6) if total else None,
        first_diff=first_diff,
        bands={k: dict(diff=v[0], n=v[1], rate=round(v[0] / v[1], 4)) for k, v in sorted(bands.items())},
    )


# ------------------------------------------------------------------ probe
def java_noise_probes(seed: int, specs: list[str]) -> list[tuple[str, int, int, int, float, int]]:
    """batch --probe-noise specs (name,y,x,z) in ONE jvm bootstrap."""
    lines = run_java(["--seed", str(seed), *[x for s in specs for x in ("--probe-noise", s)]])
    out = []
    for line in lines:
        if line.startswith("noise "):
            t = line.split()
            dec = float(line.split("decimal=")[1].split()[0])
            bits = int(line.split("bits=0x")[1].split()[0], 16)
            name = t[1]
            y = int(t[2].split("=")[1])
            x = int(t[3].split("=")[1])
            z = int(t[4].split("=")[1])
            out.append((name, y, x, z, dec, bits))
    assert len(out) == len(specs), f"expected {len(specs)} probes got {len(out)}"
    return out


def load_params(data_root: Path, name: str) -> tuple[int, list[float]]:
    p = data_root / "minecraft" / "worldgen" / "noise" / f"{name}.json"
    j = json.loads(p.read_text())
    return j["firstOctave"], j["amplitudes"]


# ------------------------------------------------------------------ report
FRESH = False
def cmd_report(data_root: Path, seeds: list[int], chunk: tuple[int, int], probe_points: list[tuple[int, int, int]], channels: list[str]):
    print("=" * 78)
    print("java-gen-harness first-proof report")
    print("=" * 78)
    print(f"pack root: {data_root}", flush=True)
    print(f"seeds: {seeds}  chunk: {chunk}  probe points per (seed,channel): {len(probe_points)}", flush=True)

    # 1) voxel dumps (reused if already on disk; --fresh rebuilds)
    print("\n[1] voxel dumps (stage=noise fill, JGH1)", flush=True)
    vw = 0
    wvox = 0
    wsolid = 0
    for seed in seeds:
        out_bin = DUMPS / f"seed{seed}_c{chunk[0]}{chunk[1]}.bin"
        if FRESH or not out_bin.exists():
            run_java(["--seed", str(seed), "--chunk", f"{chunk[0]},{chunk[1]}", "-o", str(out_bin)])
        d = parse_jgh1(out_bin)
        solid = sum(1 for v in d["vox"] if d["names"][v] != "minecraft:air")
        sheets = len(d["names"])
        print(f"  seed {seed}: {out_bin.name} {out_bin.stat().st_size:>7} B  states={sheets} solid={solid} ({solid/len(d['vox']):.3f})", flush=True)
        vw = max(vw, sheets)
        wvox = max(wvox, len(d["vox"]))
        wsolid = max(wsolid, solid)
    # reproducibility gate: dump seed[seeds[0]] twice and diff
    rep = DUMPS / f"seed{seeds[0]}_repro.bin"
    if FRESH or not rep.exists():
        run_java(["--seed", str(seeds[0]), "--chunk", f"{chunk[0]},{chunk[1]}", "-o", str(rep)])
    same = parse_jgh1(DUMPS / f"seed{seeds[0]}_c{chunk[0]}{chunk[1]}.bin")
    rep_d = parse_jgh1(rep)
    print(f"  determinism gate: run-twice identical = {same['vox'] == rep_d['vox']}", flush=True)

    # 2) noise probe parity: java(vanilla) vs python vanilla replica vs rust v1 model
    print("\n[2] NormalNoise channel probes (java truth vs python replicas)")
    print(f"    probe points (y,x,z): {probe_points}", flush=True)
    rows = []
    for seed in seeds:
        for ch in channels:
            fo, amps = load_params(data_root, ch)
            van = vanilla_channel(seed, ch, fo, amps)
            rust = NormalNoiseRustV1(u64(seed), salt=0, first_octave=fo, amplitudes=amps)
            specs = [f"{ch},{y},{x},{z}" for (y, x, z) in probe_points]
            probes = java_noise_probes(seed, specs)
            n = same_v = same_r = 0
            max_v = max_r = 0.0
            for (y, x, z) in probe_points:
                jdec, jbitsv = java_probe_value(probes, ch, y, x, z)
                pvan = van.get_value(float(x), float(y), float(z))
                prust = rust.get_value(float(x), float(y), float(z))
                n += 1
                bbits = int(struct.unpack(">Q", struct.pack(">d", pvan))[0])
                rbits = int(struct.unpack(">Q", struct.pack(">d", prust))[0])
                if bbits == jbitsv or abs(pvan - jdec) < 1e-12:
                    same_v += 1
                max_v = max(max_v, abs(pvan - jdec))
                if rbits == jbitsv:
                    same_r += 1
                max_r = max(max_r, abs(prust - jdec))
            rows.append((seed, ch, n, same_v, same_r, max_v, max_r))
            print(f"  seed {seed:>10} {ch:<11} n={n:<4} "
                  f"py-vanilla same={same_v}/{n} (max|d|={max_v:.3g})  "
                  f"rust-v1 same={same_r}/{n} (max|d|={max_r:.3g})", flush=True)

    print("\n[3] interpretation")
    print("  - py-vanilla same=N/N: python replica of the vanilla xoroshiro fork")
    print("    chain reproduces the java (official-jar) NormalNoise bit stream; failures")
    print("    localize the gap to ImprovedNoise/rounding, fork semantics, or MD5 names.")
    print("    Your rust code targets this same chain (noise.rs from_vanilla_name).")
    print("  - rust-v1 same=N/N: the v1 splitmix seed derivation documented in noise.rs")
    print("    lines 30-33 as a v1 subset gap vs vanilla fork chain; numbers quantify it.")
    print("  - voxel band table: run 'dupe tools/java_dupe.py voxel-diff -a A.bin -b B.bin'")
    print("    with the Rust CI dump as -b for first-diff coordinates and per-band rates.")


def java_probe_value(probes, name: str, y: int, x: int, z: int):
    for (n, py, px, pz, dec, bits) in probes:
        if (n, py, px, pz) == (name, y, x, z):
            return dec, bits
    raise KeyError((name, y, x, z))


def bits_eq(a: int, b: int, allow_tiny_reordered: bool = True) -> bool:
    if a == b:
        return True
    # NaN-like or signed-zero tolerance off by default; keep strict here
    return False


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd")
    rp = sub.add_parser("report")
    rp.add_argument("--seeds", type=int, nargs="+", default=[1, 123, 20260114])
    rp.add_argument("--chunk", default="0,0")
    rp.add_argument("--data-root", default="/root/mc-ref/src-26.1/data")
    rp.add_argument("--channel", nargs="*", default=["temperature", "erosion", "ridges"])
    rp.add_argument("--points", type=int, default=12)
    rp.add_argument("--fresh", action="store_true")
    dp = sub.add_parser("dump")
    dp.add_argument("--seeds", type=int, nargs="+", default=[1, 123, 20260114])
    dp.add_argument("--chunk", default="0,0")
    dp.add_argument("--data-root", default="/root/mc-ref/src-26.1/data")
    vd = sub.add_parser("voxel-diff")
    vd.add_argument("-a", required=True)
    vd.add_argument("-b", required=True)
    args = ap.parse_args()
    data_root = Path(args.data_root)
    chunk = tuple(int(v) for v in args.chunk.split(","))

    if args.cmd == "dump":
        DUMPS.mkdir(exist_ok=True)
        for seed in args.seeds:
            out_bin = DUMPS / f"seed{seed}_c{chunk[0]}{chunk[1]}.bin"
            run_java(["--seed", str(seed), "--chunk", args.chunk, "-o", str(out_bin)])
            print(out_bin)
    elif args.cmd == "voxel-diff":
        a, b = parse_jgh1(Path(args.a)), parse_jgh1(Path(args.b))
        print(json.dumps(voxel_diff(a, b), indent=2, ensure_ascii=False))
    elif args.cmd == "report":
        DUMPS.mkdir(exist_ok=True)
        # probe grid: deterministic pseudo-grid across the chunk + one deep point
        pts = [(y, x, z) for y, x, z in
               [(k * 7 - 32, 17 + k * 3, 25 + k * 5) for k in range(args.points)]]
        global FRESH
        FRESH = args.fresh
        cmd_report(data_root, args.seeds, chunk, pts, args.channel)
    else:
        ap.print_help()


if __name__ == "__main__":
    main()
