//! sub-chunk 重网格产物（任务板 #172）：编辑触发的重网格从 384 整柱降到
//! 受影响的扫描行集合，未受影响行的旧产物字节整体复用，`combine` 拼回
//! 整柱网格与 [`crate::mesher::build_mesh`] 逐字节相同。
//!
//! **原版机制实证（src-26.1，只读快照）**：
//! - 单格编辑的脏区定位：`LevelRenderer.setBlockDirty`（LevelRenderer.java
//!   :1344-1351）对编辑点 ±1 的 27 格逐格算 section 坐标调
//!   `setSectionDirty` → 编辑 y 在 section 边界 ±1 时邻 section 一并脏；
//!   整段编辑走 `setBlocksDirty`（:1354-1361）同一 ±1 膨胀。
//! - section 粒度重建：`SectionRenderDispatcher.RenderSection.setDirty`
//!   （SectionRenderDispatcher.java:316）只把该 section 的 RebuildTask
//!   （:510-540）入队；网格输入 = 3×3×3 section 邻域拷贝
//!   （RenderSectionRegion.java:16-17 `RADIUS=1/SIZE=3`，:37-46 按 section
//!   坐标寻址）——原版 mesh 一个 section 读 3×3×3 邻域缓冲，不是只读
//!   ±1 体素；本仓快照本就是 3×3 区块整柱，覆盖等价。
//!
//! **本仓的结构性差异（为何不是纯 16³ 复用）**：本仓网格器是整柱贪心
//! （mesher.rs build_pass，golden 锁定的冻结语义），axis0/2 的切片 v 轴 =
//! 世界 y、贪心沿 +v 合并（MAX_MERGE=15）——编辑切开一个跨缝合并带时，
//! 锚点自编辑处向上逐 quad 位移（15 连石带挖掉锚行 → 其上方所有 quad
//! 锚行整体重排），脏区向上方的波及**无先验高度界**（小样验证：整高
//! 石墙单格编辑改变编辑点以上全部 quad 锚位）。因此本 API 的重放单元
//! =「(轴,向,层) 扫描行」：
//! - axis1（y 面）：脏 y 行 ±1（cell 键只读 y±1 平面：面邻格 +
//!   corner_ao/corner_smooth 的角采样落在 u=x/v=z 平面内）→ 天然就是
//!   section 粒度的 ≤3 层；
//! - axis0/2（x/z 面）：脏 x/z 列 ±1 的**整高**重放——列内 y 向锚点链
//!   不可截断（上段论证）；未脏列（≥13/16）逐字节复用；
//! - 非立方形状段：按 section 复用（形状 quad 只读方块 ±1 邻格），脏
//!   section = 脏 y 区间相交的 ≤3 个。
//!
//! 产物按桶存储（桶键 = axis | dir<<2 | 层号<<3，section 归属独立），
//! quad 整体记在**锚行**所在 section；整柱扫描在单 (轴,向,层) 内按锚
//! (v,u) 升序发射、锚行跨 section 单调递增，因此 combine 以
//! 「(轴↑,向,层,section↑)」嵌套即逐字节复现整柱发射序
//! （tests/sections.rs 全场景 + 单格编辑两路锁定）。
//!
//! **收益与债（诚实登记，交卷汇报同步）**：单格编辑重放单元 = 2 向 ×
//! (3 x 列 + 3 z 列 + 3 y 层) = 18 个 (轴,向,层) 扫描行 + ≤3 个形状
//! section（整柱 = 832 行 + 24 section，≈2%）；但 axis0/2 脏列是整高
//! 重放，24 个 section 的 x/z 面桶均被重写（其中 ≥21 个内容不变）——
//! x/z 面维度的「section 产物复用」是字节重写而非跳过，登记为债。
//! 彻底消除该耦合需网格语义变更（section 独立贪心 = golden 三表重锚），
//! 属后续波。

use std::collections::BTreeMap;

use crate::mesher::{MeshData, Rows, Sink, VERTEX_STRIDE, build_into, vertex_bytes};

/// 区块 section 数（384/16，v6 原版几何）。
pub const SECTION_COUNT: usize = mcv_core::CHUNK_SY / 16;

/// 全柱 (轴,向,层) 扫描行总数（观测分母：2 向 × (16 + 384 + 16)）。
pub const TOTAL_SCANNED_ROWS: usize = 2 * (16 + mcv_core::CHUNK_SY + 16);

