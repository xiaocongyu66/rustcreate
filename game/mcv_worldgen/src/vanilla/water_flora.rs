//! vanilla 后端 pass 2.3 —— 水草植被（海草 [SeagrassFeature] + 海带
//! [KelpFeature] 执行器）。
//!
//! 把 26.1 水草特征接进 [`crate::vanilla::terrain::generate`]：树 pass 后、
//! freeze 前——原版两特征与树同在 VEGETAL_DECORATION 特征阶，槽内按群系
//! features 数组序（ocean.json 的 vegetal 槽 trees_water 先于
//! seagrass_normal/kelp_cold）；freeze_top_layer 是次阶（freeze_pass 头注
//! features[10]），terrain.rs 调度序（lake→ore→tree→水草→freeze）与之逐段
//! 对齐。执行器与重放模型照 [`crate::vanilla::ore_pass`] 先例（#178 执行
//! 器面）：3×3 origin 区块重放 + 裁剪写自块；判据读 pass-1 纯函数重放态
//! （[`crate::vanilla::ore_pass::Replay`]，不读缓冲、不读先前特征写）。
//!
//! ## 26.1 实物锚点（相对 `data/minecraft/worldgen/`，Java 相对
//! `net/minecraft/world/.../`）
//!
//! - **数据行**：biome/ocean.json:92 `"minecraft:seagrass_normal"` →
//!   placed_feature/seagrass_normal.json（count 48）→ configured
//!   seagrass_short.json（probability 0.3）；ocean.json:93
//!   `"minecraft:kelp_cold"` → placed_feature/kelp_cold.json
//!   （noise_based_count ratio 120 / factor 80 / offset 0）→ configured
//!   kelp.json；biome/river.json:93 `"minecraft:seagrass_river"` →
//!   placed_feature/seagrass_river.json（count 48）→ configured
//!   seagrass_slightly_less_short.json（probability 0.4）。
//!   frozen_river.json 无水草行（原样不带，与任务点名「河流群系」核对：
//!   原版冻河本就不长水草）。swamp（seagrass_swamp count 64 p0.6）/
//!   mangrove_swamp/warm/cold/deep 系群系行本仓 10 群系点表（biomes.rs
//!   v1，无 Swamp/深海变体）选不出 → 不登记、路径不可达（tree_pass
//!   profile 对 Jungle/DarkForest 同款处理），群系表扩 v2 时补行。
//!
//! - **SeagrassFeature.place（SeagrassFeature.java:20-51）**：feature 内
//!   抖动 `x = nextInt(8) − nextInt(8)`、z 同式（:26-27，水位门前 4 抽全
//!   消耗）；`y = getHeight(OCEAN_FLOOR)`（:28，heightmap 值 = 洋底/河床
//!   实顶首空位，Heightmap.java:149-150 MATERIAL_MOTION_BLOCKING——水
//!   不计顶）；grassPos 水位门（:30，零随机）；`isTall = nextDouble <
//!   probability`（:31，**水位门失败则不消耗**该抽）；canSurvive（:33）；
//!   tall 先 roll 后查、半格写法：双格需上格水（:34-40，HALF=UPPER
//!   :35，上格门 :37），上格非水则整投放弃（不写任何格）；短格单格
//!   （:41-43）。
//!
//! - **KelpFeature.place（KelpFeature.java:19-54）**：无 feature 内水平
//!   抖动（:25 直接用放置链 origin）；`y = getHeight(OCEAN_FLOOR)`（:24
//!   同水不计顶口径）kelpPos = 值位，水位门（:26）；柱高
//!   `height = 1 + nextInt(10) ∈ 1..10`（:29，任务单「1..25」与反编译
//!   树不符——25 是 KelpBlock.AGE 方块状态域上限，非特征柱高，柱面以
//!   实物为准）；h 循环 `for h in 0..=height`（:31），每层三联门（本层
//!   水 + 上层 WATER + canSurvive，:32）；`h == height` 落**头**（KELP
//!   age nextInt(4)+20，:34）否则落**体**（KELP_PLANT，:37）；门败且
//!   h>0 截断（:39-46）：`below = kelpPos.below()`（:40，= 最上一节
//!   体——头仅在 h==height 落，柱中恒体）`stateTop.canSurvive(below)`
//!   （读位 = below.below()，GrowingPlantBlock.java:47-53
//!   attachedToPos）+ `below.below() 非 KELP 头`（:41）才把该节换头
//!   （:42，age 同抽）后 break（:45）；kelpPos.above() 逐层（:48，成
//!   功/失败同进——h==0 失败仅空转前进）。h=0 失败的合法解（浅水单
//!   格：y0 水但 y0+1 非水）由 h=1 截断分支补：under = below.below =
//!   洋底/河床 y0−1（重放实块），门过则单点头。
//!
//! - **附着门（GrowingPlantBlock.java:47-56 + KelpBlock.java:46-48 +
//!   SeagrassBlock.java:46-47）**：canSurvive = attachedTo（below）非
//!   canAttachTo 否 → 头/体/isFaceSturdy 三选一（GrowingPlantBlock
//!   .java:47-53）；kelp 的 canAttachTo = 非 CANNOT_SUPPORT_KELP
//!   （KelpBlock.java:46-48）；seagrass = mayPlaceOn = face-sturdy(UP)
//!   且非 CANNOT_SUPPORT_SEAGRASS（SeagrassBlock.java:46-47）。本仓面：
//!   face-sturdy ≈ 生成面实心（`mcv_core::BLOCKS[id].solid`；pass-1 首
//!   实心恒 solid）；tag 静态表 `mcv_core::tags` 只含 magma_block
//!   （id 585，TAG_BLOCK_CANNOT_SUPPORT_KELP/SEAGRASS，gen-tags 波骨架
//!   产物）。kelp 体（509）/头（508）solid=false → 头/体成员显式名单。
//!
//! - **放置链（placed JSON 修饰序，PlacedFeature.java:47 lazy
//!   flatMap 逐修饰串行；origin = 区块角，ChunkGenerator.java:314-318
//!   `SectionPos.origin()`）**：seagrass 链 = in_square（
//!   InSquarePlacement.java:17-19，2×nextInt(16)，单复制位置）→
//!   heightmap(OCEAN_FLOOR_WG) → count 48（ConstantInt 零随机）→ biome
//!   （BiomeFilter.java:20-26 零随机，判 Placement 位群系）→ feature 48
//!   次**同锚点**执行（count 先产 48 份同 pos 复制，biome 门对同点位零
//!   随机判 1 次；每次特征内 ±7 抖动 → 48 次重试的「锚点丛」分布，
//!   与 placed JSON 修饰序逐位同构）；kelp 链 = noise_based_count（
//!   NoiseBasedCountPlacement.java:34-36：
//!   `BIOME_INFO_NOISE.getValue(x/factor, z/factor, false)` 的
//!   `ceil((n + offset) · ratio)`，在 origin（区块角）一次评估、负数 =
//!   IntStream.range 空域零尝试）→ in_square 每复制 2 抽 → heightmap →
//!   biome → feature。
//!
//! - **BIOME_INFO_NOISE**（Biome.java:66：`PerlinSimplexNoise(
//!   WorldgenRandom(LegacyRandomSource(2345)), [0])`，常量种子=世界种子
//!   无关）= [`crate::vanilla::simplex::PerlinSimplexNoise`] 实物移植
//!   （TEMPERATURE_NOISE 种子 1234 同构造先例 simplexs.rs `temperature_
//!   noise`）。
//!
//! - **随机域**：每 (origin 区块, 行) 一条独立流（ore_pass
//!   ·origin_ore_writes 域盐同型 splitmix；非原版 setDecorationSeed 特征
//!   种子链——本仓契约「同 seed 同字节」，非逐位对拍，tree_pass 同款登
//!   记）。抽取序 = 链序 + 特征序逐字照抄（消耗面以实物抽取点为准）。
//!
//! ## 本仓建模登记（KNOWN-DIVERGENCE 同族）
//! - **方块状态面**：kelp 头 age 20-23、seagrass_tall HALF=UPPER、
//!   WATERLOGGED（原版水草恒含水 `fluidState=WATER.source`，
//!   KelpPlantBlock.java:49-52/SeagrassBlock.java:57-59）均无 voxel 表达
//!   （单 id u16、GEN_BLOCKS 九列无 state/waterlogged 位）——生成期以
//!   「替换水体格 id」表达，「含水块标记按 blocks 表需要」盘点结论：
//!   blocks 表无所需字段，不做状态写入（渲染/流体面由 mesher/fluid-flow
//!   另案）。tall_seagrass 上格 = 同 id 1002。
//! - **判据读 pass-1 重放态**：lake/ore 树前特征与本特征、树写的互见与
//!   原版 features 全局串行世界写不同频（ore_pass 头注同款登记：逐格确
//!   定、跨块无缝、低频赢家差）。
//! - **写入帽 / 几何契约**：本仓水体面含 y=SEA 行（aquifer.rs
//!   open_column 契约、geometry_locks 锁死「海下柱 63 层非水」；原版
//!   最高水行 = seaLevel−1）——水草/tall 上格一律写 y ≤ SEA−1（浅水
//!   单格柱 [地板 62/水仅 63] 与 y=62 浅水 tall 在原文本可见出海面外，
//!   本仓按几何契约放弃），kelp 循环自带的「上层水门」（层门 :32 含
//!   y+1 水）另使头/体恒 ≤ SEA−1。kelp 头水位门即上层有水 → 海面行
//!   从不被植被占格（freeze 冰层在其后照常）。

