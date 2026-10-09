//! 合成/创造取物界面的 HUD 构建：纯函数把 [`CraftScreen`]/[`CreativePicker`] 与
//! [`Hotbar`] 渲染成 [`HudQuad`]，同时产出命中矩形表（桌面鼠标/触屏共用）。
//!
//! 面板与槽位布局对齐原版 26.1（反编译源码核对）：
//! - 面板 = 整幅 blit `gui/container/inventory.png`（2x2 随身）/
//!   `gui/container/crafting_table.png`（3x3 工作台）到
//!   `((w-imageWidth)/2, (h-imageHeight)/2)`（AbstractContainerScreen.java:89-90），
//!   imageWidth/imageHeight = 176/166（InventoryScreen.java:98、
//!   CraftingScreen.java:35 从 256x256 贴图取左上 176x166）。
//! - 槽距 18（AbstractCraftingMenu.java:32 `left + x*18, top + y*18`）。
//! - 2x2：网格 (98,18)、结果槽 (154,28)（InventoryMenu.java:52-53）；
//!   3x3：网格 (30,17)、结果槽 (124,35)（CraftingMenu.java:54-55）。
//! - 玩家背包：主背包 (8,84)、快捷栏 (8,84+58)（AbstractContainerMenu.java:86-90
//!   addStandardInventorySlots hotbarSeparator=4 → 3*18+4=58）。
//! - 标签：titleLabelY=6 默认（AbstractContainerScreen.java:82-83），
//!   工作台 x=29（CraftingScreen.java:22）、随身 x=97（InventoryScreen.java:29）；
//!   "Inventory" 标签 y = 166-94 = 72（:83）。颜色 0xFF404040。
//! - 面板贴图自带槽位凹槽与合成箭头：命中面板精灵时不再程序化画槽底/箭头。
//!
//! 精灵表缺失（无素材部署）时回退程序化面板（同布局纯色矩形）。
//! 创造取物界面原版用分页 tab 贴图（CreativeModeInventoryScreen），暂不
//! 对齐，保持程序化面板。

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

// 原版容器面板 imageWidth/imageHeight（AbstractContainerScreen.java:89-90 定位基准）。
const PANEL_W: f32 = 176.0;
const PANEL_H: f32 = 166.0;
const SLOT: f32 = 18.0;
const ICON: f32 = 16.0;
// 面板内格子区左上角（槽命中矩形取 18x18，含凹槽 1px 边）。
const PAD: f32 = 4.0;
// 原版容器底色 #C6C6C6 / 槽内色 #8B8B8B（仅无素材回退路径使用）。
const PANEL: [f32; 4] = [0.776, 0.776, 0.776, 0.95];
const SLOT_BG: [f32; 4] = [0.545, 0.545, 0.545, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
// 原版标签字色 -12566464 = #404040（renderLabels）。
const LABEL: [f32; 4] = [0.251, 0.251, 0.251, 1.0];

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

/// 一格槽：`x,y` 为槽左上角屏幕坐标。`with_bg`=面板贴图缺失时补程序化槽底
/// （原版槽位凹槽直接烤在面板贴图里，AbstractContainerScreen 无逐槽底绘制）。
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
    with_bg: bool,
) {
    if with_bg {
        q.push(text::rect(x + s, y + s, ICON * s, ICON * s, SLOT_BG));
    }
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

/// 面板下段：27 主背包（全局 9..35，面板内 (8,84) 起）+ 9 快捷栏
/// （全局 0..8，(8,142) 起 = 84+58，AbstractContainerMenu.java:86-90）。
#[allow(clippy::too_many_arguments)] // 原版布局参数天然多（面板原点+双区+回退开关）
fn inv_section(
    q: &mut Vec<HudQuad>,
    hot: &mut Vec<HotRect>,
    hb: &Hotbar,
    px: f32,
    py: f32,
    s: f32,
    g: Option<&SpriteSheet>,
    with_bg: bool,
) {
    for (i, st) in hb.main.iter().enumerate() {
        let x = px + (8.0 + (i % 9) as f32 * SLOT) * s;
        let y = py + (84.0 + (i / 9) as f32 * SLOT) * s;
        slot(q, hot, INV_IDS[9 + i], x, y, s, Some(st), g, with_bg);
    }
    for (i, st) in hb.slots.iter().enumerate() {
        let x = px + (8.0 + i as f32 * SLOT) * s;
        slot(
            q,
            hot,
            INV_IDS[i],
            x,
            py + 142.0 * s,
            s,
            Some(st),
            g,
            with_bg,
        );
    }
}

/// 关屏按钮（触屏可点；桌面 Esc 同样能关）。返回矩形。
/// 引擎自有控件（触屏无 Esc），贴在面板右上角空白区。
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

/// 合成界面（随身 2x2 / 工作台 3x3）：原版面板贴图 + 原版槽位坐标。
/// 网格/结果槽面板内原点：2x2 → (98,18)/(154,28)（InventoryMenu.java:52-53）；
/// 3x3 → (30,17)/(124,35)（CraftingMenu.java:54-55）。
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
    let pw = PANEL_W * s;
    let ph = PANEL_H * s;
    let (px, py) = ((w - pw) * 0.5, (h - ph) * 0.5);
    // 面板：原版整幅贴图；缺失回退程序化底色（同时补程序化槽底）。
    let panel = match cw {
        3 => ("craft_panel", (30.0, 17.0), (124.0, 35.0), "Crafting", 29.0),
        _ => ("inv_panel", (98.0, 18.0), (154.0, 28.0), "Crafting", 97.0),
    };
    let panel_sprite = g.and_then(|g| g.sprite_full(panel.0, px, py, pw, ph, WHITE));
    let with_bg = panel_sprite.is_none();
    match panel_sprite {
        Some(pq) => q.push(pq),
        None => q.push(text::rect(px, py, pw, ph, PANEL)),
    }
    // 标签：合成段标题（titleLabelY=6 默认，x 见 craft_ui 模块注释）+ 背包段。
    q.extend(text::text_quads(
        panel.3,
        px + panel.4 * s,
        py + 6.0 * s,
        s,
        LABEL,
    ));
    q.extend(text::text_quads(
        "Inventory",
        px + 8.0 * s,
        py + 72.0 * s,
        s,
        LABEL,
    ));
    close_button(&mut q, &mut hot, px, py, pw, s);
    // 合成网格（槽距 18，AbstractCraftingMenu.java:32）
    let (gx, gy) = panel.1;
    for (i, gid) in GRID_IDS.iter().enumerate().take(cw * cw) {
        let x = px + (gx + (i % cw) as f32 * SLOT) * s;
        let y = py + (gy + (i / cw) as f32 * SLOT) * s;
        slot(
            &mut q,
            &mut hot,
            gid,
            x,
            y,
            s,
            craft.grid.get(i),
            g,
            with_bg,
        );
    }
    // 结果槽（合成箭头烤在面板贴图内，仅回退时补画）
    let (rx, ry) = panel.2;
    let res = craft.result();
    slot(
        &mut q,
        &mut hot,
        "res",
        px + rx * s,
        py + ry * s,
        s,
        res.as_ref(),
        g,
        with_bg,
    );
    if with_bg {
        q.extend(text::text_quads(
            ">",
            px + (rx - 12.0) * s,
            py + (ry + 4.0) * s,
            s,
            LABEL,
        ));
    }
    // 背包段
    inv_section(&mut q, &mut hot, hb, px, py, s, g, with_bg);
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
/// 原版创造界面为 tab 贴图组合（CreativeModeInventoryScreen），保持程序化面板。
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
        slot(&mut q, &mut hot, CRE_IDS[k], x, y, s, Some(&fake), g, true);
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
    // 背包段（创造面板非原版布局，保持既有 PAD 网格）
    for (i, st) in hb.main.iter().enumerate() {
        let x = px + PAD * s + (i % 9) as f32 * SLOT * s;
        let y = py + (PAD + picker_h + 26.0) * s + (i / 9) as f32 * SLOT * s;
        slot(&mut q, &mut hot, INV_IDS[9 + i], x, y, s, Some(st), g, true);
    }
    let hb_y = py + (PAD + picker_h + 26.0) * s + 3.0 * SLOT * s + 6.0 * s;
    for (i, st) in hb.slots.iter().enumerate() {
        let x = px + PAD * s + i as f32 * SLOT * s;
        slot(&mut q, &mut hot, INV_IDS[i], x, hb_y, s, Some(st), g, true);
    }
    (q, hot)
}

