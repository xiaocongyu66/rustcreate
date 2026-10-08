// Terrain kernel: three-layer noise heightmap + dual-threshold 3D caves +
// deterministic cross-chunk tree projection.
//
// 常数按 /root/mc-ref/NOTES-terrain.md 从 MC 26.1 反编译源提取校准
// （NoiseRouterData / TerrainProvider / PerlinNoise / noise JSON /
// SurfaceRuleData），机制简化到 bloomcraft 基准：世界 0..255、海平面 96。
// 可步行尺度约束：波长 > ~300 格的通道一律 ÷8 压到 ≤256，保持相对比例
// （NOTES §7），否则小测试区（±6 chunk < 320 格）内通道近似常数、地形全平。
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
constexpr uint16_t AIR = 0;
constexpr uint16_t STONE = 1;
constexpr uint16_t DIRT = 2;
constexpr uint16_t GRASS = 3;
constexpr uint16_t SAND = 4;
constexpr uint16_t WATER = 5;
constexpr uint16_t LOG = 6;
constexpr uint16_t LEAVES = 7;
constexpr uint16_t BEDROCK = 10;
constexpr uint16_t SNOW_GRASS = 11;
constexpr uint16_t FLOWER_RED = 12;
constexpr uint16_t FLOWER_YELLOW = 13;

// ---- 通道波长（格）。括号内为 MC 26.1 原始波长，见 NOTES §1/§7 映射表 ----
constexpr float kWavelengthCont = 256.0f;    // continentalness 2048 ÷8（封顶）
constexpr float kWavelengthErosion = 256.0f; // erosion 2048 ÷8
constexpr float kWavelengthRidge = 64.0f;    // ridges 512 ÷8（保持 4:1 比例）
constexpr float kWavelengthDetail = 48.0f;   // jagged 基波 ≈43.7（1:1 可步行）
constexpr float kSpagXZ = 128.0f;            // spaghetti_3d_* 单倍频 128（1:1）
constexpr float kSpagY = 128.0f;
constexpr float kCheeseXZ = 256.0f;          // cave_cheese 384 → 封顶 256
constexpr float kCheeseY = 160.0f;           // 世界高 384→256，纵向再压缩

// ---- 倍频振幅序列：noise/*.json amplitudes 前缀（权重 ∝ amp/2^i，
// 截断项权重 <4%，见 NOTES §1）。0 表示跳过该倍频（erosion 原生置 0）。----
constexpr float kAmpCont[6] = {1.0f, 1.0f, 2.0f, 2.0f, 2.0f, 1.0f};  // 9→6 倍频
constexpr float kAmpErosion[5] = {1.0f, 1.0f, 0.0f, 1.0f, 1.0f};     // 5 倍频
constexpr float kAmpRidge[3] = {1.0f, 2.0f, 1.0f};                   // [1,2,1,0,0,0]
constexpr float kAmpCheese[5] = {0.5f, 1.0f, 2.0f, 1.0f, 2.0f};      // 9→5 倍频

// ---- 洞穴阈值（NOTES §3：NoiseRouterData spaghetti/cheese 公式）----
// 意面：MC max(|w1|,|w2|)+thickness<0，thickness=mappedNoise[-0.065,-0.088]
// → 雕刻带 |n| ∈ 0.065..0.088；深度线性替代 rarity 量化（简化）。
constexpr float kSpagThickMin = 0.065f;
constexpr float kSpagThickDeep = 0.023f;   // = 0.088 − 0.065，深处更宽
constexpr float kSpagDeepRefY = 40.0f;     // 深度基准（MC 深部洞穴集中带）
// 奶酪：MC 有效雕刻阈值 ≈ −0.27 + slide 正项 clamp(1.5−0.64·sloped,0,0.5)
// （均值 ≈0.33，浅部最大 0.5）→ 对称坐标下 0.60 / 浅部 0.66。
constexpr float kCheeseThresh = 0.60f;
constexpr float kCheeseThreshShallow = 0.66f;
// 洞口渐隐：analogue yClampedGradient(−10,30,+0.3,0) + 顶部 slide 封 −0.078125。
constexpr int kEntranceFade = 14;

constexpr uint64_t kSeedCont = 0x1111222233334444ull;
constexpr uint64_t kSeedErosion = 0x2468ACE013579BDFull;
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