use crate::vanilla::aquifer::AquiferField;
use crate::vanilla::biomes::Biome;
use crate::vanilla::channel_seed;
use crate::vanilla::ore_pass::Replay;
use crate::vanilla::simplex::{LegacyRandom, PerlinSimplexNoise};
use crate::vanilla::splitmix64;
use crate::vanilla::terrain::{Orchestrator, WATER};
use std::sync::OnceLock;

/// 水草通道域盐（ORE_DOMAIN/TREE_DOMAIN/LAKE_DOMAIN 先例；每 (origin 区
/// 块, 行) 流再域内派生，见模块头注随机域段）。
pub const WATER_FLORA_DOMAIN: u64 = 0x005E_A5EE_0000_0001;

/// 下行 id 引 `mcv_core::BLOCKS`（blocks_gen.inc.rs，注释 = 表行号）：
///海带头方块（AGE 0..25 状态面本仓不存，/id 即视觉）。
const KELP: u16 = 508; // blocks_gen.inc.rs:536 ("kelp")
/// 海带体方块。
const KELP_PLANT: u16 = 509; // blocks_gen.inc.rs:537 ("kelp_plant")
/// 海草。
const SEAGRASS: u16 = 905; // blocks_gen.inc.rs:933 ("seagrass")
/// 高海草（上/下半同 id；HALF 状态面本仓不存）。
const TALL_SEAGRASS: u16 = 1002; // blocks_gen.inc.rs:1030 ("tall_seagrass")

