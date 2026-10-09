//! 布局原语：线性排列（对照 26.1 `net/minecraft/client/gui/layouts/*`）。
//!
//! 原版体系：`Layout`（layouts/Layout.java:4-20，arrangeElements/
//! visitChildren/尺寸查询）→ `AbstractLayout`（AbstractLayout.java:65-85，
//! 子项尺寸含 padding、`setX/setY` 按 `lerp(align, padStart,
//! avail-size-padEnd)` 在槽内定位）→ `GridLayout`（GridLayout.java:22-91，
//! 行列宽度取各列最大、偏移前缀和 + spacing）→ `LinearLayout`
//! （LinearLayout.java:6-113，单行/单列 GridLayout 的方向包装）。
//!
//! 本移植取 `LinearLayout` 常用面（原版标题屏/选项屏的堆叠骨架）：
//! - 方向 horizontal/vertical（LinearLayout.java:93-95 Orientation）；
//! - spacing（:20-23 → GridLayout columnSpacing/rowSpacing，:97-105）；
//! - 单元格 padding + 交叉轴对齐（LayoutSettings.java:64-113 字段与
//!   builder；AbstractLayout.java:73-85 定位公式）；
//! - spacer 占位（SpacerElement.java:17-30 `width()/height()`）。
//!
//! 与原版差异（有意为之，见各测试）：坐标为 **f32 逻辑像素**（原版 int
//! 像素，AbstractLayout.java:76/:83 取整；本框架统一 f32，像素对齐交给
//! 渲染管线）；子项以「固定尺寸 + CellSettings」登记，arrange 返回计算
//! 出的 [`Rect`] 表——控件层（widget/slot）自持矩形，无需实现原版
//! LayoutElement 的 setX/setY 可变协议。

use crate::context::Rect;

/// 交叉轴对齐档位（LayoutSettings.align：0=起点，0.5=居中，1=终点；
/// alignHorizontallyCenter :34-36 等 default 方法对应常量）。
pub const ALIGN_START: f32 = 0.0;
pub const ALIGN_CENTER: f32 = 0.5;
pub const ALIGN_END: f32 = 1.0;

/// 单元格设置（LayoutSettings.LayoutSettingsImpl :64-113：四边 padding +
/// 归一化对齐；builder 方法与原版同名）。`x_align`/`y_align` 在主轴方向
/// 无效果（线性布局槽宽=子项宽，AbstractLayout.java:75 的 most==least）。
/// `Default` = `LayoutSettings.defaults()`（LayoutSettings.java:61-63）：
/// 零内边距、起点（左上，0.0）对齐。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellSettings {
    pub pad_left: f32,
    pub pad_top: f32,
    pub pad_right: f32,
    pub pad_bottom: f32,
    pub x_align: f32,
    pub y_align: f32,
}

impl CellSettings {
    /// padding(int)（LayoutSettings.java:69-71）。
    pub fn padding(mut self, p: f32) -> Self {
        self.pad_left = p;
        self.pad_right = p;
        self.pad_top = p;
        self.pad_bottom = p;
        self
    }

    /// paddingHorizontal（:89-91）。
    pub fn padding_h(mut self, p: f32) -> Self {
        self.pad_left = p;
        self.pad_right = p;
        self
    }

    /// paddingVertical（:93-96）。
    pub fn padding_v(mut self, p: f32) -> Self {
        self.pad_top = p;
        self.pad_bottom = p;
        self
    }

    /// align(x, y)（:98-102）。
    pub fn align(mut self, x: f32, y: f32) -> Self {
        self.x_align = x;
        self.y_align = y;
        self
    }

    /// alignHorizontallyCenter（:34-36）。
    pub fn align_center_x(mut self) -> Self {
        self.x_align = ALIGN_CENTER;
        self
    }

    /// alignVerticallyMiddle（:43-45）。
    pub fn align_center_y(mut self) -> Self {
        self.y_align = ALIGN_CENTER;
        self
    }
}

/// 排列方向（LinearLayout.Orientation :93-112）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// 已登记子项：固定尺寸 + 单元格设置；arrange 后得到矩形。
#[derive(Clone, Copy, Debug)]
struct Child {
    w: f32,
    h: f32,
    settings: CellSettings,
}

/// 线性布局（LinearLayout 移植）。用法：
///
/// ```text
/// let mut col = LinearLayout::vertical().spacing(4.0);
/// col.add(200.0, 20.0, CellSettings::default());      // 按钮位 1
/// col.add(200.0, 20.0, CellSettings::default());      // 按钮位 2
/// let rects = col.arrange(cx - 100.0, 30.0);          // 整栈左上角
/// // col.width()/height() → 可再用于整栈居中
/// ```
pub struct LinearLayout {
    orientation: Orientation,
    spacing: f32,
    children: Vec<Child>,
    /// arrange 后的整体尺寸（GridLayout.java:89-90 的 width/height）。
    width: f32,
    height: f32,
}

