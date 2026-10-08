//! The single C ABI boundary between Rust and C++ (see cpp/include/mcv.h).
//!
//! Ownership contract:
//! - Voxel/light/heightmap memory is owned by Rust; C++ borrows pointers for
//!   the duration of one call and must not retain them.
//! - `McvMeshBuffer` memory belongs to a C++ [`MemPool`]; the safe wrapper
//!   [`CxxMeshBuffer`] releases it exactly once on Drop.
//! - Errors cross as int32 codes (never exceptions, never panics).

use std::marker::PhantomData;
use std::sync::Arc;

pub mod err {
    pub const OK: i32 = 0;
    pub const NULL_ARG: i32 = -1;
    pub const VERSION_MISMATCH: i32 = -2;
    pub const OOM: i32 = -3;
    pub const CANARY_CORRUPT: i32 = -4;
    pub const DOUBLE_FREE: i32 = -5;
    pub const BAD_ARG: i32 = -6;
    pub const NOT_IMPLEMENTED: i32 = -7;
}

/// ABI hash agreed by mcv.h (`#define MCV_ABI_VERSION`).
pub const ABI_VERSION: u32 = 0x5255_4331;

/// Mesh buffer memory owned by the C++ pool. See cpp/include/mcv.h for the
/// vertex layout (stride 24).
#[repr(C)]
pub struct McvMeshBuffer {
    pub vertex_data: *mut u8,
    pub indices: *mut u32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub vertex_cap: u32,
    pub index_cap: u32,
    pub pool_class: u32,
}

const _: () = assert!(size_of::<McvMeshBuffer>() == 40);

/// Mesh pass selector for [`mcv_mesh_build`].
pub const MESH_OPAQUE: u32 = 0;
pub const MESH_WATER: u32 = 1;

const _: () = assert!(size_of::<usize>() == 8);

extern "C" {
    fn mcv_api_version() -> u32;
    fn mcv_terrain_generate(
        seed: u64,
        chunk_x: i32,
        chunk_z: i32,
        out_voxels: *mut u8,
        out_heightmap: *mut u8,
    ) -> i32;
    fn mcv_mesh_build(
        voxels: *const *const u8,
        light: *const *const u8,
        mesh_kind: u32,
        out: *mut McvMeshBuffer,
    ) -> i32;
    fn mcv_set_active_pool(pool: *mut McvPool);
    fn mcv_pool_create(budget_bytes: u64) -> *mut McvPool;
    fn mcv_pool_destroy(pool: *mut McvPool);
    fn mcv_meshbuf_acquire(
        pool: *mut McvPool,
        vertex_cap: u32,
        index_cap: u32,
        out: *mut McvMeshBuffer,
    ) -> i32;
    fn mcv_meshbuf_release(buf: *mut McvMeshBuffer) -> i32;
    fn mcv_pool_stats(
        pool: *mut McvPool,
        out_live_buffers: *mut u32,
        out_bytes_live: *mut u64,
    ) -> u32;
}

enum McvPool {}

/// `mcv_api_version` handshake; panics on mismatch (stale/mixed build).
pub fn check_abi() {
    let v = unsafe { mcv_api_version() };
    assert!(
        v == ABI_VERSION,
        "C++ ABI version mismatch: C++ {v:#x} != Rust {ABI_VERSION:#x}"
    );
}

struct PoolInner {
    raw: *mut McvPool,
}

// The C++ pool is internally synchronized (atomic free lists); the raw
// pointer is only used through the FFI, never dereferenced from Rust.
unsafe impl Send for PoolInner {}
unsafe impl Sync for PoolInner {}

impl Drop for PoolInner {
    fn drop(&mut self) {
        unsafe { mcv_pool_destroy(self.raw) };
    }
}

/// RAII owner of a C++ mesh buffer pool. Thread-safe: the pool uses atomic
/// free lists, so buffers may be acquired on worker threads and released on
/// the main thread (or vice versa).
pub struct MemPool {
    inner: Arc<PoolInner>,
}

// The C++ pool is internally synchronized with atomics.
unsafe impl Send for MemPool {}
unsafe impl Sync for MemPool {}

impl MemPool {
    pub fn new(budget_bytes: u64) -> Option<Self> {
        let raw = unsafe { mcv_pool_create(budget_bytes) };
        if raw.is_null() {
            None
        } else {
            Some(Self {
                inner: Arc::new(PoolInner { raw }),
            })
        }
    }

    pub fn acquire(&self, vertex_cap: u32, index_cap: u32) -> Result<CxxMeshBuffer, i32> {
        let mut buf = McvMeshBuffer {
            vertex_data: std::ptr::null_mut(),
            indices: std::ptr::null_mut(),
            vertex_count: 0,
            index_count: 0,
            vertex_cap: 0,
            index_cap: 0,
            pool_class: 0,
        };
        let rc = unsafe { mcv_meshbuf_acquire(self.inner.raw, vertex_cap, index_cap, &mut buf) };
        if rc == err::OK {
            Ok(CxxMeshBuffer {
                raw: buf,
                _pool: Arc::clone(&self.inner),
            })
        } else {
            Err(rc)
        }
    }

