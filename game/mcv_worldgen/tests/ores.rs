//! 矿石 pass 行为锁（#178 刀3c）。
//!
//! 覆盖：v7 全 chunk 字节哈希黄金（防后续漂移）、频度带（μ 推导见
//! `freq_band` 注）、钻石 trapezoid 峰形、目标岩门（只替换 pass-1 石头，
//! 不打水/空气）、确定性 + 3×3 重放双侧一致（矿脉缝合）。
//! v7 说明：surface 波（#192 深板岩层+基岩五层+surfaceDepth）合入后的
//! 完整基线，含深板岩/基岩维度（重锚证据见 WANT 行注）。

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
    // surface 波（#192）重锚证据：同 sha 双 run——run 38472795386
    // （9987f069）原执行与 rerun-failed 第二次执行六例逐位一致；同 run
    // cross_chunk/ores_only/diamond/freq_band 与 streaming 湖锁全绿
    // = 矿石输出零漂移，仅地形基线变更（深板岩/基岩维度入面）。
    // registry 期首锚（0cf9…六值，run 38439382386/38458062994）见
    // docs 历史；实现定稿于 4a5ac2f2。
    // #209 重锚（density fork 链+prelim find_top_surface+含水层囊袋点
    // Xoroshiro 化后的 byte 基线）：双 run 证据 run 38749575664 首跑与
    // 同 sha rerun --failed 二跑六例逐位一致（2026-10-16）。
    const WANT: [u64; 6] = [
        0x4C6E_B2F8_769C_0C45,
        0xBA4D_5339_61FA_2355,
        0x6868_C030_CC34_E823,
        0x49C6_EE4D_EDB3_CA02,
        0xABD9_3A7D_11ED_ECD7,
        0xBD0B_89B2_97FE_B64C,
    ];
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
    // vidx 契约（mcv_core/src/lib.rs:36-43）：x/z 为**区块内局部**坐标
    // 0..16。贴 x 界的写还可能同时贴 z 界（blob z 向伸出本区块列域），
    // z 出 [0,16) 的格属 z 邻列区块——与 project_ores 盒裁剪同口径
    // （ore_pass.rs:584-587 `!(0..16).contains(&lz)` → 由该邻块重放落
    // 其自块）。本循环只验属本块 (0,0)/(1,0) 者；存在性断言仍看全量。
    let v0 = gen_vox(seed, 0, 0);
    for w in left.iter().copied().filter(|w| (0..16).contains(&w.z)) {
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
    for w in right.iter().copied().filter(|w| (0..16).contains(&w.z)) {
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
    // 最终缓冲每个 ore 格的 pass-1 重放态必为 STONE。普查面=16 真矿行
    // 的全部 target 态（ore_type_ids 32 项，freq_band 同源）：替代岩行
    // （ore_dirt/
    // ore_gravel/ore_granite 等，data placed_feature 同名 json）写的
    // state 与表层系统自然材质**同 id**（如 ore_dirt state=dirt=2 =
    // subsoil 带），id 普查无法区分「矿写」与「表层天然格」，对其断言
    // STONE 是口径错误；替代岩行的目标岩门由写级判据锁
    // （place_blob canPlaceOre 对位 + cross_chunk/deterministic/freq
    // 三锁的 writes 面）。真矿 18 态在本分支地形中不可能由 pass-1 产
    // 生，普查 = 精确的 ore 写集合。deepslate 维度随
    // feat/surface-deepslate 入面后放宽为 tag 成员。
    let seed = 0x7051_0002;
    let orch = Orchestrator::new(seed);
    let aq = Aquifer::new(seed);
    let mut f = AquiferField::new(&aq, &orch);
    let vox = gen_vox(seed, 0, 0);
    let ore_ids = ore_type_ids();
    let mut checked = 0;
    for z in 0..16u32 {
        for x in 0..16u32 {
            for y in (mcv_core::WORLD_MIN_Y + 1..mcv_core::WORLD_MAX_Y).rev() {
                let id = vox[mcv_core::vidx(x as usize, y, z as usize)];
                if !ore_ids.contains(&id) {
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
    // 下限实物推导（gen-ores 25 行逐行 placed json 实物参数）：16 真矿
    // 行单 chunk 期望 = Σ count×P(rarity_filter)×E[带下实格]（python 几
    // 何 MC：OreFeature.java:55-166 椭球格 + :44-52 OCEAN_FLOOR_WG 探针
    // 上带截断 + RarityFilter.java:24 + CountPlacement.java:23-24）
    // ≈ 384（copper 84 / coal_lower 82 / iron_middle 68 / redstone 系
    // 44 / lapis 系 28 / diamond 系 30 / gold 系 30 / iron_small 18 /
    // 上带行 ≈0）；未计入洞穴邻空气 discard（OreFeature.java:183-189，
    // 深层岩浆海腔密集）再削 ~20% → 实测量级 ~300。旧 500 是对含替代
    // 岩大 blob 的普查面定的，按 16 真矿行普查面重立 = 推导 ×0.52。
    assert!(checked > 200, "采样 chunk 矿格过少（{checked}）——接线失效?");
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

/// ore 型矿物 id（16 真矿行的全部 target 态，共 32 项；替代岩行除外）。
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
    // trapezoid(above_bottom −80..80 → 世界 −144..16)：峰=区间中点
    // y=−64（TrapezoidHeight.java:55-57 plateauStart/End 对称），叠加
    // ore_diamond_medium 的 uniform −64..−4 平峰分量（data
    // placed_feature/ore_diamond_medium.json）→「逐层密度自峰向顶单调
    // 衰减」只在**等宽分桶**下成立；旧 16/32 混宽桶把 32 格桶的平峰分
    // 量按 2 倍宽度吸入，正确实现也必违 bins[0]>bins[1]（python 实物
    // 模型 2.5 万格 MC：正确实现宽桶比 b1/b0≈1.4）。格自抽样高度向
    // 下伸 [−3,0]（OreFeature.java:36-37 y0/y1=origin+U(0,2)−2），世
    // 界底板 −64 截断（:132 isOutsideBuildHeight）→ 峰下无带。5×16
    // 等宽分桶；断言口径见下方判据注（半区质量 + 上半区衰减 + 顶带）。
    let seed = 0xD1A0_0004;
    let mut bins = [0usize; 5];
    for cz in -2..2i32 {
        for cx in -2..2i32 {
            let vox = gen_vox(seed, cx, cz);
            for id in diamond_ids() {
                for z in 0..16usize {
                    for x in 0..16usize {
                        for (bi, (lo, hi)) in
                            [(-64, -48), (-48, -32), (-32, -16), (-16, 0), (0, 16)]
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
    // 峰形判据（环境稳健口径）。实物理论形状（python 2.5 万格 MC）：
    // 五桶份额 [0.322, 0.275, 0.228, 0.146, 0.026]，峰带半区 (b0+b1)/
    // (b2+b3)=1.60；峰整体上移 +16 的变体 = 1.14。但峰带恰是含水层岩
    // 浆海/洞穴空气腔最密带（aquifer 全局岩浆 y<lava_sea + :183-189
    // 邻空气 discard），洞穴按深度切割 b0 → 实测 b0 被环境压 ~20%
    // （本仓 height 采样流已 python 逐位复刻验证与实物三角公式同分
    // 布：五桶 [0.254, 0.314, 0.258, 0.144, 0.030] vs CI 实测
    // [0.266, 0.318, 0.216, 0.170, 0.030] 逐桶吻合）→ 逐桶严格单调
    // （b0>b1）对环境不稳健，改锁：峰带半区质量占优（阈值 1.3，实物
    // 1.60 / 变体 1.14）+ 上半区桶间衰减保持 + 顶带趋零。
    assert!(
        bins[0] + bins[1] > (bins[2] + bins[3]) * 13 / 10,
        "钻石峰形半区质量不足（峰位偏移）{bins:?}"
    );
    assert!(
        bins[1] > bins[2] && bins[2] > bins[3] && bins[3] >= bins[4],
        "钻石峰形上半区非单调衰减 {bins:?}"
    );
    assert!(bins[4] * 3 < bins[0], "顶带未趋零 {bins:?}");
}

fn diamond_ids() -> Vec<u16> {
    ORE_ROWS
        .iter()
        .filter(|r| r.name.starts_with("ore_diamond"))
        .flat_map(|r| r.targets.iter().map(|t| t.state))
        .collect()
}
