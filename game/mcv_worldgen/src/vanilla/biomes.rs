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
//!
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

/// 群系气候参数（tint/积雪判据数据源，26.1 实物数值）。
///
/// 字段对位 [`net/minecraft/world/level/biome/Biome.ClimateSettings`]
/// （Biome.java:255-258：`hasPrecipitation/temperature/downfall`，JSON 顶层
/// 同名字段直载）与 26.1 新增的视觉属性
/// `minecraft:visual/sky_color`（biome JSON `attributes` 节，替代旧
/// BiomeSpecialEffects.skyColor）。用途：
/// - `temperature/downfall` → 草/叶 colormap 查色
///   （Biome.getGrassColorFromTexture/getFoliageColorFromTexture，
///   Biome.java:212-226，clamp 到 0..1 后 `ColorMapColorUtil.get`）；
/// - `temperature + has_precipitation` → 降水/积雪判据
///   （Biome.getPrecipitationAt/warmEnoughToRain：高度调整温度 < 0.15 =
///   SNOW，Biome.java:104-110,171-177；高度调整见
///   [`height_adjusted_temperature`]，噪声源 [`crate::vanilla::simplex`]）；
/// - `sky_color` → 天空色（ClientLevel 经 biome 属性取色）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BiomeClimate {
    /// JSON 顶层 `"temperature"`（基础温度；per-position 修正见
    /// [`height_adjusted_temperature`]，实物 Biome.getHeightAdjustedTemperature）。
    pub temperature: f64,
    /// JSON 顶层 `"downfall"`。
    pub downfall: f64,
    /// JSON 顶层 `"has_precipitation"`，按 10 群系 JSON 实物逐群系抄录：
    /// desert.json/savanna.json 明写 `false`，其余 8 群系 `true`
    /// （26.1 data/minecraft/worldgen/biome/ 实证）。false →
    /// getPrecipitationAt 恒 NONE（Biome.java:104-108）：无雪也无雨。
    pub has_precipitation: bool,
    /// `attributes."minecraft:visual/sky_color"` RGB。
    pub sky_color: [u8; 3],
}

/// 10 群系气候表。数值逐条抄自 26.1 数据文件（file:line 注于各分支），
/// 数据事实非代码表达，可入库（版权红线：不搬原版代码）。
/// #181 群系生成化波若把本表改为脚本产出，数值与引注随表迁移。
#[must_use]
pub const fn climate(b: Biome) -> BiomeClimate {
    match b {
        // data/minecraft/worldgen/biome/plains.json:200/10/3
        Biome::Plains => BiomeClimate {
            temperature: 0.8,
            downfall: 0.4,
            has_precipitation: true,
            sky_color: [0x78, 0xA7, 0xFF],
        },
        // forest.json:195/17/10
        Biome::Forest => BiomeClimate {
            temperature: 0.7,
            downfall: 0.8,
            has_precipitation: true,
            sky_color: [0x79, 0xA6, 0xFF],
        },
        // birch_forest.json:190/17/10
        Biome::BirchForest => BiomeClimate {
            temperature: 0.6,
            downfall: 0.6,
            has_precipitation: true,
            sky_color: [0x7A, 0xA5, 0xFF],
        },
        // desert.json:193/18/11，"has_precipitation": false
        Biome::Desert => BiomeClimate {
            temperature: 2.0,
            downfall: 0.0,
            has_precipitation: false,
            sky_color: [0x6E, 0xB1, 0xFF],
        },
        // savanna.json:206/11/4，"has_precipitation": false
        Biome::Savanna => BiomeClimate {
            temperature: 2.0,
            downfall: 0.0,
            has_precipitation: false,
            sky_color: [0x6E, 0xB1, 0xFF],
        },
        // taiga.json:200/10/3
        Biome::Taiga => BiomeClimate {
            temperature: 0.25,
            downfall: 0.8,
            has_precipitation: true,
            sky_color: [0x7D, 0xA3, 0xFF],
        },
        // snowy_plains.json:181/11/3
        Biome::SnowyPlains => BiomeClimate {
            temperature: 0.0,
            downfall: 0.5,
            has_precipitation: true,
            sky_color: [0x7F, 0xA1, 0xFF],
        },
        // ocean.json:206/27/20
        Biome::Ocean => BiomeClimate {
            temperature: 0.5,
            downfall: 0.5,
            has_precipitation: true,
            sky_color: [0x7B, 0xA4, 0xFF],
        },
        // beach.json:161/10/3
        Biome::Beach => BiomeClimate {
            temperature: 0.8,
            downfall: 0.4,
            has_precipitation: true,
            sky_color: [0x78, 0xA7, 0xFF],
        },
        // river.json:194/27/20
        Biome::River => BiomeClimate {
            temperature: 0.5,
            downfall: 0.5,
            has_precipitation: true,
            sky_color: [0x7B, 0xA4, 0xFF],
        },
    }
}

