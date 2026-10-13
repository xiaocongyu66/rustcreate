//! #192 深板岩层 + 基岩五层——overworld.json surface_rule 实物行为锁。
//!
//! 取证锚点（26.1 原版，/root/mc-ref/src-26.1，铁律 1）：
//! - **基岩五层**：`data/minecraft/worldgen/noise_settings/overworld.json`
//!   surface_rule top[0]（:446-463）= condition `vertical_gradient`
//!   `true_at_and_below={above_bottom:0}`、`false_at_and_above={above_bottom:5}`、
//!   `random_name="minecraft:bedrock_floor"` → block bedrock。判定语义 =
//!   `SurfaceRules.java:758-781 VerticalGradientCondition.compute`：
//!   y≤true 恒真、y≥false 恒假、之间 `Mth.map(y,true,false,1,0)` 线性概率
//!   与逐位 `nextFloat()<p` 比较 → y=−64 恒基岩、−63..−60 概率
//!   0.8/0.6/0.4/0.2、≥−59 恒假。**「y=−64..−62 三层全实心」是 1.18 前旧
//!   BedrockFeature 记忆（旧代码注释同源），26.1 实物 −63/−62 只有
//!   0.8/0.6 概率**——本锁按实物阈值推期望区间（任务书「按原版阈值推期望
//!   区间」条款）。26.1 JSON 无 bedrock_roof 项（全文 bedrock 仅 2 处，均
//!   属 floor）→ 世界顶部不得出现基岩。
//! - **深板岩层**：同 JSON surface_rule 树尾 top[2] = condition
//!   `vertical_gradient` `true_at_and_below={absolute:0}`、
//!   `false_at_and_above={absolute:8}`、`random_name="minecraft:deepslate"`
//!   → block deepslate（axis=y）。y≤0 恒深板岩、1..7 概率 (8−y)/8、
//!   y≥8 恒假——1.18 快照期的「8..19 上浮斑块」不在 26.1 实物，本锁按实物
//!   取 y≥8 零深板岩。规则只作用 defaultBlock=石头位
//!   （SurfaceSystem.java:147）：表层草/土/沙不替换；y≤8 的洞壁/洞底石头
//!   照换（buildSurface 自顶扫到 min y，非仅表层带）。
//! - **序列次序**：bedrock（top[0]）先于 deepslate（top[2]）→ 基岩散点带
//!   未中位落 deepslate 项（y≤0 恒真）出深板岩，带内无石头。
//!
//! 随机链：本仓位置随机 = hash01 通道域（KNOWN-DIVERGENCE RNG-1，AQUI-1
//! 先例），非 Xoroshiro fromHashOf(name) 种子链——统计带锁替代字节对拍。

use mcv_core::{CHUNK_VOL, ChunkPos, WORLD_MAX_Y, WORLD_MIN_Y};
use mcv_worldgen::vanilla::terrain::{BEDROCK, DEEPSLATE, STONE};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

/// 主测种子（surface_rules.rs / quality.rs 同源）。
const SEED: u64 = 0x7E_A20_0B1;

const AIR: u16 = 0;
const WATER: u16 = 5;
const LAVA: u16 = 516;

/// 统计窗：4×4 区块（y=7 层固体样本 ≥ 数千，带统计 σ ≤ 0.01）。
const CX: [i32; 4] = [2, 3, 4, 5];
const CZ: [i32; 4] = [-4, -3, -2, -1];

fn gen_all() -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    for &cx in &CX {
        for &cz in &CZ {
            let t = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(cx, cz))
                .expect("vanilla gen");
            out.push(t.voxels.as_u16_slice().to_vec());
        }
    }
    out
}

#[inline]
fn vidx(x: usize, y: i32, z: usize) -> usize {
    mcv_core::vidx(x, y, z)
}

