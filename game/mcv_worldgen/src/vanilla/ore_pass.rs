//! vanilla 后端 pass 1.7 —— 矿石投影（#178 刀3b；OreFeature 参数化执行器）。
//!
//! 把 [`crate::vanilla::ores_data::ORE_ROWS`]（gen-ores.py 产的 26.1 实物
//! 参数表）接进 [`crate::vanilla::terrain::generate`]：pass 1 柱层 +
//! 岩浆湖后、树 pass 前（26.1 GenerationStep.Decoration 枚举序：
//! LAKES=1 < UNDERGROUND_ORES=6 < VEGETAL_DECORATION=9，
//! GenerationStep.java:8-18 —— 本仓 lake→ore→tree 与其一致）。
//!
//! ## 执行器机制（OreFeature.java:23-181 逐机制对位）
//! - **place**（:23-53）：`dir = nextFloat()·π`；端点 x0/x1 =
//!   origin.x ∓ sin(dir)·(size/8)、z0/z1 = ∓ cos 同式（:28-34，先 + 后 −
//!   的调用序）；y0/y1 各 `nextInt(3) - 2`（:36-37，两次独立抽取）；
//!   盒 xStart/zStart = origin ∓ceil(spread) ∓ maxRadius、
//!   maxRadius = ⌈(size/16·2+1)/2⌉（:30,38-42）；OCEAN_FLOOR_WG 探针
//!   （:44-50）：盒内任一列 `yStart ≤ height` 才进 doPlace（浮空 blob
//!   整体弃、**不消耗** doPlace 的 size 个 nextDouble —— 探针在随机流上
//!   的截断语义照抄）。height 本仓替身 = max(首个 density>0 位, SEA)+1
//!   （water 计顶：Heightmap.java:149,31，lake_pass::Replay::top_floor
//!   同口径；WG 高度图无 surface/特征 = pass-1 密度场实顶，天然一致）。
//! - **doPlace**（:55-166）：size 节点沿中心线 lerp（`step=i/size` f32、
//!   `Mth.lerp(double,double,double)` 重载 = 双精度 start+α(end−start)，
//!   Mth.java:540）；半径 `ss = nextDouble()·size/16`、
//!   `r = ((SIN_LUT(πf·step)+1)·ss+1)/2`（:82-83；Mth.sin 是 65536 项查表
//!   Mth.java:31-36,48-50，本表 SIN_SCALE/索引式逐字节复刻，浮点 sin
//!   求值器为 Rust libm —— clean-room 允许的实现层差异，量化步长本身保
//!   留）；大球吃小球 O(n²)（:90-108，`dr² > d²` 判负置 −1）；节点盒
//!   xMax/yMax/zMax 取 `max(floor(c+r), min)`（:117-122 实物如此，上界
//!   不钳盒——照抄）；椭球判据 (c+0.5−c0)/r 三次和 <1（:124-131）；
//!   isOutsideBuildHeight 兜界（:132，本仓 [−64,320)）；BitSet 防重索引
//!   `x−xStart + (y−yStart)·sizeXZ + (z−zStart)·sizeXZ·sizeY`（:133）；
//!   targetStates 首个 canPlaceOre 胜出即 set+break（:145-151）。
//!   `ensureCanWrite` 26.1 默认恒真（WorldGenLevel.java:10-12），
//!   BulkSectionAccess 邻块可读（:23-52）→ 本仓区块盒无 y 钳之外限制。
//! - **canPlaceOre**（:168-181）：tag 命中（无随机）→
//!   shouldSkipAirCheck（:183-189：discard≤0 恒真零随机；≥1 恒假零随机；
//!   (0,1) 内 `nextFloat() ≥ discard` —— 抽取点照抄）→ !isAdjacentToAir
//!   （六邻 isAir；越界 section 读 = AIR，BulkSectionAccess.java:43-47）。
//! - **placed 链消费序**（PlacedFeature.java:57-77 lazy flatMap，逐尝试
//!   串行）：rarity_filter `nextFloat < 1/chance`（RarityFilter.java:24）
//!   或 count IntProvider（ConstantInt 零随机 / UniformInt 一次
//!   nextInt(hi−lo+1)，CountPlacement.java:23-24 + Mth.java:664-666）→
//!   in_square 2×nextInt(16)（InSquarePlacement.java:17-18）→ height
//!   抽样（UniformHeight.java:37-48 一次均匀、min>max 返 min 零随机；
//!   TrapezoidHeight.java:39-54 两次均匀叠加、plateau≥range 退化、
//!   plateauStart=(range−plateau)/2）→ biome 门零随机
//!   （BiomeFilter.java:24-28，真源=群系 json features[6] → 行位图）→
//!   place/doPlace。本仓每个 (origin 区块, 行) 一条独立流（lake_pass
//!   LAKE-1 同型的域盐派生，非原版 WorldgenRandom 特征种子链）。
//!
//! ## 跨区块模型（tree/lake 重放先例）
//! 原版特征经 WorldGenRegion 写邻块（blob 半径 ≤ ⌈size/8⌉+maxRadius ≤ 6
//! 格 > 区块写半径）；本仓 worker 并行无跨界写面：`origin_ore_writes`
//! 是 (seed, origin 区块) 纯函数，本块对 3×3 origin 域重放并按盒裁剪写
//! 自块——每个体素仍只由所属块写一次，两侧块对同一 blob 的几何/随机流
//! 恒等（矿脉缝合）。判据读 pass-1 **重放态**（密度/含水层/地表材质
//! 纯函数复刻），不读缓冲、不读先前特征写入：blob 重叠与邻块特征先后
//! 的低频赢家差异与 LAKE-1 同型（原版按区块生成序全局定赢家，重放模型
//! 按各自块内应用序覆盖；逐格确定、跨块无缝）。
//! 应用全序 = (origin dz, dx 外层升序；行表序；尝试序；doPlace 扫描序)，
//! 两侧重放同序。

