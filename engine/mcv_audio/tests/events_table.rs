//! 事件→音效名映射表完备性（本波新增接线 #143 的守护测试）。
//!
//! 两层：
//! 1. **纯表层**（不依赖素材）：`events::WIRED` 与语义映射表集合相等、
//!    无重复、每个映射音名 ∈ 已接事件集——调用点只准引用常量，
//!    字面量改名必须同步两处，这里锁死。
//! 2. **白名单层**（sounds.json 在场时真校验）：每个已接音名必须存在于
//!    原版 `sounds.json`（资产索引 index 30）。全量树不入 git，本地未
//!    fetch 时打印 skip 通过（CI test job 先跑 `ci/fetch-sounds.sh`，
//!    校验在 CI 真实发生——与 `sounds_exist.rs` 同款降级约定）。
use mcv_audio::{SoundLoader, SoundTable, default_sounds_dir, events};

/// 语义键 → 已接事件名（mcv_logic/game.rs 与 mcv_app/app.rs 调用点
/// 引用的 [`events`] 常量全量；键名对应消费点见 events.rs 文档引注）。
const MAPPING: &[(&str, &str)] = &[
    ("player_hurt", events::PLAYER_HURT),
    ("player_splash", events::PLAYER_SPLASH),
    ("player_splash_high_speed", events::PLAYER_SPLASH_HIGH_SPEED),
    ("player_fall_big", events::PLAYER_FALL_BIG),
    ("player_fall_small", events::PLAYER_FALL_SMALL),
    ("item_break", events::ITEM_BREAK),
    ("generic_eat", events::GENERIC_EAT),
    ("arrow_shoot", events::ARROW_SHOOT),
    ("item_pickup", events::ITEM_PICKUP),
    ("ui_button_click", events::UI_BUTTON_CLICK),
];

#[test]
fn mapping_table_is_complete_and_consistent_with_wired() {
    // 映射键唯一。
    let mut keys: Vec<&str> = MAPPING.iter().map(|(k, _)| *k).collect();
    keys.sort_unstable();
    let key_dups = keys.len()
        - keys
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len();
    assert_eq!(key_dups, 0, "映射键重复: {keys:?}");

    // 映射音名 ∈ 已接事件集。
    for (key, name) in MAPPING {
        assert!(
            events::WIRED.contains(name),
            "映射 {key} → {name}: 音名不在已接事件集 WIRED"
        );
    }

    // WIRED 无重复。
    let mut wired: Vec<&str> = events::WIRED.to_vec();
    wired.sort_unstable();
    let wired_dups = wired.len()
        - wired
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len();
    assert_eq!(wired_dups, 0, "WIRED 内部重复: {wired:?}");

    // 集合相等：WIRED 无孤儿（表完备），映射无越集（表不虚构）。
    let mut mapped: Vec<&str> = MAPPING.iter().map(|(_, v)| *v).collect();
    mapped.sort_unstable();
    assert_eq!(wired, mapped, "WIRED 与映射表集合不一致");
}

#[test]
fn wired_events_exist_in_vanilla_sounds_json() {
    let dir = default_sounds_dir();
    let table = match SoundTable::load(&dir) {
        Ok(t) => t,
        Err(_) => {
            println!("skip: sounds/sounds.json 不存在(全量树未 fetch),跳过白名单校验");
            return;
        }
    };
    assert!(!table.is_empty(), "sounds.json 存在但解析为空表");
    let loader = SoundLoader::new(&dir);
    for (key, name) in MAPPING {
        assert!(
            loader.table().pick(name, 0).is_some(),
            "映射 {key} → {name}: 事件不在原版 sounds.json(index 30)——26.1 改名未对齐或私造事件名"
        );
    }
}
