//! Synthetic-data tests for the C++ greedy mesher via `Mesher`.
//! All light arrays are filled with 0xF0 (sky 15, block 0).

use mcv_mesher::{MeshBuffer, Mesher, Slot};

const VOL: usize = 65536;

type Chunk = (Box<[u16; VOL]>, Box<[u8; VOL]>);

fn chunk(id: u16, light: u8) -> Chunk {
    (Box::new([id; VOL]), Box::new([light; VOL]))
}

fn put(vox: &mut [u16; VOL], x: usize, y: usize, z: usize, id: u16) {
    vox[(y << 8) | (z << 4) | x] = id;
}

/// All 9 slots loaded: center chunk plus 8 copies of `side` (usually air).
fn full9<'a>(center: &'a Chunk, side: &'a Chunk) -> [Option<Slot<'a>>; 9] {
    let mut out = [Some(Slot {
        voxels: &side.0[..],
        light: &side.1[..],
    }); 9];
    out[4] = Some(Slot {
        voxels: &center.0[..],
        light: &center.1[..],
    });
    out
}

/// Only the center slot loaded; all 8 neighbours are `None` (opaque
/// boundary for the mesher).
fn only_center<'a>(center: &'a Chunk) -> [Option<Slot<'a>>; 9] {
    let mut out: [Option<Slot<'a>>; 9] = [None; 9];
    out[4] = Some(Slot {
        voxels: &center.0[..],
        light: &center.1[..],
    });
    out
}

/// 按注册名查方块 id（测试只依赖生成表，不硬编码 id）。
fn id_of(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .expect("unknown block name") as u16
}

struct Vtx {
    pos: [f32; 3],
    uv: [u16; 2],
    tex: u16,
    sky: u8,
    ao: u8,
    flags: u8,
}

#[allow(clippy::almost_complete_range)]
#[allow(unknown_lints, clippy::manual_chunks)]
fn decode(buf: &MeshBuffer) -> Vec<Vtx> {
    buf.vertex_data()
        .chunks(24)
        .map(|b| {
            let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
            Vtx {
                pos: [f(0), f(4), f(8)],
                uv: [
                    u16::from_le_bytes([b[12], b[13]]),
                    u16::from_le_bytes([b[14], b[15]]),
                ],
                tex: u16::from_le_bytes([b[16], b[17]]),
                sky: b[19],
                ao: b[20],
                flags: b[21],
            }
        })
        .collect()
}

/// UV 单位回归（2026-10-10 真机纯色事故）：一个方块面必须横跨整 tile
/// 0..65535。旧实现 kUvPerBlock=65535/16 把「1 tile」当「16 方块」，
/// 每面只采样 tile 左上 1 纹素 → 全平台纯色。原版烘焙规则：每个面吃满
/// sprite 满幅 0..1（FaceBakery.java:26-35/166）。
#[test]
fn uv_spans_full_tile_per_block() {
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, 3); // grass block floating in air
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();

    for (qi, quad) in decode(&buf).chunks(4).enumerate() {
        for axis in 0..2 {
            let lo = quad.iter().map(|v| v.uv[axis] as i32).min().unwrap();
            let hi = quad.iter().map(|v| v.uv[axis] as i32).max().unwrap();
            assert_eq!(
                hi - lo,
                65535,
                "quad {qi} uv axis {axis}: single block must span full tile, got {lo}..{hi}"
            );
        }
    }
}

/// 贪心合并面的 uv 环绕：2 方块宽的面跨度 = 2×65535 mod 65536 = 65534
/// （旧 saturate-at-65535 写法会把合并面钉死在 tile 边界=纯色）。
#[test]
fn greedy_merged_face_uv_wraps_past_one_tile() {
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, 3);
    put(&mut c.0, 9, 8, 8, 3); // 2-block strip merges along x
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();

    let merged = decode(&buf).chunks(4).any(|q| {
        (0..2).any(|axis| {
            let lo = q.iter().map(|v| v.uv[axis] as i32).min().unwrap();
            let hi = q.iter().map(|v| v.uv[axis] as i32).max().unwrap();
            hi - lo == 65534
        })
    });
    assert!(
        merged,
        "2-block merged face must wrap uv mod 65536 (expect span 65534)"
    );
}

