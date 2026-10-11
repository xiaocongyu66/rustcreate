//! 熔炉波测试（26.1 节奏对账，派单 #145）。
//!
//! 两层：
//! - 屏级语义（[`screen_semantics`]）：[`FurnaceScreen`] 纯状态机 =
//!   `AbstractFurnaceBlockEntity.serverTick` 直译（AFBE:139-207），无头可跑；
//! - 接线（GameRuntime 无头集成）：interact 开/关屏、fixed_step on_tick
//!   驱动、产物走 Hotbar::add 既有拾取通道、炉毁关屏归还。
//!
//! 引注约定：AFBE = AbstractFurnaceBlockEntity.java、AM = AbstractFurnaceMenu
//! .java（26.1 反编译树，行号见逐断言注释）。

use mcv_logic::ui::{FurnaceScreen, burn_duration};

fn stack(item: u16, n: u8) -> mcv_item::ItemStack {
    mcv_item::ItemStack::new(item, n)
}

// ==== 屏级语义（纯逻辑）====================================================

mod screen_semantics {
    use super::{FurnaceScreen, burn_duration, stack};
    use mcv_item::{
        BOW, CHEST, COAL, COBBLESTONE, CRAFTING_TABLE, IRON_INGOT, LOG, OAK_BUTTON, OAK_DOOR,
        OAK_FENCE_GATE, OAK_PRESSURE_PLATE, OAK_SAPLING, OAK_SLAB, OAK_STAIRS, OAK_TRAPDOOR,
        PLANKS, RAW_IRON, STICK, STONE_ITEM, WOODEN_PICKAXE_INDEX, WOODEN_SWORD_INDEX,
    };

    /// 1 煤 + 1 圆石 → 200 tick 出 1 石头；煤在点亮帧一次抽走、余程不再
    /// 出现（「coal 消失节奏逐值」）。物料经槽点击入屏（setItem 面置
    /// cook_total，AFBE:292-302——直接写字段不经过菜单面，非原版路径）。
    #[test]
    fn smelts_cobblestone_to_stone_in_200_ticks() {
        let mut f = FurnaceScreen::new();
        f.cursor = stack(COBBLESTONE, 1);
        f.click_input(true);
        f.cursor = stack(COAL, 1);
        f.click_fuel(true);
        // tick 1：lit=0 → isLit=false，但有燃料+输入 → 当帧抽煤点亮
        //（AFBE:164-173），cook 前进 1。
        f.tick(1);
        assert!(
            f.fuel.is_empty(),
            "煤在点亮帧整颗抽走（consumeFuel AFBE:209-216）"
        );
        assert_eq!(
            f.lit_time, 1600,
            "litTotalTime = coal burnDuration（AFBE:165-167）"
        );
        assert_eq!(f.lit_total, 1600);
        assert_eq!(f.cook_time, 1);
        assert!(f.is_lit());
        assert!(
            (f.lit_progress() - 1.0).abs() < 1e-6,
            "满火焰（AM:160-166）"
        );
        // tick 100：cook=100、lit=1600-99=1501，进度半格。
        f.tick(99);
        assert_eq!(f.cook_time, 100);
        assert_eq!(f.lit_time, 1501);
        assert!(
            (f.cook_progress() - 0.5).abs() < 1e-6,
            "箭头半程（AM:152-158）"
        );
        // tick 200：烧成一件。
        f.tick(100);
        assert_eq!(
            (f.result.item, f.result.count),
            (STONE_ITEM, 1),
            "圆石→石头"
        );
        assert!(f.input.is_empty(), "输入烧尽（burn AFBE:233-246）");
        assert_eq!(f.cook_time, 0);
        assert_eq!(f.cook_total, 200, "烧完重置为配方时长（AFBE:179）");
        assert_eq!(f.lit_time, 1401, "1600−199：点亮帧不递减、其后逐帧递减");
        // 余煤不存在：无输入无燃料，结果不再增长。
        f.tick(200);
        assert_eq!((f.result.item, f.result.count), (STONE_ITEM, 1));
        assert!(f.fuel.is_empty());
    }