use crate::vanilla::aquifer::{AquiferField, Substance, open_column};
use crate::vanilla::channel_seed;
use crate::vanilla::ores_data::{HeightDist, ORE_ROWS, OreCount, OreRow, biome_allowed};
use crate::vanilla::splitmix64;
use crate::vanilla::terrain::{
    AIR, BEDROCK, ColumnState, LAVA, Orchestrator, REAL_TOP_SCAN_MARGIN, STONE, SurfaceMats, WATER,
    bedrock_noise, half_negative, solid_material,
};
use std::collections::HashMap;
use std::sync::OnceLock;

/// 矿石通道域盐（TREE_DOMAIN/LAKE_DOMAIN 先例；非原版特征种子链，
/// 见模块头注）。
pub const ORE_DOMAIN: u64 = 0x0DE0_2611_0000_0001;

/// Mth.sin 查表（Mth.java:31-36）：`SIN[i] = (float) Math.sin(i /
/// 10430.378350470453)`，索引 `(long)(x·SCALE) & 65535`（:48-50）。
/// 量化步长/取整语义复刻；sin 求值器 = Rust libm（见模块头注）。
fn sin_lut() -> &'static [f32; 65536] {
    static LUT: OnceLock<Box<[f32; 65536]>> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0.0f32; 65536];
        for (i, v) in t.iter_mut().enumerate() {
            *v = ((i as f64) / 10430.378350470453).sin() as f32;
        }
        Box::new(t)
    })
}

/// Mth.sin(float→double 加宽后的查表体)。
fn mth_sin(x: f64) -> f32 {
    let idx = ((x * 10430.378350470453) as i64) & 65535;
    sin_lut()[idx as usize]
}

/// 矿石通道随机流（LakeRng 先例：splitmix64 续流 + Lemire 无偏；
/// 尾数位数对位 Java nextFloat 24 / nextDouble 53）。
pub struct OreRng {
    s: u64,
}