#[test]
fn single_block_six_faces() {
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, 3); // grass block floating in air

    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    assert_eq!(buf.counts(), (24, 36));

    let verts = decode(&buf);
    for (i, v) in verts.iter().enumerate() {
        let face = (v.flags & 7) as usize;
        // quad grouping: 4 consecutive vertices per face, face ids 0..5
        assert_eq!(i / 4, face, "vertex {i} grouped into face {face}");
        assert_eq!(v.tex as usize, mcv_core::BLOCKS[3].tiles[face] as usize);
    }
    // index pattern: per quad (0,1,2),(0,2,3)
    for (t, &ix) in buf.indices().iter().enumerate() {
        let expected = (t / 6 * 4) as u32 + [0u32, 1, 2, 0, 2, 3][t % 6];
        assert_eq!(ix, expected, "index {t}");
    }
}

#[test]
fn block_table_tex_parity_with_mcv_core() {
    // 生成表（cpp/src/blocks_gen.inc）与 mcv_core::BLOCKS 必须逐 id 一致：
    // 对每个 id 悬空放一块，不透明 pass 按 shape 模板出几何、逐面 tile 层
    // 相同。水走水 pass 除外。全空气环境下各形状顶点数：Cube/单根 Fence/
    // 单块 Slab/Torch 细柱 = 6 面；Cross = 2 条双面 quad；Stairs = 两盒
    // 12 面（不做盒间剔除，宁多勿漏）。
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    // 生成表 geom=false 的隐形方块（屏障/光源/空气族/结构空位/气泡柱）：原版
    // 不可见、不产几何（ci/gen-blocks.py INVISIBLE_GEOM；生成表侧由
    // mcv_core/tests/blocks_table.rs attribute_hotspots_match_java 锁定）。
    // 2026-10-10 M7a 审计前它们顶着「层 0 贴图」出全盒占位，现期望 0 面。
    const INVISIBLE: [&str; 6] = [
        "barrier",
        "light",
        "cave_air",
        "void_air",
        "structure_void",
        "bubble_column",
    ];
    for id in 1u16..mcv_core::BLOCKS.len() as u16 {
        if id == 5 {
            // water belongs to the water pass
            continue;
        }
        put(&mut c.0, 8, 8, 8, id);
        let buf = mesher.build(&full9(&c, &side), 0).unwrap();
        let def = &mcv_core::BLOCKS[id as usize];
        let expected = if INVISIBLE.contains(&def.name) {
            (0, 0)
        } else {
            match mcv_core::shape::Shape::from_u8(def.shape) {
                mcv_core::shape::Shape::Cross => (8, 24),
                mcv_core::shape::Shape::Stairs => (48, 72),
                _ => (24, 36),
            }
        };
        assert_eq!(buf.counts(), expected, "block {id} face count");
        for v in decode(&buf) {
            let face = (v.flags & 7) as usize;
            assert_eq!(
                v.tex as usize, def.tiles[face] as usize,
                "block {id} face {face} tex layer"
            );
        }
        put(&mut c.0, 8, 8, 8, 0);
    }
}

#[test]
fn greedy_merges_row_of_four() {
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    // solid dirt floor at y=7 so the row's -Y faces are culled
    for x in 0..16 {
        for z in 0..16 {
            put(&mut c.0, x, 7, z, 2);
        }
    }
    // 4x1 stone row on top at y=8, z=8
    for x in 0..4 {
        put(&mut c.0, x, 8, 8, 1);
    }

    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    let verts = decode(&buf);
    // stone top face (+Y): one merged quad -> 4 vertices
    let stone_top = mcv_core::BLOCKS[1].tiles[2];
    let top: Vec<_> = verts
        .iter()
        .filter(|v| v.flags & 7 == 2 && v.tex == stone_top)
        .collect();
    assert_eq!(top.len(), 4, "4x1 row top must merge into a single quad");
    for v in &top {
        assert_eq!(v.pos[1], 9.0);
        assert_eq!(v.ao, 3, "open sky corners are unoccluded");
    }
    let xs: Vec<i32> = top.iter().map(|v| v.pos[0] as i32).collect();
    let zs: Vec<i32> = top.iter().map(|v| v.pos[2] as i32).collect();
    assert!(xs.contains(&0) && xs.contains(&4), "span x 0..4");
    assert!(zs.contains(&8) && zs.contains(&9), "span z 8..9");
}

