//! 食物：`FoodProperties` 属性表 + 进食状态机（26.1 反编译实读）。
//!
//! 源码依据（类名:行号）：
//! - **FoodProperties**：`world/food/FoodProperties.java:20` —
//!   `record(nutrition, saturation, canAlwaysEat)`，其中 `saturation` 是
//!   Builder.build() 时由 `saturationByModifier(nutrition, satmod)` 预乘的
//!   **绝对值**（FoodProperties.java:71-74）。本仓存 **modifier 原值**，结算
//!   时再乘（同 FoodData.eat(int, float) 的入参形态，FoodData.java:24-26）。
//! - **饱和公式**：`FoodConstants.saturationByModifier`（FoodConstants.java:30-32）
//!   = `nutrition × modifier × 2`。
//! - **FoodData.add 溢出规则**（FoodData.java:19-22）：hunger 钳 0..20、
//!   saturation 钳 `0..hunger`（饱和永远盖不过饥饿条）。
//! - **进食时长**：`Consumable.consumeTicks()` = `consumeSeconds × 20`
//!   （Consumable.java:100-101）；默认食物 `defaultFood().consumeSeconds(1.6F)`
//!   （Consumables.java:69）→ **32 tick**。派单说「默认 20」与源不符——
//!   1.6s × 20Hz = 32，按源码实况取 32。
//! - **canEat 门**：`Player.canEat`（Player.java:1581-1582）=
//!   `invulnerable || canAlwaysEat || foodData.needsFood()`（即 hunger < 20）。
//!   本仓 `invulnerable` 对应创造模式，由调用方并入。
//! - **右键重触发延迟**：原版按住右键每 4 tick 重走一次 useItem
//!   （`Minecraft.rightClickDelay = 4`，Minecraft.java:1705；按住自动重触发
//!   门 Minecraft.java:2036），连续进食节奏由它给出。
//!
//! ## 登记不造（物品表暂缺、不新增 id——等图标数据落地再入表）
//!
//! 数值逐条取自 26.1 `world/food/Foods.java` / `Consumables.java`，供后续
//! 接线直取（apple 原在此表，#90 掉落经济把物品补进表后已挪入 FOODS）：
//!
//! | 物品 | nutrition | satmod | 效果 | 出处 |
//! |---|---|---|---|---|
//! | baked_potato | 5 | 0.6 | — | Foods.java:5 |
//! | beef(生牛肉) | 3 | 0.3 | — | Foods.java:6 |
//! | bread | 5 | 0.6 | — | Foods.java:9 |
//! | carrot | 3 | 0.6 | — | Foods.java:10 |
//! | chicken(生鸡) | 2 | 0.3 | Hunger 600t @0.3 | Foods.java:11 + Consumables.java:23-25 |
//! | cooked_beef | 8 | 0.8 | — | Foods.java:14 |
//! | cooked_porkchop | 8 | 0.8 | — | Foods.java:18 |
//! | cookie | 2 | 0.1 | — | Foods.java:21 |
//! | golden_apple | 4 | 1.2 | alwaysEdible | Foods.java:24 |
//! | poisonous_potato | 2 | 0.3 | Poison 100t @0.6 | Foods.java:30 + Consumables.java:45-47 |
//! | porkchop(生猪排) | 3 | 0.3 | — | Foods.java:31 |
//! | potato | 1 | 0.3 | — | Foods.java:32 |

use crate::{APPLE, ITEMS, ROTTEN_FLESH, SPIDER_EYE};

/// 默认进食时长（tick）：`defaultFood().consumeSeconds(1.6F)`（Consumables.java:69）
/// × 20 tick/s（Consumable.java:100-101）= 32。
pub const DEFAULT_EAT_TICKS: u16 = 32;

/// 右键重触发延迟（tick）：原版 `Minecraft.rightClickDelay = 4`。
pub const RIGHT_CLICK_DELAY_TICKS: u32 = 4;

/// 食物附带效果的种类（26.1 MobEffects 的食物子集）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoodEffectKind {
    /// HUNGER：每 tick exhaustion +0.005×(amp+1)（HungerMobEffect.java:13-19）。
    Hunger,
    /// POISON：伤害结算走完整 hurt 管线（毒不死人，health>1 才扣）。
    Poison,
}

