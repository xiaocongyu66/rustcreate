//! Text → HUD quad layout (bitmap font, integer scaling).

use crate::font::{glyph_uv, SOLID_CELL};
use crate::gpu::HudQuad;

pub const GLYPH_PX: f32 = 8.0;

/// Lays out a string as glyph quads. ASCII 32..127; other chars become '?'.
pub fn text_quads(s: &str, x: f32, y: f32, scale: f32, color: [f32; 4]) -> Vec<HudQuad> {
    let mut out = Vec::with_capacity(s.len());
    let mut cx = x;
    for ch in s.chars() {
        if ch == ' ' {
            cx += GLYPH_PX * scale;
            continue;
        }
        let code = ch as u32;
        let glyph = if (32..127).contains(&code) { code as u8 } else { b'?' };
        out.push(HudQuad {
            x: cx,
            y,
            w: GLYPH_PX * scale,
            h: GLYPH_PX * scale,
            uv: glyph_uv(glyph),
            color,
            tex: 0,
            layer: 0,
        });
        cx += GLYPH_PX * scale;
    }
    out
}

/// Solid rectangle (via the reserved white cell).
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
    }
}

/// Terrain tile icon (hotbar).
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
    }
}