#[test]
fn ao_l_shape_corner_values() {
    // Floor: full stone layer at y=0 (top faces at y=1).
    // L-shaped occluders at y=1: (0,1,0), (1,1,0), (0,1,1).
    //
    // Hand-computed AO of floor cell (1,0,1) top-face corners; each corner
    // checks side1/side2/corner cells in the y=1 plane:
    //   corner (x=1,z=1): side1=(0,1,1) opaque, side2=(1,1,0) opaque
    //     -> s1 && s2 -> ao = 0
    //   corner (x=2,z=1): side1=(2,1,1) air, side2=(1,1,0) opaque,
    //     corner=(2,1,0) air -> 3-(0+1+0) = 2
    //   corner (x=1,z=2): mirrored -> 2
    //   corner (x=2,z=2): all three air -> 3
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    for x in 0..16 {
        for z in 0..16 {
            put(&mut c.0, x, 0, z, 1);
        }
    }
    put(&mut c.0, 0, 1, 0, 1);
    put(&mut c.0, 1, 1, 0, 1);
    put(&mut c.0, 0, 1, 1, 1);

    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    let verts = decode(&buf);
    let at = |x: f32, z: f32| {
        verts
            .iter()
            .find(|v| v.flags & 7 == 2 && v.pos[0] == x && v.pos[1] == 1.0 && v.pos[2] == z)
            .unwrap_or_else(|| panic!("missing top vertex at ({x}, {z})"))
    };
    assert_eq!(at(1.0, 1.0).ao, 0);
    assert_eq!(at(2.0, 1.0).ao, 2);
    assert_eq!(at(1.0, 2.0).ao, 2);
    assert_eq!(at(2.0, 2.0).ao, 3);
}

#[test]
fn water_mesh_rules() {
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);

    // Single water block: 6 faces (all neighbours air).
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, 5);
    let nb = full9(&c, &side);
    let w = mesher.build(&nb, 1).unwrap();
    assert_eq!(w.counts(), (24, 36));
    for v in decode(&w) {
        if v.flags & 7 == 2 {
            assert_eq!(v.flags & 8, 8, "water top face waves");
            assert!(
                (v.pos[1] - 8.9f32).abs() < 1e-4,
                "top face sunk by 0.1, got {}",
                v.pos[1]
            );
        } else {
            assert_eq!(v.flags & 8, 0, "only top faces wave");
        }
    }

    // Two adjacent water blocks (x=8, x=9): the shared water-water face is
    // culled, so the only +X/-X faces sit at planes x=10/x=8; the two top
    // faces merge into one 2x1 quad (4 verts, still sunk + waving).
    let mut c2 = chunk(0, 0xF0);
    put(&mut c2.0, 8, 8, 8, 5);
    put(&mut c2.0, 9, 8, 8, 5);
    let w2 = mesher.build(&full9(&c2, &side), 1).unwrap();
    assert_eq!(w2.counts(), (24, 36), "shared face culled, rest merges");
    let verts2 = decode(&w2);
    assert!(
        verts2.iter().all(|v| match v.flags & 7 {
            0 => v.pos[0] == 10.0,
            1 => v.pos[0] == 8.0,
            _ => true,
        }),
        "no faces at the shared water-water plane x=9"
    );
    let top2: Vec<_> = verts2.iter().filter(|v| v.flags & 7 == 2).collect();
    assert_eq!(top2.len(), 4, "top faces merge into one quad");
    for v in top2 {
        assert_eq!(v.flags & 8, 8);
        assert!((v.pos[1] - 8.9f32).abs() < 1e-4);
        assert!(v.pos[0] == 8.0 || v.pos[0] == 10.0);
    }

    // Opaque pass ignores water entirely.
    let o = mesher.build(&nb, 0).unwrap();
    assert_eq!(o.counts(), (0, 0));
}

#[test]
fn null_neighbour_is_opaque_boundary() {
    let mesher = Mesher::new(1 << 20).unwrap();
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 15, 8, 8, 1); // stone flush against the +X chunk border

    let buf = mesher.build(&only_center(&c), 0).unwrap();
    // +X face faces the unloaded neighbour -> culled; the other 5 faces
    // face in-chunk air -> generated.
    assert_eq!(buf.counts(), (20, 30));
    let verts = decode(&buf);
    assert!(
        verts.iter().all(|v| v.flags & 7 != 0),
        "no +X faces toward unloaded neighbour"
    );
    assert_eq!(verts.iter().filter(|v| v.flags & 7 == 1).count(), 4);
    assert_eq!(verts.iter().filter(|v| v.flags & 7 == 2).count(), 4);
}

