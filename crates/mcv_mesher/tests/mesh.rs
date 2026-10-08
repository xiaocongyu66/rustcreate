//! Synthetic-data tests for the C++ greedy mesher via `Mesher`.
//! All light arrays are filled with 0xF0 (sky 15, block 0).

use mcv_mesher::{CxxMeshBuffer, Mesher, Slot};

const VOL: usize = 65536;

type Chunk = (Box<[u8; VOL]>, Box<[u8; VOL]>);

fn chunk(id: u8, light: u8) -> Chunk {
    (Box::new([id; VOL]), Box::new([light; VOL]))
}

fn put(vox: &mut [u8; VOL], x: usize, y: usize, z: usize, id: u8) {
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

struct Vtx {
    pos: [f32; 3],
    tex: u16,
    ao: u8,
    flags: u8,
}

fn decode(buf: &CxxMeshBuffer) -> Vec<Vtx> {
    buf.vertex_data()
        .chunks_exact(24)
        .map(|b| {
            let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
            Vtx {
                pos: [f(0), f(4), f(8)],
                tex: u16::from_le_bytes([b[16], b[17]]),
                ao: b[20],
                flags: b[21],
            }
        })
        .collect()
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
    let mesher = Mesher::new(1 << 20).unwrap();
    let side = chunk(0, 0xF0);
    for id in 1u8..14 {
        // water belongs to the water pass; flowers emit no geometry
        if id == 5 || id == 12 || id == 13 {
            continue;
        }
        let mut c = chunk(0, 0xF0);
        put(&mut c.0, 8, 8, 8, id);
        let buf = mesher.build(&full9(&c, &side), 0).unwrap();
        assert_eq!(buf.counts(), (24, 36), "block {id} face count");
        for v in decode(&buf) {
            let face = (v.flags & 7) as usize;
            assert_eq!(
                v.tex as usize,
                mcv_core::BLOCKS[id as usize].tiles[face] as usize,
                "block {id} face {face} tex layer"
            );
        }
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
    // stone top face (+Y, tex layer 4): one merged quad -> 4 vertices
    let top: Vec<_> = verts
        .iter()
        .filter(|v| v.flags & 7 == 2 && v.tex == 4)
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
