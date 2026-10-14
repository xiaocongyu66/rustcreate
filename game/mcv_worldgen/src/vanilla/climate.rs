//! vanilla 气候通道采样器 — clean-room 对齐 26.1 `DensityFunctions.java` 的
//! ShiftedNoise 编排与 `worldgen/noise/*.json` 的噪声参数表。
//!
//! 编排语义（引用机制 ≠ 搬运代码）：
//! - SHIFT（Noises.SHIFT = createKey("offset")，Noises.java:20；offset.json
//!   firstOctave −3、amplitudes [1,1,1,0]）：shift_x/shift_z **共用同一**
//!   NormalNoise 实例（NoiseRouterData.java:81/84 两次
//!   `noises.getOrThrow(Noises.SHIFT)` 同一 Holder → visitNoise 的
//!   computeIfAbsent 同一实例，RandomState.java:112-114）；两包装的采样
//!   点不同——ShiftA = `compute(blockX, 0, blockZ)`、ShiftB =
//!   `compute(blockZ, blockX, 0)`（DensityFunctions.java:1000-1043，
//!   shift_z **x/z 轴对调**）；ShiftNoise.compute =
//!   `offset.getValue(localX·0.25, localY·0.25, localZ·0.25)·4`
//!   （:1058-1062，×4 乘子）；
//! - 通道值 = noise.getValue(blockX·0.25 + shiftX, ·, blockZ·0.25 + shiftZ)
//!   （DensityFunctions.ShiftedNoise.compute:1091-1097，xzScale = 0.25）；
//! - 通道种子链 = 原版 fork 链：`root_positional_factory(worldSeed)`（
//!   RandomState.java:36 根位置随机工厂）→ `Noises.instantiate` =
//!   `NormalNoise.create(root.fromHashOf("minecraft:{name}"), params)`
//!   （Noises.java:78-83 + RandomState.java:112-114 getOrCreateNoise）——
//!   [`NormalNoise::from_vanilla_name`]，同 seed 与官方 jar 逐位同位型
//!   （tools/fork-chain-mirror-run.txt 3 seed×10 通道×12 点 360/360）；
//! - continentalness / erosion / ridge(weirdness) 与 temperature / vegetation
//!   共用同一 SHIFT 偏移（NoiseRouterData:89-97/325-331）。
//!
//! 波长真相（地形保真波 T2 还原，任务卡 #122 / triage ①）：xzScale 0.25 ×
//! firstOctave −9 ⇒ 大陆度/侵蚀波长 2048 格、山脊 512、温度 4096（−10 首层）
//! ——原版生存的大地貌尺度。旧「可步行尺度 ÷8 压缩（XS=2.0）」取舍随世界
//! 升 384 作废：÷8 后盆带每 ~256 格穿越一遍，平原窗占比 0.054 vs 原版口径
//! 0.41（/root/audit/_sim3.py 移植复算）。

use crate::vanilla::noise::NormalNoise;
use crate::vanilla::spline::Climate;
use crate::vanilla::xoroshiro::root_positional_factory;

/// 五参数/温湿度通道采样缩放（26.1 overworld 各通道 json `xz_scale: 0.25`，
/// NoiseRouterData.java:90/93/96/325-328）。
const XS: f64 = 0.25;

/// 振幅表——**逐字抄 data/minecraft/worldgen/noise/*.json，含尾部零填充**。
/// 表长进 PerlinNoise.lowestFreqValueFactor = 2^(n−1)/(2^n−1) 归一
/// （PerlinNoise.java:128，octaves = amplitudes.size()），截短会把首倍频
/// 权重系统性抬高（temperature 6→3 项时 ×1.125）——python 镜像 360/360
/// 位型门的先决条件（expectedDeviation 才按非零倍频跨度计，两级归一各管
/// 各的，NormalNoise.java:59-70/127）。
const AMPS_OFFSET: [f64; 4] = [1.0, 1.0, 1.0, 0.0];
const AMPS_CONTINENTALNESS: [f64; 9] = [1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0];
const AMPS_EROSION: [f64; 5] = [1.0, 1.0, 0.0, 1.0, 1.0];
const AMPS_RIDGE: [f64; 6] = [1.0, 2.0, 1.0, 0.0, 0.0, 0.0];
const AMPS_TEMPERATURE: [f64; 6] = [1.5, 0.0, 1.0, 0.0, 0.0, 0.0];
const AMPS_VEGETATION: [f64; 6] = [1.0, 1.0, 0.0, 0.0, 0.0, 0.0];

/// 每列气候快照（供样条求值与群系选择）。
#[derive(Clone, Copy, Debug)]
pub struct ClimateSample {
    pub continents: f64,
    pub erosion: f64,
    /// weirdness（未折叠）。
    pub ridges: f64,
    pub ridges_folded: f64,
    pub temperature: f64,
    pub vegetation: f64,
}

impl ClimateSample {
    /// 供 density 样条求值的输入（只含三轴气候）。
    #[must_use]
    pub const fn spline_climate(&self) -> Climate {
        Climate {
            continents: self.continents,
            erosion: self.erosion,
            ridges: self.ridges,
            ridges_folded: self.ridges_folded,
        }
    }
}

pub struct ClimateSampler {
    /// offset 噪声（Noises.SHIFT）——shift_x/shift_z 共用此一实例
    ///（头注 SHIFT 条：NoiseRouterData.java:81/84 同一 Holder）。
    offset: NormalNoise,
    continentalness: NormalNoise,
    erosion: NormalNoise,
    ridge: NormalNoise,
    temperature: NormalNoise,
    vegetation: NormalNoise,
}

