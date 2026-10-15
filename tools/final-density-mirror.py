#!/usr/bin/env python3
"""final_density-mirror — terrain.rs final_density 组装的 python 逐行镜像 vs 官方探针。

镜像对象 = game/mcv_worldgen/src/vanilla/{terrain,climate,noise,perlin,blended,
spline}.rs 的 density 组装链（本仓 rust 无法本机跑，行为锁在 CI）——
fork-chain-mirror.py 已证 10 通道种子链机制（climate/aquifer 360/360），
本脚本把对拍面扩到 final_density 全树：

- 通道构造：router 密度通道全部 = rust `NormalNoise::from_vanilla_name`
  原版 fork 链（jd.NormalNoiseVanilla，官方 108/108 位型互证的同组复刻），
  firstOctave/振幅表逐通道 load_params 直读 mc-ref noise json（不抄 rust
  注释——表错则对拍直接红）；
- base_3d：rust `BlendedNoise::new(world_seed, 0x00B1_3D00)` 的 clean-room
  splitmix 域盐链逐行转写（noise.rs:46-60 splitmix64/noise_seed、
  perlin.rs:150-163 ImprovedNoise::new、blended.rs:65-78 三栈派生 +
  :83-137 compute）——⚠ 非官方 fromHashOf("terrain") 链（KNOWN-DIVERGENCE
  登记残余），本镜像取的是 rust 的期望值，官方同点自然有差；
- climate/sample + 列常量（climate.rs:116-140 + terrain.rs:762-798）、
  offset/factor/jaggedness 样条树（terrain.rs:156-419 Builder 逐节点）与
  spline.rs 的 Hermite/linear_extend 求值语义、density()（terrain.rs:1005-1046）
  全部逐行对位转写（span_map/y_grad/quarter_negative/peaks_and_valleys
  用 rust 字面量，不归并成等价形式——保 f64 运算序）。

官方真值（/root/java-harness/tools/java-harness，JVM 探针）：
- `--density y,X,Z` = RandomState.finalDensity().compute(单点)——**单点求值
  无 cell interpolation**（NoiseInterpolator 未填充时 Interpolated.compute
  直透 wrap 参数；blend_density 在无混合邻域时同透）——即 raw 树值；
- `--probe-noise name,y,x,z` = getOrCreateNoise(name).getValue(x,y,z)。
  缩放通道用「采样坐标恰为整数」的世界点对拍（如 cave_entrance
  (x·0.75, y·0.5, z·0.75) 取 x,z ≡ 0 mod 4 的偶 y 点）。

用法: python3 tools/final-density-mirror.py [--seed 132784305] [--quick]
只读 /root/java-harness 与 /root/mc-ref；不写 java-harness。
"""

import argparse
import json
import math
import struct
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, "/root/java-harness/tools")
import java_dupe as jd  # noqa: E402  借 java-harness 108/108 位型互证组复刻件

DATA = Path("/root/mc-ref/src-26.1/data/minecraft/worldgen/noise")
JH = Path("/root/java-harness/tools/java-harness")

# ------------------------------------------------------------ rust 算子转写
# terrain.rs:119-137（lerp1/slope/span_map/y_grad/y_clamp）
def lerp1(a, b, t):
    return a + (b - a) * t


def span_map(n, lo, hi):
    return (lo + hi) * 0.5 + (hi - lo) * 0.5 * n


def y_grad(y, f_y, t_y, f_v, t_v):
    t = min(max((y - f_y) / (t_y - f_y), 0.0), 1.0)
    return lerp1(f_v, t_v, t)


def y_clamp(y):
    return y_grad(y, -64.0, 320.0, 1.5, -1.5)


def quarter_negative(x):
    return x if x > 0.0 else x * 0.25


def half_negative(x):
    return x if x > 0.0 else x * 0.5


# terrain.rs:152-154（rust 字面量 0.6666667 / 0.33333334，不归并）
def peaks_and_valleys(r):
    return -(3.0) * ((abs(r) - 0.6666667).__abs__() - 0.33333334)


# noise.rs:213-216（巨坐标封装）
WRAP_D = 3.355_443_2e7


def wrap(x):
    return x - math.floor(x / WRAP_D + 0.5) * WRAP_D


# ------------------------------------------------------------ 噪声构造
# rust NormalNoise::factors（noise.rs:182-209，expectedDeviation 按非零跨度）
def factors(amplitudes):
    min_i, max_i = None, None
    for i, a in enumerate(amplitudes):
        if a != 0.0:
            if min_i is None:
                min_i = i
            max_i = i
    span = 0 if min_i is None else (max_i - min_i)
    expected_deviation = 0.1 * (1.0 + 1.0 / (span + 1.0))
    value_factor = 0.16666666666666666 / expected_deviation
    return value_factor


class NormalNoiseRust:
    """rust NormalNoise::from_vanilla_name 同链（fork 链件用 jd 已互证组）。

    one_perlin/get_value 按 noise.rs:150-174 的运算序（wrap→noise→
    amp·value_factor；second 层坐标 ×1.0181268882175227）。"""

    def __init__(self, root, name, first_octave, amplitudes):
        source = jd.factory_from_hash(root, name)
        self.first = self._build(source, first_octave, amplitudes)
        self.second = self._build(source, first_octave, amplitudes)
        self.fo = first_octave
        self.amps = list(amplitudes)
        self.vf = factors(amplitudes)

    @staticmethod
    def _build(source, fo,amps):
        positional = source.fork_positional()
        out = []
        for i, a in enumerate(amps):
            if a != 0.0:
                rng = jd.factory_from_hash(positional, f"octave_{fo + i}")
                out.append(jd.ImprovedNoise(rng))
            else:
                out.append(None)
        return out

    def _one(self, octaves, x, y, z):
        value = 0.0
        factor = 2.0 ** self.fo
        value_factor = 2.0 ** (len(self.amps) - 1) / (2.0 ** len(self.amps) - 1.0)
        for amp, inst in zip(self.amps, octaves):
            if amp != 0.0:
                value += amp * inst.noise(wrap(x * factor), wrap(y * factor), wrap(z * factor)) * value_factor
            factor *= 2.0
            value_factor /= 2.0
        return value

    def get_value(self, x, y, z):
        d = 1.018_126_888_217_522_7
        return (self._one(self.first, x, y, z)
                + self._one(self.second, x * d, y * d, z * d)) * self.vf


