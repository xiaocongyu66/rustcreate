//! 挖掘公式表驱动测试。MC 26.1 原式（BlockBehaviour#getDestroyProgress）：
//! `progress_per_tick = speed / hardness / (正确工具 ? 30 : 100)`，≥1 破坏；
//! 空中 /5、水下 ×0.2。推导见 /root/mc-ref/NOTES-physics.md。

use mcv_core::{BlockId, BlockPos};
use mcv_game::consts::FIXED_DT;
use mcv_game::mining::{
    DigState, HeldTool, MC_TICK, MODIFIER_CORRECT, MODIFIER_INCORRECT, TOOL_SPEED_WOOD,
    break_seconds, hardness, has_correct_tool_for_drops, progress_per_tick, requires_correct_tool,
};

// 与 mcv_core::BLOCKS 注册顺序一致（硬度已由 BLOCKS 校准代理按 26.1 对齐，
// 因此期望值一律由硬度现值反推，硬度再调也不会脆断）。
const STONE: u16 = 1;
const DIRT: u16 = 2;
const LOG: u16 = 6;
const PLANKS: u16 = 8;
const COBBLE: u16 = 9;
const BEDROCK: u16 = 10;
const FLOWER_RED: u16 = 12;

fn bid(id: u16) -> BlockId {
    BlockId(id)
}

/// 按公式独立重算期望时长（秒）：h × modifier × MC_TICK / speed。
fn expect_seconds(id: u16, speed: f32, correct_for_drops: bool) -> f32 {
    let h = hardness(bid(id));
    if h.is_infinite() {
        return f32::INFINITY;
    }
    if h <= 0.0 {
        return 0.0;
    }
    let modifier = if !requires_correct_tool(bid(id)) || correct_for_drops {
        MODIFIER_CORRECT
    } else {
        MODIFIER_INCORRECT
    };
    h * modifier * MC_TICK / speed
}

/// 相对误差判等（f32 累积）；无穷大只与同符号无穷大相等，0 与 0 相等。
fn close(a: f32, b: f32) -> bool {
    if a.is_infinite() || b.is_infinite() || a == 0.0 || b == 0.0 {
        return a == b;
    }
    (a - b).abs() <= 1e-3 * a.abs().max(b.abs())
}

/// 1. 表驱动：各方块 × {空手, 木镐} 的耗时与公式值一致。
///    用 mcv_core 现硬度：stone 1.5 → 空手 7.5 s / 木镐 1.125 s；
///    dirt 0.5 → 空手 0.75 s；log/planks 2.0 → 空手 3.0 s。
#[test]
fn break_time_matches_formula_table() {
    let ids = [STONE, DIRT, LOG, PLANKS, COBBLE, BEDROCK, FLOWER_RED];
    let ground = HeldTool::BARE_HAND;
    for id in ids {
        let got = break_seconds(bid(id), &HeldTool::BARE_HAND, true, false);
        let want = expect_seconds(id, 1.0, ground.correct_for_drops);
        assert!(close(got, want), "空手 {id}: {got} != {want}");
        let wood = HeldTool {
            speed: TOOL_SPEED_WOOD,
            correct_for_drops: true,
        };
        let got = break_seconds(bid(id), &wood, true, false);
        let want = expect_seconds(id, TOOL_SPEED_WOOD, true);
        assert!(close(got, want), "木镐 {id}: {got} != {want}");
    }
    // 锚定 MC 已知值（硬度现值 1.5/0.5 下的精确秒数）：
    assert!(close(
        break_seconds(bid(STONE), &HeldTool::BARE_HAND, true, false),
        7.5
    ));
    assert!(close(
        break_seconds(
            bid(STONE),
            &HeldTool {
                speed: TOOL_SPEED_WOOD,
                correct_for_drops: true
            },
            true,
            false
        ),
        1.125
    ));
    assert!(close(
        break_seconds(bid(DIRT), &HeldTool::BARE_HAND, true, false),
        0.75
    ));
}