/// BIOME_INFO_NOISE（Biome.java:66，常量种子 2345、octave 集 [0]）。
fn biome_info_noise() -> &'static PerlinSimplexNoise {
    static NOISE: OnceLock<PerlinSimplexNoise> = OnceLock::new();
    NOISE.get_or_init(|| {
        let mut r = LegacyRandom::new(2345);
        PerlinSimplexNoise::new(&mut r, &[0])
    })
}

/// kelp 附着门（GrowingPlantBlock.java:47-56 + KelpBlock.java:46-48）：
/// 下方非 CANNOT_SUPPORT_KELP 且（头/体 || face-sturdy）。本仓
/// face-sturdy ≈ `BLOCKS[id].solid`；tag 静态表当前只含 magma_block。
fn kelp_supports(below: u16) -> bool {
    !mcv_core::tags::TAG_BLOCK_CANNOT_SUPPORT_KELP
        .set
        .contains(below)
        && (below == KELP || below == KELP_PLANT || mcv_core::BLOCKS[usize::from(below)].solid)
}

/// seagrass 附着门（SeagrassBlock.java:46-47）：下方 face-sturdy 且非
/// CANNOT_SUPPORT_SEAGRASS。
fn seagrass_supports(below: u16) -> bool {
    !mcv_core::tags::TAG_BLOCK_CANNOT_SUPPORT_SEAGRASS
        .set
        .contains(below)
        && mcv_core::BLOCKS[usize::from(below)].solid
}

