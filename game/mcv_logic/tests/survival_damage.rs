//! 生存伤害与死亡闭环域回归锁（fix/survival-damage-wave，任务板 #95）：
//! 岩浆接触点燃扣血、火焰方块点燃、窒息、虚空、饿死走管线、死亡掉全背包、
//! 游泳 exhaustion。Java 依据均为 /root/mc-ref/src-26.1 反编译实读
//! （断言注释附 file:line；与派单行号不符处以实读为准）。

use std::{path::PathBuf, sync::Arc};

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_logic::game::{GameMode, GamePhase, GameRuntime};

/// 唯一临时存档目录（测试并行安全）。
fn tmp_world(tag: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-survival-{}-{:x}", tag, h.finish()))
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

/// 加载态收尾：出生点邻域 49 块顶到 Uploaded 并步进到游玩态
/// （同 tests/runtime.rs::finish_loading 模式）。
fn finish_loading(rt: &mut GameRuntime) {
    rt.player.vel = Vec3::ZERO;
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            let pos = ChunkPos::new(dx, dz);
            rt.chunks
                .entry(pos)
                .or_insert_with(|| Arc::new(ChunkHandle::new(pos)))
                .advance_to(Stage::Uploaded);
        }
    }
    for _ in 0..10 {
        rt.fixed_step(1.0 / 60.0);
        if rt.phase == GamePhase::Playing {
            return;
        }
    }
    panic!("加载态未在预算步内转游玩态");
}

/// 测试场地：压实三列石头到 y=69（顶面 y=70），工作区 3×3 列自 y=70 向
/// 上全部清空（同 tests/runtime.rs::build_platform 的防嵌固做法）。玩家站
/// (8.3, 70, 8.5)：AABB x∈[8.0,8.6]/z∈[8.2,8.8] 只占 (8,8) 列。
fn build_platform(rt: &mut GameRuntime) {
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    {
        let mut v = h.voxels.write().unwrap();
        for (x, z) in [(8usize, 8usize), (8, 7), (7, 8)] {
            for y in 0..70usize {
                v[y << 8 | z << 4 | x] = BlockId(1);
            }
        }
        for y in 70..86usize {
            for z in 7..10usize {
                for x in 7..10usize {
                    v[y << 8 | z << 4 | x] = BlockId(0);
                }
            }
        }
    }
    h.advance_to(Stage::TerrainReady);
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
    rt.player.pos = Vec3::new(8.3, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
}

/// 单格写体素（区块 (0,0) 内）。
fn set_block(rt: &mut GameRuntime, x: usize, y: usize, z: usize, id: BlockId) {
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    h.voxels.write().unwrap()[y << 8 | z << 4 | x] = id;
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
}

/// 按注册名查方块 id。
fn id_of(name: &str) -> u16 {
    (0..mcv_core::BLOCKS.len() as u16)
        .find(|i| mcv_core::BLOCKS[*i as usize].name == name)
        .unwrap()
}

/// app.rs 固定步驱动循环复刻：frames 帧 60fps → 等量 1/60 步。
fn run_steps(rt: &mut GameRuntime, steps: usize) {
    for _ in 0..steps {
        rt.fixed_step(1.0 / 60.0);
    }
}

/// 驱动 n 个 20 Hz game tick（3 步/tick，另 +2 步余量保证最后一个 tick 跨界）。
fn run_ticks(rt: &mut GameRuntime, ticks: usize) {
    run_steps(rt, ticks * 3 + 2);
}

// ---------------------------------------------------------------------------
// #95-1 岩浆接触伤害 + 点燃（HIGH）
// ---------------------------------------------------------------------------

/// 没入岩浆掉血：脚部格在 lava → lavaHurt 4.0（Entity.java:613-624）+
/// lavaIgnite 点燃 300 tick（:607-611 igniteForSeconds(15)）。
#[test]
fn lava_contact_deals_4_damage_and_ignites() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("lava"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    let lava = id_of("lava");
    // 脚部格 (8,70,8) 换岩浆（站上去：岩浆无碰撞，玩家落在 y=70 石顶，
    // 脚底中心 (8.3,70,8.5) 所在格 = lava）。
    set_block(&mut rt, 8, 70, 8, BlockId(lava));

    assert_eq!(rt.player.health, 20.0);
    run_ticks(&mut rt, 1);
    assert!(
        (rt.player.health - 16.0).abs() < 1e-4,
        "首 tick 岩浆 4.0 伤害，got {}",
        rt.player.health
    );
    assert!(
        rt.fire_ticks >= 15 * 20 - 5 && rt.fire_ticks > 0,
        "lavaIgnite 点燃 300 tick（每 tick −1 后），got {}",
        rt.fire_ticks
    );

    // 节奏：i 帧门在 invulnerableTime==10 处放行（LivingEntity.java:1196）
    // → 岩浆实际每 10 tick（0.5s）一跳 4.0，20 tick 后 20−8=12。
    run_ticks(&mut rt, 19);
    assert!(
        (rt.player.health - 12.0).abs() < 1e-4,
        "岩浆 0.5s/4.0 节奏（源码 i 帧门实况），got {}",
        rt.player.health
    );
}

/// 创造免岩浆（hurt_player 创造豁免在伤害侧；原版创造对环境伤害同样免伤）。
#[test]
fn creative_immune_to_lava_damage() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("lavac"), GameMode::Creative);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    set_block(&mut rt, 8, 70, 8, BlockId(id_of("lava")));
    run_ticks(&mut rt, 20);
    assert_eq!(rt.player.health, 20.0, "创造不因岩浆掉血");
}

/// 离开岩浆后余燃继续：on_fire 每 20 tick 1.0（Entity.java:538-540）。
#[test]
fn burning_continues_after_leaving_lava() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("burn"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    set_block(&mut rt, 8, 70, 8, BlockId(id_of("lava")));
    run_ticks(&mut rt, 2);
    let hp_after_lava = rt.player.health;
    assert!(hp_after_lava < 20.0, "岩浆先扣血");
    // 清掉脚下岩浆 → 离开岩浆，余燃账（fire_ticks>0）继续结算。
    set_block(&mut rt, 8, 70, 8, BlockId(0));
    let fire_before = rt.fire_ticks;
    assert!(fire_before > 0, "余燃持续");
    // on_fire 节拍：fire_ticks%20==0 处 1.0（Entity.java:538-540）。
    run_ticks(&mut rt, 20);
    assert!(
        (rt.player.health - (hp_after_lava - 1.0)).abs() < 1e-4,
        "离火后 20 tick 内 on_fire 恰好一跳 1.0，got {}",
        rt.player.health
    );
    assert!(rt.fire_ticks < fire_before, "燃烧账随 tick 递减");
}

/// 防火效果免疫火系伤害（LivingEntity.hurtServer:1163-1165 IS_FIRE 门）。
#[test]
fn fire_resistance_blocks_lava_damage() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("fireres"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    // 施加防火（EffectBook.apply_simple：kind/duration tick/amplifier）。
    rt.effects
        .apply_simple(mcv_entity::Kind::FireResistance, 20 * 60, 0);
    set_block(&mut rt, 8, 70, 8, BlockId(id_of("lava")));
    run_ticks(&mut rt, 20);
    assert_eq!(rt.player.health, 20.0, "防火免疫 lava/on_fire");
}
