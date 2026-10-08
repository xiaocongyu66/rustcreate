//! Unifont 位图字体(SIL OFL,cjk.f16,见 ci/gen-unifont.py):
//! 中文 UI 文本渲染。MC 26.1 同款方案 —— ascii 8x8 字体只覆盖拉丁,
//! CJK 走 unifont 16x16 位图回退(半角 8 列、全角 16 列)。
//!
//! 数据为 `crates/mcv_app/data/font/cjk.f16`(DEVELOP_ONLY,政策同
//! texturepack):索引按 codepoint 升序二分,位图 1bpp。本模块只做
//! 解析 + 排版 + quad 生成,不碰 wgpu;图集上传由 app 侧把
//! [`Unifont::atlas_rgba`] 铺成纹理/HUD 图集层,并把 quads 拼进
//! `Scene::hud`(与 [`crate::text`] 的 ascii 路径并行使用)。
//!
//! 排版规则(MC unifontprovider 惯例):全角 advance = 12(= 2×半角 6,
//! 即 ascii 字体均值宽的两倍),半角 advance = 6;缺字渲染空心框,
//! advance 按目标宽度取(全角空位 12 / 半角 8,未知按 12)。
//! ASCII(<0x80)不在本字体内,由 [`crate::text`] 渲染。

use crate::gpu::HudQuad;
use std::collections::HashMap;

/// 位图字形高度(像素,未乘 scale)。
pub const GLYPH_H: f32 = 16.0;
/// 全角字形位图宽。
pub const FULL_W: f32 = 16.0;
/// 半角字形位图宽。
pub const HALF_W: f32 = 8.0;
/// 全角推进宽度(= 2×半角,MC 规则)。
pub const FULL_ADVANCE: f32 = 12.0;
/// 半角推进宽度。
pub const HALF_ADVANCE: f32 = 6.0;
/// 图集每行字形数;图集布局 = RGBA8,(COLS × rows) 像素,格 = 16x16,
/// 行主序;最后一个格是缺字空心框。
pub const ATLAS_COLS: usize = 256;
/// HudQuad.tex 的约定取值(app 接入 unifont 纹理后按需覆写
/// [`Unifont::atlas_tex`];gpu.rs 目前只识别 0..2,3 为预留)。
pub const DEFAULT_ATLAS_TEX: u32 = 3;

const MAGIC: &[u8; 4] = b"FCF1";
const VERSION: u32 = 1;

/// 单个字形:16 行位图(MSB=最左像素,行宽 16 位;半角只用高 8 位)、
/// 是否半宽、在图集里的格号、推进宽度。
#[derive(Clone, Copy)]
pub struct Glyph {
    pub rows: [u16; 16],
    pub half: bool,
    pub cell: u32,
    pub advance: f32,
}

/// cjk.f16 解析结果。不可变查询 + `atlas_tex` 覆写。
pub struct Unifont {
    map: HashMap<u32, Glyph>,
    box_cell: u32,
    cols: usize,
    rows: usize,
    atlas: Vec<u8>,
    /// 生成的 quads 携带的 HudQuad.tex 值(见 [`DEFAULT_ATLAS_TEX`])。
    pub atlas_tex: u32,
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Unifont {
    /// 解析 cjk.f16(见 ci/gen-unifont.py 头注释的格式)。数据非法时
    /// 返回 None;调用方按“无中文字体”降级即可。
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 16 || &data[0..4] != MAGIC || u32le(data, 4) != VERSION {
            return None;
        }
        let count = u32le(data, 8) as usize;
        let half_count = u32le(data, 12) as usize;
        if half_count > count {
            return None;
        }
        let idx_end = 16 + count * 5;
        let bmp_end = idx_end + half_count * 16 + (count - half_count) * 32;
        if data.len() < bmp_end {
            return None;
        }
        let mut map = HashMap::with_capacity(count);
        // 两段各自严格升序（半宽段在前、全宽段在后是格式约定；
        // 段间不要求整体有序——半宽字形码位(U+FF61…)排在全宽(U+3000…)前）
        let mut prev_half = None::<u32>;
        let mut prev_full = None::<u32>;
        for i in 0..count {
            let at = 16 + i * 5;
            let cp = u32le(data, at);
            let half = data[at + 4] & 1 != 0;
            if half != (i < half_count) {
                return None; // 半宽段必须整体在前
            }
            let prev = if half { &mut prev_half } else { &mut prev_full };
            if prev.is_some_and(|p| cp <= p) {
                return None; // 段内严格升序(二分前提)
            }
            *prev = Some(cp);
            let mut rows = [0u16; 16];
            if half {
                let b = idx_end + i * 16;
                for (r, row) in rows.iter_mut().enumerate() {
                    *row = u16::from(data[b + r]) << 8;
                }
            } else {
                let b = idx_end + half_count * 16 + (i - half_count) * 32;
                for (r, row) in rows.iter_mut().enumerate() {
                    *row = u16::from_be_bytes([data[b + r * 2], data[b + r * 2 + 1]]);
                }
            }
            map.insert(
                cp,
                Glyph {
                    rows,
                    half,
                    cell: i as u32,
                    advance: if half { HALF_ADVANCE } else { FULL_ADVANCE },
                },
            );
        }
        // 缺字空心框占最后一格
        let box_cell = count as u32;
        let total = count + 1;
        let cols = ATLAS_COLS;
        let rows = total.div_ceil(cols);
        let mut atlas = vec![0u8; cols * rows * 16 * 4];
        for g in map.values() {
            Self::paint_cell(&mut atlas, cols, g.cell, &g.rows, g.half);
        }
        // 空心框:1px 边框 + 对角线
        let mut box_rows = [0u16; 16];
        box_rows[0] = 0xFFFF;
        box_rows[15] = 0xFFFF;
        for (r, row) in box_rows.iter_mut().enumerate() {
            *row |= 0x8001 | (1 << (15 - r)); // 左右边界 + 主对角线
        }
        Self::paint_cell(&mut atlas, cols, box_cell, &box_rows, false);
        Some(Self {
            map,
            box_cell,
            cols,
            rows,
            atlas,
            atlas_tex: DEFAULT_ATLAS_TEX,
        })
    }

