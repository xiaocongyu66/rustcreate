//! Meshing orchestration: feeds 3x3 neighbourhoods to the C++ mesher and
//! owns mesh buffer handles.

pub use mcv_ffi::CxxMeshBuffer;

/// One loaded chunk slot: voxel ids (65536 u16) and light bytes (65536),
/// layout `(y<<8)|(z<<4)|x`; light low nibble = block, high nibble = sky.
#[derive(Clone, Copy)]
pub struct Slot<'a> {
    pub voxels: &'a [u16],
    pub light: &'a [u8],
}

/// Owns the C++ pool and drives [`mcv_ffi::mesh_build_raw`] calls.
pub struct Mesher {
    pool: mcv_ffi::MemPool,
}

/// Mesh pass selector for [`Mesher::build`].
pub const MESH_OPAQUE: u32 = mcv_ffi::MESH_OPAQUE;
pub const MESH_WATER: u32 = mcv_ffi::MESH_WATER;

impl Mesher {
    /// Creates the underlying pool with the given live-byte budget.
    pub fn new(budget_bytes: u64) -> Option<Self> {
        Some(Self {
            pool: mcv_ffi::MemPool::new(budget_bytes)?,
        })
    }

    /// Builds a mesh for the center chunk (neighbourhood index 4). The
    /// neighbourhood is row-major dz-outer/dx-inner, index
    /// `(dz+1)*3 + (dx+1)`; `None` = not loaded, which C++ treats as an
    /// opaque boundary (no faces facing it). `kind`: 0 = opaque, 1 = water.
    pub fn build(
        &self,
        neighborhood: &[Option<Slot<'_>>; 9],
        kind: u32,
    ) -> Result<CxxMeshBuffer, i32> {
        let mut voxels: [Option<&[u16]>; 9] = [None; 9];
        let mut lights: [Option<&[u8]>; 9] = [None; 9];
        for (i, slot) in neighborhood.iter().enumerate() {
            if let Some(s) = slot {
                debug_assert!(s.voxels.len() >= mcv_core::CHUNK_VOL);
                debug_assert!(s.light.len() >= mcv_core::CHUNK_VOL);
                voxels[i] = Some(s.voxels);
                lights[i] = Some(s.light);
            }
        }
        mcv_ffi::mesh_build_raw(&voxels, &lights, kind, &self.pool)
    }
}
