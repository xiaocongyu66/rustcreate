//! #175 含水层波 — 26.1 `NoiseBasedAquifer` 的结构移植（原 inner class，
//! 快照 `net/minecraft/world/level/levelgen/Aquifer.java:58-513`）。
//!
//! 修复真机症状：主循环旧判据「y≤SEA 的一切空腔一律 WATER」是 1.18 前
//! 平铺模型；原版语义 = 地下空气默认干，注水由「16×12×16 含水层胞格 +
//! floodedness 噪声阈值 + barrier 压力」门控（#91/#122 移植时整体漏掉，
//! 全仓 grep aquifer 曾零匹配）。
//!
//! ## 原版机制摘要（file:line 实证，快照 /root/mc-ref/src-26.1）
//! - 启用路对照：`NoiseChunk.java:144-152` `settings.isAquifersEnabled()`
//!   假 → `createDisabled`（Aquifer.java:30-40，纯 fluidLevelForPos 平铺，
//!   = 本仓旧行为）；真 → `NoiseBasedAquifer`（overworld.json
//!   `aquifers_enabled: true`，data/minecraft/worldgen/noise_settings/
//!   overworld.json）。本仓恒走启用路。
//! - 合成调用点：`NoiseChunk.java:160`——density（finalDensity+beardifier）
//!   ≤0 时把终判交给 `aquifer.computeSubstance(context, density)`；
//!   实心侧（density>0）`Aquifer.java:149-152` 直接返回 null=石头。
//! - 判据本体 `computeSubstance`（Aquifer.java:149-296）：
//!   1. y > skipSamplingAboveY（表面+~30，构造期由 `maxPreliminarySurface
//!      Level`+12 过格线算得，Aquifer.java:134-137）→ 直接全局流体
//!      （水面以上空气/以下水）；
//!   2. 全局岩浆（y < -54，见下）→ LAVA（:167-170）；
//!   3. 12 邻胞格（x∈{0,1}×y∈{-1,0,1}×z∈{0,1}，:180-230）取最近 4 胞心
//!      （胞格 16×12×16，胞心 = 胞格原点 + 位置随机 nextInt(10/9/10)，
//!      :199-207；常量表 :61-70）；
//!   4. `similarity(d1,d2) = 1-(d2-d1)/25`（:299-301）≤0 → 最近胞状态
//!      一票通过；否则 barrier 压力合成：`density + sim·pressure > 0` →
//!      **石头**（囊袋被压力封住＝干），三对 (1,2)(1,3)(2,3) 逐一过门
//!      （:262-283），全漏网才出流体；
//!   5. 水格且 y−1 处全局岩浆 → 直接水（原版蒸汽边界，:246-249）；
//!   6. `calculatePressure`（:304-365）：两胞液位差梯度 + barrier 噪声
//!      （|grad|≤2 才采样，梯度截断），返回 2·(noise+grad)。
//! - 胞格流体状态 `computeFluid`（:396-440）：13 个区块偏移采样
//!   `preliminarySurfaceLevel`+8（偏移表 :92-95，adjust :441-443）——
//!   胞底已高于中心地表 → 全局流体（地表水体）；胞顶戳出低于海平面的
//!   地表列 → 全局流体；否则 `computeSurfaceLevel`（:445-480）：
//!   deep dark 区（erosion<−0.225 且 depth>0.9，OverworldBiomeBuilder.
//!   java:1008-1010；depth = yClampedGradient(−64,320,1.5,−1.5)+offset，
//!   NoiseRouterData.java:180-182）强制干；否则 floodedness 噪声（截断
//!   [−1,1]）对比双阈值——完全淹（阈值 0.8−1.1·f，f=水下地表深度因子
//!   clampedMap(地表下深,0,64,1,0)）→ 液面=全局海平面；半淹（阈值 0.4−
//!   1.2·f）→ `computeRandomizedFluidSurfaceLevel`（:482-496，16×40×16
//!   胞 + spread 噪声×10 按 3 量化，min(最低地表, 胞中 y+量化散布)）；
//!   都不过 → WAY_BELOW_MIN_Y（全干）。
//! - 岩浆分界 `computeFluidType`（:498-513）：半淹液面 ≤ −10 且非全局
//!   岩浆时，lavaNoise 在 (x/64, y/40, z/64) 格采样，|v|>0.3 → 该胞
//!   流体改 LAVA。全局分界 `NoiseBasedChunkGenerator.java:69-81`
//!   `createFluidPicker`：y < min(−54, seaLevel) → FluidStatus(−54,
//!   LAVA)，否则 FluidStatus(seaLevel=63, WATER)；`FluidStatus.at(y) =
//!   y < fluidLevel ? fluid : AIR`（Aquifer.java:52-56）。
//! - 噪声参数（data/minecraft/worldgen/noise/*.json，amplitudes 均
//!   [1.0]）：aquifer_barrier firstOctave −3、fluid_level_floodedness
//!   −7、fluid_level_spread −5、lava −1；路由缩放
//!   NoiseRouterData.java:319-322 + DensityFunctions.Noise 语义
//!   （DensityFunctions.java:861-875；单 double 参重载 = xzScale 1.0，
//!   :173-175）：barrier (x, 0.5y, z)、floodedness (x, 0.67y, z)、
//!   spread (x, 0.7142857y, z)、lava (x, y, z)。
//!
//! ## 结构复刻裁决（非字节级同原版，登记 docs/KNOWN-DIVERGENCE.md AQUI-1）
//! 逐行复刻原版噪声数学在**端到端字节对拍**上不成立：传入本判据的
//! density 本身即 clean-room 管线（vanilla/noise.rs 头注：域盐 splitmix
//! 自有派生，非原版 Xoroshiro fork 序），`density + barrier > 0` 的
//! 字节级复刻需要 density、preliminary surface、Xoroshiro 位置随机
//! （RandomState.java:38 fromHashOf("aquifer") 内部序）、Java
//! StringHashing 种子链全部同真，任一不合即输出不等——单复刻含水层
//! 噪声内核得不到任何端到端一致性，只锁死演进。故本波 = **结构复刻**：
//! 同胞格几何/阈值/相似度/压力数学、同噪声参数（firstOctave/振幅表/
//! 路由缩放逐一照抄），域盐自有（TREE_DOMAIN 先例），胞心随机用
//! splitmix+Lemire 无偏整除（替代 Xoroshiro 三连 nextInt，同分布）。
//! 统计形态由 tests/aquifer.rs 对拍门锁定（干洞穴占比/囊袋并存/三态）。
//!
//! 其余裁剪（同登记）：
//! - `shouldScheduleFluidUpdate`/`markPosForPostprocessing`
//!   （Aquifer.java:160/246 等，原版水流更新调度）无对应存储面——不建模；
//! - preliminary surface 用密度场解析过零面 `crossing`（terrain.rs 实顶
//!   扫描同款），非原版 preliminary_surface_level 密度函数（本仓 density
//!   管线未移植该函数；两者同为「无洞穴地表高度估计」，语义对位）。