    pub fn stats(&self) -> (u32, u64) {
        let (mut live, mut bytes) = (0u32, 0u64);
        unsafe {
            mcv_pool_stats(self.inner.raw, &mut live, &mut bytes);
        }
        (live, bytes)
    }

    /// Makes this pool the active pool for `mcv_mesh_build` on the current
    /// thread. Returns a guard that restores the previous state on Drop.
    pub fn activate(&self) -> ActivePool<'_> {
        unsafe { mcv_set_active_pool(self.inner.raw) };
        ActivePool {
            _lifetime: PhantomData,
        }
    }
}

/// Single-owner handle to pool memory. `Drop` releases it exactly once.
/// The `_pool` field keeps the owning pool alive for the buffer's lifetime.
pub struct CxxMeshBuffer {
    raw: McvMeshBuffer,
    _pool: Arc<PoolInner>,
}

unsafe impl Send for CxxMeshBuffer {}

impl CxxMeshBuffer {
    pub fn vertex_data(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(self.raw.vertex_data, self.raw.vertex_count as usize * 24)
        }
    }

    pub fn indices(&self) -> &[u32] {
        unsafe { std::slice::from_raw_parts(self.raw.indices, self.raw.index_count as usize) }
    }

    pub fn counts(&self) -> (u32, u32) {
        (self.raw.vertex_count, self.raw.index_count)
    }

    /// Consumes the buffer, returning the raw FFI struct. Ownership
    /// transfers; the caller must release it exactly once via
    /// [`meshbuf_release_raw`]. Test-only escape hatch (leaks one pool Arc
    /// reference per call).
    pub fn into_raw(self) -> McvMeshBuffer {
        let raw = unsafe { std::ptr::read(&self.raw) };
        std::mem::forget(self);
        raw
    }

    /// Byte capacity: class payload split into vertices + indices.
    pub fn capacity_bytes(&self) -> usize {
        (self.raw.vertex_cap as usize * 24) + (self.raw.index_cap as usize * 4)
    }
}

impl Drop for CxxMeshBuffer {
    fn drop(&mut self) {
        let raw = &mut self.raw as *mut McvMeshBuffer;
        let rc = unsafe { mcv_meshbuf_release(raw) };
        debug_assert_eq!(rc, err::OK, "mesh buffer release failed: {rc}");
    }
}

/// Scoped marker for [`MemPool::activate`].
pub struct ActivePool<'a> {
    _lifetime: PhantomData<&'a MemPool>,
}

/// Raw terrain generation. `out_voxels` must hold 65536 bytes, `out_heightmap`
/// 256 bytes.
pub fn terrain_generate_raw(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    out_voxels: &mut [u8],
    out_heightmap: &mut [u8],
) -> Result<(), i32> {
    debug_assert!(out_voxels.len() >= mcv_core::CHUNK_VOL);
    debug_assert!(out_heightmap.len() >= 256);
    let rc = unsafe {
        mcv_terrain_generate(
            seed,
            chunk_x,
            chunk_z,
            out_voxels.as_mut_ptr(),
            out_heightmap.as_mut_ptr(),
        )
    };
    if rc == err::OK {
        Ok(())
    } else {
        Err(rc)
    }
}

/// Raw mesh build over a 3x3 neighbourhood (row-major, dz outer, dx inner;
/// index = (dz+1)*3 + (dx+1); center = 4). `None` neighbours are treated by
/// C++ as opaque boundaries. Voxels and light arrays are passed separately.
pub fn mesh_build_raw(
    voxels: &[Option<&[u8]>; 9],
    lights: &[Option<&[u8]>; 9],
    mesh_kind: u32,
    pool: &MemPool,
) -> Result<CxxMeshBuffer, i32> {
    let _active = pool.activate();
    let mut voxel_ptrs: [*const u8; 9] = [std::ptr::null(); 9];
    let mut light_ptrs: [*const u8; 9] = [std::ptr::null(); 9];
    for (i, slot) in voxels.iter().enumerate() {
        if let Some(bytes) = slot {
            voxel_ptrs[i] = bytes.as_ptr();
        }
    }
    for (i, slot) in lights.iter().enumerate() {
        if let Some(bytes) = slot {
            light_ptrs[i] = bytes.as_ptr();
        }
    }
    let mut buf = McvMeshBuffer {
        vertex_data: std::ptr::null_mut(),
        indices: std::ptr::null_mut(),
        vertex_count: 0,
        index_count: 0,
        vertex_cap: 0,
        index_cap: 0,
        pool_class: 0,
    };
    let rc = unsafe {
        mcv_mesh_build(
            voxel_ptrs.as_ptr(),
            light_ptrs.as_ptr(),
            mesh_kind,
            &mut buf,
        )
    };
    if rc == err::OK {
        Ok(CxxMeshBuffer {
            raw: buf,
            _pool: Arc::clone(&pool.inner),
        })
    } else {
        Err(rc)
    }
}

/// Raw release for buffers handed out by [`CxxMeshBuffer::into_raw`].
/// Returns the FFI error code verbatim (canary / double-free detection).
pub fn meshbuf_release_raw(buf: &mut McvMeshBuffer) -> i32 {
    unsafe { mcv_meshbuf_release(buf as *mut McvMeshBuffer) }
}