/// 食物附带效果（`ApplyStatusEffectsConsumeEffect` + `MobEffectInstance`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoodEffect {
    pub kind: FoodEffectKind,
    /// 时长（tick）；600 = 30s、100 = 5s（MobEffectInstance 构造入参）。
    pub duration_ticks: i32,
    pub amplifier: u8,
    /// 触发概率 [0,1]（ApplyStatusEffectsConsumeEffect probability）。
    pub chance: f32,
}

/// 食物属性（26.1 `Item.Properties().food(...)` 的数据面）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoodProperties {
    /// nutrition（Foods.java 各 Builder.nutrition）。
    pub nutrition: u8,
    /// saturationModifier（Builder.saturationModifier 原值，结算时 ×2×nutrition）。
    pub saturation_modifier: f32,
    /// 进食时长（tick）。默认 [`DEFAULT_EAT_TICKS`]；干海带 16（0.8s）、
    /// 蜂蜜瓶 40（2.0s）等特例在 26.1 里走 consumeSeconds 覆盖。
    pub eat_ticks: u16,
    /// 完食附带效果（可缺省）。
    pub effect: Option<FoodEffect>,
    /// alwaysEdible（饥饿满也能吃；腐肉/蜘蛛眼均 false）。
    pub can_always_eat: bool,
}

/// 进食中的状态（`player.isUsingItem()` 的本仓账本）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eating {
    /// 开始进食时的物品 id——换手（选中槽指向别的物品）即取消。
    pub item: u16,
    /// 已进行的进食 tick（每 on_tick +1）。
    pub ticks: u32,
}

impl FoodProperties {
    /// `FoodConstants.saturationByModifier`（FoodConstants.java:30-32）。
    pub fn saturation(&self) -> f32 {
        saturation_by_modifier(i32::from(self.nutrition), self.saturation_modifier)
    }

    /// `Consumable.consumeTicks`（Consumable.java:100-101）。
    pub fn consume_ticks(&self) -> u32 {
        u32::from(self.eat_ticks)
    }
}

/// 物品 id → 食物属性。未登记 = 非食物（空手/非食物右键零变化的前提）。
pub fn food_properties(item: u16) -> Option<&'static FoodProperties> {
    let idx = item as usize;
    if idx >= ITEMS.len() {
        return None;
    }
    FOODS.iter().find(|(id, _)| *id == item).map(|(_, f)| f)
}

/// 快捷栏/掉落可得的食物集（物品表现有 Material 杂物对得上原版的为准；
/// 面包/熟牛排等见文件头「登记不造」表——缺图标数据，不新增 id。apple
/// 由掉落经济 #90 追加进表，随此收进食物集）。
///
/// | 本仓物品 | 原版值 | 出处 |
/// |---|---|---|
/// | rotten_flesh(36) | 4 / 0.1 / Hunger 600t amp0 @0.8 | Foods.java:37 + Consumables.java:59-61 |
/// | spider_eye(41) | 2 / 0.8 / Poison 100t amp0 @1.0 | Foods.java:39 + Consumables.java:62-64 |
/// | apple(49) | 4 / 0.3 / — | Foods.java:4 |
pub static FOODS: [(u16, FoodProperties); 3] = [
    (
        ROTTEN_FLESH,
        FoodProperties {
            nutrition: 4,
            saturation_modifier: 0.1,
            eat_ticks: DEFAULT_EAT_TICKS,
            effect: Some(FoodEffect {
                kind: FoodEffectKind::Hunger,
                duration_ticks: 600,
                amplifier: 0,
                chance: 0.8,
            }),
            can_always_eat: false,
        },
    ),
    (
        SPIDER_EYE,
        FoodProperties {
            nutrition: 2,
            saturation_modifier: 0.8,
            eat_ticks: DEFAULT_EAT_TICKS,
            effect: Some(FoodEffect {
                kind: FoodEffectKind::Poison,
                duration_ticks: 100,
                amplifier: 0,
                chance: 1.0,
            }),
            can_always_eat: false,
        },
    ),
    (
        APPLE,
        FoodProperties {
            nutrition: 4,
            saturation_modifier: 0.3,
            eat_ticks: DEFAULT_EAT_TICKS,
            effect: None,
            can_always_eat: false,
        },
    ),
];

