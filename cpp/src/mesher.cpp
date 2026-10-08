// Greedy voxel mesher: opaque and water passes over a 3x3 chunk
// neighbourhood (see mcv.h for the ABI contract).
//
// Two-phase output: quads accumulate in local std::vector storage, then one
// mcv_meshbuf_acquire from the thread's active pool backs the returned
// McvMeshBuffer (Rust owns it via CxxMeshBuffer and releases it exactly
// once).
//
// Vertex (24 B): pos f32x3 | uv u16x2 | tex_layer u16 | block_light u8 |
// sky_light u8 | ao u8 | flags u8 (bit0-2 face id, bit3 water top wave) |
// 2 B pad. Indices u32, CCW winding seen from outside.
//
// Block ids and tiles mirror crates/mcv_core/src/lib.rs BLOCKS.

#include "mcv.h"

#include <cstring>
#include <vector>

namespace {

/* Sentinel block id: unloaded neighbour chunk or below-world cell. Treated
 * as opaque, so faces pointing at it are never generated. Sits at the top
 * of the u16 id space so it cannot collide with real ids once the registry
 * grows toward ~1000 blocks. */
constexpr uint16_t kBarrier = 0xFFFF;

/* UV units per block edge: 65535 / 16, so a full 16-block quad fills the
 * u16 range exactly (repeat wrap comes from the sampler, values & 0xFFFF). */
constexpr float kUvPerBlock = 4095.9375f;
constexpr float kWaterTopSink = 0.1f;

enum FaceId {
    kFacePx = 0,
    kFaceNx = 1,
    kFacePy = 2,
    kFaceNy = 3,
    kFacePz = 4,
    kFaceNz = 5,
};

/* mcv_core BLOCKS order. */
enum BlockId {
    kAir = 0,
    kStone,
    kDirt,
    kGrass,
    kSand,
    kWater,
    kLog,
    kLeaves,
    kPlanks,
    kCobble,
    kBedrock,
    kSnowGrass,
    kFlowerRed,
    kFlowerYellow,
};

struct BlockInfo {
    bool opaque;
    bool liquid;
    bool geom; /* emits geometry in the opaque pass */
    uint16_t tiles[6]; /* [+X, -X, +Y, -Y, +Z, -Z] */
};

/* Generated from the same ci/gen-blocks.py run as mcv_core::BLOCKS, so id
 * order/flags/tile layers match the Rust registry by construction (tiles[]
 * are texture-array layer indices from tiles_manifest.json). Non-cube blocks
 * (stairs/slabs/fences/crosses, model_kind=1) are placeholder-rendered as
 * full cubes until the shape system lands. ids >= kBlocksCount fall back to
 * kUnknown. */
constexpr BlockInfo kBlocks[] = {
#include "blocks_gen.inc"
};
constexpr uint16_t kBlocksCount = sizeof(kBlocks) / sizeof(kBlocks[0]);

/* Per emitted corner (quad-local): fraction of quad width along the slice
 * grid u axis and height along the v axis. Orders are chosen so
 * cross(c1 - c0, c3 - c0) points along the outward face normal (CCW). */
constexpr int kCornerOrder[6][4][2] = {
    /* +X */ {{0, 0}, {0, 1}, {1, 1}, {1, 0}},
    /* -X */ {{0, 0}, {1, 0}, {1, 1}, {0, 1}},
    /* +Y */ {{0, 0}, {0, 1}, {1, 1}, {1, 0}},
    /* -Y */ {{0, 0}, {1, 0}, {1, 1}, {0, 1}},
    /* +Z */ {{0, 0}, {1, 0}, {1, 1}, {0, 1}},
    /* -Z */ {{0, 0}, {0, 1}, {1, 1}, {1, 0}},
};

struct Neighborhood {
    const uint16_t* voxels[9];
    const uint8_t* light[9];
};

struct QuadVertex {
    float pos[3];
    uint16_t uv[2];
    uint16_t tex_layer;
    uint8_t block_light;
    uint8_t sky_light;
    uint8_t ao;
    uint8_t flags;
    uint8_t pad[2];
};
static_assert(sizeof(QuadVertex) == 24, "vertex stride must be 24 bytes");

/* One slice-grid cell. `ao4` packs 4 corners x 2 bits, corner index
 * (b*2 + a) where a/b in {0,1} are the corner offsets toward +u/+v. */
struct Cell {
    uint16_t id;
    uint16_t tex;
    uint8_t sky;
    uint8_t blk;
    uint8_t ao4;
    uint8_t wave; /* water top face: above cell is air */
    uint8_t visible;
};

void neighborhood_init(const uint16_t* const voxels[9],
                       const uint8_t* const light[9], Neighborhood* out) {
    for (int i = 0; i < 9; ++i) {
        out->voxels[i] = voxels[i];
        out->light[i] = light[i];
    }
}

/* Block id at neighbourhood coords; kBarrier for unloaded neighbours and
 * y < 0; air for y >= 256 (world top is open). */
uint16_t block_at(const Neighborhood& n, int x, int y, int z) {
    if (y < 0) {
        return kBarrier;
    }
    if (y >= 256) {
        return kAir;
    }
    int cx = 0;
    int cz = 0;
    if (x < 0) {
        cx = -1;
        x += 16;
    } else if (x >= 16) {
        cx = 1;
        x -= 16;
    }
    if (z < 0) {
        cz = -1;
        z += 16;
    } else if (z >= 16) {
        cz = 1;
        z -= 16;
    }
    const uint16_t* arr = n.voxels[(cz + 1) * 3 + (cx + 1)];
    if (arr == nullptr) {
        return kBarrier;
    }
    return arr[static_cast<size_t>(y << 8) | static_cast<size_t>(z << 4) |
               static_cast<size_t>(x)];
}

/* Bounds-checked registry access for the (now u16-wide) id space: unknown
 * ids are treated as fully opaque, mirroring mcv_light::opacity. */
constexpr BlockInfo kUnknown{true, false, false, {0, 0, 0, 0, 0, 0}};

const BlockInfo& block_info(uint16_t id) {
    return id < kBlocksCount ? kBlocks[id] : kUnknown;
}

bool is_opaque(uint16_t id) {
    return id >= kBarrier || block_info(id).opaque;
}

/* Low nibble = block light, high nibble = sky light (mcv_core::ChunkLight).
 * Above the world: sky 15; below: 0. Missing light arrays read as 0. */
void light_at(const Neighborhood& n, int x, int y, int z, uint8_t* out_sky,
              uint8_t* out_block) {
    if (y >= 256) {
        *out_sky = 15;
        *out_block = 0;
        return;
    }
    if (y < 0) {
        *out_sky = 0;
        *out_block = 0;
        return;
    }
    int cx = 0;
    int cz = 0;
    if (x < 0) {
        cx = -1;
        x += 16;
    } else if (x >= 16) {
        cx = 1;
        x -= 16;
    }
    if (z < 0) {
        cz = -1;
        z += 16;
    } else if (z >= 16) {
        cz = 1;
        z -= 16;
    }
    const uint8_t* arr = n.light[(cz + 1) * 3 + (cx + 1)];
    if (arr == nullptr) {
        *out_sky = 0;
        *out_block = 0;
        return;
    }
    uint8_t v = arr[static_cast<size_t>(y << 8) | static_cast<size_t>(z << 4) |
                    static_cast<size_t>(x)];
    *out_block = static_cast<uint8_t>(v & 0x0F);
    *out_sky = static_cast<uint8_t>(v >> 4);
}

/* AO for one quad corner: side1/side2/corner are the three cells touching
 * the corner in the exposed (neighbor) layer. (ux,uy,uz)/(vx,vy,vz) are the
 * slice grid u/v axes in 3D; (nx,ny,nz) is the exposed neighbour cell. */
uint8_t corner_ao(const Neighborhood& n, int nx, int ny, int nz, int ux,
                  int uy, int uz, int vx, int vy, int vz, int a, int b) {
    int du = a ? 1 : -1;
    int dv = b ? 1 : -1;
    bool s1 = is_opaque(block_at(n, nx + du * ux, ny + du * uy, nz + du * uz));
    bool s2 = is_opaque(block_at(n, nx + dv * vx, ny + dv * vy, nz + dv * vz));
    bool c = is_opaque(block_at(n, nx + du * ux + dv * vx, ny + du * uy + dv * vy,
                                nz + du * uz + dv * vz));
    if (s1 && s2) {
        return 0;
    }
    return static_cast<uint8_t>(3 - (s1 + s2 + c));
}

/* Grid slice geometry per sweep axis: grid u/v sizes, layer count, grid
 * cell (u,v) -> chunk-local (x,y,z), and grid u/v axes in 3D. */
struct SliceGeom {
    int gu;
    int gv;
    int layers;
};

SliceGeom slice_geom(int axis) {
    if (axis == 1) {
        return {16, 16, 256}; /* u = x, v = z */
    }
    return {16, 256, 16}; /* axis 0: u = z, v = y; axis 2: u = x, v = y */
}

void cell_coords(int axis, int layer, int u, int v, int* out_x, int* out_y,
                 int* out_z) {
    if (axis == 0) {
        *out_x = layer;
        *out_y = v;
        *out_z = u;
    } else if (axis == 1) {
        *out_x = u;
        *out_y = layer;
        *out_z = v;
    } else {
        *out_x = u;
        *out_y = v;
        *out_z = layer;
    }
}

void grid_axes(int axis, int* ux, int* uy, int* uz, int* vx, int* vy,
               int* vz) {
    if (axis == 0) {
        *ux = 0; *uy = 0; *uz = 1; /* u = z */
        *vx = 0; *vy = 1; *vz = 0; /* v = y */
    } else if (axis == 1) {
        *ux = 1; *uy = 0; *uz = 0; /* u = x */
        *vx = 0; *vy = 0; *vz = 1; /* v = z */
    } else {
        *ux = 1; *uy = 0; *uz = 0; /* u = x */
        *vx = 0; *vy = 1; *vz = 0; /* v = y */
    }
}

uint16_t uv_coord(float blocks) {
    float scaled = blocks * kUvPerBlock + 0.5f;
    if (scaled >= 65535.0f) {
        return 65535;
    }
    return static_cast<uint16_t>(scaled);
}

bool same_key(const Cell& a, const Cell& b) {
    return a.visible && b.visible && a.id == b.id && a.tex == b.tex &&
           a.sky == b.sky && a.blk == b.blk && a.ao4 == b.ao4 &&
           a.wave == b.wave;
}

void push_vertex(std::vector<QuadVertex>& verts, float x, float y, float z,
                 uint16_t uu, uint16_t vv, const Cell& c, uint8_t ao,
                 uint8_t flags) {
    QuadVertex q;
    q.pos[0] = x;
    q.pos[1] = y;
    q.pos[2] = z;
    q.uv[0] = uu;
    q.uv[1] = vv;
    q.tex_layer = c.tex;
    q.block_light = c.blk;
    q.sky_light = c.sky;
    q.ao = ao;
    q.flags = flags;
    q.pad[0] = 0;
    q.pad[1] = 0;
    verts.push_back(q);
}

/* Emits one merged quad: corners follow kCornerOrder[face], AO comes from
 * the anchor cell's packed corner values (equal across the rect by key). */
void emit_quad(std::vector<QuadVertex>& verts, std::vector<uint32_t>& indices,
               bool water_pass, int axis, int dir, int layer, int u0, int v0,
               int wq, int hq, const Cell& anchor) {
    const int face = axis * 2 + dir;
    float plane = static_cast<float>(layer) + (dir == 0 ? 1.0f : 0.0f);
    if (water_pass && face == kFacePy && anchor.wave != 0) {
        plane -= kWaterTopSink;
    }
    const uint16_t u_max = uv_coord(static_cast<float>(wq));
    const uint16_t v_max = uv_coord(static_cast<float>(hq));
    const uint8_t flags =
        static_cast<uint8_t>(face | (anchor.wave != 0 ? 0x08 : 0x00));

    const uint32_t base = static_cast<uint32_t>(verts.size());
    for (int k = 0; k < 4; ++k) {
        const int a = kCornerOrder[face][k][0];
        const int b = kCornerOrder[face][k][1];
        const float uu = static_cast<float>(u0 + a * wq);
        const float vv = static_cast<float>(v0 + b * hq);
        float x;
        float y;
        float z;
        if (axis == 0) {
            x = plane;
            y = vv;
            z = uu;
        } else if (axis == 1) {
            x = uu;
            y = plane;
            z = vv;
        } else {
            x = uu;
            y = vv;
            z = plane;
        }
        const uint16_t cu = a != 0 ? u_max : 0;
        const uint16_t cv = b != 0 ? v_max : 0;
        const uint8_t ao =
            static_cast<uint8_t>((anchor.ao4 >> (2 * (b * 2 + a))) & 0x3);
        push_vertex(verts, x, y, z, cu, cv, anchor, ao, flags);
    }
    const uint32_t i0 = base;
    const uint32_t i1 = base + 1;
    const uint32_t i2 = base + 2;
    const uint32_t i3 = base + 3;
    const uint32_t quad[6] = {i0, i1, i2, i0, i2, i3};
    indices.insert(indices.end(), quad, quad + 6);
}

/* Standard greedy sweep: per (axis, direction, layer), expand width along
 * the grid u axis first, then height along v; cells merge only when the
 * full quad key matches (block, tile, sky/block light, packed AO, wave). */
void build_pass(const Neighborhood& n, bool water_pass,
                std::vector<QuadVertex>& verts,
                std::vector<uint32_t>& indices) {
    for (int axis = 0; axis < 3; ++axis) {
        const SliceGeom g = slice_geom(axis);
        const size_t grid_len = static_cast<size_t>(g.gu) * g.gv;
        std::vector<Cell> cells(grid_len);
        std::vector<uint8_t> visited(grid_len);
        int ux, uy, uz, vx, vy, vz;
        grid_axes(axis, &ux, &uy, &uz, &vx, &vy, &vz);

        for (int dir = 0; dir < 2; ++dir) {
            const int face = axis * 2 + dir;
            const int nx_step = axis == 0 ? (dir == 0 ? 1 : -1) : 0;
            const int ny_step = axis == 1 ? (dir == 0 ? 1 : -1) : 0;
            const int nz_step = axis == 2 ? (dir == 0 ? 1 : -1) : 0;

            for (int layer = 0; layer < g.layers; ++layer) {
                std::memset(cells.data(), 0, grid_len * sizeof(Cell));

                for (int v = 0; v < g.gv; ++v) {
                    for (int u = 0; u < g.gu; ++u) {
                        int x, y, z;
                        cell_coords(axis, layer, u, v, &x, &y, &z);
                        const uint16_t id = block_at(n, x, y, z);
                        if (id >= kBarrier) {
                            continue;
                        }
                        const int nx = x + nx_step;
                        const int ny = y + ny_step;
                        const int nz = z + nz_step;
                        const uint16_t nb = block_at(n, nx, ny, nz);

                        bool visible = false;
                        uint8_t wave = 0;
                        if (water_pass) {
                            /* water vs water (or barrier) shows nothing */
                            if (id == kWater && nb < kBarrier && nb != kWater) {
                                visible = true;
                                if (face == kFacePy) {
                                    wave = nb == kAir ? 1 : 0;
                                }
                            }
                        } else {
                            visible = block_info(id).geom && !is_opaque(nb);
                        }
                        if (!visible) {
                            continue;
                        }

                        Cell c;
                        std::memset(&c, 0, sizeof(Cell));
                        c.id = id;
                        c.tex = block_info(id).tiles[face];
                        c.wave = wave;
                        c.visible = 1;
                        light_at(n, nx, ny, nz, &c.sky, &c.blk);
                        if (water_pass) {
                            c.ao4 = 0xFF; /* all corners fully lit */
                        } else {
                            for (int b = 0; b < 2; ++b) {
                                for (int a = 0; a < 2; ++a) {
                                    c.ao4 = static_cast<uint8_t>(
                                        c.ao4 | (corner_ao(n, nx, ny, nz, ux,
                                                            uy, uz, vx, vy,
                                                            vz, a, b)
                                                 << (2 * (b * 2 + a))));
                                }
                            }
                        }
                        cells[static_cast<size_t>(v) * g.gu + u] = c;
                    }
                }

                std::memset(visited.data(), 0, grid_len);
                for (int v = 0; v < g.gv; ++v) {
                    for (int u = 0; u < g.gu; ++u) {
                        const size_t i =
                            static_cast<size_t>(v) * g.gu + u;
                        if (visited[i] || !cells[i].visible) {
                            continue;
                        }
                        const Cell key = cells[i];

                        int wq = 1;
                        while (u + wq < g.gu) {
                            const size_t j =
                                static_cast<size_t>(v) * g.gu + u + wq;
                            if (visited[j] || !same_key(key, cells[j])) {
                                break;
                            }
                            ++wq;
                        }

                        int hq = 1;
                        bool grew = true;
                        while (v + hq < g.gv && grew) {
                            for (int k = 0; k < wq; ++k) {
                                const size_t j = static_cast<size_t>(v + hq) *
                                                     g.gu +
                                                 u + k;
                                if (visited[j] || !same_key(key, cells[j])) {
                                    grew = false;
                                    break;
                                }
                            }
                            if (grew) {
                                ++hq;
                            }
                        }

                        for (int dv = 0; dv < hq; ++dv) {
                            for (int du = 0; du < wq; ++du) {
                                visited[static_cast<size_t>(v + dv) * g.gu +
                                        u + du] = 1;
                            }
                        }
                        emit_quad(verts, indices, water_pass, axis, dir, layer,
                                  u, v, wq, hq, key);
                    }
                }
            }
        }
    }
}

}  // namespace

