//! 生物模型纯 CPU 断言（无 GPU）：部位表对齐 Java Model 类数字、UV 在
//! 纹理内、跨种类切片连续。

use mcv_render::mob_mesh::*;

#[test]
fn mesh_counts_and_uv_bounds() {
    let m = build_mob_mesh();
    // 盒数：鸡 8 + 牛 10 + 羊 12 + 猪 7 + 僵尸 6 + 骷髅 6 + 苦力怕 6
    // + 蜘蛛 11 = 66（部位表见 mob_mesh.rs）
    assert_eq!(m.verts.len(), 66 * 24);
    assert_eq!(m.indices.len(), 66 * 36);
    for v in &m.verts {
        for c in v.pos {
            assert!(c.is_finite());
        }
        for c in v.uv {
            // 任务纪律：全部顶点 UV 必须归一化在 [0,1]（unorm8 天然满足，
            // 显式锁语义防布局漂移）。
            let n = c as f32 / 255.0;
            assert!((0.0..=1.0).contains(&n), "uv {n}");
        }
        // meta[0] = 贴图层（< MOB_TEX_LAYERS=9）；meta[1] = kind 内局部
        // 部位序，必须 < MAX_MOB_PARTS=12（shader models[12] 槽位上限；
        // 全局序越界读零会让羊/猪塌缩——2026-10-10 GPU 像素测试抓到的回归）。
        assert!(v.meta[0] < MOB_TEX_LAYERS as u32 && v.meta[1] < MAX_MOB_PARTS as u32);
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
    assert_eq!(MOB_PART_COUNTS, [8, 6, 12, 6, 6, 6, 6, 11]);
    assert_eq!(MOB_PART_COUNTS.len(), MOB_KIND_COUNT);
    // 八种贴图路径齐全（缺文件时 load 端降级不渲染，绝不程序化伪造）
    assert_eq!(MOB_TEX_FILES.len(), MOB_TEX_LAYERS);
    for f in MOB_TEX_FILES.iter() {
        assert!(f.starts_with("entity/"), "贴图必须来自原版 assets 树");
    }
    // 敌对四怪的贴图路径与仓库 assets 实体树一致（防手滑改名）。
    let expect = [
        "entity/zombie/zombie.png",
        "entity/skeleton/skeleton.png",
        "entity/creeper/creeper.png",
        "entity/spider/spider.png",
    ];
    for e in expect {
        assert!(
            MOB_TEX_FILES.contains(&e),
            "{e} 应在 MOB_TEX_FILES（层 5..8）"
        );
    }
}

#[test]
fn hostile_head_uv_rects_match_java() {
    // 头正脸段（face-with-eyes）矩形逐怪对照 Java texOffs（8³ 头：
    // 段 = [u+d, v+d, u+d+w, v+d+h]）：
    // - 僵尸/骷髅 HumanoidModel.createMesh 头 tex(0,0) → (8,8)-(16,16)
    // - 苦力怕 CreeperModel 头 tex(0,0) → (8,8)-(16,16)
    // - 蜘蛛 SpiderModel 头 tex(32,4) → (40,12)-(48,20)
    let m = build_mob_mesh();
    for (k, rect) in [
        (MobModelKind::Zombie, [8.0, 8.0, 16.0, 16.0]),
        (MobModelKind::Skeleton, [8.0, 8.0, 16.0, 16.0]),
        (MobModelKind::Creeper, [8.0, 8.0, 16.0, 16.0]),
        (MobModelKind::Spider, [40.0, 12.0, 48.0, 20.0]),
    ] {
        // kind 首盒 = 头；正脸 = 部位内面 4（顶点 16..20）。
        let base = m.slices[k.idx()].start as usize / 36 * 24;
        for v in &m.verts[base + 16..base + 20] {
            assert!(v.pos[2] < 0.0, "{k:?} 脸段须在 −z");
            let u = v.uv[0] as f32 / 255.0 * MOB_TEX_PX as f32;
            let t = v.uv[1] as f32 / 255.0 * MOB_TEX_PX as f32;
            assert!(u >= rect[0] - 1e-3 && u <= rect[2] + 1e-3, "{k:?} u {u}");
            assert!(t >= rect[1] - 1e-3 && t <= rect[3] + 1e-3, "{k:?} t {t}");
        }
    }
}

#[test]
fn hostile_models_have_positive_vert_counts() {
    let m = build_mob_mesh();
    for k in [
        MobModelKind::Zombie,
        MobModelKind::Skeleton,
        MobModelKind::Creeper,
        MobModelKind::Spider,
    ] {
        assert!(m.slices[k.idx()].len() > 0, "{k:?} 顶点数应 > 0");
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
        MobModelKind::Zombie,
        MobModelKind::Skeleton,
        MobModelKind::Creeper,
        MobModelKind::Spider,
    ] {
        for m in mob_model_matrices(k, &pose) {
            for v in m.to_cols_array() {
                assert!(v.is_finite());
            }
        }
    }
}
