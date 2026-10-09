//! 出生投放/respawn/存档位置恢复专项测试（game.rs tests 的子模块，共用
//! headless_rt / lit_chunk 无头装配）。
//!
//! 覆盖（派单要求 a–d）：
//! a. 投放点脚部与头部两格必须 passable（非实体且非流体）；
//! b. 真实 C++ 地形（含真机取证旧档 seed 458746403253）下投放永不埋入、
//!    不落水中；
//! c. respawn 与首次进入走同一投放路径（game.rs
//!    respawn_reenters_loading_and_drops_at_surface）；
//! d. 存档玩家位置存在时不走出生投放，且 seed/调度器对齐存档。

use std::sync::Arc;

use glam::Vec3;

// 显式导入（tests 模块的无头装配 + game 模块的投放纯函数），不用 glob：
// 保持与其他测试文件解耦，避免同名遮蔽歧义。
use super::{BlockId, ChunkHandle, ChunkPos, GameMode, GameRuntime, spawn_column_feet_y};
use super::{STONE, headless_rt, lidx, lit_chunk};

/// blocks_gen "glass"（id 419）：damp=0 的实体方块——heightmap 跳过它、
/// 碰撞挡住它，是构造「hm 与实际可站立面不一致」的最小手段。
const GLASS: u16 = 419;
const WATER: u16 = 5;
const FLOWER_RED: u16 = 12;

fn floor_voxels(top: usize) -> Box<[BlockId; 65536]> {
    let mut v = Box::new([BlockId(0); 65536]);
    for y in 0..=top {
        for z in 0..16usize {
            for x in 0..16usize {
                v[lidx(x, y, z)] = BlockId(STONE);
            }
        }
    }
    v
}

fn hm_of(v: &[BlockId; 65536]) -> Box<[u8; 256]> {
    mcv_worldgen::recompute_heightmap(bytemuck::cast_slice(v))
}

fn def_of(id: u16) -> Option<&'static mcv_core::BlockDef> {
    mcv_core::BLOCKS.get(id as usize)
}

// ---- a：单列判定（spawn_column_feet_y 纯函数）----

#[test]
fn spawn_column_feet_y_rejects_unsafe_columns() {
    let v = floor_voxels(20);
    let hm = hm_of(&v);
    // 净空地面：gy=20（hm 21 −1），脚位 = gy+1 = 21（26.1 pos.above()）。
    assert_eq!(spawn_column_feet_y(&v[..], &hm[..], 8, 8), Some(21));

    // 脚位实体（玻璃 damp=0 不抬 hm）：脚被堵 → 拒列。
    let mut v2 = v.clone();
    v2[lidx(8, 21, 8)] = BlockId(GLASS);
    assert_eq!(
        spawn_column_feet_y(&v2[..], &hm[..], 8, 8),
        None,
        "脚位被玻璃堵住须拒列"
    );

    // 头顶高处的玻璃棚：脚头两格净空 → 仍可投（上扫遇实体即停）。
    let mut v3 = v.clone();
    v3[lidx(8, 40, 8)] = BlockId(GLASS);
    assert_eq!(
        spawn_column_feet_y(&v3[..], &hm[..], 8, 8),
        Some(21),
        "高空棚下可投"
    );

    // 花：无碰撞非流体 → 可站在其中（26.1 花列放行）。
    let mut v4 = v.clone();
    v4[lidx(8, 21, 8)] = BlockId(FLOWER_RED);
    assert_eq!(
        spawn_column_feet_y(&v4[..], &hm[..], 8, 8),
        Some(21),
        "花列放行"
    );

    // 水列（海面/湖床）：地面上方出现流体 → 拒；浅水与齐海平面深水同判
    // （PlayerSpawnFinder.java:156-159 / :166 遇流体即弃）。
    for water_top in [21usize, 96] {
        let mut vw = v.clone();
        for y in 21..=water_top {
            vw[lidx(8, y, 8)] = BlockId(WATER);
        }
        let hm_w = hm_of(&vw);
        assert_eq!(
            spawn_column_feet_y(&vw[..], &hm_w[..], 8, 8),
            None,
            "水顶 {water_top} 的水列须拒"
        );
    }

    // 实体堆叠：顶变成堆顶 → 落在堆顶上（脚 = 新 gy+1）。
    let mut v5 = v.clone();
    v5[lidx(8, 21, 8)] = BlockId(STONE);
    let hm5 = hm_of(&v5);
    assert_eq!(
        spawn_column_feet_y(&v5[..], &hm5[..], 8, 8),
        Some(22),
        "堆顶可投"
    );

    // 全空列：recompute 回落 hm=1 → gy=0=空气，地面判定拒绝。
    let empty = Box::new([BlockId(0); 65536]);
    let hm_e = hm_of(&empty);
    assert_eq!(
        spawn_column_feet_y(&empty[..], &hm_e[..], 8, 8),
        None,
        "全空列拒"
    );
}

