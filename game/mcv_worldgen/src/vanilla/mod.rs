//! vanilla 后端 — 按 Minecraft 26.1 机制 clean-room 移植的地形编排管线：
//! NoiseRouter 式密度编排（气候样条 → 最终 density 判实心）+ MultiNoise
//! 群系选择（v1 子集）+ 双洞穴（意面 + 奶酪）+ per-群系地表规则。
//!
//! 机制出处一律指向 26.1 反编译树（/root/mc-ref/src-26.1，只对齐机制与
//! 常数，零代码搬运）；OpenSimplex2S 内核是唯一 vendored 例外（CC0-1.0）。
//! 26.1 原生 y ∈ [-64, 320)（高 384）线性映射到本世界 y ∈ [0, 256)：
//! y_mc = y_ours·1.5 − 64（`mc_y_to_ours` / `ours_y_to_mc` 集中换算）；
//! 海平面 96、总高 256 为任务拍板基准。

pub mod noise;
pub mod spline;

/// 世界高（自研 256）。
pub const SY: i32 = 256;
/// 海平面（自研 96；26.1 原生 63 → 归一位置 0.33，自研 96/256 = 0.375 保留
/// bloomcraft 基准）。
pub const SEA: i32 = 96;

/// MC y（曲线锚点空间）→ 自研 y：y_ours = (y_mc + 64)·256/384。
#[must_use]
pub const fn mc_y_to_ours(mc: f64) -> f64 {
    (mc + 64.0) * (2.0 / 3.0)
}

/// 自研 y → MC y（样条/梯度锚点空间）：y_mc = y_ours·384/256 − 64。
#[must_use]
pub const fn ours_y_to_mc(y: f64) -> f64 {
    y * 1.5 - 64.0
}

/// peaksAndValleys 折叠（NoiseRouterData.peaksAndValleys：|r| = 2/3 处出
/// +1 脊线，|r|→1 回落到 0）。
#[must_use]
pub fn peaks_and_valleys(r: f64) -> f64 {
    -3.0_f64 * ((r.abs() - (2.0 / 3.0)).abs() - (1.0 / 3.0))
}

/// squeeze 映射（Mapped::transform 的 SQUEEZE = c/2 − c³/24，保号）。
#[must_use]
pub fn squeeze(c: f64) -> f64 {
    let c = c.clamp(-1.0, 1.0);
    c / 2.0 - c * c * c / 24.0
}
