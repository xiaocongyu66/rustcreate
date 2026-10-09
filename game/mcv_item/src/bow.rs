//! 弓：蓄力曲线与蓄/放状态机（26.1 `BowItem`，26.1 反编译实读）。
//!
//! 源码依据（类名:行号）：
//! - **蓄力曲线**：`BowItem.getPowerForTime`（BowItem.java:74-81）
//!   `p = t/20; p = (p² + 2p)/3; clamp ≤ 1`——**不是** (t/20)²（派单口误）：
//!   半蓄 10 tick → (0.25+1)/3 ≈ 0.417（平方曲线只有 0.25）；20 tick 满蓄 1.0。
//! - **放箭**：`BowItem.releaseUsing`（BowItem.java:28-43）——
//!   `pow < 0.1` 取消（:38）；初速 = `pow × 3.0`（:41，shoot 的 velocity 参数）；
//!   `pow == 1.0` 置暴击 flag（:41 第 5 参，AbstractArrow.java:434-437 结算）。
//! - **最大持有时长**：`getUseDuration`（BowItem.java:87-89）= 72000 tick。
//! - **弹药**：`player.getProjectile` 空则不放（BowItem.java:30-34）——
//!   调用方查快捷栏箭数。
//!
//! 状态机 [`step_charge`] 把「按住 placing 蓄力、松开 release」折叠成纯函数，
//! game.rs 只消费 [`BowRelease`]。

/// `BowItem.java:38` — pow < 0.1 取消不放。
pub const MIN_POWER: f32 = 0.1;

/// `BowItem.java:87-89` — getUseDuration = 72000 tick。
pub const MAX_HOLD_TICKS: u32 = 72_000;

/// 一次放箭的产出：`power` = 蓄力系数 0.1..=1.0。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BowRelease {
    pub power: f32,
}

/// 初速（格/tick）：`BowItem.java:41` shoot velocity = `pow × 3.0F`。
pub fn release_velocity(power: f32) -> f32 {
    power * 3.0
}

/// 蓄力曲线（BowItem.java:74-81）：`(p² + 2p)/3`，p = t/20，封顶 1.0。
pub fn power_for_time(time_held: u32) -> f32 {
    let p = time_held as f32 / 20.0;
    let pow = (p * p + p * 2.0) / 3.0;
    pow.min(1.0)
}

/// 蓄/放状态机（`state` = 持有 tick；None = 未蓄）。
/// - `has_bow=false`：复位并吞掉输入（选中槽不是弓）。
/// - 按住且每 game tick +1（`on_tick` 门，20 tick/s）。
/// - 松开：结算 power；< 0.1 取消（BowItem.java:38）。
pub fn step_charge(
    now_placing: bool,
    on_tick: bool,
    has_bow: bool,
    state: &mut Option<u32>,
) -> Option<BowRelease> {
    if !has_bow {
        *state = None;
        return None;
    }
    match (*state, now_placing) {
        (None, true) => {
            *state = Some(0);
            None
        }
        (Some(t), true) => {
            if on_tick {
                *state = Some((t + 1).min(MAX_HOLD_TICKS));
            }
            None
        }
        (Some(t), false) => {
            *state = None;
            let pow = power_for_time(t);
            (pow >= MIN_POWER).then_some(BowRelease { power: pow })
        }
        (None, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BowItem.java:74-81 — 20 tick 满蓄 1.0；10 tick (0.25+1)/3≈0.4167
    /// （平方曲线误为 0.25）；1 tick ≈ 0.035 < 0.1 → 松开不放。
    #[test]
    fn power_curve_matches_source() {
        assert_eq!(power_for_time(20), 1.0);
        assert_eq!(power_for_time(0), 0.0);
        let half = power_for_time(10);
        assert!((half - (0.25 + 1.0) / 3.0).abs() < 1e-6, "half {half}");
        assert!(power_for_time(30) <= 1.0, "封顶");
        assert!(power_for_time(1) < MIN_POWER);
    }

    /// BowItem.java:41 — 初速 = pow×3.0。
    #[test]
    fn velocity_is_power_times_three() {
        assert_eq!(release_velocity(1.0), 3.0);
        assert_eq!(release_velocity(0.5), 1.5);
    }

    /// 蓄→放主流程：按住 N tick、松开结算；非 tick 步不涨。
    #[test]
    fn charge_then_release() {
        let mut st = None;
        assert!(step_charge(true, true, true, &mut st).is_none());
        assert_eq!(st, Some(0));
        // 60 Hz 步里只有 on_tick 步涨计。
        step_charge(true, false, true, &mut st);
        assert_eq!(st, Some(0));
        for _ in 0..20 {
            step_charge(true, true, true, &mut st);
        }
        assert_eq!(st, Some(20));
        let rel = step_charge(false, true, true, &mut st).expect("release");
        assert_eq!(rel.power, 1.0);
        assert_eq!(st, None);
    }

    /// BowItem.java:38 — pow < 0.1 取消（1 tick 内松手）。
    #[test]
    fn tap_cancelled() {
        let mut st = None;
        step_charge(true, true, true, &mut st);
        assert!(step_charge(false, true, true, &mut st).is_none());
    }

    /// 选中槽不是弓 → 状态复位（切武器打断蓄力，vanilla releaseUsing 行为）。
    #[test]
    fn no_bow_resets() {
        let mut st = Some(5);
        assert!(step_charge(true, true, false, &mut st).is_none());
        assert_eq!(st, None);
    }

    /// 72000 tick 封顶（BowItem.java:87-89）。
    #[test]
    fn hold_caps() {
        let mut st = Some(MAX_HOLD_TICKS - 1);
        step_charge(true, true, true, &mut st);
        assert_eq!(st, Some(MAX_HOLD_TICKS));
        step_charge(true, true, true, &mut st);
        assert_eq!(st, Some(MAX_HOLD_TICKS));
    }
}