#[test]
fn shape_templates_structural_invariants() {
    // 每种非立方形状：中心 (8,8,8) 悬浮于全空气 3x3x3 世界。断言：有几何、
    // 两次 build 逐字节相等（确定性替代黄金逐顶点）、索引不越界、顶点全部
    // 落在方块 AABB 内、光照采到邻空气格 sky=15。
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    for (name, lo, hi) in [
        ("flower_red", [8.0f32, 8.0, 8.0], [9.0, 9.0, 9.0]),
        ("torch", [8.4, 8.0, 8.4], [8.6, 8.625, 8.6]),
        ("oak_fence", [8.0, 8.0, 8.0], [9.0, 9.0, 9.0]),
        ("oak_slab", [8.0, 8.0, 8.0], [9.0, 8.5, 9.0]),
        ("oak_stairs", [8.0, 8.0, 8.0], [9.0, 9.0, 9.0]),
    ] {
        let id = id_of(name);
        let mut c = chunk(0, 0xF0);
        put(&mut c.0, 8, 8, 8, id);
        let a = mesher.build(&full9(&c, &side), 0).unwrap();
        let b = mesher.build(&full9(&c, &side), 0).unwrap();
        assert_eq!(a.vertex_data(), b.vertex_data(), "{name}: 顶点不确定");
        assert_eq!(a.indices(), b.indices(), "{name}: 索引不确定");
        let (vc, ic) = a.counts();
        assert!(vc > 0 && ic > 0, "{name}: 无几何");
        assert!(a.indices().iter().all(|&i| i < vc), "{name}: 索引越界");
        for v in decode(&a) {
            for axis in 0..3 {
                assert!(
                    (lo[axis]..=hi[axis]).contains(&v.pos[axis]),
                    "{name}: 顶点 {:?} 越出 AABB {lo:?}..{hi:?}",
                    v.pos
                );
            }
            assert_eq!(v.sky, 15, "{name}: 邻空气格 sky 应为 15");
        }
        put(&mut c.0, 8, 8, 8, 0);
    }
}

#[test]
fn cross_plant_two_double_sided_quads() {
    // 十字植物：2 条对角 quad ×（正反索引各 6）= 8 顶点 / 24 索引；
    // flags 全部 +Y 面档、ao 恒 3、贴图用方块 tile 层、顶点落在对角线上。
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let flower = id_of("flower_red");
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, flower);
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    assert_eq!(buf.counts(), (8, 24));
    for v in decode(&buf) {
        assert_eq!(v.flags & 7, 2, "cross 面档应为 +Y");
        assert_eq!(v.ao, 3, "cross 无 AO 采样，应全亮");
        assert_eq!(
            v.tex as usize,
            mcv_core::BLOCKS[flower as usize].tiles[2] as usize
        );
        let fx = v.pos[0] - 8.0;
        let fz = v.pos[2] - 8.0;
        assert!(
            (fx - fz).abs() < 1e-4 || (fx + fz - 1.0).abs() < 1e-4,
            "顶点应落在两条对角线上: ({fx}, {fz})"
        );
    }
}

#[test]
fn slab_half_boxes_and_exposed_mid_face() {
    // 下半砖（state 0）y∈[8,8.5]：5 外面 + y=0.5 中层面（永远暴露）= 6 面；
    // 上半砖（state 1）y∈[8.5,9] 同 6 面；上下叠放不合并，各自 6 面。
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let slab = id_of("oak_slab");
    let tiles = mcv_core::BLOCKS[slab as usize].tiles;

    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, slab);
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    assert_eq!(buf.counts(), (24, 36), "下半砖 6 面");
    for v in decode(&buf) {
        assert!(
            (8.0..=8.5).contains(&v.pos[1]),
            "下半砖顶点 y = {}",
            v.pos[1]
        );
        assert_eq!(v.tex as usize, tiles[(v.flags & 7) as usize] as usize);
    }

    let mut c2 = chunk(0, 0xF0);
    put(&mut c2.0, 8, 8, 8, slab | (1u16 << 12));
    let buf2 = mesher.build(&full9(&c2, &side), 0).unwrap();
    assert_eq!(buf2.counts(), (24, 36), "上半砖 6 面");
    for v in decode(&buf2) {
        assert!(
            (8.5..=9.0).contains(&v.pos[1]),
            "上半砖顶点 y = {}",
            v.pos[1]
        );
    }

    let mut c3 = chunk(0, 0xF0);
    put(&mut c3.0, 8, 8, 8, slab);
    put(&mut c3.0, 8, 9, 8, slab | (1u16 << 12));
    let buf3 = mesher.build(&full9(&c3, &side), 0).unwrap();
    assert_eq!(buf3.counts(), (48, 72), "叠放两块各 6 面，无合并");
    let ys: Vec<f32> = decode(&buf3).iter().map(|v| v.pos[1]).collect();
    assert!(ys.contains(&8.5) && ys.contains(&9.5), "两个中层面都在");
}

