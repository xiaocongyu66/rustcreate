//! Text → HUD quad layout。MC 规则:格 = codepoint(见 font.rs),
//! 逐字形 advance,先画偏移 (1,1) 的暗色阴影再画正文
//! (Font.java:shadow = (text & 0xFCFCFC) >> 2)。

use crate::font::{advance, glyph_uv, SOLID_CELL};
use crate::gpu::HudQuad;

pub const GLYPH_PX: f32 = 8.0;

/// 缺失字形统一显示 '?'(MC: AllMissingGlyphProvider)。
const MISSING: u32 = b'?' as u32;

/// 把 codepoint 映射到可渲染的格:可打印且有宽度;空格跳过;其余归 '?'.
fn cell_for(ch: char) -> Option<u32> {
    let cp = ch as u32;
    if cp == b' ' as u32 {
        return None;
    }
    let w = advance(cp);
    if cp < 256 && w > 1.0 {
        Some(cp)
    } else {
        Some(MISSING)
    }
}

/// 字符串像素宽(字体像素,未乘 scale)。
pub fn text_width(s: &str, scale: f32) -> f32 {
    s.chars().map(|c| advance(c as u32)).sum::<f32>() * scale
}

/// 单色文字 → 字形 quad 列表(先阴影后正文,支持任意 scale)。
pub fn text_quads(s: &str, x: f32, y: f32, scale: f32, color: [f32; 4]) -> Vec<HudQuad> {
    let mut out = Vec::with_capacity(s.len() * 2);
    // MC 阴影色:(c & 0xFCFCFC) >> 2,alpha 保持
    let shadow = [
        (color[0] * 0.25).min(0.99),
        (color[1] * 0.25).min(0.99),
        (color[2] * 0.25).min(0.99),
        color[3],
    ];
    // 阴影偏移固定 1 屏幕像素(MC shadowOffset = 1,不随字号缩放)
    for (dx, dy, col) in [(1.0, 1.0, shadow), (0.0, 0.0, color)] {
        let mut cx = x + dx;
        for ch in s.chars() {
            let adv = advance(ch as u32) * scale;
            if let Some(cell) = cell_for(ch) {
                out.push(HudQuad {
                    x: cx,
                    y: y + dy,
                    w: GLYPH_PX * scale,
                    h: GLYPH_PX * scale,
                    uv: glyph_uv(cell),
                    color: col,
                    tex: 0,
                    layer: 0,
                    rot: 0.0,
                });
            }
            cx += adv;
        }
    }
    out
}

/// 居中文字(x 为屏幕中心横坐标)。
pub fn text_quads_centered(s: &str, cx: f32, y: f32, scale: f32, color: [f32; 4]) -> Vec<HudQuad> {
    text_quads(s, cx - text_width(s, scale) * 0.5, y, scale, color)
}

/// 纯色矩形(借用字体图集的保留实心白格)。
pub fn rect(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> HudQuad {
    HudQuad {
        x,
        y,
        w,
        h,
        uv: glyph_uv(SOLID_CELL),
        color,
        tex: 0,
        layer: 0,
        rot: 0.0,
    }
}

/// 地形贴图图标(快捷栏槽位)。
pub fn tile_icon(layer: u8, x: f32, y: f32, size: f32) -> HudQuad {
    HudQuad {
        x,
        y,
        w: size,
        h: size,
        uv: [[0.0, 0.0], [1.0, 1.0]],
        color: [1.0, 1.0, 1.0, 1.0],
        tex: 1,
        layer: u32::from(layer),
        rot: 0.0,
    }
}