impl OreRng {
    #[must_use]
    const fn new(seed: u64) -> Self {
        Self { s: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.s = splitmix64(self.s ^ 0x9E37_79B9_7F4A_7C15);
        self.s
    }

    /// [0,1) f32（24 位，同 RandomSource.nextFloat 精度面）。
    fn next_float(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// [0,1) f64（53 位，同 nextDouble）。
    fn next_double(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// [0,n) 均匀（Lemire 高位乘，同 LakeRng::next_int）。
    fn next_int(&mut self, n: u32) -> u32 {
        ((self.next_u64() as u128 * u128::from(n)) >> 64) as u32
    }

    /// Mth.randomBetweenInclusive（Mth.java:664-666）。
    fn next_int_inclusive(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.next_int((hi - lo + 1) as u32) as i32
    }
}

/// 一条写记录（世界坐标 + 矿物 id）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OreWrite {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: u16,
}

/// pass-1 重放判据（湖 Replay 同族； ore 需要精确方块 id 做 tag 命中，
/// 故在此复刻 generate 柱层扫描语义）：
/// - `y > fill_top`：缓冲恒 AIR（generate 不上探）；
/// - `y > 首个 density>0 位 fs`：开放柱 `open_column`（generate
///   hit_solid=false 分支，:924-932 同式；fs 扫描= 自 fill_top 向下首个
///   d>0，与 generate 的 hit_solid 首实心位同定义，基岩底 −64 兜底）；
/// - `y ≤ fs`：d>0 ∪ Rock 封口袋为实心；实心且 `y < min_surface` 恒
///   STONE（solid_material 带下兜底，零扫描），带内 [min_surface, fs]
///   才做有界自顶向下 depth_above 计数（SurfaceSystem.java:146/126-128
///   同式）；d≤0 走含水层三态；
/// - 基岩层 −64..−60 同 generate:894-901。
pub struct Replay<'a, 'f> {
    seed: u64,
    orch: &'a Orchestrator,
    field: &'a mut AquiferField<'f>,
    cols: HashMap<(i32, i32), Col>,
}

struct Col {
    cs: ColumnState,
    fill_top: i32,
    min_surface: i32,
    /// 三层材质栈（海洋 SURF-2 波：`column_surface_mats` 群系/水情面，
    /// 与 terrain 缓冲侧共用同一函数）。
    mats: SurfaceMats,
    /// 实物亚表带宽（getSurfaceDepth 逐列值，terrain 同式——随本波一并
    /// 替换旧块级常数 3，消与缓冲侧的带深漂移）。
    surface_depth: i32,
    skip: i32,
    /// 自 fill_top 向下首个 density>0 位（无则 MIN_Y 基岩）。
    first_solid: i32,
    /// block_at 本列 memo（纯函数判据，缓存不改语义；邻块查询复用高）。
    blocks: HashMap<i32, u16>,
    /// [min_surface, fs] 深度梯：depths[fs − y] = depth_above(y)，
    /// 每列一次 O(带) 自顶向下扫描摊销（原逐 query 各扫一遍 = O(带²)）。
    depths: Option<Vec<i32>>,
}

impl<'a, 'f> Replay<'a, 'f> {
    #[must_use]
    pub fn new(seed: u64, orch: &'a Orchestrator, field: &'a mut AquiferField<'f>) -> Self {
        Self {
            seed,
            orch,
            field,
            cols: HashMap::new(),
        }
    }

    /// OCEAN_FLOOR_WG 高度图值（探针用；water 计顶 = max(fs, SEA)+1，
    /// lake_pass::Replay::top_floor 引注同口径）。
    fn top_floor(&mut self, x: i32, z: i32) -> i32 {
        let c = self.col(x, z);
        c.first_solid.max(mcv_core::SEA_LEVEL) + 1
    }

    fn col(&mut self, x: i32, z: i32) -> &mut Col {
        if !self.cols.contains_key(&(x, z)) {
            let cs = self.orch.column_state(x, z);
            let jag = cs.unscaled_jag * half_negative(cs.jag_noise);
            let crossing = (128.0_f64 + 128.0 * (cs.offset + jag)).round() as i32;
            let scan_top = (crossing + REAL_TOP_SCAN_MARGIN)
                .clamp(mcv_core::WORLD_MIN_Y + 1, mcv_core::WORLD_MAX_Y - 1);
            let mats = self.orch.column_surface_mats(x, z, &cs);
            let surface_depth = self.orch.surface_depth(x, z);
            // 表层门基官方实物（SurfaceRules.java:330-383 bilerp4 + sd − 8；
            // 海洋 SURF-2 波随 terrain generate 同式，杜绝重放/缓冲漂移）。
            let min_surface = self.orch.min_surface_level(x, z, surface_depth);
            let skip = self
                .field
                .chunk_skip(x.div_euclid(16) * 16, z.div_euclid(16) * 16);
            let mut c = Col {
                cs,
                fill_top: scan_top.max(mcv_core::SEA_LEVEL),
                min_surface,
                mats,
                surface_depth,
                skip,
                first_solid: mcv_core::WORLD_MIN_Y,
                blocks: HashMap::new(),
                depths: None,
            };
            // fs 扫描（开放段有界：fill_top→fs 通常 <20 格）。
            let mut y = c.fill_top;
            while y > mcv_core::WORLD_MIN_Y && self.orch.density(&c.cs, x, y, z) <= 0.0 {
                y -= 1;
            }
            c.first_solid = y;
            self.cols.insert((x, z), c);
        }
        self.cols.get_mut(&(x, z)).expect("刚插入")
    }

    fn density(&mut self, x: i32, y: i32, z: i32) -> f64 {
        let cs = self.col(x, z).cs;
        self.orch.density(&cs, x, y, z)
    }

    /// 带内 depth_above（自 fs 向下连续实心计数、空气重置；y ∈
    /// [min_surface, fs] 才调用——范围有界，见结构头注）。梯构建的
    /// 逐步 update 序（先记 `cur+1` 再处理 yy 本身）与原逐 query 循环
    /// `while yy > y`（严格处理 y 之上格）逐位等价。每格含水层终判用
    /// **真实 density** 评估一次（generate 柱层 terrain.rs:902-923 同
    /// 式）：Rock 封口袋在 aquifer::substance 的 `density + b > 0` 比较
    /// 中直接消费 dval 实值，伪密度捷径（旧梯内 -1.0）会翻转 Rock/流体
    /// 判定、使带内深度计数与缓冲漂移（红案 run 38422516959
    /// ores_only_replace_pass1_stone 病根：表层 ore 判据拿到假 STONE）。
    fn depth_above(&mut self, x: i32, z: i32, fs: i32, y: i32, skip: i32) -> i32 {
        if let Some(l) = &self.cols.get(&(x, z)).expect("列必建").depths {
            return l[(fs - y) as usize];
        }
        let min_surface = self.col(x, z).min_surface;
        let mut l = Vec::with_capacity((fs - min_surface + 1).max(0) as usize);
        let mut cur = 0;
        let mut yy = fs;
        while yy >= min_surface {
            l.push(cur + 1);
            let d = self.density(x, yy, z);
            if d > 0.0 {
                cur += 1;
            } else {
                match self.field.substance(x, yy, z, d, skip) {
                    Substance::Rock => cur += 1,             // 封口袋实心计 +1
                    Substance::Water | Substance::Lava => {} // 流体不重置
                    Substance::Air => cur = 0,               // 空气重置
                }
            }
            yy -= 1;
        }
        let c = self.col(x, z);
        c.depths = Some(l);
        c.depths.as_ref().expect("刚写入")[(fs - y) as usize]
    }

    /// 判据读点：pass-1 缓冲该格方块 id 的纯函数复刻（per-列 memo：
    /// 纯函数缓存不改输出，is_adjacent_to_air/相邻扫描复用率高）。
    pub fn block_at(&mut self, x: i32, y: i32, z: i32) -> u16 {
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&y) {
            return AIR; // 越界 section 读 = AIR（BulkSectionAccess.java:43-47）
        }
        if let Some(v) = self
            .cols
            .get(&(x, z))
            .and_then(|c| c.blocks.get(&y).copied())
        {
            return v;
        }
        let v = self.block_at_calc(x, y, z);
        self.col(x, z).blocks.insert(y, v);
        v
    }

    fn block_at_calc(&mut self, x: i32, y: i32, z: i32) -> u16 {
        if y == mcv_core::WORLD_MIN_Y
            || (y <= mcv_core::WORLD_MIN_Y + 4 && bedrock_noise(self.seed, x, y, z))
        {
            return BEDROCK;
        }
        let (fill_top, fs, min_surface, mats, surface_depth, skip) = {
            let c = self.col(x, z);
            (
                c.fill_top,
                c.first_solid,
                c.min_surface,
                c.mats,
                c.surface_depth,
                c.skip,
            )
        };
        if y > fill_top {
            return AIR;
        }
        if y > fs {
            // 开放柱（generate hit_solid=false 分支）。
            return match open_column(y) {
                Substance::Water => WATER,
                Substance::Lava => LAVA,
                Substance::Air | Substance::Rock => AIR,
            };
        }
        let d = self.density(x, y, z);
        if d <= 0.0 {
            match self.field.substance(x, y, z, d, skip) {
                Substance::Water => return WATER,
                Substance::Lava => return LAVA,
                Substance::Air => return AIR,
                Substance::Rock => {}
            }
        }
        if y < min_surface {
            return STONE; // solid_material 带下兜底（terrain.rs:972-973）
        }
        let depth = self.depth_above(x, z, fs, y, skip);
        solid_material(y, min_surface, depth, mats, surface_depth)
    }

    fn is_adjacent_to_air(&mut self, x: i32, y: i32, z: i32) -> bool {
        self.block_at(x - 1, y, z) == AIR
            || self.block_at(x + 1, y, z) == AIR
            || self.block_at(x, y - 1, z) == AIR
            || self.block_at(x, y + 1, z) == AIR
            || self.block_at(x, y, z - 1) == AIR
            || self.block_at(x, y, z + 1) == AIR
    }
}

/// 单个 placed 行的 OCEAN_FLOOR_WG 探针 + doPlace（OreFeature.java:44-166
/// 逐段对位）。
fn place_blob(
    rp: &mut Replay<'_, '_>,
    rng: &mut OreRng,
    row: &OreRow,
    wx: i32,
    wy: i32,
    wz: i32,
    writes: &mut Vec<OreWrite>,
) -> bool {
    let size = u32::from(row.size);
    // 实物 size ∈ [3,64]（codec intRange(0,64)）；size=0 行不存在，
    // 仍按 dir/y 抽取消耗保流形状，节点/放矿循环自然为空。
    let dir = rng.next_float() * std::f32::consts::PI;
    let spread = size as f32 / 8.0f32;
    let max_radius = ((size as f32 / 16.0f32 * 2.0f32 + 1.0f32) / 2.0f32).ceil() as i32;
    let (sx, sy) = ((dir as f64).sin(), (dir as f64).cos());
    let x0 = f64::from(wx) + sx * f64::from(spread);
    let x1 = f64::from(wx) - sx * f64::from(spread);
    let z0 = f64::from(wz) + sy * f64::from(spread);
    let z1 = f64::from(wz) - sy * f64::from(spread);
    let y0 = f64::from(wy + rng.next_int(3) as i32 - 2);
    let y1 = f64::from(wy + rng.next_int(3) as i32 - 2);
    let x_start = wx - spread.ceil() as i32 - max_radius;
    let y_start = wy - 2 - max_radius;
    let z_start = wz - spread.ceil() as i32 - max_radius;
    let size_xz = 2 * (spread.ceil() as i32 + max_radius);
    let size_y = 2 * (2 + max_radius);

    // OCEAN_FLOOR_WG 探针（:44-50；无随机消耗，存在性等价早退）。
    let mut probe_ok = false;
    'probe: for px in x_start..=x_start + size_xz {
        for pz in z_start..=z_start + size_xz {
            if y_start <= rp.top_floor(px, pz) {
                probe_ok = true;
                break 'probe;
            }
        }
    }
    if !probe_ok {
        return false;
    }

