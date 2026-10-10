// Greedy voxel mesher: opaque and water passes over a 3x3 chunk
// neighbourhood (see mcv.h for the ABI contract). Cube blocks take the
// greedy merge; non-cube shapes (cross/torch/fence/slab/stairs) go through
// the emit_shapes() parametric template path keyed by the state nibble.
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

/* 体素 u16 打包（同 mcv_core::BlockId）：bit0-11 = 方块 id，bit12-15 =
 * 状态 nibble（半砖上下/楼梯朝向）。kBarrier 经 MCV_ID 得 0x0FFF（未注册
 * → kUnknown 不透明），且拦截先行于取模，哨兵语义不变。 */
#define MCV_ID(v) ((v) &0x0FFFu)
#define MCV_STATE(v) (((v) >> 12) & 0xFu)

/* UV units per block edge: one block edge spans ONE FULL TILE = 4096
 * units; the shader divides by 4096.0 to get tile counts and the sampler
 * repeats (vanilla bake rule: every face covers the sprite's entire 0..1
 * uv range, FaceBakery.java:26-35/166). Units are tile-normalized, not
 * u16-normalized: a merged greedy quad spanning N blocks carries N*4096
 * units, which u16 holds for N ≤ 15 — greedy growth is capped at
 * kMaxMerge for exactly this reason. The first fix used 65535/block, so
 * merged faces ≥2 blocks overflowed u16 and the mod-65536 wrap corrupted
 * the linearly-interpolated coordinate → one tile stretched over the
 * whole merged face ("mosaic stretch", 2026-10-10 device round 2). */
constexpr float kUvPerBlock = 4096.0f;
/* Max greedy run per axis: 15 * 4096 = 61440 < 65536 (u16 safe). */
constexpr int kMaxMerge = 15;
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
    uint8_t shape; /* mcv_core::Shape: 0 cube 1 cross 2 torch 3 fence 4
                    * slab 5 stairs（按注册名分类，见 blocks_gen.inc） */
};

/* Generated from the same ci/gen-blocks.py run as mcv_core::BLOCKS, so id
 * order/flags/tile layers match the Rust registry by construction (tiles[]
 * are texture-array layer indices from tiles_manifest.json). Non-cube shapes
 * (cross/torch/fence/slab/stairs, model_kind=1) are emitted by the
 * emit_shapes() template path; only shape==0 (cube) blocks take the greedy
 * merge. ids >= kBlocksCount fall back to kUnknown. */
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
constexpr BlockInfo kUnknown{true, false, false, {0, 0, 0, 0, 0, 0}, 0};

const BlockInfo& block_info(uint16_t id) {
    const uint16_t base = MCV_ID(id);
    return base < kBlocksCount ? kBlocks[base] : kUnknown;
}

