//! 测试共享件（重锚波②）：树测试的**运行时选窗**。
//!
//! 旧形态写死 FOREST_CX/CZ 等常量——cell 外壳落地后气候场整体位移
//! （run 38825194406：Forest/Ocean 窗气候漂移），写死窗必然「测试
//! 空转」。改为运行时按 `biome_at` 扫描找第一个全 16×16=want 的
//! chunk 窗——对 biome 分布漂移鲁棒，断言语义（窗内树密度/跨界完整）
//! 不变。

use mcv_core::SEA_LEVEL;
use mcv_worldgen::vanilla::{Biome, biome_at, surface_block_y};

/// 从原点螺旋外扫（Chebyshev 环），返回第一个 16×16 全 `want` 的
/// chunk 坐标 `(cx, cz)`；±96 chunk 环内找不到即 panic（气候场崩坏）。
#[must_use]
pub fn find_biome_window(seed: u64, want: Biome) -> (i32, i32) {
    for r in 0..=160_i32 {
        for cz in -r..=r {
            for cx in -r..=r {
                if cx.abs() != r && cz.abs() != r {
                    continue; // 只扫新环
                }
                if window_all(seed, cx, cz, want)
                    && ground_pure(seed, cx, cz)
                    && mcv_worldgen::vanilla::column_temperature(seed, cx * 16 + 8, cz * 16 + 8)
                        > 0.2
                {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("find_biome_window: ±160 chunk 环内无合格 {want:?} 窗——生物群系气候场崩坏");
}

/// 立地纯度：≥14/16 列「表面高于海面」（surface_block_y≥SEA_LEVEL）+
/// 窗中心列温 >0.2（#196 列级积雪按温度上图，biome=Forest 的低温列
/// 表面带雪→树基立地弃→树测试空转；run 38843693793/38845044818 实证）。
/// 三重合并后窗=「全 Forest + 高海面 + 暖带」的干净林地。
#[must_use]
fn ground_pure(seed: u64, cx: i32, cz: i32) -> bool {
    let mut ok = 0;
    for z in 0..16_i32 {
        for x in 0..16_i32 {
            if surface_block_y(seed, cx * 16 + x, cz * 16 + z) >= SEA_LEVEL {
                ok += 1;
            }
        }
    }
    ok >= 14
}

fn window_all(seed: u64, cx: i32, cz: i32, want: Biome) -> bool {
    for z in 0..16_i32 {
        for x in 0..16_i32 {
            if biome_at(seed, cx * 16 + x, cz * 16 + z) != want {
                return false;
            }
        }
    }
    true
}