    // 节点链（:75-88）。
    let mut data = vec![0.0f64; size as usize * 4];
    for i in 0..size as usize {
        let step = (i as f32) / size as f32;
        let a = f64::from(step);
        data[i * 4] = x0 + a * (x1 - x0);
        data[i * 4 + 1] = y0 + a * (y1 - y0);
        data[i * 4 + 2] = z0 + a * (z1 - z0);
        let ss = rng.next_double() * f64::from(size) / 16.0;
        let s = mth_sin(f64::from(std::f32::consts::PI * step));
        data[i * 4 + 3] = ((f64::from(s + 1.0f32)) * ss + 1.0) / 2.0;
    }

    // 大球吃小球（:90-108）。
    for i1 in 0..size as usize - 1 {
        if data[i1 * 4 + 3] <= 0.0 {
            continue;
        }
        for i2 in i1 + 1..size as usize {
            if data[i2 * 4 + 3] <= 0.0 {
                continue;
            }
            let dx = data[i1 * 4] - data[i2 * 4];
            let dy = data[i1 * 4 + 1] - data[i2 * 4 + 1];
            let dz = data[i1 * 4 + 2] - data[i2 * 4 + 2];
            let dr = data[i1 * 4 + 3] - data[i2 * 4 + 3];
            if dr * dr > dx * dx + dy * dy + dz * dz {
                if dr > 0.0 {
                    data[i2 * 4 + 3] = -1.0;
                } else {
                    data[i1 * 4 + 3] = -1.0;
                }
            }
        }
    }

