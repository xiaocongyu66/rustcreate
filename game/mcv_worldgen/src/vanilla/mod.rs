//! vanilla 后端 — 按 Minecraft 26.1 机制 clean-room 移植的地形编排管线：
//! NoiseRouter 式密度编排（气候样条 → 最终 density 判实心）+ MultiNoise
//! 群系选择（v1 子集）+ 双洞穴（意面 + 奶酪）+ per-群系地表规则。
//!
//! 机制出处一律指向 26.1 反编译树（/root/mc-ref/src-26.1，只对齐机制与
//! 常数，零代码搬运）；OpenSimplex2S 内核是唯一 vendored 例外（CC0-1.0）。
//! v6 几何升原版：本世界 y 即 26.1 原生 y ∈ [-64, 320)（高 384、海平面
//! 63，`mcv_core::WORLD_MIN_Y`/`SEA_LEVEL` 唯一定义源），曲线锚点空间与
//! 世界空间重合——`mc_y_to_ours`/`ours_y_to_mc` 退化为恒等映射（保留
//! 函数壳，锚点表达式不引入任何世界常数，恢复「原生 63 归一」；旧
//! 256 时代的 1.5× 压缩与 96 海平面重标随之作废）。

pub mod biomes;
pub mod climate;
pub mod noise;
pub mod spline;
pub mod terrain;

pub use biomes::Biome;

/// 世界高（v6 = 384，`mcv_core::CHUNK_SY` 唯一定义源）。
pub const SY: i32 = mcv_core::CHUNK_SY as i32;
/// 海平面（v6 = 原生 63 = `mcv_core::SEA_LEVEL`；旧 96 是 256 基准自选值，
/// 其「96/256 = 0.375 vs 原生 0.33」重标随恒等映射恢复而作废）。
pub const SEA: i32 = mcv_core::SEA_LEVEL;
/// 世界竖直下界（v6 = -64）。
pub const MIN_Y: i32 = mcv_core::WORLD_MIN_Y;

/// MC y（曲线锚点空间）→ 本世界绝对 y。v6 起两空间重合，恒等映射。
#[must_use]
pub const fn mc_y_to_ours(mc: f64) -> f64 {
    mc
}

/// 本世界绝对 y → MC y（样条/梯度锚点空间）。恒等映射（见上）。
#[must_use]
pub const fn ours_y_to_mc(y: f64) -> f64 {
    y
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

/// [0, 1) 网格哈希（复用 legacy 内核同源结构）。种子取 i64：vanilla 模块的
/// 通道种子一律经 `channel_seed`（Java 世界种子为 long 语义），按位重解释
/// 进 u64 哈希域。
#[must_use]
pub fn hash01(seed: i64, x: i64, y: i64, z: i64) -> f32 {
    let h = splitmix64(
        (seed as u64)
            ^ (x as u64).wrapping_mul(0x9E37_79B1)
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
    // 通道→参数对位（26.1 RandomState.java:101-109 构造 Climate.Sampler）：
    // router.vegetation → humidity 槽位、router.ridges → weirdness 槽位；
    // ClimateSample 字段名镜像 NoiseRouter 通道（vegetation/ridges），
    // TargetPoint 字段名镜像 Climate.TargetPoint（humidity/weirdness）。
    let t = biomes::TargetPoint {
        temperature: (sample.temperature * biomes::Q as f64) as i64,
        humidity: (sample.vegetation * biomes::Q as f64) as i64,
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
