//! 字体图集:MC 原版 ascii.png(`assets/minecraft/textures/font/ascii.png`)。
//! 素材红线(2026-10 任务 #53):**无程序化字体回退**——旧版公共域 8x8
//! 位图字体(FONT8X8_BASIC)已删除;ascii.png 缺失/解码失败时 [`load_atlas`]
//! 返回 Err,调用方 log::error 后以全透明纹理占位(HUD 文字整体不上屏),
//! 绝不画假字形。布局与原版一致:16x16 格、每格 8x8、格序号 = codepoint
//! (MC 映射:行 = cp >> 4,列 = cp & 0xF)。
//! MC 参照:BitmapProvider / SpaceProvider(advances: 空格 = 4,
//! 其余 advance = 墨迹宽 + 1)。

use std::sync::OnceLock;

/// 字体纹理尺寸:16x16 格,每格 8x8(与 MC ascii.png 一致)。
pub const TEX_W: usize = 128;
pub const TEX_H: usize = 128;
pub const CELL: usize = 8;
pub const GRID: usize = 16;

/// 保留的纯白格(HUD 实心矩形用)。ascii.png 该格为空,加载后覆盖。
pub const SOLID_CELL: u32 = 127;

/// 格子的 UV 矩形(字体纹理空间 0..1)。MC 映射:行 = g >> 4,列 = g & 15。
pub fn glyph_uv(g: u32) -> [[f32; 2]; 2] {
    let col = (g & 15) as f32;
    let row = (g >> 4) as f32;
    [
        [col / GRID as f32, row / GRID as f32],
        [(col + 1.0) / GRID as f32, (row + 1.0) / GRID as f32],
    ]
}

/// 在 128x128 RGBA 图集上把 SOLID_CELL 格刷成不透明白。
fn paint_solid_cell(data: &mut [u8]) {
    let col = SOLID_CELL & 15;
    let row = SOLID_CELL >> 4;
    for py in 0..CELL as u32 {
        for px in 0..CELL as u32 {
            let x = col * CELL as u32 + px;
            let y = row * CELL as u32 + py;
            let o = ((y as usize) * TEX_W + x as usize) * 4;
            data[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
}

/// 按 MC BitmapProvider.getActualGlyphWidth 逐格量墨迹宽:
/// 从右往左找第一个不透明像素列,advance = 该列 + 1 + 1(墨迹宽 + 1)。
/// 空格按 SpaceProvider 固定 4;控制符与实心格不给宽度。
pub fn glyph_widths(data: &[u8]) -> [u8; 256] {
    let mut w = [0u8; 256];
    for cell in 0..256u32 {
        if cell < 32 || cell == SOLID_CELL {
            continue;
        }
        let ox = (cell & 15) * CELL as u32;
        let oy = (cell >> 4) * CELL as u32;
        let mut ink = 0u8;
        'col: for col in (0..CELL as u32).rev() {
            for row in 0..CELL as u32 {
                let x = ox + col;
                let y = oy + row;
                let a = data[((y as usize) * TEX_W + x as usize) * 4 + 3];
                if a != 0 {
                    ink = col as u8 + 1;
                    break 'col;
                }
            }
        }
        w[cell as usize] = if cell == b' ' as u32 { 4 } else { ink + 1 };
    }
    w
}

/// 进程级宽度表:Renderer 构造时从 ascii.png 装载。
static WIDTHS: OnceLock<[u8; 256]> = OnceLock::new();

/// 用实际字体数据(ascii.png)的宽度覆盖全局宽度表;若已被初始化则忽略。
pub fn install_widths(w: [u8; 256]) {
    let _ = WIDTHS.set(w);
}

/// codepoint 的推进宽度(字体像素单位,未乘 GUI scale)。
/// 宽度表未装载(ascii.png 缺失,文字本就不上屏)时返回 MC ascii 最大
/// 字形 advance 9 作排版保底——不伪造字形,只保证布局运算不 panic。
pub fn advance(cp: u32) -> f32 {
    let Some(w) = WIDTHS.get() else {
        return 9.0;
    };
    if (cp as usize) < 256 {
        w[cp as usize] as f32
    } else {
        w[b'?' as usize] as f32
    }
}

/// 加载字体图集:`dir/textures/font/ascii.png`(MC 原版,任意尺寸近邻
/// 重采样到 128x128)。失败返回 Err(调用方 log::error,无程序化回退)。
/// 返回 (RGBA 数据, 宽度表)。`dir` = 资源根(assets/minecraft)。
pub fn load_atlas(dir: Option<&std::path::Path>) -> Result<(Vec<u8>, [u8; 256]), String> {
    let dir = dir.ok_or_else(|| {
        "font: 资源根未提供,textures/font/ascii.png 无法加载(无程序化字体回退)".to_string()
    })?;
    // M8c：读取走 mcv_assets 统一入口（AssetError 自带完整路径）。
    let path = dir.join("textures").join("font/ascii.png");
    let bytes = mcv_assets::read_file(&path).map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&bytes)
        .map_err(|e| format!("font: ascii.png 解码失败 {}: {e}", path.display()))?;
    let rgba = img.to_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        return Err(format!("font: ascii.png 尺寸为空 {}", path.display()));
    }
    let mut data = vec![0u8; TEX_W * TEX_H * 4];
    resample(rgba.as_raw(), rgba.width(), rgba.height(), &mut data);
    paint_solid_cell(&mut data);
    let widths = glyph_widths(&data);
    log::info!("font: MC ascii.png ({}x{})", rgba.width(), rgba.height());
    Ok((data, widths))
}

/// 近邻重采样任意 RGBA → 128x128 字体图集。
fn resample(src: &[u8], sw: u32, sh: u32, dst: &mut [u8]) {
    for y in 0..TEX_H as u32 {
        for x in 0..TEX_W as u32 {
            let sx = (x * sw / TEX_W as u32) as usize;
            let sy = (y * sh / TEX_H as u32) as usize;
            let s = (sy * sw as usize + sx) * 4;
            let d = ((y as usize) * TEX_W + x as usize) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}
