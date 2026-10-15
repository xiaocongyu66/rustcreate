//! NoiseChunk cell 预积分插值外壳 — 26.1 `NoiseChunk` + `Mth` +
//! `NoiseBasedChunkGenerator` 逐版面移植（cell-perf-report.md §2 引注）。
//!
//! 目标：被 `minecraft:interpolated` 包裹的 5 个子树（final_density 树内
//! 恰 5 处，cell-perf-report.md §1）只在 cell 角点采样（cell 尺寸 (4,8,4)＝
//! noiseSettings size_horizontal=1/size_vertical=2 × 4），cell 内 4×8×4=128
//! 体素三线插值——base_3d/splines 等重 3D 求值从逐体素降到角面（~64×）。
//! 树内非包裹节点（squeeze/min/blend 门外壳/noodle 门）逐体素直算。
//!
//! 26.1 官方口径（mc-ref 实文件摘录）：
//! - 角面缓存：`NoiseInterpolator` slice0/slice1 = `[cellCountXZ+1=5]
//!   [cellCountY+1=49]` 双平面（NoiseChunk.java:693-717 allocateSlice），
//!   `fillSlice` 以 `inCellX=inCellY=inCellZ=0` 直算原始树（:235-252）——
//!   角值 = 单点 raw 值（`--density` 探针同口径，已 max|d|=0）。
//!   本壳按「角列 xz（≡0 mod 4）→ 49 层角值」缓存，与 slice 平面值集
//!   全等（同一列的全部角层）。
//! - 服务：真 chunk 生成环（NoiseBasedChunkGenerator.java:392-453）每体素
//!   `updateForY/X/Z` 重算（NoiseInterpolator.java:749-766）——**Y 最内 →
//!   X → Z 最外**，`Mth.lerp(t, lo, hi) = lo + t·(hi−lo)`（Mth.java:540-542）：
//!   `value = lerp(fz, lerp(fx, lerp(fy, n000, n010), lerp(fy, n100, n110)),
//!   lerp(fx, lerp(fy, n001, n011), lerp(fy, n101, n111)))`。
//!   （`Mth.lerp3` 是 X 最内的另一关联序——只服务 `cache_all_in_cell`
//!   通道，overworld final_density 不使用；NoiseChunk.java:769 的
//!   fillingCell 分支。）
//! - 角命名 nXYZ（`selectCellYZ`，NoiseChunk.java:732-745）：x∈{0,1} =
//!   slice0/1（相差 cellWidth=4），y∈{0,1} = cellYIndex 底/顶（相差
//!   cellHeight=8），z 同 x。系数 fx=inCellX/4、fy=inCellY/8、fz=inCellZ/4。
//! - 合成（final_density JSON 逐节点）：`min(squeeze(0.64·W1插值·),
//!   noodle_device)`，noodle_device = `range_choice(W2插值 ∈ [−1e6,0) → 64，
//!   否则 add(W3插值, 1.5·max(|W4插值|, |W5插值|)))`——门/abs/mul/max 在
//!   包裹**外**，作用于插值后值（对比：ridge abs 外包 ↔ W3 基底整段包）。
//! - 任意点（非 chunk fill 环）语义不变：`Interpolated.compute` 直透
//!   wrapped raw（NoiseChunk.java:757-760）——本模块只服务 fill 环等
//!   cell 语义面（terrain.rs `generate` / `column_real_top`、lake_pass
//!   Replay），ore_pass 的单点 raw 面（OreVeinifier 通道）另有口径。

use std::collections::HashMap;
use std::rc::Rc;

use crate::vanilla::terrain::{ColumnState, Orchestrator};

/// cell 宽（x/z 方向-block 数）：noiseSettings.size_horizontal(1) × 4。
/// NoiseChunk.java:108-110（overworld sizeVertical=2 → cellHeight=8）。
pub(crate) const CELL_WIDTH: i32 = 4;
/// cell 高（y 方向-block 数）＝ 8；`cellCountY = 384/8 = 48`。
pub(crate) const CELL_HEIGHT: i32 = 8;
/// 角层总数 = cellCountY + 1 = 49（y = −64..=320 步 8）。
pub(crate) const GY_LEVELS: usize = 49;
/// noodle 内层 range_choice y 下界（noodle.json min_inclusive=-60;
/// 上界 321 本域恒满足——角层顶 y=320）。
pub(crate) const NOODLE_Y_MIN: f64 = -60.0;

/// 角列缓存槽位（final_density 5 处 INTERPOLATED 包裹节点，§1 表）。
pub(crate) const W_TERRAIN: usize = 0;
pub(crate) const W_MASK: usize = 1;
pub(crate) const W_BASE: usize = 2;
pub(crate) const W_RIDGE_A: usize = 3;
pub(crate) const W_RIDGE_B: usize = 4;
pub(crate) const W_SLOTS: usize = 5;

/// 一个角列（world xz ≡ 0 mod 4）的全部角层值：`[槽位][角层 gy]`，
/// gy 的角 y = −64 + 8·gy。
pub struct CornerColumn {
    w: Box<[[f64; GY_LEVELS]; W_SLOTS]>,
}