/// 饱和增量（FoodConstants.java:30-32）：`nutrition × modifier × 2`。
pub fn saturation_by_modifier(nutrition: i32, modifier: f32) -> f32 {
    nutrition as f32 * modifier * 2.0
}

/// `FoodData.add`（FoodData.java:19-22）：hunger 钳 0..20、饱和钳 0..hunger。
pub fn add_food(hunger: &mut f32, saturation: &mut f32, nutrition: i32, saturation_add: f32) {
    *hunger = (*hunger + nutrition as f32).clamp(0.0, 20.0);
    *saturation = (*saturation + saturation_add).clamp(0.0, *hunger);
}

/// `FoodData.eat(int, float)`（FoodData.java:24-26）：nutrition 回饥饿，
/// 饱和 = 公式值，溢出规则照 `add_food`。
pub fn eat(hunger: &mut f32, saturation: &mut f32, nutrition: i32, saturation_modifier: f32) {
    let sat = saturation_by_modifier(nutrition, saturation_modifier);
    add_food(hunger, saturation, nutrition, sat);
}

/// `Player.canEat`（Player.java:1581-1582）去掉 invulnerable 项的纯函数面：
/// 永远可吃（canAlwaysEat）或饥饿未满。invulnerable = 创造，调用方并入。
pub fn can_eat(hunger: f32, can_always_eat: bool) -> bool {
    can_always_eat || hunger < 20.0
}

/// 效果概率掷（ApplyStatusEffectsConsumeEffect）：`rand01 ∈ [0,1)` 小于
/// chance 即触发。纯函数面——随机源留在调用方（可测）。
pub fn effect_fires(effect: &FoodEffect, rand01: f32) -> bool {
    rand01 < effect.chance
}