    /// 2 件连烧：litTime 复用、不二次抽煤；1 coal = 1600 tick 恰 8 件
    ///（coal=1600，FuelValues.java:47；200 tick/件，AFBE:54）。
    #[test]
    fn one_coal_smelts_eight_items_without_relight() {
        let mut f = FurnaceScreen::new();
        f.cursor = stack(COBBLESTONE, 8);
        f.click_input(true);
        f.cursor = stack(COAL, 1);
        f.click_fuel(true);
        f.tick(1);
        assert!(f.fuel.is_empty(), "coal 只在第一帧抽一次");
        f.tick(399); // 共 400
        assert_eq!(
            f.result.count, 2,
            "结果槽同品并堆连烧（canBurn AFBE:218-231）"
        );
        assert_eq!(f.cook_time, 0);
        f.tick(1200); // 共 1600
        assert_eq!(f.result.count, 8, "1 coal = 8 件（1600/200）");
        assert!(f.input.is_empty());
        assert_eq!(f.lit_time, 1, "第 1600 帧递减后恰剩 1");
        f.tick(1);
        assert_eq!(f.lit_time, 0, "lit 耗尽");
        assert_eq!(f.result.count, 8, "无燃料不续烧");
    }

    /// 燃料耗尽：isLit 熄、cook 按 BURN_COOL_SPEED=2 退坡（AFBE:55,194-196）；
    /// 续料当帧再抽燃料点亮、cook 从退坡值继续。
    #[test]
    fn fuel_out_cools_cook_and_refuel_resumes() {
        let mut f = FurnaceScreen::new();
        f.cursor = stack(COBBLESTONE, 2);
        f.click_input(true);
        f.cursor = stack(PLANKS, 1);
        f.click_fuel(true); // planks = 300（FuelValues.java:51）
        f.tick(200);
        assert_eq!(f.result.count, 1);
        assert_eq!(f.lit_time, 101, "300−199");
        f.tick(100);
        assert_eq!(f.lit_time, 1);
        assert_eq!(f.cook_time, 100, "第二件烧到一半");
        // tick 301：lit 1→0，无燃料 → cook 退坡 −2。
        f.tick(1);
        assert_eq!(f.lit_time, 0);
        assert!(!f.is_lit());
        assert_eq!(f.cook_time, 98, "BURN_COOL_SPEED=2（AFBE:194-196）");
        f.tick(4);
        assert_eq!(f.cook_time, 90, "持续退坡逐值 98→96→94→92→90");
        // 续料：当帧再抽（AFBE:164-173），cook 从 90 续。
        f.fuel = stack(PLANKS, 1);
        f.tick(1);
        assert_eq!(f.lit_time, 300);
        assert_eq!(f.lit_total, 300);
        assert!(f.fuel.is_empty());
        assert_eq!(f.cook_time, 91);
        f.tick(109); // 91+109=200 → 第二件完成
        assert_eq!(f.result.count, 2);
        assert!(f.input.is_empty());
    }

    /// 输入换品 cook 归零（setItem AFBE:292-302）；同品补量不重置
    /// （isSameItemSameComponents，AFBE:294）。
    #[test]
    fn input_swap_resets_cook_but_topping_same_item_does_not() {
        let mut f = FurnaceScreen::new();
        f.fuel = stack(COAL, 1);
        f.cursor = stack(COBBLESTONE, 3);
        f.click_input(true);
        assert!(f.cursor.is_empty());
        assert_eq!(f.input.count, 3);
        assert_eq!(f.cook_total, 200, "setItem 置 getTotalCookTime（AFBE:298）");
        assert_eq!(f.cook_time, 0);
        f.tick(100);
        assert_eq!(f.cook_time, 100);
        // 同品补量：右键 +1 → cook 保持。
        f.cursor = stack(COBBLESTONE, 1);
        f.click_input(false);
        assert_eq!(f.input.count, 4);
        assert_eq!(f.cook_time, 100, "同品补量不重置（AFBE:294）");
        // 换品：生铁换入 → cook 归零、cook_total 重估。
        f.cursor = stack(RAW_IRON, 2);
        f.click_input(true);
        assert_eq!(f.input.item, RAW_IRON);
        assert_eq!(f.cook_time, 0, "换品 cook 归零（AFBE:297-301）");
        assert_eq!(f.cook_total, 200);
        f.tick(200);
        assert_eq!(
            (f.result.item, f.result.count),
            (IRON_INGOT, 1),
            "生铁→铁锭"
        );
        assert_eq!(f.input.count, 1);
    }

