//! 合成/创造取物界面的 HUD 构建：纯函数把 [`CraftScreen`]/[`CreativePicker`] 与
//! [`Hotbar`] 渲染成 [`HudQuad`]，同时产出命中矩形表（桌面鼠标/触屏共用）。
//! 布局仿原版 InventoryScreen：18px 槽距、居中面板，下段 27 主背包接 9 快捷栏
//! （全局 0..35 = `Hotbar::slot_mut` 序）；光标手持物跟随鼠标绘制。

use mcv_item::{Hotbar, ItemKind, ItemStack};
use mcv_logic::ui::{CraftScreen, CreativePicker};
use mcv_render::gui::SpriteSheet;
use mcv_render::{HudQuad, gui_scale, text};

/// 可点击矩形（与 `menu_hot` 同思路，id 路由到点击处理）。
pub struct HotRect {
    pub id: &'static str,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// 创造页物品清单 = 物品表全部 36 种（id 即下标）。
pub const ALL_ITEMS: [u16; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35,
];

const GRID_IDS: [&str; 9] = ["g0", "g1", "g2", "g3", "g4", "g5", "g6", "g7", "g8"];
const INV_IDS: [&str; 36] = [
    "i0", "i1", "i2", "i3", "i4", "i5", "i6", "i7", "i8", "i9", "i10", "i11", "i12", "i13", "i14",
    "i15", "i16", "i17", "i18", "i19", "i20", "i21", "i22", "i23", "i24", "i25", "i26", "i27",
    "i28", "i29", "i30", "i31", "i32", "i33", "i34", "i35",
];
const CRE_IDS: [&str; 36] = [
    "c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8", "c9", "c10", "c11", "c12", "c13", "c14",
    "c15", "c16", "c17", "c18", "c19", "c20", "c21", "c22", "c23", "c24", "c25", "c26", "c27",
    "c28", "c29", "c30", "c31", "c32", "c33", "c34", "c35",
];

// vanilla container 常数：槽距 18、图标 16、面板内边距 4。
const SLOT: f32 = 18.0;
const ICON: f32 = 16.0;
const PAD: f32 = 4.0;
// 原版容器底色 #C6C6C6 / 槽内色 #8B8B8B。
const PANEL: [f32; 4] = [0.776, 0.776, 0.776, 0.95];
const SLOT_BG: [f32; 4] = [0.545, 0.545, 0.545, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// 物品图标（与 `build_hud` 快捷栏同款规则）：Block 取方块图集侧面 tile，
/// 其余取 GUI 精灵表；count>1 右下计数；损伤工具画耐久条。
fn item_quads(
    stack: &ItemStack,
    g: Option<&SpriteSheet>,
    x: f32,
    y: f32,
    size: f32,
    s: f32,
) -> Vec<HudQuad> {
    let mut q = Vec::new();
    match stack.def().kind {
        ItemKind::Block(bid) => q.push(text::tile_icon(
            mcv_core::BLOCKS[bid.0 as usize].tiles[2],
            x,
            y,
            size,
        )),
        _ => match g {
            Some(g) => q.extend(g.sprite_full(stack.def().name, x, y, size, size, WHITE)),
            None => q.push(text::rect(
                x + size * 0.15,
                y + size * 0.15,
                size * 0.7,
                size * 0.7,
                [0.55, 0.5, 0.42, 0.95],
            )),
        },
    }
    if stack.count > 1 {
        let t = stack.count.to_string();
        let tw = text::text_width(&t, s);
        q.extend(text::text_quads(
            &t,
            x + size - tw,
            y + size - 8.0 * s,
            s,
            WHITE,
        ));
    }
    if stack.damage > 0 {
        let max = stack.max_damage().max(1) as f32;
        let f = 1.0 - stack.damage as f32 / max;
        let bar = (size * 0.8 * f).max(0.0);
        q.push(text::rect(
            x + size * 0.1,
            y + size - 3.0 * s,
            size * 0.8,
            s,
            [0.0, 0.0, 0.0, 1.0],
        ));
        q.push(text::rect(
            x + size * 0.1,
            y + size - 3.0 * s,
            bar,
            s,
            [f * 0.392, f, 0.0, 1.0],
        ));
    }
    q
}

/// 一格槽：底色 + 物品 + 命中矩形（`x,y` 为槽左上角屏幕坐标）。
#[allow(clippy::too_many_arguments)]
fn slot(
    q: &mut Vec<HudQuad>,
    hot: &mut Vec<HotRect>,
    id: &'static str,
    x: f32,
    y: f32,
    s: f32,
    stack: Option<&ItemStack>,
    g: Option<&SpriteSheet>,
) {
    q.push(text::rect(x + s, y + s, ICON * s, ICON * s, SLOT_BG));
    if let Some(st) = stack.filter(|st| !st.is_empty()) {
        q.extend(item_quads(st, g, x + s, y + s, ICON * s, s));
    }
    hot.push(HotRect {
        id,
        x,
        y,
        w: SLOT * s,
        h: SLOT * s,
    });
}

/// 面板下段：27 主背包（全局 9..35）+ 9 快捷栏（全局 0..8）。
fn inv_section(
    q: &mut Vec<HudQuad>,
    hot: &mut Vec<HotRect>,
    hb: &Hotbar,
    px: f32,
    inv_y: f32,
    s: f32,
    g: Option<&SpriteSheet>,
) {
    for (i, st) in hb.main.iter().enumerate() {
        let x = px + PAD * s + (i % 9) as f32 * SLOT * s;
        let y = inv_y + (i / 9) as f32 * SLOT * s;
        slot(q, hot, INV_IDS[9 + i], x, y, s, Some(st), g);
    }
    let hb_y = inv_y + 3.0 * SLOT * s + 6.0 * s;
    for (i, st) in hb.slots.iter().enumerate() {
        let x = px + PAD * s + i as f32 * SLOT * s;
        slot(q, hot, INV_IDS[i], x, hb_y, s, Some(st), g);
    }
}

/// 关屏按钮（触屏可点；桌面 Esc 同样能关）。返回矩形。
fn close_button(q: &mut Vec<HudQuad>, hot: &mut Vec<HotRect>, px: f32, py: f32, pw: f32, s: f32) {
    let bs = 12.0 * s;
    let (bx, by) = (px + pw - PAD * s - bs, py + PAD * s);
    q.push(text::rect(bx, by, bs, bs, [0.55, 0.18, 0.15, 1.0]));
    q.extend(text::text_quads("x", bx + 3.0 * s, by + 2.0 * s, s, WHITE));
    hot.push(HotRect {
        id: "close",
        x: bx,
        y: by,
        w: bs,
        h: bs,
    });
}

/// 背包面板总高（主背包 3 行 + 间距 + 快捷栏 1 行）。
fn inv_height() -> f32 {
    3.0 * SLOT + 6.0 + SLOT
}

/// 合成界面（随身 2x2 / 工作台 3x3）：网格 + 结果槽 + 36 格背包。
pub fn craft_quads(
    craft: &CraftScreen,
    hb: &Hotbar,
    g: Option<&SpriteSheet>,
    w: f32,
    h: f32,
    pointer: Option<(f32, f32)>,
) -> (Vec<HudQuad>, Vec<HotRect>) {
    let s = gui_scale(h);
    let (mut q, mut hot) = (Vec::new(), Vec::new());
    let cw = craft.width;
    let craft_h = cw as f32 * SLOT;
    let pw = (9.0 * SLOT + PAD * 2.0) * s;
    let ph = (PAD * 2.0 + craft_h + 10.0 + inv_height()) * s;
    let (px, py) = ((w - pw) * 0.5, (h - ph) * 0.5);
    q.push(text::rect(px, py, pw, ph, PANEL));
    q.extend(text::text_quads_centered(
        "Craft",
        w * 0.5,
        py - 12.0 * s,
        s,
        WHITE,
    ));
    close_button(&mut q, &mut hot, px, py, pw, s);
    // 合成网格
    for (i, gid) in GRID_IDS.iter().enumerate().take(cw * cw) {
        let x = px + PAD * s + (i % cw) as f32 * SLOT * s;
        let y = py + PAD * s + (i / cw) as f32 * SLOT * s;
        slot(&mut q, &mut hot, gid, x, y, s, craft.grid.get(i), g);
    }
    // 结果槽 + 箭头
    let rx = px + pw - PAD * s - SLOT * s;
    let ry = py + PAD * s + (craft_h - SLOT) * 0.5 * s;
    let res = craft.result();
    slot(&mut q, &mut hot, "res", rx, ry, s, res.as_ref(), g);
    q.extend(text::text_quads(
        ">",
        rx - 12.0 * s,
        ry + 4.0 * s,
        s,
        [0.15, 0.15, 0.15, 1.0],
    ));
    // 背包段
    let inv_y = py + PAD * s + (craft_h + 10.0) * s;
    inv_section(&mut q, &mut hot, hb, px, inv_y, s, g);
    // 光标手持物跟随鼠标
    if !craft.cursor.is_empty()
        && let Some((mx, my)) = pointer
    {
        q.extend(item_quads(
            &craft.cursor,
            g,
            mx - ICON * s * 0.5,
            my - ICON * s * 0.5,
            ICON * s,
            s,
        ));
    }
    (q, hot)
}

/// 创造取物界面：4x9 物品页 + 翻页 + 36 格背包。
pub fn creative_quads(
    cre: &CreativePicker,
    hb: &Hotbar,
    g: Option<&SpriteSheet>,
    w: f32,
    h: f32,
) -> (Vec<HudQuad>, Vec<HotRect>) {
    let s = gui_scale(h);
    let (mut q, mut hot) = (Vec::new(), Vec::new());
    let picker_h = 4.0 * SLOT;
    let pw = (9.0 * SLOT + PAD * 2.0) * s;
    let ph = (PAD * 2.0 + picker_h + 20.0 + 6.0 + inv_height()) * s;
    let (px, py) = ((w - pw) * 0.5, (h - ph) * 0.5);
    q.push(text::rect(px, py, pw, ph, PANEL));
    q.extend(text::text_quads_centered(
        "Creative",
        w * 0.5,
        py - 12.0 * s,
        s,
        WHITE,
    ));
    close_button(&mut q, &mut hot, px, py, pw, s);
    // 物品页（4x9）：icon 借单件假堆渲染，点击直接得物（无光标持物）。
    for (k, &item) in cre.page_items(&ALL_ITEMS).iter().enumerate() {
        let x = px + PAD * s + (k % 9) as f32 * SLOT * s;
        let y = py + PAD * s + (k / 9) as f32 * SLOT * s;
        let fake = ItemStack::new(item, 1);
        slot(&mut q, &mut hot, CRE_IDS[k], x, y, s, Some(&fake), g);
    }
    // 翻页行：< page/total >
    let row_y = py + PAD * s + picker_h * s + 4.0 * s;
    let bw = 20.0 * s;
    let bh = 14.0 * s;
    for (id, bx, label) in [
        ("prev", px + PAD * s, "<"),
        ("next", px + pw - PAD * s - bw, ">"),
    ] {
        q.push(text::rect(bx, row_y, bw, bh, SLOT_BG));
        q.extend(text::text_quads(
            label,
            bx + 7.0 * s,
            row_y + 3.0 * s,
            s,
            [0.15, 0.15, 0.15, 1.0],
        ));
        hot.push(HotRect {
            id,
            x: bx,
            y: row_y,
            w: bw,
            h: bh,
        });
    }
    let page = format!("{} / {}", cre.page + 1, cre.pages(ALL_ITEMS.len()));
    q.extend(text::text_quads_centered(
        &page,
        w * 0.5,
        row_y + 3.0 * s,
        s,
        [0.15, 0.15, 0.15, 1.0],
    ));
    // 背包段
    let inv_y = py + PAD * s + (picker_h + 20.0 + 6.0) * s;
    inv_section(&mut q, &mut hot, hb, px, inv_y, s, g);
    (q, hot)
}
