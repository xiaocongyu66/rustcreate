//! MultiNoise 群系选择（v1 子集）— clean-room 对齐 26.1
//! `world/level/biome/Climate.java` 的点集取最近机制与
//! `biome/OverworldBiomeBuilder.java` 的六参数点表。
//!
//! 机制锚点（引用机制与常数 ≠ 搬运代码）：
//! - 目标点/参数点坐标量化 ×10000（Climate.QUANTIZATION_FACTOR）；
//! - 参数距离 = 目标坐标到区间 [min,max] 的间隔距离（在区间内为 0），
//!   fitness = 六参数距离平方和 + offset²（Climate.ParameterPoint.fitness），
//!   取 fitness 最小者（RTree 只是加速结构，小表下暴力求最近等价；
//!   另见 Climate.findValueBruteForce——两语言公开 API 均含暴力核）；
//! - 表观（surface 群系）在 depth 0.0 与 1.0 两个参数点各注册一份
//!   （OverworldBiomeBuilder.addSurfaceBiome）；surface 目标取地形深度
//!   depth = 0（地表/地表上方）。
//!
//! 点表形态（地形保真波 T2 重锚）：**span 平铺分区表**。旧表把每段收缩成
//! 波段中点单点——XS=2.0 的高频下靠采样密度掩盖，恢复原版 xz_scale 0.25
//! 后 Ocean 1.1%/Beach 1.9% 越带（triage ④/审计 B2：点化是结构性降级）。
//! 现按 26.1 OverworldBiomeBuilder 的段界原值平铺参数空间：
//! - 温度/湿度：5×5 段界（OBB.java:67-79，span 原值）；
//! - 大陆度：deepOcean [-1.05,-0.455]/ocean [-0.455,-0.19]/coast
//!   [-0.19,-0.11]/inland [-0.11,1.0]（OBB.java:70-79；near/mid/far 三段在
//!   10 群系子集内同 MIDDLE 矩阵，并作一段）；
//! - 侵蚀：MIDDLE_BIOMES 选择只依赖 temp/hum 下标（OBB pickMiddleBiome），
//!   内陆元组侵蚀取 FULL 段；
//! - weirdness：谷带切片 [-0.05,0.05]（OBB addValleys，RIVER 对 erosions[0..6]
//!   即全侵蚀段注册）专属河；滩/内陆取谷带外两段；
//! - ocean：每温度段注册（OBB.java:202-209 addOcean，OCEANS 表 :72-75），
//!   湿度/侵蚀/weirdness 全段（FULL_RANGE）；
//! - beach：coast 段 × 未冻结温度段 1..3（OBB.java:922-925 pickBeachBiome：
//!   0→SNOWY_BEACH、4→DESERT，本子集缺席的群系由最近点自然回落）。
//! 未收进 10 群系的气候元组（JUNGLE/BADLANDS/…）不注册、不会选出；六参数
//! 含义与取最近机制逐一保留。

/// 本世界支持的群系（10 个，任务拍板）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Biome {
    Plains,
    Forest,
    BirchForest,
    Desert,
    Savanna,
    Taiga,
    SnowyPlains,
    Ocean,
    Beach,
    River,
}

pub const ALL_BIOMES: [Biome; 10] = [
    Biome::Plains,
    Biome::Forest,
    Biome::BirchForest,
    Biome::Desert,
    Biome::Savanna,
    Biome::Taiga,
    Biome::SnowyPlains,
    Biome::Ocean,
    Biome::Beach,
    Biome::River,
];

/// 量化基数（Climate.QUANTIZATION_FACTOR = 10000）。
pub const Q: i64 = 10_000;

/// 参数区间 [min, max]（量化后）。
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub min: i64,
    pub max: i64,
}

impl Param {
    pub const fn point(v: f64) -> Self {
        let q = (v * Q as f64) as i64;
        Self { min: q, max: q }
    }

    pub const fn span(lo: f64, hi: f64) -> Self {
        Self {
            min: (lo * Q as f64) as i64,
            max: (hi * Q as f64) as i64,
        }
    }

    /// 间隔距离（Climate.Parameter.distance(target)）。
    #[must_use]
    pub fn distance(&self, target: i64) -> i64 {
        let above = target - self.max;
        if above > 0 {
            above
        } else {
            let below = self.min - target;
            below.max(0)
        }
    }
}

/// 一个群系的参数点：6 参数 + offset。
#[derive(Clone, Copy, Debug)]
pub struct ParameterPoint {
    pub temperature: Param,
    pub humidity: Param,
    pub continentalness: Param,
    pub erosion: Param,
    pub depth: Param,
    pub weirdness: Param,
    pub offset: i64,
}

/// surface 群系目标点（量化后；第 7 维恒 0——TargetPoint.toParameterArray）。
#[derive(Clone, Copy, Debug)]
pub struct TargetPoint {
    pub temperature: i64,
    pub humidity: i64,
    pub continentalness: i64,
    pub erosion: i64,
    pub depth: i64,
    pub weirdness: i64,
}