use crate::vanilla::noise::{NormalNoise, splitmix64};
use crate::vanilla::terrain::Orchestrator;
use crate::vanilla::{MIN_Y, SEA};
use std::collections::HashMap;

/// 全局岩浆面（26.1 `NoiseBasedChunkGenerator.java:70` 硬编码 −54，
/// 与 seaLevel 取 min 后作分界，:77）。
pub const LAVA_SEA_LEVEL: i32 = -54;
/// 干胞哨兵液面（DimensionType.WAY_BELOW_MIN_Y = MIN_Y << 4，
/// DimensionType.java:54 语义；MIN_Y=−64 → −1024）。
pub const WAY_BELOW: i32 = MIN_Y << 4;

/// 开放柱（实顶以上大气柱）的无条件全局流体——#175 终修
/// （geometry_locks::vanilla_surface_bounds_and_sea_fill 裁决：洋面
/// 水体恒灌满，含水层胞格噪声只作用于地表以下岩层空腔）。原版
/// 「修正 preliminary 面（= prelim+8，`adjustSurfaceLevel`，
/// Aquifer.java:441-442）以上 = globalFluid」由两段无条件路合成：
/// - 区块免采样线（= max prelim+8+12 网格化，:136-138）以上
///   `computeSubstance` 直返 `globalFluid.at(posY)`（:159-161），
///   不进胞格逻辑；
/// - 胞体全在修正面上（`bottomOfAquiferCell > adj` → 直返
///   globalFluid，:416-417）或戳出修正面且面上有流体（→ 直返
///   globalFluidAtSurface，:420-429）——floodedness 噪声门
///   （computeSurfaceLevel :445+）只有面上全部岩层内的胞才可达。
/// globalFluid = seaStatus/lavaStatus（NoiseBasedChunkGenerator.java:
/// 69-81，`FluidStatus.at` y<fluidLevel 出流体，Aquifer.java:52-55）。
/// 本仓以**本列实顶扫描位**（terrain.rs `hit_solid` 门，
/// `column_real_top` 与 lake_pass `Replay` 同型共用）为面判据——
/// 原版 skip 线取区块最大 prelim（邻列抬高本列的线），但洋面恒灌
/// 结论不依赖该最大值（低列自身的 pokes/global 路同为恒水），
/// 两式在锁面等价。液面含 y=SEA 行为本仓几何契约（geometry_locks
/// 锁死，承刀 A 前平铺灌水面；原版最高水行 = seaLevel−1）。
#[must_use]
pub const fn open_column(y: i32) -> Substance {
    if y > SEA {
        Substance::Air
    } else if y < LAVA_SEA_LEVEL {
        Substance::Lava // 全局岩浆海（lavaStatus，:70/:78）
    } else {
        Substance::Water // 洋面恒灌满段
    }
}

