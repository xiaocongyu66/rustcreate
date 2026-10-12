//! GameRuntime 无头集成测试（new_headless：NullMesher 不触 GPU）。
//! 固定 seed 世界生成 + 手工压实的测试列，验证：生存挖矿 → ItemDrop →
//! delay 后拾取入栏；创造秒破零掉落；交互到达距离按模式 4.5/5.0；
//! 玩家死亡掉全部物品（40 tick 延迟）。

use std::{path::PathBuf, sync::Arc};

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, ChunkStatus, Stage};
use mcv_logic::game::{GameMode, GamePhase, GameRuntime};

/// 唯一临时存档目录（测试并行安全）。
fn tmp_world(tag: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-runtime-{}-{:x}", tag, h.finish()))
}

/// 流式加载直到出生区块地形就绪（后台 worker 有真实延迟，轮询等待）。
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

/// 加载态收尾：把出生点邻域（半径 3，49 块 = 26.1 EXPECTED_PLAYER_CHUNKS
/// = Mth.square(7)，LevelLoadProgressTracker.java:15）全部顶到 Uploaded
/// 并步进到游玩态。真实路径 = 网格建好上传后 stream 推进 Uploaded
/// （game.rs mesh 循环 advance_to）；无头 NullMesher 不产网格，按
/// mobs_runtime.rs:44 同款手工推进。刀 3b 门谓词解耦（stage==Uploaded →
/// status ≥ Surface）后，fixture 同步置位 status（生产管线中
/// advance_status_by_level 按票级别授权推进；本 fixture 绕过 stream，
/// 镜像真实终态：级别 ≤33 邻域齐 → Full）。
fn finish_loading(rt: &mut GameRuntime) {
    rt.player.pos = Vec3::new(8.3, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            let pos = ChunkPos::new(dx, dz);
            rt.chunks_mut()
                .entry(pos)
                .or_insert_with(|| Arc::new(ChunkHandle::new(pos)))
                .advance_to(Stage::Uploaded);
            rt.chunks[&pos].advance_status(ChunkStatus::Full);
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
/// 上全部清空（seed 地形可能天然堆高：玩家嵌进实体会被逐块顶出、越抬
/// 越高，step_mining 的 5.5 距离守卫随即 abort——挖矿永不完成）。玩家站
/// (8.3, 70, 8.5)：AABB x∈[8.0,8.6] 只占 x=8 列，与眼平高靶方块 (7,71,8)
/// 不重叠（重叠会把玩家顶到靶块顶上，掉落物落在拾取盒之下收不走）。
fn build_platform(rt: &mut GameRuntime) {
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    {
        let mut v = h.voxels.write().unwrap();
        for (x, z) in [(8usize, 8usize), (8, 7), (7, 8)] {
            for y in 0..70usize {
                v.set_flat(mcv_core::vidx(x, y as i32, z), BlockId(1));
            }
        }
        for y in 70..86usize {
            for z in 7..10usize {
                for x in 7..10usize {
                    v.set_flat(mcv_core::vidx(x, y as i32, z), BlockId(0));
                }
            }
        }
    }
    h.advance_to(Stage::TerrainReady);
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
    rt.player.pos = Vec3::new(8.3, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
}

/// 眼睛对准 (7,71,8) 方块（眼平高的靶块）。瞄准点 z 取 8.499 避开
/// atan2 负零分支（d.z=+0.0 时 -d.z=-0.0 → yaw=-π 朝 +z，射线打偏）。
fn set_aim_at_block(rt: &mut GameRuntime) {
    let eye = rt.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
    let d = Vec3::new(7.5, 71.5, 8.499) - eye;
    rt.player.yaw = d.x.atan2(-d.z);
    rt.player.pitch = (d.y / d.length()).asin();
}

fn set_block(rt: &mut GameRuntime, x: usize, y: usize, z: usize, id: BlockId) {
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    h.voxels
        .write()
        .unwrap()
        .set_flat(mcv_core::vidx(x, y as i32, z), id);
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
}

fn block_at(rt: &GameRuntime, x: usize, y: usize, z: usize) -> BlockId {
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    h.voxels
        .read()
        .unwrap()
        .get_flat(mcv_core::vidx(x, y as i32, z))
}

fn hotbar_count(rt: &GameRuntime, item: u16) -> u32 {
    rt.hotbar
        .slots
        .iter()
        .filter(|s| s.item == item)
        .map(|s| s.count as u32)
        .sum()
}

/// 生存：木镐挖石头 → ItemDrop 生成（默认 10 tick 拾取延迟）→ 贴身玩家
/// 在 delay 后数步内拾取入栏。实体存在性/延迟语义由
/// mcv_entity/tests/item_drops.rs 覆盖，这里验证端到端 give 路径。
#[test]
fn survival_mining_stone_drops_and_pickup() {
    let dir = tmp_world("mine");
    let mut rt = GameRuntime::new_headless(20261009, dir, GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    set_block(&mut rt, 7, 71, 8, BlockId(1)); // 眼平高靶块
    set_aim_at_block(&mut rt);
    // 石头掉落需要镐（has_correct_tool 门控），给选中槽木镐。
    rt.hotbar.slots[0] = mcv_item::ItemStack::new(mcv_item::WOODEN_PICKAXE_INDEX, 1);
    rt.player.sel_slot = 0;
    assert_eq!(hotbar_count(&rt, mcv_item::COBBLESTONE), 0);

    rt.input.mining = true;
    rt.on_left_press();
    let mut mined_at = None;
    for i in 0..600 {
        rt.fixed_step(1.0 / 60.0);
        if block_at(&rt, 7, 71, 8).0 == 0 {
            mined_at = Some(i);
            break;
        }
    }
    let mined = mined_at.expect("按住左键应在预算 tick 内挖掉石头");
    // 掉落生成带 10 tick（30 步）拾取延迟 → 40 步内必入栏。
    let mut got_at = None;
    for k in 0..45 {
        rt.fixed_step(1.0 / 60.0);
        if hotbar_count(&rt, mcv_item::COBBLESTONE) >= 1 {
            got_at = Some(k);
            break;
        }
    }
    assert!(
        got_at.is_some(),
        "贴身挖掘的掉落物须被拾取入栏（mined@{mined}）"
    );
    assert_eq!(
        rt.mobs_app.world.component_count::<mcv_entity::ItemDrop>(),
        0,
        "拾取完成掉落物 despawn"
    );
}

/// 创造：秒破不留掉落物、快捷栏数量不变。
#[test]
fn creative_mining_produces_no_drop() {
    let dir = tmp_world("creative");
    let mut rt = GameRuntime::new_headless(20261009, dir, GameMode::Creative);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    set_block(&mut rt, 7, 71, 8, BlockId(1));
    set_aim_at_block(&mut rt);
    let cobble_before = hotbar_count(&rt, mcv_item::COBBLESTONE);

    rt.on_left_press(); // 创造：秒破路径
    assert_eq!(block_at(&rt, 7, 71, 8).0, 0, "创造单击必秒破");
    for _ in 0..30 {
        rt.fixed_step(1.0 / 60.0);
    }
    assert_eq!(
        rt.mobs_app.world.component_count::<mcv_entity::ItemDrop>(),
        0,
        "创造破坏不留掉落物"
    );
    assert_eq!(
        hotbar_count(&rt, mcv_item::COBBLESTONE),
        cobble_before,
        "创造破坏不额外入栏"
    );
}

/// 玩家死亡（生存）：快捷栏全部生成 ItemDrop（40 tick 拾取延迟）并清栏。
#[test]
fn player_death_scatters_hotbar_as_drops() {
    let dir = tmp_world("death");
    let mut rt = GameRuntime::new_headless(20261010, dir, GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    rt.hotbar.slots[0] = mcv_item::ItemStack::new(mcv_item::COBBLESTONE, 5);
    rt.hotbar.slots[3] = mcv_item::ItemStack::new(mcv_item::IRON_SWORD_INDEX, 1);
    rt.player.pos = Vec3::new(8.0, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;

    rt.hurt_player(100.0, None);
    assert!(rt.dead, "致命伤置死亡态");
    assert!(
        rt.hotbar.slots.iter().all(|s| s.is_empty()),
        "死亡清空快捷栏"
    );
    // 掉落物实体 = 原栏非空格数，数量/延迟逐格对应。
    let drops: Vec<(u16, u8, u8)> = rt
        .mobs_app
        .world
        .read::<mcv_entity::ItemDrop>()
        .iter()
        .map(|(_, d)| (d.item, d.count, d.pickup_delay))
        .collect();
    assert!(
        drops.contains(&(mcv_item::COBBLESTONE, 5, mcv_entity::DEATH_PICKUP_DELAY)),
        "圆石 5 个带 40 tick 延迟在掉落清单里: {drops:?}"
    );
    assert!(
        drops.contains(&(
            mcv_item::IRON_SWORD_INDEX,
            1,
            mcv_entity::DEATH_PICKUP_DELAY
        )),
        "铁剑 1 把在掉落清单里: {drops:?}"
    );
    assert_eq!(drops.len(), 2, "非空格逐格掉落: {drops:?}");
}

/// 新档背包回归锁：全模式（生存/创造）快捷栏与主背包构造即空。
/// 原版依据：26.1 Inventory.java:56 items = NonNullList.withSize(36,
/// ItemStack.EMPTY)，:62-65 构造只挂 player/equipment 引用、从不发放
/// 物品——旧实现开局塞铁剑/创造 8 格调试方块均无源码依据（真机投诉）。
#[test]
fn fresh_world_inventory_is_empty_in_all_modes() {
    for mode in [GameMode::Survival, GameMode::Creative] {
        let dir = tmp_world(&format!("empty-inv-{mode:?}"));
        let rt = GameRuntime::new_headless(20261010, dir, mode);
        assert!(
            rt.hotbar.slots.iter().all(|s| s.is_empty()),
            "{mode:?} 新档快捷栏必须全空（无开局发放）: {:?}",
            rt.hotbar.slots
        );
        assert!(
            rt.hotbar.main.iter().all(|s| s.is_empty()),
            "{mode:?} 新档主背包必须全空: {:?}",
            rt.hotbar.main
        );
    }
}

/// 玩家死亡（生存）：主背包 27 格同样逐格掉落——原版 Player.die →
/// Inventory.dropAll 掉全 36 格；只掉快捷栏则 main 格死亡保留 = 变相
/// keepInventory。
#[test]
fn player_death_scatters_main_inventory_as_drops() {
    let dir = tmp_world("death-main");
    let mut rt = GameRuntime::new_headless(20261010, dir, GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    rt.hotbar.main[10] = mcv_item::ItemStack::new(mcv_item::COBBLESTONE, 12);
    rt.hotbar.slots[0] = mcv_item::ItemStack::new(mcv_item::COBBLESTONE, 3);
    rt.player.pos = Vec3::new(8.0, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;

    rt.hurt_player(100.0, None);
    assert!(rt.dead, "致命伤置死亡态");
    assert!(
        rt.hotbar.main.iter().all(|s| s.is_empty()),
        "死亡清空主背包"
    );
    let drops: Vec<(u16, u8, u8)> = rt
        .mobs_app
        .world
        .read::<mcv_entity::ItemDrop>()
        .iter()
        .map(|(_, d)| (d.item, d.count, d.pickup_delay))
        .collect();
    assert!(
        drops.contains(&(mcv_item::COBBLESTONE, 12, mcv_entity::DEATH_PICKUP_DELAY)),
        "main 格圆石 12 个带 40 tick 延迟在掉落清单里: {drops:?}"
    );
    assert!(
        drops.contains(&(mcv_item::COBBLESTONE, 3, mcv_entity::DEATH_PICKUP_DELAY)),
        "快捷栏格圆石 3 个也在掉落清单里: {drops:?}"
    );
    assert_eq!(drops.len(), 2, "hotbar+main 逐格掉落: {drops:?}");
}

/// 到达距离常数：生存 4.4 成 / 4.6 败，创造 4.6 成（4.5/5.0）。
/// 观测量 = mining_overlay 的准星 DDA —— 与 interact/start_mining 共用
/// 同一 `block_interaction_reach`（mcv_game::raycast::REACH + 创造 0.5）。
#[test]
fn reach_constants_survival_creative() {
    let overlay_at = |mode: GameMode, dz: f32| -> bool {
        let dir = tmp_world(&format!("reach-{:?}-{dz}", mode));
        let mut rt = GameRuntime::new_headless(777, dir, mode);
        wait_terrain(&mut rt, ChunkPos::new(0, 0));
        let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
        {
            let mut v = h.voxels.write().unwrap();
            for y in 0..70usize {
                v.set_flat(mcv_core::vidx(8, y as i32, 8), BlockId(1));
            }
            // 清空准星走廊（y=71，x=8，z=4..8），只留靶：悬浮石块 (8,71,3)。
            for z in 4..9usize {
                v.set_flat(mcv_core::vidx(8, 71, z), BlockId(0));
            }
            // 准星靶：悬浮石块 (8,71,3)，水平瞄准。眼睛 (8.5,71.62,8.5+dz)
            // → 近面 z=4 距离 = 4.5+dz（dz=+0.1 → 4.6；-0.1 → 4.4）。
            v.set_flat(mcv_core::vidx(8, 71, 3), BlockId(1));
        }
        h.advance_to(Stage::TerrainReady);
        rt.player.pos = Vec3::new(8.5, 70.0, 8.5 + dz);
        rt.player.vel = Vec3::ZERO;
        rt.player.yaw = 0.0; // 朝 -z；眼睛 y=71.62 ∈ 石块 [71,72)
        rt.player.pitch = 0.0;
        rt.mining_overlay().is_some()
    };
    assert!(overlay_at(GameMode::Survival, -0.1), "生存 4.4 < 4.5 命中");
    assert!(
        !overlay_at(GameMode::Survival, 0.1),
        "生存 4.6 > 4.5 不命中"
    );
    assert!(overlay_at(GameMode::Creative, 0.1), "创造 4.6 < 5.0 命中");
}

/// #150 刀 1：掉落物渲染快照通路——ECS ItemDrop → `item_drops()` 快照
/// （同源图标解析 item_icon + 所在格光照 + tick 相位），渲染端
/// `Scene.drops` 的数据供给行为锁。
#[test]
fn item_drops_snapshot_feeds_render_batch() {
    let dir = tmp_world("dropsnap");
    let mut rt = GameRuntime::new_headless(20261009, dir, GameMode::Survival);
    wait_terrain(&mut rt, ChunkPos::new(0, 0));
    build_platform(&mut rt);
    finish_loading(&mut rt);
    assert!(rt.item_drops().is_empty(), "无掉落物时渲染快照必须为空");
    let mut rng = || 20261012u32;
    // 玩家脚边一枚圆石（方块物品）+ 一枚木棍（材料物品），带拾取延迟防
    // 步进后被吸走入栏。
    mcv_entity::spawn_item_drop(
        &mut rt.mobs_app.world,
        Vec3::new(8.3, 70.0, 8.5),
        mcv_item::COBBLESTONE,
        3,
        mcv_entity::PICKUP_DELAY,
        &mut rng,
    );
    mcv_entity::spawn_item_drop(
        &mut rt.mobs_app.world,
        Vec3::new(7.5, 70.0, 8.5),
        mcv_item::STICK,
        1,
        mcv_entity::PICKUP_DELAY,
        &mut rng,
    );
    let snaps = rt.item_drops();
    assert_eq!(snaps.len(), 2, "两枚掉落物都应进快照");
    // 图标同源：圆石 → Block(9)，木棍 → Sprite("stick")（item_icon 分支）。
    assert!(
        snaps
            .iter()
            .any(|s| s.icon == mcv_render::gpu::HandItem::Block(9)),
        "方块物品图标 = 方块 id: {:?}",
        snaps.iter().map(|s| s.icon).collect::<Vec<_>>()
    );
    assert!(
        snaps
            .iter()
            .any(|s| s.icon == mcv_render::gpu::HandItem::Sprite("stick")),
        "材料物品图标 = 精灵名: {:?}",
        snaps.iter().map(|s| s.icon).collect::<Vec<_>>()
    );
    // 光照：快照携带的光照 = game 层 light_at(掉落物盒中心格)——直接对照
    // 区块 light 缓冲 nibble（低 4 位 block、高 4 位 sky，与 mesher 同约），
    // 锁住「取的是掉落物所在格」的供给关系（具体值依赖流式光照就绪态）。
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    for s in &snaps {
        let cell = mcv_core::BlockPos::new(
            s.pos[0].floor() as i32,
            (s.pos[1] + 0.125).floor() as i32,
            s.pos[2].floor() as i32,
        );
        let [lx, _, lz] = cell.local();
        let v = h.light.read().unwrap()[mcv_core::vidx(lx, cell.y, lz)];
        assert_eq!(
            s.light,
            [v & 0xF, v >> 4],
            "掉落物快照光照 = 所在格 nibble: {s:?}"
        );
    }
    // 步进后年龄以 tick 计（≥1 步 / STEPS_PER_TICK，含 partialTick 位）。
    for _ in 0..3 {
        rt.fixed_step(1.0 / 60.0);
    }
    let aged = rt.item_drops();
    assert_eq!(aged.len(), 2, "拾取延迟内不掉不出快照");
    for s in &aged {
        assert!(s.age_ticks > 0.0, "步进后 age_ticks 必须推进: {s:?}");
        assert!(s.pos[1] >= 70.0 - 0.01, "掉落物已落地平台顶（y≈70）：{s:?}");
    }
    // 视距裁剪（渲染距离 ×16 = 128）：远处的掉落物不进快照。
    mcv_entity::spawn_item_drop(
        &mut rt.mobs_app.world,
        Vec3::new(500.0, 70.0, 500.0),
        mcv_item::COBBLESTONE,
        1,
        mcv_entity::PICKUP_DELAY,
        &mut rng,
    );
    assert_eq!(rt.item_drops().len(), 2, "视距外掉落物被渲染收集器剔除");
}