impl ParameterPoint {
    /// fitness（Climate.ParameterPoint.fitness(target)）。
    #[must_use]
    pub fn fitness(&self, t: &TargetPoint) -> i64 {
        sq(self.temperature.distance(t.temperature))
            + sq(self.humidity.distance(t.humidity))
            + sq(self.continentalness.distance(t.continentalness))
            + sq(self.erosion.distance(t.erosion))
            + sq(self.depth.distance(t.depth))
            + sq(self.weirdness.distance(t.weirdness))
            + sq(self.offset)
    }
}

#[must_use]
const fn sq(v: i64) -> i64 {
    v * v
}

// 26.1 OBB 段界（OverworldBiomeBuilder.java:67-79 逐值转写）：
// 温度 5 段 [-1,-0.45,-0.15,0.2,0.55,1]、湿度 5 段 [-1,-0.35,-0.1,0.1,0.3,1]。
const T_SPANS: [(f64, f64); 5] = [
    (-1.0, -0.45),
    (-0.45, -0.15),
    (-0.15, 0.2),
    (0.2, 0.55),
    (0.55, 1.0),
];
const H_SPANS: [(f64, f64); 5] = [
    (-1.0, -0.35),
    (-0.35, -0.1),
    (-0.1, 0.1),
    (0.1, 0.3),
    (0.3, 1.0),
];
// 大陆度/侵蚀/weirdness 段（OBB.java:70-79 + addValleys/pickBeachBiome）。
const FULL: Param = Param::span(-1.0, 1.0);
/// UNFROZEN_RANGE（OBB.java:81 = span(temperatures[1], temperatures[4])）。
const UNFROZEN: Param = Param::span(-0.45, 1.0);
/// 谷带切片（OBB addValleys 调用点：span(-0.05, 0.05)）。
const RIVER_WEIRD: Param = Param::span(-0.05, 0.05);
/// 谷带外切片（内陆/滩的 weirdness 两段；10 群系子集内不区分正负切片）。
const WEIRD_OUTSIDE: [(f64, f64); 2] = [(-1.0, -0.05), (0.05, 1.0)];
// 段内探针（单测用「注册点必中」断言的坐标，取段界中值）。
const T_MID: [f64; 5] = [-0.725, -0.3, 0.025, 0.375, 0.775];
const H_MID: [f64; 5] = [-0.675, -0.225, 0.0, 0.2, 0.65];

/// 内陆气候元组 -> 群系（MIDDLE_BIOMES[温度段][湿度段] 语义的 10 群系收窄）。
const MIDDLE: [[Option<Biome>; 5]; 5] = [
    [
        Some(Biome::SnowyPlains),
        Some(Biome::SnowyPlains),
        Some(Biome::SnowyPlains),
        None,
        Some(Biome::Taiga),
    ],
    [
        Some(Biome::Plains),
        Some(Biome::Plains),
        Some(Biome::Forest),
        Some(Biome::Taiga),
        None,
    ],
    [
        None,
        Some(Biome::Plains),
        Some(Biome::Forest),
        Some(Biome::BirchForest),
        None,
    ],
    [
        Some(Biome::Savanna),
        Some(Biome::Savanna),
        Some(Biome::Forest),
        None,
        None,
    ],
    [
        Some(Biome::Desert),
        Some(Biome::Desert),
        Some(Biome::Desert),
        Some(Biome::Desert),
        None,
    ],
];

/// 组装一个参数点。
const fn climate_point(
    temperature: Param,
    humidity: Param,
    continentalness: Param,
    erosion: Param,
    weirdness: Param,
    depth: f64,
) -> ParameterPoint {
    ParameterPoint {
        temperature,
        humidity,
        continentalness,
        erosion,
        depth: Param::point(depth),
        weirdness,
        // offset 取 26.1 inland 表的 0.01（surface 类全表同值 → 常量项；
        // 段平铺后边界带不靠 offset 仲裁）。
        offset: 100,
    }
}

