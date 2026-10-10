//! 生存伤害与死亡闭环域回归锁（fix/survival-damage-wave，任务板 #95）：
//! 岩浆接触点燃扣血、火焰方块点燃、窒息、虚空、饿死走管线、死亡掉全背包、
//! 游泳 exhaustion。Java 依据均为 /root/mc-ref/src-26.1 反编译实读
//! （断言注释附 file:line；与派单行号不符处以实读为准）。

use std::{path::PathBuf, sync::Arc};

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, Stage};
use mcv_logic::game::{GameMode, GamePhase, GameRuntime, WORLD_MIN_Y};

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
                v[mcv_core::vidx(x, y as i32, z)] = BlockId(1);
            }
        }
        for y in 70..86usize {
            for z in 7..10usize {
                for x in 7..10usize {
                    v[mcv_core::vidx(x, y as i32, z)] = BlockId(0);
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
    h.voxels.write().unwrap()[mcv_core::vidx(x, y as i32, z)] = id;
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

/// 驱动 n 个 20 Hz game tick。tick 按 3 步/tick 落在 60 Hz 步网格上
/// （accumulate_ticks：frac 每 1/60 步 +1/3，第 3 步整数过界 → 恰好 n 个
/// tick、零余量；前任的 +2 余量反而把边界顶出第 n+1 个 tick，使 tick 账
/// 随 frac 残留漂移 1——固测试节奏先修这个）。
fn run_ticks(rt: &mut GameRuntime, ticks: usize) {
    run_steps(rt, ticks * 3);
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
        rt.fire_ticks >= 15 * 20 - 5,
        "lavaIgnite 点燃 300 tick（每 tick −1 后），got {}",
        rt.fire_ticks
    );

    // 节奏：i 帧门在 invulnerableTime==10 处放行（LivingEntity.java:1196）
    // → 岩浆实际每 10 tick（0.5s）一跳 4.0：t1、t11 两跳 = 20−8=12。但
    // FoodData 自然回血同样每拍结算（FoodData.java:44-52 无条件 tick，游戏
    // 规则 NATURAL_HEALTH_REGENERATION 默认 true）：t11 先回 min(饱和,6)/6
    // = 5.0/6（起始饱和 5.0、food=20、受伤），再吃岩浆跳 → 12 + 5/6。
    // 回血代价 exhaustion+5：t12 起 5>4 扣饱和 → 下轮回血量 4/6（本窗不达）。
    run_ticks(&mut rt, 19);
    assert!(
        (rt.player.health - (12.0 + 5.0 / 6.0)).abs() < 1e-4,
        "岩浆 0.5s/4.0 节奏 + FoodData 自然回血（FoodData.java:48-49），got {}",
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
    // 燃烧账 t1 置 300 并 −1 → 299；t2 lavaIgnite 只增不减再回 300、拍末
    // 仍 299；离岩浆后每拍 −1，t21（=280）首撞 %20==0 → 恰一跳 1.0（此刻
    // invul 早已衰减见底，全额过门）。同时 FoodData 自然回血两轮：t11 回
    // 5.0/6（起始饱和 5.0）、t12 扣饱和 → t21 回 4.0/6（FoodData.java:44-52）。
    // 终值 = 16 − 1 + 5/6 + 4/6。
    run_ticks(&mut rt, 20);
    assert!(
        (rt.player.health - (hp_after_lava - 1.0 + 5.0 / 6.0 + 4.0 / 6.0)).abs() < 1e-4,
        "离火后 20 tick 内 on_fire 恰一跳 1.0 + FoodData 两轮回血，got {}",
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

// ---------------------------------------------------------------------------
// #95-1b 饿死走完整受伤管线（FoodData.java:60-68）
// ---------------------------------------------------------------------------

/// hunger=0 的 80 tick 门开拍 → starve 1.0 走 hurtServer 完整管线
/// （FoodData.java:64 `player.hurtServer(…starve(), 1.0F)`；难度封顶门
/// Normal `health>1` :63。管线侧证 = hurt 结算副作用 invulnerableTime=20
/// （LivingEntity.java:1206）与 lastHurt=1.0（:1204）——旧实现就地
/// `health -= 1.0` 绕门，两者均不会置位）。
#[test]
fn starvation_routes_through_hurt_pipeline() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("starve"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    rt.player.hunger = 0.0;
    rt.player.saturation = 0.0;
    // FoodData 第三路 hunger≤0：tickTimer 1..80 → 第 80 拍门开
    // （FoodData.java:61-62），封顶门 Normal health=20>1 放行 → 19.0。
    run_ticks(&mut rt, 80);
    assert_eq!(
        rt.player.health, 19.0,
        "80 tick 门首拍饿死 1.0（FoodData.java:61-64），got {}",
        rt.player.health
    );
    // 管线侧证：完整 hurtServer 结算置 invulnerableTime=20 + lastHurt=1.0
    // （LivingEntity.java:1206/:1204）——就地直扣两值均留 0，此处即接线
    // 证伪点；次一 80 tick 拍（:1196 门早已过期）照常再掉 1 → 18.0。
    assert_eq!(
        rt.player.invulnerable, 20,
        "饿死走完整管线 → invulnerableTime=20（LivingEntity.java:1206），got {}",
        rt.player.invulnerable
    );
    assert_eq!(
        rt.player.last_hurt, 1.0,
        "饿死走完整管线 → lastHurt=1.0（LivingEntity.java:1204），got {}",
        rt.player.last_hurt
    );
    run_ticks(&mut rt, 80);
    assert_eq!(
        rt.player.health, 18.0,
        "第二个 80 tick 拍再掉 1（FoodData.java:61-64），got {}",
        rt.player.health
    );
}

// ---------------------------------------------------------------------------
// #95-2 火焰方块接触点燃（Entity.java:539 伤害节奏按源）
// ---------------------------------------------------------------------------

/// 站火格 → 点燃 160 tick + in_fire 1.0/tick（i 帧后 0.5s/1.0 节奏，
/// LivingEntity.java:1196 门在 invulnerableTime==10 处放行）。
#[test]
fn fire_block_ignites_and_deals_damage() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("fireblk"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    set_block(&mut rt, 8, 70, 8, BlockId(id_of("fire")));
    run_ticks(&mut rt, 1);
    assert!(
        (rt.player.health - 19.0).abs() < 1e-4,
        "火焰方块 in_fire 1.0，got {}",
        rt.player.health
    );
    assert!(
        rt.fire_ticks >= 8 * 20 - 5,
        "fireIgnite 点燃 160 tick，got {}",
        rt.fire_ticks
    );
    // 20 tick（1s）实际节奏：t1（in_fire）、t11（fire_ticks=160 首撞
    // %20==0，on_fire 与 in_fire 同拍同为 1.0，只过一跳）→ 20−2=18；t11
    // 先走 FoodData 自然回血 min(饱和,6)/6 = 5/6（FoodData.java:44-52，
    // 起始饱和 5.0、food=20、受伤）→ 18 + 5/6。回血代价 exhaustion+5，
    // t12 扣饱和 → 下轮回血量 4/6（本窗不达）。
    run_ticks(&mut rt, 19);
    assert!(
        (rt.player.health - (18.0 + 5.0 / 6.0)).abs() < 1e-4,
        "火焰 0.5s/1.0 实际节奏 + FoodData 自然回血（FoodData.java:48-49），got {}",
        rt.player.health
    );
}

/// 魂火双倍：in_fire 2.0（SoulFireBlock.java:22）。
#[test]
fn soul_fire_deals_double_damage() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("soulfire"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    set_block(&mut rt, 8, 70, 8, BlockId(id_of("soul_fire")));
    run_ticks(&mut rt, 1);
    assert!(
        (rt.player.health - 18.0).abs() < 1e-4,
        "魂火 in_fire 2.0，got {}",
        rt.player.health
    );
}

// ---------------------------------------------------------------------------
// #95-3 窒息（MEDIUM）
// ---------------------------------------------------------------------------

/// 头埋实心方块 → 1.0/tick 走 hurt_player，i 帧门节流成 ~1/s
/// （LivingEntity.java:405-406 in_wall；Entity.isInWall:2164-2182 眼盒求交）。
#[test]
fn suffocation_deals_1_per_second() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("suffocate"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    // 眼位 (8.3, 71.62, 8.5) 所在格 (8,71,8) 填石头。
    set_block(&mut rt, 8, 71, 8, BlockId(1));
    run_ticks(&mut rt, 1);
    assert!(
        (rt.player.health - 19.0).abs() < 1e-4,
        "首拍 in_wall 1.0，got {}",
        rt.player.health
    );
    // 20 tick 实际节奏：i 帧门放行两跳（t1、t11）→ 20−2=18；t11 处
    // FoodData 自然回血先于伤害结算（min(饱和,6)/6 = 5/6，FoodData.java:
    // 44-52：起始饱和 5.0、food=20、受伤；回血代价 exhaustion+5 → t12 扣
    // 饱和，下轮 4/6 本窗不达）→ 18 + 5/6。
    run_ticks(&mut rt, 19);
    assert!(
        (rt.player.health - (18.0 + 5.0 / 6.0)).abs() < 1e-4,
        "窒息 ~1/s（i 帧实况 0.5s/跳）+ FoodData 自然回血，got {}",
        rt.player.health
    );
}

/// 头部格空 → 不窒息（对照）。
#[test]
fn no_suffocation_with_clear_head() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("suffocatectl"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    run_ticks(&mut rt, 40);
    assert_eq!(rt.player.health, 20.0, "露天不窒息");
}

// ---------------------------------------------------------------------------
// #95-4 虚空伤害规则（MEDIUM）
// ---------------------------------------------------------------------------

/// 生存虚空：y < WORLD_MIN_Y−64（Entity.checkBelowWorld:579-583）→
/// fellOutOfWorld 4.0/tick（LivingEntity.java:2142-2144）走正常管线，
/// i 帧节流 0.5s/跳，掉血直至死。
#[test]
fn void_deals_4_per_tick_until_death_survival() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("void"), GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    finish_loading(&mut rt);
    rt.player.pos = Vec3::new(8.3, WORLD_MIN_Y - 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    // 阈值以下不触发（y=−63 > −64）。
    rt.player.pos.y = WORLD_MIN_Y - 63.0;
    run_ticks(&mut rt, 2);
    assert_eq!(rt.player.health, 20.0, "y=−63 未越界不掉血");
    // 越界（y=−70 < −64）→ 每 10 tick 一跳 4.0（i 帧门放行节奏）：本窗
    // t3（invul 从 0 全额过门）、t13（invul 衰减到 10 再过门）两跳
    // = 20−8=12；t13 处 FoodData 自然回血先于虚空伤害（min(饱和,6)/6
    // = 5/6，FoodData.java:44-52：起始饱和 5.0、food=20、受伤；回血代价
    // exhaustion+5 → t14 扣饱和，下轮 4/6 本窗不达）→ 12 + 5/6。
    rt.player.pos.y = WORLD_MIN_Y - 70.0;
    run_ticks(&mut rt, 11);
    assert!(
        (rt.player.health - (12.0 + 5.0 / 6.0)).abs() < 1e-4,
        "两跳 4.0（t3、t13）+ FoodData 自然回血，got {}",
        rt.player.health
    );
    // 持续坠落 → 死亡置位（不再复活传送外的豁免）。
    let mut guarded = 0;
    while !rt.dead && guarded < 200 {
        rt.fixed_step(1.0 / 60.0);
        guarded += 1;
    }
    assert!(rt.dead, "生存虚空持续掉血致死（{guarded} 步内）");
    assert_eq!(rt.player.health, 0.0);
}

/// 创造虚空照样死：out_of_world ∈ bypasses_invulnerability
/// （tags/damage_type/bypasses_invulnerability.json）穿过创造免疫
/// （Entity.isInvulnerableToBase:2955-2960）——旧实现创造无限坠落软锁。
#[test]
fn void_kills_creative_too() {
    let mut rt = GameRuntime::new_headless(7, tmp_world("voidc"), GameMode::Creative);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    finish_loading(&mut rt);
    rt.player.pos = Vec3::new(8.3, WORLD_MIN_Y - 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    let mut guarded = 0;
    while !rt.dead && guarded < 200 {
        rt.fixed_step(1.0 / 60.0);
        guarded += 1;
    }
    assert!(rt.dead, "创造虚空穿过免疫照样死（{guarded} 步内）");
    assert_eq!(rt.player.health, 0.0);
}
