//! Unifont 位图字体(cjk.f16)加载与排版测试。纯 CPU,无需 GPU。

use mcv_render::unifont::{Unifont, DEFAULT_ATLAS_TEX, FULL_ADVANCE, HALF_ADVANCE};

fn load() -> Unifont {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/font/cjk.f16");
    let bytes = std::fs::read(path).expect("cjk.f16 缺失:先跑 python3 ci/gen-unifont.py");
    Unifont::from_bytes(&bytes).expect("cjk.f16 解析失败")
}

fn ink_pixels(f: &Unifont, cp: u32) -> usize {
    let (atlas, w, _h) = f.atlas_rgba();
    let cols = w / 16;
    let cell = f.cell_of(cp) as usize;
    let cx = (cell % cols) * 16;
    let cy = (cell / cols) * 16;
    (0..16)
        .flat_map(|r| (0..16).map(move |c| (c, r)))
        .filter(|&(c, r)| atlas[((cy + r) * w + cx + c) * 4 + 3] != 0)
        .count()
}

#[test]
fn loads_and_common_glyphs_have_ink() {
    let f = load();
    // 抽查常用字:位图必须非空
    for ch in ['游', '戏', '设', '置'] {
        assert!(f.has(ch as u32), "缺字形 {ch}");
        assert!(ink_pixels(&f, ch as u32) > 0, "{ch} 位图为空");
        assert_eq!(
            f.quads(ch.encode_utf8(&mut [0u8; 4]), 0.0, 0.0, 1.0, [1.0; 4])
                .len(),
            1
        );
    }
}

#[test]
fn fullwidth_ascii_is_double_advance() {
    let f = load();
    // Ａ = U+FF21 全角:advance = 2×半角(MC 规则)
    assert!(f.has(0xFF21));
    assert_eq!(f.advance(0xFF21), Some(FULL_ADVANCE));
    assert_eq!(FULL_ADVANCE, 2.0 * HALF_ADVANCE);
    assert_eq!(f.measure("Ａ", 1.0).0, 12.0);
    assert_eq!(f.measure("ＡＢ", 2.0).0, 48.0);
}

#[test]
fn halfwidth_forms_are_half_advance() {
    let f = load();
    // ｱ = U+FF61(半角片假名,源 hex 8 列宽)
    assert!(f.has(0xFF61));
    assert_eq!(f.advance(0xFF61), Some(HALF_ADVANCE));
}

#[test]
fn ascii_excluded_and_missing_renders_box() {
    let f = load();
    assert!(!f.covers('A')); // ASCII 走 text.rs
    assert!(f.covers('中'));
    let missing = '\u{FFFF}'; // 未收录
    assert!(!f.has(missing as u32));
    let q = f.quads(missing.to_string().as_str(), 0.0, 0.0, 1.0, [1.0; 4]);
    assert_eq!(q.len(), 1, "缺字应画空心框而非丢弃");
    assert_eq!(q[0].tex, DEFAULT_ATLAS_TEX);
    assert!(
        ink_pixels(&f, f.cell_of(missing as u32)) > 0,
        "空心框格应有像素"
    );
}

#[test]
fn menu_strings_fully_covered() {
    let f = load();
    // zh 菜单文案抽查:全部收录,且整句宽 = 字数×全角 advance
    let samples = [
        "单人游戏",
        "退出游戏",
        "选项",
        "创建新的世界",
        "返回",
        "游戏模式",
        "生存模式",
        "渲染距离",
        "鼠标灵敏度",
        "保存并退回到标题屏幕",
    ];
    for s in samples {
        for ch in s.chars() {
            assert!(f.has(ch as u32), "缺字形: {ch} U+{:04X}", ch as u32);
        }
        assert_eq!(f.measure(s, 1.0).0, s.chars().count() as f32 * FULL_ADVANCE);
    }
}
