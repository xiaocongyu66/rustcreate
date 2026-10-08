// Terrain / mesh implementations land in M2 and M4 (lighting is pure Rust).
// These stubs keep the M1 ABI surface linkable and testable.

#include "mcv.h"

extern "C" {

int32_t mcv_terrain_generate(uint64_t, int32_t, int32_t, uint8_t*,
                             uint8_t*) {
    return MCV_ERR_NOT_IMPLEMENTED;
}

int32_t mcv_mesh_build(const uint8_t* const[9], const uint8_t* const[9],
                       uint32_t, McvMeshBuffer*) {
    return MCV_ERR_NOT_IMPLEMENTED;
}

}  // extern "C"
