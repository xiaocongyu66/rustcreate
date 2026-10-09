//! 挖掘主循环集成测试（26.1 对账，派单 C1 + 惩罚接线 + Reach 接线）。
//!
//! 挖掘状态机 [`MineMachine`] 是纯逻辑结构（射线与速率由 GameRuntime 每 tick
//! 现算喂入），所以这里无需 GPU/世界生成即可完整驱动原版语义：
//! - 换目标 ABORT+START：`MultiPlayerGameMode.java:168-176`
//! - 每 tick continue：`Minecraft.java:1606-1628`（on_tick 门在 GameRuntime）
//! - 挖穿 destroyDelay=5 再开下一块：`MultiPlayerGameMode.java:226-228,282`
//! - 创造按住连秒破：`MultiPlayerGameMode.java:230-242`
//! - 松手 = ABORT 进度作废：`MultiPlayerGameMode.java:207-222`（0.7 只在
//!   `ServerPlayerGameMode.java:216-236` 复核 STOP 上报，此处不复活补判）
//! - 空中 ÷5 / 眼在水中 ×0.2：`Player.java:607-613`
//! - Reach 4.5 / 创造 +0.5：`Attributes.java:22-23`、`ServerPlayer.java:215-216`

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkPos};
use mcv_game::VoxelAccess;
use mcv_game::mining::{HeldTool, progress_per_tick as engine_progress_per_tick};
use mcv_game::raycast::{REACH, raycast};
use mcv_item::mining::{progress_per_tick, progress_per_tick_env};
use mcv_logic::game::{GameMode, MineHit, MineMachine, MineTick, block_interaction_reach};

const STONE: BlockId = BlockId(1);
const DIRT: BlockId = BlockId(2);

fn pos(x: i32, y: i32, z: i32) -> BlockPos {
    BlockPos::new(x, y, z)
}

fn hit(p: BlockPos, per: f32) -> Option<MineHit> {
    Some(MineHit {
        pos: p,
        per_tick: per,
    })
}

/// 换目标即重置：挖一半 → 准星移到另一方块 → 原目标进度作废、新目标从头开始。
/// （26.1 continueDestroyBlock 换目标落到 startDestroyBlock，先 ABORT 旧再
/// START 新，MultiPlayerGameMode.java:168-176,285-286。）
#[test]
fn target_switch_forfeits_old_progress() {
    let a = pos(10, 64, 0);
    let b = pos(10, 64, 5);
    let mut m = MineMachine::default();
    assert_eq!(m.start(a, 0.1), None);
    for _ in 0..3 {
        assert_eq!(m.continue_tick(hit(a, 0.1)), MineTick::Idle);
    }
    assert_eq!(m.pos, Some(a));
    assert!(m.progress > 0.3 && m.progress < 0.5, "累计应为 0.4 上下");

    // 准星移到 b：旧 0.4 作废，b 从 START 计第一 tick（服务端 (ticks+1) 公式）。
    assert_eq!(m.continue_tick(hit(b, 0.25)), MineTick::Idle);
    assert_eq!(m.pos, Some(b));
    assert_eq!(
        m.progress, 0.25,
        "换目标后进度必须清零重计，而不是 0.4+0.25"
    );
    assert_eq!(m.delay, 0);
}

/// 连挖：生存挖穿后 destroyDelay=5，冷却 5 tick 减尽、第 6 tick 自动对新目标
/// START，再 1 tick 挖穿。（26.1 MultiPlayerGameMode.java:226-228,274-282：
/// 挖穿 `>=1.0F` 破坏 + delay=5；冷却 tick 先减且完全不看目标。）
#[test]
fn break_then_five_tick_delay_then_auto_restart() {
    let a = pos(0, 64, 0);
    let b = pos(0, 64, 1);
    let mut m = MineMachine::default();
    assert_eq!(m.start(a, 0.5), None);
    // 恰好 0.5+0.5 = 1.0 也判破坏（原版阈值 >= 1.0F，client :274）。
    assert_eq!(m.continue_tick(hit(a, 0.5)), MineTick::Broken(a));
    assert_eq!(m.pos, None);
    assert_eq!(m.delay, 5);

    // 冷却 5 tick：即使准星已指新目标 b，也只做延迟递减，不推进、不破坏。
    for i in 0..5 {
        assert_eq!(
            m.continue_tick(hit(b, 0.5)),
            MineTick::Idle,
            "冷却第 {i} tick"
        );
    }
    assert_eq!(m.delay, 0);
    assert_eq!(m.pos, None);

    // 第 6 tick：对新目标自动 START（进度计第一 tick）。
    assert_eq!(m.continue_tick(hit(b, 0.5)), MineTick::Idle);
    assert_eq!(m.pos, Some(b));
    assert_eq!(m.progress, 0.5);
    // 再 1 tick 挖穿 b，按住继续连挖。
    assert_eq!(m.continue_tick(hit(b, 0.5)), MineTick::Broken(b));
    assert_eq!(m.delay, 5);
}