impl CornerColumn {
    /// 槽位角值（对拍/探针出口；运行时 kernel 直接走字段）。
    #[must_use]
    pub fn corner_value(&self, slot: usize, gy: usize) -> f64 {
        self.w[slot][gy]
    }
}

/// 角列四件套（体素所在的 cell 的 4 个 XZ 角列）：[x0z0, x1z0, x0z1, x1z1]。
/// Rc 在 [`CellShell`] 缓存与填充环 per-column 快照之间共享，serve 期间
/// 无可变访问（缓存只在 cols4 预取时构建）。
pub type CellCorners = [Rc<CornerColumn>; 4];

/// per-世界（可跨块）cell 外壳：角列缓存 + 角面构建。确定性：同为
/// (orch, 角列坐标) 纯函数——双块重放两侧各自构建逐位一致。
pub struct CellShell<'a> {
    orch: &'a Orchestrator,
    cols: HashMap<(i32, i32), Rc<CornerColumn>>,
}

impl<'a> CellShell<'a> {
    /// 缓存挂同一编排器（种子链在 Orchestrator 内）。
    pub fn new(orch: &'a Orchestrator) -> Self {
        Self {
            orch,
            cols: HashMap::new(),
        }
    }

    /// 体素 (block_x, block_z) 所在 cell 的 4 个 XZ 角列（懒建 + 共享）。
    pub fn cols4(&mut self, block_x: i32, block_z: i32) -> CellCorners {
        let kx = block_x.div_euclid(CELL_WIDTH);
        let kz = block_z.div_euclid(CELL_WIDTH);
        [
            self.corner(kx, kz),
            self.corner(kx + 1, kz),
            self.corner(kx, kz + 1),
            self.corner(kx + 1, kz + 1),
        ]
    }

    /// 角列 (kx, kz)（世界坐标 kx·4, kz·4）的角值，未缓存则构建。
    fn corner(&mut self, kx: i32, kz: i32) -> Rc<CornerColumn> {
        let key = (kx * CELL_WIDTH, kz * CELL_WIDTH);
        if let Some(c) = self.cols.get(&key) {
            return Rc::clone(c);
        }
        // 角值 = fillSlice 语义：inCellX=inCellY=inCellZ=0 的单点 raw 求值
        // （NoiseChunk.java:235-252），列状态取角列（flat_cache/
        // cache_2d 的列 = 求值点所在列，逐角列各自采样）。
        let corner_x = key.0;
        let corner_z = key.1;
        let cs: ColumnState = self.orch.column_state(corner_x, corner_z);
        let mut w: Box<[[f64; GY_LEVELS]; W_SLOTS]> = Box::new([[0.0; GY_LEVELS]; W_SLOTS]);
        for (gy, level) in w.iter_mut().enumerate() {
            let corner_y = crate::vanilla::MIN_Y + 8 * gy as i32;
            let y_mc = crate::vanilla::ours_y_to_mc(f64::from(corner_y));
            // W1：地形+slide 全合成（含 base_3d/splines/entrances/
            // spaghetti/underground/pillars 门——树内无包裹节点）。
            level[W_TERRAIN] = self.orch.terrain_density(&cs, corner_x, corner_y, corner_z);
            // W2：noodle 掩膜——range_choice(y∈[-60,321) → noise(noodle,1,1),
            // else −1)（noodle.json 内层分支；else 取 -1 使外层门走
            // when_in_range=64，与 terrain.rs `noodle()` 的 NOODLE_OFF
            // 提前返回同语义）。
            level[W_MASK] = if y_mc < NOODLE_Y_MIN {
                -1.0
            } else {
                self.orch
                    .noodle_mask_at(f64::from(corner_x), y_mc, f64::from(corner_z))
            };
            // W3：厚度基底 add(-0.07500000000000001, -0.025·thickness)
            // ——与 terrain.rs `noodle()` 左半 op 序逐位同（a - b·t）。
            level[W_BASE] = if y_mc < NOODLE_Y_MIN {
                0.0
            } else {
                let t =
                    self.orch
                        .noodle_thickness_at(f64::from(corner_x), y_mc, f64::from(corner_z));
                -0.075_000_000_000_000_01_f64 - 0.025 * t
            };
            // W4/W5：ridge_a/b——abs 在包裹外（对插值后值取 |·|），
            // xz/y 缩放 2.6666666666666665 在包裹内。
            let s = crate::vanilla::terrain::NOODLE_RIDGE_SCALE;
            level[W_RIDGE_A] = if y_mc < NOODLE_Y_MIN {
                0.0
            } else {
                self.orch.noodle_ridge_a_at(
                    f64::from(corner_x) * s,
                    y_mc * s,
                    f64::from(corner_z) * s,
                )
            };
            level[W_RIDGE_B] = if y_mc < NOODLE_Y_MIN {
                0.0
            } else {
                self.orch.noodle_ridge_b_at(
                    f64::from(corner_x) * s,
                    y_mc * s,
                    f64::from(corner_z) * s,
                )
            };
        }
        let rc = Rc::new(CornerColumn { w });
        self.cols.insert(key, Rc::clone(&rc));
        rc
    }