/// 终判材质（原版 null=石头 / AIR / WATER / LAVA 四态）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Substance {
    /// 压力封口袋：density≤0 但原位出 defaultBlock=STONE。
    Rock,
    Air,
    Water,
    Lava,
}

/// `Aquifer.FluidStatus`（Aquifer.java:52-56）：液面 + 流体种类。
#[derive(Clone, Copy)]
pub struct FluidStatus {
    pub level: i32,
    pub kind: Substance,
}

impl FluidStatus {
    /// `FluidStatus.at`：y < fluidLevel → 流体，否则空气。
    #[must_use]
    pub const fn at(self, y: i32) -> Substance {
        if y < self.level {
            self.kind
        } else {
            Substance::Air
        }
    }
}

/// 全局流体选择器（NoiseBasedChunkGenerator.java:69-81，x/z 不参与）。
#[must_use]
pub const fn global_fluid(y: i32) -> FluidStatus {
    if y < LAVA_SEA_LEVEL {
        FluidStatus {
            level: LAVA_SEA_LEVEL,
            kind: Substance::Lava,
        }
    } else {
        FluidStatus {
            level: SEA,
            kind: Substance::Water,
        }
    }
}

/// 列探针 = 原版 preliminarySurfaceLevel 列缓存（NoiseChunk.java:218-227）
/// 的语义替身：crossing（解析过零面）+ 含水层要用的两条密度通道列值。
#[derive(Clone, Copy)]
pub struct ColumnProbe {
    /// preliminary surface（crossing 口径，+8 在采样处施加 =
    /// Aquifer.java:441-443 adjustSurfaceLevel）。
    pub prelim: i32,
    /// offset 通道（deep dark 的 depth = 梯度 + offset，
    /// NoiseRouterData.java:180-182）。
    pub offset: f64,
    /// erosion 通道（OverworldBiomeBuilder.java:1009）。
    pub erosion: f64,
}