    // 放矿（:111-163）。HashSet 对位 BitSet（索引式逐字照抄 :133）。
    let mut tested: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut placed = 0usize;
    for i in 0..size as usize {
        let r = data[i * 4 + 3];
        if r < 0.0 {
            continue;
        }
        let (xx, yy, zz) = (data[i * 4], data[i * 4 + 1], data[i * 4 + 2]);
        let x_min = (xx - r).floor().max(f64::from(x_start)) as i32;
        let y_min = (yy - r).floor().max(f64::from(y_start)) as i32;
        let z_min = (zz - r).floor().max(f64::from(z_start)) as i32;
        let x_max = ((xx + r).floor() as i32).max(x_min);
        let y_max = ((yy + r).floor() as i32).max(y_min);
        let z_max = ((zz + r).floor() as i32).max(z_min);
        for x in x_min..=x_max {
            let xd = (f64::from(x) + 0.5 - xx) / r;
            if xd * xd >= 1.0 {
                continue;
            }
            for y in y_min..=y_max {
                let yd = (f64::from(y) + 0.5 - yy) / r;
                if xd * xd + yd * yd >= 1.0 {
                    continue;
                }
                for z in z_min..=z_max {
                    let zd = (f64::from(z) + 0.5 - zz) / r;
                    if xd * xd + yd * yd + zd * zd >= 1.0 {
                        continue;
                    }
                    if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&y) {
                        continue; // isOutsideBuildHeight（:132）
                    }
                    let bit = i64::from(x - x_start)
                        + i64::from(y - y_start) * i64::from(size_xz)
                        + i64::from(z - z_start) * i64::from(size_xz) * i64::from(size_y);
                    if !tested.insert(bit) {
                        continue;
                    }
                    let cur = rp.block_at(x, y, z);
                    for t in row.targets {
                        if !t.tag.set.contains(cur) {
                            continue;
                        }
                        // shouldSkipAirCheck（OreFeature.java:183-189）。
                        let skip_air = if row.discard_on_air <= 0.0 {
                            true
                        } else if row.discard_on_air >= 1.0 {
                            false
                        } else {
                            rng.next_float() >= row.discard_on_air
                        };
                        if !skip_air && rp.is_adjacent_to_air(x, y, z) {
                            continue;
                        }
                        writes.push(OreWrite {
                            x,
                            y,
                            z,
                            id: t.state,
                        });
                        placed += 1;
                        break;
                    }
                }
            }
        }
    }
    placed > 0
}