    /// 服务一批角列下的体素插值值：5 槽位各跑三线增量序（Y 最内 →
    /// X → Z），再按 final_density 外壳合成
    /// `min(squeeze(0.64·W1), noodle_device)`。
    ///
    /// 逐位口径 = NoiseBasedChunkGenerator.java:392-453 增量服务序 +
    /// Mth.lerp（`lo + t·(hi−lo)`）：
    /// `lerp(fz, lerp(fx, lerp(fy, n000, n010), lerp(fy, n100, n110)),
    /// lerp(fx, lerp(fy, n001, n011), lerp(fy, n101, n111)))`。
    #[must_use]
    pub fn density_at(cell: &CellCorners, block_x: i32, block_y: i32, block_z: i32) -> f64 {
        // inCell 系数：fx=inCellX/4、fy=inCellY/8、fz=inCellZ/4
        //（updateForY/X/Z 调元 factor = (double)inCell / cell 尺寸）。
        let kx = block_x.div_euclid(CELL_WIDTH);
        let kz = block_z.div_euclid(CELL_WIDTH);
        let in_cell_x = block_x - kx * CELL_WIDTH;
        let in_cell_z = block_z - kz * CELL_WIDTH;
        // 角层 index：cellStartBlockY = floorDiv(y,8)·8；上角 = +8。
        let y_rel = (block_y - crate::vanilla::MIN_Y).clamp(0, (GY_LEVELS as i32 - 1) * 8 - 1);
        let cell_y_index = y_rel / CELL_HEIGHT;
        let in_cell_y = y_rel - cell_y_index * CELL_HEIGHT;
        let gy0 = cell_y_index as usize;
        let gy1 = gy0 + 1;
        let fx = f64::from(in_cell_x) / f64::from(CELL_WIDTH);
        let fy = f64::from(in_cell_y) / f64::from(CELL_HEIGHT);
        let fz = f64::from(in_cell_z) / f64::from(CELL_WIDTH);

        // n000=(x0,y0,z0) n100=(x1,y0,z0) n010=(x0,y1,z0) n110=(x1,y1,z0)
        // n001=(x0,y0,z1) n101=(x1,y0,z1) n011=(x0,y1,z1) n111=(x1,y1,z1)
        // —— 固定 gy 偶对：先 Y 内插（gy0/gy1），再 X（两角列），再 Z。
        let terrain = interp_slot(cell, W_TERRAIN, gy0, gy1, fx, fy, fz);
        let d = crate::vanilla::terrain::squeeze(0.64 * terrain);
        let mask = interp_slot(cell, W_MASK, gy0, gy1, fx, fy, fz);
        // noodle_device：掩膜插值 ∈ [-1e6, 0) → 64（min 不生效），
        // 否则 add(W3插值, 1.5·max(|W4插值|, |W5插值|)) ——与 terrain.rs
        // `noodle()` raw 表达式在同一 op 序（base 左支 + 1.5·max 右支）。
        let noodle = if mask < 0.0 {
            crate::vanilla::terrain::NOODLE_OFF
        } else {
            let base = interp_slot(cell, W_BASE, gy0, gy1, fx, fy, fz);
            let a = interp_slot(cell, W_RIDGE_A, gy0, gy1, fx, fy, fz).abs();
            let b = interp_slot(cell, W_RIDGE_B, gy0, gy1, fx, fy, fz).abs();
            base + 1.5 * a.max(b)
        };
        d.min(noodle)
    }
}

/// 单槽位三线插值（fill 期间增量服务序，见模块头；8 角值取自
/// 4 角列 × gy0/gy1 两层）。
#[inline]
fn interp_slot(
    cell: &CellCorners,
    slot: usize,
    gy0: usize,
    gy1: usize,
    fx: f64,
    fy: f64,
    fz: f64,
) -> f64 {
    let c00 = &cell[0].w[slot];
    let c10 = &cell[1].w[slot];
    let c01 = &cell[2].w[slot];
    let c11 = &cell[3].w[slot];
    let n000 = c00[gy0];
    let n100 = c10[gy0];
    let n010 = c00[gy1];
    let n110 = c10[gy1];
    let n001 = c01[gy0];
    let n101 = c11[gy0];
    let n011 = c01[gy1];
    let n111 = c11[gy1];
    // updateForY（NoiseInterpolator.java:749-754）
    let value_xz00 = n000 + fy * (n010 - n000);
    let value_xz10 = n100 + fy * (n110 - n100);
    let value_xz01 = n001 + fy * (n011 - n001);
    let value_xz11 = n101 + fy * (n111 - n101);
    // updateForX（:756-759）
    let value_z0 = value_xz00 + fx * (value_xz10 - value_xz00);
    let value_z1 = value_xz01 + fx * (value_xz11 - value_xz01);
    // updateForZ（:761-763）
    value_z0 + fz * (value_z1 - value_z0)
}