    /// 结果未取：满槽/异品堵住下一件（canBurn AFBE:218-231）且 cook 清零
    /// （AFBE:187-189）；canBurn 门在抽燃料**之前**（AFBE:163-173）——
    /// 未点亮 + 堵槽 = 不抽煤不点亮（原版语义）；已点亮时 lit 照递减
    /// （AFBE:143-150）。取出后恢复。
    #[test]
    fn result_slot_full_or_foreign_blocks_next_and_resets_cook() {
        let mut f = FurnaceScreen::new();
        f.cursor = stack(COBBLESTONE, 2);
        f.click_input(true);
        f.cursor = stack(COAL, 1);
        f.click_fuel(true);
        f.result = stack(STONE_ITEM, 64);
        f.tick(200);
        assert_eq!(f.result.count, 64, "满槽不烧下一件");
        assert_eq!(f.input.count, 2, "输入不消耗");
        assert_eq!(f.cook_time, 0, "canBurn 假分支 cook 清零（AFBE:187-189）");
        assert_eq!(f.fuel.count, 1, "未点亮 + 堵槽：不抽煤（AFBE:163 门序）");
        assert_eq!(f.lit_time, 0, "炉未点亮");
        // 取出整堆恢复。
        assert_eq!(
            f.take_result().map(|s| (s.item, s.count)),
            Some((STONE_ITEM, 64))
        );
        assert!(f.result.is_empty());
        f.tick(200);
        assert_eq!((f.result.item, f.result.count), (STONE_ITEM, 1));
        assert_eq!(f.input.count, 1, "第二件续烧中");
        // 异品堵槽：结果槽是铁锭时圆石不再烧（输入 = 剩余 1 + 右键再放 1）。
        f.result = stack(IRON_INGOT, 1);
        f.cursor = stack(COBBLESTONE, 1);
        f.click_input(false);
        assert_eq!(f.input.count, 2);
        f.tick(1);
        f.tick(300);
        assert_eq!(f.result.count, 1, "异品结果不并堆");
        assert_eq!(f.input.count, 2, "堵槽不消耗输入");
    }

    /// 燃料槽只认燃料（FurnaceFuelSlot.mayPlace = isFuel，
    /// FurnaceFuelSlot.java:17-19）；结果槽只取不放
    /// （FurnaceResultSlot.java:24-27），取出即计一件。
    #[test]
    fn fuel_slot_gate_and_result_take() {
        let mut f = FurnaceScreen::new();
        f.cursor = stack(COBBLESTONE, 5);
        f.click_fuel(true);
        assert!(f.fuel.is_empty(), "非燃料不得入燃料槽");
        assert_eq!(f.cursor.count, 5, "被拒点击不吞光标");
        f.cursor = stack(COAL, 3);
        f.click_fuel(true);
        assert_eq!(f.fuel.count, 3);
        assert!(f.cursor.is_empty());
        // 结果槽取出（整堆直出，不经光标）。
        f.result = stack(STONE_ITEM, 2);
        assert_eq!(
            f.take_result().map(|s| (s.item, s.count)),
            Some((STONE_ITEM, 2))
        );
        assert!(f.result.is_empty());
        assert!(f.take_result().is_none(), "空结果无物可取");
        // 光标异物未被结果槽路径触碰。
        f.cursor = stack(COBBLESTONE, 1);
        assert_eq!(f.cursor.count, 1);
    }

    /// 燃烧时长表抽验（26.1 FuelValues.vanillaBurnTimes，baseUnit=200，
    /// FuelValues.java:38-42；逐项引注见 [`mcv_logic::ui::burn_duration`]）。
    #[test]
    fn burn_duration_table_matches_26_1() {
        assert_eq!(burn_duration(COAL), 1600); // :47 baseUnit×8
        assert_eq!(burn_duration(PLANKS), 300); // :51 ×3/2
        assert_eq!(burn_duration(LOG), 300); // :49 LOGS tag
        assert_eq!(burn_duration(STICK), 100); // :87 ×1/2
        assert_eq!(burn_duration(BOW), 300); // :72
        assert_eq!(burn_duration(WOODEN_PICKAXE_INDEX), 200); // :82
        assert_eq!(burn_duration(WOODEN_SWORD_INDEX), 200); // :78
        assert_eq!(burn_duration(OAK_SLAB), 150); // :55 ×3/4
        assert_eq!(burn_duration(OAK_STAIRS), 300); // :53
        assert_eq!(burn_duration(OAK_TRAPDOOR), 300); // :57
        assert_eq!(burn_duration(OAK_DOOR), 200); // :83
        assert_eq!(burn_duration(OAK_FENCE_GATE), 300); // :61
        assert_eq!(burn_duration(OAK_BUTTON), 100); // :86
        assert_eq!(burn_duration(OAK_PRESSURE_PLATE), 300); // :58
        assert_eq!(burn_duration(OAK_SAPLING), 100); // :88 SAPLINGS
        assert_eq!(burn_duration(CRAFTING_TABLE), 300); // :69
        assert_eq!(burn_duration(CHEST), 300); // :67
        // 非燃料 = 0。
        assert_eq!(burn_duration(COBBLESTONE), 0);
        assert_eq!(burn_duration(STONE_ITEM), 0);
        assert_eq!(burn_duration(RAW_IRON), 0);
    }