class SplitmixStream:
    """perlin.rs:119-135（next_f64 = (u64>>11)·2^-53；next_below = (u64>>32)%bound）"""

    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFFFFFFFFFF

    def next_u64(self):
        self.s = splitmix64(self.s)
        return self.s

    def next_f64(self):
        return float(self.next_u64() >> 11) * (1.0 / float(1 << 53))

    def next_below(self, bound):
        return (self.next_u64() >> 32) % bound


def splitmix64(x):
    M = 0xFFFFFFFFFFFFFFFF
    x = (x + 0x9E3779B97F4A7C15) & M
    x = ((x ^ (x >> 30)) * 0xBF58476D1CE4E5B9) & M
    x = ((x ^ (x >> 27)) * 0x94D049BB133111EB) & M
    return x ^ (x >> 31)


SHIFT_UP_EPSILON = 1.0000000116860974e-07


class ImprovedNoiseRust:
    """perlin.rs:147-262 clean-room 构造（SplitmixStream 原点/排列），内核
    采样 = jd.ImprovedNoise 同源 vanilla 算术（sampleAndLerp 逐点转写，
    含 noise_fudged 5 参 yFudge，yAlpha 用量化前 yrOriginal）。"""

    def __init__(self, seed):
        rng = SplitmixStream(seed)
        self.xo = rng.next_f64() * 256.0
        self.yo = rng.next_f64() * 256.0
        self.zo = rng.next_f64() * 256.0
        p = list(range(256))
        for i in range(256):
            o = rng.next_below(256 - i)
            p[i], p[i + o] = p[i + o], p[i]
        self.p = [v & 0xFF for v in p]

    def _p(self, x):
        return self.p[x & 0xFF]

    def noise_fudged(self, x, y, z, y_scale, y_fudge):
        x += self.xo
        y += self.yo
        z += self.zo
        xf, yf, zf = math.floor(x), math.floor(y), math.floor(z)
        xr, yr, zr = x - xf, y - yf, z - zf
        if y_scale != 0.0:
            fudge_limit = y_fudge if (0.0 <= y_fudge < yr) else yr
            yr_fudge = math.floor(fudge_limit / y_scale + SHIFT_UP_EPSILON) * y_scale
        else:
            yr_fudge = 0.0
        # sample_and_lerp（perlin.rs:232-262）
        x0 = self._p(xf)
        x1 = self._p(xf + 1)
        xy00 = self._p(x0 + yf)
        xy01 = self._p(x0 + yf + 1)
        xy10 = self._p(x1 + yf)
        xy11 = self._p(x1 + yf + 1)
        d000 = jd.ImprovedNoise._grad_dot(self._p(xy00 + zf), xr, yr, zr)
        d100 = jd.ImprovedNoise._grad_dot(self._p(xy10 + zf), xr - 1.0, yr, zr)
        d010 = jd.ImprovedNoise._grad_dot(self._p(xy01 + zf), xr, yr - 1.0, zr)
        d110 = jd.ImprovedNoise._grad_dot(self._p(xy11 + zf), xr - 1.0, yr - 1.0, zr)
        d001 = jd.ImprovedNoise._grad_dot(self._p(xy00 + zf + 1), xr, yr, zr - 1.0)
        d101 = jd.ImprovedNoise._grad_dot(self._p(xy10 + zf + 1), xr - 1.0, yr, zr - 1.0)
        d011 = jd.ImprovedNoise._grad_dot(self._p(xy01 + zf + 1), xr, yr - 1.0, zr - 1.0)
        d111 = jd.ImprovedNoise._grad_dot(self._p(xy11 + zf + 1), xr - 1.0, yr - 1.0, zr - 1.0)
        xa = jd.smoothstep(xr)
        ya = jd.smoothstep(yr)          # yAlpha 用量化前 yrOriginal（:101）
        za = jd.smoothstep(zr)
        return jd.lerp3(xa, ya, za, d000, d100, d010, d110, d001, d101, d011, d111)

    def noise(self, x, y, z):
        return self.noise_fudged(x, y, z, 0.0, 0.0)


class BlendedNoiseRust:
    """rust BlendedNoise::new/get_value（blended.rs:36-137）clean-room 盐链。"""

    XZ_MULT = 171.103          # 684.412·0.25
    Y_MULT = 85.5515           # 684.412·0.125
    XZ_FACTOR = 80.0
    Y_FACTOR = 160.0
    SMEAR_MULT = 8.0
    OCTAVE_SEED_STEP = 0x10000000

    def __init__(self, world_seed, salt=0x00B1_3D00):
        s = splitmix64(world_seed ^ salt)

        def stack(root, n):
            return [ImprovedNoiseRust((root + i * self.OCTAVE_SEED_STEP) & 0xFFFFFFFFFFFFFFFF)
                    for i in range(n)]

        self.min_limit = stack(splitmix64(s ^ 0x5A11_1A5A), 16)
        self.max_limit = stack(splitmix64(s ^ 0x5A22_2A5A), 16)
        self.main = stack(splitmix64(s ^ 0x5A33_3A5A), 8)

    def get_value(self, bx, by, bz):
        lm = self.Y_MULT * self.SMEAR_MULT
        ms = lm / self.Y_FACTOR
        limit_x, limit_y, limit_z = bx * self.XZ_MULT, by * self.Y_MULT, bz * self.XZ_MULT
        main_x, main_y, main_z = limit_x / self.XZ_FACTOR, limit_y / self.Y_FACTOR, limit_z / self.XZ_FACTOR
        pow_ = 1.0
        main_value = 0.0
        for inst in self.main:
            main_value += inst.noise_fudged(
                wrap(main_x * pow_), wrap(main_y * pow_), wrap(main_z * pow_),
                ms * pow_, main_y * pow_,
            ) / pow_
            pow_ *= 0.5
        factor = (main_value / 10.0 + 1.0) / 2.0
        is_max = factor >= 1.0
        is_min = factor <= 0.0
        pow_ = 1.0
        blend_min = blend_max = 0.0
        for mn, mx in zip(self.min_limit, self.max_limit):
            wx, wy, wz = wrap(limit_x * pow_), wrap(limit_y * pow_), wrap(limit_z * pow_)
            y_scale_pow = lm * pow_
            if not is_max:
                blend_min += mn.noise_fudged(wx, wy, wz, y_scale_pow, limit_y * pow_) / pow_
            if not is_min:
                blend_max += mx.noise_fudged(wx, wy, wz, y_scale_pow, limit_y * pow_) / pow_
            pow_ *= 0.5
        lo, hi = blend_min / 512.0, blend_max / 512.0
        if factor < 0.0:
            v = lo
        elif factor > 1.0:
            v = hi
        else:
            v = lo + (hi - lo) * factor
        return v / 128.0