/// 单 kind（不透明或水）的分 section 产物。
#[derive(Default, Clone)]
pub struct KindProducts {
    sections: Vec<SectionProduct>,
}

impl KindProducts {
    /// 空产物（combine 等价于空网格）。
    pub fn empty() -> Self {
        Self {
            sections: vec![SectionProduct::default(); SECTION_COUNT],
        }
    }

    /// 按整柱扫描序拼接为 [`MeshData`]——与同快照 [`crate::mesher::build_mesh`]
    /// 逐字节相同（tests/sections.rs 全场景锁）。
    pub fn combine(&self) -> MeshData {
        let mut out = MeshData::default();
        // 整柱发射序：axis(0..3) → dir(0..2) → layer。
        // axis0：层 = x 列；axis2：层 = z 列；单 (轴,向,层) 内锚行 v 升序
        // ⇒ 锚 section 升序 ⇒ 桶内已按序，桶间按 section 升序拼接即可。
        // axis1：层 = 全局 y 行（0..384），桶键层号只存低 4 位，
        // section = y>>4 唯一定位（y 面 quad 不跨 y 层）。
        for dir in 0..2usize {
            for axis in [0usize, 2] {
                for layer in 0..16usize {
                    let tag = (axis | (dir << 2) | (layer << 3)) as u8;
                    for sec in self.sections.iter() {
                        sec.append_bucket(tag, &mut out);
                    }
                }
            }
            for ylayer in 0..mcv_core::CHUNK_SY {
                let tag = (1 | (dir << 2) | ((ylayer & 15) << 3)) as u8;
                self.sections[ylayer >> 4].append_bucket(tag, &mut out);
            }
        }
        // 形状段整柱序 = 全部贪心之后（build_into 的 emit_shapes 位序），
        // 段内 y 升序 ⇒ section 升序拼接。
        for sec in self.sections.iter() {
            sec.append_shapes(&mut out);
        }
        out
    }
}

/// 一个 section 的分桶产物：贪心桶（tag → 顶点/索引，索引记桶内基）+
/// 形状段（本 section 方块发射的 quad）。
#[derive(Default, Clone)]
struct SectionProduct {
    buckets: BTreeMap<u8, (Vec<u8>, Vec<u32>)>,
    shapes: (Vec<u8>, Vec<u32>),
}

impl SectionProduct {
    fn append_bucket(&self, tag: u8, out: &mut MeshData) {
        let Some((v, i)) = self.buckets.get(&tag) else {
            return;
        };
        let base = out.vertex_base();
        out.vertices.extend_from_slice(v);
        out.indices.extend(i.iter().map(|x| x + base));
    }

    fn append_shapes(&self, out: &mut MeshData) {
        let base = out.vertex_base();
        out.vertices.extend_from_slice(&self.shapes.0);
        out.indices.extend(self.shapes.1.iter().map(|x| x + base));
    }
}

/// 分桶 sink：与 [`MeshData`] 直写实现同一扫描主体（build_into），只差
/// 「字节落在哪」。begin_* 声明归属后，quad 的 4 顶点/索引整组路由到
/// 该 section 的桶；`vertex_base` 保持整柱序语义（与 MeshData 一致），
/// 落桶时替换为桶内基。
struct BucketSink<'a> {
    sections: &'a mut [SectionProduct],
    /// 当前归属：section + 桶 tag（None = 形状段）。
    cur_sec: usize,
    cur_tag: Option<u8>,
    /// 整柱（扫描序）已发顶点数。
    emitted: u32,
    /// 当前 quad 的桶内基（begin_* 时记录）。
    quad_local_base: u32,
}

impl BucketSink<'_> {
    fn buffers(&mut self) -> (&mut Vec<u8>, &mut Vec<u32>) {
        let sec = &mut self.sections[self.cur_sec];
        match self.cur_tag {
            Some(tag) => {
                let b = sec.buckets.entry(tag).or_default();
                (&mut b.0, &mut b.1)
            }
            None => (&mut sec.shapes.0, &mut sec.shapes.1),
        }
    }
}