    /// 关屏守恒：光标 + 三槽全部交还，一件不少（审计 #52）。
    #[test]
    fn close_to_world_returns_every_slot() {
        let mut f = FurnaceScreen::new();
        f.input = stack(COBBLESTONE, 7);
        f.fuel = stack(COAL, 2);
        f.result = stack(STONE_ITEM, 1);
        f.cursor = stack(STICK, 4);
        let (cursor, drops) = f.close_to_world();
        assert_eq!(cursor.map(|s| (s.item, s.count)), Some((STICK, 4)));
        assert_eq!(drops.len(), 3);
        assert_eq!(drops.iter().map(|s| s.count as u32).sum::<u32>(), 10);
        assert!(drops.iter().any(|s| s.item == COBBLESTONE && s.count == 7));
        // 全空炉：光标 None、零掉落。
        let (c, d) = FurnaceScreen::new().close_to_world();
        assert!(c.is_none() && d.is_empty());
    }
}

// ==== 接线（GameRuntime 无头集成）==========================================

use std::path::PathBuf;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, ChunkStatus, Stage};
use mcv_logic::game::{GameMode, GamePhase, GameRuntime};

/// 唯一临时存档目录（测试并行安全；runtime.rs 同款）。
fn tmp_world(tag: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-furnace-{}-{:x}", tag, h.finish()))
}

/// 流式加载直到出生区块地形就绪（runtime.rs 同款）。
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

/// 加载态收尾（runtime.rs 同款：49 块顶到 Uploaded + Full，步进到游玩态）。
fn finish_loading(rt: &mut GameRuntime) {
    rt.player.pos = Vec3::new(8.3, 70.0, 8.5);
    rt.player.vel = Vec3::ZERO;
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            let pos = ChunkPos::new(dx, dz);
            rt.chunks
                .entry(pos)
                .or_insert_with(|| std::sync::Arc::new(ChunkHandle::new(pos)))
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

/// 测试场地（runtime.rs 同款）：压实三列石头到 y=69，工作区 3×3 清空。
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

/// 眼睛对准 (7,71,8)（runtime.rs 同款；z 取 8.499 避开 atan2 负零分支）。
fn set_aim_at_block(rt: &mut GameRuntime) {
    let eye = rt.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
    let d = Vec3::new(7.5, 71.5, 8.499) - eye;
    rt.player.yaw = d.x.atan2(-d.z);
    rt.player.pitch = (d.y / d.length()).asin();
}

fn set_block(rt: &mut GameRuntime, x: usize, y: usize, z: usize, id: BlockId) {
    let h = rt.chunks.get(&ChunkPos::new(0, 0)).unwrap().clone();
    h.voxels.write().unwrap()[mcv_core::vidx(x, y as i32, z)] = id;
    h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
}

fn hotbar_count(rt: &GameRuntime, item: u16) -> u32 {
    rt.hotbar
        .slots
        .iter()
        .filter(|s| s.item == item)
        .map(|s| s.count as u32)
        .sum()
}

/// 场地 + 眼平高熔炉 + 准星就位。
fn furnace_stage(rt: &mut GameRuntime) {
    wait_terrain(rt, ChunkPos::new(0, 0));
    build_platform(rt);
    finish_loading(rt);
    let furnace_id = mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == "furnace")
        .expect("blocks_gen 注册 furnace") as u16;
    set_block(rt, 7, 71, 8, BlockId(furnace_id));
    set_aim_at_block(rt);
}

/// interact 命中 furnace → 开/关熔炉屏（派单分派点）。
#[test]
fn interact_on_furnace_toggles_screen() {
    let dir = tmp_world("toggle");
    let mut rt = GameRuntime::new_headless(20261012, dir, GameMode::Survival);
    furnace_stage(&mut rt);
    rt.interact(true);
    let (pos, f) = rt.furnace.as_ref().expect("对炉右键开熔炉屏");
    assert_eq!(*pos, mcv_core::BlockPos::new(7, 71, 8));
    assert!(f.input.is_empty() && f.fuel.is_empty() && f.result.is_empty());
    rt.interact(true);
    assert!(rt.furnace.is_none(), "再右键关闭");
    // 关了再开：全新会话。
    rt.interact(true);
    assert!(rt.furnace.is_some());
}

