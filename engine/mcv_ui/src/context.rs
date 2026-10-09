//! GUI 上下文：逻辑像素坐标 + GUI scale 缩放语义 + HudQuad 收集器。
//!
//! 机制对照（26.1）：
//! - `Window.BASE_WIDTH/BASE_HEIGHT = 320/240`（Window.java:38-39）；
//! - `Window.calculateScale`（Window.java:445-463）：scale 从 1 起，
//!   只要 `fb/(scale+1)` 宽 ≥320 且高 ≥240 就继续加档；maxScale=0
//!   表示自动（永不相等，一直加到放不下为止）；
//! - `Window.setGuiScale`（Window.java:465-472）：逻辑分辨率 =
//!   帧缓冲/scale **向上取整**（余数归大屏边）；
//! - `GuiGraphicsExtractor.enableScissor/disableScissor`
//!   （GuiGraphicsExtractor.java:147-155）+ `ScissorStack`
//!   （:1429-1459）：矩形入栈与栈顶**求交**，containsPoint 空栈恒真；
//!   本仓 HudQuad 无 GPU scissor，收集器在 CPU 侧裁剪 quad（UV 按比例
//!   截取），渲染语义一致。

use mcv_render::HudQuad;

/// 逻辑分辨率下限（Window.java:38-39）。
pub const BASE_WIDTH: i32 = 320;
pub const BASE_HEIGHT: i32 = 240;

/// 字体行高（字体像素；GuiGraphics 文本以 8px 字体为基准）。
pub const GLYPH_H: f32 = 8.0;

/// GUI 整数缩放档位（Window.calculateScale 移植）。`max_scale = 0` =
/// 自动（原版"自动"档）。
pub fn calculate_scale(fb_w: i32, fb_h: i32, max_scale: i32) -> i32 {
    let mut scale = 1;
    while scale != max_scale
        && scale < fb_w
        && scale < fb_h
        && fb_w / (scale + 1) >= BASE_WIDTH
        && fb_h / (scale + 1) >= BASE_HEIGHT
    {
        scale += 1;
    }
    scale
}

/// 逻辑边长：帧缓冲/scale 向上取整（setGuiScale 的 `> width ? width+1` 分支）。
/// 注：i32 的 div_ceil 属 unstable int_roundings，此处手写。
#[allow(clippy::manual_div_ceil)]
pub fn scaled_len(fb: i32, scale: i32) -> i32 {
    if scale <= 0 {
        return fb.max(0);
    }
    (fb + scale - 1) / scale
}

/// 轴对齐矩形（逻辑像素）。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// 交集（ScissorStack.push 的 intersection 语义；无交 = None）。
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        if x1 > x0 && y1 > y0 {
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        } else {
            None
        }
    }
}

/// GUI 上下文：当前 scale 与逻辑分辨率（每帧随窗口 resize 更新）。
#[derive(Clone, Copy, Debug)]
pub struct UiContext {
    /// 帧缓冲（物理）像素尺寸。
    pub fb_w: i32,
    pub fb_h: i32,
    /// GUI scale（整数档）。
    pub scale: i32,
    /// 逻辑分辨率（guiScaledWidth/Height）。
    pub width: i32,
    pub height: i32,
}

impl Default for UiContext {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

impl UiContext {
    pub fn new(fb_w: i32, fb_h: i32, max_scale: i32) -> Self {
        let scale = calculate_scale(fb_w, fb_h, max_scale);
        Self {
            fb_w,
            fb_h,
            scale,
            width: scaled_len(fb_w, scale),
            height: scaled_len(fb_h, scale),
        }
    }

    /// 窗口 resize：更新并返回 scale/尺寸是否变化（变化时调用方应
    /// 走 `ScreenStack::resize` → 屏幕重建布局，对照 Screen.resize）。
    pub fn update(&mut self, fb_w: i32, fb_h: i32, max_scale: i32) -> bool {
        let next = Self::new(fb_w, fb_h, max_scale);
        let changed =
            next.scale != self.scale || next.width != self.width || next.height != self.height;
        *self = next;
        changed
    }