impl LinearLayout {
    /// LinearLayout.horizontal()（LinearLayout.java:89-91）。
    pub fn horizontal() -> Self {
        Self::with_orientation(Orientation::Horizontal)
    }

    /// LinearLayout.vertical()（:85-87）。
    pub fn vertical() -> Self {
        Self::with_orientation(Orientation::Vertical)
    }

    fn with_orientation(orientation: Orientation) -> Self {
        Self {
            orientation,
            spacing: 0.0,
            children: Vec::new(),
            width: 0.0,
            height: 0.0,
        }
    }

    /// spacing(int)（:20-23；水平=列距、垂直=行距，Orientation.setSpacing
    /// :97-105）。
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn len(&self) -> usize {
        self.children.len()
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// addChild(child)（:37-39；尺寸即控件的 w/h，settings 见
    /// [`CellSettings`]）。
    pub fn add(&mut self, w: f32, h: f32, settings: CellSettings) {
        self.children.push(Child { w, h, settings });
    }

    /// SpacerElement.width/height（SpacerElement.java:22-27）：占位块，
    /// 常用于推开两端（如原版底部按钮行的左右留白）。
    pub fn add_spacer(&mut self, w: f32, h: f32) {
        self.add(w, h, CellSettings::default());
    }

    /// arrangeElements（Layout.java:11 → GridLayout.arrangeElements
    /// :22-91 的单向特化）：以 (x, y) 为整栈左上角排布，返回每个子项的
    /// 矩形（**控件像素矩形，不含 padding**，对齐/padding 已计入坐标）。
    ///
    /// 主轴：槽尺寸 = 子项尺寸 + 主轴 padding（单行网格中
    /// maxColumnWidths 即各项自身，GridLayout.java:37-50 特化）；
    /// 交叉轴：整行厚度 = max(子项交叉尺寸 + 交叉 padding)（:39-50），
    /// 子项按 `lerp(align, padStart, thick - size - padEnd)` 在厚度带内
    /// 定位（AbstractLayout.java:73-85）。
    pub fn arrange(&mut self, x: f32, y: f32) -> Vec<Rect> {
        let vertical = self.orientation == Orientation::Vertical;
        // 交叉轴厚度 = max(child cross + padding)（AbstractChildWrapper
        // .getWidth/getHeight 含 padding，AbstractLayout.java:65-71）
        let cross = |c: &Child| -> f32 {
            if vertical {
                c.w + c.settings.pad_left + c.settings.pad_right
            } else {
                c.h + c.settings.pad_top + c.settings.pad_bottom
            }
        };
        let cross_total = self.children.iter().map(cross).fold(0.0f32, f32::max);

        let mut out = Vec::with_capacity(self.children.len());
        let mut cursor = 0.0f32;
        for c in &self.children {
            let pad_main_start = if vertical {
                c.settings.pad_top
            } else {
                c.settings.pad_left
            };
            let pad_main_end = if vertical {
                c.settings.pad_bottom
            } else {
                c.settings.pad_right
            };
            let pad_cross_start = if vertical {
                c.settings.pad_left
            } else {
                c.settings.pad_top
            };
            let pad_cross_end = if vertical {
                c.settings.pad_right
            } else {
                c.settings.pad_bottom
            };
            // 交叉轴定位（AbstractChildWrapper.setX/setY 的 lerp 公式，
            // AbstractLayout.java:73-85；主轴对齐 most==least，退化为起点）
            let align = if vertical {
                c.settings.x_align
            } else {
                c.settings.y_align
            };
            let least = pad_cross_start;
            let size_cross = if vertical { c.w } else { c.h };
            let most = (cross_total - size_cross - pad_cross_end).max(least);
            let cross_off = least + (most - least) * align;

            let (rx, ry) = if vertical {
                (x + cross_off, y + cursor + pad_main_start)
            } else {
                (x + cursor + pad_main_start, y + cross_off)
            };
            out.push(Rect::new(rx, ry, c.w, c.h));
            cursor +=
                (if vertical { c.h } else { c.w }) + pad_main_start + pad_main_end + self.spacing;
        }
        // 整体尺寸：主轴 = 末项终点（去掉尾间距），交叉 = 厚度
        // （GridLayout.java:89-90 width/height 语义）。
        let main_total = if self.children.is_empty() {
            0.0
        } else {
            cursor - self.spacing
        };
        let (cross_main, cross_side) = if vertical {
            (cross_total, main_total)
        } else {
            (main_total, cross_total)
        };
        self.width = cross_main;
        self.height = cross_side;
        out
    }

    /// 上次 arrange 的整体宽（Layout.getWidth，LinearLayout.java:55-58）。
    pub fn width(&self) -> f32 {
        self.width
    }

    /// 上次 arrange 的整体高（Layout.getHeight，:60-63）。
    pub fn height(&self) -> f32 {
        self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_stacks_with_spacing() {
        // 原版选项屏按钮栈骨架：两枚 200x20 按钮 + 4px 行距
        let mut col = LinearLayout::vertical().spacing(4.0);
        col.add(200.0, 20.0, CellSettings::default());
        col.add(200.0, 20.0, CellSettings::default());
        let rects = col.arrange(10.0, 30.0);
        assert_eq!(rects[0], Rect::new(10.0, 30.0, 200.0, 20.0));
        assert_eq!(rects[1], Rect::new(10.0, 54.0, 200.0, 20.0)); // 30+20+4
        assert_eq!((col.width(), col.height()), (200.0, 44.0));
    }

    #[test]
    fn horizontal_lays_left_to_right() {
        let mut row = LinearLayout::horizontal().spacing(2.0);
        row.add(60.0, 20.0, CellSettings::default());
        row.add(60.0, 20.0, CellSettings::default());
        let rects = row.arrange(0.0, 100.0);
        assert_eq!(rects[1], Rect::new(62.0, 100.0, 60.0, 20.0));
        assert_eq!((col_w(&row), col_h(&row)), (122.0, 20.0));
    }

    fn col_w(l: &LinearLayout) -> f32 {
        l.width()
    }
    fn col_h(l: &LinearLayout) -> f32 {
        l.height()
    }

    #[test]
    fn spacer_pushes_items_apart() {
        // SpacerElement.width（SpacerElement.java:22-24）：中间 40 占位
        let mut row = LinearLayout::horizontal();
        row.add(60.0, 20.0, CellSettings::default());
        row.add_spacer(40.0, 0.0);
        row.add(60.0, 20.0, CellSettings::default());
        let rects = row.arrange(0.0, 0.0);
        assert_eq!(rects[0].x, 0.0);
        assert_eq!(rects[2].x, 100.0); // 60 + 40
        assert_eq!(row.width(), 160.0);
    }

    #[test]
    fn cross_axis_alignment() {
        // 垂直栈：窄项在厚度带（最宽项 200）内居中/靠右
        let mut col = LinearLayout::vertical();
        col.add(
            100.0,
            20.0,
            CellSettings {
                x_align: ALIGN_CENTER,
                ..Default::default()
            },
        );
        col.add(
            200.0,
            20.0,
            CellSettings {
                x_align: ALIGN_END,
                ..Default::default()
            },
        );
        let rects = col.arrange(0.0, 0.0);
        // lerp(0.5, 0, 200-100)=50（AbstractLayout.java:73-78）
        assert_eq!(rects[0].x, 50.0);
        // 200 项即厚度带本身，ALIGN_END → x=0
        assert_eq!(rects[1].x, 0.0);
        assert_eq!(col.width(), 200.0);
    }

    #[test]
    fn padding_offsets_and_thickens_band() {
        // paddingTop=3（KeyBinds 行距手法）：控件下移 3，且主轴槽含
        // padding（AbstractChildWrapper.getHeight :65-67）
        let mut col = LinearLayout::vertical();
        col.add(
            100.0,
            20.0,
            CellSettings {
                pad_top: 3.0,
                ..Default::default()
            },
        );
        col.add(100.0, 20.0, CellSettings::default());
        let rects = col.arrange(0.0, 0.0);
        assert_eq!(rects[0].y, 3.0);
        assert_eq!(rects[1].y, 23.0); // 20+3（首项槽含 pad_top）
        assert_eq!(col.height(), 43.0);
    }

    #[test]
    fn cross_padding_enters_cross_align() {
        // paddingLeft=10 + ALIGN_CENTER：least=10，most=带宽-项宽-10，
        // 居中取中点（AbstractLayout.java:74-76）
        let mut col = LinearLayout::vertical();
        col.add(
            50.0,
            10.0,
            CellSettings {
                pad_left: 10.0,
                pad_right: 10.0,
                x_align: ALIGN_CENTER,
                ..Default::default()
            },
        );
        col.add(100.0, 10.0, CellSettings::default());
        let rects = col.arrange(0.0, 0.0);
        // 带厚 = max(50+20, 100) = 100；least=10 most=100-50-10=40 → 25
        assert_eq!(rects[0].x, 25.0);
        assert_eq!(col.width(), 100.0);
    }

    #[test]
    fn empty_layout_has_zero_extent() {
        let mut col = LinearLayout::vertical().spacing(4.0);
        let rects = col.arrange(5.0, 5.0);
        assert!(rects.is_empty());
        assert!(col.is_empty());
        assert_eq!((col.width(), col.height()), (0.0, 0.0));
    }

    #[test]
    fn horizontal_vertical_align_bottom() {
        // 行内矮项贴底（y_align=END，高带宽 20：10 高项 y=+10）
        let mut row = LinearLayout::horizontal();
        row.add(
            30.0,
            10.0,
            CellSettings {
                y_align: ALIGN_END,
                ..Default::default()
            },
        );
        row.add(30.0, 20.0, CellSettings::default());
        let rects = row.arrange(0.0, 0.0);
        assert_eq!(rects[0].y, 10.0);
        assert_eq!(row.height(), 20.0);
    }
}
