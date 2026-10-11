//! 生长波引擎接线回归锁（#146）——GameRuntime.growth_tick 结算面（仅
//! CI 跑：完整 crate 依赖树，本机 rustc harness 不含）。纯决策域锁在
//! `tests/growth.rs`。
//!
//! 覆盖：random tick 只写目标方块（草方块世界零扰动）、树苗上方被挡不生长
//! （STAGE=1 也不越门）、STAGE=1 合立地苗最终长成树（大样本确定性预算）、
//! 右键不催熟（无骨粉路径）、作物收获物品注册表缺口（GROWTH-1 ② 锁）。

use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkHandle, ChunkPos, ChunkStatus, Stage, vidx};
use mcv_logic::game::{GameMode, GamePhase, GameRuntime};
use mcv_logic::growth::{self, ids};

/// 唯一临时存档目录（测试并行安全）。
fn tmp_world(tag: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-growth-{}-{:x}", tag, h.finish()))
}

/// 流式加载直到出生区块地形就绪。
fn wait_terrain(rt: &mut GameRuntime, pos: ChunkPos) {
    for _ in 0..3000 {
        rt.stream();
        if rt
            .chunks
            .get(&pos)
            .is_some_and(|h| (h.stage() as u8) >= (Stage::TerrainReady as u8))
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("地形未在预算内就绪");
}

/// 压平区块 (0,0)：y<64 全石、上方清空（顶面 y=64 石面——生长画布），
/// 并把光照填成「天空 15 全柱」（树苗/作物亮度门 fixture；
/// ChunkHandle 新建光全 0，不填则天光门恒挂）。
fn build_flat(rt: &mut GameRuntime) {
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    {
        let mut v = h.voxels.write().unwrap();
        for y in 0..64usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    v[vidx(x, y as i32, z)] = BlockId(1);
                }
            }
        }
        for y in 64..96usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    v[vidx(x, y as i32, z)] = BlockId(0);
                }
            }
        }
    }
    {
        let mut l = h.light.write().unwrap();
        for i in 0..l.len() {
            l[i] = 0xF0; // sky=15, block=0（高 nibble 天光）
        }
    }
    h.advance_to(Stage::TerrainReady);
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
}

/// 出生区块压实 + 玩家落到 (2.5, 65, 8.5) + 5×5 邻域顶到 Uploaded（生长
/// 域 Chebyshev ±2 全覆盖），步进到游玩态（tests/movement.rs::enter_playing
/// 模式，域扩到 5×5）。
fn enter_playing(rt: &mut GameRuntime) {
    wait_terrain(rt, ChunkPos::new(0, 0));
    build_flat(rt);
    rt.player.pos = Vec3::new(2.5, 65.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    for dx in -2i32..=2 {
        for dz in -2i32..=2 {
            let pos = ChunkPos::new(dx, dz);
            rt.chunks
                .entry(pos)
                .or_insert_with(|| Arc::new(ChunkHandle::new(pos)))
                .advance_to(Stage::Uploaded);
            rt.chunks[&pos].advance_status(ChunkStatus::Full);
        }
    }
    // 邻域块光照同款填天空（域内所有块都要过亮度门）。
    for dx in -2i32..=2 {
        for dz in -2i32..=2 {
            if let Some(h) = rt.chunks.get(&ChunkPos::new(dx, dz)) {
                let mut l = h.light.write().unwrap();
                for v in l.iter_mut() {
                    *v = 0xF0;
                }
            }
        }
    }
    for _ in 0..60 {
        rt.fixed_step(1.0 / 60.0);
        if rt.phase == GamePhase::Playing {
            return;
        }
    }
    panic!("加载态未在预算步内转游玩态");
}

fn run_ticks(rt: &mut GameRuntime, steps: usize) {
    for _ in 0..steps {
        rt.fixed_step(1.0 / 60.0);
    }
}

fn set_cell(rt: &mut GameRuntime, p: BlockPos, id: BlockId) {
    let h = rt.chunks.get(&p.chunk()).unwrap().clone();
    let [lx, _, lz] = p.local();
    h.voxels.write().unwrap()[vidx(lx, p.y, lz)] = id;
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
}

/// 列遮光（fixture 光照维护：实体块压顶后，该列 from_y 以上天光归 0）。
fn shade_column(rt: &mut GameRuntime, x: i32, z: i32, from_y: i32) {
    let h = rt
        .chunks
        .get(&BlockPos::new(x, 0, z).chunk())
        .unwrap()
        .clone();
    let [lx, _, lz] = BlockPos::new(x, 0, z).local();
    let mut l = h.light.write().unwrap();
    for y in from_y..mcv_core::WORLD_MAX_Y {
        l[vidx(lx, y, lz)] = 0;
    }
}

fn get_cell(rt: &mut GameRuntime, p: BlockPos) -> BlockId {
    let h = rt.chunks.get(&p.chunk()).unwrap().clone();
    let [lx, _, lz] = p.local();
    h.voxels.read().unwrap()[vidx(lx, p.y, lz)]
}

fn count_id(rt: &mut GameRuntime, chunk: ChunkPos, id: u16) -> usize {
    let h = rt.chunks.get(&chunk).unwrap().clone();
    h.voxels
        .read()
        .unwrap()
        .iter()
        .filter(|b| b.id() == id)
        .count()
}

/// random tick 只写目标方块：纯草方块世界（无苗无麦）跑 400 步（≈133
/// tick，约 4790 次尝试），全区块体素零变化——草方块蔓延未建模，
/// growth 决策面对非目标恒 None（tests/growth.rs
/// random_tick_only_acts_on_registered_targets 的引擎半边）。
#[test]
fn grass_world_random_tick_leaves_voxels_untouched() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("grass"), GameMode::Survival);
    enter_playing(&mut rt);
    let before: Vec<u16> = rt.chunks[&ChunkPos::new(0, 0)]
        .voxels
        .read()
        .unwrap()
        .iter()
        .map(|b| b.0)
        .collect();
    run_ticks(&mut rt, 400);
    let after: Vec<u16> = rt.chunks[&ChunkPos::new(0, 0)]
        .voxels
        .read()
        .unwrap()
        .iter()
        .map(|b| b.0)
        .collect();
    assert_eq!(before, after, "无目标方块时 random tick 不得改写体素");
}