/// 26 邻域内是否存在岩浆格（岩浆湖 barrier 豁免判据；跨界邻居忽略，
/// 只影响豁免不影响违例方向的正确性——违例只会更多不会更少时方向才反，
/// 这里跨界漏判会把 barrier 石误计为违例，故取「块内任一邻格即豁免」的
/// 宽松侧）。
fn touches_lava(vox: &[u16], x: usize, y: i32, z: usize) -> bool {
    for dy in -1..=1_i32 {
        for dz in -1..=1_isize {
            for dx in -1..=1_isize {
                let wy = y + dy;
                let wx = x as i32 + dx as i32;
                let wz = z as i32 + dz as i32;
                if !(WORLD_MIN_Y..WORLD_MAX_Y).contains(&wy) {
                    continue;
                }
                if !(0..16).contains(&wx) || !(0..16).contains(&wz) {
                    continue;
                }
                if vox[vidx(wx as usize, wy, wz as usize)] == LAVA {
                    return true;
                }
            }
        }
    }
    false
}

/// 一层的 (STONE, DEEPSLATE, BEDROCK, 其余固体) 计数（全窗 16×16×16 列）。
fn layer_counts(chunks: &[Vec<u16>], y: i32) -> (usize, usize, usize, usize) {
    let (mut st, mut ds, mut br, mut other) = (0usize, 0usize, 0usize, 0usize);
    for vox in chunks {
        for z in 0..16usize {
            for x in 0..16usize {
                match vox[vidx(x, y, z)] {
                    STONE => st += 1,
                    DEEPSLATE => ds += 1,
                    BEDROCK => br += 1,
                    AIR | WATER | LAVA => {}
                    _ => other += 1,
                }
            }
        }
    }
    (st, ds, br, other)
}

/// 基岩五层规则：y=−64 全窗 100% 基岩；−63..−60 散点比例带
/// （0.8/0.6/0.4/0.2，overworld.json top[0] + SurfaceRules.java:764-775
/// 线性概率实物；分母 = 基岩+深板岩 = defaultBlock 位，洞腔/流体不参与
/// 规则）；≥−59 与世界顶部零基岩（26.1 JSON 无 bedrock_roof）。
#[test]
fn bedrock_five_layer_rule() {
    let chunks = gen_all();
    let n_cols = chunks.len() * 256;
    // 世界底行：final_density bottom slide 常数 0.1171875 → squeeze ≈
    // +0.037 恒实心 → defaultBlock 恒出 → vertical_gradient y≤true 恒真。
    let (st, ds, br, other) = layer_counts(&chunks, WORLD_MIN_Y);
    assert_eq!(
        (st, ds, other),
        (0, 0, 0),
        "y=−64 存在非基岩固体 st={st} ds={ds} other={other}"
    );
    assert_eq!(br, n_cols, "y=−64 基岩 {br} ≠ 全窗列数 {n_cols}");
    // 散点四层：期望 1 − k/5（k = y−MIN_Y）。
    for (k, expect) in [(1_i32, 0.8_f64), (2, 0.6), (3, 0.4), (4, 0.2)] {
        let (st, ds, br, _other) = layer_counts(&chunks, WORLD_MIN_Y + k);
        let denom = br + ds;
        assert!(
            denom > 1000,
            "y={} defaultBlock 样本过少（{denom}）——窗口退化",
            WORLD_MIN_Y + k
        );
        assert_eq!(
            st,
            0,
            "y={} 散点带内残留石头（sequence 次序违例）",
            WORLD_MIN_Y + k
        );
        let frac = br as f64 / denom as f64;
        assert!(
            (frac - expect).abs() <= 0.08,
            "y={} 基岩占比 {frac:.3} ∉ {expect:.1}±0.08（vertical_gradient \
             线性概率实物，n={denom}）",
            WORLD_MIN_Y + k
        );
    }
    // ≥−59 全高度零基岩（含顶部——26.1 overworld.json bedrock 仅 floor 一处）。
    let mut stray = 0usize;
    for vox in &chunks {
        for y in (WORLD_MIN_Y + 5)..WORLD_MAX_Y {
            for z in 0..16usize {
                for x in 0..16usize {
                    if vox[vidx(x, y, z)] == BEDROCK {
                        stray += 1;
                    }
                }
            }
        }
    }
    assert_eq!(
        stray, 0,
        "散点带外基岩 {stray} 格（26.1 JSON 无 bedrock_roof）"
    );
}

