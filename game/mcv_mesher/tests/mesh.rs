//! Synthetic-data tests for the C++ greedy mesher via `Mesher`.
//! All light arrays are filled with 0xF0 (sky 15, block 0).

use mcv_mesher::{MeshData, Mesher, Slot};

// v6：384 高绝对 y 域，索引走 mcv_core::vidx（y 为绝对世界 y）。
const VOL: usize = mcv_core::CHUNK_VOL;

type Chunk = (Box<[u16; VOL]>, Box<[u8; VOL]>);

fn chunk(id: u16, light: u8) -> Chunk {
    (Box::new([id; VOL]), Box::new([light; VOL]))
}

fn put(vox: &mut [u16; VOL], x: usize, y: usize, z: usize, id: u16) {
    vox[mcv_core::vidx(x, y as i32, z)] = id;
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
    tint: [u8; 3],
}

#[allow(clippy::almost_complete_range)]
#[allow(unknown_lints, clippy::manual_chunks)]
fn decode(buf: &MeshData) -> Vec<Vtx> {
    buf.vertex_data()
        .chunks(mcv_mesher::VERTEX_STRIDE)
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
                tint: [b[24], b[25], b[26]],
            }
        })
        .collect()
}

/// UV 单位回归（2026-10-10 真机事故两轮）：一个方块面横跨一整 tile =
/// 4096 单位（shader 除 4096 得 tile 数）。第一代 bug：kUvPerBlock=
/// 65535/16 每面只采 1 纹素=纯色；第二代 bug：65535/块使 ≥2 块合并面
/// u16 溢出环绕=拉伸。原版烘焙：每面吃满 sprite 满幅（FaceBakery.java:26-35/166）。
#[test]
fn uv_spans_full_tile_per_block() {
    let mesher = Mesher::new();
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
                4096,
                "quad {qi} uv axis {axis}: single block must span one tile (4096), got {lo}..{hi}"
            );
        }
    }
}

/// 贪心合并面 uv：2 方块宽面跨度 = 8192（2 tile），**无环绕**——
/// 顶点 uv 线性插值下 mod-wrap 会折叠跨度、一 tile 拉伸铺满合并面
/// （真机 round 2「马赛克拉伸」签名）。
#[test]
fn greedy_merged_face_uv_spans_multiple_tiles() {
    let mesher = Mesher::new();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, 3);
    put(&mut c.0, 9, 8, 8, 3); // 2-block strip merges along x
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();

    let merged = decode(&buf).chunks(4).any(|q| {
        (0..2).any(|axis| {
            let lo = q.iter().map(|v| v.uv[axis] as i32).min().unwrap();
            let hi = q.iter().map(|v| v.uv[axis] as i32).max().unwrap();
            hi - lo == 8192
        })
    });
    assert!(
        merged,
        "2-block merged face must carry span 8192 (2 tiles, no u16 wrap)"
    );
}

/// 侧面 v 方向：贴图 row 0（PNG 顶行=草裙）无翻转上传后落在采样
/// coord.y=0，所以顶角顶点（y 大）uv.v=0、底角 uv.v=4096。此前 v 随 y
/// 增，真机草裙全部倒挂（round 2）。数据走真 mesher，非手写 quad。
#[test]
fn side_face_uv_v0_at_top_edge() {
    let mesher = Mesher::new();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, 3);
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();

    let mut sides = 0;
    for (qi, quad) in decode(&buf).chunks(4).enumerate() {
        let ys: Vec<f32> = quad.iter().map(|v| v.pos[1]).collect();
        let ymin = ys.iter().cloned().fold(f32::INFINITY, f32::min);
        let ymax = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        if (ymax - ymin) < 0.5 {
            continue; // top/bottom face: constant y
        }
        sides += 1;
        for v in quad {
            let want = if v.pos[1] > ymin + 0.25 { 0i32 } else { 4096 };
            assert_eq!(
                v.uv[1] as i32, want,
                "side quad {qi}: vertex y={} must have v={want} (v=0 at top), got {}",
                v.pos[1], v.uv[1]
            );
        }
    }
    assert_eq!(
        sides, 8,
        "one floating block = 4 base + 4 overlay side quads (overlays 334)"
    );
}

