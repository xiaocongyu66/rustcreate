//! 开发期素材完整性:`sounds/` 下每个 SoundId 对应的 ogg 文件必须存在。
use mcv_audio::{SoundId, default_sounds_dir};

#[test]
fn every_sound_id_has_a_file() {
    let dir = default_sounds_dir();
    assert!(dir.is_dir(), "素材目录不存在: {:?}", dir);
    for id in SoundId::ALL {
        let path = dir.join(id.file_name());
        assert!(path.is_file(), "{id:?} 对应素材缺失: {:?}", path);
        // ogg 魔数抽查,防止占位空文件混入。
        let head = std::fs::read(&path).unwrap();
        assert!(
            head.len() > 64 && &head[..4] == b"OggS",
            "{:?} 不是合法 ogg",
            path
        );
    }
}

#[test]
fn sound_id_tables_are_consistent() {
    // ALL 覆盖全部变体;文件名唯一;vanilla 路径的基名须出现在扁平化文件名里。
    assert_eq!(SoundId::ALL.len(), 12);
    let mut seen_files = std::collections::HashSet::new();
    for id in SoundId::ALL {
        assert!(
            seen_files.insert(id.file_name()),
            "文件名重复: {}",
            id.file_name()
        );
        assert!(id.file_name().ends_with(".ogg"));
        let base = id.vanilla_path().rsplit('/').next().unwrap();
        assert!(
            id.file_name().contains(base),
            "{id:?}: {:?} 与 {:?} 对不上",
            id.file_name(),
            id.vanilla_path()
        );
    }
}