# ------------------------------------------------------------ 样条（spline.rs 语义）
def find_interval_start(locations, t):
    lo, hi = 0, len(locations)
    while lo < hi:
        mid = (lo + hi) // 2
        if t < locations[mid]:
            hi = mid
        else:
            lo = mid + 1
    return lo - 1


def linear_extend(value, derivative, t, location):
    if derivative == 0.0:
        return value
    return value + derivative * (t - location)


class Multipoint:
    """spline.rs:53-101 Multipoint 语义（值=常数/嵌套样条；导数=节点导数）。"""

    def __init__(self, coord, points):
        self.coord = coord
        self.locs = [p[0] for p in points]
        self.vals = [p[1] if isinstance(p[1], (Multipoint, float, int)) else p[1] for p in points]
        self.ders = [p[2] for p in points]

    def eval(self, c):
        t = self.coord(c)
        i = find_interval_start(self.locs, t)
        last = len(self.locs) - 1
        if i < 0:
            vi = self.vals[0].eval(c) if isinstance(self.vals[0], Multipoint) else float(self.vals[0])
            return linear_extend(vi, self.ders[0], t, self.locs[0])
        if i == last:
            vi = self.vals[last].eval(c) if isinstance(self.vals[last], Multipoint) else float(self.vals[last])
            return linear_extend(vi, self.ders[last], t, self.locs[last])
        x1, x2 = self.locs[i], self.locs[i + 1]
        tt = (t - x1) / (x2 - x1)
        v1 = self.vals[i].eval(c) if isinstance(self.vals[i], Multipoint) else self.vals[i]
        v2 = self.vals[i + 1].eval(c) if isinstance(self.vals[i + 1], Multipoint) else self.vals[i + 1]
        d1, d2 = self.ders[i], self.ders[i + 1]
        a = d1 * (x2 - x1) - (v2 - v1)
        b = -d2 * (x2 - x1) + (v2 - v1)
        lt = v1 + (v2 - v1) * tt
        return lt + tt * (1.0 - tt) * (a + (b - a) * tt)


def P(x, v, d=0.0):
    return (x, v, d)


# ------------------------------------------------------------ terrain.rs 样条树（156-419 逐节点）
RIDGE_AMPLITUDE = 0.46082947
RIDGE_OFFSET = 1.17


def mountain_continentalness(ridge, modulation):
    ridge_slope = 1.0 - (1.0 - modulation) * 0.5
    ridge_intersect = 0.5 * (1.0 - modulation)
    adjusted = (ridge + RIDGE_OFFSET) * RIDGE_AMPLITUDE
    c = adjusted * ridge_slope - ridge_intersect
    if ridge < -0.7:
        return max(c, -0.2222)
    return max(c, 0.0)


def mountain_ridge_zero_point(modulation):
    ridge_slope = 1.0 - (1.0 - modulation) * 0.5
    ridge_intersect = 0.5 * (1.0 - modulation)
    return ridge_intersect / (RIDGE_AMPLITUDE * ridge_slope) - RIDGE_OFFSET


def ridge_shell(valley, low, mid, high, peaks, min_valley):
    d1 = max(0.5 * (low - valley), min_valley)
    d2 = 5.0 * (mid - low)
    return Multipoint(lambda c: c[3], [
        P(-1.0, valley, d1),
        P(-0.4, low, min(d1, d2)),
        P(0.0, mid, d2),
        P(0.4, high, 2.0 * (high - mid)),
        P(1.0, peaks, 0.7 * (peaks - high)),
    ])


def mountain_ridge(modulation, saddle):
    min_c = mountain_continentalness(-1.0, modulation)
    max_c = mountain_continentalness(1.0, modulation)
    zero_point = mountain_ridge_zero_point(modulation)
    pts = []
    if -0.65 < zero_point < 1.0:
        after_river = mountain_continentalness(-0.65, modulation)
        before_river = mountain_continentalness(-0.75, modulation)
        min_d = (min_c - before_river) / (-1.0 - -0.75)
        pts.append(P(-1.0, min_c, min_d))
        pts.append(P(-0.75, before_river))
        pts.append(P(-0.65, after_river))
        zero_c = mountain_continentalness(zero_point, modulation)
        max_d = (zero_c - max_c) / (zero_point - 1.0)
        pts.append(P(zero_point - 0.01, zero_c))
        pts.append(P(zero_point, zero_c, max_d))
        pts.append(P(1.0, max_c, max_d))
    else:
        simple = (min_c - max_c) / (-1.0 - 1.0)
        if saddle:
            pts.append(P(-1.0, max(0.2, min_c)))
            pts.append(P(0.0, lerp1(0.5, min_c, max_c), simple))
        else:
            pts.append(P(-1.0, min_c, simple))
        pts.append(P(1.0, max_c, simple))
    return Multipoint(lambda c: c[3], pts)


def erosion_factor(base_value, shattered):
    base_spline = Multipoint(lambda c: c[2], [P(-0.2, 6.3), P(0.2, base_value)])
    pts = []
    pts.append(P(-0.6, base_spline))
    pts.append(P(-0.5, Multipoint(lambda c: c[2], [P(-0.05, 6.3), P(0.05, 2.67)])))
    pts.append(P(-0.35, base_spline))
    pts.append(P(-0.25, base_spline))
    pts.append(P(-0.1, Multipoint(lambda c: c[2], [P(-0.05, 2.67), P(0.05, 6.3)])))
    pts.append(P(0.03, base_spline))
    if shattered:
        weird = Multipoint(lambda c: c[2], [P(0.0, base_value), P(0.1, 0.625)])
        ridges_sh = Multipoint(lambda c: c[3], [P(-0.9, base_value), P(-0.69, weird)])
        pts.append(P(0.35, base_value))
        pts.append(P(0.45, ridges_sh))
        pts.append(P(0.55, ridges_sh))
        pts.append(P(0.62, base_value))
    else:
        extreme = Multipoint(lambda c: c[3], [P(-0.7, base_spline), P(-0.15, 1.37)])
        extra = Multipoint(lambda c: c[3], [P(0.45, base_spline), P(0.7, 1.56)])
        pts.append(P(0.05, extra))
        pts.append(P(0.4, extra))
        pts.append(P(0.45, extreme))
        pts.append(P(0.55, extreme))
        pts.append(P(0.58, base_value))
    return Multipoint(lambda c: c[1], pts)


