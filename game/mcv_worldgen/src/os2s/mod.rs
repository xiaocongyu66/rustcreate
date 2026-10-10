//! Vendored OpenSimplex2S（"SuperSimplex"）噪声内核 — 地形 2.0（任务板 #91）
//! 的格点噪声底层。上游：https://github.com/KdotJPG/OpenSimplex2
//! @ `4cd120d35bfc27096698de90d1bcbf4f9d359a3b`，许可 **CC0-1.0**。
//!
//! 只 vendor `rust/smooth.rs`（OS2S 本体，即用户点名内核）：
//! - `rust/fast.rs` 是另一算法（OpenSimplex2 *fast* 变体），非 OS2S 的
//!   公开变体，未引入；
//! - `rust/ffi.rs` 是 wasm/外部 FFI 壳（`extern` 导出），mcv_worldgen 消费
//!   进程内 API，无需引入，也避免无谓的 unsafe 面。
//!
//! 完整性对照（vendored 审计，2026-10-10）：官方 Java `OpenSimplex2S.java`
//! 同 commit 的公开 API 面恰为 10 个方法 —— `noise2` / `noise2_ImproveX`、
//! `noise3_ImproveXY` / `noise3_ImproveXZ` / `noise3_Fallback`、
//! `noise4_ImproveXYZ_ImproveXY` / `noise4_ImproveXYZ_ImproveXZ` /
//! `noise4_ImproveXYZ` / `noise4_ImproveXY_ImproveZW` / `noise4_Fallback`。
//! vendored 版 10 个全有；上游（Java/rust 两语言）均不存在
//! psx_psl/cellular/int 等输出变体——工单里该提法无源码实据（见汇报
//! 「派单说法 vs 源码实况」）。正确性以官方 Java 编译跑出的黄金向量
//! 锁定（`tests/os2s_parity.rs`）。
// vendored 保持上游代码原样：以下 clippy 提示均为上游源码自身的风格
//（字面量精度按算法锚点书写、位逻辑含 | 0、into_iter 在引用上、grad4
// 参数多），修写它们会偏离 vendored 基线，故整模块 allow。
#![allow(non_snake_case)]
#![allow(clippy::approx_constant)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::identity_op)]
#![allow(clippy::into_iter_on_ref)]
#![allow(clippy::too_many_arguments)]

pub mod smooth;