/// 背包面板下段总高（主背包 3 行 + 间距 + 快捷栏 1 行；仅创造回退布局用）。
fn inv_height() -> f32 {
    3.0 * SLOT + 6.0 + SLOT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 槽命中矩形应落在原版槽位坐标上（无素材路径 g=None，布局同原版）。
    #[test]
    fn vanilla_slot_layout() {
        let hb = Hotbar::default();
        // 3x3 工作台：网格 (30,17)、结果 (124,35)、主背包 (8,84)、快捷栏 (8,142)。
        let craft = CraftScreen::new(3);
        let (w, h) = (640.0, 480.0);
        let s = gui_scale(h);
        let (_, hot) = craft_quads(&craft, &hb, None, w, h, None);
        let (pw, ph) = (PANEL_W * s, PANEL_H * s);
        let (px, py) = ((w - pw) * 0.5, (h - ph) * 0.5);
        let rect = |id: &str| hot.iter().find(|r| r.id == id).expect(id);
        let r = rect("g0");
        assert_eq!((r.x, r.y), (px + 30.0 * s, py + 17.0 * s), "网格首槽");
        let r = rect("g8");
        assert_eq!(
            (r.x, r.y),
            (px + (30.0 + 2.0 * SLOT) * s, py + (17.0 + 2.0 * SLOT) * s),
            "网格末槽（槽距 18）"
        );
        let r = rect("res");
        assert_eq!((r.x, r.y), (px + 124.0 * s, py + 35.0 * s), "结果槽");
        let r = rect("i9");
        assert_eq!((r.x, r.y), (px + 8.0 * s, py + 84.0 * s), "主背包首槽");
        let r = rect("i0");
        assert_eq!((r.x, r.y), (px + 8.0 * s, py + 142.0 * s), "快捷栏首槽");
        // 2x2 随身：网格 (98,18)、结果 (154,28)（InventoryMenu.java:52-53）。
        let craft = CraftScreen::new(2);
        let (_, hot) = craft_quads(&craft, &hb, None, w, h, None);
        let r = hot.iter().find(|r| r.id == "g0").unwrap();
        assert_eq!((r.x, r.y), (px + 98.0 * s, py + 18.0 * s));
        let r = hot.iter().find(|r| r.id == "res").unwrap();
        assert_eq!((r.x, r.y), (px + 154.0 * s, py + 28.0 * s));
    }
}