#[test]
fn single_block_six_faces() {
    let mesher = Mesher::new();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, 3); // grass block floating in air

    // 6 base 面条 + 4 侧 tintindex 插层（overlays [334,334,0,0,334,334]，
    // 顶/底无插层）：26.1 grass_block 双元素模型逐元素烘焙双 quad。
    let def = &mcv_core::BLOCKS[3];
    let overlay_faces: Vec<usize> = (0..6).filter(|&f| def.overlays[f] != 0).collect();
    let extra = overlay_faces.len();
    assert_eq!(extra, 4, "grass 行应四侧带插层数据");
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    assert_eq!(buf.counts(), (24 + 4 * extra as u32, 36 + 6 * extra as u32));

    let verts = decode(&buf);
    let mut base_geom: [[[f32; 3]; 4]; 6] = [[[0.0; 3]; 4]; 6]; // base 逐面顶点 pos
    let mut n_base = 0usize;
    let mut n_ovl = 0usize;
    // 分档按 tex 不锁位序：插层 quad **紧随其 base 面**发射（emit_quad
    // 单函数内 base→overlay 连发，draw 序 winner 语义所需），quad 位序为
    // base0/ovl0/base1/ovl1/.../base5 交错——旧写法「qi==face 且 0..5 全
    // base 段」是插层发射前的位置约定（CI 38843334897 红：vertex 1
    // grouped into face 0）。分组的本质不变式=4 顶点/quad + 面档 flags +
    // 逐面 tex 对位，全数保留。
    for (qi, quad) in verts.chunks(4).enumerate() {
        let face = (quad[0].flags & 7) as usize;
        if quad[0].tex == def.tiles[face] {
            // base quad：tex = 生成表 base 列；记录几何供 overlay 对位。
            for v in quad {
                assert_eq!(
                    v.tex as usize, def.tiles[face] as usize,
                    "quad {qi} face {face} base tex"
                );
            }
            for (k, v) in quad.iter().enumerate() {
                base_geom[face][k] = v.pos;
            }
            n_base += 1;
        } else {
            // 插层 quad：同面档、tex = 生成表 overlay 列、**同位共面**（顶点
            // 坐标逐位复刻 base 同面——深度 LEQUAL + 后绘制 winner 语义，
            // 与 26.1 同平面双元素模型一致）。
            assert!(
                overlay_faces.contains(&face),
                "overlay 仅应有侧面档，got face {face}"
            );
            for (k, v) in quad.iter().enumerate() {
                assert_eq!(
                    v.tex, def.overlays[face],
                    "quad {qi} face {face} overlay tex"
                );
                assert_eq!(v.pos, base_geom[face][k], "overlay 同位复刻 base 面几何");
            }
            n_ovl += 1;
        }
    }
    assert_eq!(n_base, 6, "六条 base 面条");
    assert_eq!(n_ovl, extra, "插层条数 = 非零 overlay 档数");
    // index pattern: per quad (0,1,2),(0,2,3)
    for (t, &ix) in buf.indices().iter().enumerate() {
        let expected = (t / 6 * 4) as u32 + [0u32, 1, 2, 0, 2, 3][t % 6];
        assert_eq!(ix, expected, "index {t}");
    }
}

