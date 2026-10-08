//! MC GUI 精灵表:从 texturepack/gui/*.png 读取原版按钮/HUD 精灵/logo,
//! 运行时拼成一张 RGBA 纹理(HUD 管线 tex=2)。任一文件缺失/解码失败都
//! 返回 None,调用方回退程序化绘制(安卓包不带素材也不能崩)。
//!
//! 尺寸参照(见 /root/mc-ref/NOTES-ui.md):button.png 200x20 九宫格
//! border=3(AbstractButton + mcmeta),hotbar.png 182x22、选中 24x23、
//! 心/饥饿 9x9、准星 15x15(Gui.java),logo 逻辑 256x64 取上 44
//! (LogoRenderer.LOGO_HEIGHT)。

use crate::gpu::HudQuad;

/// 精灵条目:文件名(texturepack/gui/ 下)→ 逻辑尺寸。
const SPRITES: [(&str, &str, u32, u32); 13] = [
    ("logo", "minecraft.png", 256, 64),
    ("button", "button.png", 200, 20),
    ("button_hl", "button_highlighted.png", 200, 20),
    ("button_dis", "button_disabled.png", 200, 20),
    ("hotbar", "hotbar.png", 182, 22),
    ("hotbar_sel", "hotbar_selection.png", 24, 23),
    ("crosshair", "crosshair.png", 15, 15),
    ("heart_container", "heart_container.png", 9, 9),
    ("heart_full", "heart_full.png", 9, 9),
    ("heart_half", "heart_half.png", 9, 9),
    ("food_empty", "food_empty.png", 9, 9),
    ("food_full", "food_full.png", 9, 9),
    ("food_half", "food_half.png", 9, 9),
];

/// logo 实际绘制行数(MC 纹理 256x64 只显示上 44 行)。
pub const LOGO_VISIBLE_H: u32 = 44;