/// 2. 空手耗时排序：stone（需镐 → 100 档）> 木（log=planks，30 档）> dirt。
#[test]
fn bare_hand_time_order_stone_wood_dirt() {
    let t = |id| break_seconds(bid(id), &HeldTool::BARE_HAND, true, false);
    let (stone, planks, log, dirt) = (t(STONE), t(PLANKS), t(LOG), t(DIRT));
    assert!(stone > planks, "stone({stone}) 应慢于木板({planks})");
    assert!(close(planks, log), "木板与原木硬度相同：{planks} vs {log}");
    assert!(planks > dirt, "木板({planks}) 应慢于 dirt({dirt})");
    assert!(dirt > 0.0);
}

/// 3. 空手惩罚与工具倍率：空手/木镐 = (100/30) × (2.0/1.0) = 20/3。
///    正确工具还须走 30 档（MC hasCorrectToolForDrops 语义）。
#[test]
fn bare_hand_penalty_and_tool_multiplier() {
    let t_stone_bare = break_seconds(bid(STONE), &HeldTool::BARE_HAND, true, false);
    let t_stone_wood = break_seconds(
        bid(STONE),
        &HeldTool {
            speed: TOOL_SPEED_WOOD,
            correct_for_drops: true,
        },
        true,
        false,
    );
    let ratio = t_stone_bare / t_stone_wood;
    assert!(
        close(
            ratio,
            (MODIFIER_INCORRECT / MODIFIER_CORRECT) * TOOL_SPEED_WOOD
        ),
        "比值 {ratio}"
    );
    // 速度对但工具不对（如木锄挖石）：只吃倍率、不吃 30 档。
    let t_wrong = break_seconds(
        bid(STONE),
        &HeldTool {
            speed: TOOL_SPEED_WOOD,
            correct_for_drops: false,
        },
        true,
        false,
    );
    assert!(close(
        t_wrong,
        hardness(bid(STONE)) * MODIFIER_INCORRECT * MC_TICK / TOOL_SPEED_WOOD
    ));
}

/// 4. 空中 /5 与水下 ×0.2（= /5）惩罚。
#[test]
fn air_and_water_penalties_table() {
    let bare = HeldTool::BARE_HAND;
    let t = |on_ground: bool, submerged: bool, id: u16| {
        break_seconds(bid(id), &bare, on_ground, submerged)
    };
    assert!(close(t(false, false, STONE), 5.0 * t(true, false, STONE)));
    assert!(close(t(true, true, DIRT), 5.0 * t(true, false, DIRT)));
    // 两个惩罚可叠乘（空中+水下 = /25）。
    assert!(close(t(false, true, DIRT), 25.0 * t(true, false, DIRT)));
}

/// 5. 可挖掘性：基岩（INFINITY）永不破坏；硬度 0 的花瞬间破坏。
#[test]
fn bedrock_unbreakable_and_flower_instant() {
    assert_eq!(
        break_seconds(bid(BEDROCK), &HeldTool::BARE_HAND, true, false),
        f32::INFINITY
    );
    let mut dig = DigState::default();
    let pos = BlockPos::new(0, 0, 0);
    for _ in 0..1000 {
        assert!(!dig.advance(
            pos,
            bid(BEDROCK),
            &HeldTool::BARE_HAND,
            true,
            false,
            FIXED_DT
        ));
    }
    assert_eq!(dig.stage(), 0);
    assert!(dig.progress == 0.0);

    // 花：第一个非零 dt 即破坏。
    let mut dig = DigState::default();
    assert!(dig.advance(
        pos,
        bid(FLOWER_RED),
        &HeldTool::BARE_HAND,
        true,
        false,
        FIXED_DT
    ));
    assert_eq!(dig.target, None);
    assert_eq!(
        break_seconds(bid(FLOWER_RED), &HeldTool::BARE_HAND, true, false),
        0.0
    );
}