/// 深板岩边界：y≤0（散点带以上、基岩带以下）石头占比恒 0、y=1..7
/// deepslate 占比逐层 = (8−y)/8±0.08 且随深度单调升（stone 占比单调降）、
/// y≥8 全高度零 deepslate（1.18 快照的 8..19 上浮斑块不在 26.1 实物）。
#[test]
fn deepslate_gradient_boundary() {
    let chunks = gen_all();
    // 恒真段（基岩散点带之上）：石头必须清零（y≤0 vertical_gradient 恒真，
    // 且 −59..0 无表层带可达）。豁免：岩浆湖 barrier 铺石（lake_pass，
    // #175 刀C）= 特征阶 STONE——原版 LakeFeature 的 barrier/铺装块在
    // features 阶段落位，晚于 surface 阶段，同样不经 deepslate 替换
    // （实测原版深层岩浆湖湖沿为石头）；判据 = 26 邻域含 LAVA。
    let (mut st, mut ds) = (0usize, 0usize);
    let mut exempt = 0usize;
    for vox in &chunks {
        for y in (WORLD_MIN_Y + 5)..=0 {
            for z in 0..16usize {
                for x in 0..16usize {
                    match vox[vidx(x, y, z)] {
                        STONE => {
                            if touches_lava(vox, x, y, z) {
                                exempt += 1;
                            } else {
                                st += 1;
                            }
                        }
                        DEEPSLATE => ds += 1,
                        _ => {}
                    }
                }
            }
        }
    }
    assert_eq!(
        st, 0,
        "y=−59..0 残留石头 {st}（恒真段必须全深板岩；岩浆湖 barrier 豁免 \
         {exempt} 格）"
    );
    assert!(ds > 10_000, "深板岩样本过少（{ds}）——窗口退化");
    // 渐变段：逐层占比带 + 单调。
    let mut fracs = Vec::new();
    for y in 1..8_i32 {
        let (st, ds, _br, _other) = layer_counts(&chunks, y);
        let n = st + ds;
        assert!(n > 500, "y={y} 石类样本过少（{n}）");
        let f = ds as f64 / n as f64;
        let expect = f64::from(8 - y) / 8.0;
        assert!(
            (f - expect).abs() <= 0.08,
            "y={y} deepslate 占比 {f:.3} ∉ {expect:.3}±0.08（n={n}）"
        );
        fracs.push(f);
    }
    for w in fracs.windows(2) {
        assert!(
            w[0] > w[1],
            "y=1..7 占比非单调（{:?}）——vertical_gradient 线性概率必须随 y 升递减",
            fracs
        );
    }
    // 恒假段：y≥8 全高度零深板岩（含地表与深层洞壁）。
    let mut stray = 0usize;
    for vox in &chunks {
        for y in 8..WORLD_MAX_Y {
            for z in 0..16usize {
                for x in 0..16usize {
                    if vox[vidx(x, y, z)] == DEEPSLATE {
                        stray += 1;
                    }
                }
            }
        }
    }
    assert_eq!(
        stray, 0,
        "y≥8 深板岩 {stray} 格（false_at_and_above=8 恒假段）"
    );
}

/// 确定性：同 seed 同区块两跑逐字节一致（本规则族局部镜像；E2 全局门在
/// quality.rs / golden 侧）。
#[test]
fn deepslate_bedrock_deterministic() {
    let a = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(2, -4)).expect("a");
    let b = generate_terrain_with(TerrainBackend::Vanilla, SEED, ChunkPos::new(2, -4)).expect("b");
    assert_eq!(a.voxels.as_u16_slice(), b.voxels.as_u16_slice());
    assert_eq!(a.heightmap[..], b.heightmap[..]);
    assert_eq!(a.voxels.as_u16_slice().len(), CHUNK_VOL);
}