def overworld_factor_spline():
    b = Multipoint.__new__(Multipoint)
    pts = [P(-0.19, 3.95)]
    pts.append(P(-0.15, erosion_factor(6.25, True)))
    pts.append(P(-0.1, erosion_factor(5.47, True)))
    pts.append(P(0.03, erosion_factor(5.08, True)))
    pts.append(P(0.06, erosion_factor(4.69, False)))
    return Multipoint(lambda c: c[0], pts)


def ridge_jaggedness(at_peak, at_high):
    start = peaks_and_valleys(0.4)
    end = peaks_and_valleys(0.56666666)
    middle = (start + end) / 2.0
    pts = [P(start, 0.0)]
    if at_high > 0.0:
        pts.append(P(middle, weirdness_jaggedness(at_high)))
    else:
        pts.append(P(middle, 0.0))
    if at_peak > 0.0:
        pts.append(P(1.0, weirdness_jaggedness(at_peak)))
    else:
        pts.append(P(1.0, 0.0))
    return Multipoint(lambda c: c[3], pts)


def weirdness_jaggedness(jag_factor):
    return Multipoint(lambda c: c[2], [P(-0.01, 0.63 * jag_factor), P(0.01, 0.3 * jag_factor)])


def erosion_jaggedness(a0, a1, h0, h1):
    r0 = ridge_jaggedness(a0, h0)
    r1 = ridge_jaggedness(a1, h1)
    return Multipoint(lambda c: c[1], [
        P(-1.0, r0), P(-0.78, r1), P(-0.5775, r1), P(-0.375, 0.0),
    ])


def overworld_jaggedness_spline():
    return Multipoint(lambda c: c[0], [
        P(-0.11, 0.0),
        P(0.03, erosion_jaggedness(1.0, 0.5, 0.0, 0.0)),
        P(0.65, erosion_jaggedness(1.0, 1.0, 1.0, 0.0)),
    ])


def erosion_offset_spline(low_valley, hill, tall_hill, mf, plain, swamp, include_extreme_hills, saddle):
    very_low = mountain_ridge(lerp1(0.6, 1.5, mf), saddle)
    low_m = mountain_ridge(lerp1(0.6, 1.0, mf), saddle)
    m = mountain_ridge(mf, saddle)
    wide = ridge_shell(low_valley - 0.15, 0.5 * mf, 0.5 * mf, 0.5 * mf, 0.6 * mf, 0.5)
    narrow = ridge_shell(low_valley, plain * mf, hill * mf, 0.5 * mf, 0.6 * mf, 0.5)
    plains = ridge_shell(low_valley, plain, plain, hill, tall_hill, 0.5)
    extreme_hills = Multipoint(lambda c: c[3], [
        P(-1.0, low_valley), P(-0.4, plains), P(0.0, tall_hill + 0.07),
    ])
    swamps = ridge_shell(-0.02, swamp, swamp, hill, tall_hill, 0.0)
    pts = []
    pts.append(P(-0.85, very_low))
    pts.append(P(-0.7, low_m))
    pts.append(P(-0.4, m))
    pts.append(P(-0.35, wide))
    pts.append(P(-0.1, narrow))
    pts.append(P(0.2, plains))
    if include_extreme_hills:
        pts.append(P(0.4, plains))
        pts.append(P(0.45, extreme_hills))
        pts.append(P(0.55, extreme_hills))
        pts.append(P(0.58, plains))
    pts.append(P(0.7, swamps))
    return Multipoint(lambda c: c[1], pts)


def overworld_offset_spline():
    beach = erosion_offset_spline(-0.15, 0.0, 0.0, 0.1, 0.0, -0.03, False, False)
    low = erosion_offset_spline(-0.1, 0.03, 0.1, 0.1, 0.01, -0.03, False, False)
    mid = erosion_offset_spline(-0.1, 0.03, 0.1, 0.7, 0.01, -0.03, True, True)
    high = erosion_offset_spline(-0.05, 0.03, 0.1, 1.0, 0.01, 0.01, True, True)
    return Multipoint(lambda c: c[0], [
        P(-1.1, 0.044), P(-1.02, -0.2222), P(-0.51, -0.2222), P(-0.44, -0.12),
        P(-0.18, -0.12), P(-0.16, beach), P(-0.15, beach), P(-0.1, low),
        P(0.25, mid), P(1.0, high),
    ])


GLOBAL_OFFSET = -0.50375
NOODLE_OFF = 64.0
PILLARS_GATE_MIN = 0.03
CARVE_OFF = -1.0e6
NOODLE_RIDGE_SCALE = 2.6666666666666665