// Surface height: continentalness + erosion-gated folded ridges + jagged
// detail. 三通道与振幅序列对应 MC 26.1 TerrainProvider.overworldOffset 的
// 样条输入轴（continents/erosion/ridges_folded，NOTES §2）；peaks_valleys
// 折叠逐系数等于 NoiseRouterData.peaksAndValleys。
float base_height(uint64_t seed, float wx, float wz) {
    // 大陆度：continentalness 振幅序列，波长 2048→256（÷8 封顶）。
    const float cont =
        mcvnoise::fbm2_w(seed ^ kSeedCont, wx / kWavelengthCont,
                         wz / kWavelengthCont, kAmpCont, 6) *
            2.0f -
        1.0f;  // [-1, 1]
    // 侵蚀度：erosion 振幅序列 [1,1,0,1,1]（第 3 倍频原生置 0）。
    const float ero =
        mcvnoise::fbm2_w(seed ^ kSeedErosion, wx / kWavelengthErosion,
                         wz / kWavelengthErosion, kAmpErosion, 5) *
            2.0f -
        1.0f;
    // 山脊：ridges 序列 [1,2,1]，波长 512→64（与大陆度保持 4:1）。
    const float r =
        mcvnoise::fbm2_w(seed ^ kSeedRidge, wx / kWavelengthRidge,
                         wz / kWavelengthRidge, kAmpRidge, 3) *
            2.0f -
        1.0f;
    const float ridge = std::max(0.0f, mcvnoise::peaks_valleys(r));
    // 门控 = 大陆度盆带（offset 样条 beach→high 档）× 低侵蚀档
    // （样条“低侵蚀→高 inland 偏移 0→0.7/1.0”，高侵蚀=平原压平山脊）。
    const float inland =
        std::clamp(cont * 1.6f + 0.15f, 0.0f, 1.0f) *
        (1.0f - 0.6f * std::max(0.0f, ero));
    // 锯齿细节：jagged 基波 ≈43.7 格 → 48，±3 格（等幅序列=fbm2 语义）。
    const float det =
        (mcvnoise::fbm2(seed ^ kSeedDetail, wx / kWavelengthDetail,
                        wz / kWavelengthDetail, 3) *
             2.0f -
         1.0f) *
        3.0f;
    // 盆带映射（NOTES §7）：深海盆底 ≈ SEA−22、平原均值 SEA+4（+4 与
    // GLOBAL_OFFSET=-0.50375 同向的简化抬高），山脊 inland 门控内最高 +38。
    return static_cast<float>(kSEA) + 4.0f + cont * 26.0f +
           ridge * inland * 38.0f + det;
}

// Forest density mask in [0, 1]: 0 = plains, 1 = dense forest.
float forest_mask(uint64_t seed, float wx, float wz) {
    return mcvnoise::fbm2(seed ^ kSeedForest, wx / 300.0f, wz / 300.0f, 2);
}

// Dual-threshold cave test (world coords)：spaghetti 管道（两路噪声取 max、
// |n| < thickness —— MC 26.1 NoiseRouterData entrances() 的 spaghetti3D 配方）
// + cheese 大洞（大尺度阈值，underground() 的 solidifiedCheese 折算）；
// 洞口随深度渐隐（entrances yClampedGradient 同构），海底保持水密
// （aquifer barrier 的简化）。见 /root/mc-ref/NOTES-terrain.md §3/§7。
bool carve_cave(uint64_t seed, int x, int y, int z, int surface, bool ocean) {
    if (y <= 2 || y > surface) {
        return false;
    }
    if (ocean && y > kSEA - 6) {
        return false;
    }

    // spaghetti：MC 为单倍频 128 格噪声按 rarity(0.75..2) 缩放坐标取
    // max(|w1|,|w2|)；我们以 2 倍频近似 rarity 量化 + roughness 抖动。
    const float a =
        mcvnoise::fbm3(seed ^ kSeedSpag1, static_cast<float>(x) / kSpagXZ,
                       static_cast<float>(y) / kSpagY,
                       static_cast<float>(z) / kSpagXZ, 2) *
            2.0f -
        1.0f;
    const float b =
        mcvnoise::fbm3(seed ^ kSeedSpag2, static_cast<float>(x) / kSpagXZ,
                       static_cast<float>(y) / kSpagY,
                       static_cast<float>(z) / kSpagXZ, 2) *
            2.0f -
        1.0f;
    const float deep =
        std::clamp((kSpagDeepRefY - static_cast<float>(y)) / kSpagDeepRefY,
                   0.0f, 1.0f);
    // 厚度带 = MC mappedNoise(SPAGHETTI_3D_THICKNESS, −0.065, −0.088) 幅值。
    const float thick = kSpagThickMin + kSpagThickDeep * deep;
    const float m = std::max(std::fabs(a), std::fabs(b));
    const bool tunnel = m < thick;

    // cheese：cave_cheese 振幅序列（前 3 倍频主导），xz 384→256 封顶。
    const float c =
        mcvnoise::fbm3_w(seed ^ kSeedCheese, static_cast<float>(x) / kCheeseXZ,
                         static_cast<float>(y) / kCheeseY,
                         static_cast<float>(z) / kCheeseXZ, kAmpCheese, 5) *
            2.0f -
        1.0f;
    // 浅部更难雕：对应 slide 正项 clamp(1.5−0.64·slopedCheese, 0, 0.5)。
    const bool cavern = c > (y < 40 ? kCheeseThresh : kCheeseThreshShallow);

    if (!tunnel && !cavern) {
        return false;
    }

    // 洞口渐隐：地表下 kEntranceFade 格内按深度收紧（tunnels 比 caverns 更易
    // 开口），替代 MC entrances 的 yClampedGradient(−10,30,+0.3,0) 平滑门。
    const int depth_below = surface - y;
    if (depth_below < kEntranceFade) {
        const float t = static_cast<float>(depth_below) / kEntranceFade;
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
void stamp_tree(uint64_t seed, uint16_t* voxels, const TreeInfo& t, int base_x,
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
                uint16_t& cell = voxels[vidx(lx, y, lz)];
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
                                        int32_t chunk_z, uint16_t* out_voxels,
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
                uint16_t id;
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
                const uint16_t id = out_voxels[vidx(x, y, z)];
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
