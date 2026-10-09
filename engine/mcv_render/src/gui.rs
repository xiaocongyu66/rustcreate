//! MC GUI 精灵表:从单一资源根 `assets/minecraft/textures/**` 读取原版
//! 按钮/HUD 精灵/logo(路径即 jar 内原版路径),运行时拼成一张 RGBA 纹理
//! (HUD 管线 tex=2)。核心 HUD 精灵任一文件缺失/解码失败都返回 None,调用方
//! 回退程序化绘制(安卓包不带素材也不能崩);物品图标缺失只跳过该条。
//!
//! 尺寸参照(见 mc-ref/NOTES-ui.md):button.png 200x20 九宫格
//! border=3(AbstractButton + mcmeta),hotbar.png 182x22、选中 24x23、
//! 心/饥饿 9x9、准星 15x15(Gui.java),logo 逻辑 256x64 取上 44
//! (LogoRenderer.LOGO_HEIGHT)。

use crate::gpu::HudQuad;

/// 核心 HUD 精灵条目:`textures/` 下的原版路径 → 逻辑尺寸。缺一整表弃用。
const CORE_SPRITES: [(&str, &str, u32, u32); 13] = [
    ("logo", "gui/title/minecraft.png", 256, 64),
    ("button", "gui/sprites/widget/button.png", 200, 20),
    (
        "button_hl",
        "gui/sprites/widget/button_highlighted.png",
        200,
        20,
    ),
    (
        "button_dis",
        "gui/sprites/widget/button_disabled.png",
        200,
        20,
    ),
    ("hotbar", "gui/sprites/hud/hotbar.png", 182, 22),
    ("hotbar_sel", "gui/sprites/hud/hotbar_selection.png", 24, 23),
    ("crosshair", "gui/sprites/hud/crosshair.png", 15, 15),
    (
        "heart_container",
        "gui/sprites/hud/heart/container.png",
        9,
        9,
    ),
    ("heart_full", "gui/sprites/hud/heart/full.png", 9, 9),
    ("heart_half", "gui/sprites/hud/heart/half.png", 9, 9),
    ("food_empty", "gui/sprites/hud/food_empty.png", 9, 9),
    ("food_full", "gui/sprites/hud/food_full.png", 9, 9),
    ("food_half", "gui/sprites/hud/food_half.png", 9, 9),
];

/// 物品图标条目(名字 = mcv_item::ItemDef.name,快捷栏直接按名查)。
/// 与核心表不同:缺文件只跳过该条(sprite_full 返回 None,调用方回退),
/// 不拖垮整表。Block 类物品不走此表——用方块图集 tile 面。
/// lapis 原版贴图叫 lapis_lazuli.png,条目名保持物品名。
const ITEM_SPRITES: [(&str, &str, u32, u32); 26] = [
    ("stick", "item/stick.png", 16, 16),
    ("coal", "item/coal.png", 16, 16),
    ("iron_ingot", "item/iron_ingot.png", 16, 16),
    ("diamond", "item/diamond.png", 16, 16),
    ("lapis", "item/lapis_lazuli.png", 16, 16),
    ("book", "item/book.png", 16, 16),
    ("enchanted_book", "item/enchanted_book.png", 16, 16),
    ("wooden_sword", "item/wooden_sword.png", 16, 16),
    ("stone_sword", "item/stone_sword.png", 16, 16),
    ("iron_sword", "item/iron_sword.png", 16, 16),
    ("diamond_sword", "item/diamond_sword.png", 16, 16),
    ("golden_sword", "item/golden_sword.png", 16, 16),
    ("netherite_sword", "item/netherite_sword.png", 16, 16),
    ("wooden_pickaxe", "item/wooden_pickaxe.png", 16, 16),
    ("stone_pickaxe", "item/stone_pickaxe.png", 16, 16),
    ("iron_pickaxe", "item/iron_pickaxe.png", 16, 16),
    ("diamond_pickaxe", "item/diamond_pickaxe.png", 16, 16),
    ("golden_pickaxe", "item/golden_pickaxe.png", 16, 16),
    ("netherite_pickaxe", "item/netherite_pickaxe.png", 16, 16),
    ("wooden_axe", "item/wooden_axe.png", 16, 16),
    ("stone_axe", "item/stone_axe.png", 16, 16),
    ("iron_axe", "item/iron_axe.png", 16, 16),
    ("diamond_axe", "item/diamond_axe.png", 16, 16),
    ("iron_shovel", "item/iron_shovel.png", 16, 16),
    ("diamond_shovel", "item/diamond_shovel.png", 16, 16),
    ("wooden_shovel", "item/wooden_shovel.png", 16, 16),
];

/// 其他 best-effort 精灵(缺文件只跳过该条,不拖垮整表)。
/// menu_background:进世界加载画面背景(26.1 Screen.java:404-420
/// extractMenuBackgroundTexture 以 32×32 平铺全屏,LevelLoadingScreen
/// Reason.OTHER 背景的组成部分;panorama 立方渲染器本仓未做,见
/// loading_ui.rs 模块注释)。
const EXTRA_SPRITES: [(&str, &str, u32, u32); 1] =
    [("menu_background", "gui/menu_background.png", 32, 32)];

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
    /// 从资源根(`assets/minecraft`)读 `textures/` 下原版精灵。
    /// 核心 HUD 精灵缺一即 None(调用方整体回退程序化绘制);
    /// 物品图标 best-effort:缺文件只 warn 并跳过该条。
    pub fn load(dir: &std::path::Path) -> Option<Self> {
        let decode = |name: &'static str,
                      file: &str,
                      w: u32,
                      h: u32|
         -> Option<(&'static str, Vec<u8>, u32, u32)> {
            let path = dir.join("textures").join(file);
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
            Some((name, buf, w, h))
        };
        let mut loaded: Vec<(&'static str, Vec<u8>, u32, u32)> = Vec::new();
        for (name, file, w, h) in CORE_SPRITES {
            loaded.push(decode(name, file, w, h)?);
        }
        let items_before = loaded.len();
        for (name, file, w, h) in ITEM_SPRITES {
            loaded.extend(decode(name, file, w, h));
        }
        let items = loaded.len() - items_before;
        if items < ITEM_SPRITES.len() {
            log::warn!("gui: {}/{} item icons missing", items, ITEM_SPRITES.len());
        }
        for (name, file, w, h) in EXTRA_SPRITES {
            loaded.extend(decode(name, file, w, h));
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
            [x / self.w as f32, y / self.h as f32],
            [(x + w) / self.w as f32, (y + h) / self.h as f32],
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
