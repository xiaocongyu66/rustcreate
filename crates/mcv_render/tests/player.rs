//! 玩家网格纯 CPU 断言(无 GPU,任何 test job 可跑)。

use mcv_render::player_mesh::*;

#[test]
fn mesh_counts_and_uv_bounds() {
    let m = build_player_mesh();
    assert_eq!(
        m.verts.len(),
        SKIN_LAYERS as usize * PART_COUNT * PART_VERTS
    );
    assert_eq!(
        m.indices.len(),
        SKIN_LAYERS as usize * PART_COUNT * PART_INDEXES
    );
    for v in &m.verts {
        for c in v.pos {
            assert!(c.is_finite(), "NaN/inf pos {v:?}");
        }
        for c in v.uv {
            // unorm8 → 归一化后必在 0..=1(u8 天然满足,显式固化语义)
            let n = c as f32 / 255.0;
            assert!((0.0..=1.0).contains(&n));
        }
        assert!(v.meta[0] < SKIN_LAYERS && v.meta[1] < PART_COUNT as u32);
    }
    let max_i = *m.indices.iter().max().unwrap() as usize;
    assert_eq!(max_i, m.verts.len() - 1, "索引应恰好覆盖全部顶点");
    for s in 0..SKIN_LAYERS as usize {
        let mut prev = 0;
        for p in 0..PART_COUNT {
            let r = &m.slices[s][p];
            assert_eq!(r.len(), PART_INDEXES);
            assert_eq!(r.start, prev, "slice 应连续");
            prev = r.end;
        }
    }
}

#[test]
fn stride_matches_layout_assumption() {
    assert_eq!(PLAYER_STRIDE, std::mem::size_of::<PlayerVertex>());
    assert_eq!(PART_VERTS * PLAYER_STRIDE, 24 * 24);
}

#[test]
fn model_matrices_finite_under_extreme_pose() {
    let pose = PlayerPose {
        pos: glam::Vec3::new(0.5, 7.0, -125.25),
        yaw: 3.7,
        pitch: -1.55,
        phase: 5.9,
        amount: 0.88,
    };
    for mm in model_matrices(&pose) {
        for c in mm.to_cols_array() {
            assert!(c.is_finite());
        }
    }
}