#[test]
fn block_table_tex_parity_with_mcv_core() {
    // 生成表黄金件（tests/golden/blocks_gen.inc，删除 cpp/ 前的快照）与
    // mcv_core::BLOCKS 必须逐 id 一致：
    // 对每个 id 悬空放一块，不透明 pass 按 shape 模板出几何、逐面 tile 层
    // 相同。水走水 pass 除外。全空气环境下各形状顶点数：Cube/单根 Fence/
    // 单块 Slab/Torch 细柱 = 6 面；Cross = 2 条双面 quad；Stairs = 两盒
    // 12 面（不做盒间剔除，宁多勿漏）。
    let mesher = Mesher::new();
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
        // 全空气下插层 quad 与 base 面同盯同剔：6 面全暴露 → 非零 overlay
        // 档（生成表允许 kind==0 立方的面）各发一条 4 顶点/6 索引的第二层
        // quad（当前数据全表仅 grass 行——kind==1 模板的 overlay 恒 0）。
        if INVISIBLE.contains(&def.name) {
            assert_eq!(buf.counts(), (0, 0), "block {id} face count");
        } else {
            let base = match mcv_core::shape::Shape::from_u8(def.shape) {
                mcv_core::shape::Shape::Cross => (8, 24),
                mcv_core::shape::Shape::Stairs => (48, 72),
                _ => (24, 36),
            };
            let extra = def.overlays.iter().filter(|&&o| o != 0).count() as u32;
            let expected = (base.0 + 4 * extra, base.1 + 6 * extra);
            assert_eq!(buf.counts(), expected, "block {id} face count");
        }
        for v in decode(&buf) {
            let face = (v.flags & 7) as usize;
            assert!(
                v.tex == def.tiles[face]
                    || (def.overlays[face] != 0 && v.tex == def.overlays[face]),
                "block {id} face {face} tex layer"
            );
        }
        put(&mut c.0, 8, 8, 8, 0);
    }
}

