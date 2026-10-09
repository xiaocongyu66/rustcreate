//! 全量原版音效树完整性:树不入 git,由 `ci/fetch-sounds.sh` 在构建时拉取。
//! 若 `sounds/sounds.json` 存在(本地已 fetch 或 CI test job 已拉取),校验全部
//! 非 stream 变体的 ogg 文件存在且为合法 ogg;不存在则打印 skip 直接通过
//! (校验在 CI 的 test job 真实发生)。
use mcv_audio::{SoundLoader, SoundTable, default_sounds_dir};

#[test]
fn full_tree_variants_exist_when_table_present() {
    let dir = default_sounds_dir();
    let table = match SoundTable::load(&dir) {
        Ok(t) => t,
        Err(_) => {
            println!("skip: sounds/sounds.json 不存在(全量树未 fetch),跳过全树校验");
            return;
        }
    };
    assert!(!table.is_empty(), "sounds.json 存在但解析为空表");
    println!("events: {}", table.len());

    // 已知关键事件必须可解析(fixture 素材映射的锚点)。
    let loader = SoundLoader::new(&dir);
    for event in [
        "block.stone.break",
        "block.stone.place",
        "block.stone.step",
        "block.grass.break",
        "block.grass.step",
        "block.wood.break",
        "block.wood.step",
        "entity.player.hurt",
        "entity.player.small_fall",
        "entity.experience_orb.pickup",
    ] {
        assert!(
            loader.table().pick(event, 0).is_some(),
            "关键事件 {event} 缺失:26.1 素材映射失效"
        );
    }

    // 全树校验:每个非 stream 变体的 ogg 必须存在且带 OggS 魔数。
    let mut checked = 0usize;
    for (event, se) in table.events() {
        for v in &se.variants {
            if v.stream {
                continue;
            }
            let path = dir.join(format!("{}.ogg", v.path));
            let head = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("{event}: 变体文件读取失败 {:?}: {e}", path));
            assert!(
                head.len() > 64 && &head[..4] == b"OggS",
                "{event}: {:?} 不是合法 ogg",
                path
            );
            checked += 1;
        }
    }
    println!("checked {checked} non-stream ogg variants");
    assert!(checked > 4000, "非 stream 变体数异常: {checked}");
}

#[test]
fn dev_flat_files_still_present() {
    // 仓库自带的 12 个扁平开发音效必须始终在(fixture 单测依赖,见 common)。
    let dir = default_sounds_dir();
    for name in [
        "dig_stone1",
        "dig_grass1",
        "dig_wood1",
        "step_stone1",
        "step_grass1",
        "step_wood1",
        "random_bow",
        "random_orb",
        "damage_hit1",
        "damage_hit2",
        "damage_hit3",
        "damage_fallsmall",
    ] {
        let path = dir.join(format!("{name}.ogg"));
        assert!(path.is_file(), "扁平开发音效缺失: {:?}", path);
        let head = std::fs::read(&path).unwrap();
        assert!(
            head.len() > 64 && &head[..4] == b"OggS",
            "{:?} 不是合法 ogg",
            path
        );
    }
}
