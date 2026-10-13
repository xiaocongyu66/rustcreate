//! 矿石 pass 行为锁（#178 刀3c）。
//!
//! 覆盖：v7 全 chunk 字节哈希黄金（防后续漂移）、频度带（μ 推导见
//! `freq_band` 注）、钻石 trapezoid 峰形、目标岩门（只替换 pass-1 石头，
//! 不打水/空气）、确定性 + 3×3 重放双侧一致（矿脉缝合）。
//! v7 说明：字节口径自「+矿石」维度重锚（feat/surface-deepslate 分支未
//! 合，本分支重锚只含矿石维度，不含深板岩）。

use mcv_core::ChunkPos;
use mcv_worldgen::vanilla::aquifer::{Aquifer, AquiferField};
use mcv_worldgen::vanilla::ore_pass::{ORE_DOMAIN, OreWrite, ore_writes, origin_ore_writes};
use mcv_worldgen::vanilla::ores_data::ORE_ROWS;
use mcv_worldgen::vanilla::terrain::Orchestrator;
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

fn gen_vox(seed: u64, cx: i32, cz: i32) -> Vec<u16> {
    generate_terrain_with(TerrainBackend::Vanilla, seed, ChunkPos::new(cx, cz))
        .expect("vanilla 后端生成")
        .voxels
        .as_u16_slice()
        .to_vec()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

/// 全部矿石行可能写出的 id 集（stone/deepslate 两变体 + scatter 替代岩）。
fn all_ore_ids() -> Vec<u16> {
    let mut v: Vec<u16> = ORE_ROWS
        .iter()
        .flat_map(|r| r.targets.iter().map(|t| t.state))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn is_ore(id: u16) -> bool {
    all_ore_ids().contains(&id)
}

// ---------------------------------------------------------------- 黄金（v7）

/// v7 全 chunk 字节哈希（FNV-1a，口径同 tests/golden/cases.rs）：固定
/// seed×chunk 集防后续漂移。**首锚通道**：表值 0 时打印实测哈希并失败
/// ——把打印值填回本表再绿（CI-only 验证环境的锚定流程，同 GOLDEN_UPDATE
/// 思路的单文件版）。
#[test]
fn ore_v7_byte_golden() {
    const CASES: [(u64, i32, i32); 6] = [
        (0x0DE0_2611_0000_0001, 0, 0),
        (0x0DE0_2611_0000_0001, 3, -2),
        (0x0DE0_2611_0000_0001, -1, 5),
        (0x5EED_0000_0000_0007, 0, 0),
        (0x5EED_0000_0000_0007, -4, -4),
        (0x5EED_0000_0000_0007, 7, 2),
    ];
    // v7：+矿石（gen-ores 25 行 + ore_pass 执行器）锚定哈希。
    const WANT: [u64; 6] = [0; 6];
    let mut report = String::new();
    let mut fails = 0;
    for (i, &(seed, cx, cz)) in CASES.iter().enumerate() {
        let vox = gen_vox(seed, cx, cz);
        let bytes: Vec<u8> = vox.iter().flat_map(|v| v.to_le_bytes()).collect();
        let got = fnv1a(&bytes);
        if WANT[i] == 0 || got != WANT[i] {
            fails += 1;
            report.push_str(&format!(
                "\n  case{i} seed={seed:016x} ({cx},{cz}) => {got:016x}"
            ));
        }
    }
    assert_eq!(
        fails, 0,
        "v7 矿石黄金漂移/未锚定——把下列哈希填入 WANT:{report}"
    );
}

// ------------------------------------------------------------ 确定性/双侧一致

#[test]
fn deterministic_same_seed_and_pure_replay() {
    // 同 seed 同 chunk 两次 generate 逐字节一致（pass 序/HashMap 迭代不
    // 入判定面）。
    let a = gen_vox(0xABCD_0001, 1, -2);
    let b = gen_vox(0xABCD_0001, 1, -2);
    assert_eq!(a, b);
    // 双侧一致：origin_ore_writes 是 (seed, origin) 纯函数——两个独立
    // Replay（不同宿主块）逐条相等 = 矿脉缝合的判据面。
    let orch = Orchestrator::new(0xABCD_0001);
    let aq = Aquifer::new(0xABCD_0001);
    let mut f1 = AquiferField::new(&aq, &orch);
    let mut f2 = AquiferField::new(&aq, &orch);
    let w1 = ore_writes(0xABCD_0001, 4, 4, &orch, &mut f1);
    let w2 = ore_writes(0xABCD_0001, 4, 4, &orch, &mut f2);
    assert_eq!(w1.len(), w2.len());
    assert!(w1.iter().zip(w2.iter()).all(|(x, y)| x == y));
    assert!(!w1.is_empty(), "32×32 区块应有矿写");
}

#[test]
fn cross_chunk_vein_stitching() {
    // 跨界 blob：origin (1,0) 伸入 x=15（chunk 0）的写，必须由 chunk 0
    // 自身的重放落进其缓冲（两侧各写自块、同源同几何）。
    let seed = 0x000C_5EED_0001;
    let orch = Orchestrator::new(seed);
    let aq = Aquifer::new(seed);
    let mut f = AquiferField::new(&aq, &orch);
    let ws = ore_writes(seed, 1, 0, &orch, &mut f);
    let left: Vec<&OreWrite> = ws.iter().filter(|w| w.x == 15).collect();
    let right: Vec<&OreWrite> = ws.iter().filter(|w| w.x == 16).collect();
    assert!(
        !left.is_empty() || !right.is_empty(),
        "origin (1,0) 应有贴界写（左 {}/右 {}）",
        left.len(),
        right.len()
    );
    let v0 = gen_vox(seed, 0, 0);
    for w in &left {
        let got = v0[mcv_core::vidx(15, w.y, w.z as usize)];
        assert!(
            is_ore(got),
            "缝合缺位 ({},{},{}) 期望矿 got {got}",
            w.x,
            w.y,
            w.z
        );
    }
    let v1 = gen_vox(seed, 1, 0);
    for w in &right {
        let got = v1[mcv_core::vidx(0, w.y, w.z as usize)];
        assert!(is_ore(got), "本体缺位 (16,{},{}): got {got}", w.y, w.z);
    }
    // 域盐接线哨兵：ORE_DOMAIN 非零且不与树/湖同值（通道隔离）。
    assert_ne!(ORE_DOMAIN, 0x7BEE_5501);
    assert_ne!(ORE_DOMAIN, 0x0000_1A3E_005E);
    // origin_ore_writes 经 Replay 引用共享缓存的入口同果（同一 rp 两次
    // 调用逐条相等 = 缓存不掺结果）。
    let mut rp = mcv_worldgen::vanilla::ore_pass::Replay::new(seed, &orch, &mut f);
    let a = origin_ore_writes(seed, 1, 0, &mut rp);
    let b = origin_ore_writes(seed, 1, 0, &mut rp);
    assert_eq!(a, b);
}

// ------------------------------------------------------------------ 目标岩门

#[test]
fn ores_only_replace_pass1_stone() {
    // 最终缓冲每个 ore 格的 pass-1 重放态必为 STONE（本分支实物面：
    // tag 成员中只有石头会出现在地形——deepslate 维度随
    // feat/surface-deepslate 分支入面后此断言自然放宽为 tag 成员）。
    let seed = 0x7051_0002;
    let orch = Orchestrator::new(seed);
    let aq = Aquifer::new(seed);
    let mut f = AquiferField::new(&aq, &orch);
    let vox = gen_vox(seed, 0, 0);
    let mut checked = 0;
    for z in 0..16u32 {
        for x in 0..16u32 {
            for y in (mcv_core::WORLD_MIN_Y + 1..mcv_core::WORLD_MAX_Y).rev() {
                let id = vox[mcv_core::vidx(x as usize, y, z as usize)];
                if !is_ore(id) {
                    continue;
                }
                checked += 1;
                let pure = ore_pass_block_at(seed, &orch, &mut f, x as i32, y, z as i32);
                assert_eq!(
                    pure,
                    mcv_worldgen::vanilla::terrain::STONE,
                    "ore({id}) 落在非石头 {pure} @({x},{y},{z})"
                );
                // 不打空气/水：石头判据已含；再锁水铁（同格不可能双是）。
                assert_ne!(pure, mcv_worldgen::vanilla::terrain::WATER);
            }
        }
    }
    assert!(checked > 500, "采样 chunk 矿格过少（{checked}）——接线失效?");
}

fn ore_pass_block_at(
    seed: u64,
    orch: &Orchestrator,
    field: &mut AquiferField<'_>,
    x: i32,
    y: i32,
    z: i32,
) -> u16 {
    let mut rp = mcv_worldgen::vanilla::ore_pass::Replay::new(seed, orch, field);
    mcv_worldgen::vanilla::ore_pass::Replay::block_at(&mut rp, x, y, z)
}

// ------------------------------------------------------------------ 频度带

#[test]
fn freq_band_mu_derivation() {
    // μ 推导（每 chunk 期望 ore 型矿格 = Σ行 count × E[blob 实格] × 有效
    // 率（探针存活/discard/重叠））：主导行 iron_middle 10×~11、coal_lower
    // 20×~24×~0.7、copper 16×~14、redstone 12×~9、lapis 6×~7、diamond
    // 13×~4+埋藏系、gold 4.5×~10×~0.7、iron_small 10×~4 ≈ 1.0-1.3k；
    // upper 系（coal/iron upper、diamond_medium）多在air带由 OCEAN_FLOOR_WG
    // 探针截断 → 小项。带取 [μ/2.5, 2.5μ] = [400, 3300]（16 chunk 均值，
    // 方差 << 带宽；granite/diorite 等替代岩不计）。
    const MU_LO: usize = 400;
    const MU_HI: usize = 3300;
    let seed = 0xF7E0_0003;
    let ids: Vec<u16> = ["coal_ore", "deepslate_coal_ore"]
        .iter()
        .map(|n| block_id(n))
        .collect();
    assert!(!ids.is_empty());
    let mut per_chunk = Vec::new();
    let mut total = 0usize;
    for cz in 0..4i32 {
        for cx in 0..4i32 {
            let vox = gen_vox(seed, cx, cz);
            // ore 型 = ORE_ROWS 中 targets 命中 stone/deepslate 系矿（排除
            // granite/diorite/andesite/tuff/dirt/gravel 替代岩行）。
            let mut n = 0;
            for id in ore_type_ids() {
                n += vox.iter().filter(|&&v| v == id).count();
            }
            per_chunk.push(n);
            total += n;
        }
    }
    let mean = total / 16;
    assert!(
        (MU_LO..=MU_HI).contains(&mean),
        "每 chunk 矿格均值 {mean} 出带 [{MU_LO},{MU_HI}]: {per_chunk:?}"
    );
    assert!(per_chunk.iter().any(|&n| n > 0), "全零 = 接线失效");
}

/// ore 型矿物 id（18 个 *_ore；替代岩行除外）。
fn ore_type_ids() -> Vec<u16> {
    let skip = ["granite", "diorite", "andesite", "tuff", "dirt", "gravel"];
    ORE_ROWS
        .iter()
        .filter(|r| {
            let nm = r.targets[0].state;
            let name = mcv_core::BLOCKS[nm as usize].name;
            !skip.contains(&name)
        })
        .flat_map(|r| r.targets.iter().map(|t| t.state))
        .collect()
}

fn block_id(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("缺方块 {name}")) as u16
}

// ---------------------------------------------------------------- 钻石峰形

#[test]
fn diamond_trapezoid_peak_shape() {
    // trapezoid(above_bottom −80..80 → 世界 −144..16) 三角峰：min=−144
    // max=16 峰在 (min+max)/2=−64（恰 world floor）→ 自底向上单调衰减、
    // 顶界趋零。断言 [−64,−48) > [−48,−16) > [−16,0) ≥ [0,17) 且顶带 <
    // 底带 1/3（边界趋零）。
    let seed = 0xD1A0_0004;
    let mut bins = [0usize; 4];
    for cz in -2..2i32 {
        for cx in -2..2i32 {
            let vox = gen_vox(seed, cx, cz);
            for id in diamond_ids() {
                for z in 0..16usize {
                    for x in 0..16usize {
                        for (bi, (lo, hi)) in [(-64, -48), (-48, -16), (-16, 0), (0, 17)]
                            .into_iter()
                            .enumerate()
                        {
                            let mut c = 0;
                            for y in lo..hi {
                                if vox[mcv_core::vidx(x, y, z)] == id {
                                    c += 1;
                                }
                            }
                            bins[bi] += c;
                        }
                    }
                }
            }
        }
    }
    assert!(bins[0] > 200, "样本过小 {bins:?}");
    assert!(
        bins[0] > bins[1] && bins[1] > bins[2] && bins[2] >= bins[3],
        "钻石峰形非单调衰减 {bins:?}"
    );
    assert!(bins[3] * 3 < bins[0], "顶带未趋零 {bins:?}");
}

fn diamond_ids() -> Vec<u16> {
    ORE_ROWS
        .iter()
        .filter(|r| r.name.starts_with("ore_diamond"))
        .flat_map(|r| r.targets.iter().map(|t| t.state))
        .collect()
}