impl ClimateSampler {
    /// 气候通道全部为 2D 切片采样（sample() 中 y 恒 0；vanilla
    /// ShiftedNoise/ShiftNoise.compute 亦 y·0.25 归零，NoiseRouterData.java:89-97
    /// 与 DensityFunctions.java ShiftNoise 接口）。底层内核为
    /// [`crate::vanilla::perlin::ImprovedNoise`]（原版机制本体，第四棒换代）：
    /// vanilla `DensityFunctions.shiftedNoise2d` 的 yScale=0 即按 (x,0,z) 求值
    /// （DensityFunctions.java:159-163），ImprovedNoise 直接采样无任何校准。
    /// 种子链 = 原版 fork 链（头注「通道种子链」条；通道名 =
    /// Noises.java:11-19/20 实名，随机源 = 根位置随机工厂 fromHashOf）。
    #[must_use]
    pub fn new(world_seed: u64) -> Self {
        let root = root_positional_factory(world_seed as i64);
        let ch = |name: &str, fo: i32, amps: &'static [f64]| {
            NormalNoise::from_vanilla_name(&root, name, fo, amps)
        };
        Self {
            offset: ch("minecraft:offset", -3, &AMPS_OFFSET),
            continentalness: ch("minecraft:continentalness", -9, &AMPS_CONTINENTALNESS),
            erosion: ch("minecraft:erosion", -9, &AMPS_EROSION),
            ridge: ch("minecraft:ridge", -7, &AMPS_RIDGE),
            temperature: ch("minecraft:temperature", -10, &AMPS_TEMPERATURE),
            vegetation: ch("minecraft:vegetation", -8, &AMPS_VEGETATION),
        }
    }

    /// 采样一列气候（ShiftedNoise 式：先算 SHIFT 偏移，再查通道值）。
    #[must_use]
    pub fn sample(&self, block_x: i32, block_z: i32) -> ClimateSample {
        let bx = f64::from(block_x);
        let bz = f64::from(block_z);
        // SHIFT 偏移（同一 offset 实例双包装，头注 SHIFT 条）：
        // ShiftA = offset.getValue(bx·0.25, 0, bz·0.25)·4、
        // ShiftB = offset.getValue(bz·0.25, bx·0.25, 0)·4——后者 x/z 轴对调
        //（DensityFunctions.java:1000-1022 + ShiftNoise.compute:1058-1062）。
        let sx = self.offset.get_value(bx * XS, 0.0, bz * XS) * 4.0;
        let sz = self.offset.get_value(bz * XS, bx * XS, 0.0) * 4.0;
        let continents = self
            .continentalness
            .get_value(bx * XS + sx, 0.0, bz * XS + sz);
        let erosion = self.erosion.get_value(bx * XS + sx, 0.0, bz * XS + sz);
        let ridges = self.ridge.get_value(bx * XS + sx, 0.0, bz * XS + sz);
        let temperature = self.temperature.get_value(bx * XS + sx, 0.0, bz * XS + sz);
        let vegetation = self.vegetation.get_value(bx * XS + sx, 0.0, bz * XS + sz);
        ClimateSample {
            continents,
            erosion,
            ridges,
            ridges_folded: crate::vanilla::peaks_and_valleys(ridges),
            temperature,
            vegetation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 采样确定性 + 不同列不同值 + 值域哨兵（归一通道近似 [-1,1]）。
    #[test]
    fn climate_samples_deterministic_and_bounded() {
        let s = ClimateSampler::new(2024);
        for k in 0..32_i32 {
            let a = s.sample(k * 13, k * 7);
            let b = s.sample(k * 13, k * 7);
            assert_eq!(a.continents.to_bits(), b.continents.to_bits());
            assert_eq!(a.ridges_folded.to_bits(), b.ridges_folded.to_bits());
            for v in [
                a.continents,
                a.erosion,
                a.ridges,
                a.temperature,
                a.vegetation,
            ] {
                assert!(v.abs() <= 4.0, "气候通道越界 {v}");
            }
            let folded = crate::vanilla::peaks_and_valleys(a.ridges);
            assert!((a.ridges_folded - folded).abs() < 1e-12);
            // RIDGES 即 shiftedNoise2d 原始噪声（NoiseRouterData.java:96-98），
            // 无 squeezed/mapped 折叠：p(r)=−3(|‖r‖−2/3|−1/3) 在 |r|≤1 段上界
            // +1（r=±2/3 处），|r|>1 段随原始噪声无界延伸——以通道哨兵
            // |ridges|≤4.0（振幅和 1+2+1）推导折叠值域 [−9, 1]。
            assert!(
                a.ridges_folded <= 1.0 + 1e-9 && a.ridges_folded >= -9.0 - 1e-9,
                "折叠值域 {a:?}"
            );
        }
    }

    /// 分块尺度上气候轴确有变化（≠ 全平）：±4096 格网格（256 格步距）上
    /// continentalness 跨越 0 两侧。窗口随 T2 波长还原同步扩——原 384 格
    /// 路径只有 0.2 个大陆度波长（2048 格），恢复原版尺度后不具代表性。
    #[test]
    fn continentalness_varies_across_region() {
        let s = ClimateSampler::new(7);
        let mut saw_neg = false;
        let mut saw_pos = false;
        for dz in -16..16 {
            for dx in -16..16 {
                let v = s.sample(dx * 256, dz * 256).continents;
                saw_neg |= v < -0.1;
                saw_pos |= v > 0.1;
            }
        }
        assert!(
            saw_neg && saw_pos,
            "continentalness 退化常数（±4096 网格未跨 0）"
        );
    }
}
