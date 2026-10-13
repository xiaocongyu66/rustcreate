//! #190 取证锁：渲染（mesh）与碰撞（collision）**双源一致性**——派单
//! 口径「渲染空气格碰撞必须空气」的逐方块表对偶（碰撞实心 ⟹ 网格必须
//! 出几何；对偶方向「mesh 无几何 ⟹ 碰撞必须空气」等价）。
//!
//! 真机症状「在已加载区块中平飞，面前什么方块都没有却撞上实心」的
//! 表级假说：某 BlockId 在碰撞表（`mcv_core::shape` 模板盒 / `solid`
//! 位）有阻挡盒，但网格器（`mcv_mesher`）对它零发射 = 隐形实心。本测
//! 对**注册表全量 id** 逐一隔离判定（孤格 + 全空气邻域 + 满天空光，
//! 排除面遮蔽造成的合法零发射）。
//!
//! 反方向不锁（原版语义：火把/花草/楼梯 outline 可见性各异，cross 有
//! 几何无碰撞属正确）。跨区块边界口径：碰撞查询走世界坐标逐格读
//! （`WorldView::block`），网格器邻域 = 3×3 快照——本测的 9 邻域快照
//! 与 `snapshot_neighborhood` 同款，边界格（15,70,15）单列一条用例，
//! 邻块全空气，断言同一格的双源一致。

use mcv_core::{BlockId, BlockPos, CHUNK_VOL, ChunkPos, ID_MASK, WORLD_MAX_Y, WORLD_MIN_Y, vidx};
use mcv_game::{VoxelAccess, blockshapes};
use mcv_mesher::{VERTEX_STRIDE, build_mesh};

/// 被测格的世界坐标（远离一切世界边界）。
const CELL: (i32, i32, i32) = (8, 70, 8);
/// 边界格用例：中心区块的 (+X,+Z) 角。
const CELL_EDGE: (i32, i32, i32) = (15, 70, 15);

/// 单格世界（稀疏 HashMap）：与生产 `WorldView::block` 同界的空气默认。
struct OneBlock {
    id: BlockId,
    at: (i32, i32, i32),
}

impl VoxelAccess for OneBlock {
    fn block(&self, p: BlockPos) -> BlockId {
        if !(WORLD_MIN_Y..WORLD_MAX_Y).contains(&p.y) {
            return BlockId(0);
        }
        if (p.x, p.y, p.z) == self.at {
            self.id
        } else {
            BlockId(0)
        }
    }
    fn light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, _c: ChunkPos) -> bool {
        true
    }
}

/// 被测格的碰撞盒数（世界坐标 (0,0,0) 基底的中心区块 = 碰撞读的世界
/// 格 = `CELL`，与网格原点一致：网格器以区块原点 (0,0,0) 铺设，CELL
/// 的区块即中心区块）。
fn collision_count(id: BlockId, at: (i32, i32, i32)) -> usize {
    let world = OneBlock { id, at };
    let mut boxes = [blockshapes::EMPTY_AABB; blockshapes::MAX_SHAPE_BOXES];
    blockshapes::collision_boxes(&world, BlockPos::new(at.0, at.1, at.2), &mut boxes)
}

/// 隔离单格网格：中心区块含被测格、8 个邻块全空气（与
/// `snapshot_neighborhood` 同款平铺布局），返回 (opaque 顶点数,
/// water 顶点数)。
fn isolated_mesh(id: BlockId, at: (i32, i32, i32)) -> (usize, usize) {
    let vol = CHUNK_VOL;
    let mut voxels = vec![0u16; 9 * vol];
    let lights = vec![0xF0u8; 9 * vol]; // 满天光，排除光照面
    voxels[4 * vol + vidx(at.0 as usize, at.1, at.2 as usize)] = id.0;
    let mut vsa = [None; 9];
    let mut lsa = [None; 9];
    for i in 0..9 {
        vsa[i] = Some(&voxels[i * vol..(i + 1) * vol]);
        lsa[i] = Some(&lights[i * vol..(i + 1) * vol]);
    }
    let solid = build_mesh(&vsa, &lsa, 0);
    let water = build_mesh(&vsa, &lsa, 1);
    (
        solid.vertices.len() / VERTEX_STRIDE,
        water.vertices.len() / VERTEX_STRIDE,
    )
}

fn vertex_y(mesh: &mcv_mesher::MeshData) -> Vec<f32> {
    // 顶点布局：偏移 0..12 = f32 LE (x, y, z)（QuadVertex 平铺，
    // mesher.rs push_vertex 注）。
    mesh.vertices
        .as_chunks::<VERTEX_STRIDE>()
        .0
        .iter()
        .map(|v| f32::from_le_bytes([v[4], v[5], v[6], v[7]]))
        .collect()
}

