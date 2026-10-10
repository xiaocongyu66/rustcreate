//! 天体贴图（太阳/月相）：M8c 起纯 IO/解码半边下沉到
//! [`mcv_assets::celestial`]（经 AssetManager 统一读取/缓存/缺素材硬错误
//! 登记），本模块保留游戏侧时间映射 [`moon_phase`] 与原路径 re-export
//! （`mcv_render::celestial::load_payload` 等签名不变）。
//!
//! 原版 26.1 依据（反编译源码核对，详注随 mcv_assets::celestial 迁移）：
//! - 太阳/月亮是**贴图四边形**，非程序化圆盘：太阳 quad 取 celestials
//!   图集的 SUN_SPRITE（SkyRenderer.java:125-127），月亮 8 个 quad 逐
//!   MoonPhase 取 `moon/<serializedName>` 精灵（SkyRenderer.java:149-157）。
//!   26.1 月相为每相独立文件，图集目录 `environment/celestial`
//!   （AtlasProvider.java:162 + celestials.json directory source）。
//! - 月相名与序：MoonPhase.java FULL_MOON(0)→WANING_GIBBOUS(7)，时间映射
//!   为 192000 tick（8 游戏日）周期逐日一相（data/minecraft/timeline/
//!   moon.json visual/moon_phase 关键帧：0/24000/.../168000）。

pub use mcv_assets::celestial::{
    CELESTIAL_LAYERS, CELESTIAL_PX, MOON_LAYER_BASE, SUN_LAYER, load_payload, load_payload_via,
};

/// 游戏时间 → MoonPhase 序号 0..=7。26.1 月相周期 192000 tick、每日推进
/// 一相（timeline/moon.json `period_ticks` + visual/moon_phase 关键帧序）。
pub fn moon_phase(time_ticks: u64) -> u32 {
    ((time_ticks / 24_000) % 8) as u32
}