/// 一张拼好的 GUI 精灵表 + 名称→表内像素矩形。
pub struct SpriteSheet {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
    entries: Vec<(&'static str, u32, u32, u32, u32)>, // name, x, y, w, h
}

/// 近邻重采样 RGBA 到目标尺寸。
fn resample(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32, dst: &mut [u8]) {
    for y in 0..dh as usize {
        for x in 0..dw as usize {
            let sx = (x as u32 * sw / dw).min(sw - 1) as usize;
            let sy = (y as u32 * sh / dh).min(sh - 1) as usize;
            let s = (sy * sw as usize + sx) * 4;
            let d = (y * dw as usize + x) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

impl SpriteSheet {
    /// 从纹理包目录(texturepack 根)读 gui/ 子目录。缺文件 → None。
    pub fn load(dir: &std::path::Path) -> Option<Self> {
        let mut loaded: Vec<(&'static str, Vec<u8>, u32, u32)> = Vec::new();
        for (name, file, w, h) in SPRITES {
            let path = dir.join("gui").join(file);
            let bytes = std::fs::read(&path).ok()?;
            let img = image::load_from_memory(&bytes)
                .map_err(|e| log::warn!("gui: decode {}: {e}", path.display()))
                .ok()?;
            let rgba = img.to_rgba8();
            let (sw, sh) = (rgba.width(), rgba.height());
            if sw == 0 || sh == 0 {
                return None;
            }
            let mut buf = vec![0u8; (w * h * 4) as usize];
            resample(rgba.as_raw(), sw, sh, w, h, &mut buf);
            loaded.push((name, buf, w, h));
        }
        // 竖排:宽度取最大(256),总高向上取整到 2 的幂
        let sheet_w = loaded.iter().map(|(_, _, w, _)| *w).max().unwrap_or(0);
        let total_h: u32 = loaded.iter().map(|(_, _, _, h)| *h).sum();
        let mut sheet_h = 32u32;
        while sheet_h < total_h {
            sheet_h *= 2;
        }
        let mut rgba = vec![0u8; (sheet_w * sheet_h * 4) as usize];
        let mut entries = Vec::with_capacity(loaded.len());
        let mut y = 0u32;
        for (name, buf, w, h) in loaded {
            for row in 0..h {
                let s = ((row * w * 4) as usize)..(((row + 1) * w * 4) as usize);
                let d =
                    (((y + row) * sheet_w * 4) as usize)..(((y + row) * sheet_w + w) * 4) as usize;
                rgba[d].copy_from_slice(&buf[s]);
            }
            entries.push((name, 0, y, w, h));
            y += h;
        }
        log::info!("gui: MC sprite sheet {sheet_w}x{sheet_h} loaded");
        Some(Self {
            rgba,
            w: sheet_w,
            h: sheet_h,
            entries,
        })
    }

    fn rect(&self, name: &str) -> Option<(u32, u32, u32, u32)> {
        self.entries
            .iter()
            .find(|(n, ..)| *n == name)
            .map(|(_, x, y, w, h)| (*x, *y, *w, *h))
    }

    fn uv_px(&self, x: f32, y: f32, w: f32, h: f32) -> [[f32; 2]; 2] {
        [
            [x / self.w, y / self.h],
            [(x + w) / self.w, (y + h) / self.h],
        ]
    }

    /// 整枚精灵贴图四边形(源矩形按比例取子区域,frac ∈ 0..1)。
    #[allow(clippy::too_many_arguments)] // 精灵绘制参数天然多（源矩形+目标矩形+着色）
    pub fn sprite(
        &self,
        name: &str,
        fx0: f32,
        fy0: f32,
        fx1: f32,
        fy1: f32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
    ) -> Option<HudQuad> {
        let (rx, ry, rw, rh) = self.rect(name)?;
        let u0 = rx as f32 + rw as f32 * fx0;
        let v0 = ry as f32 + rh as f32 * fy0;
        let u1 = rx as f32 + rw as f32 * fx1;
        let v1 = ry as f32 + rh as f32 * fy1;
        Some(HudQuad {
            x,
            y,
            w,
            h,
            uv: self.uv_px(u0, v0, u1 - u0, v1 - v0),
            color,
            tex: 2,
            layer: 0,
            rot: 0.0,
        })
    }

    /// 整枚精灵。
    pub fn sprite_full(
        &self,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
    ) -> Option<HudQuad> {
        self.sprite(name, 0.0, 0.0, 1.0, 1.0, x, y, w, h, color)
    }

    /// 九宫格拉伸(button.png 的 mcmeta:nine_slice border=3 源像素)。
    /// `scale` 为 GUI 整数缩放:目标圆角块取 border*scale,其余拉伸。
    #[allow(clippy::too_many_arguments)]
    pub fn nine_slice(
        &self,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        border_src: u32,
        scale: f32,
        color: [f32; 4],
    ) -> Vec<HudQuad> {
        let Some((rx, ry, rw, rh)) = self.rect(name) else {
            return Vec::new();
        };
        let sb = border_src.min(rw / 2).min(rh / 2) as f32;
        let bw = (sb * scale).min(w * 0.5);
        let bh = (sb * scale).min(h * 0.5);
        let (bx, by) = (w - bw * 2.0, h - bh * 2.0);
        let (sf, tf) = (rw as f32, rh as f32);
        let mut out = Vec::with_capacity(9);
        // (源 x, 源 y, 源 w, 源 h, 目标相对 x, 目标相对 y, 目标 w, 目标 h)
        let cells = [
            (0.0, 0.0, sb, sb, 0.0, 0.0, bw, bh),
            (sb, 0.0, sf - sb * 2.0, sb, bw, 0.0, bx, bh),
            (sf - sb, 0.0, sb, sb, w - bw, 0.0, bw, bh),
            (0.0, sb, sb, tf - sb * 2.0, 0.0, bh, bw, by),
            (sb, sb, sf - sb * 2.0, tf - sb * 2.0, bw, bh, bx, by),
            (sf - sb, sb, sb, tf - sb * 2.0, w - bw, bh, bw, by),
            (0.0, tf - sb, sb, sb, 0.0, h - bh, bw, bh),
            (sb, tf - sb, sf - sb * 2.0, sb, bw, h - bh, bx, bh),
            (sf - sb, tf - sb, sb, sb, w - bw, h - bh, bw, bh),
        ];
        for (sx, sy, sw, sh, dx, dy, dw, dh) in cells {
            if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
                continue;
            }
            out.push(HudQuad {
                x: x + dx,
                y: y + dy,
                w: dw,
                h: dh,
                uv: self.uv_px(rx as f32 + sx, ry as f32 + sy, sw, sh),
                color,
                tex: 2,
                layer: 0,
                rot: 0.0,
            });
        }
        out
    }

    /// 是否存在指定精灵。
    pub fn has(&self, name: &str) -> bool {
        self.rect(name).is_some()
    }
}