/// height 抽样（UniformHeight.java:37-48 / TrapezoidHeight.java:39-54）。
fn sample_height(h: &HeightDist, rng: &mut OreRng) -> i32 {
    match *h {
        HeightDist::Uniform { lo, hi } => {
            if lo > hi {
                lo
            } else {
                rng.next_int_inclusive(lo, hi)
            }
        }
        HeightDist::Trapezoid { lo, hi, plateau } => {
            if lo > hi {
                return lo;
            }
            let range = hi - lo;
            if plateau >= range {
                return rng.next_int_inclusive(lo, hi);
            }
            let ps = (range - plateau) / 2;
            let pe = range - ps;
            lo + rng.next_int_inclusive(0, pe) + rng.next_int_inclusive(0, ps)
        }
        HeightDist::Const(v) => v,
    }
}

/// 单个 origin 区块的全部 ore 写（全序；(seed, origin) 纯函数——两侧块
/// 重放恒等）。`rp` 只做跨调用缓存（列/含水层查询），不掺入结果。
pub fn origin_ore_writes(seed: u64, ocx: i32, ocz: i32, rp: &mut Replay<'_, '_>) -> Vec<OreWrite> {
    let ts = splitmix64(channel_seed(seed, ORE_DOMAIN) as u64);
    let cs = splitmix64(
        ts ^ (i64::from(ocx) as u64).wrapping_mul(0x9E37_79B1_9E37_79B1)
            ^ (i64::from(ocz) as u64).wrapping_mul(0x85EB_CA77_85EB_CA77),
    );
    let mut writes = Vec::new();
    for (idx, row) in ORE_ROWS.iter().enumerate() {
        let mut rng = OreRng::new(splitmix64(
            cs ^ (idx as u64 + 1).wrapping_mul(0x36B1_C4A2_77B1_23A3),
        ));
        // rarity_filter（RarityFilter.java:24；placed 实物链首，整链门）。
        if row.rarity > 0
            && rng.next_float().partial_cmp(&(1.0f32 / row.rarity as f32))
                != Some(std::cmp::Ordering::Less)
        {
            continue;
        }
        // count IntProvider（一次抽取）。
        let n = match row.count {
            OreCount::Fixed(n) => n,
            OreCount::Uniform { lo, hi } => lo + rng.next_int(hi - lo + 1),
        };
        for _ in 0..n {
            // in_square（InSquarePlacement.java:17-18）。
            let wx = ocx * 16 + rng.next_int(16) as i32;
            let wz = ocz * 16 + rng.next_int(16) as i32;
            // height_range。
            let wy = sample_height(&row.height, &mut rng);
            // biome 门（零随机；BiomeFilter.java:24-28）。
            if !biome_allowed(row, rp.orch.biome_at(wx, wz)) {
                continue;
            }
            place_blob(rp, &mut rng, row, wx, wy, wz, &mut writes);
        }
    }
    writes
}

/// pass 1.7：矿脉投影。3×3 origin 区块重放 + 盒裁剪写自块（模块头注
/// 跨区块模型）；应用全序 = (dz, dx, 行, 尝试, 扫描序)，两侧一致。
pub fn project_ores(
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
            for w in origin_ore_writes(seed, chunk_x + dx, chunk_z + dz, &mut rp) {
                let lx = w.x - base_x;
                let lz = w.z - base_z;
                if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
                    continue; // 属邻块：由邻块重放同一 blob 补齐
                }
                voxels[mcv_core::vidx(lx as usize, w.y, lz as usize)] = w.id;
                applied += 1;
            }
        }
    }
    applied
}

/// 测试/工具入口：自备 Replay 的便捷封装（同 (seed, origin) 恒同输出）。
#[must_use]
pub fn ore_writes(
    seed: u64,
    ocx: i32,
    ocz: i32,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
) -> Vec<OreWrite> {
    let mut rp = Replay::new(seed, orch, field);
    origin_ore_writes(seed, ocx, ocz, &mut rp)
}