/// 进食状态机一拍（推进/取消/完成三态；**启动不在本函数**——原版按住
/// 右键的启动走 useItem 入口，工作台等「方块 use 优先」的拦截发生在
/// 那一层，本仓对应 game.rs `interact(true)` 的食物分支）：
/// - `held` = 选中槽物品 id（空槽 None）；`food` = 该物品的食物属性；
/// - `can_eat` = 调用方并入创造（invulnerable）后的 [`can_eat`] 结果；
/// - 返回 Some(item) = 本拍完成进食（调用方结算 FoodData.eat / 效果 /
///   物品 −1）；取消（松手/换手/非食物/不可吃）一律清账、进度不保留。
pub fn step_eating(
    now_placing: bool,
    on_tick: bool,
    held: Option<u16>,
    food: Option<&FoodProperties>,
    can_eat: bool,
    state: &mut Option<Eating>,
) -> Option<u16> {
    // 松手 = releaseUsing 取消；手上不再是开始时那件食物 = 换手取消。
    let still_holding = held.is_some() && state.is_some_and(|s| held == Some(s.item));
    if !now_placing || !still_holding || !can_eat {
        *state = None;
        return None;
    }
    let Some(props) = food else {
        *state = None;
        return None;
    };
    let s = state.as_mut()?;
    if on_tick {
        s.ticks += 1;
        if s.ticks >= props.consume_ticks() {
            let item = s.item;
            *state = None;
            return Some(item);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const BONE: u16 = 37;

    /// Foods.java:9 BREAD = nutrition 5 / satmod 0.6（登记不造，走纯函数面）。
    fn eat_bread(hunger: &mut f32, saturation: &mut f32) {
        eat(hunger, saturation, 5, 0.6);
    }

    #[test]
    fn eat_bread_adds_five_hunger_and_formula_saturation() {
        // 吃面包：hunger +5、饱和 = 5×0.6×2 = 6.0（FoodConstants.java:30-32）。
        let (mut hunger, mut saturation) = (10.0_f32, 0.0_f32);
        eat_bread(&mut hunger, &mut saturation);
        assert_eq!(hunger, 15.0);
        assert_eq!(saturation, 6.0);
        // 饱和钳 0..hunger（FoodData.java:21）：金苹果(4/1.2，Foods.java:24)
        // 饱和公式 9.6 > 新 hunger 4 → 盖到 4。
        let (mut hunger, mut saturation) = (0.0_f32, 0.0_f32);
        eat(&mut hunger, &mut saturation, 4, 1.2);
        assert_eq!(hunger, 4.0);
        assert_eq!(saturation, 4.0, "饱和不得盖过饥饿条（FoodData.java:21）");
    }

    #[test]
    fn full_hunger_wastes_nutrition_and_gates_eating() {
        // 溢出规则（FoodData.java:19-22）：hunger 19 吃牛排(8/0.8) →
        // hunger 钳 20、饱和 = 12.8（≤ 新 hunger 20，不截）。
        let (mut hunger, mut saturation) = (19.0_f32, 0.0_f32);
        eat(&mut hunger, &mut saturation, 8, 0.8);
        assert_eq!(hunger, 20.0, "nutrition 超出 20 上限整段浪费（钳 0..20）");
        assert_eq!(saturation, 12.8);
        // 满饥饿门（Player.java:1581-1582 needsFood）：20 → 拒吃；
        // canAlwaysEat（金苹果）豁免。
        assert!(!can_eat(20.0, false));
        assert!(can_eat(20.0, true));
        assert!(can_eat(19.5, false));
        // 满饱和吃面包：饱和公式 6.0 但 hunger 已满 → 钳到 20。
        let (mut hunger, mut saturation) = (20.0_f32, 19.0_f32);
        eat_bread(&mut hunger, &mut saturation);
        assert_eq!(hunger, 20.0);
        assert_eq!(saturation, 20.0, "饱和钳 0..hunger=20");
    }

    #[test]
    fn rotten_flesh_and_spider_eye_match_vanilla() {
        // Foods.java:37 + Consumables.java:59-61。
        let rf = food_properties(ROTTEN_FLESH).expect("腐肉是食物");
        assert_eq!(rf.nutrition, 4);
        assert_eq!(rf.saturation_modifier, 0.1);
        assert_eq!(
            rf.eat_ticks, 32,
            "默认 1.6s × 20 = 32 tick（派单 20 与源不符）"
        );
        let eff = rf.effect.expect("腐肉挂 hunger 效果");
        assert_eq!(eff.kind, FoodEffectKind::Hunger);
        assert_eq!(eff.duration_ticks, 600, "600 tick = 30s");
        assert_eq!(eff.amplifier, 0);
        assert!((eff.chance - 0.8).abs() < f32::EPSILON);
        assert_eq!(rf.saturation(), 0.8, "4 × 0.1 × 2");
        // Foods.java:39 + Consumables.java:62-64。
        let se = food_properties(SPIDER_EYE).expect("蜘蛛眼是食物");
        assert_eq!(se.nutrition, 2);
        assert_eq!(se.saturation_modifier, 0.8);
        let eff = se.effect.expect("蜘蛛眼挂 poison 效果");
        assert_eq!(eff.kind, FoodEffectKind::Poison);
        assert_eq!(eff.duration_ticks, 100);
        assert!(effect_fires(&eff, 0.0), "chance 1.0 必中");
        assert!(effect_fires(&se.effect.unwrap(), 0.999_9));
        // Foods.java:4 APPLE = 4 / 0.3，无效果（#90 起物品在表）。
        let ap = food_properties(APPLE).expect("苹果是食物");
        assert_eq!(ap.nutrition, 4);
        assert_eq!(ap.saturation_modifier, 0.3);
        assert!(ap.effect.is_none());
        assert_eq!(ap.saturation(), 2.4, "4 × 0.3 × 2");
        // 非食物：骨头不在 FOODS；越界 id 平淡回 None。
        assert!(food_properties(BONE).is_none());
        assert!(food_properties(u16::MAX).is_none());
        // 概率面纯函数。
        let rf_eff = rf.effect.unwrap();
        assert!(effect_fires(&rf_eff, 0.79));
        assert!(!effect_fires(&rf_eff, 0.81));
    }

    #[test]
    fn chained_eating_caps_at_twenty() {
        // 连续吃到 20 上限：hunger 1 起步、腐肉 nutrition 4 → 5 次到 20，
        // 第 6 次被满饥饿门拦下（nutrition 浪费规则 = 吃不进）。
        let rf = food_properties(ROTTEN_FLESH).unwrap();
        let (mut hunger, mut saturation) = (1.0_f32, 0.0_f32);
        let mut eats = 0;
        while can_eat(hunger, rf.can_always_eat) {
            eat(
                &mut hunger,
                &mut saturation,
                i32::from(rf.nutrition),
                rf.saturation_modifier,
            );
            eats += 1;
            assert!(eats <= 5, "不应超过 5 次仍可吃");
        }
        assert_eq!(hunger, 20.0);
        assert_eq!(eats, 5);
        // 腐肉饱和增量 = 4×0.1×2 = 0.8/次；5 次 4.0，钳 0..20 不截。
        assert!((saturation - 4.0).abs() < 1e-4);
    }

    #[test]
    fn releasing_mid_eat_cancels_without_progress_memory() {
        // 进食中途松手：清账、不结算、进度不保留（下次按住从头吃）。
        // 启动沿归 useItem 入口（game.rs interact），此处手工落启动态。
        let rf = food_properties(ROTTEN_FLESH).unwrap();
        let mut state = Some(Eating {
            item: ROTTEN_FLESH,
            ticks: 0,
        });
        // FoodData 对照组：状态机不落结算，末尾逐值断言账未动。
        let (hunger, saturation) = (10.0_f32, 0.0_f32);
        // 推进 11 tick。
        for _ in 0..11 {
            assert!(
                step_eating(true, true, Some(ROTTEN_FLESH), Some(rf), true, &mut state).is_none()
            );
        }
        assert_eq!(
            state,
            Some(Eating {
                item: ROTTEN_FLESH,
                ticks: 11
            })
        );
        // 松手 → 取消。
        assert!(step_eating(false, true, Some(ROTTEN_FLESH), Some(rf), true, &mut state).is_none());
        assert_eq!(state, None);
        // 重新按住必须先有新启动态；从头计时 31 tick 不到。
        for _ in 0..31 {
            assert!(
                step_eating(true, true, Some(ROTTEN_FLESH), Some(rf), true, &mut state).is_none()
            );
        }
        assert_eq!(state, None, "无启动态不凭空续吃");
        state = Some(Eating {
            item: ROTTEN_FLESH,
            ticks: 0,
        });
        for _ in 0..31 {
            assert!(
                step_eating(true, true, Some(ROTTEN_FLESH), Some(rf), true, &mut state).is_none()
            );
        }
        assert_eq!(state.as_ref().map(|s| s.ticks), Some(31));
        assert_eq!(
            step_eating(true, true, Some(ROTTEN_FLESH), Some(rf), true, &mut state),
            Some(ROTTEN_FLESH),
            "32 tick 完成"
        );
        assert_eq!(state, None);
        assert_eq!((hunger, saturation), (10.0, 0.0), "纯状态机不碰 FoodData");
    }

    #[test]
    fn switching_item_or_losing_food_cancels() {
        // 换手（selected 变成别的物品）→ 取消；选中槽变空 → 取消。
        let rf = food_properties(ROTTEN_FLESH).unwrap();
        let mut state = Some(Eating {
            item: ROTTEN_FLESH,
            ticks: 3,
        });
        // 手上换成骨头（非食物）：food=None、held 变化。
        assert!(step_eating(true, true, Some(BONE), None, true, &mut state).is_none());
        assert_eq!(state, None);
        // 手上空了。
        state = Some(Eating {
            item: ROTTEN_FLESH,
            ticks: 3,
        });
        assert!(step_eating(true, true, None, None, true, &mut state).is_none());
        assert_eq!(state, None);
        // 满饥饿（can_eat=false）进行中的进食一并取消。
        state = Some(Eating {
            item: ROTTEN_FLESH,
            ticks: 3,
        });
        assert!(step_eating(true, true, Some(ROTTEN_FLESH), Some(rf), false, &mut state).is_none());
        assert_eq!(state, None);
    }
}
