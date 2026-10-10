//! vanilla 后端 — 按 Minecraft 26.1 机制 clean-room 移植的地形编排管线：
//! NoiseRouter 式密度编排（气候样条 → 最终 density 判实心）+ MultiNoise
//! 群系选择（v1 子集）+ 双洞穴（意面 + 奶酪）+ per-群系地表规则。
//!
//! 机制出处一律指向 26.1 反编译树（/root/mc-ref/src-26.1，只对齐机制与
//! 常数，零代码搬运）；OpenSimplex2S 内核是唯一 vendored 例外（CC0-1.0）。
//! 26.1 原生 y ∈ [-64, 320)（高 384）线性映射到本世界 y ∈ [0, 256)：
//! y_mc = y_ours·1.5 − 64（`mc_y_to_ours` / `ours_y_to_mc` 集中换算）；
//! 海平面 96、总高 256 为任务拍板基准。

pub mod biomes;
pub mod climate;
pub mod noise;
pub mod spline;
pub mod terrain;

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

/// 通道域盐：世界种子 ^ 通道码 → 终混合（自有派生，防通道同种子）。
#[must_use]
pub const fn channel_seed(world_seed: u64, domain: u64) -> i64 {
    splitmix64(world_seed ^ domain) as i64
}

/// splitmix64 终混合。
#[must_use]
pub const fn splitmix64(x: u64) -> u64 {
    let x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// [0, 1) 网格哈希（复用 legacy 内核同源结构）。
#[must_use]
pub fn hash01(seed: u64, x: i64, y: i64, z: i64) -> f32 {
    let h = splitmix64(
        seed ^ (x as u64).wrapping_mul(0x9E37_79B1)
            ^ (y as u64).wrapping_mul(0x85EB_CA77)
            ^ (z as u64).wrapping_mul(0xC2B2_AE3D),
    );
    ((h >> 40) as f32) * (1.0_f32 / 16_777_216.0)
}

/// 一列地表的方块 y（用于统计测试；生成路径见 `terrain::generate`）。
#[must_use]
pub fn surface_block_y(world_seed: u64, block_x: i32, block_z: i32) -> i32 {
    terrain::Orchestrator::new(world_seed)
        .column_state(block_x, block_z)
        .surface_ours
}

/// 一列的 MultiNoise 群系（地表参数点，depth = 0）。
#[must_use]
pub fn biome_at(world_seed: u64, block_x: i32, block_z: i32) -> biomes::Biome {
    let sample = climate::ClimateSampler::new(world_seed).sample(block_x, block_z);
    let t = biomes::TargetPoint {
        temperature: (sample.temperature * biomes::Q as f64) as i64,
        humidity: (sample.humidity * biomes::Q as f64) as i64,
        continentalness: (sample.continents * biomes::Q as f64) as i64,
        erosion: (sample.erosion * biomes::Q as f64) as i64,
        depth: 0,
        weirdness: (sample.ridges * biomes::Q as f64) as i64,
    };
    static POINT_TABLE: std::sync::OnceLock<Vec<(biomes::ParameterPoint, biomes::Biome)>> =
        std::sync::OnceLock::new();
    let table = POINT_TABLE.get_or_init(biome_point_list);
    biomes::pick_biome(table, &t)
}

/// 群系点表（缓存包装）。
#[must_use]
pub fn biome_point_list() -> Vec<(biomes::ParameterPoint, biomes::Biome)> {
    biomes::biome_point_table()
}