static AMPS_ONE: [f64; 1] = [1.0];

/// 域盐（TREE_DOMAIN 先例：同世界种子下各通道派生互不相同）。
/// 非原版 Xoroshiro 种子链（RandomState.java:38），登记 AQUI-1。
const SALT_BARRIER: u64 = 0x0000_A01F_0001;
const SALT_FLOODED: u64 = 0x0000_A01F_0002;
const SALT_SPREAD: u64 = 0x0000_A01F_0003;
const SALT_LAVA: u64 = 0x0000_A01F_0004;
const SALT_CENTER: u64 = 0x0000_A01F_00FF;

/// 四条含水层密度通道（NoiseRouterData.java:319-322 + noise json 参数）。
pub struct Aquifer {
    barrier: NormalNoise,
    floodedness: NormalNoise,
    spread: NormalNoise,
    lavan: NormalNoise,
    center_seed: u64,
}

impl Aquifer {
    #[must_use]
    pub fn new(world_seed: u64) -> Self {
        Self {
            // firstOctave/振幅表逐字抄 data/minecraft/worldgen/noise/*.json
            barrier: NormalNoise::new(world_seed, SALT_BARRIER, -3, &AMPS_ONE),
            floodedness: NormalNoise::new(world_seed, SALT_FLOODED, -7, &AMPS_ONE),
            spread: NormalNoise::new(world_seed, SALT_SPREAD, -5, &AMPS_ONE),
            lavan: NormalNoise::new(world_seed, SALT_LAVA, -1, &AMPS_ONE),
            center_seed: splitmix64(world_seed ^ SALT_CENTER),
        }
    }

    /// DensityFunctions.Noise 求值（xz/y 缩放见模块头注；本仓
    /// NormalNoise::get_value 吃已缩放坐标）。
    fn barrier_at(&self, x: i32, y: i32, z: i32) -> f64 {
        self.barrier
            .get_value(f64::from(x), f64::from(y) * 0.5, f64::from(z))
    }

    /// floodedness，原版 Mth.clamp(v, -1, 1)（Aquifer.java:463）。
    fn flooded_at(&self, x: i32, y: i32, z: i32) -> f64 {
        self.floodedness
            .get_value(f64::from(x), f64::from(y) * 0.67, f64::from(z))
            .clamp(-1.0, 1.0)
    }

    fn spread_at(&self, cx: i32, cy: i32, cz: i32) -> f64 {
        self.spread.get_value(
            f64::from(cx),
            f64::from(cy) * 0.714_285_714_285_714_3,
            f64::from(cz),
        )
    }

    fn lava_at(&self, cx: i32, cy: i32, cz: i32) -> f64 {
        self.lavan
            .get_value(f64::from(cx), f64::from(cy), f64::from(cz))
    }

    /// 胞心（Aquifer.java:199-207：原点 + nextInt(10/9/10)）。胞格
    /// 16×12×16（X/Y/Z_SPACING，:67-69）；随机三元 = splitmix 链 +
    /// Lemire 高位乘无偏归约（替代 Xoroshiro 三连 nextInt，登记 AQUI-1）。
    #[must_use]
    pub fn cell_center(&self, gx: i32, gy: i32, gz: i32) -> (i32, i32, i32) {
        let mix = {
            let a = u64::from(gx.cast_unsigned()).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let b = u64::from(gy.cast_unsigned())
                .wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
                .rotate_left(17);
            let c = u64::from(gz.cast_unsigned())
                .wrapping_mul(0x1656_67B1_9E37_79F9)
                .rotate_left(31);
            splitmix64(self.center_seed ^ a ^ b ^ c)
        };
        let h1 = splitmix64(mix);
        let h2 = splitmix64(h1);
        let h3 = splitmix64(h2);
        // Lemire：floor(n · r / 2^64)（trees.rs:86 同款无偏整除）
        let r10x = ((h1 as u128 * 10) >> 64) as i32;
        let r9y = ((h2 as u128 * 9) >> 64) as i32;
        let r10z = ((h3 as u128 * 10) >> 64) as i32;
        (gx * 16 + r10x, gy * 12 + r9y, gz * 16 + r10z)
    }