// ---- a：搜索窗级判定（经 stream() 全路径）----

#[test]
fn spawn_search_leaves_blocked_chunk_via_spiral() {
    let mut rt = headless_rt("blocked");
    rt.render_dist = 1; // 搜索窗 ±1，足以触达螺旋第二格 (1,0)
    for dx in -1i32..=1 {
        for dz in -1i32..=1 {
            rt.chunks
                .insert(ChunkPos::new(dx, dz), lit_chunk(dx, dz, 69));
        }
    }
    // 出生区块所有列的脚位放玻璃（damp0 实体，hm 不抬）：全列非法。
    {
        let h = rt.chunks[&ChunkPos::new(0, 0)].clone();
        let mut vg = h.voxels.write().unwrap();
        for z in 0..16usize {
            for x in 0..16usize {
                vg[lidx(x, 70, z)] = BlockId(GLASS);
            }
        }
    }
    rt.stream();
    // 螺旋（MinecraftServer.java:513-517 转向式）第二格 = (1,0)，其首个
    // 合法列 = 局部 (0,0)：脚 70，世界坐标 (16.5, 70.0, 0.5)。
    assert_eq!(
        rt.player.pos,
        Vec3::new(16.5, 70.0, 0.5),
        "出生区块全列非法时搜索应螺旋落到邻块"
    );
    assert!(rt.spawned);
}

#[test]
fn all_ocean_window_falls_back_above_water_surface() {
    let mut rt = headless_rt("ocean");
    rt.render_dist = 0; // 搜索窗退化为单块（模拟 ±3 全海洋的极限情形）
    let h = lit_chunk(0, 0, 20); // 石底 0..=20，hm=21
    {
        let mut vg = h.voxels.write().unwrap();
        for y in 21..=30usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    vg[lidx(x, y, z)] = BlockId(WATER);
                }
            }
        }
    }
    // 水不参与 heightmap（流体跳过），hm 仍 21：所有列上扫见水 → 全拒。
    rt.chunks.insert(ChunkPos::new(0, 0), h);
    rt.stream();
    // fixupSpawnHeight 兜底（PlayerSpawnFinder.java:89-104 收敛等价）：
    // 列内最高流体顶 30 + 1 = 31，落在水面之上，不再埋入海床。
    assert_eq!(
        rt.player.pos,
        Vec3::new(8.5, 31.0, 8.5),
        "全海洋窗口兜底 = 建议列水面之上"
    );
    assert!(rt.spawned);
}

// ---- b：真实 C++ 地形全路径（含真机取证 seed）----