/// 群系点表（span 平铺分区，见模块头注）：ocean → river → beach → MIDDLE，
/// 每元组 depth 0.0 / 1.0 各一份（addSurfaceBiome 双注册）。注册序即平局序
///（fitness 同为 offset² 时首个最小者胜）。
#[must_use]
pub fn biome_point_table() -> Vec<(ParameterPoint, Biome)> {
    let mut v = Vec::new();
    // ocean：每温度段 × deep/ocean 大陆度段（OBB.java:202-209、OCEANS :72-75、
    // deepOceanContinentalness/oceanContinentalness :70-71）。
    for &(t_lo, t_hi) in T_SPANS.iter() {
        for &(c_lo, c_hi) in [(-1.05, -0.455), (-0.455, -0.19)].iter() {
            for d in [0.0_f64, 1.0] {
                v.push((
                    climate_point(
                        Param::span(t_lo, t_hi),
                        FULL,
                        Param::span(c_lo, c_hi),
                        FULL,
                        FULL,
                        d,
                    ),
                    Biome::Ocean,
                ));
            }
        }
    }
    // river：谷带切片 × 全侵蚀段（OBB addValleys 对 erosions[0..6] 注册
    // RIVER）× coast..farInland 大陆度段 × UNFROZEN 温度段。
    for d in [0.0_f64, 1.0] {
        v.push((
            climate_point(
                UNFROZEN,
                FULL,
                Param::span(-0.19, 1.0),
                FULL,
                RIVER_WEIRD,
                d,
            ),
            Biome::River,
        ));
    }
    // beach：coast 段 × 未冻结温度段 1..3（pickBeachBiome，OBB.java:922-925）×
    // 谷带外 weirdness（谷带切片在 coast 段属 river，OBB addValleys 同点）。
    for &(t_lo, t_hi) in T_SPANS[1..4].iter() {
        for &(w_lo, w_hi) in WEIRD_OUTSIDE.iter() {
            for d in [0.0_f64, 1.0] {
                v.push((
                    climate_point(
                        Param::span(t_lo, t_hi),
                        FULL,
                        Param::span(-0.19, -0.11),
                        FULL,
                        Param::span(w_lo, w_hi),
                        d,
                    ),
                    Biome::Beach,
                ));
            }
        }
    }
    // 内陆：温度段 × 湿度段（MIDDLE 矩阵 = MIDDLE_BIOMES[ti][hi] 语义）×
    // inland 大陆度段 × 全侵蚀 × 谷带外 weirdness。
    for (ti, row) in MIDDLE.iter().enumerate() {
        for (hi, cell) in row.iter().enumerate() {
            if let Some(b) = cell {
                for &(w_lo, w_hi) in WEIRD_OUTSIDE.iter() {
                    for d in [0.0_f64, 1.0] {
                        v.push((
                            climate_point(
                                Param::span(T_SPANS[ti].0, T_SPANS[ti].1),
                                Param::span(H_SPANS[hi].0, H_SPANS[hi].1),
                                Param::span(-0.11, 1.0),
                                FULL,
                                Param::span(w_lo, w_hi),
                                d,
                            ),
                            *b,
                        ));
                    }
                }
            }
        }
    }
    v
}

/// 群系选择：取 fitness 最小者（Climate.ParameterList.findValue 语义；
/// 严格小于保持 26.1「首个最小者胜」的平局序）。
#[must_use]
pub fn pick_biome(table: &[(ParameterPoint, Biome)], t: &TargetPoint) -> Biome {
    let mut best = table[0].0.fitness(t);
    let mut want = table[0].1;
    for (p, b) in table.iter().skip(1) {
        let f = p.fitness(t);
        if f < best {
            best = f;
            want = *b;
        }
    }
    want
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 群系全部注册。
    #[test]
    fn table_covers_all_ten_biomes() {
        let table = biome_point_table();
        for b in ALL_BIOMES {
            assert!(
                table.iter().any(|(_, want)| *want == b),
                "群系 {b:?} 未注册"
            );
        }
    }

    /** 点集取最近语义：目标落在某注册 span 组内（六参数全含）时 fitness 只剩
    offset²，必中该组群系；跨段目标按区间距离仲裁。 */
    #[test]
    fn nearest_semantics_hits_exact_climate() {
        let table = biome_point_table();
        // ocean 探针：temp2 段中值 × ocean 大陆度段中值 × 谷带外 weirdness——
        // 全部落进 ocean 注册 span（Climate.java findValue 语义）。
        let ocean = TargetPoint {
            temperature: q_probe(T_MID[2]),
            humidity: q_probe(H_MID[2]),
            continentalness: q_probe(-0.3225),
            erosion: 0,
            depth: 0,
            weirdness: q_probe(-0.5),
        };
        assert_eq!(pick_biome(&table, &ocean), Biome::Ocean);
        // desert 探针：temp4 段中值 × 内陆 × 正切片。
        let desert = TargetPoint {
            temperature: q_probe(T_MID[4]),
            humidity: q_probe(H_MID[2]),
            continentalness: q_probe(0.165),
            erosion: 0,
            depth: 0,
            weirdness: q_probe(0.5),
        };
        assert_eq!(pick_biome(&table, &desert), Biome::Desert);
    }

    /** 平局序：首个最小者胜（26.1 `fitness < best` 的严格小于序）。 */
    #[test]
    fn tie_goes_to_first() {
        let table = biome_point_table();
        let t = TargetPoint {
            temperature: q_probe(T_MID[0]),
            humidity: q_probe(H_MID[0]),
            continentalness: q_probe(0.165),
            erosion: 0,
            depth: 0,
            weirdness: q_probe(0.5),
        };
        assert_eq!(pick_biome(&table, &t), Biome::SnowyPlains);
    }

    /// f64 → 量化坐标（测试探针用；与 Param::point 同式）。
    const fn q_probe(v: f64) -> i64 {
        (v * Q as f64) as i64
    }
}