    /// barrier 压力（calculatePressure，Aquifer.java:304-365）。
    /// `barrier_noise` 槽对齐原版 MutableDouble（同一子块内 ≤3 次压力
    /// 评估共享一次噪声采样，:268/:275/:281 传同一 ref）。
    fn pressure(
        &self,
        x: i32,
        y: i32,
        z: i32,
        barrier_noise: &mut Option<f64>,
        a: FluidStatus,
        b: FluidStatus,
    ) -> f64 {
        // 水/岩浆异种相邻 → 常数 2.0（:312-313,361-363）
        let clash = (a.kind == Substance::Lava && b.kind == Substance::Water)
            || (a.kind == Substance::Water && b.kind == Substance::Lava);
        if clash {
            return 2.0;
        }
        let diff = (a.level - b.level).abs();
        if diff == 0 {
            return 0.0;
        }
        let avg = 0.5 * (a.level + b.level) as f64;
        let h = f64::from(y) + 0.5 - avg;
        let edge = f64::from(diff) / 2.0 - h.abs(); // distanceFromBarrierEdgeTowardsMiddle
        let grad = if h > 0.0 {
            let cp = edge; // topBias = 0.0（:330-337）
            if cp > 0.0 { cp / 1.5 } else { cp / 2.5 }
        } else {
            let cp = 3.0 + edge; // bottomBias = 3.0（:338-345）
            if cp > 0.0 { cp / 3.0 } else { cp / 10.0 }
        };
        // |grad| > 2 不采噪声（:352-360）
        let noise = if grad.abs() <= 2.0 {
            *barrier_noise.get_or_insert_with(|| self.barrier_at(x, y, z))
        } else {
            0.0
        };
        2.0 * (noise + grad)
    }
}

/// 表面采样偏移（13 列，Aquifer.java:92-95，单位=区块）。
const SURFACE_OFFSETS: [(i32, i32); 13] = [
    (0, 0),
    (-2, -1),
    (-1, -1),
    (0, -1),
    (1, -1),
    (-3, 0),
    (-2, 0),
    (-1, 0),
    (1, 0),
    (-2, 1),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// 胞格键（网格坐标三元组）。
type CellKey = (i32, i32, i32);
/// 胞格缓存条目：(胞心, 状态惰性位)——对齐原版
/// aquiferLocationCache/aquiferCache 双层惰性。
type CellEntry = (CellKey, Option<FluidStatus>);

/// 一区块（一次 `generate` 调用）作用域的含水层现场：列探针缓存
/// （= NoiseChunk.preliminarySurfaceLevelCache）+ 胞格状态缓存 +
/// 免采样线。纯世界坐标函数——缓存只省算力，不改语义。
pub struct AquiferField<'a> {
    aq: &'a Aquifer,
    orch: &'a Orchestrator,
    /// key = quart 量化列 (x & !3, z & !3)（NoiseChunk.java:219-221）
    cols: HashMap<(i32, i32), ColumnProbe>,
    /// key = 胞格坐标；value = (胞心, 状态)（状态惰性算，对齐
    /// aquiferCache/aquiferLocationCache 双层惰性，Aquifer.java:80-82）
    cells: HashMap<CellKey, CellEntry>,
    /// 各区块的 skipSamplingAboveY（Aquifer.java:134-137）缓存：此线以上
    /// 不再采样胞格，直接用全局流体（地表水体/海面空气由这条路径给出）。
    /// 该线是**区块窗口**函数（原版 NoiseChunk 每区块算一次），体素判据
    /// 必须用**所属区块**的线——`column_real_top` 跨块重放据此取树干列
    /// 所属区块的线，保证与缓冲构建侧逐字节一致。
    skips: HashMap<(i32, i32), i32>,
}