#[test]
fn greedy_merges_row_of_four() {
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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
    let mesher = Mesher::new();
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

/// 129-L/L0（129-C §5 首要根因复现）：贴实心的水面对照原版两条判据——
/// FluidRenderer.java:290 `renderCondition && !isFaceOccludedByNeighbor(...)`
/// 被满块遮蔽形邻域遮住的流体面不发射（isFaceOccludedByState :36-47：满块
/// 遮蔽形下除 +Y 外全遮，+Y 仅流体高度满 1.0 才遮）；:430-432 流体面取光 =
/// max(自身格, 上格)。修复前：贴石底面照发且取光采对面石格（实心内恒 0）
/// →「黑水皮」与亮沙顶面共面，LessEqual 深度 tie 逐帧翻转（水下黑影闭环）。
#[test]
fn water_faces_against_floor_culled_and_lit_per_vanilla() {
    // 海洋切片：y<=9 石海床，y=10..=12 水（手工光照 sky 12,11,10——
    // init 水柱序列由 light.rs:640 锁定，此处直填），y=13 空气 15。
    let ocean = || -> Chunk {
        let stone = id_of("stone");
        let water = id_of("water");
        let mut vox = Box::new([0u16; VOL]);
        let mut light = Box::new([0u8; VOL]);
        for x in 0..16usize {
            for z in 0..16usize {
                for y in 0..=9i32 {
                    vox[mcv_core::vidx(x, y, z)] = stone;
                }
                for y in 10..=12i32 {
                    vox[mcv_core::vidx(x, y, z)] = water;
                    light[mcv_core::vidx(x, y, z)] = ((12 - (y - 10)) << 4) as u8;
                }
                light[mcv_core::vidx(x, 13, z)] = 15 << 4;
            }
        }
        (vox, light)
    };
    let mesher = Mesher::new();
    let center = ocean();
    let side = ocean(); // 8 邻同构：跨界侧面水对水剔除，只剩顶/底候选
    let buf = mesher
        .build(&full9(&center, &side), mcv_mesher::MESH_WATER)
        .unwrap();
    let verts = decode(&buf);
    assert!(!verts.is_empty(), "水 pass 必须仍有几何（顶面在）");
    for (i, v) in verts.iter().enumerate() {
        let face = v.flags & 7;
        // (a) FluidRenderer.java:290：贴满块（海床）的底面不发射。
        assert!(
            face != 3 || v.pos[1] != 10.0,
            "顶点 {i}: 水底面贴海床仍发射且共面（:290 应遮蔽）"
        );
        // (b) :430-432 取光 max(自身,上)：任何发射面 sky 不得为 0
        //（黑水皮复现值；正午浅水渲染亮度 ≈0.18，见 129-C §6）。
        assert!(v.sky > 0, "顶点 {i}: 水面对实心邻域取到 0 光 → 黑水皮");
    }
    // 顶面保留（原版 0.8889 高度不遮 +Y）。
    assert!(
        verts.iter().any(|v| v.flags & 7 == 2),
        "水顶面应保留（水体可见性回归门 water_pass_emits_vertices 同款）"
    );
}

/// 129-L/L1 判据钉：流体面取光采「自身格+上格取 max」（FluidRenderer.java
/// :168 顶面 / :234 侧面——sideLightCoords 在水平循环外一次采样、:204 底面
/// getLightCoords(pos.below())=max(下格,自身)、:430-432 定义），**不采水平
/// 邻格**。暗邻格（玻璃后暗室/暗玻璃本身）不得再染黑亮水格的面。
#[test]
fn water_face_light_takes_own_and_above_not_dark_neighbour() {
    let mesher = Mesher::new();
    let water = id_of("water");
    let glass = id_of("glass");
    let side = chunk(0, 0xF0); // 邻区块全亮空气

    // 场景 A：暗玻璃贴侧。玻璃非满块遮蔽形（原版 occluder 空）→ +X 面
    // 仍发射；取光应 max(自身 10, 上格 15)=15，而非玻璃格 0。
    let mut c = chunk(0, 0);
    put(&mut c.0, 8, 8, 8, water);
    put(&mut c.0, 9, 8, 8, glass);
    c.1[mcv_core::vidx(8, 8, 8)] = 10 << 4;
    c.1[mcv_core::vidx(8, 9, 8)] = 15 << 4;
    let buf = mesher
        .build(&full9(&c, &side), mcv_mesher::MESH_WATER)
        .unwrap();
    let verts = decode(&buf);
    assert!(
        verts.iter().any(|v| v.flags & 7 == 0),
        "+X 面应发射（玻璃非遮蔽体，:38-39 空遮蔽形不挡）"
    );
    for v in &verts {
        let face = v.flags & 7;
        if face == 3 {
            // 底面 = max(下格 0, 自身 10)：场景 A 非其判据主体，只须非 0。
            assert!(v.sky >= 10, "底面取光不得为 0");
        } else {
            // 顶/侧面 = max(自身 10, 上格 15) = 15（暗邻格不得入样）。
            assert_eq!(v.sky, 15, "面档 {face}: 取光应为 max(自身,上)=15");
        }
    }

    // 场景 B：暗玻璃贴底。底面取光 = getLightCoords(pos.below())
    //（:204）= max(下格 0, 下格上方=自身 10) = 10。
    let mut c2 = chunk(0, 0);
    put(&mut c2.0, 8, 8, 8, water);
    put(&mut c2.0, 8, 7, 8, glass);
    c2.1[mcv_core::vidx(8, 8, 8)] = 10 << 4;
    c2.1[mcv_core::vidx(8, 9, 8)] = 15 << 4;
    let buf2 = mesher
        .build(&full9(&c2, &side), mcv_mesher::MESH_WATER)
        .unwrap();
    let bottom: Vec<_> = decode(&buf2)
        .into_iter()
        .filter(|v| v.flags & 7 == 3 && v.pos[1] == 8.0)
        .collect();
    assert!(!bottom.is_empty(), "贴玻璃底面应发射（玻璃非遮蔽体）");
    for v in &bottom {
        assert_eq!(v.sky, 10, "底面取光应为 max(下格,自身)=10");
    }
}

/// 水 pass 回归（2026-10-10 真机「水不可见」审计 H1 配套）：水体必须产出
/// 非空顶点+索引（游戏层曾只传索引丢弃顶点，水从未上传 GPU）。2×2×2 水
/// 浮在空气中：每个水面 quad 都要出（贪心合并后 ≤24），顶点必须 >0。
#[test]
fn water_pass_emits_vertices() {
    let mesher = Mesher::new();
    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    let water = id_of("water");
    for x in 8..10 {
        for y in 8..10 {
            for z in 8..10 {
                put(&mut c.0, x, y, z, water);
            }
        }
    }
    let buf = mesher
        .build(&full9(&c, &side), mcv_mesher::MESH_WATER)
        .unwrap();
    let (nv, ni) = buf.counts();
    assert!(
        nv > 0 && ni > 0,
        "water pass must emit geometry, got ({nv},{ni})"
    );
    assert_eq!(
        ni,
        nv / 4 * 6,
        "water indices must be quads of the emitted verts"
    );
}

// ---------------------------------------------------------------------------
// M 波 #138：形状模板全表（template_boxes 单一数据源；6..9 真模板 +
// 10..15 新档替换全盒占位）。刀2 红：全盒占位下薄盒断言必红。
// ---------------------------------------------------------------------------

/// 薄盒模板逐形状锚：孤立单块 6 面 (24,36)，顶点全部落在原版模板盒内。
#[test]
fn m_wave_thin_templates_replace_full_box_placeholders() {
    let mesher = Mesher::new();
    let side = chunk(0, 0xF0);
    let cases: &[(&str, u16, [f32; 3], [f32; 3])] = &[
        // 地毯：CarpetBlock.java:17 column(16,0,1)。
        ("red_carpet", 0, [8.0, 8.0, 8.0], [9.0, 8.0625, 9.0]),
        // 活板门：关下 3px / 关上 y 13..16px / 开（facing=-Z → 竖板贴 +Z 边）。
        ("oak_trapdoor", 0, [8.0, 8.0, 8.0], [9.0, 8.1875, 9.0]),
        ("oak_trapdoor", 4, [8.0, 8.8125, 8.0], [9.0, 9.0, 9.0]),
        ("oak_trapdoor", 0b1001, [8.0, 8.0, 8.8125], [9.0, 9.0, 9.0]),
        // 柵栏门：±Z 朝向 → 4px 板横跨 X；±X 朝向 → 板横跨 Z。
        ("oak_fence_gate", 0, [8.0, 8.0, 8.375], [9.0, 9.0, 8.625]),
        ("oak_fence_gate", 2, [8.375, 8.0, 8.0], [8.625, 9.0, 9.0]),
        // 门：关门 3px 板贴 FACING 对侧边（0=+Z → -Z 边）。
        ("oak_door", 0, [8.0, 8.0, 8.0], [9.0, 9.0, 8.1875]),
        // 梯子：3px 板贴 FACING 对侧边（2=+X → -X 边）。
        ("ladder", 2, [8.0, 8.0, 8.0], [8.1875, 9.0, 9.0]),
        // 按钮：地面 6×2×4px 小盒。
        (
            "oak_button",
            0,
            [8.3125, 8.0, 8.375],
            [8.6875, 8.125, 8.625],
        ),
        // 压力板：14×1×14px。
        (
            "stone_pressure_plate",
            0,
            [8.0625, 8.0, 8.0625],
            [8.9375, 8.0625, 8.9375],
        ),
        // 雪层：1 层 2px / 7 层 14px。
        ("snow", 0, [8.0, 8.0, 8.0], [9.0, 8.125, 9.0]),
        ("snow", 6, [8.0, 8.0, 8.0], [9.0, 8.875, 9.0]),
    ];
    for (name, state, lo, hi) in cases {
        let id = id_of(name);
        let mut c = chunk(0, 0xF0);
        put(&mut c.0, 8, 8, 8, id | (state << 12));
        let buf = mesher.build(&full9(&c, &side), 0).unwrap();
        assert_eq!(
            buf.counts(),
            (24, 36),
            "{name} state {state}: 单盒模板 6 面"
        );
        for v in decode(&buf) {
            for axis in 0..3 {
                assert!(
                    (lo[axis]..=hi[axis]).contains(&v.pos[axis]),
                    "{name} state {state}: 顶点 {:?} 越出模板盒 {lo:?}..{hi:?}（全盒占位未换模板？）",
                    v.pos
                );
            }
        }
    }
}

/// 玻璃板/墙：孤立仅柱（x/z 7..9 / 4..12px）；同族相邻出臂、臂触格边。
#[test]
fn pane_and_wall_post_with_connected_arms() {
    let mesher = Mesher::new();
    let side = chunk(0, 0xF0);
    let pane = id_of("glass_pane");
    let wall = id_of("cobblestone_wall");

    // 板：孤立柱。
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, pane);
    let buf = mesher.build(&full9(&c, &side), 0).unwrap();
    assert_eq!(buf.counts(), (24, 36), "孤立板仅柱 6 面");
    for v in decode(&buf) {
        assert!((8.4375..=8.5625).contains(&v.pos[0]), "柱 x = {}", v.pos[0]);
        assert!((8.4375..=8.5625).contains(&v.pos[2]), "柱 z = {}", v.pos[2]);
        assert!((8.0..=9.0).contains(&v.pos[1]));
    }
    // 板：+X 同族 → 柱+臂，臂触格边 x=9.0。
    let mut c2 = chunk(0, 0xF0);
    put(&mut c2.0, 8, 8, 8, pane);
    put(&mut c2.0, 9, 8, 8, pane);
    let buf2 = mesher.build(&full9(&c2, &side), 0).unwrap();
    assert_eq!(buf2.counts(), (96, 144), "两板各柱+臂（盒间共面不剔）");
    assert!(
        decode(&buf2).iter().any(|v| v.pos[0] == 9.0),
        "板臂应触格边 x=9.0"
    );

    // 墙：孤立柱 x/z 4..12px。
    let mut c3 = chunk(0, 0xF0);
    put(&mut c3.0, 8, 8, 8, wall);
    let buf3 = mesher.build(&full9(&c3, &side), 0).unwrap();
    assert_eq!(buf3.counts(), (24, 36), "孤立墙仅柱 6 面");
    for v in decode(&buf3) {
        assert!((8.25..=8.75).contains(&v.pos[0]), "墙柱 x = {}", v.pos[0]);
    }
    // 墙：+X 同族 → 柱+低臂（outline 14px），臂自 +X 格边伸入 11px。
    let mut c4 = chunk(0, 0xF0);
    put(&mut c4.0, 8, 8, 8, wall);
    put(&mut c4.0, 9, 8, 8, wall);
    let buf4 = mesher.build(&full9(&c4, &side), 0).unwrap();
    assert_eq!(buf4.counts(), (96, 144), "两墙各柱+臂");
    let verts = decode(&buf4);
    assert!(verts.iter().any(|v| v.pos[0] == 9.0), "墙臂应触格边 x=9.0");
    assert!(
        verts.iter().any(|v| v.pos[0] == 9.0 && v.pos[1] == 8.875),
        "墙臂 outline 高 14px（0.875）"
    );
}

