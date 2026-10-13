//! Meshing orchestration: feeds 3x3 neighbourhoods to the mesher and owns
//! mesh buffer handles.
//!
//! 网格构建全走纯 Rust 路径（src/mesher.rs，任务板 #76 移植）；任务板 #77
//! 删除 C++ oracle 路径（原 mcv_ffi/mcv_mesh_build + mempool 池句柄）后，
//! 回归防线是 tests/golden.rs 对黄金数据（删除前 frozen oracle 输出落盘）
//! 逐字节对拍。生产消费方（mcv_logic）只经由 [`MeshData`] 的读取接口消费。

mod mesher;
mod sched;

pub use mesher::{MeshData, TintInput, VERTEX_STRIDE, block_geom, build_mesh};
pub use sched::{
    MeshBytes, MeshDone, MeshJob, MeshResults, MeshScheduler, mesh_worker_count,
    neighborhood_slots, worker_count_from,
};

/// One loaded chunk slot: voxel ids (98304 u16) and light bytes (98304),
/// layout `mcv_core::vidx`（v6：384 高，y 为绝对世界 y）; light low nibble =
/// block, high nibble = sky.
#[derive(Clone, Copy)]
pub struct Slot<'a> {
    pub voxels: &'a [u16],
    pub light: &'a [u8],
}

/// Mesh pass selector for [`Mesher::build`].
pub const MESH_OPAQUE: u32 = 0;
pub const MESH_WATER: u32 = 1;

/// 入参校验失败（沿删除前 mcv_ffi::err::BAD_ARG 的错误码语义）。
const BAD_ARG: i32 = -6;

/// Drives [`mesher::build_mesh`] calls over chunk neighbourhoods. 任务板
/// #77 前本类型持有 C++ mempool 句柄并按 `MCV_MESHER_BACKEND` 选路；纯
/// Rust 路径自有缓冲，预算/环境变量开关一并拆除。
#[derive(Clone, Copy, Default)]
pub struct Mesher;

impl Mesher {
    /// Creates the mesher.
    pub fn new() -> Self {
        Self
    }

    /// Builds a mesh for the center chunk (neighbourhood index 4). The
    /// neighbourhood is row-major dz-outer/dx-inner, index
    /// `(dz+1)*3 + (dx+1)`; `None` = not loaded，按不透明边界处理
    /// （mesher.rs BARRIER 哨兵）。`kind`: [`MESH_OPAQUE`] = 0,
    /// [`MESH_WATER`] = 1；其余值 `Err(BAD_ARG)`。无群系染色（顶点色白，
    /// 贴图原样）——黄金件/无 colormap 装配路径用；带色走 [`Self::build_with_tint`]。
    pub fn build(&self, neighborhood: &[Option<Slot<'_>>; 9], kind: u32) -> Result<MeshData, i32> {
        self.build_with_tint(neighborhood, kind, None)
    }

    /// [`Self::build`] + 可选群系 tint 场（`tint` 含中心区块世界方块原点；
    /// 仅 opaque pass 消费，water pass 忽略 tint，见 [`build_mesh`]）。
    pub fn build_with_tint(
        &self,
        neighborhood: &[Option<Slot<'_>>; 9],
        kind: u32,
        tint: Option<TintInput>,
    ) -> Result<MeshData, i32> {
        if kind > 1 {
            return Err(BAD_ARG);
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
        Ok(mesher::build_mesh(&voxels, &lights, kind, tint))
    }
}