#[test]
fn every_collidable_block_emits_geometry() {
    let mut checked_collidable = 0usize;
    for (i, def) in mcv_core::BLOCKS.iter().enumerate() {
        if i == 0 {
            continue; // air
        }
        let id = BlockId(i as u16);
        let n = collision_count(id, CELL);
        if n == 0 {
            continue; // 无碰撞（火把/花草/水/非固体）：反方向不锁
        }
        checked_collidable += 1;
        let (solid_v, water_v) = isolated_mesh(id, CELL);
        assert!(
            solid_v + water_v > 0,
            "K-MESH-COLL-PARITY：{}（id={i}）碰撞盒 {n} 个但网格零顶点——\
             真机隐形实心墙（渲染空气格碰撞实心）",
            def.name,
        );
    }
    // 哨兵：注册表非空且确有碰撞项进入判定（防空表假绿）。
    assert!(
        checked_collidable >= 500,
        "碰撞项过少（{checked_collidable}）= 表或断言坏了"
    );
}

/// 边界格（15,70,15）双源一致：石整块的碰撞与几何同真（跨区块邻域
/// 全空气时面必须可见，不产生「边界隐形实心」）。
#[test]
fn boundary_cell_parity() {
    let stone = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "stone")
        .unwrap();
    let id = BlockId(stone as u16);
    assert_eq!(collision_count(id, CELL_EDGE), 1, "stone 碰撞必须整格");
    let (solid_v, _) = isolated_mesh(id, CELL_EDGE);
    assert!(solid_v > 0, "边界格 stone 必须出几何（邻域全空气）");
    // 顶点 y 落在 [70, 71]（绝对域，无绕回）：#71 绕回面回归口径。
    let ys = {
        let vol = CHUNK_VOL;
        let mut voxels = vec![0u16; 9 * vol];
        let lights = vec![0xF0u8; 9 * vol];
        voxels[4 * vol + vidx(CELL_EDGE.0 as usize, CELL_EDGE.1, CELL_EDGE.2 as usize)] = id.0;
        let mut vsa = [None; 9];
        let mut lsa = [None; 9];
        for i in 0..9 {
            vsa[i] = Some(&voxels[i * vol..(i + 1) * vol]);
            lsa[i] = Some(&lights[i * vol..(i + 1) * vol]);
        }
        vertex_y(&build_mesh(&vsa, &lsa, 0))
    };
    assert!(
        ys.iter()
            .all(|y| (*y - 70.0).abs() < 1e-3 || (*y - 71.0).abs() < 1e-3),
        "y 顶点越出格域（绕回面回归）：{ys:?}"
    );
}

/// #71/#190 深层带网格覆盖锁：整段 y ∈ [-64,-49]（绝对负域）实心 +
/// 上方全空气，网格必须产出 y=-64.0 与 y=-49.0 的顶点——负 y 段不许
/// 有任何「体素实心但网格缺席」的深度盲区（透明/隐形两症状的渲染面）。
#[test]
fn deep_section_below_zero_is_meshed() {
    let stone = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "stone")
        .unwrap() as u16;
    let vol = CHUNK_VOL;
    let mut voxels = vec![0u16; 9 * vol];
    let lights = vec![0xF0u8; 9 * vol];
    for y in -64..=-49 {
        for z in 0..16 {
            for x in 0..16 {
                voxels[4 * vol + vidx(x, y, z)] = stone;
            }
        }
    }
    let mut vsa = [None; 9];
    let mut lsa = [None; 9];
    for i in 0..9 {
        vsa[i] = Some(&voxels[i * vol..(i + 1) * vol]);
        lsa[i] = Some(&lights[i * vol..(i + 1) * vol]);
    }
    let mesh = build_mesh(&vsa, &lsa, 0);
    let ys = vertex_y(&mesh);
    assert!(!ys.is_empty(), "负 y 实心段零几何（深度盲区回归）");
    let has = |t: f32| ys.iter().any(|y| (y - t).abs() < 1e-3);
    assert!(has(-64.0), "缺底面顶点 y=-64（#71 绕回面口径）");
    assert!(has(-49.0), "缺顶面顶点 y=-49");
    assert!(
        ys.iter().all(|y| *y >= -64.0 - 1e-3 && *y <= -48.0 + 1e-3),
        "顶点越出实心段（y 绕回）：min={} max={}",
        ys.iter().fold(f32::INFINITY, |a, &b| a.min(b)),
        ys.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b))
    );
}

/// id 位段口径：state nibble 不改变碰撞↔几何同真（同 id 全 16 态
/// 逐一，防「某状态位网格跳过但碰撞不跳」的漂移面）。
#[test]
fn state_nibble_parity() {
    let stone = mcv_core::BLOCKS
        .iter()
        .position(|d| d.name == "stone")
        .unwrap() as u16;
    for st in 0..16u16 {
        let id = BlockId((stone & ID_MASK) | (st << mcv_core::STATE_SHIFT));
        let n = collision_count(id, CELL);
        let (solid_v, water_v) = isolated_mesh(id, CELL);
        assert_eq!(
            n > 0,
            solid_v + water_v > 0,
            "stone state={st} 碰撞({n}) 与几何({solid_v}+{water_v}) 不同真"
        );
    }
}
