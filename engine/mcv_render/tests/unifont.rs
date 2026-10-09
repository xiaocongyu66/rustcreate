//! Unifont 位图字体(cjk.f16)加载与排版测试。纯 CPU,无需 GPU。
//! 度量断言对齐原版:UnihexProvider.java:298(advance = 位图宽/2+1)、
//! :320(oversample 2.0)与 GlyphBitmap.java:20-30(quad = 位图/2),
//! Font.java:37(lineHeight 9)。

use mcv_render::text::GLYPH_PX;
use mcv_render::unifont::{
    DEFAULT_ATLAS_TEX, FULL_ADVANCE, FULL_W, GLYPH_H, HALF_ADVANCE, HALF_W, OVERSAMPLE,
    SPACE_ADVANCE, Unifont,
};

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
fn advance_matches_unihex_provider() {
    let f = load();
    // Ａ = U+FF21 全角:advance = 位图宽/2 + 1(UnihexProvider.java:298)
    assert!(f.has(0xFF21));
    assert_eq!(f.advance(0xFF21), Some(FULL_ADVANCE));
    assert_eq!(FULL_ADVANCE, FULL_W / OVERSAMPLE + 1.0);
    assert_eq!(HALF_ADVANCE, HALF_W / OVERSAMPLE + 1.0);
    assert_eq!(f.measure("Ａ", 1.0).0, 9.0);
    assert_eq!(f.measure("ＡＢ", 2.0).0, 36.0);
}

#[test]
fn halfwidth_forms_are_half_advance() {
    let f = load();
    // ｱ = U+FF61(半角片假名,源 hex 8 列宽)
    assert!(f.has(0xFF61));
    assert_eq!(f.advance(0xFF61), Some(HALF_ADVANCE));
}

#[test]
fn glyph_quad_height_matches_ascii_line_box() {
    let f = load();
    // 原版不变量:unifont quad = 位图/oversample(UnihexProvider.java:320
    // + GlyphBitmap.java:20-30),全角 8x8、半角 4x8,与 ASCII 的 8px 格
    // 同尺度(Font.java:37 行高 9 → 8px 字形 + 1px 阴影行)。
    for (ch, want_w) in [
        ('游', FULL_W / OVERSAMPLE),
        ('戏', FULL_W / OVERSAMPLE),
        ('ｱ', HALF_W / OVERSAMPLE),
    ] {
        let q = f.quads(ch.encode_utf8(&mut [0u8; 4]), 0.0, 0.0, 1.0, [1.0; 4]);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].h, GLYPH_H / OVERSAMPLE, "{ch} 绘制高应为 8");
        assert_eq!(q[0].h, GLYPH_PX, "{ch} 与 ASCII 字形同高");
        assert_eq!(q[0].w, want_w, "{ch} 绘制宽");
    }
}

#[test]
fn space_uses_space_provider_advance() {
    let f = load();
    // 空格由 SpaceProvider 承担(default font 空格 = 4),unifont 路径
    // 保持与 ASCII 宽度表一致(font.rs)。
    assert_eq!(f.measure(" ", 1.0).0, SPACE_ADVANCE);
    assert_eq!(SPACE_ADVANCE, 4.0);
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
    // 缺字框按全角占位:advance 9、绘制 8x8(同原版全角字形度量)
    assert_eq!(q[0].w, FULL_W / OVERSAMPLE);
    assert_eq!(q[0].h, GLYPH_H / OVERSAMPLE);
    assert!(
        ink_pixels(&f, f.cell_of(missing as u32)) > 0,
        "空心框格应有像素"
    );
}

#[test]
fn menu_strings_fully_covered() {
    let f = load();
    // zh 菜单文案抽查:全部收录,且整句宽 = 字数×全角 advance
    // (UnihexProvider.java:298:全角 advance = 16/2+1 = 9)
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