/// 一行水草 placed 行（群系 + configured 特征类型 + count 参数）。
struct FloraRow {
    biome: Biome,
    count: i32,
    kind: FloraKind,
}

enum FloraKind {
    /// configured_feature/seagrass_*.json（`probability` → tall roll）。
    Seagrass { probability: f64 },
    /// configured_feature/kelp.json（noise_based_count 参数）。
    Kelp {
        ratio: i32,
        factor: f64,
        offset: f64,
    },
}

/// 26.1 数据行（引注见模块头注数据行段）。10 群系点表选不出的群系
/// 变体（swamp/deep/warm/cold 系）不登记——路径不可达，见头注。
const FLORA_ROWS: [FloraRow; 3] = [
    // biome/ocean.json:92 → seagrass_normal.json（count 48）→ seagrass_short（p 0.3）。
    FloraRow {
        biome: Biome::Ocean,
        count: 48,
        kind: FloraKind::Seagrass { probability: 0.3 },
    },
    // biome/ocean.json:93 → kelp_cold.json（ratio 120 / factor 80 / offset 0）。
    FloraRow {
        biome: Biome::Ocean,
        count: 0,
        kind: FloraKind::Kelp {
            ratio: 120,
            factor: 80.0,
            offset: 0.0,
        },
    },
    // biome/river.json:93 → seagrass_river.json（count 48）→ slightly_less_short（p 0.4）。
    FloraRow {
        biome: Biome::River,
        count: 48,
        kind: FloraKind::Seagrass { probability: 0.4 },
    },
];

/// 水草通道随机流（OreRng 同型：splitmix64 续流 + Lemire 无偏；尾数位
/// 数对位 Java nextFloat 24 / nextDouble 53——ore_pass 头注「本仓每个
/// (origin 区块, 行) 一条独立流」同款，非原版特征种子链）。
struct FloraRng {
    s: u64,
}

