// rustcreate C ABI — single source of truth for the Rust <-> C++ boundary.
//
// Contract:
// - extern "C" only; int32_t error codes, 0 = MCV_OK; no exceptions cross
//   this boundary (compiled with -fno-exceptions).
// - No C++ types in signatures; POD structs only. Rust mirrors every struct
//   as #[repr(C)] with a static size assert.
// - Pointers passed into terrain/light calls are borrowed for the duration
//   of the call only; C++ must not retain them.
// - Every allocation has a paired release. McvMeshBuffer memory belongs to
//   the McvPool and must be returned via mcv_meshbuf_release exactly once.

#ifndef MCV_H
#define MCV_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum {
    MCV_OK = 0,
    MCV_ERR_NULL_ARG = -1,
    MCV_ERR_VERSION_MISMATCH = -2,
    MCV_ERR_OOM = -3,
    MCV_ERR_CANARY_CORRUPT = -4, /* debug builds only */
    MCV_ERR_DOUBLE_FREE = -5,    /* debug builds only */
    MCV_ERR_BAD_ARG = -6,
    MCV_ERR_NOT_IMPLEMENTED = -7,
};

#define MCV_ABI_VERSION 0x52554332u /* "RUC2" — u16 block ids */

uint32_t mcv_api_version(void);

/* ---- terrain ---------------------------------------------------------- */

/* Fills out_voxels[65536] of uint16_t block ids (layout (y<<8)|(z<<4)|x)
 * and out_heightmap[256] (uint8_t, index (z<<4)|x, value = topmost opaque
 * y + 1). Trees near chunk borders are placed deterministically from
 * neighbour chunk coordinates. */
int32_t mcv_terrain_generate(uint64_t seed, int32_t chunk_x, int32_t chunk_z,
                             uint16_t* out_voxels, uint8_t* out_heightmap);

/* ---- light (pure Rust in mcv_light; no C++ ABI) ----------------------- */

/* ---- meshing ---------------------------------------------------------- */

typedef struct McvMeshBuffer {
    uint8_t* vertex_data; /* interleaved, stride 24 */
    uint32_t* indices;
    uint32_t vertex_count;
    uint32_t index_count;
    uint32_t vertex_cap;
    uint32_t index_cap;
    uint32_t pool_class;
} McvMeshBuffer;

/* voxels[i]: uint16_t block ids; light[i]: uint8_t packed nibbles. 3x3
 * neighbourhood, order (dz+1)*3+(dx+1), [4] = center; NULL = neighbour not
 * loaded (treated as opaque boundary).
 * mesh_kind: 0 = opaque pass, 1 = water pass. Memory comes from the active
 * pool (see mcv_set_active_pool). */
int32_t mcv_mesh_build(const uint16_t* const voxels[9],
                       const uint8_t* const light[9], uint32_t mesh_kind,
                       McvMeshBuffer* out);

/* ---- memory pool ------------------------------------------------------ */

typedef struct McvPool McvPool;

/* Sets the thread-local pool used by mcv_mesh_build. Pass NULL to clear. */
void mcv_set_active_pool(McvPool* pool);

McvPool* mcv_pool_create(uint64_t budget_bytes);
void mcv_pool_destroy(McvPool* pool); /* debug: aborts on leak/corruption */
int32_t mcv_meshbuf_acquire(McvPool* pool, uint32_t vertex_cap,
                            uint32_t index_cap, McvMeshBuffer* out);
/* Returns MCV_OK, MCV_ERR_CANARY_CORRUPT or MCV_ERR_DOUBLE_FREE (debug). */
int32_t mcv_meshbuf_release(McvMeshBuffer* buf);
uint32_t mcv_pool_stats(McvPool* pool, uint32_t* out_live_buffers,
                        uint64_t* out_bytes_live);

#ifdef __cplusplus
}
#endif

#endif /* MCV_H */