/// 实物 `Biome.getHeightAdjustedTemperature`（Biome.java:112-121）逐句转写：
///
/// ```text
/// float adjustedTemperature = climateSettings.temperatureModifier
///                                 .modifyTemperature(pos, getBaseTemperature());
/// int snowLevel = seaLevel + 17;
/// if (pos.getY() > snowLevel) {
///     float v = (float)(TEMPERATURE_NOISE.getValue(pos.getX() / 8.0F,
///                                                  pos.getZ() / 8.0F, false) * 8.0);
///     return adjustedTemperature - (v + pos.getY() - snowLevel) * 0.05F / 40.0F;
/// }
/// return adjustedTemperature;
/// ```
///
/// 逐参数对位：
/// - `temperatureModifier`：本 10 群系 JSON 均无 `"temperature_modifier"`
///   键 → `TemperatureModifier.NONE`（Biome.java:388-392 恒等返回），
///   `adjustedTemperature = climate(b).temperature`；
/// - `TEMPERATURE_NOISE`：Biome.java:61 常量种子 1234（非世界种子），
///   [`crate::vanilla::simplex::temperature_noise`]；
/// - 坐标为**绝对世界坐标**（BlockPos）：x/z 以 block 计、公式内 ÷8；
///   y 为世界绝对 y（本仓 `MIN_Y..WORLD_MAX_Y` 即原生 y，无重映射）；
/// - f32 域算式（`0.05F/40.0F`、float 中间量）保实物逐位语义。
#[must_use]
pub fn height_adjusted_temperature(b: Biome, x: i32, y: i32, z: i32, sea_level: i32) -> f32 {
    let adjusted = climate(b).temperature as f32;
    let snow_level = sea_level + 17;
    if y > snow_level {
        let v = (crate::vanilla::simplex::temperature_noise()
            .get_value((x as f32 / 8.0_f32) as f64, (z as f32 / 8.0_f32) as f64, false)
            * 8.0) as f32;
        adjusted - (v + (y - snow_level) as f32) * 0.05_f32 / 40.0_f32
    } else {
        adjusted
    }
}

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

    // 段内探针（「注册点必中」断言的坐标，取段界中值；仅测试消费，
    // 置于测试模块避免 lib 目标 dead_code）。
    const T_MID: [f64; 5] = [-0.725, -0.3, 0.025, 0.375, 0.775];
    const H_MID: [f64; 5] = [-0.675, -0.225, 0.0, 0.2, 0.65];

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

    /// 气候表逐值对拍原版 JSON（tint/积雪数据源回归锁：任何改值必须
    /// 同步改 file:line 引注）。
    #[test]
    fn climate_table_matches_vanilla_json() {
        // (群系, temperature, downfall, sky_color, has_precipitation)——
        // precip 列 = 各 JSON 顶层 "has_precipitation" 实物值（desert/
        // savanna 明写 false，余 true）。
        let cases: [(Biome, f64, f64, [u8; 3], bool); 10] = [
            (Biome::Plains, 0.8, 0.4, [0x78, 0xA7, 0xFF], true),
            (Biome::Forest, 0.7, 0.8, [0x79, 0xA6, 0xFF], true),
            (Biome::BirchForest, 0.6, 0.6, [0x7A, 0xA5, 0xFF], true),
            (Biome::Desert, 2.0, 0.0, [0x6E, 0xB1, 0xFF], false),
            (Biome::Savanna, 2.0, 0.0, [0x6E, 0xB1, 0xFF], false),
            (Biome::Taiga, 0.25, 0.8, [0x7D, 0xA3, 0xFF], true),
            (Biome::SnowyPlains, 0.0, 0.5, [0x7F, 0xA1, 0xFF], true),
            (Biome::Ocean, 0.5, 0.5, [0x7B, 0xA4, 0xFF], true),
            (Biome::Beach, 0.8, 0.4, [0x78, 0xA7, 0xFF], true),
            (Biome::River, 0.5, 0.5, [0x7B, 0xA4, 0xFF], true),
        ];
        for (b, t, d, sky, precip) in cases {
            let c = climate(b);
            assert_eq!((c.temperature, c.downfall), (t, d), "{b:?}");
            assert_eq!(c.sky_color, sky, "{b:?} sky_color");
            assert_eq!(c.has_precipitation, precip, "{b:?} has_precipitation");
        }
    }

    /// 表覆盖 10 群系（新增群系时逼气候分支）。
    #[test]
    fn climate_covers_all_biomes() {
        for b in ALL_BIOMES {
            let _ = climate(b);
        }
    }

    /** 高度调整温度的界带锁（Biome.java:112-121 公式的确定性推论；
    修正项 = (v + y−(sea+17))·0.05/40，v = noise·8 ∈ [−8, 8]，sea = 63）：
    - taiga(0.25)：y > 168 时 v+dy > 80 对任意 v 成立 → 恒 < 0.15（雪）；
      y ≤ 152 时 v+dy ≤ 80 → 恒 ≥ 0.15（不雪）；(152,168] 为噪声模糊带。
    - snowy_plains(0.0)：修正上界 +0.00875 → 全高域恒雪。
    - ocean(0.5)：y=319 最冷仍 0.19125 ≥ 0.15 → 恒不雪。 */
    #[test]
    fn height_adjusted_temperature_bands() {
        let sea = mcv_core::SEA_LEVEL;
        for k in 0..32_i32 {
            let (x, z) = (k * 37 - 500, k * 91 + 13);
            // taiga 高山恒雪带 / 低山恒不雪带。
            assert!(
                height_adjusted_temperature(Biome::Taiga, x, 170, z, sea) < 0.15,
                "taiga y=170 应恒越界成雪 (x={x},z={z})"
            );
            assert!(
                height_adjusted_temperature(Biome::Taiga, x, 150, z, sea) >= 0.15,
                "taiga y=150 应恒不雪 (x={x},z={z})"
            );
            // 噪声只改模糊带，不改两端界（抽查 152/168 边界内值在界带间）。
            let mid = height_adjusted_temperature(Biome::Taiga, x, 160, z, sea);
            assert!((0.11..=0.19).contains(&mid), "taiga y=160 值 {mid} 越界带");
            // snowy_plains 全高域恒雪；ocean 全高域恒不雪。
            assert!(height_adjusted_temperature(Biome::SnowyPlains, x, 319, z, sea) < 0.15);
            assert!(height_adjusted_temperature(Biome::Ocean, x, 319, z, sea) >= 0.15);
            // y ≤ sea+17 = 基础温度原样返回（无修正）。
            assert_eq!(
                height_adjusted_temperature(Biome::Taiga, x, sea + 17, z, sea),
                0.25
            );
            assert_eq!(height_adjusted_temperature(Biome::Desert, x, sea + 5, z, sea), 2.0);
        }
    }
}