/* Defined in mempool.cpp: thread-local pool installed by mcv_set_active_pool.
 * Internal helper (not part of the public mcv.h surface) so mcv_mesh_build
 * can allocate its output buffer from the active pool. */
extern "C" McvPool* mcv_active_pool(void);

extern "C" {

int32_t mcv_mesh_build(const uint16_t* const voxels[9],
                       const uint8_t* const light[9], uint32_t mesh_kind,
                       McvMeshBuffer* out) {
    if (out == nullptr || voxels == nullptr || light == nullptr) {
        return MCV_ERR_NULL_ARG;
    }
    if (mesh_kind > 1) {
        return MCV_ERR_BAD_ARG;
    }

    Neighborhood n;
    neighborhood_init(voxels, light, &n);

    std::vector<QuadVertex> verts;
    std::vector<uint32_t> indices;
    build_pass(n, mesh_kind == 1, verts, indices);

    uint32_t vertex_cap = static_cast<uint32_t>(verts.size());
    uint32_t index_cap = static_cast<uint32_t>(indices.size());
    if (vertex_cap == 0) {
        /* Empty mesh: the pool rejects a (0, 0) acquire, so take the
         * smallest block and report zero counts. */
        vertex_cap = 1;
        index_cap = 1;
    }

    McvMeshBuffer buf;
    const int32_t rc =
        mcv_meshbuf_acquire(mcv_active_pool(), vertex_cap, index_cap, &buf);
    if (rc != MCV_OK) {
        return rc;
    }

    if (!verts.empty()) {
        std::memcpy(buf.vertex_data, verts.data(),
                    verts.size() * sizeof(QuadVertex));
        std::memcpy(buf.indices, indices.data(),
                    indices.size() * sizeof(uint32_t));
    }
    buf.vertex_count = static_cast<uint32_t>(verts.size());
    buf.index_count = static_cast<uint32_t>(indices.size());
    *out = buf;
    return MCV_OK;
}

}  // extern "C"
