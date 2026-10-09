//! 进世界加载画面（26.1 `LevelLoadingScreen` 的等价布局，纯函数可测）。
//!
//! 布局依据（26.1 反编译源，GUI 坐标 = 逻辑分辨率像素，本仓按
//! `gui_scale` 换算）：
//! - 区块状态网格（`ChunkLoadStatusView` 等价）：cell 2×2、margin 0，
//!   直径 (2r+1) 格居中于屏幕中心（`extractChunksForRendering`，
//!   LevelLoadingScreen.java:122-143）；
//! - 标题 `multiplayer.downloadingTerrain`：centeredText 于 textTop；
//!   有网格时 textTop = yCenter − radius×2 − 27，否则 yCenter − 50
//!   （LevelLoadingScreen.java:101-111）；
//! - 进度条：`drawProgressBar`（:117-120）fill 程序化绘制——黑底
//!   0xFF000000 宽 200 高 2，绿条 0xFF00FF00 宽 round(progress×200)；
//!   显示条件 `loadTracker.hasProgress()`（:112-114）在本仓恒真
//!   （单进程一体，进入加载态即开始数区块，无服务端 stage 概念）；
//! - 背景：Reason.OTHER = 全景 + 模糊 + menu_background 平铺
//!   （LevelLoadingScreen.java:160-164 + Screen.java:396-420，
//!   `extractMenuBackgroundTexture` 以 32×32 平铺全屏）。本仓无全景
//!   立方渲染器（KNOWN-DIVERGENCE），回退 = 近黑底 + 原版
//!   `gui/menu_background.png` 32×32 平铺（best-effort，缺素材即纯色）。
//!
//! 网格颜色沿用原版状态色板（LevelLoadingScreen.java:37-51 `COLORS`），
//! 本仓 6 状态按管线角色取近义位：Empty→EMPTY、TerrainReady→NOISE
//! （体素已生成）、LightLocalReady→INITIALIZE_LIGHT、Lit→LIGHT、
//! MeshReady→SPAWN（原版无网格阶段，取 FULL 前一站）、Uploaded→FULL。

use mcv_render::gui::SpriteSheet;
use mcv_render::text;
use mcv_render::{HudQuad, gui_scale};

/// 加载画面数据模型（GameRuntime 侧统计 + i18n 文案，app 层组装）。
pub struct LoadScreenModel {
    /// 平滑进度 0..1（LevelLoadingScreen.java:84 每 tick lerp 0.2）。
    pub progress: f32,
    /// 状态网格视野半径（26.1 chunkStatusViewRadius = 7）。
    pub grid_radius: i32,
    /// `(dx, dz, Stage as u8)` 网格格子；`None` = 区块未加载
    /// （原版 `statusView.get` 返回 null → COLORS 缺省 0 → 黑格）。
    pub grid: Vec<(i32, i32, Option<u8>)>,
    /// 标题文案（`multiplayer.downloadingTerrain`）。
    pub title: String,
}

/// 本仓区块状态 → 原版状态色板（LevelLoadingScreen.java:39-50，ARGB
/// 去掉 alpha 后的 RGB/255）。
pub fn stage_color(stage: Option<u8>) -> [f32; 4] {
    // 26.1 COLORS: EMPTY 5526612, NOISE 13750737, INITIALIZE_LIGHT
    // 13421772, LIGHT 16769184, SPAWN 15884384, FULL 16777215。
    let rgb: [f32; 3] = match stage {
        Some(0) => [0.329412, 0.329412, 0.329412], // EMPTY    #545454
        Some(1) => [0.819608, 0.819608, 0.819608], // NOISE    #D1D1D1
        Some(2) => [0.800000, 0.800000, 0.800000], // INIT_LIGHT #CCCCCC
        Some(3) => [1.000000, 0.878431, 0.627451], // LIGHT    #FFE0A0
        Some(4) => [0.949020, 0.376471, 0.376471], // SPAWN    #F26060
        Some(5) => [1.000000, 1.000000, 1.000000], // FULL     #FFFFFF
        // 未加载：原版 get(x,z)==null → COLORS.getInt(null)=defaultReturnValue(0)
        // → ARGB.opaque(0) = 不透明黑。
        _ => [0.0, 0.0, 0.0],
    };
    [rgb[0], rgb[1], rgb[2], 1.0]
}

