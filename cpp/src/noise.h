// Deterministic value noise + fbm. Integer-hash based so results are
// identical across platforms and compilers (float ops stay in the same
// order per call site).
//
// 倍频常数参照 MC 26.1 synth/PerlinNoise.getValue：每倍频频率 ×2.0、
// 值权重 ÷2.0（相对权重 ∝ amplitude[i]/2^i）；提取笔记见
// /root/mc-ref/NOTES-terrain.md §1（机制与常数，非代码搬运）。

#ifndef MCV_NOISE_H
#define MCV_NOISE_H

#include <cmath>
#include <cstdint>

namespace mcvnoise {

// MC PerlinNoise.getValue 的两个跨噪声恒定常数（全 26.1 noise JSON 只改
// amplitude 表，不改这两个递推常数）。
constexpr float kFreqRatio = 2.0f;  // 频率比：每倍频波长 ÷2
constexpr float kAmpDecay = 0.5f;   // 振幅衰减：每倍频值权重 ×0.5

inline uint64_t splitmix64(uint64_t x) {
    x += 0x9E3779B97F4A7C15ull;
    x = (x ^ (x >> 30)) * 0xBF58476D1CE4E5B9ull;
    x = (x ^ (x >> 27)) * 0x94D049BB133111EBull;
    return x ^ (x >> 31);
}

inline float hash01(uint64_t seed, int64_t x, int64_t y, int64_t z) {
    uint64_t h = splitmix64(seed ^ (static_cast<uint64_t>(x) * 0x9E3779B1ull) ^
                             (static_cast<uint64_t>(y) * 0x85EBCA77ull) ^
                             (static_cast<uint64_t>(z) * 0xC2B2AE3Dull));
    return static_cast<float>(h >> 40) * (1.0f / 16777216.0f);
}

inline float fade(float t) {
    return t * t * t * (t * (t * 6.0f - 15.0f) + 10.0f);
}

inline float lerp(float a, float b, float t) { return a + (b - a) * t; }

// 2D value noise in [0, 1).
inline float value2(uint64_t seed, float x, float y) {
    float fx = std::floor(x);
    float fy = std::floor(y);
    float tx = fade(x - fx);
    float ty = fade(y - fy);
    int64_t ix = static_cast<int64_t>(fx);
    int64_t iy = static_cast<int64_t>(fy);
    float c00 = hash01(seed, ix, iy, 0);
    float c10 = hash01(seed, ix + 1, iy, 0);
    float c01 = hash01(seed, ix, iy + 1, 0);
    float c11 = hash01(seed, ix + 1, iy + 1, 0);
    return lerp(lerp(c00, c10, tx), lerp(c01, c11, tx), ty);
}

// 3D value noise in [0, 1).
inline float value3(uint64_t seed, float x, float y, float z) {
    float fx = std::floor(x);
    float fy = std::floor(y);
    float fz = std::floor(z);
    float tx = fade(x - fx);
    float ty = fade(y - fy);
    float tz = fade(z - fz);
    int64_t ix = static_cast<int64_t>(fx);
    int64_t iy = static_cast<int64_t>(fy);
    int64_t iz = static_cast<int64_t>(fz);
    float c000 = hash01(seed, ix, iy, iz);
    float c100 = hash01(seed, ix + 1, iy, iz);
    float c010 = hash01(seed, ix, iy + 1, iz);
    float c110 = hash01(seed, ix + 1, iy + 1, iz);
    float c001 = hash01(seed, ix, iy, iz + 1);
    float c101 = hash01(seed, ix + 1, iy, iz + 1);
    float c011 = hash01(seed, ix, iy + 1, iz + 1);
    float c111 = hash01(seed, ix + 1, iy + 1, iz + 1);
    float a = lerp(lerp(c000, c100, tx), lerp(c010, c110, tx), ty);
    float b = lerp(lerp(c001, c101, tx), lerp(c011, c111, tx), ty);
    return lerp(a, b, tz);
}

// Fractal Brownian motion (2D), result in [0, 1).
// 倍频语义与 MC 26.1 PerlinNoise.getValue 一致：频率比 2.0、振幅衰减 0.5
// （等价于 amplitude 全 1 序列，权重 ∝ amp[i]/2^i）。见 /root/mc-ref/NOTES-terrain.md §1。
inline float fbm2(uint64_t seed, float x, float y, int octaves) {
    float sum = 0.0f;
    float amp = 1.0f;
    float norm = 0.0f;
    for (int i = 0; i < octaves; ++i) {
        sum += value2(seed + static_cast<uint64_t>(i) * 0x1000'0000ull, x, y) * amp;
        norm += amp;
        amp *= kAmpDecay;
        x *= kFreqRatio;
        y *= kFreqRatio;
    }
    return sum / norm;
}

// Fractal Brownian motion (3D), result in [0, 1). 倍频语义同上。
inline float fbm3(uint64_t seed, float x, float y, float z, int octaves) {
    float sum = 0.0f;
    float amp = 1.0f;
    float norm = 0.0f;
    for (int i = 0; i < octaves; ++i) {
        sum += value3(seed + static_cast<uint64_t>(i) * 0x1000'0000ull, x, y, z) * amp;
        norm += amp;
        amp *= kAmpDecay;
        x *= kFreqRatio;
        y *= kFreqRatio;
        z *= kFreqRatio;
    }
    return sum / norm;
}

// 振幅序列版 fbm（2D/3D），result in [0, 1)。
// 对应 MC noise/*.json 的 amplitudes 表：第 i 倍频权重 = amps[i]·kAmpDecay^i
// （PerlinNoise.getValue：频率 ×2/倍频、值权重 ÷2/倍频，再乘 amplitude[i]）。
// amps[i] = 0 即跳过该倍频（如 erosion [1,1,0,1,1]）。归一化保证有界。
inline float fbm2_w(uint64_t seed, float x, float y, const float* amps,
                    int count) {
    float sum = 0.0f;
    float norm = 0.0f;
    float amp = 1.0f;
    for (int i = 0; i < count; ++i) {
        const float w = amps[i] * amp;
        sum += value2(seed + static_cast<uint64_t>(i) * 0x1000'0000ull, x, y) * w;
        norm += w;
        amp *= kAmpDecay;
        x *= kFreqRatio;
        y *= kFreqRatio;
    }
    return sum / norm;
}

inline float fbm3_w(uint64_t seed, float x, float y, float z, const float* amps,
                    int count) {
    float sum = 0.0f;
    float norm = 0.0f;
    float amp = 1.0f;
    for (int i = 0; i < count; ++i) {
        const float w = amps[i] * amp;
        sum += value3(seed + static_cast<uint64_t>(i) * 0x1000'0000ull, x, y, z) * w;
        norm += w;
        amp *= kAmpDecay;
        x *= kFreqRatio;
        y *= kFreqRatio;
        z *= kFreqRatio;
    }
    return sum / norm;
}

// Peaks/valleys fold from MC 26.1 NoiseRouterData.peaksAndValleys: input r in
// [-1, 1], output in [-1, 1] with +1 ridge lines at |r| = 2/3. 逐系数一致：
// -3·(||r|−2/3| − 1/3)。
inline float peaks_valleys(float r) {
    return -3.0f * (std::fabs(std::fabs(r) - (2.0f / 3.0f)) - (1.0f / 3.0f));
}

}  // namespace mcvnoise

#endif /* MCV_NOISE_H */
