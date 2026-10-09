//! 生物模型纯 CPU 断言（无 GPU）：部位表对齐 Java Model 类数字、UV 在
//! 纹理内、跨种类切片连续。

use mcv_render::mob_mesh::*;

#[test]
fn mesh_counts_and_uv_bounds() {
    let m = build_mob_mesh();
    // 盒数：鸡 8 + 牛 10 + 羊 12 + 猪 7 = 37（部位表见 mob_mesh.rs）
    assert_eq!(m.verts.len(), 37 * 24);
    assert_eq!(m.indices.len(), 37 * 36);
    for v in &m.verts {
        for c in v.pos {
            assert!(c.is_finite());
        }
        for c in v.uv {
            let n = c as f32 / 255.0;
            assert!((0.0..=1.0).contains(&n));
        }
        // meta[0] = 贴图层（<5）；meta[1] = 跨四种生物的全局部位序
        //（8+6+12+6 = 32，非单体 MAX_MOB_PARTS）。
        assert!(v.meta[0] < 5 && v.meta[1] < 8 + 6 + 12 + 6);
    }
    let max_i = *m.indices.iter().max().unwrap() as usize;
    assert_eq!(max_i, m.verts.len() - 1);
}

#[test]
fn kind_slices_are_contiguous() {
    let m = build_mob_mesh();
    let mut prev = 0;
    for (k, r) in m.slices.iter().enumerate() {
        assert_eq!(r.start, prev, "kind {k} 切片应连续");
        assert_eq!(r.len() % 36, 0);
        prev = r.end;
    }
}

#[test]
fn part_counts_match_table() {
    assert_eq!(MOB_PART_COUNTS, [8, 6, 12, 6]);
    assert_eq!(MOB_PART_COUNTS.len(), MOB_KIND_COUNT);
    // 四种贴图路径齐全（结构可扩展）
    assert_eq!(MOB_TEX_FILES.len(), MOB_TEX_LAYERS);
    for f in MOB_TEX_FILES.iter() {
        assert!(f.starts_with("entity/"), "贴图必须来自原版 assets 树");
    }
}

#[test]
fn pose_matrices_finite_under_extreme() {
    let pose = MobPose {
        pos: glam::Vec3::new(-310.5, 61.0, 511.75),
        yaw: 5.1,
        phase: 2.7,
        amount: 0.88,
        head_pitch: 1.4,
        wing_angle: 2.0,
        head_drop: 1.0,
        head_eat_angle: 0.628,
    };
    for k in [
        MobModelKind::Chicken,
        MobModelKind::Cow,
        MobModelKind::Sheep,
        MobModelKind::Pig,
    ] {
        for m in mob_model_matrices(k, &pose) {
            for v in m.to_cols_array() {
                assert!(v.is_finite());
            }
        }
    }
}
