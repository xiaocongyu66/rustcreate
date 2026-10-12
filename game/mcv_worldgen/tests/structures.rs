//! 结构模板真件测试——依赖 ci/fetch-structures.sh 拉取的
//! assets/minecraft/structure/（不入 git）。无资产环境优雅 skip
//! （打印 K-STRUCT skip 行),有资产时断言。

use std::path::{Path, PathBuf};

use mcv_worldgen::vanilla::structure::{Mirror, Rotation, Template};

fn structure_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/minecraft/structure")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("assets/minecraft/structure"))
}

fn ruined_portal_files() -> Option<Vec<PathBuf>> {
    let dir = structure_dir().join("ruined_portal");
    if !dir.is_dir() {
        return None;
    }
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "nbt"))
        .collect();
    v.sort();
    if v.is_empty() { None } else { Some(v) }
}

#[test]
fn ruined_portal_templates_load_and_reencode_stable() {
    let Some(files) = ruined_portal_files() else {
        println!("K-STRUCT skip: assets/minecraft/structure/ 不在(跑 ci/fetch-structures.sh 启用)");
        return;
    };
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let tpl = Template::from_nbt(&bytes).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        assert!(
            tpl.size.iter().all(|&v| v >= 1),
            "{}: size {:?}",
            f.display(),
            tpl.size
        );
        assert!(!tpl.palette.is_empty(), "{}: palette 空", f.display());
        assert!(!tpl.blocks.is_empty(), "{}: blocks 空", f.display());
        // 重编码字节稳定:root compound 读→写→读→写,两次写字节相同。
        let root = mcv_nbt::read_auto(&bytes).unwrap();
        let b1 = mcv_nbt::write_raw(&root).unwrap();
        let root2 = mcv_nbt::read_raw(&b1).unwrap();
        assert_eq!(root, root2, "{}: 往返树不等", f.display());
        let b2 = mcv_nbt::write_raw(&root2).unwrap();
        assert_eq!(b1, b2, "{}: 二次编码字节不稳定", f.display());
    }
    println!(
        "K-STRUCT {} ruined_portal 模板真件解析+重编码稳定通过",
        files.len()
    );
}

#[test]
fn ruined_portal_place_deterministic_and_rotation_size_law() {
    let Some(files) = ruined_portal_files() else {
        println!("K-STRUCT skip: assets/minecraft/structure/ 不在");
        return;
    };
    let tpl = Template::from_nbt(&std::fs::read(&files[0]).unwrap()).unwrap();
    // 同 seed 两次输出逐字节一致。
    let a = tpl.place_at(
        [0, 0, 0],
        [0, 0, 0],
        Mirror::None,
        Rotation::None,
        1.0,
        0x5EED,
    );
    let b = tpl.place_at(
        [0, 0, 0],
        [0, 0, 0],
        Mirror::None,
        Rotation::None,
        1.0,
        0x5EED,
    );
    assert_eq!(a, b);
    assert!(!a.is_empty());
    // integrity 半量必少于全量(同 seed 单调前缀?掷骰独立,只断言 ≤)。
    let half = tpl.place_at(
        [0, 0, 0],
        [0, 0, 0],
        Mirror::None,
        Rotation::None,
        0.5,
        0x5EED,
    );
    assert!(half.len() <= a.len());

    // 90° 旋转包围盒 x/z 互换律(对模板整体包围盒,用 pivot=中心)。
    let ext = |v: &[(i32, i32, i32, mcv_core::BlockId)]| {
        let x0 = v.iter().map(|p| p.0).min().unwrap();
        let x1 = v.iter().map(|p| p.0).max().unwrap();
        let z0 = v.iter().map(|p| p.2).min().unwrap();
        let z1 = v.iter().map(|p| p.2).max().unwrap();
        (x1 - x0 + 1, z1 - z0 + 1)
    };
    let plain = ext(&a);
    let rot = ext(&tpl.place_at(
        [0, 0, 0],
        [tpl.size[0] / 2, 0, tpl.size[2] / 2],
        Mirror::None,
        Rotation::Clockwise90,
        1.0,
        0x5EED,
    ));
    // 90° 旋转是刚体变换 (x,z)→(±z+c, ±x+c):包围盒 x/z 跨度严格互换。
    assert_eq!(
        rot,
        (plain.1, plain.0),
        "90° 包围盒互换律: plain {plain:?} rot {rot:?}"
    );
    // mirror 保包围盒尺寸。
    let mir = ext(&tpl.place_at(
        [0, 0, 0],
        [0, 0, tpl.size[2] - 1],
        Mirror::LeftRight,
        Rotation::None,
        1.0,
        0x5EED,
    ));
    assert_eq!(mir, plain, "mirror 应保包围盒尺寸");
}

#[test]
fn all_asset_templates_parse() {
    // 全树冒烟:1202 件全部可解析(资产在时)。
    let dir = structure_dir();
    if !dir.is_dir() {
        println!("K-STRUCT skip: assets/minecraft/structure/ 不在");
        return;
    }
    let mut n = 0usize;
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read_dir") {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "nbt")
                && p.file_stem().is_some_and(|s| s != "empty")
            {
                let bytes = std::fs::read(&p).unwrap();
                let _ =
                    Template::from_nbt(&bytes).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                n += 1;
            }
        }
    }
    println!("K-STRUCT {n} 个结构模板全树解析通过");
    assert!(n > 1000, "全树模板数异常: {n}");
}