    /// 物理 → 逻辑像素（原版 MouseHandler：xpos * guiScaled / 窗口宽）。
    pub fn to_logical(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.scale.max(1) as f32;
        (x / s, y / s)
    }
}

/// HudQuad 收集器（对照 GuiGraphicsExtractor）：文本 / 矩形 / 精灵 /
/// 物品图标，全部输出到 `quads`。基面坐标为**逻辑像素**，出队时乘
/// scale 变物理像素（HUD 管线按物理像素映射，hud.wgsl vs_h_hud）；
/// [`UiGraphics::push_raw`] 接受已换算好的物理像素 quad（精灵表产物）。
pub struct UiGraphics {
    /// 当前 GUI scale（整数档；外部按需换算）。
    pub scale: f32,
    /// 逻辑分辨率。
    pub width: i32,
    pub height: i32,
    /// 当前时刻（毫秒，调用方时钟；tooltip 悬停延迟用）。
    pub now_ms: u64,
    pub quads: Vec<HudQuad>,
    /// scissor 栈（逻辑像素；空 = 不裁剪）。
    scissor: Vec<Rect>,
}

impl UiGraphics {
    pub fn new(ctx: &UiContext, now_ms: u64) -> Self {
        Self {
            scale: ctx.scale.max(1) as f32,
            width: ctx.width,
            height: ctx.height,
            now_ms,
            quads: Vec::new(),
            scissor: Vec::new(),
        }
    }

    // ---- scissor（GuiGraphicsExtractor.java:147-155 / :1429-1459）----

    /// enableScissor：入栈并与栈顶求交。
    pub fn push_scissor(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let r = Rect::new(x, y, w, h);
        let clipped = match self.scissor.last() {
            Some(top) => top.intersect(&r).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0)),
            None => r,
        };
        self.scissor.push(clipped);
    }

    /// disableScissor（栈空为程序错误，静默忽略以保渲染不炸）。
    pub fn pop_scissor(&mut self) {
        self.scissor.pop();
    }

    /// containsPointInScissor（空栈恒真）。
    pub fn in_scissor(&self, x: f32, y: f32) -> bool {
        match self.scissor.last() {
            Some(r) => r.contains(x, y),
            None => true,
        }
    }

    // ---- 图元（逻辑像素）----

    /// 纯色矩形（fill；借用字体图集保留实心白格）。
    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let s = self.scale;
        self.emit(mcv_render::text::rect(x * s, y * s, w * s, h * s, color));
    }

    /// 单行文字（当前 GUI scale 字号，先阴影后正文）。
    pub fn text(&mut self, s: &str, x: f32, y: f32, color: [f32; 4]) {
        self.text_scaled(s, x, y, 1.0, color);
    }

    /// 居中文字（cx 为逻辑屏宽中心横坐标）。
    pub fn text_centered(&mut self, s: &str, cx: f32, y: f32, color: [f32; 4]) {
        // 逻辑宽 = 字体像素宽（mcv_render::text::text_width(s,1.0)）
        let w = mcv_render::text::text_width(s, 1.0);
        self.text(s, cx - w * 0.5, y, color);
    }

    /// 指定倍率文字（k 乘 GUI scale；行内自适应缩号用）。
    pub fn text_scaled(&mut self, s: &str, x: f32, y: f32, k: f32, color: [f32; 4]) {
        let sc = self.scale;
        for q in mcv_render::text::text_quads(s, x * sc, y * sc, sc * k, color) {
            self.emit(q);
        }
    }

    /// 物品图标（地形数组层贴图；快捷栏/槽位用）。
    pub fn item_icon(&mut self, layer: u16, x: f32, y: f32, size: f32) {
        let s = self.scale;
        let q = mcv_render::text::tile_icon(layer, x * s, y * s, size * s);
        self.emit(q);
    }

    // ---- 精灵（物理像素直通；供 SpriteSheet::sprite/nine_slice 产物）----

    /// 物理像素 quad 入队（经 scissor 裁剪）。
    pub fn push_raw(&mut self, quad: HudQuad) {
        self.emit(quad);
    }

    fn emit(&mut self, quad: HudQuad) {
        // 裁剪在物理空间做（scissor 逻辑矩形换算一次）
        let q = match self.scissor.last() {
            Some(r) => {
                let s = self.scale;
                let phys = Rect::new(r.x * s, r.y * s, r.w * s, r.h * s);
                match clip_quad(&quad, &phys) {
                    Some(c) => c,
                    None => return,
                }
            }
            None => quad,
        };
        self.quads.push(q);
    }

    /// 取走全部 quad（游戏层塞进 `Scene.hud`）。
    pub fn into_quads(self) -> Vec<HudQuad> {
        self.quads
    }
}

