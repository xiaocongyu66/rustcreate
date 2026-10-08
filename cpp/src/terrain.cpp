// Terrain kernel: three-layer noise heightmap + dual-threshold 3D caves +
// deterministic cross-chunk tree projection.
//
// Constants follow /root/mc-ref/NOTES.md (decompiled MC 26.1 mechanisms,
// simplified to the bloomcraft baseline: world 0..255, sea level 96).
//
// Block ids mirror crates/mcv_core/src/lib.rs BLOCKS.

#include "mcv.h"
#include "noise.h"

#include <algorithm>
#include <cstdlib>

namespace {

constexpr int kSY = 256;
constexpr int kSEA = 96;

// Block ids (mcv_core BLOCKS order).
constexpr uint8_t AIR = 0;
constexpr uint8_t STONE = 1;
constexpr uint8_t DIRT = 2;
constexpr uint8_t GRASS = 3;
constexpr uint8_t SAND = 4;
constexpr uint8_t WATER = 5;
constexpr uint8_t LOG = 6;
constexpr uint8_t LEAVES = 7;
constexpr uint8_t BEDROCK = 10;
constexpr uint8_t SNOW_GRASS = 11;
constexpr uint8_t FLOWER_RED = 12;
constexpr uint8_t FLOWER_YELLOW = 13;

constexpr uint64_t kSeedCont = 0x1111222233334444ull;
constexpr uint64_t kSeedRidge = 0x5555666677778888ull;
constexpr uint64_t kSeedDetail = 0x9999AAAABBBBCCCCull;
constexpr uint64_t kSeedSpag1 = 0x5EED0000C0DE0001ull;
constexpr uint64_t kSeedSpag2 = 0x5EED0000C0DE0002ull;
constexpr uint64_t kSeedCheese = 0x5EED0000C0DE0003ull;
constexpr uint64_t kSeedForest = 0xF0FEF0FEF0FEF0FEull;
constexpr uint64_t kSeedTree = 0x7EE57EE57EE57EE5ull;
constexpr uint64_t kSeedFlower = 0xF10BF10BF10BF10Bull;
constexpr uint64_t kSeedMisc = 0x0D00D00D0D00D00Dull;

inline size_t vidx(int x, int y, int z) {
    return (static_cast<size_t>(y) << 8) | (static_cast<size_t>(z) << 4) |
           static_cast<size_t>(x);
}

inline int floor_div(int v, int d) {
    int q = v / d;
    if ((v % d) != 0 && ((v < 0) != (d < 0))) {
        --q;
    }
    return q;
}

// Surface height: continents + folded ridges + detail (NOTES: 26.1
// peaks_valleys fold applied to the ridge channel).
float base_height(uint64_t seed, float wx, float wz) {
    const float cont =
        mcvnoise::fbm2(seed ^ kSeedCont, wx / 700.0f, wz / 700.0f, 3) * 2.0f -
        1.0f;  // [-1, 1]
    const float r =
        mcvnoise::fbm2(seed ^ kSeedRidge, wx / 220.0f, wz / 220.0f, 4) * 2.0f -
        1.0f;
    const float ridge = std::max(0.0f, mcvnoise::peaks_valleys(r));
    const float inland = std::clamp(cont * 1.6f + 0.15f, 0.0f, 1.0f);
    const float det =
        (mcvnoise::fbm2(seed ^ kSeedDetail, wx / 40.0f, wz / 40.0f, 3) * 2.0f -
         1.0f) *
        3.0f;
    return static_cast<float>(kSEA) + 2.0f + cont * 38.0f +
           ridge * inland * 46.0f + det;
}

// Forest density mask in [0, 1]: 0 = plains, 1 = dense forest.
float forest_mask(uint64_t seed, float wx, float wz) {
    return mcvnoise::fbm2(seed ^ kSeedForest, wx / 300.0f, wz / 300.0f, 2);
}

// Dual-threshold cave test (world coords): spaghetti tunnels (two-noise max,
// abs minus thickness — the MC 26.1 spaghetti recipe) + cheese caverns
// (large-scale threshold); entrances fade near the surface, ocean floors
// stay watertight.
bool carve_cave(uint64_t seed, int x, int y, int z, int surface, bool ocean) {
    if (y <= 2 || y > surface) {
        return false;
    }
    if (ocean && y > kSEA - 6) {
        return false;
    }

    const float a =
        mcvnoise::fbm3(seed ^ kSeedSpag1, static_cast<float>(x) / 110.0f,
                       static_cast<float>(y) / 70.0f,
                       static_cast<float>(z) / 110.0f, 2) *
            2.0f -
        1.0f;
    const float b =
        mcvnoise::fbm3(seed ^ kSeedSpag2, static_cast<float>(x) / 110.0f,
                       static_cast<float>(y) / 70.0f,
                       static_cast<float>(z) / 110.0f, 2) *
            2.0f -
        1.0f;
    const float deep = std::clamp((40.0f - static_cast<float>(y)) / 40.0f, 0.0f, 1.0f);
    const float thick = 0.055f + 0.02f * deep;
    const float m = std::max(std::fabs(a), std::fabs(b));
    const bool tunnel = m < thick;

    const float c =
        mcvnoise::fbm3(seed ^ kSeedCheese, static_cast<float>(x) / 280.0f,
                       static_cast<float>(y) / 160.0f,
                       static_cast<float>(z) / 280.0f, 2) *
            2.0f -
        1.0f;
    const bool cavern = c > (y < 40 ? 0.60f : 0.66f);

    if (!tunnel && !cavern) {
        return false;
    }

    // Entrance gradient: carve-outs fade within 14 blocks under the surface
    // (tunnels break through more often than caverns).
    const int depth_below = surface - y;
    if (depth_below < 14) {
        const float t = static_cast<float>(depth_below) / 14.0f;
        const float gate = mcvnoise::hash01(seed ^ kSeedMisc, x, y, z);
        const float keep = tunnel ? (0.35f + 0.65f * t) : (0.75f * t);
        if (gate > keep) {
            return false;
        }
    }
    return true;
}

struct TreeInfo {
    int x;  // world coords
    int z;
    int base_y;  // surface y (trunk starts at base_y + 1)
    int height;  // trunk blocks 4..6
};

// Deterministic tree lookup for one 5x5 jittered cell.
bool tree_in_cell(uint64_t seed, int cell_x, int cell_z, TreeInfo* out) {
    const float gate = mcvnoise::hash01(seed ^ kSeedTree, cell_x, cell_z, 1);
    const float jx = mcvnoise::hash01(seed ^ kSeedTree, cell_x, cell_z, 2);
    const float jz = mcvnoise::hash01(seed ^ kSeedTree, cell_x, cell_z, 3);
    const float jh = mcvnoise::hash01(seed ^ kSeedTree, cell_x, cell_z, 4);
    const int wx = cell_x * 5 + static_cast<int>(jx * 5.0f);
    const int wz = cell_z * 5 + static_cast<int>(jz * 5.0f);
    const int sy = static_cast<int>(base_height(
        seed, static_cast<float>(wx), static_cast<float>(wz)));
    if (sy <= kSEA || sy > 148) {
        return false;  // no trees on beaches/water or snowy peaks
    }
    const float density =
        0.010f +
        forest_mask(seed, static_cast<float>(wx), static_cast<float>(wz)) * 0.045f;
    if (gate >= density) {
        return false;
    }
    out->x = wx;
    out->z = wz;
    out->base_y = sy;
    out->height = 4 + static_cast<int>(jh * 3.0f);
    return true;
}

// Writes canopy + trunk cells that fall inside this chunk (world->local
// clipping makes cross-chunk projection implicit).
void stamp_tree(uint64_t seed, uint8_t* voxels, const TreeInfo& t, int base_x,
                int base_z) {
    const int top = t.base_y + t.height;
    for (int dy = -2; dy <= 1; ++dy) {
        const int y = top + dy;
        if (y < 0 || y >= kSY) {
            continue;
        }
        const int r = (dy <= -1) ? 2 : 1;
        for (int dx = -r; dx <= r; ++dx) {
            for (int dz = -r; dz <= r; ++dz) {
                if (dy == 1 && std::abs(dx) + std::abs(dz) > 1) {
                    continue;  // cap is a plus shape
                }
                if (dy <= -1 && std::abs(dx) == r && std::abs(dz) == r &&
                    mcvnoise::hash01(seed ^ kSeedMisc, t.x + dx, y,
                                     t.z + dz) < 0.5f) {
                    continue;  // clip lower-layer corners
                }
                const int lx = t.x + dx - base_x;
                const int lz = t.z + dz - base_z;
                if (lx < 0 || lx > 15 || lz < 0 || lz > 15) {
                    continue;
                }
                uint8_t& cell = voxels[vidx(lx, y, lz)];
                if (cell == AIR) {
                    cell = LEAVES;
                }
            }
        }
    }
    const int tx = t.x - base_x;
    const int tz = t.z - base_z;
    if (tx >= 0 && tx <= 15 && tz >= 0 && tz <= 15) {
        for (int y = t.base_y + 1; y <= top && y < kSY; ++y) {
            voxels[vidx(tx, y, tz)] = LOG;
        }
    }
}

}  // namespace