/// 生存徒手挖泥土：按硬度表 per-tick（1/0.5/30），按住 ≤20 tick 挖穿
/// （15 tick 即满 1.0）。表驱动：per 直接取 mcv_item 现算值。
#[test]
fn survival_hold_breaks_dirt_within_20_ticks() {
    let per = progress_per_tick(DIRT, None);
    assert!((per - 1.0 / 0.5 / 30.0).abs() < 1e-9, "dirt 徒手 per-tick");
    let a = pos(4, 5, 4);
    let mut m = MineMachine::default();
    assert_eq!(m.start(a, per), None);
    let mut broken_at = None;
    for tick in 1..=20 {
        if m.continue_tick(hit(a, per)) == MineTick::Broken(a) {
            broken_at = Some(tick + 1); // +1 = START 当 tick（服务端 ticksSpent+1）
            break;
        }
    }
    let t = broken_at.expect("泥土必须在 21 tick 内挖穿");
    assert!(t <= 21, "实际 {t} tick（含 START tick）");
}

/// 空中 ÷5 / 眼在水中 ×0.2 惩罚接线（26.1 Player.java:607-613）：同块同手，
/// 空中 per-tick 为地面 1/5、水下 ×0.2、双叠 1/25；并与引擎侧参考实现
/// mcv_game::mining::progress_per_tick 逐项一致（死代码接活的一致性锁）。
#[test]
fn air_and_submerged_penalty_ratios() {
    let ground = progress_per_tick_env(STONE, None, true, false);
    let air = progress_per_tick_env(STONE, None, false, false);
    let wet = progress_per_tick_env(STONE, None, true, true);
    let both = progress_per_tick_env(STONE, None, false, true);
    assert!(ground > 0.0);
    let rel = |a: f32, b: f32| ((a - b) / b).abs();
    assert!(
        rel(air, ground / 5.0) < 1e-6,
        "空中惩罚应为 ÷5，得 {air} vs {}",
        ground / 5.0
    );
    assert!(rel(wet, ground * 0.2) < 1e-6, "水下惩罚应为 ×0.2");
    assert!(rel(both, ground / 25.0) < 1e-6, "双惩罚叠乘 ÷25");
    assert_eq!(progress_per_tick(STONE, None), ground, "无惩罚版=地面版");

    // 引擎参考实现一致性（stone 空手：两表都判"需镐未持"→ 100 档、速度 1.0）。
    for (g, s) in [(true, false), (false, false), (true, true), (false, true)] {
        assert_eq!(
            progress_per_tick_env(STONE, None, g, s),
            engine_progress_per_tick(STONE, &HeldTool::BARE_HAND, g, s),
            "与 mcv_game::mining 参考实现必须逐位一致（on_ground={g}, submerged={s}）"
        );
    }
}