#[test]
fn stairs_facing_and_top_flip() {
    // facing 编码 0=+Z 1=-Z 2=+X 3=-X，语义 = 26.1 FACING（玩家水平视线
    // 同向，StairBlock.java:101-102 getHorizontalDirection）。几何判据按
    // StairBlock.java:37-38 推得：facing=NORTH(-Z) → 上半盒占 -Z 半格，
    // 即踏步（整高半）位于朝向侧——本用例断言的正是"顶面顶点 y=9 只出现
    // 在朝向半格"。三段贯通：玩家面向 -Z（yaw=0）放置 → nibble facing=1
    // （game.rs::placement_tests::stairs_facing_follows_vanilla）→ 本用例
    // facing=1 分支 → 踏步在 -Z 半格（踏步在玩家面前，可拾级而上）。
    // bit2=top（点底面放置，StairBlock.java:104 DOWN→TOP）上下翻转。
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let stairs = id_of("oak_stairs");
    for facing in 0u16..4 {
        let mut c = chunk(0, 0xF0);
        put(&mut c.0, 8, 8, 8, stairs | (facing << 12));
        let buf = mesher.build(&full9(&c, &side), 0).unwrap();
        assert_eq!(buf.counts(), (48, 72), "facing {facing}: 两盒 12 面");
        for v in decode(&buf).iter().filter(|v| v.pos[1] == 9.0) {
            let in_half = match facing {
                0 => v.pos[2] >= 8.5,
                1 => v.pos[2] <= 8.5,
                2 => v.pos[0] >= 8.5,
                _ => v.pos[0] <= 8.5,
            };
            assert!(in_half, "facing {facing}: 踏步越出朝向半格 {:?}", v.pos);
        }
        put(&mut c.0, 8, 8, 8, 0);
    }
    // top 翻转：底座占上半，踏步半盒在下半的朝向侧（facing 0 = +Z）。
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, stairs | (4u16 << 12));
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    assert_eq!(buf.counts(), (48, 72));
    let verts = decode(&buf);
    assert!(
        verts.iter().any(|v| v.pos[1] == 8.0 && v.pos[2] >= 8.5),
        "翻转后踏步底面应留在 +Z 半格"
    );
    assert!(
        !verts.iter().any(|v| v.pos[1] == 8.0 && v.pos[2] < 8.5),
        "翻转后 y=8 平面不应有 -Z 半格顶点"
    );
}

