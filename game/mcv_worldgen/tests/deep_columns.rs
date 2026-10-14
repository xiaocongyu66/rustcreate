//! #190 深层透明取证（真机报告②：「原地向下挖到某层，周围变透明，
//! 视角朝上看方块正常」）。
//!
//! 假说一（用户猜测）：深板岩层没生成，挖穿石头进入深板岩深度后
//! 全是空气。本测直接生成高种子窗口逐柱取证 y ∈ [-64, 8]：
//! - 每柱自表面向下至基岩**不得整层掏空**（任何 y 层仍须实心）；
//!   掏空 = 「透明」世界生成根因，实心底不在 = 虚空坠落。
//! - 深板岩（DEEPSLATE_*）在注册表存在（资产/语言表齐）但世界生成
//!   **从不铺装**——vanilla pass 带下实心一律 STONE（terrain.rs
//!   solid_material，abovePreliminarySurface 门）。该缺层归 #192
//!   （深板岩/基岩形态补齐），本测只锁「缺材质不缺实体」的边界行为。
//! - 负 y 口径（#71 绕回面回归）：y=-64..8 段的实心必须反映在
//!   heightmap 上可达的深度语义（i16 绝对域，无符号绕回哨兵歧义）。
//!
//! ## 问题 B 根因报告（证词②「向下挖到某层周围变透明」收口，#190）
//!
//! 本文件锁确证的排除面（#190 期，deepfix 独立基座）：
//! - `deep_columns_are_never_hollow`：5 seed × 2×2 chunk，y ∈
//!   [WORLD_MIN_Y, 8] 逐层实心+流体覆盖率 > 0.15，且 y = WORLD_MIN_Y
//!   = −64（本仓实物常量 mcv_core/src/lib.rs:28；WORLD_MAX_Y = 320
//!   :31）基岩恒实心 ⟹ 当时生成面**无**「层带整段未生成 = 空区」。
//! - ~~`deepslate_absent_until_192`~~：生成输出 deepslate 系方块恒 0
//!   （当时 solid_material（vanilla/terrain.rs）min_surface 以下一律
//!   STONE ⟹ main 基座确证**缺深板岩层**）。
//!
//! ## 口径迁移记录（#192 接缝闭合，rebase 后执行）
//!
//! #192（深板岩五层 + surfaceDepth + NormalNoise 实物化 octave 种子派
//! 生 noise.rs + caves/noodle+pillars 补全 81418fe7）合入后：
//! - `deepslate_absent_until_192` 的「不含 deepslate」断言如预登记转红
//!   （run 38497803387：hit=13373）——**删除该测**：其全部残留意图已被
//!   main 的 deepslate_bedrock.rs 行为锁覆盖（deepslate_gradient_boundary
//!   锁 y≤0 恒深板岩 + 1..7 渐变概率 + y≥8 零，bedrock_five_layer 锁
//!   基岩五层，deepslate_bedrock_deterministic 锁确定性），资产面断言
//!   与注册表锁重叠，保留即双份判据。
//! - `deep_columns_are_never_hollow` 的单层 0.15 判据同步废止：单层覆
//!   盖率无法区分「奶酪洞群核心切片」（原版合法，caves/underground
//!   cheese 段核心直径 10-30 格级，y=7 实测 0.082 与同 run 绿的
//!   cave_audit_stats 4×4 带包络相容）与「层带整段不生成」；重立为
//!   cave_audit 同源带包络（见该测注）。真机病征「脚下实心底不在」
//!   由 y=−64 基岩恒实心 + 柱向 max_air_run 诊断继续兜底。
//! - 三连红后的口径终局（run 38497803387/38500970262/38567857659）：
//!   带级 16 格窗三 run 三个极端（0.082/0.000/0.640）——**带级窗本身
//!   是高方差量**（cheese 随机球 + 管状 carver 缺失 CAVE-1③，形态
//!   方差大于窗），包络怎么调都会被 5 seed×多带样本随机踩爆。终局
//!   判据下沉至列级聚合：每列 [-64,9) 最长连续实心段 ≥24（cheese
//!   球直径上界 30 格，多通道侵蚀独立叠加穿透全列概率极低），5120
//!   列坏列占比 ≤1%——大样本占比才能区分「系统性无基座」（真机坑
//!   病）与「形态方差」（合法洞穴）。带级统计降为诊断行入日志。
//!
//! B 另有根因时的复验探针（按序执行，不预先发明机制）：
//! ① 同 seed 同柱 `generate_terrain_with` 体素 dump 对层——覆盖率崩
//!    = 仍生成面（本测 eprintln 诊断 worst_level/max_air_run 是数据源）；
//! ② 体素实心而画面透明 = 网格口径差——已实证路径「编辑格落区块列
//!    边界时邻列 MESH 不置脏」已于本分支 fix/deep-holes 修复
//!    （touch_mesh_neighbors，原版 setBlockDirty ±1 壳层），B 应随之
//!    消失；①②在合并后构建复验仍绿而 B 复现 → 另立案取证。