    fn paint_cell(atlas: &mut [u8], cols: usize, cell: u32, rows: &[u16; 16], half: bool) {
        let cw = if half { HALF_W } else { FULL_W } as usize;
        let cx = (cell as usize % cols) * 16;
        let cy = (cell as usize / cols) * 16;
        for (r, bits) in rows.iter().enumerate() {
            for c in 0..cw {
                if bits & (0x8000 >> c) != 0 {
                    let o = ((cy + r) * cols * 16 + cx + c) * 4;
                    atlas[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
    }

    /// 字体是否收录该码位(ASCII 恒为 false:走 crate::text)。
    pub fn has(&self, cp: u32) -> bool {
        self.map.contains_key(&cp)
    }

    /// 码位推进宽度(字体像素);不在字体内返回 None。
    pub fn advance(&self, cp: u32) -> Option<f32> {
        self.map.get(&cp).map(|g| g.advance)
    }

    /// 该 cp 是否应回退到 unifont(非 ASCII 且被收录)。
    pub fn covers(&self, ch: char) -> bool {
        (ch as u32) >= 0x80 && self.has(ch as u32)
    }

    fn glyph_or_box(&self, cp: u32) -> Glyph {
        self.map.get(&cp).copied().unwrap_or(Glyph {
            rows: [0; 16],
            half: false,
            cell: self.box_cell,
            advance: FULL_ADVANCE,
        })
    }

    /// 字符串尺寸(屏幕像素,已乘 scale)。高度恒为一行。
    pub fn measure(&self, s: &str, scale: f32) -> (f32, f32) {
        let w: f32 = s
            .chars()
            .map(|c| {
                let cp = c as u32;
                if cp == b' ' as u32 {
                    HALF_ADVANCE
                } else {
                    self.glyph_or_box(cp).advance
                }
            })
            .sum();
        (w * scale, GLYPH_H * scale)
    }

    /// `measure` 的宽度分量。
    pub fn width(&self, s: &str, scale: f32) -> f32 {
        self.measure(s, scale).0
    }

    /// 整段文字的 glyph quad 列表(无阴影;16 高的中文字形加 MC 式 1px
    /// 阴影效果差,故不做)。缺字渲染空心框,不中断。
    pub fn push_quads(
        &self,
        s: &str,
        x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
        out: &mut Vec<HudQuad>,
    ) {
        let mut cx = x;
        for ch in s.chars() {
            let cp = ch as u32;
            if cp == b' ' as u32 {
                cx += HALF_ADVANCE * scale;
                continue;
            }
            let g = self.glyph_or_box(cp);
            let w = if g.half { HALF_W } else { FULL_W };
            out.push(HudQuad {
                x: cx,
                y,
                w: w * scale,
                h: GLYPH_H * scale,
                uv: self.cell_uv(g.cell),
                color,
                tex: self.atlas_tex,
                layer: 0,
                rot: 0.0,
            });
            cx += g.advance * scale;
        }
    }

    /// `push_quads` 的 Vec 版本。
    pub fn quads(&self, s: &str, x: f32, y: f32, scale: f32, color: [f32; 4]) -> Vec<HudQuad> {
        let mut out = Vec::with_capacity(s.chars().count());
        self.push_quads(s, x, y, scale, color, &mut out);
        out
    }

    /// 居中版本的 [`quads`](Self::quads)(x 为屏幕中心横坐标)。
    pub fn quads_centered(
        &self,
        s: &str,
        cx: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
    ) -> Vec<HudQuad> {
        self.quads(s, cx - self.width(s, scale) * 0.5, y, scale, color)
    }

    /// 格 UV(图集 0..1 空间)。
    pub fn cell_uv(&self, cell: u32) -> [[f32; 2]; 2] {
        let col = (cell % self.cols as u32) as f32;
        let row = (cell / self.cols as u32) as f32;
        [
            [col / self.cols as f32, row / self.rows as f32],
            [
                (col + 1.0) / self.cols as f32,
                (row + 1.0) / self.rows as f32,
            ],
        ]
    }

    /// 字形 → 格号(收录字形与缺字框)。
    pub fn cell_of(&self, cp: u32) -> u32 {
        self.glyph_or_box(cp).cell
    }

    /// 程序化图集(RGBA8 白字透明底,16x16/格,[`ATLAS_COLS`] 列行主序,
    /// 最后一格为缺字框)。上传为单独 HUD 纹理后用 quads 绘制。
    pub fn atlas_rgba(&self) -> (&[u8], usize, usize) {
        (&self.atlas, self.cols * 16, self.rows * 16)
    }
}
