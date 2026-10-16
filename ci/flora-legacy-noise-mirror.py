#!/usr/bin/env python3
# DEVELOP_ONLY 花域噪声 python 镜像（fork-chain-mirror.py 先例；「python
# 独立复算」金标准样板）：复刻 mcv_worldgen/src/vanilla/flora.rs 的花域
# legacy 种子链，逐位独立复算——NormalNoise.create(WorldgenRandom(
# LegacyRandomSource(2345)), firstOctave=0, amplitudes=[1.0])
# （NoiseBasedStateProvider.java:31，flower_plain.json noise_threshold_
# provider）：
#   - LegacyRandomSource = 48-bit LCG（next(bits) 取高 bits，
#     LegacyRandomSource.java:41-48）；
#   - nextLong = BitRandomSource.java:37-41 默认（upper<<32 + lower 有符
#     号相加）；setSeed = (seed ^ 25214903917) & 2^48−1（:32-34）；
#   - NormalNoise.java:43-44：first/second 各 PerlinNoise.create → 各
#     forkPositional（= nextLong 作工厂种子，LegacyRandomSource.java:
#     27-29）→ octave_0 = fromHashOf("octave_0") = hashCode ^ 工厂种子
#     （:71-73，int 符号扩展 XOR long）；
#   - ImprovedNoise.java:14-30 构造流 + :83-104 sampleAndLerp（p() =
#     p[i&255]&0xFF、gradDot h&15 × GRADIENT 16 表、smoothstep/lerp3）；
#   - NormalNoise.java:75-79 值 = (first + second·1.0181268882175227) ×
#     0.16666666666666666/0.2（span0 expectedDeviation :97-103）；
#     坐标 wrap（PerlinNoise.java:188-190）。
# 用法: python3 ci/flora-legacy-noise-mirror.py [seed=2345]
import math
import struct
import sys

MASK = (1 << 48) - 1
MULT = 25214903917
INC = 11


class LegacyRandom:
    def __init__(self, seed):
        self.s = (seed ^ MULT) & MASK

    def next(self, bits):
        self.s = (self.s * MULT + INC) & MASK
        r = self.s >> (48 - bits)
        return r - (1 << bits) if r >= (1 << (bits - 1)) else r

    def next_int_bound(self, bound):
        assert bound > 0
        if (bound & -bound) == bound:
            return (bound * self.next(31)) >> 31
        while True:
            sample = self.next(31)
            modulo = sample % bound
            if sample - modulo + (bound - 1) >= 0:
                return modulo

    def next_double(self):
        # BitRandomSource.java:55-60：long×float 提升 → float 域乘。
        combined = (self.next(26) << 27) + self.next(27)
        return struct.unpack('<f', struct.pack('<f', combined * 1.110223e-16))[0]

    def next_long(self):
        return (self.next(32) << 32) + self.next(32)


def java_string_hash(s) -> int:
    h = 0
    for c in s.encode('utf-8'):
        h = (h * 31 + c) & 0xFFFFFFFF
        if h >= 1 << 31:
            h -= 1 << 32
    return h


GRAD = [
    [1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0],
    [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1],
    [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1],
    [1, 1, 0], [0, -1, 1], [-1, 1, 0], [0, -1, -1],
]


def smoothstep(t):
    # Mth.smoothstep。
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0)


def lerp3(ax, ay, az, d000, d100, d010, d110, d001, d101, d011, d111):
    # Mth.lerp3。
    x0 = d000 + ax * (d100 - d000)
    x1 = d010 + ax * (d110 - d010)
    x2 = d001 + ax * (d101 - d001)
    x3 = d011 + ax * (d111 - d011)
    y0 = x0 + ay * (x1 - x0)
    y1 = x2 + ay * (x3 - x2)
    return y0 + az * (y1 - y0)


class ImprovedNoise:
    def __init__(self, rng: LegacyRandom):
        self.xo = rng.next_double() * 256.0
        self.yo = rng.next_double() * 256.0
        self.zo = rng.next_double() * 256.0
        self.per = [i & 0xFF for i in range(256)]

    def p(self, i):
        return self.per[i & 0xFF] & 0xFF

    def gdot(self, h, x, y, z):
        g = GRAD[h & 15]
        return g[0] * x + g[1] * y + g[2] * z

    def noise(self, x, y, z):
        x += self.xo
        y += self.yo
        z += self.zo
        xf, yf, zf = math.floor(x), math.floor(y), math.floor(z)
        xr, yr, zr = x - xf, y - yf, z - zf
        x0 = self.p(xf)
        x1 = self.p(xf + 1)
        xy00 = self.p(x0 + yf)
        xy01 = self.p(x0 + yf + 1)
        xy10 = self.p(x1 + yf)
        xy11 = self.p(x1 + yf + 1)
        d000 = self.gdot(self.p(xy00 + zf), xr, yr, zr)
        d100 = self.gdot(self.p(xy10 + zf), xr - 1.0, yr, zr)
        d010 = self.gdot(self.p(xy01 + zf), xr, yr - 1.0, zr)
        d110 = self.gdot(self.p(xy11 + zf), xr - 1.0, yr - 1.0, zr)
        d001 = self.gdot(self.p(xy00 + zf + 1), xr, yr, zr - 1.0)
        d101 = self.gdot(self.p(xy10 + zf + 1), xr - 1.0, yr, zr - 1.0)
        d011 = self.gdot(self.p(xy01 + zf + 1), xr, yr - 1.0, zr - 1.0)
        d111 = self.gdot(self.p(xy11 + zf + 1), xr - 1.0, yr - 1.0, zr - 1.0)
        return lerp3(smoothstep(xr), smoothstep(yr), smoothstep(zr),
                     d000, d100, d010, d110, d001, d101, d011, d111)


def wrap(x):
    D = 3.3554432e7
    return x - math.floor(x / D + 0.5) * D


SECOND_SCALE = 1.0181268882175227
VALUE_FACTOR = 0.16666666666666666 / 0.2


def build(seed=2345):
    rng = LegacyRandom(seed)
    leaves = []
    for _ in range(2):
        factory = rng.next_long()
        h = java_string_hash("octave_0")
        # fromHashOf：int 符号扩展 XOR long（LegacyRandomSource.java:71-73）。
        h64 = h - (1 << 32) if h >= (1 << 31) else h
        mixed = h64 ^ factory
        # LegacyRandomSource 构造 = setSeed(mixed)（:10-13）。
        leaves.append(ImprovedNoise(LegacyRandom(mixed)))
    return leaves


if __name__ == '__main__':
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 2345
    first, second = build(seed)
    print(f"K-FLORA-MIRROR seed={seed} hash_octave_0={java_string_hash('octave_0')}")
    probes = [(0, 64, 0), (320, 64, 320), (-2455, 70, 841), (12345, 63, -6789)]
    f32 = lambda v: struct.unpack('<f', struct.pack('<f', v))[0]
    for (x, y, z) in probes:
        w = lambda n, cx, cy, cz: n.noise(wrap(cx), wrap(cy), wrap(cz))
        v = (w(first, x, y, z) + w(second, x * SECOND_SCALE, y * SECOND_SCALE, z * SECOND_SCALE)) * VALUE_FACTOR
        b = struct.unpack('<q', struct.pack('<d', v))[0] & 0xFFFFFFFFFFFFFFFF
        branch = 'low(tulip)' if v < f32(-0.8) else 'high(1/3)|default(dandelion)'
        print(f"K-FLORA-MIRROR probe=({x},{y},{z}) value_bits=0x{b:016x} decimal={v!r} branch={branch}")
