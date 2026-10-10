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
//! 点表取舍（验收报告说明）：只需覆盖 10 群系。气候元组取 26.1 实际波段
//! （OverworldBiomeBuilder 的 temperatures/humidities/erosions 段、
//!   continentalness 段带与 weirdness 切片带）的**波段中点**，即
//!   MIDDLE_BIOMES[温度段][湿度段] 语义收窄到本任务群系集：未收进 10
//!   群系的气候元组（SNOWY_TAIGA/JUNGLE/BADLANDS/…）不注册、不会选出；
//!   六参数含义与取最近机制逐一保留。

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

// 26.1 气候波段中点（OverworldBiomeBuilder 段界的中值）：
// 温度 5 段：[-1,-0.45,-0.15,0.2,0.55,1]；湿度 5 段：[-1,-0.35,-0.1,0.1,0.3,1]。
const T_MID: [Param; 5] = [
    Param::point(-0.725),
    Param::point(-0.3),
    Param::point(0.025),
    Param::point(0.375),
    Param::point(0.775),
];
const H_MID: [Param; 5] = [
    Param::point(-0.675),
    Param::point(-0.225),
    Param::point(0.0),
    Param::point(0.2),
    Param::point(0.65),
];
const FULL: Param = Param::span(-1.0, 1.0);
const MID: Param = Param::point(0.0);

// 大陆度波段中点（OverworldBiomeBuilder 段界）：
// ocean [-0.455,-0.19]、coast [-0.19,-0.11]、内陆（near..far 收窄为一点）。
const OCEAN_C: Param = Param::point(-0.3225);
const COAST_C: Param = Param::point(-0.15);
const INLAND_C: Param = Param::point(0.165);
// 侵蚀中段点（内陆元组共用的第 4 参数点）。
const E_INLAND: Param = Param::point(-0.08625);
// weirdness 切片带中点：负切片 (-1,0)、正切片 (0,1)、河流带 [-0.05,0.05]。
const WEIRD_NEG: Param = Param::point(-0.5);
const WEIRD_POS: Param = Param::point(0.5);
const RIVER_WEIRD: Param = Param::point(0.0);

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
        // offset 取 26.1 inland 表的 0.01（surface 类全表同值 → 常量项）。
        offset: 100,
    }
}

/// 群系点表：内陆按 MIDDLE 展开（温度段 × 湿度段中点，双向 weird 切片），
/// 海/滩/河为专属气候元组；每元组 depth 0.0 / 1.0 各一份。
#[must_use]
pub fn biome_point_table() -> Vec<(ParameterPoint, Biome)> {
    let mut v = Vec::new();
    for (ti, row) in MIDDLE.iter().enumerate() {
        for (hi, cell) in row.iter().enumerate() {
            if let Some(b) = cell {
                for w in [WEIRD_NEG, WEIRD_POS] {
                    v.push((
                        climate_point(T_MID[ti], H_MID[hi], INLAND_C, E_INLAND, w, 0.0),
                        *b,
                    ));
                    v.push((
                        climate_point(T_MID[ti], H_MID[hi], INLAND_C, E_INLAND, w, 1.0),
                        *b,
                    ));
                }
            }
        }
    }
    // ocean：temp2 段、cont ocean 带中点
    for w in [WEIRD_NEG, WEIRD_POS] {
        for d in [0.0_f64, 1.0] {
            v.push((
                climate_point(T_MID[2], H_MID[2], OCEAN_C, FULL, w, d),
                Biome::Ocean,
            ));
        }
    }
    // beach：temp1/temp2 段、cont coast 带中点
    for t in [1usize, 2] {
        for d in [0.0_f64, 1.0] {
            v.push((
                climate_point(T_MID[t], H_MID[2], COAST_C, FULL, MID, d),
                Biome::Beach,
            ));
        }
    }
    // river：weirdness 河流带点、侵蚀中段点、temp2 段
    for d in [0.0_f64, 1.0] {
        v.push((
            climate_point(
                T_MID[2],
                H_MID[2],
                INLAND_C,
                Param::point(0.0875),
                RIVER_WEIRD,
                d,
            ),
            Biome::River,
        ));
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

    /** 点集取最近语义：以 ocean 参数点自身为目标必得 ocean；
    以 desert 参数点为目标必得 desert（fitness=0 精确命中）。 */
    #[test]
    fn nearest_semantics_hits_exact_climate() {
        let table = biome_point_table();
        let ocean = TargetPoint {
            temperature: T_MID[2].min,
            humidity: H_MID[2].min,
            continentalness: OCEAN_C.min,
            erosion: MID.min,
            depth: 0,
            weirdness: MID.min,
        };
        assert_eq!(pick_biome(&table, &ocean), Biome::Ocean);
        let desert = TargetPoint {
            temperature: T_MID[4].min,
            humidity: H_MID[2].min,
            continentalness: INLAND_C.min,
            erosion: E_INLAND.min,
            depth: 0,
            weirdness: WEIRD_POS.min,
        };
        assert_eq!(pick_biome(&table, &desert), Biome::Desert);
    }

    /** 平局序：首个最小者胜（26.1 `fitness < best` 的严格小于序）。 */
    #[test]
    fn tie_goes_to_first() {
        let table = biome_point_table();
        let t = TargetPoint {
            temperature: T_MID[0].min,
            humidity: H_MID[0].min,
            continentalness: INLAND_C.min,
            erosion: E_INLAND.min,
            depth: 0,
            weirdness: WEIRD_POS.min,
        };
        assert_eq!(pick_biome(&table, &t), Biome::SnowyPlains);
    }
}