# ------------------------------------------------------------ rust Orchestrator 镜像
class Orch:
    def __init__(self, world_seed):
        self.seed = world_seed & 0xFFFFFFFFFFFFFFFF
        root = jd.XoroshiroRandomSource.from_seed(world_seed if world_seed < 2**63 else world_seed - 2**64).fork_positional()
        # rust 侧 world_seed as i64 —— jd.from_seed 期望有符号位型一致

        # 通道构造（rust Orchestrator::new/ClimateSampler::new 的 n()/ch() 闭包）
        def n(name, fo):
            j = json.loads((DATA / f"{name.removeprefix('minecraft:')}.json").read_text())
            assert j["firstOctave"] == fo, f"{name}: rust fo={fo} json={j['firstOctave']}"
            return NormalNoiseRust(root, name, fo, j["amplitudes"])

        self.n = n
        self.offset = n("minecraft:offset", -3)
        self.continentalness = n("minecraft:continentalness", -9)
        self.erosion = n("minecraft:erosion", -9)
        self.ridge = n("minecraft:ridge", -7)
        self.temperature = n("minecraft:temperature", -10)
        self.vegetation = n("minecraft:vegetation", -8)
        self.offset_spline = overworld_offset_spline()
        self.factor_spline = overworld_factor_spline()
        self.jag_spline = overworld_jaggedness_spline()
        self.base_3d = BlendedNoiseRust(self.seed)
        self.cave_entrance = n("minecraft:cave_entrance", -7)
        self.sp3d_rarity = n("minecraft:spaghetti_3d_rarity", -11)
        self.sp3d_1 = n("minecraft:spaghetti_3d_1", -7)
        self.sp3d_2 = n("minecraft:spaghetti_3d_2", -7)
        self.sp3d_thickness = n("minecraft:spaghetti_3d_thickness", -8)
        self.sp_roughness_mod = n("minecraft:spaghetti_roughness_modulator", -8)
        self.sp_roughness = n("minecraft:spaghetti_roughness", -5)
        self.sp2d_mod = n("minecraft:spaghetti_2d_modulator", -11)
        self.sp2d = n("minecraft:spaghetti_2d", -7)
        self.sp2d_elevation = n("minecraft:spaghetti_2d_elevation", -8)
        self.sp2d_thickness = n("minecraft:spaghetti_2d_thickness", -11)
        self.cave_layer = n("minecraft:cave_layer", -8)
        self.cave_cheese = n("minecraft:cave_cheese", -8)
        self.jagged = n("minecraft:jagged", -16)
        self.pillar = n("minecraft:pillar", -7)
        self.pillar_rareness = n("minecraft:pillar_rareness", -8)
        self.pillar_thickness = n("minecraft:pillar_thickness", -8)
        self.noodle_noise = n("minecraft:noodle", -8)
        self.noodle_thickness = n("minecraft:noodle_thickness", -8)
        self.noodle_ridge_a = n("minecraft:noodle_ridge_a", -7)
        self.noodle_ridge_b = n("minecraft:noodle_ridge_b", -7)

        _p = DATA / "terrain.json"
        self.terrain_official = _p.exists()

    @staticmethod
    def _params(name):
        j = json.loads((DATA / f"{name.removeprefix('minecraft:')}.json").read_text())
        if j.get("type") == "minecraft:old_blended_noise":
            return None
        return j["amplitudes"]

    def _chan(self, name, fo):
        amps = self._params(name)
        j = json.loads((DATA / f"{name.removeprefix('minecraft:')}.json").read_text())
        assert j["firstOctave"] == fo, f"{name}: rust fo={fo} json={j['firstOctave']}"
        return NormalNoiseRust(root, name, fo, amps)

    # climate.rs:116-140
    def climate(self, bx, bz):
        bxf, bzf = float(bx), float(bz)
        sx = self.offset.get_value(bxf * 0.25, 0.0, bzf * 0.25) * 4.0
        sz = self.offset.get_value(bzf * 0.25, bxf * 0.25, 0.0) * 4.0
        c_ = self.continentalness.get_value(bxf * 0.25 + sx, 0.0, bzf * 0.25 + sz)
        e = self.erosion.get_value(bxf * 0.25 + sx, 0.0, bzf * 0.25 + sz)
        r = self.ridge.get_value(bxf * 0.25 + sx, 0.0, bzf * 0.25 + sz)
        return (c_, e, r, peaks_and_valleys(r))

    def column_state(self, bx, bz):
        c = self.climate(bx, bz)
        offset = self.offset_spline.eval(c) + GLOBAL_OFFSET
        factor = self.factor_spline.eval(c)
        unscaled_jag = self.jag_spline.eval(c)
        jag_noise = self.jagged.get_value(float(bx) * 1500.0, 0.0, float(bz) * 1500.0)
        elev_n = self.sp2d_elevation.get_value(float(bx), 0.0, float(bz))
        sp2d_elev = span_map(elev_n, -8.0, 8.0)
        return dict(offset=offset, factor=factor, unscaled_jag=unscaled_jag,
                    jag_noise=jag_noise, sp2d_elev=sp2d_elev)

    # terrain.rs:1005-1046 density()
    def density(self, cs, block_x, block_y, block_z):
        v = self.terrain_density(cs, block_x, block_y, block_z)
        bx, bz = float(block_x), float(block_z)
        y = float(block_y)  # ours_y_to_mc 恒等
        noodle_v = self.noodle(bx, y, bz)
        c = min(max(0.64 * v, -1.0), 1.0)
        return c / 2.0 - c * c * c / 24.0, noodle_v

    # terrain.rs terrain_density（#210 拆出）——final_density 内被
    # `minecraft:interpolated` 包裹的合成树（W1）raw 值：blend_density
    # 恒等 + slide 双 lerp，op 序与 rust 逐位同。
    def terrain_density(self, cs, block_x, block_y, block_z):
        bx, bz = float(block_x), float(block_z)
        y = float(block_y)
        jag = cs["unscaled_jag"] * half_negative(cs["jag_noise"])
        depth = y_clamp(y) + cs["offset"]
        initial = 4.0 * quarter_negative(cs["factor"] * (depth + jag))
        base_3d_v = self.base_3d.get_value(bx, y, bz)
        sloped = initial + base_3d_v
        e = self.entrances(bx, y, bz)
        if sloped < 1.5625:
            caves = min(sloped, 5.0 * e)
        else:
            sp2d_v = self.spaghetti_2d(cs, bx, y, bz)
            u = self.underground(bx, y, bz, sloped)
            base = min(min(u, e), sp2d_v + self.spaghetti_roughness(bx, y, bz))
            p = self.pillars(bx, y, bz)
            caves = max(base, CARVE_OFF if p < PILLARS_GATE_MIN else p)
        top_factor = y_grad(y, 240.0, 256.0, 1.0, 0.0)
        v = lerp1(-0.078125, caves, top_factor)
        bottom_factor = y_grad(y, -64.0, -40.0, 0.0, 1.0)
        return lerp1(0.1171875, v, bottom_factor)

    # ---------------- #210 cell 预积分插值外壳（vanilla/cell_interp.rs 镜像）----------------
    # 角列缓存：kx,kz（世界角坐标 = kx*4, kz*4）→ [5 槽位][49 角层]，
    # 角层 y = -64 + 8*gy。槽位 0..4 = W1(地形 slide 合成)/W2(noodle 掩膜)/
    # W3(厚度基底)/W4(ridge_a)/W5(ridge_b)（cell_interp.rs W_* 常数序）。
    GY_LEVELS = 49
    NOODLE_Y_MIN = -60.0
    NOODLE_RIDGE_SCALE_SHELL = NOODLE_RIDGE_SCALE  # terrain.rs 常数单源

    def corner_column(self, cache, kx, kz):
        key = (kx, kz)
        hit = cache.get(key)
        if hit is not None:
            return hit
        corner_x = kx * 4
        corner_z = kz * 4
        cs = self.column_state(corner_x, corner_z)
        w = [[0.0] * self.GY_LEVELS for _ in range(5)]
        s = self.NOODLE_RIDGE_SCALE_SHELL
        for gy in range(self.GY_LEVELS):
            corner_y = -64 + 8 * gy
            y = float(corner_y)
            # W1：地形+slide 全合成（树内无包裹节点）
            w[0][gy] = self.terrain_density(cs, corner_x, corner_y, corner_z)
            # W2：noodle 掩膜 range_choice(y∈[-60,321) → noise, else -1)
            w[1][gy] = (-1.0 if y < self.NOODLE_Y_MIN
                        else self.noodle_noise.get_value(float(corner_x), y, float(corner_z)))
            # W3：add(-0.07500000000000001, -0.025·thickness)，else 0
            w[2][gy] = (0.0 if y < self.NOODLE_Y_MIN
                        else -0.075_000_000_000_000_01
                        - 0.025 * self.noodle_thickness.get_value(float(corner_x), y, float(corner_z)))
            # W4/W5：ridge_a/b（xz/y·2.6666…，else 0；abs 在包裹外）
            w[3][gy] = (0.0 if y < self.NOODLE_Y_MIN else
                        self.noodle_ridge_a.get_value(float(corner_x) * s, y * s, float(corner_z) * s))
            w[4][gy] = (0.0 if y < self.NOODLE_Y_MIN else
                        self.noodle_ridge_b.get_value(float(corner_x) * s, y * s, float(corner_z) * s))
        cache[key] = w
        return w

    @staticmethod
    def interp_slot(slot, c00, c10, c01, c11, gy0, gy1, fx, fy, fz):
        """cell_interp.rs interp_slot 镜像：增量序 Y 内→X→Z，lerp=lo+t·(hi-lo)。"""
        n000 = c00[slot][gy0]
        n100 = c10[slot][gy0]
        n010 = c00[slot][gy1]
        n110 = c10[slot][gy1]
        n001 = c01[slot][gy0]
        n101 = c11[slot][gy0]
        n011 = c01[slot][gy1]
        n111 = c11[slot][gy1]
        xz00 = n000 + fy * (n010 - n000)
        xz10 = n100 + fy * (n110 - n100)
        xz01 = n001 + fy * (n011 - n001)
        xz11 = n101 + fy * (n111 - n101)
        z0 = xz00 + fx * (xz10 - xz00)
        z1 = xz01 + fx * (xz11 - xz01)
        return z0 + fz * (z1 - z0)

    def cell_density(self, cache, block_x, block_y, block_z):
        """cell_interp.rs CellShell::density_at 镜像：5 槽位插值 + 外壳合成。"""
        kx = block_x // 4   # python floor-div = i32 div_euclid（负坐标同型）
        kz = block_z // 4
        in_cell_x = block_x - kx * 4
        in_cell_z = block_z - kz * 4
        y_rel = min(max(block_y - (-64), 0), 48 * 8 - 1)
        cell_y_index = y_rel // 8
        in_cell_y = y_rel - cell_y_index * 8
        gy0 = cell_y_index
        gy1 = cell_y_index + 1
        fx = float(in_cell_x) / 4.0
        fy = float(in_cell_y) / 8.0
        fz = float(in_cell_z) / 4.0
        c00 = self.corner_column(cache, kx, kz)
        c10 = self.corner_column(cache, kx + 1, kz)
        c01 = self.corner_column(cache, kx, kz + 1)
        c11 = self.corner_column(cache, kx + 1, kz + 1)
        terrain = self.interp_slot(0, c00, c10, c01, c11, gy0, gy1, fx, fy, fz)
        c = min(max(0.64 * terrain, -1.0), 1.0)
        d = c / 2.0 - c * c * c / 24.0
        mask = self.interp_slot(1, c00, c10, c01, c11, gy0, gy1, fx, fy, fz)
        if mask < 0.0:
            noodle = NOODLE_OFF
        else:
            base = self.interp_slot(2, c00, c10, c01, c11, gy0, gy1, fx, fy, fz)
            a = abs(self.interp_slot(3, c00, c10, c01, c11, gy0, gy1, fx, fy, fz))
            b = abs(self.interp_slot(4, c00, c10, c01, c11, gy0, gy1, fx, fy, fz))
            noodle = base + 1.5 * max(a, b)
        return min(d, noodle)

    def spaghetti_roughness(self, bx, y, bz):
        mod_v = span_map(self.sp_roughness_mod.get_value(bx, y, bz), 0.0, -0.1)
        return mod_v * (abs(self.sp_roughness.get_value(bx, y, bz)) - 0.4)

    def entrances(self, bx, y, bz):
        rarity_mod = self.sp3d_rarity.get_value(bx * 2.0, y, bz * 2.0)
        rarity = (0.75 if rarity_mod < -0.5 else 1.0 if rarity_mod < 0.0
                  else 1.5 if rarity_mod < 0.5 else 2.0)
        w1 = rarity * abs(self.sp3d_1.get_value(bx / rarity, y / rarity, bz / rarity))
        w2 = rarity * abs(self.sp3d_2.get_value(bx / rarity, y / rarity, bz / rarity))
        thick = span_map(self.sp3d_thickness.get_value(bx, y, bz), -0.065, -0.088)
        sp3d = min(max(max(w1, w2) + thick, -1.0), 1.0)
        big_noise = self.cave_entrance.get_value(bx * 0.75, y * 0.5, bz * 0.75)
        big = big_noise + 0.37 + y_grad(y, -10.0, 30.0, 0.3, 0.0)
        return min(big, sp3d + self.spaghetti_roughness(bx, y, bz))

    def spaghetti_2d(self, cs, bx, y, bz):
        rarity_mod = self.sp2d_mod.get_value(bx * 2.0, y, bz * 2.0)
        if rarity_mod < -0.75:
            rarity = 0.5
        elif rarity_mod < -0.5:
            rarity = 0.75
        elif rarity_mod < 0.5:
            rarity = 1.0
        elif rarity_mod < 0.75:
            rarity = 2.0
        else:
            rarity = 3.0
        cave = rarity * abs(self.sp2d.get_value(bx / rarity, y / rarity, bz / rarity))
        thick2 = span_map(self.sp2d_thickness.get_value(bx * 2.0, y, bz * 2.0), -0.6, -1.3)
        sloped_spaghetti = abs(cs["sp2d_elev"] + y_grad(y, -64.0, 320.0, 8.0, -40.0))
        inner = sloped_spaghetti + thick2
        layer_ridged = inner * inner * inner
        cave_noise = cave + 0.083 * thick2
        return min(max(max(cave_noise, layer_ridged), -1.0), 1.0)

    def underground(self, bx, y, bz, sloped_cheese):
        layer = self.cave_layer.get_value(bx, y * 8.0, bz)
        layerized = 4.0 * layer * layer
        cheese = self.cave_cheese.get_value(bx, y * (2.0 / 3.0), bz)
        solidified = min(max(0.27 + cheese, -1.0), 1.0) + min(max(1.5 - 0.64 * sloped_cheese, 0.0), 0.5)
        return layerized + solidified

    def pillars(self, bx, y, bz):
        p = self.pillar.get_value(bx * 25.0, y * 0.3, bz * 25.0)
        r = self.pillar_rareness.get_value(bx, y, bz)
        t = self.pillar_thickness.get_value(bx, y, bz)
        c = 0.55 + 0.55 * t
        return ((2.0 * p) + (-1.0 - r)) * (c * c * c)

    def noodle(self, bx, y, bz):
        if y < -60.0:
            return NOODLE_OFF
        mask = self.noodle_noise.get_value(bx, y, bz)
        if mask < 0.0:
            return NOODLE_OFF
        t = self.noodle_thickness.get_value(bx, y, bz)
        s = NOODLE_RIDGE_SCALE
        a = self.noodle_ridge_a.get_value(bx * s, y * s, bz * s)
        b = self.noodle_ridge_b.get_value(bx * s, y * s, bz * s)
        return -0.075_000_000_000_000_01 - 0.025 * t + 1.5 * max(abs(a), abs(b))