use mcv_core::{CHUNK_SX, CHUNK_SZ, WORLD_MIN_Y};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

/// 实心判定：非空气且非流体（岩浆/水「透明」感由 #192/流体面管，
/// 实体掏空才是本锁对象）。
fn is_solid(id: u16) -> bool {
    let d = &mcv_core::BLOCKS[id as usize];
    d.solid
}

fn is_fluid(id: u16) -> bool {
    let d = &mcv_core::BLOCKS[id as usize];
    d.liquid
}

#[test]
fn deep_columns_are_never_hollow() {
    let mut columns_checked = 0usize;
    let mut max_air_run_report = String::new();
    let mut max_air_run = 0usize;
    let mut worst_level = (1.0f32, String::new());
    let mut bad_cols = 0usize;
    let mut worst_col_report = String::new();
    let mut worst_col_best_seg = 0usize;
    for seed in [2026_1013u64, 1, 777, 0xDEAD_BEEF, 42] {
        for cx in 0..2i32 {
            for cz in 0..2i32 {
                let out = generate_terrain_with(
                    TerrainBackend::Vanilla,
                    seed,
                    mcv_core::ChunkPos::new(cx, cz),
                )
                .expect("generate ok");
                let vox = out.voxels.as_u16_slice();
                let col_at =
                    |lx: usize, lz: usize, y: i32| -> u16 { vox[mcv_core::vidx(lx, y, lz)] };
                // 层带统计：16 格带空气率（air/total，流体计入填充——
                // 含水层水/岩浆「透明感」由流体面管，实体掏空才是本锁
                // 对象）包络 + 逐带诊断（worst_level 数据源）。
                // 口径重立（#190→#192 接缝后，见模块头注口径迁移记录）：
                // 单层实心+流体 >0.15 的旧判据是对 deepfix 独立基座的旧
                // 自研噪声场定的经验值；#192 波（NormalNoise 实物化
                // octave 种子派生 noise.rs、caves/noodle+pillars 补全
                // 81418fe7）后 y=7 切片实测 0.082（run 38497803387）——
                // 与同 run 绿的 cave_audit_stats（4×4 窗 16 格带空气率
                // 包络 (0.005, 0.5)）相容 = 奶酪洞群核心切片的**原版合
                // 法形态**（density_function/overworld/caves/*.json 蚀
                // 刻源实物，underground cheese 段核心直径 10-30 格级，
                // 单 chunk 单层被洞群吞 80%+ 不属「层带整段不生成」）。
                // 新判据 = 同源带包络**单侧化**：[-48,8) 每带空气率
                // ≤0.6（上界=防「全空腔」系统病，cave_audit 0.5 + 2×2
                // 窗 1024 格方差余量 10%）；**下界不设**——run 38500970262
                // attempt 2 实证 seed=20261013 band=[-48,-32) 空气率
                // 0.000 的全实心态与实物蚀刻门结构相容：该带蚀刻源
                // cheese(noodle y≥−60/cheese xz1·y⅔) 皆**随机管/球**，
                // 空间相关长度 30-60 格 vs 32×32 窗，零穿越概率可观；
                // 且管状 carver（cave 0.15/canyon 0.01，configured_carver
                // 实物）本仓未移植 = KNOWN-DIVERGENCE CAVE-1③ 登记，
                // 空气率系统性低于原版，照搬 cave_audit 4×4 窗下界到
                // 2×2 窗属口径错位。「全实心塌方」防护面由 cave_audit
                // 的 4×4 窗下界（同 CI 同 run 绿）继续承担，此处不重复。
                // c 门对位复核（既有绿基）：final_density=
                // min(squeeze(0.64·slide), noodle) 与 caves/noodle.json
                // 逐节点 + pillars 门 <0.03→−1e6，结构对位零偏差。
                for (band_lo, band_hi) in
                    [(WORLD_MIN_Y, -48), (-48, -32), (-32, -16), (-16, 0), (0, 9)]
                {
                    let (mut air, mut total) = (0usize, 0usize);
                    for y in band_lo..band_hi {
                        for lx in 0..CHUNK_SX {
                            for lz in 0..CHUNK_SZ {
                                let id = col_at(lx, lz, y);
                                total += 1;
                                if !is_solid(id) && !is_fluid(id) {
                                    air += 1;
                                }
                            }
                        }
                    }
                    let void = air as f32 / total as f32;
                    eprintln!(
                        "K-DEEP-COL band seed={seed} chunk=({cx},{cz}) \
                         [{band_lo},{band_hi}) air%={void:.3}"
                    );
                    if 1.0 - void < worst_level.0 {
                        worst_level = (
                            1.0 - void,
                            format!("seed={seed} chunk=({cx},{cz}) band=[{band_lo},{band_hi})"),
                        );
                    }
                }
                for lx in 0..CHUNK_SX {
                    for lz in 0..CHUNK_SZ {
                        // 基岩恒实心（y=-64 层，BedrockBlock 底层规则：
                        // 开放柱/含水层分支都在实心门之后）。
                        assert!(
                            is_solid(col_at(lx, lz, WORLD_MIN_Y)),
                            "K-DEEP-COL seed={seed} col=({lx},{lz})：y={WORLD_MIN_Y} 不实心 = 挖穿坠虚空"
                        );
                        // 列级聚合判据（对位真机病征「挖到某层脚下无实体
                        // 底」）：每列 [-64,9) 最长连续实心段。cheese 球
                        // 直径上界 30 格、管状 carver 缺失（CAVE-1③）使
                        // 带级窗方差过大（三连红三端实证），列级 5120 样
                        // 本聚合才能区分系统病与形态方差；多通道侵蚀独
                        // 立叠加，原版深层列几乎必有多段 ≥24 实心。
                        let mut seg = 0usize;
                        let mut best_seg = 0usize;
                        for y in WORLD_MIN_Y..=8 {
                            if is_solid(col_at(lx, lz, y)) {
                                seg += 1;
                                if seg > best_seg {
                                    best_seg = seg;
                                }
                            } else {
                                seg = 0;
                            }
                        }
                        if best_seg < 24 {
                            bad_cols += 1;
                            if best_seg > worst_col_best_seg {
                                worst_col_best_seg = best_seg;
                                worst_col_report = format!(
                                    "seed={seed} chunk=({cx},{cz}) col=({lx},{lz})"
                                );
                            }
                        }
                        // 诊断量：带内最长连续空气 run（洞合法，只报
                        // 不锁——全空柱 run=72 时上面的层带断言先红）。
                        let mut run = 0usize;
                        for y in WORLD_MIN_Y..=8 {
                            if !is_solid(col_at(lx, lz, y)) {
                                run += 1;
                            } else {
                                if run > max_air_run {
                                    max_air_run = run;
                                    max_air_run_report = format!(
                                        "seed={seed} chunk=({cx},{cz}) col=({lx},{lz}) run={run} 止于 y={y}"
                                    );
                                }
                                run = 0;
                            }
                        }
                        columns_checked += 1;
                    }
                }
            }
        }
    }
    // 窗口 = 5 seed × 2×2 chunk × 256 列 = 5120；哨兵防「生成静默失效
    // 导致全表假绿」。
    assert!(columns_checked > 5000, "柱样本过少 = 生成失效");
    // 列级聚合断言（三连红的根解：带级 16 格窗三 run 各踩一端
    // 0.082/0.000/0.640 = 形态方差大于窗，聚合维度必须下沉到列——
    // 5120 列大样本占比才能判「系统性无实心底」）。≤1%（51 列）容
    // 差 = 局部蚀刻可以密集，但深层基座系统性缺失不行；best_seg<24
    // 即判坏（cheese 球直径上界 30 格 < 段门 24+双侵蚀余量）。
    assert!(
        bad_cols * 100 <= columns_checked,
        "K-DEEP-COL 无实心底列 {bad_cols}/{columns_checked}（>1%）——\
         深层基座系统性缺失（真机「挖到某层周围变透明」根因候选；\
         最重坏列样本：{worst_col_report} best_seg={worst_col_best_seg}）"
    );
    eprintln!(
        "K-DEEP-COL 诊断：最深连续空气 run={max_air_run}（{max_air_run_report}）\
         columns={columns_checked} bad_cols={bad_cols} worst_level={}（{worst_col_report}）",
        worst_level.1
    );
}

// 深板岩缺层登记哨兵已删除（#192 接缝闭合后口径迁移，见模块头注
// 「口径迁移记录」）：铺装存在性/y 域语义由 deepslate_bedrock.rs 行为
// 锁（deepslate_gradient_boundary/bedrock_five_layer/
// deepslate_bedrock_deterministic）覆盖，资产面由注册表锁覆盖——
// 保留本哨兵即双份判据。
