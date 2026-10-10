//! Meshing orchestration: feeds 3x3 neighbourhoods to the mesher and owns
//! mesh buffer handles.
//!
//! Backend switch (纯 Rust 移植，见 src/mesher.rs）：默认走 Rust 网格器；
//! `MCV_MESHER_BACKEND=ffi` 回退 C++ oracle 路径（mcv_ffi/mcv_mesh_build，
//! C++ 在本阶段仍是对拍预言机，删除属后续任务）。两路输出逐字节对拍锁定
//! （tests/parity.rs），生产消费方（mcv_logic）无感切换。

pub use mcv_ffi::CxxMeshBuffer;

mod mesher;

pub use mesher::{MeshData, block_geom, build_mesh};

/// One loaded chunk slot: voxel ids (65536 u16) and light bytes (65536),
/// layout `(y<<8)|(z<<4)|x`; light low nibble = block, high nibble = sky.
#[derive(Clone, Copy)]
pub struct Slot<'a> {
    pub voxels: &'a [u16],
    pub light: &'a [u8],
}

/// Mesh pass selector for [`Mesher::build`].
pub const MESH_OPAQUE: u32 = mcv_ffi::MESH_OPAQUE;
pub const MESH_WATER: u32 = mcv_ffi::MESH_WATER;

/// 网格器后端。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    /// 纯 Rust 网格器（src/mesher.rs，生产路径）。
    Rust,
    /// C++ oracle 路径（mcv_ffi → cpp/src/mesher.cpp，对拍/回退用）。
    Ffi,
}

impl Backend {
    /// `MCV_MESHER_BACKEND=ffi` → C++ oracle；其余（含未设置）→ Rust。
    pub fn from_env() -> Self {
        if std::env::var("MCV_MESHER_BACKEND").as_deref() == Ok("ffi") {
            Self::Ffi
        } else {
            Self::Rust
        }
    }
}

impl Default for Backend {
    fn default() -> Self {
        Self::from_env()
    }
}

/// 网格构建产物：C++ 池句柄或 Rust 自有缓冲。两种变体对外暴露同一读取
/// 接口（mcv_logic 的 ChunkMesher 只经由这些方法消费，见 game.rs:92-101）。
pub enum MeshBuffer {
    Pool(CxxMeshBuffer),
    Owned(MeshData),
}

impl MeshBuffer {
    /// 顶点字节（24 B 步长交错，见 mesher.rs 模块注释）。
    pub fn vertex_data(&self) -> &[u8] {
        match self {
            Self::Pool(b) => b.vertex_data(),
            Self::Owned(m) => &m.vertices,
        }
    }

    /// 索引（u32，外视 CCW）。
    pub fn indices(&self) -> &[u32] {
        match self {
            Self::Pool(b) => b.indices(),
            Self::Owned(m) => &m.indices,
        }
    }

    /// (顶点数, 索引数)。
    pub fn counts(&self) -> (u32, u32) {
        match self {
            Self::Pool(b) => b.counts(),
            Self::Owned(m) => (
                (m.vertices.len() / mesher::VERTEX_STRIDE) as u32,
                m.indices.len() as u32,
            ),
        }
    }
}

/// Owns the C++ pool (FFI 后端用；Rust 后端不触池，保留创建以维持既有
/// 构造语义) and drives [`Mesher::build`] calls.
pub struct Mesher {
    pool: mcv_ffi::MemPool,
    backend: Backend,
}

impl Mesher {
    /// Creates the underlying pool with the given live-byte budget. 后端按
    /// `Backend::from_env()` 选择（默认 Rust）。
    pub fn new(budget_bytes: u64) -> Option<Self> {
        Self::with_backend(budget_bytes, Backend::from_env())
    }

    /// 显式后端构造（对拍测试用，避免环境变量歧义）。
    pub fn with_backend(budget_bytes: u64, backend: Backend) -> Option<Self> {
        Some(Self {
            pool: mcv_ffi::MemPool::new(budget_bytes)?,
            backend,
        })
    }

    /// Builds a mesh for the center chunk (neighbourhood index 4). The
    /// neighbourhood is row-major dz-outer/dx-inner, index
    /// `(dz+1)*3 + (dx+1)`; `None` = not loaded，两侧一致按不透明边界处理
    /// （mesher.cpp:139-168 block_at / tests/parity.rs 覆盖）。`kind`:
    /// 0 = opaque, 1 = water。
    pub fn build(
        &self,
        neighborhood: &[Option<Slot<'_>>; 9],
        kind: u32,
    ) -> Result<MeshBuffer, i32> {
        if kind > 1 {
            return Err(mcv_ffi::err::BAD_ARG);
        }
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
        match self.backend {
            Backend::Rust => Ok(MeshBuffer::Owned(mesher::build_mesh(
                &voxels, &lights, kind,
            ))),
            Backend::Ffi => Ok(MeshBuffer::Pool(mcv_ffi::mesh_build_raw(
                &voxels, &lights, kind, &self.pool,
            )?)),
        }
    }
}