/// #190 深层带行界连续锁（新）：整区块截面实心柱的侧面沿世界 y 每个
/// 行界必须有顶点（原版逐面 quad 语义——纵向禁合并，mesher.rs build_pass
/// #190 注）。负 y 域（y=-64..=-50，顶面 y=-49）与正 y 域（y=70..=84，
/// 顶面 y=85）同款断言 [y0, y1+1] 全整数顶点：若「世界 y 向贪心合并」
/// 回归，负 y 集先丢 -62..=-50（deep_section_below_zero_is_meshed 的
/// #184 红签名 = 真机「挖到某层下方变透明」的网格面），golden bounds
/// 场景同锁。正 y 对照此前无任何行界用例覆盖（golden 全正 y + bounds
/// 单列），本测双域补钉。
#[test]
fn side_faces_keep_every_row_boundary_y() {
    let stone = id_of("stone");
    for (y0, y1) in [(-64i32, -50i32), (70, 84)] {
        let side = chunk(0, 0xF0);
        let mut c = chunk(0, 0xF0);
        for y in y0..=y1 {
            for z in 0..16usize {
                for x in 0..16usize {
                    c.0[mcv_core::vidx(x, y, z)] = stone;
                }
            }
        }
        let buf = Mesher::new().build(&full9(&c, &side), 0).unwrap();
        let ys: Vec<i32> = decode(&buf).iter().map(|v| v.pos[1] as i32).collect();
        for want in y0..=y1 + 1 {
            assert!(
                ys.contains(&want),
                "y∈[{y0},{y1}] 实心柱缺行界顶点 y={want}（世界 y 向合并回归，\
                 负 y 域=深层透明波签名）：ys 集 {:?}",
                {
                    let mut u: Vec<i32> = ys.clone();
                    u.sort_unstable();
                    u.dedup();
                    u
                }
            );
        }
    }
}

