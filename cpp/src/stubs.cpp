// Mesh implementation lands in M4 (terrain is implemented; lighting is pure
// Rust). This stub keeps the M1 ABI surface linkable and testable.

#include "mcv.h"

extern "C" {

int32_t mcv_mesh_build(const uint8_t* const[9], const uint8_t* const[9],
                       uint32_t, McvMeshBuffer*) {
    return MCV_ERR_NOT_IMPLEMENTED;
}

}  // extern "C"
