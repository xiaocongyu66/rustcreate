//! 测试共享件（重锚波②）：树测试的**运行时选窗**。
//!
//! 旧形态写死 FOREST_CX/CZ 等常量——cell 外壳落地后气候场整体位移
//! （run 38825194406：Forest/Ocean 窗气候漂移），写死窗必然「测试
//! 空转」。改为运行时按 `biome_at` 扫描找第一个全 16×16=want 的
//! chunk 窗——对 biome 分布漂移鲁棒，断言语义（窗内树密度/跨界完整）
//! 不变。
//!
//! **性能合同（run 38847295786 卡死教训）**：每个候选窗**只建一次**
//! [`Orchestrator`]（噪声重初始化毫秒级/次——每列重建=十万次构造=
//! 测试挂钟爆表），column_state/biome_at 全走该实例。

use mcv_core::SEA_LEVEL;
use mcv_worldgen::vanilla::Biome;
use mcv_worldgen::vanilla::climate::ClimateSampler;
use mcv_worldgen::vanilla::terrain::Orchestrator;

/// 从原点螺旋外扫（Chebyshev 环），返回第一个满足三条件的 chunk 坐标
/// `(cx, cz)`：①16×16 全 `want`；②≥14/16 列表面高于海面（防滩/浅水
/// 立地列——树基弃置→树测试空转）；③窗中心列温 >0.2（#196 列级积雪
/// 低温上图→表面带雪→同样空转；run 38843693793/38845044818 实证）。
/// ±160 chunk 环内找不到即 panic（气候场崩坏）。
#[must_use]
pub fn find_biome_window(seed: u64, want: Biome) -> (i32, i32) {
    let orch = Orchestrator::new(seed);
    let climate = ClimateSampler::new(seed);
    for r in 0..=160_i32 {
        for cz in -r..=r {
            for cx in -r..=r {
                if cx.abs() != r && cz.abs() != r {
                    continue; // 只扫新环
                }
                if !window_biome(&orch, cx, cz, want) || !ground_pure(&orch, cx, cz) {
                    continue;
                }
                if climate.sample(cx * 16 + 8, cz * 16 + 8).temperature > 0.2 {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("find_biome_window: ±160 chunk 环内无合格 {want:?} 窗——生物群系气候场崩坏");
}

/// 单窗群系检查（复用同一 Orchestrator——性能合同见模块注）。
fn window_biome(orch: &Orchestrator, cx: i32, cz: i32, want: Biome) -> bool {
    for z in 0..16_i32 {
        for x in 0..16_i32 {
            if orch.biome_at(cx * 16 + x, cz * 16 + z) != want {
                return false;
            }
        }
    }
    true
}

/// 立地纯度：≥14/16 列「表面高于海面」。
fn ground_pure(orch: &Orchestrator, cx: i32, cz: i32) -> bool {
    let mut ok = 0;
    for z in 0..16_i32 {
        for x in 0..16_i32 {
            if orch.column_state(cx * 16 + x, cz * 16 + z).surface_ours >= SEA_LEVEL {
                ok += 1;
            }
        }
    }
    ok >= 14
}