extern "C" int32_t mcv_terrain_generate(uint64_t seed, int32_t chunk_x,
                                        int32_t chunk_z, uint8_t* out_voxels,
                                        uint8_t* out_heightmap) {
    if (out_voxels == nullptr || out_heightmap == nullptr) {
        return MCV_ERR_NULL_ARG;
    }
    const int base_x = chunk_x * 16;
    const int base_z = chunk_z * 16;

    // Pass 1: columns — strata, water, bedrock, caves.
    for (int z = 0; z < 16; ++z) {
        for (int x = 0; x < 16; ++x) {
            const int wx = base_x + x;
            const int wz = base_z + z;
            const int surface = std::clamp(
                static_cast<int>(base_height(seed, static_cast<float>(wx),
                                             static_cast<float>(wz))),
                4, kSY - 10);
            const bool ocean = surface <= kSEA;
            const bool beach = surface <= kSEA + 1;
            const bool snowy = surface > 142;
            const int top = std::max(surface, kSEA);

            for (int y = 0; y <= top; ++y) {
                uint8_t id;
                if (y == 0 ||
                    (y <= 2 &&
                     mcvnoise::hash01(seed ^ kSeedMisc, wx, y, wz) < 0.5f)) {
                    id = BEDROCK;
                } else if (y > surface) {
                    id = (y <= kSEA) ? WATER : AIR;
                } else if (y == surface) {
                    id = beach ? SAND : (snowy ? SNOW_GRASS : GRASS);
                } else if (y >= surface - 3) {
                    id = beach ? SAND : DIRT;
                } else {
                    id = STONE;
                }

                if (id != AIR && id != BEDROCK && id != WATER &&
                    carve_cave(seed, wx, y, wz, surface, ocean)) {
                    id = AIR;
                }
                out_voxels[vidx(x, y, z)] = id;
            }

            // Flowers on intact grass columns.
            if (out_voxels[vidx(x, surface, z)] == GRASS && surface + 1 < kSY &&
                out_voxels[vidx(x, surface + 1, z)] == AIR) {
                const float f = mcvnoise::hash01(seed ^ kSeedFlower, wx, 7, wz);
                if (f < 0.006f) {
                    out_voxels[vidx(x, surface + 1, z)] =
                        (f < 0.003f) ? FLOWER_RED : FLOWER_YELLOW;
                }
            }
        }
    }

    // Pass 2: trees from this chunk and its 8 neighbours — canopy projection
    // across chunk borders is deterministic because tree placement is a pure
    // function of world-space cells.
    const int c0x = floor_div(base_x - 2, 5);
    const int c1x = floor_div(base_x + 17, 5);
    const int c0z = floor_div(base_z - 2, 5);
    const int c1z = floor_div(base_z + 17, 5);
    for (int cell_z = c0z; cell_z <= c1z; ++cell_z) {
        for (int cell_x = c0x; cell_x <= c1x; ++cell_x) {
            TreeInfo t{};
            if (tree_in_cell(seed, cell_x, cell_z, &t)) {
                stamp_tree(seed, out_voxels, t, base_x, base_z);
            }
        }
    }

    // Pass 3: heightmap = topmost light-blocking y + 1 (air/water/flowers
    // don't block direct sky light; leaves do, for tree shade).
    for (int z = 0; z < 16; ++z) {
        for (int x = 0; x < 16; ++x) {
            int y = kSY - 1;
            for (; y > 0; --y) {
                const uint8_t id = out_voxels[vidx(x, y, z)];
                if (id != AIR && id != WATER && id != FLOWER_RED &&
                    id != FLOWER_YELLOW) {
                    break;
                }
            }
            out_heightmap[(z << 4) | x] = static_cast<uint8_t>(y + 1);
        }
    }

    return MCV_OK;
}
