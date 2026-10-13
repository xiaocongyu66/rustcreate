//! 病灶#2 平滑光照层行为锁（#153 light-audit M9；26.1 原版默认 Smooth
//! Lighting ON，本仓此前从未移植）。
//!
//! 语义锚（`/root/mc-ref/src-26.1`）：
//! - 顶点四格均值 + 借中心规则：`LightCoordsUtil.smoothBlend`
//!   （LightCoordsUtil.java:61-89）——中心任一通道 >2 时零通道邻格借中心
//!   值（:62-86），四格按通道 floor 均值（:88 打包 `>>2 & 0x00FF00FF`，
//!   通道位不相交故与双通道逐通道 `>>2` 等价）；
//! - 立方面顶点 = 顶点邻接三格（暴露层内两侧格 + 对角格）+ 面中心格
//!   （面平采格）的 blend：`BlockModelLighter.prepareQuadShape` 立方面
//!   分支（BlockModelLighter.java:163-178，`lightCenter` 取
//!   `centerPosition.relative(direction)` :115-120）；
//! - `MCV_SMOOTH_LIGHT=0` 回落每面单采（M8 平采语义 = vanilla Smooth
//!   Lighting OFF，改动前输出逐位等价），缺省 = 开（原版默认语义）。
//!
//! 本文件刻意只放一个 `#[test]`：环境变量是进程级全局，平滑/回落两分支
//! 必须在同一测试内串行切换（edition 2024 env 变更为 unsafe）。

use mcv_core::{CHUNK_VOL, vidx};
use mcv_mesher::{MESH_OPAQUE, Mesher, Slot};

const VOL: usize = CHUNK_VOL;

fn id_of(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("方块 {name} 未注册")) as u16
}

/// 顶点解码（24 B 布局见 mesher.rs 头注：pos f32x3 | uv u16x2 | tex u16 |
/// block_light u8@18 | sky_light u8@19 | ao u8 | flags u8）。
struct V {
    pos: [f32; 3],
    blk: u8,
    sky: u8,
    flags: u8,
}

fn decode(buf: &mcv_mesher::MeshData) -> Vec<V> {
    buf.vertex_data()
        .chunks(mcv_mesher::VERTEX_STRIDE)
        .map(|b| V {
            pos: [
                f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                f32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                f32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            ],
            blk: b[18],
            sky: b[19],
            flags: b[21],
        })
        .collect()
}

/// 场景锁：悬空石头顶面的四顶点平滑光 = 暴露层四格均值（借中心规则），
/// 且 `MCV_SMOOTH_LIGHT=0` 回落每面单采。
#[test]
fn smooth_top_face_vertices_are_four_cell_mean_with_borrow() {
    let mesher = Mesher::new();
    let stone = id_of("stone");
    // 中心块：单块悬空石头 (8,8,8)；9 slot 全加载，side = 空气块（光 0xF0）。
    let side_vox = Box::new([0u16; VOL]);
    let side_light = Box::new([0xF0u8; VOL]);
    let mut vox = Box::new([0u16; VOL]);
    let mut light = Box::new([0xF0u8; VOL]);
    vox[vidx(8, 8, 8)] = stone;
    // 暴露层（y=9 平面）手工光照：中心 15、两侧 12/8/4、一侧与全部对角 0
    // ——借中心规则把 0 格抬到中心值 15 再均值（LightCoordsUtil.java:
    // 62-86）。块光 nibble 恒 0。
    for (dx, dz, sky) in [
        (0i32, 0i32, 15u8),
        (-1, 0, 12),
        (1, 0, 8),
        (0, -1, 4),
        (0, 1, 0),
        (-1, -1, 0),
        (1, -1, 0),
        (-1, 1, 0),
        (1, 1, 0),
    ] {
        let (x, z) = ((8 + dx) as usize, (8 + dz) as usize);
        light[vidx(x, 9, z)] = sky << 4;
    }
    let slots: [Option<Slot<'_>>; 9] = {
        let mut s: [Option<Slot<'_>>; 9] = [None; 9];
        for (i, slot) in s.iter_mut().enumerate() {
            let (v, l) = if i == 4 {
                (&vox[..], &light[..])
            } else {
                (&side_vox[..], &side_light[..])
            };
            *slot = Some(Slot {
                voxels: v,
                light: l,
            });
        }
        s
    };

    // 平滑路径（默认开）：顶面（flags 面档 2、平面 y=9）四顶点 sky =
    // [11, 14, 13, 10]（顶点发射序 v0..v3 = 角 (0,0),(0,1),(1,1),(1,0)，
    // u 轴=x、v 轴=z；四格 = 两侧 + 对角 + 中心，零通道借中心 15）：
    //   v0: 12 + 4 + 0→15 + 15 = 46>>2 = 11
    //   v1: 12 + 0→15 + 0→15 + 15 = 57>>2 = 14
    //   v2: 8 + 0→15 + 0→15 + 15 = 53>>2 = 13
    //   v3: 8 + 4 + 0→15 + 15 = 42>>2 = 10
    let buf = mesher.build(&slots, MESH_OPAQUE).expect("smooth build");
    let mut top: Vec<(f32, f32, u8, u8)> = decode(&buf)
        .iter()
        .filter(|v| v.flags & 7 == 2 && v.pos[1] == 9.0)
        .map(|v| (v.pos[0], v.pos[2], v.blk, v.sky))
        .collect();
    top.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap());
    assert_eq!(top.len(), 4, "顶面应恰 4 顶点");
    for (_, _, blk, _) in &top {
        assert_eq!(*blk, 0, "块光通道均值应恒 0");
    }
    // 角→期望（按 (x,z) 排序后：u=x 0..1、v=z 0..1）：
    // (8,8)=(0,0)→11 (8,9)=(0,1)→14 (9,9)=(1,1)→13 (9,8)=(1,0)→10。
    let expect = |x: f32, z: f32| -> u8 {
        if x == 8.0 && z == 8.0 {
            11
        } else if x == 8.0 && z == 9.0 {
            14
        } else if x == 9.0 && z == 9.0 {
            13
        } else {
            10
        }
    };
    for (x, z, _, sky) in &top {
        assert_eq!(*sky, expect(*x, *z), "顶面顶点 ({x},{z}) 平滑光");
    }

    // 回落路径：MCV_SMOOTH_LIGHT=0 → 每面单采（M8 平采语义），四顶点全
    // = 面平采格 15，且输出与平滑路径逐位不同（开关确实生效）。
    unsafe { std::env::set_var("MCV_SMOOTH_LIGHT", "0") };
    let flat = mesher.build(&slots, MESH_OPAQUE).expect("flat build");
    unsafe { std::env::remove_var("MCV_SMOOTH_LIGHT") };
    let flat_top: Vec<(f32, f32, u8, u8)> = decode(&flat)
        .iter()
        .filter(|v| v.flags & 7 == 2 && v.pos[1] == 9.0)
        .map(|v| (v.pos[0], v.pos[2], v.blk, v.sky))
        .collect();
    assert_eq!(flat_top.len(), 4);
    for (x, z, blk, sky) in &flat_top {
        assert_eq!(*sky, 15, "回落路径顶点 ({x},{z}) 应采面平采格 15");
        assert_eq!(*blk, 0);
    }
    assert_ne!(
        flat.vertex_data(),
        buf.vertex_data(),
        "回落输出必须与平滑输出不同（开关有效性）"
    );
}