/// 6. DigState 逐帧累积：stone 空手 ≈ 7.5 s → 约 450 个 1/60 步后破坏
///    （容差 ±3 步，避开 f32 舍入翻转），且前期不提前破坏、破坏后状态自动清空。
#[test]
fn dig_state_accumulates_to_break() {
    let total = break_seconds(bid(STONE), &HeldTool::BARE_HAND, true, false);
    assert!(close(total, 7.5), "stone 空手应 ≈ 7.5 s，实得 {total}");
    let n_approx = (total / FIXED_DT) as usize; // ≈450
    let pos = BlockPos::new(4, 3, 2);
    let mut dig = DigState::default();
    let mut broken_at = None;
    for i in 0..n_approx + 5 {
        if dig.advance(pos, bid(STONE), &HeldTool::BARE_HAND, true, false, FIXED_DT) {
            broken_at = Some(i + 1);
            break;
        }
        // 阈值前不得提前破坏
        assert!(dig.progress < 1.0);
    }
    let at = broken_at.expect("应在 n_approx+5 步内破坏");
    assert!(
        at + 2 >= n_approx && at <= n_approx + 3,
        "破坏步数 {at}，期望 ≈{n_approx}"
    );
    assert_eq!(dig.target, None);
    assert_eq!(dig.progress, 0.0);
}

/// 7. 换目标即重置进度（MC：换方块从零开始）。
#[test]
fn dig_state_resets_on_target_change() {
    let a = BlockPos::new(0, 1, 0);
    let b = BlockPos::new(1, 1, 0);
    let mut dig = DigState::default();
    for _ in 0..50 {
        dig.advance(a, bid(STONE), &HeldTool::BARE_HAND, true, false, FIXED_DT);
    }
    // 50 步 ≈ 50/450 ≈ 0.111，未破坏。
    assert!(dig.progress > 0.05 && dig.progress < 1.0);
    dig.advance(b, bid(STONE), &HeldTool::BARE_HAND, true, false, FIXED_DT);
    assert!(
        dig.progress < 0.005 && dig.target == Some(b),
        "换目标后进度应重置：{}",
        dig.progress
    );
    dig.cancel();
    assert_eq!(dig.target, None);
    assert_eq!(dig.progress, 0.0);
}

/// 8. 裂纹阶段表：MC `(int)(progress × 10)`，0..=9。
#[test]
fn crack_stage_table() {
    let mut dig = DigState::default();
    for (progress, stage) in [
        (0.0f32, 0u8),
        (0.099, 0),
        (0.1, 1),
        (0.55, 5),
        (0.999, 9),
        (1.0, 9), // ≥1 由 advance 触发破坏，stage 端钳到 9
    ] {
        dig.progress = progress;
        assert_eq!(dig.stage(), stage, "progress={progress}");
    }
}

/// 9. hasCorrectToolForDrops 判定表：石头/圆石需镐，其余不需（空手也 30 档）。
#[test]
fn correct_tool_requirement_table() {
    let requires = [STONE, COBBLE];
    let free = [DIRT, LOG, PLANKS, FLOWER_RED];
    for id in requires {
        assert!(requires_correct_tool(bid(id)), "{id} 应需要正确工具");
        assert!(!has_correct_tool_for_drops(bid(id), &HeldTool::BARE_HAND));
        assert!(has_correct_tool_for_drops(
            bid(id),
            &HeldTool {
                speed: TOOL_SPEED_WOOD,
                correct_for_drops: true
            }
        ));
    }
    for id in free {
        assert!(!requires_correct_tool(bid(id)));
        assert!(has_correct_tool_for_drops(bid(id), &HeldTool::BARE_HAND));
    }
    // 进度分母档位直查：空手挖石 = speed/(1.5·100)，空手挖土 = speed/(0.5·30)。
    let p_stone = progress_per_tick(bid(STONE), &HeldTool::BARE_HAND, true, false);
    let p_dirt = progress_per_tick(bid(DIRT), &HeldTool::BARE_HAND, true, false);
    assert!(close(p_stone, 1.0 / (1.5 * 100.0)));
    assert!(close(p_dirt, 1.0 / (0.5 * 30.0)));
}
