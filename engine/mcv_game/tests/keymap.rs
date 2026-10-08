use mcv_game::keymap::{Action, KeyMap, VKey};

#[test]
fn default_roundtrip() {
    let km = KeyMap::default();
    assert_eq!(KeyMap::from_text(&km.to_text()), km);
}

#[test]
fn edited_roundtrip() {
    let mut km = KeyMap::default();
    km.set(Action::Jump, VKey::KeyR);
    km.set(Action::Forward, VKey::ArrowUp);
    assert_eq!(KeyMap::from_text(&km.to_text()), km);
}

#[test]
fn defaults_match_minecraft() {
    let km = KeyMap::default();
    assert_eq!(km.get(Action::Forward), VKey::KeyW);
    assert_eq!(km.get(Action::Jump), VKey::Space);
    assert_eq!(km.get(Action::Sneak), VKey::ShiftLeft);
    assert_eq!(km.get(Action::Sprint), VKey::ControlLeft);
    assert_eq!(km.get(Action::Debug), VKey::F3);
    assert_eq!(km.get(Action::Inventory), VKey::KeyE);
    assert_eq!(km.get(Action::Pause), VKey::Escape);
}

#[test]
fn tolerant_parsing_skips_unknown() {
    let text = "\
        forward:KeyZ\n\
        bogus_action:KeyQ\n\
        jump:NoSuchKey\n\
        no-colon-line\n\
        # comment:whatever\n\
        \n\
        left:BogusKeyName\n\
        right:KeyX\n";
    let km = KeyMap::from_text(text);
    assert_eq!(km.get(Action::Forward), VKey::KeyZ); // 合法行生效
    assert_eq!(km.get(Action::Right), VKey::KeyX);
    assert_eq!(km.get(Action::Jump), VKey::Space); // 未知键 → 保留默认
    assert_eq!(km.get(Action::Left), VKey::KeyA);
    assert_eq!(km.get(Action::Back), VKey::KeyS); // 缺失动作 → 默认
}

#[test]
fn conflict_detection() {
    let mut km = KeyMap::default();
    // 把 Left 也绑到 W（Forward 的键）：MC 允许，但报告冲突。
    km.set(Action::Left, VKey::KeyW);
    assert_eq!(
        km.conflicting(Action::Left, VKey::KeyW),
        Some(Action::Forward)
    );
    // 目标键无人占用时无冲突；与自身当前绑定也不算冲突。
    assert_eq!(km.conflicting(Action::Left, VKey::Tab), None);
    assert_eq!(km.conflicting(Action::Right, VKey::KeyD), None);
    // 冲突键的动作查询取 Action::ALL 顺序靠前者（确定性）。
    assert_eq!(km.action_for(VKey::KeyW), Some(Action::Forward));
}

#[test]
fn winit_name_roundtrip() {
    for k in VKey::ALL {
        assert_eq!(VKey::from_winit_name(k.winit_name()), Some(k));
    }
    assert_eq!(VKey::from_winit_name("keyw"), Some(VKey::KeyW)); // 大小写不敏感
    assert_eq!(VKey::from_winit_name("nope"), None);
}

#[test]
fn every_action_persisted() {
    let km = KeyMap::default();
    assert_eq!(km.to_text().lines().count(), Action::ALL.len());
}