impl<'a> AquiferField<'a> {
    #[must_use]
    pub fn new(aq: &'a Aquifer, orch: &'a Orchestrator) -> Self {
        Self {
            aq,
            orch,
            cols: HashMap::new(),
            cells: HashMap::new(),
            skips: HashMap::new(),
        }
    }

    /// 区块免采样线（`base_x/base_z` = 区块原点）：
    /// fromGridY(gridY(maxPrelim+8+12)+1, 11) − 1（Aquifer.java:134-137，
    /// maxPreliminarySurfaceLevel 沿区块扩展窗 4 步采样，
    /// NoiseChunk.java:202-215）。列探针走共享缓存，边际成本只在新增列。
    pub fn chunk_skip(&mut self, base_x: i32, base_z: i32) -> i32 {
        let key = (base_x, base_z);
        if let Some(&s) = self.skips.get(&key) {
            return s;
        }
        let min_gx = (base_x - 5) >> 4;
        let max_gx = ((base_x + 15 - 5) >> 4) + 1;
        let min_gz = (base_z - 5) >> 4;
        let max_gz = ((base_z + 15 - 5) >> 4) + 1;
        let x0 = min_gx * 16;
        let x1 = max_gx * 16 + 9; // fromGridX(maxGridX, 9)
        let z0 = min_gz * 16;
        let z1 = max_gz * 16 + 9;
        let mut max_prelim = i32::MIN;
        let mut sx = x0;
        while sx <= x1 {
            let mut sz = z0;
            while sz <= z1 {
                max_prelim = max_prelim.max(self.probe(sx, sz).prelim);
                sz += 4;
            }
            sx += 4;
        }
        let max_adj = max_prelim + 8; // adjustSurfaceLevel
        let grid_y = (max_adj + 12).div_euclid(12) + 1;
        let s = grid_y * 12 + 11 - 1; // fromGridY(grid, 11) - 1
        self.skips.insert(key, s);
        s
    }

    /// 列探针（quart 量化 + 缓存）。
    fn probe(&mut self, x: i32, z: i32) -> ColumnProbe {
        let key = (x & !3, z & !3);
        if let Some(p) = self.cols.get(&key) {
            return *p;
        }
        let p = self.orch.aquifer_probe(x, z);
        self.cols.insert(key, p);
        p
    }

    /// 胞格流体状态（getAquiferStatus → computeFluid，Aquifer.java:
    /// 382-440）。
    fn cell_status(&mut self, gx: i32, gy: i32, gz: i32) -> FluidStatus {
        let key = (gx, gy, gz);
        if let Some((_, Some(s))) = self.cells.get(&key) {
            return *s;
        }
        let center = self.ensure_center((gx, gy, gz));
        let (cx, cy, cz) = center;
        let s = self.compute_fluid(cx, cy, cz);
        self.cells
            .entry(key)
            .and_modify(|e| e.1 = Some(s))
            .or_insert((center, Some(s)));
        s
    }

    /// `computeFluid`（Aquifer.java:396-440）：胞心处 13 列地表采样定
    /// 「全局流体 / 噪声囊袋 / 干」。
    fn compute_fluid(&mut self, cx: i32, cy: i32, cz: i32) -> FluidStatus {
        let global = global_fluid(cy);
        let top = cy + 12; // topOfAquiferCell（:402）
        let bottom = cy - 12; // bottomOfAquiferCell（:403）
        let mut surface_under_global = false; // surfaceAtCenterIsUnderGlobalFluidLevel（:404）
        let mut lowest = i32::MAX; // lowestPreliminarySurface（:405）
        for &(ox, oz) in &SURFACE_OFFSETS {
            let px = cx + ox * 16; // SectionPos.sectionToBlockCoord（:409）
            let pz = cz + oz * 16;
            let prelim = self.probe(px, pz).prelim;
            let adj = prelim + 8;
            let start = ox == 0 && oz == 0;
            // 胞底已在中心地表之上 → 纯地表上空间，全局流体（:412-414）
            if start && bottom > adj {
                return global;
            }
            let pokes = top > adj; // topOfAquiferCellPokesAboveSurface（:416）
            if pokes || start {
                let g = global_fluid(adj);
                if g.at(adj) != Substance::Air {
                    if start {
                        surface_under_global = true;
                    }
                    if pokes {
                        return g; // 胞戳出低洼（低于海平面）地表 → 全局水体（:421-423）
                    }
                }
            }
            lowest = lowest.min(prelim);
        }
        self.compute_surface_level(cx, cy, cz, global, lowest, surface_under_global)
    }