/// 树苗上方被实体块挡（光 0）→ STAGE=1 也不长；对照位（敞空 STAGE=1 苗）
/// 同预算内长成树（log 落苗原位）。大样本：满场 STAGE=1 苗 20×20，1200 步
/// （≈400 tick × 1800 尝试/tick）→ 期望命中 ~17 棵（区块 (0,0) 内 ~8），
/// log≥1 为 ~4σ 预算。
#[test]
fn sapling_light_gate_then_tree_at_spot() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("gate"), GameMode::Survival);
    enter_playing(&mut rt);
    // 满场 STAGE=1 oak 苗（y=65，立地在石面 y=64）。
    for z in 2..22 {
        for x in 2..22 {
            let p = BlockPos::new(x, 65, z);
            set_cell(
                &mut rt,
                p,
                growth::with_sapling_stage(BlockId(ids::OAK_SAPLING), 1),
            );
        }
    }
    // 挡光样本：苗上方压石头 + 列遮光（亮度门吃光照存储，fixture 同步）。
    let blocked = BlockPos::new(3, 65, 3);
    set_cell(&mut rt, BlockPos::new(3, 66, 3), BlockId(1));
    shade_column(&mut rt, 3, 3, 66);
    run_ticks(&mut rt, 1200);
    // ① 被挡苗必须原样（叶/原木替换门也进不来：苗非 validTreePos——
    //    tags/block/replaceable_by_trees.json 无 #saplings）。
    assert_eq!(
        get_cell(&mut rt, blocked).id(),
        ids::OAK_SAPLING,
        "上方被挡的苗不得长成树"
    );
    // ② 敞空苗群最终长树：log 数 > 0（苗本身不是 log）。
    let logs = count_id(&mut rt, ChunkPos::new(0, 0), 6);
    assert!(
        logs > 0,
        "敞空 STAGE=1 苗群 400 tick 内应长出树（log={logs}）"
    );
    // ③ 树基 = 苗原位：y=65 行的每根 log 其下必为非 log 立地（石面）。
    let h = rt.chunks[&ChunkPos::new(0, 0)].clone();
    let v = h.voxels.read().unwrap();
    let mut bases = 0usize;
    for z in 0..16usize {
        for x in 0..16usize {
            if v[vidx(x, 65, z)].id() == 6 {
                bases += 1;
                assert_eq!(v[vidx(x, 64, z)].id(), 1, "树基下方应是石面（非悬空）");
            }
        }
    }
    assert!(bases > 0, "应存在落在苗行 y=65 的树基");
}

/// 右键不催熟：空手对苗位右键（place=true）后 STAGE 0 保持——本仓无
/// 骨粉路径，生长只经 random tick（骨粉另波）。
#[test]
fn right_click_does_not_force_growth() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("noclick"), GameMode::Survival);
    enter_playing(&mut rt);
    let p = BlockPos::new(8, 65, 8);
    set_cell(&mut rt, p, BlockId(ids::OAK_SAPLING)); // STAGE 0
    // 空手右键 ×3（interact 内部无 growth 调用路径）。
    for _ in 0..3 {
        rt.interact(true);
    }
    let cell = get_cell(&mut rt, p);
    assert_eq!(cell.id(), ids::OAK_SAPLING);
    assert_eq!(growth::sapling_stage(cell), 0, "右键不得催熟（STAGE 仍 0）");
}

/// 作物收获物品缺口锁（GROWTH-1 ②）：ITEMS（#144 在扩）现无 wheat/
/// wheat_seeds → item_by_name 解析 None。#144 合入后本断言翻转、掉落
/// 自动接通——红即提示勿重复登记 DROP_TABLE 小麦行（双重掉落）。
#[test]
fn wheat_items_absent_until_items_wave_lands() {
    assert!(mcv_item::item_by_name("wheat").is_none());
    assert!(mcv_item::item_by_name("wheat_seeds").is_none());
}

/// 作物 age 位随体素存储 roundtrip（状态 nibble 进存档面：SAVE 标记 +
/// 体素直读写，引擎半边）。
#[test]
fn crop_age_survives_voxel_roundtrip() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("age"), GameMode::Survival);
    enter_playing(&mut rt);
    let p = BlockPos::new(8, 65, 8);
    set_cell(&mut rt, p, BlockId(ids::FARMLAND));
    let above = BlockPos::new(8, 66, 8);
    set_cell(
        &mut rt,
        above,
        growth::with_crop_age(BlockId(ids::WHEAT), 5),
    );
    let cell = get_cell(&mut rt, above);
    assert_eq!(cell.id(), ids::WHEAT);
    assert_eq!(growth::crop_age(cell), 5);
}
