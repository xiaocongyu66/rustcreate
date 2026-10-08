//! Terrain orchestration: drives the C++ terrain kernel per chunk and feeds
//! the block registry. Task queue integration arrives in M2.

pub fn generate_stub(seed: u64, cx: i32, cz: i32) -> Result<(), i32> {
    let mut voxels = vec![0u8; mcv_core::CHUNK_VOL];
    let mut heightmap = vec![0u8; 256];
    mcv_ffi::terrain_generate_raw(seed, cx, cz, &mut voxels, &mut heightmap)
}