    /// `computeSurfaceLevel`（Aquifer.java:445-480）：floodedness 双阈值
    /// 定液面（WAY_BELOW = 干）。
    fn compute_surface_level(
        &mut self,
        cx: i32,
        cy: i32,
        cz: i32,
        global: FluidStatus,
        lowest: i32,
        under: bool,
    ) -> FluidStatus {
        let (partial, full) = {
            // deep dark 门（OverworldBiomeBuilder.java:1008-1010）：
            // depth = yClampedGradient(−64,320,1.5,−1.5) + offset
            let p = self.probe(cx, cz);
            let depth = 1.5
                - 3.0
                    * ((cy - MIN_Y) as f64 / f64::from(mcv_core::WORLD_MAX_Y - MIN_Y))
                        .clamp(0.0, 1.0)
                + p.offset;
            if p.erosion < -0.225 && depth > 0.9 {
                (-1.0, -1.0)
            } else {
                let below = f64::from(lowest + 8 - cy);
                let factor = if under {
                    (1.0 - below / 64.0).clamp(0.0, 1.0) // Mth.clampedMap（:462）
                } else {
                    0.0
                };
                let n = self.aq.flooded_at(cx, cy, cz);
                // Mth.map(factor,1,0,−0.3/−0.8, 0.8/0.4)（:464-466）
                let full_th = 0.8 - 1.1 * factor;
                let partial_th = 0.4 - 1.2 * factor;
                (n - partial_th, n - full_th)
            }
        };
        let level = if full > 0.0 {
            global.level // 全淹 = 全局海平面（:471-472）
        } else if partial > 0.0 {
            self.randomized_level(cx, cy, cz, lowest) // :473-475
        } else {
            WAY_BELOW // :476-477
        };
        let kind = self.fluid_type(cx, cy, cz, global, level);
        FluidStatus { level, kind }
    }

    /// `computeRandomizedFluidSurfaceLevel`（Aquifer.java:482-496）：
    /// 16×40×16 胞，液面 = min(最低地表, 胞中 y + spread 噪声量化)。
    fn randomized_level(&mut self, cx: i32, cy: i32, cz: i32, lowest: i32) -> i32 {
        let fx = cx.div_euclid(16);
        let fy = cy.div_euclid(40);
        let fz = cz.div_euclid(16);
        let mid = fy * 40 + 20;
        let spread = self.aq.spread_at(fx, fy, fz) * 10.0;
        let quantized = 3 * spread.div_euclid(3.0).floor() as i32; // Mth.quantize(v,3)
        // 原版 return Math.min(lowestPreliminarySurface, target)（:495）
        (mid + quantized).min(lowest)
    }

    /// `computeFluidType`（Aquifer.java:498-513）：半淹深液面（≤−10）胞
    /// 按 lavaNoise 格（64×40×64）翻 LAVA。
    fn fluid_type(
        &mut self,
        cx: i32,
        cy: i32,
        cz: i32,
        global: FluidStatus,
        level: i32,
    ) -> Substance {
        if level <= -10 && level != WAY_BELOW && global.kind != Substance::Lava {
            let v = self
                .aq
                .lava_at(cx.div_euclid(64), cy.div_euclid(40), cz.div_euclid(64));
            if v.abs() > 0.3 {
                return Substance::Lava;
            }
        }
        global.kind
    }