/// Reach 接线：生存 = mcv_game::raycast::REACH = 4.5（Attributes.java:22-23），
/// 创造 = +0.5 = 5.0（ServerPlayer.java:215-216 加法修饰），替换硬编码 5.0。
/// 并用射线边界验证数值真实生效：面距 5.0 的墙生存打不中、创造恰好打中。
#[test]
fn reach_is_vanilla_4_5_survival() {
    assert_eq!(REACH, 4.5, "引擎常数");
    assert_eq!(block_interaction_reach(GameMode::Survival), 4.5);
    assert_eq!(block_interaction_reach(GameMode::Hardcore), 4.5);
    assert_eq!(block_interaction_reach(GameMode::Creative), 5.0);

    struct Wall {
        plane_x: i32,
    }
    impl VoxelAccess for Wall {
        fn block(&self, p: BlockPos) -> BlockId {
            if p.x == self.plane_x && p.y == 0 {
                STONE
            } else {
                BlockId(0)
            }
        }
        fn light(&self, _p: BlockPos) -> u8 {
            15
        }
        fn chunk_loaded(&self, _c: ChunkPos) -> bool {
            true
        }
    }
    let eye = Vec3::new(0.0, 0.5, 0.5);
    let dir = Vec3::X;
    // 面距 5.0 m：生存 4.5 打不中（原版生存 4.5，旧硬编码 5.0 才打得中）。
    let wall5 = Wall { plane_x: 5 };
    assert_eq!(
        raycast(
            &wall5,
            eye,
            dir,
            block_interaction_reach(GameMode::Survival)
        ),
        None,
        "5.0 距离外方块生存必须挖不到"
    );
    // 创造 4.5+0.5：恰好命中面。
    assert_eq!(
        raycast(
            &wall5,
            eye,
            dir,
            block_interaction_reach(GameMode::Creative)
        )
        .map(|(p, _)| p),
        Some(pos(5, 0, 0)),
        "创造 +0.5 reach 恰好覆盖 5.0"
    );
    // 面距 4.0 m：生存可挖。
    let wall4 = Wall { plane_x: 4 };
    assert_eq!(
        raycast(
            &wall4,
            eye,
            dir,
            block_interaction_reach(GameMode::Survival)
        )
        .map(|(p, _)| p),
        Some(pos(4, 0, 0)),
        "4.5 距离内方块必须可挖"
    );
}

/// 松手补判收紧：进度未达 1 时主动松手 = ABORT，进度作废、绝不破坏
/// （26.1 stopDestroyBlock 发 ABORT_DESTROY_BLOCK，MultiPlayerGameMode.java:
/// 207-222；服务端 ABORT 只清状态，ServerPlayerGameMode.java:239-249。
/// 旧 0.7×(tick+1) 补判删除：0.7 仅复核 STOP 完成上报，:216-236）。
#[test]
fn release_midway_forfeits_progress_never_breaks() {
    let a = pos(3, 3, 3);
    let mut m = MineMachine::default();
    assert_eq!(m.start(a, 0.1), None);
    for _ in 0..8 {
        // 0.1×9 = 0.9 < 1.0：未达破坏阈值。
        assert_eq!(m.continue_tick(hit(a, 0.1)), MineTick::Idle);
    }
    assert!(m.progress < 1.0);
    // 主动松手（GameRuntime::on_left_release → abort）：清状态、无 Broken。
    m.abort();
    assert_eq!(m.pos, None);
    assert_eq!(m.progress, 0.0);
    assert_eq!(m.delay, 0);
}

/// 创造按住连秒破：按下首破置 delay=5（:157-167），按住每 tick 递减，
/// 减尽当 tick 秒破准星并重置 delay=5（:230-242）→ 每 6 tick 一块。
#[test]
fn creative_hold_breaks_every_sixth_tick() {
    let a = pos(2, 2, 2);
    // delay=5 = on_left_press 创造分支（按下首破置 destroyDelay）的等价效果。
    let mut m = MineMachine {
        delay: 5,
        ..Default::default()
    };
    for i in 0..5 {
        assert_eq!(m.creative_tick(Some(a)), None, "创造冷却第 {i} tick 不破坏");
    }
    assert_eq!(m.creative_tick(Some(a)), Some(a), "冷却减尽当 tick 秒破");
    assert_eq!(m.delay, 5, "秒破后重置 destroyDelay=5");
    assert_eq!(m.creative_tick(Some(a)), None);
    // 准星移空：冷却衰减，不误破坏。
    m.delay = 1;
    assert_eq!(m.creative_tick(None), None);
    assert_eq!(m.delay, 0);
}