/// 组装加载画面 HUD quad 列表（先背景后前景，绘制序即遮挡序）。
pub fn quads(
    width: f32,
    height: f32,
    model: &LoadScreenModel,
    gui: Option<&SpriteSheet>,
) -> Vec<HudQuad> {
    let s = gui_scale(height);
    let mut q: Vec<HudQuad> = Vec::new();

    // 背景：近黑底 + menu_background.png 32×32 平铺（GUI 32px/格）。
    q.push(text::rect(0.0, 0.0, width, height, [0.08, 0.08, 0.10, 1.0]));
    if let Some(g) = gui {
        let tile = 32.0 * s;
        let mut n = 0u32;
        let mut y = 0.0;
        while y < height && n < 1400 {
            let mut x = 0.0;
            while x < width && n < 1400 {
                if let Some(t) = g.sprite_full("menu_background", x, y, tile, tile, [1.0; 4]) {
                    q.push(t);
                    n += 1;
                }
                x += tile;
            }
            y += tile;
        }
    }

    // 几何均以 GUI 单位表达再乘 s（原版常数的换算点）。
    let xc = width * 0.5;
    let yc = height * 0.5;
    let has_grid = model.grid_radius > 0 && !model.grid.is_empty();
    let text_top_gu = if has_grid {
        -(model.grid_radius as f32) * 2.0 - 27.0
    } else {
        -50.0
    };
    let text_top = yc + text_top_gu * s;

    // 区块状态网格（extractChunksForRendering：size=2、margin=0，
    // totalWidth = diameter×2，xStart = xCenter − totalWidth/2）。
    if has_grid {
        let diameter = model.grid_radius * 2 + 1;
        let start_gu = -(diameter as f32);
        for (dx, dz, stage) in &model.grid {
            let cell_x = xc + (start_gu + (*dx + model.grid_radius) as f32 * 2.0) * s;
            let cell_y = yc + (start_gu + (*dz + model.grid_radius) as f32 * 2.0) * s;
            q.push(text::rect(
                cell_x,
                cell_y,
                2.0 * s,
                2.0 * s,
                stage_color(*stage),
            ));
        }
    }

    // 标题：centeredText 于 (xCenter, textTop)，色 -1 = 白。
    q.extend(text::text_quads_centered(
        &model.title,
        xc,
        text_top,
        s,
        [1.0, 1.0, 1.0, 1.0],
    ));

    // 进度条（drawProgressBar，:117-120）：黑底 (xc−100, textTop+12,
    // 200×2)，绿条宽 round(progress×200) GUI px。
    let bar_left = xc - 100.0 * s;
    let bar_top = text_top + 12.0 * s;
    q.push(text::rect(
        bar_left,
        bar_top,
        200.0 * s,
        2.0 * s,
        [0.0, 0.0, 0.0, 1.0],
    ));
    let fg_w_gu = (model.progress.clamp(0.0, 1.0) * 200.0).round();
    q.push(text::rect(
        bar_left,
        bar_top,
        fg_w_gu * s,
        2.0 * s,
        [0.0, 1.0, 0.0, 1.0],
    ));
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(progress: f32, radius: i32, grid: Vec<(i32, i32, Option<u8>)>) -> LoadScreenModel {
        LoadScreenModel {
            progress,
            grid_radius: radius,
            grid,
            title: "Loading terrain...".into(),
        }
    }

    /// 布局缩放（h=480 → gui_scale=2，整数）。断言一律以 `gui_scale`
    /// 实测值为基准换算，不硬编码缩放比。
    fn layout(height: f32) -> f32 {
        gui_scale(height)
    }

    #[test]
    fn progress_bar_matches_vanilla_geometry() {
        let h = 480.0;
        let w = 800.0;
        let s = layout(h);
        let q = quads(
            w,
            h,
            &model(0.373, 0, Vec::new()),
            None, // 无素材：走纯色回退路径，几何不变
        );
        // 倒数第二个 = 黑底，最后一个 = 绿条。
        let bg = &q[q.len() - 2];
        let fg = &q[q.len() - 1];
        // 黑底：(xc−100, textTop+12, 200×2)；无网格 textTop = h/2−50。
        let xc = w * 0.5;
        let text_top = h * 0.5 - 50.0 * s;
        assert_eq!(bg.x, xc - 100.0 * s);
        assert_eq!(bg.y, text_top + 12.0 * s);
        assert_eq!(bg.w, 200.0 * s);
        assert_eq!(bg.h, 2.0 * s);
        assert_eq!(bg.color, [0.0, 0.0, 0.0, 1.0]);
        // 绿条宽 = round(0.373×200)=75 GUI px（Math.round 语义）。
        assert_eq!(fg.w, 75.0 * s);
        assert_eq!(fg.color, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn grid_layout_centers_and_colors() {
        let h = 480.0;
        let w = 800.0;
        let s = layout(h);
        let r = 2;
        // (dx,dz) 从 −r..=r，中心格 = Uploaded(5) 白，角落 = None 黑。
        let mut grid = Vec::new();
        for dx in -r..=r {
            for dz in -r..=r {
                let stage = if dx == 0 && dz == 0 { Some(5) } else { None };
                grid.push((dx, dz, stage));
            }
        }
        let q = quads(w, h, &model(0.0, r, grid), None);
        // 网格 25 格 + 背景 1 + 黑底 1 + 绿条 1 + 标题 quads（≥1）。
        assert!(q.len() >= 29);
        // 中心格（第 14 个 quad，前 1 个背景）：位置 = 中心 − 直径/2×s。
        let diameter = (2 * r + 1) as f32;
        let center_cell = &q[1 + 12]; // 背景(1) + 5×5 网格中心 = 第 13 格
        assert_eq!(center_cell.x, w * 0.5 - (diameter) * s + 2.0 * r as f32 * s);
        assert_eq!(
            center_cell.color,
            [1.0, 1.0, 1.0, 1.0],
            "Uploaded = FULL 白"
        );
        // 角落格 = 未加载 = 黑（ARGB.opaque(0)）。
        let corner = &q[1]; // 第一个格子 = (−r,−r)
        assert_eq!(corner.color, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn text_top_shifts_up_when_grid_present() {
        let h = 480.0;
        let w = 800.0;
        let s = layout(h);
        let no_grid = quads(w, h, &model(0.0, 0, Vec::new()), None);
        let with_grid = quads(w, h, &model(0.0, 7, vec![(0, 0, Some(0))]), None);
        // 无网格：q[1] = 标题首 quad，y = textTop = yc − 50（GUI px）。
        assert_eq!(no_grid[1].y, h * 0.5 - 50.0 * s, "无网格 textTop = yc−50");
        // 有网格：q[1] = 网格格 (0,0)：start(−15) + (0+7)×2 = −1 → yc − s；
        // 标题（q[2]）textTop = yc − radius×2 − 27 = yc − 41。
        assert_eq!(with_grid[1].y, h * 0.5 - 1.0 * s, "网格格 y = yc−1");
        assert_eq!(with_grid[2].y, h * 0.5 - 41.0 * s, "有网格 textTop = yc−41");
    }
}