# ------------------------------------------------------------ 官方 JVM 探针
def _classpath():
    # build.sh 同款显式 CP（libs/* 一层通配看不见 nested 目录——build.sh 注）
    out = subprocess.run(
        ["sh", "-c",
         'VJAR="libs/bundle/META-INF/versions/26.1/server-26.1.jar";'
         'printf "%s:" "$VJAR" libs/server-26.1.jar;'
         'find libs -name "*.jar" ! -name "server-26.1.jar" | sort | tr "\\n" ":"'],
        cwd=JH, capture_output=True, text=True, check=True)
    return f"{JH/'build'}:" + out.stdout.strip().rstrip(":")


import subprocess  # noqa: E402


def java_run(args, timeout=1800):
    cmd = ["java", "-cp", _classpath(), "Main", *args]
    r = subprocess.run(cmd, cwd=JH, capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0:
        raise RuntimeError(f"java failed: {r.returncode}\n{r.stderr[-2000:]}")
    return r.stdout.splitlines()


def _probe_one(spec_and_flag):
    flag, spec = spec_and_flag
    for line in java_run([*flag, spec]):
        if line.startswith(("density ", "noise ", "prelim ")):
            dec = float(line.split("decimal=")[1].split()[0])
            bits = int(line.split("bits=0x")[1].split()[0], 16)
            return line, dec, bits
    raise RuntimeError(f"no probe line for {spec}: {flag}")


def probes_parallel(specs):
    """specs: [(flag_list, spec_str)] — 每 JVM 一探（--density 单发）。"""
    with ThreadPoolExecutor(max_workers=min(8, len(specs) or 1)) as ex:
        return list(ex.map(_probe_one, specs))


def bits_f(v):
    return int(struct.unpack(">Q", struct.pack(">d", v))[0])


# ------------------------------------------------------------ 对拍主流程
P_WINDOW = [  # (x, y, z) — c2_-2 洞带 / 对照点，world 块坐标
    (40, 8, -8),      # c2_-2 [0,16) 大洞带中心
    (36, 4, -4),      # 同带西北角
    (44, 12, -12),    # 同带东南
    (40, -24, -8),    # [−32,−16) rust 独有洞带
    (36, -56, -4),    # 深层
    (44, -40, -12),   # 深层偏上
    (40, 60, -8),     # 近地表
    (40, -8, -8),     # 中层对照
    (40, 30, -8),     # 中上层对照
    (88, 8, -8),      # 窗东半（c5_* 对齐带）
    (88, -24, -8),    # 东半深层
    (36, -40, -56),   # c2_-4 深
    (60, 8, -60),     # c3_-3 带
    (60, -24, -56),   # c3_-4 [−32,−16) 带
]

CHANNELS_OK = [  # 缩放坐标可整点化的通道（官方 --probe-noise 直对拍）
    ("cave_entrance", lambda x, y, z: (int(y * 0.5) if y % 2 == 0 else None, int(x * 0.75) if x % 4 == 0 else None, int(z * 0.75) if z % 4 == 0 else None), (0.75, 0.5, 0.75)),
    ("cave_layer", lambda x, y, z: (int(y * 8), x, z), (1.0, 8.0, 1.0)),
    ("spaghetti_3d_rarity", lambda x, y, z: (y, int(x * 2), int(z * 2)), (2.0, 1.0, 2.0)),
    ("spaghetti_3d_thickness", lambda x, y, z: (y, x, z), (1.0, 1.0, 1.0)),
    ("spaghetti_roughness", lambda x, y, z: (y, x, z), (1.0, 1.0, 1.0)),
    ("spaghetti_roughness_modulator", lambda x, y, z: (y, x, z), (1.0, 1.0, 1.0)),
    ("spaghetti_2d_modulator", lambda x, y, z: (y, int(x * 2), int(z * 2)), (2.0, 1.0, 2.0)),
    ("spaghetti_2d_thickness", lambda x, y, z: (y, int(x * 2), int(z * 2)), (2.0, 1.0, 2.0)),
    ("spaghetti_2d_elevation", lambda x, y, z: (0, x, z), (1.0, 0.0, 1.0)),
    ("noodle", lambda x, y, z: (y, x, z), (1.0, 1.0, 1.0)),
    ("noodle_thickness", lambda x, y, z: (y, x, z), (1.0, 1.0, 1.0)),
    ("pillar_rareness", lambda x, y, z: (y, x, z), (1.0, 1.0, 1.0)),
    ("pillar_thickness", lambda x, y, z: (y, x, z), (1.0, 1.0, 1.0)),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=lambda s: int(s, 0), default=0x7EA20B1)
    ap.add_argument("--quick", action="store_true", help="只跑 6 点 density 对拍")
    args = ap.parse_args()
    pts = P_WINDOW[:6] if args.quick else P_WINDOW

    orch = Orch(args.seed)

    # 1) 列常量快照
    print(f"== seed {args.seed} (0x{args.seed:06X}) == column states")
    cols = {}
    for (x, z) in sorted({(p[0], p[2]) for p in pts}):
        cs = orch.column_state(x, z)
        cols[(x, z)] = cs
        print(f"  col x={x} z={z} offset={cs['offset']:.9f} factor={cs['factor']:.6f} "
              f"jag={cs['unscaled_jag']:.6f} jag_noise={cs['jag_noise']:.6f} "
              f"sp2d_elev={cs['sp2d_elev']:.6f}")

    # 2) density 单点对拍（rust 镜像 vs 官方 --density）
    print("\n== density rust-mirror vs official --density ==")
    specs = [ (["--seed", str(args.seed), "--density"], f"{y},{x},{z}") for (x, y, z) in pts ]
    off = probes_parallel(specs)
    max_d = 0.0
    first_bad = None
    for (x, y, z), line in zip(pts, off):
        runtime, noodle_v = orch.density(cols[(x, z)], x, y, z)
        # 官方探针值=树值（含 min noodle 外壳）——官方= min(squeeze(0.64v), noodle)
        official = line[1]
        d = abs(runtime - official)
        if d > max_d:
            max_d, first_bad = d, (x, y, z)
        print(f"  ({x},{y},{z}) mirror={runtime:+.17g} official={official:+.17g} "
              f"|d|={d:.3g} noodle_r={noodle_v:.3f} bit_eq={bits_f(runtime) == line[2]}")
    print(f"  max|d|={max_d:.3g} @ {first_bad}  [raw 树值口径；base_3d 盐差计入]")

    # 2.5) #210 cell 预积分插值外壳镜像（cell_interp.rs）对拍
    # 角点口径：fx/fy/fz ∈ {0,1} ⇒ 插值恒等（lerp(t=0)=lo / t=1=hi 精确），
    # 角面 cell 值必须逐位 = 官方 --density raw（max|d| 目标 0）。
    # in-cell 点：官方单点探针无外壳（直透 raw），供Collect插值平滑幅度参考，
    # 不进 max|d|=0 门。
    print("\n== cell-shell mirror (cell_interp.rs) 角点 vs official --density ==")
    cache = {}
    corner_pts = []
    seen = set()
    for (x, y, z) in pts:
        kx, kz = x // 4, z // 4
        gy = max(0, min(47, (y + 64) // 8))
        for ddx in (0, 1):
            for ddy in (0, 1):
                for ddz in (0, 1):
                    p = ((kx + ddx) * 4, -64 + 8 * (gy + ddy), (kz + ddz) * 4)
                    if -64 <= p[1] <= 320 and p not in seen:
                        seen.add(p)
                        corner_pts.append(p)
    corner_pts = corner_pts[:12]  # 矩阵 8-12 点（JVM 探针一次一点）
    specs_c = [(["--seed", str(args.seed), "--density"], f"{y},{x},{z}") for (x, y, z) in corner_pts]
    off_c = probes_parallel(specs_c)
    max_dc = 0.0
    first_badc = None
    n_eq = 0
    for (x, y, z), (line, dec, bits) in zip(corner_pts, off_c):
        cellv = orch.cell_density(cache, x, y, z)
        d = abs(cellv - dec)
        if d > max_dc:
            max_dc, first_badc = d, (x, y, z)
        if bits_f(cellv) == bits:
            n_eq += 1
        print(f"  corner ({x},{y},{z}) cell={cellv:+.17g} official={dec:+.17g} "
              f"|d|={d:.3g} bit_eq={bits_f(cellv) == bits}")
    print(f"  角点 max|d|={max_dc:.3g} @ {first_badc}（0 目标）bit_eq {n_eq}/{len(corner_pts)}")
    print("== cell-shell in-cell 参考值（官方单点=raw，差异=预期插值平滑）==")
    for (x, y, z) in pts:
        cellv = orch.cell_density(cache, x, y, z)
        rawv, _ = orch.density(cols[(x, z)], x, y, z)
        corner = (x % 4 == 0 and z % 4 == 0 and (y + 64) % 8 == 0)
        print(f"  ({x},{y},{z}) cell={cellv:+.12g} raw={rawv:+.12g} "
              f"|cell-raw|={abs(cellv - rawv):.3g} corner={corner}")

    # 3) 通道 ladder（可整点化通道 vs 官方 --probe-noise）
    print("\n== channel ladder vs official --probe-noise ==")
    spec = []
    for (x, y, z) in pts:
        for (name, mapper, _scales) in CHANNELS_OK:
            pym = mapper(x, y, z)
            if None in pym:
                continue
            spec.append((name, pym, (x, y, z)))
    jvm_specs = [(["--seed", str(args.seed), "--probe-noise"], f"{name},{py[0]},{py[1]},{py[2]}")
                 for (name, py, _w) in spec]
    off_rows = probes_parallel(jvm_specs)
    worst = {}
    name_of = {}
    for (name, py, world), (line, dec, bits) in zip(spec, off_rows):
        ch = getattr(orch, {
            "cave_entrance": "cave_entrance", "cave_layer": "cave_layer",
            "spaghetti_3d_rarity": "sp3d_rarity", "spaghetti_3d_thickness": "sp3d_thickness",
            "spaghetti_roughness": "sp_roughness", "spaghetti_roughness_modulator": "sp_roughness_mod",
            "spaghetti_2d_modulator": "sp2d_mod", "spaghetti_2d_thickness": "sp2d_thickness",
            "spaghetti_2d_elevation": "sp2d_elevation", "noodle": "noodle_noise",
            "noodle_thickness": "noodle_thickness", "pillar_rareness": "pillar_rareness",
            "pillar_thickness": "pillar_thickness",
        }[name])
        pyv = ch.get_value(float(py[1]), float(py[0]), float(py[2]))
        same = bits_f(pyv) == bits
        dd = abs(pyv - dec)
        if dd > worst.get(name, (0.0,))[0]:
            worst[name] = (dd, world, py)
        if not same:
            name_of.setdefault(name, 0)
            name_of[name] += 1
            print(f"  DIFF {name} world={world} call={py} mirror={pyv:+.17g} "
                  f"official={dec:+.17g} |d|={dd:.3g}")
    print("  worst per channel:")
    for k, (dd, world, py) in sorted(worst.items(), key=lambda kv: -kv[1][0]):
        print(f"    {k:<30} max|d|={dd:.3g} @ world={world} call={py}")


if __name__ == "__main__":
    main()