impl FloraRng {
    fn new(seed: u64) -> Self {
        Self { s: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.s = splitmix64(self.s ^ 0x9E37_79B9_7F4A_7C15);
        self.s
    }

    fn next_double(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// [0,n) 均匀（`RandomSource.nextInt(bound)` 语义面）。
    fn next_int(&mut self, n: u32) -> u32 {
        ((self.next_u64() as u128 * u128::from(n)) >> 64) as u32
    }
}

/// 一条写记录（世界坐标 + 水草 id；y ∈ 世界体素域由投影端钳）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloraWrite {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: u16,
}

/// SeagrassFeature.java:20-51 逐段对位（单次尝试）。
fn place_seagrass(
    rp: &mut Replay<'_, '_>,
    rng: &mut FloraRng,
    probability: f64,
    ax: i32,
    az: i32,
    writes: &mut Vec<FloraWrite>,
) {
    // :26-27——4 抽先于水位门全消耗。
    let x = ax + rng.next_int(8) as i32 - rng.next_int(8) as i32;
    let z = az + rng.next_int(8) as i32 - rng.next_int(8) as i32;
    // :28 getHeight(OCEAN_FLOOR) = 实顶首空位（Replay::top_ocean_floor
    // 口径：ore_pass::top_ocean_floor 引注 Heightmap.java:149-150
    // MATERIAL_MOTION_BLOCKING，水不计顶——洋底/河床顶 = fs+1）。
    let y = rp.top_ocean_floor(x, z);
    // 特征前哨：写入帽 y ≤ SEA−1（零随机；geometry_locks「海下柱 63 层
    // 非水」锁钉住——本仓几何契约的水面行 = y63，浅水单格柱（河沿/滩沿
    // 地板 62、水仅 63 一行）按原文本会在 63 落草尾，登记为几何契约
    // KNOWN-DIVERGENCE（头注放置链段末）。门败不消耗同原文本。
    if y >= mcv_core::SEA_LEVEL {
        return;
    }
    // :30 水位门（零随机）。
    if rp.block_at(x, y, z) != WATER {
        return;
    }
    // :31 tall roll（水位门后；门败不消耗）。
    let tall = rng.next_double() < probability;
    // :33 canSurvive（附着门在 roll 之后、写之前）。
    if !seagrass_supports(rp.block_at(x, y - 1, z)) {
        return;
    }
    if tall {
        // :34-40 上格必须仍是水，否则整投放弃（不写任何格）；并入写帽
        // ——上格 y+1 < SEA_LEVEL（tall 上半不进海面行 63；y=62 浅水
        // tall 按几何契约放弃，同登记 KNOWN-DIVERGENCE）。
        if rp.block_at(x, y + 1, z) == WATER && y + 1 < mcv_core::SEA_LEVEL {
            writes.push(FloraWrite {
                x,
                y,
                z,
                id: TALL_SEAGRASS,
            });
            writes.push(FloraWrite {
                x,
                y: y + 1,
                z,
                id: TALL_SEAGRASS,
            });
        }
    } else {
        // :41-43
        writes.push(FloraWrite {
            x,
            y,
            z,
            id: SEAGRASS,
        });
    }
}

/// KelpFeature.java:19-54 逐段对位（单次尝试）。
fn place_kelp(
    rp: &mut Replay<'_, '_>,
    rng: &mut FloraRng,
    wx: i32,
    wz: i32,
    writes: &mut Vec<FloraWrite>,
) {
    // :24-25 getHeight(OCEAN_FLOOR)（无特征内水平抖动，直接用放置链位）。
    let y0 = rp.top_ocean_floor(wx, wz);
    // 写入帽同 place_seagrass（geometry_locks 海面行钉住；零随机）。
    if y0 >= mcv_core::SEA_LEVEL {
        return;
    }
    // :26 水位门（零随机）。
    if rp.block_at(wx, y0, wz) != WATER {
        return;
    }
    // :29 柱高 1 + nextInt(10) ∈ 1..10。
    let height = 1 + rng.next_int(10) as i32;
    let mut y = y0;
    // h 循环 :31-49。判据读重放态（replay 不见本柱写入）——柱内下方
    // 格按「上一轮已落体」显式换算，逐位等价缓冲读。
    for h in 0..=height {
        let below = if h == 0 {
            rp.block_at(wx, y - 1, wz)
        } else {
            KELP_PLANT
        };
        // :32 本层水 + 上层水 + canSurvive（附着门）。
        let ok = rp.block_at(wx, y, wz) == WATER
            && rp.block_at(wx, y + 1, wz) == WATER
            && kelp_supports(below);
        if ok {
            if h == height {
                // :34 落头（age = nextInt(4)+20 抽消耗；状态面不存）。
                let _age = rng.next_int(4);
                writes.push(FloraWrite {
                    x: wx,
                    y,
                    z: wz,
                    id: KELP,
                });
            } else {
                // :37 落体。
                writes.push(FloraWrite {
                    x: wx,
                    y,
                    z: wz,
                    id: KELP_PLANT,
                });
            }
            y += 1; // :48 kelpPos = kelpPos.above()（无条件）
        } else if h > 0 {
            // :39-46 截断：below = kelpPos.below()（:40，= 最上一节体，
            // 头仅在最后落——柱中恒体）；canSurvive 读位 = below.below()
            // （GrowingPlantBlock.java:47-50 attachedToPos）+ 该位非 KELP
            // 头（:41）。under：h=1 时 = 洋底/河床（y0−1，重放实块）；
            // h≥2 = 更下一节体。
            let below_y = y - 1;
            let under = if h == 1 {
                rp.block_at(wx, y0 - 1, wz)
            } else {
                KELP_PLANT
            };
            let under_non_head = if h == 1 {
                rp.block_at(wx, y0 - 1, wz) != KELP
            } else {
                true
            };
            if kelp_supports(if h == 1 { under } else { KELP_PLANT }) && under_non_head {
                let _age = rng.next_int(4); // :42 age 抽（截断路径同样消耗）
                writes.push(FloraWrite {
                    x: wx,
                    y: below_y,
                    z: wz,
                    id: KELP,
                });
            }
            break; // :45
        } else {
            // h == 0 失败：Java else-if 链无分支 → 空转前进（下一层重判）。
            y += 1; // :48
        }
    }
}

/// 单个 origin 区块的全部水草写（全序；(seed, origin) 纯函数——两侧块
/// 重放恒同输出）。链序/抽取序逐字对位（模块头注）。
pub fn origin_flora_writes(
    seed: u64,
    ocx: i32,
    ocz: i32,
    rp: &mut Replay<'_, '_>,
) -> Vec<FloraWrite> {
    let ts = splitmix64(channel_seed(seed, WATER_FLORA_DOMAIN) as u64);
    let cs = splitmix64(
        ts ^ (i64::from(ocx) as u64).wrapping_mul(0x9E37_79B1_9E37_79B1)
            ^ (i64::from(ocz) as u64).wrapping_mul(0x85EB_CA77_85EB_CA77),
    );
    let mut writes = Vec::new();
    for (idx, row) in FLORA_ROWS.iter().enumerate() {
        let mut rng = FloraRng::new(splitmix64(
            cs ^ (idx as u64 + 1).wrapping_mul(0x36B1_C4A2_77B1_23A3),
        ));
        match row.kind {
            FloraKind::Seagrass { probability } => {
                // 链：in_square 抖动（InSquarePlacement.java:17-18，2 抽）
                // → heightmap → count(48) 同锚复制 → biome 门（链内，判
                // 抖动后 in-chunk 点位；零随机）→ feature 48 次同点重试。
                let ax = ocx * 16 + rng.next_int(16) as i32;
                let az = ocz * 16 + rng.next_int(16) as i32;
                if rp.biome_at(ax, az) != row.biome {
                    continue;
                }
                for _ in 0..row.count {
                    place_seagrass(rp, &mut rng, probability, ax, az, &mut writes);
                }
            }
            FloraKind::Kelp {
                ratio,
                factor,
                offset,
            } => {
                // 链：noise_based_count（NoiseBasedCountPlacement.java:
                // 38-42，origin=区块角一次评估，负数零尝试，零随机）→
                // 每复制 in_square 2 抽 → heightmap → biome 门 → feature。
                let n = biome_info_noise().get_value(
                    f64::from(ocx * 16) / factor,
                    f64::from(ocz * 16) / factor,
                    false,
                );
                let count = ((n + offset) * f64::from(ratio)).ceil() as i64;
                for _ in 0..count.max(0) {
                    let wx = ocx * 16 + rng.next_int(16) as i32;
                    let wz = ocz * 16 + rng.next_int(16) as i32;
                    if rp.biome_at(wx, wz) != row.biome {
                        continue;
                    }
                    place_kelp(rp, &mut rng, wx, wz, &mut writes);
                }
            }
        }
    }
    writes
}

/// pass 2.3：水草投影。3×3 origin 区块重放 + 裁剪写自块（ore_pass
/// ·project_ores 同型；seagrass 特征内 ±7 抖动可越界 本块 7 格、kelp
/// 柱同列——统一 3×3 域重放两侧块恒同几何）。替换写 = 原版
/// setBlock(flags 2) 直写语义；y 域出界格丢（本柱最高 y ≈ SEA+11，仅
/// 兜底）。返回写数。
pub fn project_water_flora(
    seed: u64,
    chunk_x: i32,
    chunk_z: i32,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
    voxels: &mut [u16],
) -> usize {
    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    let mut rp = Replay::new(seed, orch, field);
    let mut applied = 0usize;
    for dz in -1..=1 {
        for dx in -1..=1 {
            for w in origin_flora_writes(seed, chunk_x + dx, chunk_z + dz, &mut rp) {
                let lx = w.x - base_x;
                let lz = w.z - base_z;
                if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
                    continue; // 属邻块：由邻块重放同一 (seed, origin) 补齐
                }
                if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&w.y) {
                    continue;
                }
                voxels[mcv_core::vidx(lx as usize, w.y, lz as usize)] = w.id;
                applied += 1;
            }
        }
    }
    applied
}

/// 测试/工具入口：自备 Replay 的便捷封装（同 (seed, origin) 恒同输出；
/// ore_pass::ore_writes 先例）。
#[must_use]
pub fn flora_writes(
    seed: u64,
    ocx: i32,
    ocz: i32,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
) -> Vec<FloraWrite> {
    let mut rp = Replay::new(seed, orch, field);
    origin_flora_writes(seed, ocx, ocz, &mut rp)
}
