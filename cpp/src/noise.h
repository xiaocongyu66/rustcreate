// Deterministic value noise + fbm. Integer-hash based so results are
// identical across platforms and compilers (float ops stay in the same
// order per call site).

#ifndef MCV_NOISE_H
#define MCV_NOISE_H

#include <cmath>
#include <cstdint>

namespace mcvnoise {

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
inline float fbm2(uint64_t seed, float x, float y, int octaves) {
    float sum = 0.0f;
    float amp = 1.0f;
    float norm = 0.0f;
    for (int i = 0; i < octaves; ++i) {
        sum += value2(seed + static_cast<uint64_t>(i) * 0x1000'0000ull, x, y) * amp;
        norm += amp;
        amp *= 0.5f;
        x *= 2.0f;
        y *= 2.0f;
    }
    return sum / norm;
}

// Fractal Brownian motion (3D), result in [0, 1).
inline float fbm3(uint64_t seed, float x, float y, float z, int octaves) {
    float sum = 0.0f;
    float amp = 1.0f;
    float norm = 0.0f;
    for (int i = 0; i < octaves; ++i) {
        sum += value3(seed + static_cast<uint64_t>(i) * 0x1000'0000ull, x, y, z) * amp;
        norm += amp;
        amp *= 0.5f;
        x *= 2.0f;
        y *= 2.0f;
        z *= 2.0f;
    }
    return sum / norm;
}

// Peaks/valleys fold from MC 26.1 NoiseRouterData: input r in [-1, 1],
// output in [-1, 1] with +1 ridge lines at |r| = 2/3.
inline float peaks_valleys(float r) {
    return -3.0f * (std::fabs(std::fabs(r) - (2.0f / 3.0f)) - (1.0f / 3.0f));
}

}  // namespace mcvnoise

#endif /* MCV_NOISE_H */