/// tintindex 插层面的群系染色（26.1 面档 tint 消费，#191 顶点色通路的
/// 草裙侧面接线）：grass 悬空块全暴露，带 plains 气候 tint 场逐顶点断言
/// ——base 侧面（grass_block_side，模型无 tintindex）恒白；草顶
/// （grass_block_top）与四侧插层（grass_block_side_overlay）按世界列乘
/// grass colormap（ModelBlockRenderer.java:212-214 按 quad tintIndex 查
/// BlockColors；基础侧脸首元素无 tintindex 不染）。
#[test]
fn grass_overlay_faces_carry_biome_tint() {
    let colormap = |name: &str| {
        mcv_core::tint::decode_colormap(
            &std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                    "../../assets/minecraft/textures/colormap/{name}.png"
                )),
            )
            .expect("colormap 素材应在仓库内"),
        )
        .expect("colormap 应为 256x256")
    };
    let grass_png = colormap("grass");
    let foliage_png = colormap("foliage");
    let ct = mcv_core::tint::ChunkTint::from_climate(0, 0, &[[0.8, 0.4]], &grass_png, &foliage_png)
        .expect("1x1 气候场");
    // plains（temp 0.8 / rain 0.4）= #91BD59（colormap_formula 先例同源值）。
    let expect = ct.color_at(mcv_core::tint::TINT_GRASS, 8, 8);
    assert_eq!(expect, [0x91, 0xBD, 0x59], "单列场盒均值 = plains 草色");

    let side = chunk(0, 0xF0);
    let mut c = chunk(0, 0xF0);
    put(&mut c.0, 8, 8, 8, 3);
    let def = &mcv_core::BLOCKS[3];
    let buf = Mesher::new()
        .build_with_tint(
            &full9(&c, &side),
            0,
            Some(mcv_mesher::TintInput {
                block_x: 0,
                block_z: 0,
                tint: ct,
            }),
        )
        .unwrap();

    let mut n_base_side = 0;
    for (qi, quad) in decode(&buf).chunks(4).enumerate() {
        let face = (quad[0].flags & 7) as usize;
        if def.overlays[face] != 0 && quad[0].tex == def.overlays[face] {
            // 插层 quad：tex = grass_block_side_overlay → grass 族 → 群系色。
            for v in quad {
                assert_eq!(v.tint, expect, "quad {qi} face {face}: 插层顶点应带群系色");
            }
        } else if face == 2 && quad[0].tex == def.tiles[2] {
            // 草顶 quad：grass_block_top → grass 族 → 群系色。
            for v in quad {
                assert_eq!(v.tint, expect, "quad {qi}: 草顶顶点应带群系色");
            }
        } else {
            // base 侧面（grass_block_side 无 tintindex）/底面：恒白。
            n_base_side += usize::from(face != 3 && face != 2);
            for v in quad {
                assert_eq!(
                    v.tint,
                    mcv_core::tint::TINT_WHITE,
                    "quad {qi} face {face}: 无 tintindex 面恒白"
                );
            }
        }
    }
    assert_eq!(n_base_side, 4, "四条 base 侧 quad 应全数在场");
}