impl Sink for BucketSink<'_> {
    fn begin_greedy(&mut self, sec: usize, tag: u8) {
        self.cur_sec = sec;
        self.cur_tag = Some(tag);
        self.quad_local_base = (self.sections[sec]
            .buckets
            .get(&tag)
            .map_or(0, |b| b.0.len())
            / VERTEX_STRIDE) as u32;
    }

    fn begin_shape(&mut self, sec: usize) {
        self.cur_sec = sec;
        self.cur_tag = None;
        self.quad_local_base = (self.sections[sec].shapes.0.len() / VERTEX_STRIDE) as u32;
    }

    fn vertex_base(&self) -> u32 {
        self.emitted
    }

    fn push_vertex(
        &mut self,
        x: f32,
        y: f32,
        z: f32,
        uu: u16,
        vv: u16,
        tex: u16,
        blk: u8,
        sky: u8,
        ao: u8,
        flags: u8,
    ) {
        let b = vertex_bytes(x, y, z, uu, vv, tex, blk, sky, ao, flags);
        let (v, _) = self.buffers();
        v.extend_from_slice(&b);
        self.emitted += 1;
    }

    fn push_indices(&mut self, _base: u32, rel: &[u32; 6]) {
        let lb = self.quad_local_base;
        let (_, i) = self.buffers();
        i.extend(rel.iter().map(|r| lb + r));
    }
}

/// 脏行集重算入口：`rows = None` = 整柱重算（与 [`crate::mesher::build_mesh`]
/// 等价的产物化路径）；`Some(rows)` 只重放受影响扫描行，其余桶复用
/// `old`（缺省 = 空产物，即首建）。返回产物 + 重放工作量
/// （重放 (轴,向,层) 扫描行数 + 重放形状 section 数；整柱 =
/// [`TOTAL_SCANNED_ROWS`] + [`SECTION_COUNT`]，任务 #172 验收 3 的观测面）。
///
/// **前提（调用方责任，刀 B 接线保证）**：`old` 来自同一 3×3 快照的
/// 上一次产物，且 `rows` 未覆盖部分对应的快照字节未变——单格编辑 =
/// 编辑盒（含重光照波及盒）的并集。前提不满足时的正确回退 = 传
/// `rows = None` 整柱重算（保守路径，语义同 `build_mesh`）。
pub fn build_products(
    voxels: &[Option<&[u16]>; 9],
    lights: &[Option<&[u8]>; 9],
    kind: u32,
    rows: Option<Rows>,
    old: Option<KindProducts>,
) -> (KindProducts, usize) {
    let rows = rows.unwrap_or(Rows::ALL);
    let mut product = old.unwrap_or_else(KindProducts::empty);
    // 清空待重放桶（scan 只重写 rows 覆盖的桶；未清空的桶必须保持旧字节，
    // 被清空的桶若重放后无 quad 则与「从未有 quad」字节等价 = 缺席）。
    for dir in 0..2usize {
        for layer in 0..16usize {
            for axis in [0usize, 2] {
                if rows.active(axis, layer) {
                    let tag = (axis | (dir << 2) | (layer << 3)) as u8;
                    for sec in product.sections.iter_mut() {
                        sec.buckets.remove(&tag);
                    }
                }
            }
        }
        for ylayer in 0..mcv_core::CHUNK_SY {
            if rows.active(1, ylayer) {
                let tag = (1 | (dir << 2) | ((ylayer & 15) << 3)) as u8;
                product.sections[ylayer >> 4].buckets.remove(&tag);
            }
        }
    }
    let mut shaped = 0;
    for sec in 0..SECTION_COUNT {
        if rows.shape_active(sec) {
            product.sections[sec].shapes = Default::default();
            shaped += 1;
        }
    }
    let mut sink = BucketSink {
        sections: &mut product.sections,
        cur_sec: 0,
        cur_tag: None,
        emitted: 0,
        quad_local_base: 0,
    };
    build_into(voxels, lights, kind, rows, &mut sink);
    let scanned = scanned_rows(&rows) + shaped;
    (product, scanned)
}

/// `rows` 覆盖的 (轴,向,层) 扫描行数（含两向）。
fn scanned_rows(rows: &Rows) -> usize {
    (0..2)
        .map(|_| {
            (0..16)
                .filter(|l| rows.active(0, *l) || rows.active(2, *l))
                .count()
                + (0..mcv_core::CHUNK_SY)
                    .filter(|l| rows.active(1, *l))
                    .count()
        })
        .sum()
}

/// 单格编辑的重算行集（x0/z0 区块内局部 0..15，y 绝对世界 y）。
/// 多格编辑 = 各盒 [`Rows::from_bbox`] 的 [`Rows::union`]。
pub fn edit_rows(x0: usize, y0: i32, z0: usize) -> Rows {
    Rows::from_bbox(x0 as i32, y0, z0 as i32, x0 as i32, y0, z0 as i32)
}
