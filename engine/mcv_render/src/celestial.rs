//! 天体贴图（太阳/月相）：读原版 `textures/environment/celestial/**` 拼
//! 9 层 32x32 纹理数组（层 0 = 太阳，层 1..9 = MoonPhase 序八相）。
//!
//! 原版 26.1 依据（反编译源码核对）：
//! - 太阳/月亮是**贴图四边形**，非程序化圆盘：太阳 quad 取 celestials
//!   图集的 SUN_SPRITE（SkyRenderer.java:125-127），月亮 8 个 quad 逐
//!   MoonPhase 取 `moon/<serializedName>` 精灵（SkyRenderer.java:149-157）。
//!   注意 26.1 月相是**每相一张独立文件**（moon/full_moon.png 等 8 张），
//!   不是旧版的单张 moon_phases.png 八相图集——图集源目录即
//!   `environment/celestial`（AtlasProvider.java:162 + celestials.json
//!   directory source）。
//! - 月相名与序：MoonPhase.java FULL_MOON(0)→WANING_GIBBOUS(7)，时间映射
//!   为 192000 tick（8 游戏日）周期逐日一相（data/minecraft/timeline/
//!   moon.json visual/moon_phase 关键帧：0/24000/.../168000）。

use std::path::Path;

/// 纹理数组层数：1 太阳 + 8 月相。
pub const CELESTIAL_LAYERS: usize = 9;
/// 每层边长（原版 sun.png / moon/*.png 均 32x32）。
pub const CELESTIAL_PX: usize = 32;
/// 太阳层数组层号。
pub const SUN_LAYER: u32 = 0;
/// 月相 p（MoonPhase.index()）的数组层号。
pub const MOON_LAYER_BASE: u32 = 1;

/// MoonPhase.java 枚举序的月相文件名（textures/environment/celestial/moon/ 下）。
const MOON_FILES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];

/// 游戏时间 → MoonPhase 序号 0..=7。26.1 月相周期 192000 tick、每日推进
/// 一相（timeline/moon.json `period_ticks` + visual/moon_phase 关键帧序）。
pub fn moon_phase(time_ticks: u64) -> u32 {
    ((time_ticks / 24_000) % 8) as u32
}

/// 近邻重采样 RGBA 到 32x32（源小于 32 时留右/下透明边，等比采样）。
fn resample_to_layer(src: &[u8], sw: u32, sh: u32, layer: usize, dst: &mut [u8]) {
    let w = CELESTIAL_PX as u32;
    for y in 0..w {
        for x in 0..w {
            let sx = (u64::from(x) * u64::from(sw) / u64::from(w)) as u32;
            let sy = (u64::from(y) * u64::from(sh) / u64::from(w)) as u32;
            let s = ((sy * sw + sx) * 4) as usize;
            let d = layer * CELESTIAL_PX * CELESTIAL_PX * 4 + ((y * w + x) * 4) as usize;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

fn decode_layer(bytes: &[u8], path: &Path, layer: usize, payload: &mut [u8]) -> bool {
    let Ok(img) = image::load_from_memory(bytes) else {
        log::warn!("celestial: decode failed: {}", path.display());
        return false;
    };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return false;
    }
    resample_to_layer(rgba.as_raw(), w, h, layer, payload);
    true
}

/// 读原版天体贴图拼 mip0 载荷（9 层 32x32 RGBA 连续）。
/// 任一文件缺失/解码失败返回 None（调用方回退程序化天体圆盘——仅限
/// 无素材部署模式；正常部署素材在仓库内必命中）。
pub fn load_payload(assets_dir: &Path) -> Option<Vec<u8>> {
    let root = assets_dir.join("textures/environment/celestial");
    let mut payload = vec![0u8; CELESTIAL_LAYERS * CELESTIAL_PX * CELESTIAL_PX * 4];
    let sun = std::fs::read(root.join("sun.png")).ok()?;
    // decode_layer 返回 bool（成功与否），失败即整体放弃（回退程序化天体）。
    if !decode_layer(
        &sun,
        &root.join("sun.png"),
        SUN_LAYER as usize,
        &mut payload,
    ) {
        return None;
    }
    for (i, name) in MOON_FILES.iter().enumerate() {
        let p = root.join("moon").join(format!("{name}.png"));
        let bytes = std::fs::read(&p).ok()?;
        if !decode_layer(&bytes, &p, MOON_LAYER_BASE as usize + i, &mut payload) {
            return None;
        }
    }
    Some(payload)
}