/// 屏激活时游戏逻辑继续 tick（原版单人服务端不停），fixed_step on_tick
/// 驱动烧炼：600 步 = 200 tick 出 1 石头。
#[test]
fn furnace_session_smelts_while_game_ticks() {
    let dir = tmp_world("smelt-rt");
    let mut rt = GameRuntime::new_headless(20261012, dir, GameMode::Survival);
    furnace_stage(&mut rt);
    rt.interact(true);
    {
        let (_, f) = rt.furnace.as_mut().unwrap();
        // 物料经槽点击入屏（setItem 面置 cook_total，AFBE:292-302）。
        f.cursor = stack(COBBLESTONE, 1);
        f.click_input(true);
        f.cursor = stack(COAL, 1);
        f.click_fuel(true);
    }
    for _ in 0..600 {
        rt.fixed_step(1.0 / 60.0); // 3 步/tick → 600 步 = 200 tick
    }
    let (_, f) = rt.furnace.as_ref().unwrap();
    assert_eq!(
        (f.result.item, f.result.count),
        (STONE_ITEM, 1),
        "屏激活炉照烧（服务端不停）"
    );
}

/// 烧成产物入背包走既有拾取通道（Hotbar::add，give 路径唯一）。
#[test]
fn furnace_result_goes_to_hotbar_via_pickup_channel() {
    let dir = tmp_world("collect");
    let mut rt = GameRuntime::new_headless(20261012, dir, GameMode::Survival);
    furnace_stage(&mut rt);
    rt.interact(true);
    {
        let (_, f) = rt.furnace.as_mut().unwrap();
        f.result = stack(STONE_ITEM, 3);
    }
    rt.player.sel_slot = 0;
    assert!(rt.furnace_take_result());
    assert_eq!(hotbar_count(&rt, STONE_ITEM), 3, "产物入栏");
    assert!(
        rt.furnace.as_ref().unwrap().1.result.is_empty(),
        "结果槽取空"
    );
    assert!(!rt.furnace_take_result(), "空结果不可再取");
}

/// 关屏归还：光标先背包（ACM:588-596）、容器槽走世界掉落贴身拾取入栏
/// （CM:96-99 通道；本仓炉内容不可持久，债 HANDOFF-FURNACE.md）。
#[test]
fn closing_furnace_returns_slots() {
    let dir = tmp_world("close");
    let mut rt = GameRuntime::new_headless(20261012, dir, GameMode::Survival);
    furnace_stage(&mut rt);
    rt.interact(true);
    {
        let (_, f) = rt.furnace.as_mut().unwrap();
        f.input = stack(COBBLESTONE, 5);
        f.cursor = stack(COAL, 1);
    }
    rt.player.sel_slot = 0;
    rt.interact(true); // 关屏
    assert!(rt.furnace.is_none());
    assert_eq!(hotbar_count(&rt, COAL), 1, "光标先背包");
    for _ in 0..120 {
        rt.fixed_step(1.0 / 60.0); // PICKUP_DELAY 10 tick = 30 步后拾取
    }
    assert_eq!(hotbar_count(&rt, COBBLESTONE), 5, "容器槽落地拾取入栏");
}

/// 烧炉中的炉方块被挖 → 会话关屏归还（vanilla dropContents 对位，禁吞物）。
#[test]
fn breaking_open_furnace_returns_its_session() {
    let dir = tmp_world("break");
    let mut rt = GameRuntime::new_headless(20261012, dir, GameMode::Creative);
    furnace_stage(&mut rt);
    rt.interact(true);
    rt.furnace.as_mut().unwrap().1.input = stack(COBBLESTONE, 3);
    rt.interact(false); // 创造秒破
    assert!(rt.furnace.is_none(), "炉毁关屏归还");
    for _ in 0..120 {
        rt.fixed_step(1.0 / 60.0);
    }
    assert_eq!(hotbar_count(&rt, COBBLESTONE), 3, "会话内容不丢失");
}

/// 手持食物对炉右键：方块 use 优先 → 开炉不进食（26.1 startUseItem）。
#[test]
fn food_right_click_on_furnace_opens_instead_of_eating() {
    let dir = tmp_world("food");
    let mut rt = GameRuntime::new_headless(20261012, dir, GameMode::Survival);
    furnace_stage(&mut rt);
    rt.hotbar.slots[0] = stack(mcv_item::APPLE, 1);
    rt.player.sel_slot = 0;
    rt.player.hunger = 10.0; // needsFood：无方块 use 门时会启动进食
    rt.interact(true);
    assert!(rt.furnace.is_some(), "开炉优先于进食");
    assert_eq!(rt.hotbar.slots[0].count, 1, "苹果未消耗");
}