/* 入参为原始体素值（可含状态位；kBarrier 先行拦截）。 */
bool is_opaque(uint16_t raw) {
    return raw >= kBarrier || block_info(raw).opaque;
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
    /* Callers guarantee blocks ≤ kMaxMerge (greedy cap) or ≤1 (shape
     * templates), so blocks*4096 never overflows u16. No mod-wrap: with
     * linear vertex interpolation a wrap would collapse the span. */
    return static_cast<uint16_t>(blocks * kUvPerBlock + 0.5f);
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
        /* Sides (v axis = world +y): v=0 must sit at the TOP edge —
         * texture row 0 is the image top (grass fringe), uploaded with no
         * flip, and the shader samples coord.y=0 at row 0. The old
         * convention grew v with y, putting the fringe at the block
         * bottom (device round 2). Top/bottom faces (v axis = z) are
         * direction-agnostic noise textures. */
        const uint16_t cv =
            (axis != 1) ? (b != 0 ? 0 : v_max) : (b != 0 ? v_max : 0);
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

/* ---- 非立方形状模板（shape 1..5） -------------------------------------
 * 非 Cube 方块不进贪心合并（合并键以整格面为前提），改由 emit_shapes 按
 * 状态 nibble 发射参数化盒子/面模板。面剔除与 Cube 同判据：面法线方向
 * 邻格不透明则不发射；光照/AO/UV/flags 全部复用 Cube 路径函数
 * （light_at/corner_ao/kCornerOrder/uv_coord）。 */

/* 原版比例（16 px 方块 → 0..1）：半砖/踏步半高 8px；火把柱 2px 宽、
 * 10px 高；栅栏柱 4px 宽全高，臂梁 3px 厚（y 6..9px）。 */
constexpr float kHalf = 0.5f;
constexpr float kTorchMin = 0.4f;   /* 火把柱 x/z 0.4..0.6 */
constexpr float kTorchTop = 0.625f; /* 火把柱 y 0..0.625 */
constexpr float kFencePost = 0.375f; /* 栅栏柱 x/z 0.375..0.625 */
constexpr float kRailMin = 0.375f;  /* 臂梁 y 0.375..0.5625 */
constexpr float kRailMax = 0.5625f;

/* 盒子：方块内局部坐标（0..1），发射时再加格原点。 */
struct Box3 {
    float x0, y0, z0, x1, y1, z1;
};

/* 面所在 slice 网格的 u/v 轴序号（与 grid_axes 一致：axis0 u=z v=y，
 * axis1 u=x v=z，axis2 u=x v=y），UV 与 AO 采样共用。 */
void face_uv_axes(int axis, int* u_axis, int* v_axis) {
    if (axis == 0) {
        *u_axis = 2; *v_axis = 1;
    } else if (axis == 1) {
        *u_axis = 0; *v_axis = 2;
    } else {
        *u_axis = 0; *v_axis = 1;
    }
}

/* 发射盒子的一个面。expose=true 强制发射（半砖中层面：邻格是本方块自身
 * 格的另一半，不属于“不透明邻格”判据）。UV 取盒子在格内的实际区间，
 * 与 Cube 同一 uv_coord 换算（整盒时与贪心路径逐位一致）。 */
void emit_box_face(const Neighborhood& n, std::vector<QuadVertex>& verts,
                   std::vector<uint32_t>& indices, int x, int y, int z,
                   const BlockInfo& info, const Box3& b, int face,
                   bool expose) {
    const int axis = face / 2;
    const int step = face % 2 == 0 ? 1 : -1;
    const int nx = x + (axis == 0 ? step : 0);
    const int ny = y + (axis == 1 ? step : 0);
    const int nz = z + (axis == 2 ? step : 0);
    if (!expose && is_opaque(block_at(n, nx, ny, nz))) {
        return;
    }
    const float bmin[3] = {b.x0, b.y0, b.z0};
    const float bmax[3] = {b.x1, b.y1, b.z1};
    int u_axis;
    int v_axis;
    face_uv_axes(axis, &u_axis, &v_axis);
    /* corner_ao 的 u/v 轴单位向量（与 grid_axes 同向，对称采样不依赖符号）。 */
    const int ux = u_axis == 0 ? 1 : 0;
    const int uy = u_axis == 1 ? 1 : 0;
    const int uz = u_axis == 2 ? 1 : 0;
    const int vx = v_axis == 0 ? 1 : 0;
    const int vy = v_axis == 1 ? 1 : 0;
    const int vz = v_axis == 2 ? 1 : 0;

    Cell c;
    std::memset(&c, 0, sizeof(Cell));
    c.tex = info.tiles[face];
    c.visible = 1;
    light_at(n, nx, ny, nz, &c.sky, &c.blk);

    const float plane = face % 2 == 0 ? bmax[axis] : bmin[axis];
    const uint8_t flags = static_cast<uint8_t>(face);
    const uint32_t base = static_cast<uint32_t>(verts.size());
    for (int k = 0; k < 4; ++k) {
        const int a = kCornerOrder[face][k][0];
        const int bq = kCornerOrder[face][k][1];
        float pos[3];
        pos[axis] = plane;
        pos[u_axis] = a != 0 ? bmax[u_axis] : bmin[u_axis];
        pos[v_axis] = bq != 0 ? bmax[v_axis] : bmin[v_axis];
        const uint16_t cu = uv_coord(pos[u_axis]);
        /* Side faces: mirror y within the box so v=0 lands on the box top
         * (see emit_quad comment for the texture-row convention). */
        const float v_uv_pos = v_axis == 1
                                  ? bmax[v_axis] + bmin[v_axis] - pos[v_axis]
                                  : pos[v_axis];
        const uint16_t cv = uv_coord(v_uv_pos);
        const uint8_t ao =
            corner_ao(n, nx, ny, nz, ux, uy, uz, vx, vy, vz, a, bq);
        push_vertex(verts, pos[0] + static_cast<float>(x),
                    pos[1] + static_cast<float>(y),
                    pos[2] + static_cast<float>(z), cu, cv, c, ao, flags);
    }
    const uint32_t i0 = base;
    const uint32_t i1 = base + 1;
    const uint32_t i2 = base + 2;
    const uint32_t i3 = base + 3;
    const uint32_t quad[6] = {i0, i1, i2, i0, i2, i3};
    indices.insert(indices.end(), quad, quad + 6);
}

/* 六面发射；force_face >= 0 的面强制暴露（半砖中层面），其余按邻格剔除。 */
void emit_box(const Neighborhood& n, std::vector<QuadVertex>& verts,
              std::vector<uint32_t>& indices, int x, int y, int z,
              const BlockInfo& info, const Box3& b, int force_face) {
    for (int f = 0; f < 6; ++f) {
        emit_box_face(n, verts, indices, x, y, z, info, b, f,
                      f == force_face);
    }
}

/* 十字植物：两条对角双面 quad（正面 + 反面索引各 6——不透明管线开背面
 * 剔除，反向索引保证两侧可见）。ao 恒 3；flags 用 +Y 光照档（植物不受
 * 侧面变暗）；光照取方块自身所在格（植物不遮挡所在格光照）。 */
void emit_cross(const Neighborhood& n, std::vector<QuadVertex>& verts,
                std::vector<uint32_t>& indices, int x, int y, int z,
                const BlockInfo& info) {
    Cell c;
    std::memset(&c, 0, sizeof(Cell));
    c.tex = info.tiles[kFacePy];
    c.visible = 1;
    light_at(n, x, y, z, &c.sky, &c.blk);
    const float fx = static_cast<float>(x);
    const float fy = static_cast<float>(y);
    const float fz = static_cast<float>(z);
    const uint16_t u_max = uv_coord(1.0f);
    const uint16_t v_max = uv_coord(1.0f);
    const uint8_t flags = static_cast<uint8_t>(kFacePy);
    /* 每条对角线 4 角：底1 底2 顶2 顶1（u 沿对角线、v 沿 y）。
     * v 翻转：底角取 v_max、顶角取 0——row 0 是贴图顶边（花/苗的顶部），
     * 须落在方块顶（真机 round 2：v 随 y 增导致植物倒立）。 */
    const float corners[2][4][3] = {
        {{0, 0, 0}, {1, 0, 1}, {1, 1, 1}, {0, 1, 0}},
        {{1, 0, 0}, {0, 0, 1}, {0, 1, 1}, {1, 1, 0}},
    };
    const uint16_t cuv[4][2] = {
        {0, v_max}, {u_max, v_max}, {u_max, 0}, {0, 0}};
    for (const auto& diag : corners) {
        const uint32_t base = static_cast<uint32_t>(verts.size());
        for (int k = 0; k < 4; ++k) {
            push_vertex(verts, fx + diag[k][0], fy + diag[k][1],
                        fz + diag[k][2], cuv[k][0], cuv[k][1], c, 3, flags);
        }
        const uint32_t i0 = base;
        const uint32_t i1 = base + 1;
        const uint32_t i2 = base + 2;
        const uint32_t i3 = base + 3;
        const uint32_t front[6] = {i0, i1, i2, i0, i2, i3};
        const uint32_t back[6] = {i0, i2, i1, i0, i3, i2};
        indices.insert(indices.end(), front, front + 6);
        indices.insert(indices.end(), back, back + 6);
    }
}

/* 栅栏臂连接判据：26.1 FenceBlock.java:59-63 connectsTo ≈
 *   sturdy 邻块 ∥ 同栅栏类别 ∥ 栅栏门朝向连通
 * 的移植近似。sturdy 以"实体不透明整立方"（渲染不透明位 + shape==0）
 * 近似面朝向的 isFaceSturdy；同栅栏类别以 shape==Fence 近似。
 * KNOWN-DIVERGENCE（本引擎无 tag / blockstate 体系，勿凭直觉收紧）：
 * - BlockTags.FENCES / WOODEN_FENCES（FenceBlock.java:66-68）未移植，
 *   木质↔非木质栅栏（橡木↔下界砖）跨类会误连；
 * - FenceGateBlock 分支缺（本引擎未注册栅栏门）；
 * - isExceptionForConnection 例外名单（Block.java:255-262 叶/屏障/雕纹南
 *   瓜/南瓜灯/西瓜/南瓜/潜影盒）未移植——26.1 sturdy=面支撑形整面
 *   （SupportType.java:11-16 FULL=isFaceFull(blockSupportShape)），南瓜系
 *   sturdy 但在例外名单，我方会误连、原版排除；
 * - 玻璃等 opaque=false 整方块在 26.1 支撑形整盒 → faceSturdy=true 原版会
 *   连臂，我方近似为不透明整立方 → 不连（保守方向，宁缺勿错连）。
 * Rust 侧同一规则：engine/mcv_game/src/blockshapes.rs::fence_connects，
 * 两侧必须同步（渲染臂与碰撞臂一致性=原版防跳语义）。 */
bool fence_arm_connects(uint16_t nb) {
    if (nb >= kBarrier) {
        return false; /* 越界/未加载邻区块：不出臂 */
    }
    const BlockInfo& i = block_info(nb);
    if (i.shape == 3 /* Fence */) {
        return true;
    }
    return i.shape == 0 /* Cube */ && i.opaque;
}

/* 栅栏：中心立柱全高（x/z 0.375..0.625）+ 水平四向臂（连接判据见
 * fence_arm_connects：贴石墙等 sturdy 邻块原版也出臂，FenceBlock.java:
 * 91-94）。臂梁 y 0.375..0.5625，沿臂向从柱边到格边；臂端面贴 sturdy
 * 邻块一侧由不透明面剔除收尾，臂端面与相邻栅栏臂端面共面反向，由背面
 * 剔除消化（宁多勿漏）。 */
void emit_fence(const Neighborhood& n, std::vector<QuadVertex>& verts,
                std::vector<uint32_t>& indices, int x, int y, int z,
                const BlockInfo& info) {
    const Box3 post{kFencePost, 0.0f, kFencePost, 1.0f - kFencePost, 1.0f,
                    1.0f - kFencePost};
    emit_box(n, verts, indices, x, y, z, info, post, -1);
    const int dirs[4][2] = {{1, 0}, {-1, 0}, {0, 1}, {0, -1}};
    for (const auto& d : dirs) {
        if (!fence_arm_connects(block_at(n, x + d[0], y, z + d[1]))) {
            continue;
        }
        Box3 arm{kFencePost, kRailMin, kFencePost, 1.0f - kFencePost, kRailMax,
                 1.0f - kFencePost};
        if (d[0] == 1) {
            arm.x1 = 1.0f;
        } else if (d[0] == -1) {
            arm.x0 = 0.0f;
        } else if (d[1] == 1) {
            arm.z1 = 1.0f;
        } else {
            arm.z0 = 0.0f;
        }
        emit_box(n, verts, indices, x, y, z, info, arm, -1);
    }
}

/* 楼梯：底座（整格宽半高盒）+ 踏步（朝向侧半格、另半高盒），bit2=top
 * 上下翻转。facing 编码 0=+Z 1=-Z 2=+X 3=-X（与 game/mcv_logic
 * placement_state / mcv_core BlockId::state / blockshapes.rs 同一约定，
 * 勿按直觉写成 +X 优先）；语义 = 26.1 FACING（玩家水平视线同向，
 * StairBlock.java:101-102），几何含义“踏步（整高半）位于朝向侧半格”
 * 按 StairBlock.java:37-38 推得（facing=NORTH → 上半占 -Z 半格）。
 * 面剔除只用通用“邻格不透明”判据：同种楼梯
 * 互相做透明剔除会在半盒错位处留洞，故两盒一律按各自暴露面发射，宁多
 * 勿漏；两盒在 y=0.5 的共面相对面（+Y/-Y）由背面剔除消化，不做盒间剔除。 */
void emit_stairs(const Neighborhood& n, std::vector<QuadVertex>& verts,
                 std::vector<uint32_t>& indices, int x, int y, int z,
                 const BlockInfo& info, uint8_t st) {
    const int facing = st & 3;
    const bool flipped = (st & 4) != 0;
    Box3 base;
    Box3 step;
    if (!flipped) {
        base = Box3{0.0f, 0.0f, 0.0f, 1.0f, kHalf, 1.0f};
        step = Box3{0.0f, kHalf, 0.0f, 1.0f, 1.0f, 1.0f};
    } else {
        base = Box3{0.0f, kHalf, 0.0f, 1.0f, 1.0f, 1.0f};
        step = Box3{0.0f, 0.0f, 0.0f, 1.0f, kHalf, 1.0f};
    }
    switch (facing) {
    case 0:
        step.z0 = kHalf; /* +Z：踏步占 +Z 半格 */
        break;
    case 1:
        step.z1 = kHalf; /* -Z */
        break;
    case 2:
        step.x0 = kHalf; /* +X */
        break;
    default:
        step.x1 = kHalf; /* -X */
        break;
    }
    emit_box(n, verts, indices, x, y, z, info, base, -1);
    emit_box(n, verts, indices, x, y, z, info, step, -1);
}

/* 中心块逐格扫描（16x256x16，只管本区块；邻域只用于剔除/光照），对
 * shape != 0 的几何方块按状态发射模板；仅不透明 pass 调用。生成表只出
 * 0..5，default 整盒为防御性回退。 */
void emit_shapes(const Neighborhood& n, std::vector<QuadVertex>& verts,
                 std::vector<uint32_t>& indices) {
    for (int y = 0; y < 256; ++y) {
        for (int z = 0; z < 16; ++z) {
            for (int x = 0; x < 16; ++x) {
                const uint16_t raw = block_at(n, x, y, z);
                if (raw >= kBarrier) {
                    continue;
                }
                const BlockInfo& info = block_info(raw);
                if (!info.geom || info.shape == 0) {
                    continue;
                }
                const uint8_t st = static_cast<uint8_t>(MCV_STATE(raw));
                switch (info.shape) {
                case 1:
                    emit_cross(n, verts, indices, x, y, z, info);
                    break;
                case 2:
                    emit_box(n, verts, indices, x, y, z, info,
                             Box3{kTorchMin, 0.0f, kTorchMin,
                                  1.0f - kTorchMin, kTorchTop,
                                  1.0f - kTorchMin},
                             -1);
                    break;
                case 3:
                    emit_fence(n, verts, indices, x, y, z, info);
                    break;
                case 4:
                    if ((st & 1) != 0) {
                        /* 上半砖：y 0.5..1，中层面（-Y）永远暴露 */
                        emit_box(n, verts, indices, x, y, z, info,
                                 Box3{0.0f, kHalf, 0.0f, 1.0f, 1.0f, 1.0f},
                                 kFaceNy);
                    } else {
                        /* 下半砖：y 0..0.5，中层面（+Y）永远暴露 */
                        emit_box(n, verts, indices, x, y, z, info,
                                 Box3{0.0f, 0.0f, 0.0f, 1.0f, kHalf, 1.0f},
                                 kFacePy);
                    }
                    break;
                case 5:
                    emit_stairs(n, verts, indices, x, y, z, info, st);
                    break;
                default:
                    emit_box(n, verts, indices, x, y, z, info,
                             Box3{0.0f, 0.0f, 0.0f, 1.0f, 1.0f, 1.0f}, -1);
                    break;
                }
            }
        }
    }
}

/* Standard greedy sweep: per (axis, direction, layer), expand width along
 * the grid u axis first, then height along v; cells merge only when the
 * full quad key matches (block, tile, sky/block light, packed AO, wave).
 * 贪心仅限 shape==0（Cube）；非立方形状由 emit_shapes 另行发射。 */
void build_pass(const Neighborhood& n, bool water_pass,
                std::vector<QuadVertex>& verts,
                std::vector<uint32_t>& indices) {
    for (int axis = 0; axis < 3; ++axis) {
        const SliceGeom g = slice_geom(axis);
        const size_t gu = static_cast<size_t>(g.gu), gv = static_cast<size_t>(g.gv);
        const size_t grid_len = gu * gv;
        // 统一格子索引:符号循环变量 → 无符号显式换算一次到位。
        auto cell_idx = [gu](int vv, int uu) {
            return static_cast<size_t>(vv) * gu + static_cast<size_t>(uu);
        };
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
                        const uint16_t raw = block_at(n, x, y, z);
                        if (raw >= kBarrier) {
                            continue;
                        }
                        const uint16_t id = MCV_ID(raw);
                        const int nx = x + nx_step;
                        const int ny = y + ny_step;
                        const int nz = z + nz_step;
                        const uint16_t nb = block_at(n, nx, ny, nz);

                        bool visible = false;
                        uint8_t wave = 0;
                        if (water_pass) {
                            /* water vs water (or barrier) shows nothing */
                            if (id == kWater && nb < kBarrier &&
                                MCV_ID(nb) != kWater) {
                                visible = true;
                                if (face == kFacePy) {
                                    wave = MCV_ID(nb) == kAir ? 1 : 0;
                                }
                            }
                        } else {
                            const BlockInfo& bi = block_info(id);
                            visible =
                                bi.geom && bi.shape == 0 && !is_opaque(nb);
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
                        cells[cell_idx(v, u)] = c;
                    }
                }

                std::memset(visited.data(), 0, grid_len);
                for (int v = 0; v < g.gv; ++v) {
                    for (int u = 0; u < g.gu; ++u) {
                        const size_t i = cell_idx(v, u);
                        if (visited[i] || !cells[i].visible) {
                            continue;
                        }
                        const Cell key = cells[i];

                        /* kMaxMerge cap: uv span = run * 4096 must fit u16
                         * (see kUvPerBlock comment). */
                        int wq = 1;
                        while (u + wq < g.gu && wq < kMaxMerge) {
                            const size_t j = cell_idx(v, u + wq);
                            if (visited[j] || !same_key(key, cells[j])) {
                                break;
                            }
                            ++wq;
                        }

                        int hq = 1;
                        bool grew = true;
                        while (v + hq < g.gv && grew && hq < kMaxMerge) {
                            for (int k = 0; k < wq; ++k) {
                                const size_t j = cell_idx(v + hq, u + k);
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
                                visited[cell_idx(v + dv, u + du)] = 1;
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
    if (mesh_kind == 0) {
        /* 非立方形状模板只在不透明 pass（水 pass 只出水）。 */
        emit_shapes(n, verts, indices);
    }

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