/// CPU 侧 scissor：quad 与裁剪矩形求交，UV 按比例截取（rot != 0 的
/// 旋转 quad 不裁剪——splash 文字无 scissor 场景）。HudQuad 无
/// Clone/Copy（渲染 crate 类型），逐字段构造。
pub fn clip_quad(q: &HudQuad, r: &Rect) -> Option<HudQuad> {
    let fresh = || HudQuad {
        x: q.x,
        y: q.y,
        w: q.w,
        h: q.h,
        uv: q.uv,
        color: q.color,
        tex: q.tex,
        layer: q.layer,
        rot: q.rot,
    };
    if q.rot != 0.0 || q.w <= 0.0 || q.h <= 0.0 {
        return Some(fresh());
    }
    let x0 = q.x.max(r.x);
    let y0 = q.y.max(r.y);
    let x1 = (q.x + q.w).min(r.x + r.w);
    let y1 = (q.y + q.h).min(r.y + r.h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let fx0 = (x0 - q.x) / q.w;
    let fx1 = (x1 - q.x) / q.w;
    let fy0 = (y0 - q.y) / q.h;
    let fy1 = (y1 - q.y) / q.h;
    let mut out = fresh();
    out.x = x0;
    out.y = y0;
    out.w = x1 - x0;
    out.h = y1 - y0;
    out.uv = [
        [
            q.uv[0][0] + (q.uv[1][0] - q.uv[0][0]) * fx0,
            q.uv[0][1] + (q.uv[1][1] - q.uv[0][1]) * fy0,
        ],
        [
            q.uv[0][0] + (q.uv[1][0] - q.uv[0][0]) * fx1,
            q.uv[0][1] + (q.uv[1][1] - q.uv[0][1]) * fy1,
        ],
    ];
    Some(out)
}

/// 测试助手：标准 640x480 (scale 2) 上下文构造的收集器。
#[cfg(test)]
pub(crate) fn test_graphics(now_ms: u64) -> UiGraphics {
    UiGraphics::new(&UiContext::new(640, 480, 0), now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_scale_matches_vanilla() {
        // 自动档（0）：原版 1080p = 4、600p = 2、240p = 1
        assert_eq!(calculate_scale(1920, 1080, 0), 4);
        assert_eq!(calculate_scale(800, 600, 0), 2);
        assert_eq!(calculate_scale(640, 480, 0), 2);
        assert_eq!(calculate_scale(320, 240, 0), 1);
        // 手动上限：选项档位封顶
        assert_eq!(calculate_scale(1920, 1080, 2), 2);
        assert_eq!(calculate_scale(1920, 1080, 1), 1);
        // 宽度先到下限（320）时停
        assert_eq!(calculate_scale(640, 2000, 0), 2); // 640/3 < 320
    }

    #[test]
    fn scaled_len_ceils() {
        assert_eq!(scaled_len(1920, 4), 480);
        assert_eq!(scaled_len(1921, 4), 481);
        assert_eq!(scaled_len(0, 4), 0);
        assert_eq!(scaled_len(100, 0), 100); // scale 0 防御
    }

    #[test]
    fn ui_context_update_detects_change() {
        let mut ctx = UiContext::new(640, 480, 0);
        assert_eq!((ctx.scale, ctx.width, ctx.height), (2, 320, 240));
        assert!(!ctx.update(640, 480, 0)); // 同 scale 同逻辑尺寸（640 是
        // scale 的倍数；非倍数宽度会因向上取整改逻辑宽，属预期）
        assert!(ctx.update(1280, 960, 0));
        assert_eq!((ctx.scale, ctx.width, ctx.height), (4, 320, 240));
        // 物理→逻辑换算
        let (lx, ly) = ctx.to_logical(10.0, 20.0);
        assert_eq!((lx, ly), (2.5, 5.0));
    }

    #[test]
    fn scissor_stack_intersects_and_pops() {
        let ctx = UiContext::new(640, 480, 0);
        let mut g = UiGraphics::new(&ctx, 0);
        assert!(g.in_scissor(0.0, 0.0)); // 空栈恒真
        g.push_scissor(10.0, 10.0, 100.0, 100.0);
        g.push_scissor(50.0, 50.0, 200.0, 200.0); // 交叠区 50..110
        assert!(g.in_scissor(60.0, 60.0));
        assert!(!g.in_scissor(30.0, 60.0)); // 外层内（10..110）、交叠外
        g.pop_scissor();
        assert!(g.in_scissor(30.0, 60.0)); // 回到外层
        assert!(g.in_scissor(105.0, 60.0));
        g.pop_scissor();
        assert!(g.in_scissor(120.0, 60.0));
        g.pop_scissor(); // 栈下溢：静默忽略
    }

    #[test]
    fn fill_is_clipped_by_scissor() {
        let ctx = UiContext::new(640, 480, 0); // scale 2
        let mut g = UiGraphics::new(&ctx, 0);
        g.push_scissor(0.0, 0.0, 10.0, 10.0);
        g.fill(5.0, 5.0, 100.0, 100.0, [1.0, 1.0, 1.0, 1.0]);
        let quads = g.into_quads();
        assert_eq!(quads.len(), 1);
        // 物理：5*2=10 起，裁到 (20-10)=10 物理 px
        assert_eq!(quads[0].x, 10.0);
        assert_eq!(quads[0].w, 10.0);
    }

    #[test]
    fn clip_quad_lerps_uv() {
        let ctx = UiContext::new(640, 480, 0);
        let mut g = UiGraphics::new(&ctx, 0);
        g.push_scissor(0.0, 0.0, 4.0, 8.0);
        // 逻辑 (0,0,8,8) 字形 quad（scale 2 → 物理 16x16），UV 0..1
        g.push_raw(HudQuad {
            x: 0.0,
            y: 0.0,
            w: 16.0,
            h: 16.0,
            uv: [[0.0, 0.0], [1.0, 1.0]],
            color: [1.0; 4],
            tex: 0,
            layer: 0,
            rot: 0.0,
        });
        let quads = g.into_quads();
        assert_eq!(quads.len(), 1);
        let q = &quads[0];
        assert_eq!((q.w, q.h), (8.0, 16.0)); // 裁掉右半（物理）
        assert!((q.uv[1][0] - 0.5).abs() < 1e-6); // u 截到 0.5
        assert!((q.uv[1][1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rect_intersect_and_contains() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        let c = a.intersect(&b).unwrap();
        assert_eq!((c.x, c.y, c.w, c.h), (5.0, 5.0, 5.0, 5.0));
        assert!(a.contains(0.0, 0.0));
        assert!(!a.contains(10.0, 0.0)); // 右开
        assert!(a.intersect(&Rect::new(20.0, 0.0, 1.0, 1.0)).is_none());
    }
}