    /// 终判（computeSubstance，Aquifer.java:149-296）。`density` 必须
    /// ≤0（调用点在密度≤0 分支；原版同款前置，:150-152）。`skip` =
    /// 体素**所属区块**的免采样线（[`AquiferField::chunk_skip`]）。
    pub fn substance(&mut self, x: i32, y: i32, z: i32, density: f64, skip: i32) -> Substance {
        let global = global_fluid(y);
        if y > skip {
            return global.at(y);
        }
        if global.at(y) == Substance::Lava {
            return Substance::Lava; // 全局岩浆带（:167-170）
        }
        // 12 邻胞取最近 4（:180-230）
        let ax = (x - 5) >> 4; // gridX(posX + SAMPLE_OFFSET_X)
        let ay = (y + 1).div_euclid(12); // gridY(posY + 1)
        let az = (z - 5) >> 4;
        let mut d = [i32::MAX; 4]; // 距离平方 4 强
        let mut c = [(0, 0, 0); 4];
        for lx in 0..=1 {
            for ly in -1..=1 {
                for lz in 0..=1 {
                    let g = (ax + lx, ay + ly, az + lz);
                    let center = self.ensure_center(g);
                    let dx = center.0 - x;
                    let dy = center.1 - y;
                    let dz = center.2 - z;
                    let dist = dx * dx + dy * dy + dz * dz;
                    for s in 0..4 {
                        if dist <= d[s] {
                            d[s..].rotate_right(1);
                            c[s..].rotate_right(1);
                            d[s] = dist;
                            c[s] = g;
                            break;
                        }
                    }
                }
            }
        }
        let s1 = self.cell_status(c[0].0, c[0].1, c[0].2);
        let sim12 = 1.0 - f64::from(d[1] - d[0]) / 25.0; // similarity（:299-301）
        let fluid = s1.at(y);
        if sim12 <= 0.0 {
            return fluid; // 最近胞一票通过（:239-248）
        }
        // 水格压岩浆上界 → 直接水（原版蒸汽面处理，:246-249）
        if fluid == Substance::Water && global_fluid(y - 1).at(y - 1) == Substance::Lava {
            return Substance::Water;
        }
        let s2 = self.cell_status(c[1].0, c[1].1, c[1].2);
        let mut bn = None;
        let b12 = sim12 * self.aq.pressure(x, y, z, &mut bn, s1, s2);
        if density + b12 > 0.0 {
            return Substance::Rock; // 压力封口袋（:255-258）
        }
        let s3 = self.cell_status(c[2].0, c[2].1, c[2].2);
        let sim13 = 1.0 - f64::from(d[2] - d[0]) / 25.0;
        if sim13 > 0.0 {
            let b13 = sim12 * sim13 * self.aq.pressure(x, y, z, &mut bn, s1, s3);
            if density + b13 > 0.0 {
                return Substance::Rock; // :262-267
            }
        }
        let sim23 = 1.0 - f64::from(d[2] - d[1]) / 25.0;
        if sim23 > 0.0 {
            let b23 = sim12 * sim23 * self.aq.pressure(x, y, z, &mut bn, s2, s3);
            if density + b23 > 0.0 {
                return Substance::Rock; // :269-274
            }
        }
        fluid // 三对全漏网 → 出流体（:295）
    }

    /// 胞心惰性缓存（对齐 aquiferLocationCache，Aquifer.java:81/196-205）。
    fn ensure_center(&mut self, g: (i32, i32, i32)) -> (i32, i32, i32) {
        if let Some((center, _)) = self.cells.get(&g) {
            return *center;
        }
        let center = self.aq.cell_center(g.0, g.1, g.2);
        self.cells.insert(g, (center, None));
        center
    }
}