#[test]
fn fence_post_and_arms() {
    // 单根：仅立柱 6 面；相邻同 id：两根各出立柱+相向臂；邻格异种不连接。
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let fence = id_of("oak_fence");

    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, fence);
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    assert_eq!(buf.counts(), (24, 36), "单根栅栏只有立柱");
    for v in decode(&buf) {
        assert!((8.375..=8.625).contains(&v.pos[0]), "立柱 x = {}", v.pos[0]);
        assert!((8.375..=8.625).contains(&v.pos[2]), "立柱 z = {}", v.pos[2]);
    }

    let mut c2 = chunk(0, 0xF0);
    put(&mut c2.0, 8, 8, 8, fence);
    put(&mut c2.0, 9, 8, 8, fence);
    let buf2 = mesher.build(&full9(&c2, &side), 0).unwrap();
    assert_eq!(buf2.counts(), (96, 144), "相邻两根各出立柱 + 臂");
    for v in decode(&buf2) {
        assert!((8.0..=10.0).contains(&v.pos[0]));
        assert!((8.0..=9.0).contains(&v.pos[1]));
        assert!((8.0..=9.0).contains(&v.pos[2]));
    }

    // 贴同系（云杉栅栏，同为木质 WOODEN_FENCES，FenceBlock.java:66-68
    // 原版也互连；我方以 shape==Fence 近似同类别）：与同 id 用例同几何。
    let spruce = id_of("spruce_fence");
    let mut c2b = chunk(0, 0xF0);
    put(&mut c2b.0, 8, 8, 8, fence);
    put(&mut c2b.0, 9, 8, 8, spruce);
    let buf2b = mesher.build(&full9(&c2b, &side), 0).unwrap();
    assert_eq!(buf2b.counts(), (96, 144), "跨木种同系互连：各出立柱 + 臂");

    // 贴火把（solid=false、非栅栏 → 非 sturdy 非同系）：不出臂，仅立柱。
    let torch = id_of("torch");
    let mut c2c = chunk(0, 0xF0);
    put(&mut c2c.0, 8, 8, 8, fence);
    put(&mut c2c.0, 9, 8, 8, torch);
    let buf2c = mesher.build(&full9(&c2c, &side), 0).unwrap();
    let only_post = decode(&buf2c)
        .iter()
        .filter(|v| v.tex == mcv_core::tiles::PLANKS)
        .count();
    assert_eq!(only_post, 24, "火把邻格非 sturdy：不出臂");

    // 贴石头：26.1 FenceBlock.java:59-63/91-94——sturdy 邻块（石头非
    // isExceptionForConnection 名单 Block.java:255-259）也触发连接臂。
    // 立柱 5 面（贴石 -X 剔除）+ 臂 5 面（臂贴石端 x=0 剔除）= 40 顶点。
    // （旧实现"仅同 id 即连"→ 无臂 20 顶点，是 CRITICAL 错误，已改。）
    let mut c3 = chunk(0, 0xF0);
    put(&mut c3.0, 8, 8, 8, fence);
    put(&mut c3.0, 7, 8, 8, 1); // 石头 = sturdy，应连臂
    let buf3 = mesher.build(&full9(&c3, &side), 0).unwrap();
    let fence_verts = decode(&buf3)
        .iter()
        .filter(|v| v.tex == mcv_core::tiles::PLANKS)
        .count();
    assert_eq!(fence_verts, 40, "石头邻格：出臂，臂贴石端与立柱贴石面剔除");
    // 臂应延伸到贴石格边界（world x=8.0）。
    assert!(
        decode(&buf3)
            .iter()
            .any(|v| v.tex == mcv_core::tiles::PLANKS && v.pos[0] == 8.0),
        "石头臂应触到格边界 x=8.0"
    );

    // 空气邻格不出臂（负向对照）：仅立柱 6 面 24。
    // （已在开头单根用例断言。）
}

#[test]
fn torch_thin_column() {
    // 火把：细立柱盒 x/z 0.4..0.6、y 0..0.625，6 面含顶面（y=8.625）。
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let torch = id_of("torch");
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, torch);
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    assert_eq!(buf.counts(), (24, 36), "细柱 6 面");
    let verts = decode(&buf);
    for v in &verts {
        assert!((8.4..=8.6).contains(&v.pos[0]), "柱 x = {}", v.pos[0]);
        assert!((8.0..=8.625).contains(&v.pos[1]), "柱 y = {}", v.pos[1]);
        assert!((8.4..=8.6).contains(&v.pos[2]), "柱 z = {}", v.pos[2]);
        let face = (v.flags & 7) as usize;
        assert_eq!(
            v.tex as usize,
            mcv_core::BLOCKS[torch as usize].tiles[face] as usize
        );
    }
    assert!(verts.iter().any(|v| v.pos[1] == 8.625), "顶面缺失");
}

#[test]
fn shape_faces_cull_against_opaque_neighbours() {
    // 形状面剔除与 Cube 同判据：下半砖放在整片石头地板上，-Y 面被剔除
    // → 5 面 20 顶点；y=0.5 中层面（+Y）仍然暴露。
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    let slab = id_of("oak_slab");
    let mut c = chunk(0, 0xF0);
    for x in 0..16 {
        for z in 0..16 {
            put(&mut c.0, x, 7, z, 1); // 石地板
        }
    }
    put(&mut c.0, 8, 8, 8, slab);
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    let verts = decode(&buf);
    let slab_verts: Vec<_> = verts
        .iter()
        .filter(|v| v.tex == mcv_core::tiles::PLANKS)
        .collect();
    assert_eq!(slab_verts.len(), 20, "石面上半砖 5 面（-Y 被剔除）");
    for v in &slab_verts {
        assert!((8.0..=8.5).contains(&v.pos[1]));
        assert!(
            v.pos[1] > 8.0 || (v.flags & 7) != 3,
            "不应有 -Y 面：{:?}",
            v.pos
        );
    }
    assert!(
        slab_verts
            .iter()
            .any(|v| v.pos[1] == 8.5 && (v.flags & 7) == 2),
        "中层面（+Y）仍暴露"
    );
}