#[test]
fn worldgen_spawn_never_buries_or_submerges() {
    // 首个 = 真机取证旧档 seed（458746403253）；其余覆盖不同海陆分布。
    for seed in [458746403253u64, 1, 42, 0xDEAD_BEEF] {
        let mut rt = headless_rt("gen-spawn");
        rt.seed = seed;
        // 调度器同步对齐（headless_rt 以固定 seed 构造）：窗外的缺失区块
        // 由 stream 后台请求，seed 不对齐会混入异种子地形干扰搜索窗。
        rt.scheduler = mcv_worldgen::TerrainScheduler::new(seed, mcv_core::world_worker_count());
        rt.player.pos = Vec3::ZERO;
        for dx in -3i32..=3 {
            for dz in -3i32..=3 {
                let h = Arc::new(ChunkHandle::new(ChunkPos::new(dx, dz)));
                mcv_worldgen::generate_into(&h, seed).expect("C++ 地形生成");
                rt.chunks.insert(ChunkPos::new(dx, dz), h);
            }
        }
        rt.stream();
        assert!(rt.spawned, "seed {seed}：搜索窗就绪即投放");
        let p = rt.player.pos;
        assert_ne!(p, Vec3::ZERO, "seed {seed}：投放必然改写哨兵位");
        assert_eq!(p.y.fract(), 0.0, "seed {seed}：脚位落在整数方块顶");
        let fx = p.x.floor() as i32;
        let fy = p.y as i32;
        let fz = p.z.floor() as i32;
        let id_at = |wx: i32, wy: i32, wz: i32| -> u16 {
            let h = &rt.chunks[&ChunkPos::new(wx.div_euclid(16), wz.div_euclid(16))];
            h.voxels.read().unwrap()[lidx(
                wx.rem_euclid(16) as usize,
                wy as usize,
                wz.rem_euclid(16) as usize,
            )]
            .id()
        };
        // a：脚 + 头两格 passable —— 不埋实体、不泡流体。
        for y in [fy, fy + 1] {
            let idd = id_at(fx, y, fz);
            let dd = def_of(idd);
            assert!(
                dd.is_some_and(|b| !b.solid && !b.liquid),
                "seed {seed}：y={y} 需 passable，实得 id={idd}（{}）",
                dd.map_or("未注册", |b| b.name)
            );
        }
        // 不落水：自脚位向上首个阻挡必须是实体（水面列已被搜索拒绝；
        // 全海洋兜底落水面之上时上方全空气，同样通过）。
        let mut y = fy;
        while y <= 255 {
            match def_of(id_at(fx, y, fz)) {
                Some(b) if b.liquid => {
                    panic!("seed {seed}：脚上方 y={y} 出现流体（落水中）");
                }
                Some(b) if b.solid => break,
                _ => y += 1,
            }
        }
        // 落点支撑：脚下为实地面（正常槽位），或为流体且恰在建议列
        // (8.5,·,8.5)（全海洋窗口 fixup 兜底，脚在水面之上）。
        let below_id = id_at(fx, fy - 1, fz);
        let below = def_of(below_id);
        let on_solid = below.is_some_and(|b| b.solid);
        let on_fallback = fx == 8 && fz == 8 && below.is_some_and(|b| b.liquid);
        assert!(
            on_solid || on_fallback,
            "seed {seed}：落点既非实地面也非兜底水面之上（below id={below_id}，{}）",
            below.map_or("未注册", |b| b.name)
        );
    }
}

// ---- d：存档位置优先 + seed/调度器对齐 ----

#[test]
fn saved_player_pos_skips_spawn_drop() {
    let mut rt = headless_rt("savedpos");
    let saved = Vec3::new(100.5, 64.0, -30.5);
    rt.player.pos = saved;
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            rt.chunks
                .entry(ChunkPos::new(dx, dz))
                .or_insert_with(|| lit_chunk(dx, dz, 69));
        }
    }
    rt.stream();
    assert_eq!(rt.player.pos, saved, "存档位置不被出生投放覆写");
    assert!(!rt.spawned, "pos==ZERO 才是待投放哨兵，非零存档位不置位");
}

#[test]
fn load_meta_restores_pos_and_realigns_scheduler_seed() {
    let mut rt = headless_rt("meta");
    rt.player.pos = Vec3::new(12.5, 65.0, -7.25);
    rt.save_meta();
    // 模拟 on_world_pick：重进构造携带与存档无关的 seed。
    let wrong_seed = rt.seed ^ 0x5555_5555_5555_5555 | 1;
    let mut rt2 = GameRuntime::new_headless(wrong_seed, rt.save_dir.clone(), GameMode::Survival);
    assert_ne!(rt2.seed, rt.seed, "前置：重进构造确实用错 seed");
    rt2.load_meta();
    assert_eq!(rt2.seed, rt.seed, "seed 从存档恢复");
    assert_eq!(
        rt2.player.pos,
        Vec3::new(12.5, 65.0, -7.25),
        "玩家位置从存档恢复"
    );
    assert!(!rt2.spawned, "非 ZERO 存档位不触发出生投放");
    // 调度器已按存档 seed 重建：其生成结果与按存档 seed 的纯函数逐位
    // 一致（若仍持错 seed，地形必然不同）。旧世界重进后新请求区块不再
    // 与存档地形接不上（边界断崖/落位悬空的根因）。
    rt2.scheduler.request(ChunkPos::new(5, 5));
    let got = match rt2.scheduler.results().recv() {
        Ok(mcv_worldgen::GenResult::Terrain(Ok(out))) => out,
        Ok(mcv_worldgen::GenResult::Terrain(Err((pos, rc)))) => {
            panic!("调度器生成失败：{pos:?} rc={rc}")
        }
        Err(e) => panic!("调度器结果通道关闭：{e}"),
    };
    let want = mcv_worldgen::generate_terrain(rt2.seed, ChunkPos::new(5, 5)).expect("纯函数生成");
    assert_eq!(
        got.voxels.as_u16_slice(),
        want.voxels.as_u16_slice(),
        "调度器 seed 与存档一致"
    );
}
