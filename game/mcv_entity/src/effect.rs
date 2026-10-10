//! 状态效果机制层：效果注册数据、tick 调度、属性修饰与活动效果账本。
//!
//! 机制规格取自 26.1 反编译源码（`文件:行号` 注记=出处引用，非表达转录）；
//! 表达层为本仓自有设计：`Kind` 枚举 + 纯函数（触发计划 / tick 动作 / 属性
//! 修饰 / 混合因子）+ [`EffectBook`] 活动账本，不镜像原版的 Holder 注册表、
//! Codec、MobEffect 继承树与 MobEffectInstance 字段布局。
//!
//! 语义骨架（出处）：
//! - tick 门序：有余量 → 按 tick 相位判定触发 → 触发动作返回 false 即移除 →
//!   时长递减（含隐藏链）→ 到期回退隐藏效果（MobEffectInstance.java:223-240、
//!   :255-271）
//! - 触发相位：有限时长用 **剩余时长** 作 tick 计数（效果重施相位重启），
//!   无限用世界 tick（MobEffectInstance.java:228）
//! - 属性修饰量 = 模板量 × (amplifier+1)（MobEffect.java:200-204）；合成序 =
//!   先加 ADD_VALUE 再乘 (1+ADD_MULTIPLIED_TOTAL)（AttributeInstance.java:147-164）
//! - 无时时长 −1（MobEffectInstance.java:27）；amplifier 钳 0..255（:28-29/:81）

use std::cmp::Ordering;

/// 无限时长的哨兵值（MobEffectInstance.java:27 INFINITE_DURATION）。
pub const INFINITE: i32 = -1;
/// amplifier 上限（MobEffectInstance.java:29 MAX_AMPLIFIER；下限 0）。
pub const MAX_AMPLIFIER: u8 = 255;

/// 效果类别（MobEffectCategory.java:5-8）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Beneficial,
    Harmful,
    Neutral,
}

/// 效果种类（MobEffects.java:15-123 注册表全 40 项）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Speed,
    Slowness,
    Haste,
    MiningFatigue,
    Strength,
    InstantHealth,
    InstantDamage,
    JumpBoost,
    Nausea,
    Regeneration,
    Resistance,
    FireResistance,
    WaterBreathing,
    Invisibility,
    Blindness,
    NightVision,
    Hunger,
    Weakness,
    Poison,
    Wither,
    HealthBoost,
    Absorption,
    Saturation,
    Glowing,
    Levitation,
    Luck,
    Unluck,
    SlowFalling,
    ConduitPower,
    DolphinsGrace,
    BadOmen,
    HeroOfTheVillage,
    Darkness,
    TrialOmen,
    RaidOmen,
    WindCharged,
    Weaving,
    Oozing,
    Infested,
    BreathOfTheNautilus,
}

pub const ALL_KINDS: [Kind; 40] = [
    Kind::Speed,
    Kind::Slowness,
    Kind::Haste,
    Kind::MiningFatigue,
    Kind::Strength,
    Kind::InstantHealth,
    Kind::InstantDamage,
    Kind::JumpBoost,
    Kind::Nausea,
    Kind::Regeneration,
    Kind::Resistance,
    Kind::FireResistance,
    Kind::WaterBreathing,
    Kind::Invisibility,
    Kind::Blindness,
    Kind::NightVision,
    Kind::Hunger,
    Kind::Weakness,
    Kind::Poison,
    Kind::Wither,
    Kind::HealthBoost,
    Kind::Absorption,
    Kind::Saturation,
    Kind::Glowing,
    Kind::Levitation,
    Kind::Luck,
    Kind::Unluck,
    Kind::SlowFalling,
    Kind::ConduitPower,
    Kind::DolphinsGrace,
    Kind::BadOmen,
    Kind::HeroOfTheVillage,
    Kind::Darkness,
    Kind::TrialOmen,
    Kind::RaidOmen,
    Kind::WindCharged,
    Kind::Weaving,
    Kind::Oozing,
    Kind::Infested,
    Kind::BreathOfTheNautilus,
];

/// 即时效果（喝下即结算，无持续账本意义）——InstantenousMobEffect 子类
/// （HealOrHarmMobEffect.java:8 / SaturationMobEffect.java:7）。
pub fn is_instant(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::InstantHealth | Kind::InstantDamage | Kind::Saturation
    )
}

/// 类别（MobEffects.java 逐项构造参数）。
pub fn category(kind: Kind) -> Category {
    match kind {
        Kind::Speed
        | Kind::Haste
        | Kind::Strength
        | Kind::InstantHealth
        | Kind::JumpBoost
        | Kind::Regeneration
        | Kind::Resistance
        | Kind::FireResistance
        | Kind::WaterBreathing
        | Kind::Invisibility
        | Kind::NightVision
        | Kind::HealthBoost
        | Kind::Absorption
        | Kind::Saturation
        | Kind::Luck
        | Kind::SlowFalling
        | Kind::ConduitPower
        | Kind::DolphinsGrace
        | Kind::HeroOfTheVillage
        | Kind::BreathOfTheNautilus => Category::Beneficial,
        Kind::Slowness
        | Kind::MiningFatigue
        | Kind::InstantDamage
        | Kind::Nausea
        | Kind::Blindness
        | Kind::Hunger
        | Kind::Weakness
        | Kind::Poison
        | Kind::Wither
        | Kind::Levitation
        | Kind::Unluck
        | Kind::Darkness
        | Kind::WindCharged
        | Kind::Weaving
        | Kind::Oozing
        | Kind::Infested => Category::Harmful,
        Kind::Glowing | Kind::BadOmen | Kind::TrialOmen | Kind::RaidOmen => Category::Neutral,
    }
}

/// 粒子/图标色（RGB int，MobEffects.java 逐项构造参数）。
pub fn color(kind: Kind) -> u32 {
    match kind {
        Kind::Speed => 3402751,
        Kind::Slowness => 9154528,
        Kind::Haste => 14270531,
        Kind::MiningFatigue => 4866583,
        Kind::Strength => 16762624,
        Kind::InstantHealth | Kind::Saturation => 16262179,
        Kind::InstantDamage => 11101546,
        Kind::JumpBoost => 16646020,
        Kind::Nausea => 5578058,
        Kind::Regeneration => 13458603,
        Kind::Resistance => 9520880,
        Kind::FireResistance => 16750848,
        Kind::WaterBreathing => 10017472,
        Kind::Invisibility => 16185078,
        Kind::Blindness => 2039587,
        Kind::NightVision => 12779366,
        Kind::Hunger => 5797459,
        Kind::Weakness => 4738376,
        Kind::Poison => 8889187,
        Kind::Wither => 7561558,
        Kind::HealthBoost => 16284963,
        Kind::Absorption => 2445989,
        Kind::Glowing => 9740385,
        Kind::Levitation => 13565951,
        Kind::Luck => 5882118,
        Kind::Unluck => 12624973,
        Kind::SlowFalling => 15978425,
        Kind::ConduitPower => 1950417,
        Kind::DolphinsGrace => 8954814,
        Kind::BadOmen => 745784,
        Kind::HeroOfTheVillage => 4521796,
        Kind::Darkness => 2696993,
        Kind::TrialOmen => 1484454,
        Kind::RaidOmen => 14565464,
        Kind::WindCharged => 12438015,
        Kind::Weaving => 7891290,
        Kind::Oozing => 10092451,
        Kind::Infested => 9214860,
        Kind::BreathOfTheNautilus => 65518,
    }
}

/// 受属性影响的维度（本域全部修饰目标；Attribute 实体子集）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attr {
    MoveSpeed,
    AttackSpeed,
    AttackDamage,
    SafeFallDistance,
    MaxHealth,
    MaxAbsorption,
    Luck,
    WaypointTransmitRange,
}

/// 修饰运算（AttributeModifier.Operation 中本域用到的两种）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrOp {
    /// 加到基值上（Operation.ADD_VALUE）。
    AddValue,
    /// 总值乘 (1+amount)（Operation.ADD_MULTIPLIED_TOTAL）。
    MultiplyTotal,
}

/// 单条属性修饰（量已含 amplifier 展开倍数）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttrDelta {
    pub attr: Attr,
    pub op: AttrOp,
    pub amount: f64,
}

/// 属性修饰模板（MobEffect.java:136-141 + :200-204 amount×(amplifier+1)）。
/// 注册表出处逐项标注 MobEffects.java 行号。
pub fn attribute_modifier(kind: Kind, amplifier: u8) -> Option<AttrDelta> {
    let scale = f64::from(amplifier) + 1.0;
    let d = |attr: Attr, op: AttrOp, base: f64| {
        Some(AttrDelta {
            attr,
            op,
            amount: base * scale,
        })
    };
    match kind {
        Kind::Speed => d(Attr::MoveSpeed, AttrOp::MultiplyTotal, 0.2),
        Kind::Slowness => d(Attr::MoveSpeed, AttrOp::MultiplyTotal, -0.15),
        Kind::Haste => d(Attr::AttackSpeed, AttrOp::MultiplyTotal, 0.1),
        Kind::MiningFatigue => d(Attr::AttackSpeed, AttrOp::MultiplyTotal, -0.1),
        Kind::Strength => d(Attr::AttackDamage, AttrOp::AddValue, 3.0),
        Kind::JumpBoost => d(Attr::SafeFallDistance, AttrOp::AddValue, 1.0),
        Kind::Invisibility => d(Attr::WaypointTransmitRange, AttrOp::MultiplyTotal, -1.0),
        Kind::Weakness => d(Attr::AttackDamage, AttrOp::AddValue, -4.0),
        Kind::HealthBoost => d(Attr::MaxHealth, AttrOp::AddValue, 4.0),
        Kind::Absorption => d(Attr::MaxAbsorption, AttrOp::AddValue, 4.0),
        Kind::Luck => d(Attr::Luck, AttrOp::AddValue, 1.0),
        Kind::Unluck => d(Attr::Luck, AttrOp::AddValue, -1.0),
        _ => None,
    }
}

/// 汇总后的属性变化面（本域内同维度至多两类效果叠加）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bundle {
    /// 速度乘子 Π(1+amount)（speed +0.2、slowness −0.15）。
    pub move_speed_mult: f64,
    /// 攻速乘子 Π(1+amount)（haste +0.1、mining_fatigue −0.1）。
    pub attack_speed_mult: f64,
    /// 近战伤害加值 Σ（strength +3、weakness −4）。
    pub attack_damage_add: f64,
    /// 安全坠落距离加值 Σ（jump_boost +1）。
    pub safe_fall_add: f64,
    /// 生命上限加值 Σ（health_boost +4）。
    pub max_health_add: f64,
    /// 吸收上限加值 Σ（absorption +4）。
    pub max_absorption_add: f64,
    /// 幸运加值 Σ（luck/unluck ±1）。
    pub luck_add: f64,
}

impl Default for Bundle {
    fn default() -> Self {
        Self {
            move_speed_mult: 1.0,
            attack_speed_mult: 1.0,
            attack_damage_add: 0.0,
            safe_fall_add: 0.0,
            max_health_add: 0.0,
            max_absorption_add: 0.0,
            luck_add: 0.0,
        }
    }
}

impl Bundle {
    /// 按 AttributeInstance.java:147-164 的合成序归并一串修饰。
    pub fn merge(deltas: impl IntoIterator<Item = AttrDelta>) -> Self {
        let mut b = Bundle::default();
        for d in deltas {
            match (d.attr, d.op) {
                (Attr::MoveSpeed, AttrOp::MultiplyTotal) => b.move_speed_mult *= 1.0 + d.amount,
                (Attr::AttackSpeed, AttrOp::MultiplyTotal) => b.attack_speed_mult *= 1.0 + d.amount,
                (Attr::AttackDamage, AttrOp::AddValue) => b.attack_damage_add += d.amount,
                (Attr::SafeFallDistance, AttrOp::AddValue) => b.safe_fall_add += d.amount,
                (Attr::MaxHealth, AttrOp::AddValue) => b.max_health_add += d.amount,
                (Attr::MaxAbsorption, AttrOp::AddValue) => b.max_absorption_add += d.amount,
                (Attr::Luck, AttrOp::AddValue) => b.luck_add += d.amount,
                // WaypointTransmitRange 本仓无接收端（传送信标系统缺位），
                // 修饰表保留规格、汇总面不承载。
                (Attr::WaypointTransmitRange, _) => {}
                // 同维度混用两种 op 的组合在本注册表中不存在。
                _ => {}
            }
        }
        b
    }
}

/// 本 tick 是否触发周期动作（MobEffect.shouldApplyEffectTickThisTick
/// MobEffect.java:91-93 默认 false）。
///
/// `tick_count`：有限时长传剩余时长、无限传世界 tick（MobEffectInstance.java:228）。
pub fn fires_this_tick(kind: Kind, amplifier: u8, tick_count: i32) -> bool {
    match kind {
        // 间隔公式 25>>amp / 50>>amp / 40>>amp；interval==0 恒触发
        //（PoisonMobEffect.java:23-26 / RegenerationMobEffect.java:21-24 /
        // WitherMobEffect.java:20-23）。Java int 移位按 5 位掩码（JLS 15.19）：
        // amplifier 32..=255 实际移 0..=31 位。
        Kind::Poison => fires_on_interval(25, amplifier, tick_count),
        Kind::Regeneration => fires_on_interval(50, amplifier, tick_count),
        Kind::Wither => fires_on_interval(40, amplifier, tick_count),
        // 恒触发（HungerMobEffect.java:22-24 / AbsorptionMobEffect.java:17-19 /
        // BadOmenMobEffect.java:15-17）
        Kind::Hunger | Kind::Absorption | Kind::BadOmen => true,
        // 剩余 1 tick 那一格触发（RaidOmenMobEffect.java:15-17）
        Kind::RaidOmen => tick_count == 1,
        // 即时效果在存续期内每 tick 触发（InstantenousMobEffect.java:14-16）
        Kind::InstantHealth | Kind::InstantDamage | Kind::Saturation => tick_count >= 1,
        _ => false,
    }
}

fn fires_on_interval(base: i32, amplifier: u8, tick_count: i32) -> bool {
    let interval = base >> (amplifier & 31);
    interval <= 0 || tick_count % interval == 0
}

/// 周期动作的结算界面（本仓自有：纯引用切片，不引入目标对象抽象）。
/// 两个寿命分开：`'a` = 血/盾引用，`'f` = 食物/伤害队列引用——`FoodMut`
/// 内部引用在 `&mut` 下不变（invariant），与血引用绑死会让调用方的
/// 独立借用区无法各自收缩。
pub struct TickTarget<'a, 'f> {
    pub health: &'a mut f32,
    pub max_health: f32,
    pub absorb: &'a mut f32,
    /// 玩家侧食物三元组（mob 无 FoodData，传 None）。
    pub food: Option<&'f mut FoodMut<'f>>,
    /// 伤害出队：结算需走完整 hurt 管线（无敌帧/吸收/难度），不可就地扣。
    pub harms: &'f mut Vec<Harm>,
}

/// 魔法/凋零伤害事件（damageSources().magic() / .wither()）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Harm {
    pub amount: f32,
    pub wither: bool,
}

/// 玩家食物面（FoodData 三字段，MobEffect 只经由 add/addExhaustion 触碰）。
pub struct FoodMut<'a> {
    pub hunger: &'a mut f32,
    pub saturation: &'a mut f32,
    pub exhaustion: &'a mut f32,
}

/// 周期动作本体（MobEffect.applyEffectTick MobEffect.java:81-83 默认恒 true；
/// 返回 false = 效果提前终止）。
pub fn apply_tick(kind: Kind, amplifier: u8, t: &mut TickTarget) -> bool {
    match kind {
        // PoisonMobEffect.java:14-20：health > 1.0（严格）才掉 1.0 魔法伤害
        // （毒不死人，残血 1 豁免）。
        Kind::Poison => {
            if *t.health > 1.0 {
                t.harms.push(Harm {
                    amount: 1.0,
                    wither: false,
                });
            }
            true
        }
        // RegenerationMobEffect.java:12-18：未满血回 1.0。
        Kind::Regeneration => {
            if *t.health < t.max_health {
                *t.health = (*t.health + 1.0).min(t.max_health);
            }
            true
        }
        // WitherMobEffect.java:14-17：无条件凋零伤害 1.0。
        Kind::Wither => {
            t.harms.push(Harm {
                amount: 1.0,
                wither: true,
            });
            true
        }
        // HungerMobEffect.java:13-19：每 tick exhaustion 0.005×(amp+1)。
        Kind::Hunger => {
            if let Some(f) = t.food.as_deref_mut() {
                add_exhaustion(f, 0.005 * (f32::from(amplifier) + 1.0));
            }
            true
        }
        // AbsorptionMobEffect.java:12-14：吸收盾耗尽即效果终止。
        Kind::Absorption => *t.absorb > 0.0,
        // HealOrHarmMobEffect.java:17-25。
        Kind::InstantHealth => {
            heal_or_harm(false, amplifier, t);
            true
        }
        Kind::InstantDamage => {
            heal_or_harm(true, amplifier, t);
            true
        }
        // SaturationMobEffect.java:13-19：FoodData.eat(amp+1, 1.0)。
        Kind::Saturation => {
            if let Some(f) = t.food.as_deref_mut() {
                feed(
                    f,
                    i32::from(amplifier) + 1,
                    2.0 * (f32::from(amplifier) + 1.0),
                );
            }
            true
        }
        // BadOmenMobEffect.java:20-34 / RaidOmenMobEffect.java:20-32 的村庄/
        // 突袭系统本仓缺位：效果自身照常走时长，转换钩子待上游域。
        _ => true,
    }
}

/// 治愈/伤害二相（HealOrHarmMobEffect.java:17-25）：isHarm ==
/// isInvertedHealAndHarm（不死族反转）→ 治愈 4<<amp；否则魔法伤害 6<<amp。
/// 本仓目标侧无不死属性（僵尸不接玩家效果），反转恒 false。
fn heal_or_harm(is_harm: bool, amplifier: u8, t: &mut TickTarget) {
    // Java int 移位 5 位掩码 + Math.max(...,0)（4<<31 为负 → 0）。
    let heal = 4i32.wrapping_shl(u32::from(amplifier) & 31).max(0);
    let hurt = 6i32.wrapping_shl(u32::from(amplifier) & 31).max(0);
    if is_harm {
        t.harms.push(Harm {
            amount: hurt as f32,
            wither: false,
        });
    } else if *t.health < t.max_health {
        *t.health = (*t.health + heal as f32).min(t.max_health);
    }
}

/// 即时施加（带威力缩放，HealOrHarmMobEffect.java:28-47——投掷药水路径；
/// 饮用路径走 [`apply_tick`]）。量 = (scale·(4|6)<<amp + 0.5) 截断取整。
pub fn apply_instant(kind: Kind, amplifier: u8, scale: f64, t: &mut TickTarget) {
    if !matches!(kind, Kind::InstantHealth | Kind::InstantDamage) {
        return;
    }
    let is_harm = kind == Kind::InstantDamage;
    let base = if is_harm { 6i32 } else { 4i32 };
    let amount = (scale * f64::from(base.wrapping_shl(u32::from(amplifier) & 31)) + 0.5) as i32;
    if is_harm {
        t.harms.push(Harm {
            amount: amount as f32,
            wither: false,
        });
    } else {
        *t.health = (*t.health + amount as f32).min(t.max_health);
    }
}

/// FoodData.add（FoodData.java:19-22）：food 钳 0..20、saturation 钳 0..food。
fn feed(f: &mut FoodMut, food: i32, saturation: f32) {
    *f.hunger = (*f.hunger + food as f32).clamp(0.0, 20.0);
    *f.saturation = (*f.saturation + saturation).clamp(0.0, *f.hunger);
}

/// FoodData.addExhaustion（FoodData.java:100-102）：上限 40。
fn add_exhaustion(f: &mut FoodMut, amount: f32) {
    *f.exhaustion = (*f.exhaustion + amount).min(40.0);
}

/// 死亡爆发（MobEffect.onMobRemoved，RemovalReason.KILLED 才生效）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeathBurst {
    /// 粘液分裂：requested=2、size=2（MobEffects.java:119 + OozingMobEffect.java:21/:37-43）。
    Slimes { requested: i32, size: i32 },
    /// 蛛网铺设：数量 2..=3、15 次尝试、半径 1（MobEffects.java:117 +
    /// WeavingMobEffect.java:32-52）。
    Cobwebs {
        count: (i32, i32),
        attempts: i32,
        radius: i32,
    },
    /// 风爆：威力 3.0+rand·2.0（WindChargedMobEffect.java:18-40）。
    WindBurst,
}

pub fn death_burst(kind: Kind) -> Option<DeathBurst> {
    match kind {
        Kind::Oozing => Some(DeathBurst::Slimes {
            requested: 2,
            size: 2,
        }),
        Kind::Weaving => Some(DeathBurst::Cobwebs {
            count: (2, 3),
            attempts: 15,
            radius: 1,
        }),
        Kind::WindCharged => Some(DeathBurst::WindBurst),
        _ => None,
    }
}

/// 粘液数量对实体挤压上限的钳制（OozingMobEffect.java:30-32）：
/// cramming < 1 不限；否则 min(max(cramming−nearby, 0), requested)。
pub fn slimes_to_spawn(max_cramming: i32, nearby_slimes: i32, requested: i32) -> i32 {
    if max_cramming < 1 {
        requested
    } else {
        (max_cramming - nearby_slimes).clamp(0, requested)
    }
}

/// 风爆威力（WindChargedMobEffect.java:23）。
pub fn wind_burst_power(rand01: f32) -> f32 {
    3.0 + rand01 * 2.0
}

/// 受击爆发：概率 + 蠹虫数量区间（InfestedMobEffect.java:28-36 +
/// MobEffects.java:121 概率 0.1、数量 1..=2）。
pub fn hurt_burst(kind: Kind) -> Option<(f32, i32, i32)> {
    match kind {
        Kind::Infested => Some((0.1, 1, 2)),
        _ => None,
    }
}

/// 视觉混合时长 (进入, 消退, 提前量)，单位 tick（MobEffects.java:53 nausea
/// 150/20/60、:108 darkness 22 全量；其余 0 = 立即）。
pub fn blend_durations(kind: Kind) -> (i32, i32, i32) {
    match kind {
        Kind::Nausea => (150, 20, 60),
        Kind::Darkness => (22, 22, 22),
        _ => (0, 0, 0),
    }
}

/// 混合因子推进一步（MobEffectInstance.BlendState.tick:384-398）：目标 1/0 由
/// 「剩余 > 提前量」决定（hasEffect :400-402）；每 tick 向目标至多移动
/// 1/blend_ticks，时长 0 直接到位。渲染侧持有因子（含 partialTick 插值
/// :404-410），此处只提供纯步进。
pub fn blend_step(factor: f32, remaining: i32, blend_in: i32, blend_out: i32, advance: i32) -> f32 {
    let active = remaining == INFINITE || remaining > advance;
    let target = if active { 1.0 } else { 0.0 };
    if factor == target {
        return target;
    }
    let dur = if active { blend_in } else { blend_out };
    if dur == 0 {
        return target;
    }
    let max_delta = 1.0 / dur as f32;
    factor + (target - factor).clamp(-max_delta, max_delta)
}

/// 单条活动效果（MobEffectInstance 的账本语义子集：id/amplifier/duration/
/// ambient/visible/showIcon + 隐藏降级链 :39-46）。
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    pub kind: Kind,
    pub amplifier: u8,
    pub duration: i32,
    pub ambient: bool,
    pub visible: bool,
    pub show_icon: bool,
    /// 被更强效果覆盖时的旧效果链（MobEffectInstance.hiddenEffect :45/:139-156），
    /// 强效果到期后回退续走（:263-271）。
    hidden: Option<Box<Active>>,
}

impl Active {
    fn new(
        kind: Kind,
        duration: i32,
        amplifier: u8,
        ambient: bool,
        visible: bool,
        show_icon: bool,
    ) -> Self {
        Self {
            kind,
            amplifier: amplifier.min(MAX_AMPLIFIER),
            duration,
            ambient,
            visible,
            show_icon,
            hidden: None,
        }
    }

    fn has_remaining(&self) -> bool {
        self.duration == INFINITE || self.duration > 0
    }

    /// 替换语义（MobEffectInstance.update:132-175）。
    fn update(&mut self, incoming: Active) -> bool {
        // 可见性三元组提前拷出：强/弱分支可能整体转移 incoming（隐藏链）。
        let in_ambient = incoming.ambient;
        let in_visible = incoming.visible;
        let in_icon = incoming.show_icon;
        let mut changed = false;
        if incoming.amplifier > self.amplifier {
            // 更强即接管；若新时长更短，旧实例整体下潜为隐藏链头（:139-146）。
            if is_shorter_than(&incoming, self) {
                let mut demoted = self.clone();
                demoted.hidden = self.hidden.take();
                self.hidden = Some(Box::new(demoted));
            }
            self.amplifier = incoming.amplifier;
            self.duration = incoming.duration;
            changed = true;
        } else if is_shorter_than(self, &incoming) {
            if incoming.amplifier == self.amplifier {
                // 同强度更长 → 续时（:148-151）。
                self.duration = incoming.duration;
                changed = true;
            } else {
                // 更弱但更长 → 存入隐藏链待降级（:152-156，不计 changed）。
                match self.hidden.as_mut() {
                    Some(h) => {
                        h.update(incoming);
                    }
                    None => self.hidden = Some(Box::new(incoming)),
                }
            }
        }
        // 非 ambient 来袭解除 ambient（:159-162）；可见性/图标跟随（:164-172）。
        if (!in_ambient && self.ambient) || changed {
            self.ambient = in_ambient;
            changed = true;
        }
        if in_visible != self.visible {
            self.visible = in_visible;
            changed = true;
        }
        if in_icon != self.show_icon {
            self.show_icon = in_icon;
            changed = true;
        }
        changed
    }

    /// 时长递减，隐藏链逐层同步（MobEffectInstance.tickDownDuration:255-261）。
    fn tick_down(&mut self) {
        if let Some(h) = self.hidden.as_mut() {
            h.tick_down();
        }
        if self.duration != INFINITE {
            self.duration -= 1;
        }
    }

    /// 到期回退：duration 归 0 且有隐藏 → 换上隐藏的参数续走
    ///（MobEffectInstance.downgradeToHiddenEffect:263-271）。
    fn downgrade(&mut self) -> bool {
        if self.duration == 0
            && let Some(mut h) = self.hidden.take()
        {
            self.amplifier = h.amplifier;
            self.duration = h.duration;
            self.ambient = h.ambient;
            self.visible = h.visible;
            self.show_icon = h.show_icon;
            self.hidden = h.hidden.take();
            return true;
        }
        false
    }

    /// 单条推进（MobEffectInstance.tickServer:223-240）。
    fn advance<'a, 'f>(
        &mut self,
        world_tick: i32,
        health: &'a mut f32,
        max_health: f32,
        absorb: &'a mut f32,
        food: Option<&'f mut FoodMut<'f>>,
        harms: &'f mut Vec<Harm>,
    ) -> bool {
        if !self.has_remaining() {
            return false;
        }
        let tick_count = if self.duration == INFINITE {
            world_tick
        } else {
            self.duration
        };
        let mut t = TickTarget {
            health,
            max_health,
            absorb,
            food,
            harms,
        };
        if fires_this_tick(self.kind, self.amplifier, tick_count)
            && !apply_tick(self.kind, self.amplifier, &mut t)
        {
            return false;
        }
        self.tick_down();
        self.downgrade();
        self.has_remaining()
    }
}

/// isShorterDurationThan（MobEffectInstance.java:177-179）：非无限且
/// （自身更短 或 对方无限）。
fn is_shorter_than(a: &Active, b: &Active) -> bool {
    a.duration != INFINITE && (a.duration < b.duration || b.duration == INFINITE)
}

/// 单实体的活动效果账本（`Vec<Active>` 按 Kind 唯一，等价 activeEffects 映射）。
#[derive(Clone, Debug, Default)]
pub struct EffectBook {
    active: Vec<Active>,
}

impl EffectBook {
    /// 施加/覆盖一个效果（LivingEntity.addEffect:993-1012 +
    /// MobEffectInstance.update:132-175）。返回账本是否变化。
    pub fn apply(
        &mut self,
        kind: Kind,
        duration: i32,
        amplifier: u8,
        ambient: bool,
        visible: bool,
        show_icon: bool,
    ) -> bool {
        let incoming = Active::new(kind, duration, amplifier, ambient, visible, show_icon);
        match self.active.iter_mut().find(|a| a.kind == kind) {
            Some(cur) => cur.update(incoming),
            None => {
                self.active.push(incoming);
                true
            }
        }
    }

    /// 常用简化入口（默认非环境、可见、带图标——MobEffectInstance.java:56-58
    /// 三参构造链的默认值）。
    pub fn apply_simple(&mut self, kind: Kind, duration: i32, amplifier: u8) -> bool {
        self.apply(kind, duration, amplifier, false, true, true)
    }

    /// 入场动作（MobEffect.onEffectStarted:95-96；LivingEntity.addEffect:1010
    /// 每次施加都触发）：吸收盾抬到 max(current, 4×(amp+1))
    ///（AbsorptionMobEffect.java:22-25）。调用方在 apply 后调用。
    pub fn entrance(&self, absorb: &mut f32) {
        if let Some(a) = self.amplifier(Kind::Absorption) {
            *absorb = (*absorb).max(4.0 * (f32::from(a) + 1.0));
        }
    }

    /// 每 game tick 推进全部活动效果（MobEffectInstance.tickServer:223-240）。
    ///
    /// 回血/进食/吸收盾就地结算（保持逐效果顺序语义）；伤害经 [`Harm`]
    /// 返回，由调用方走 hurt 管线（无敌帧/吸收/难度缩放）。
    pub fn tick(
        &mut self,
        world_tick: i32,
        health: &mut f32,
        max_health: f32,
        absorb: &mut f32,
        mut food: Option<&mut FoodMut>,
    ) -> Vec<Harm> {
        let mut harms = Vec::new();
        let mut i = 0;
        while i < self.active.len() {
            let keep = self.active[i].advance(
                world_tick,
                health,
                max_health,
                absorb,
                food.as_deref_mut(),
                &mut harms,
            );
            if keep {
                i += 1;
            } else {
                self.active.remove(i);
            }
        }
        harms
    }

    pub fn get(&self, kind: Kind) -> Option<&Active> {
        self.active.iter().find(|a| a.kind == kind)
    }

    pub fn amplifier(&self, kind: Kind) -> Option<u8> {
        self.get(kind).map(|a| a.amplifier)
    }

    pub fn has(&self, kind: Kind) -> bool {
        self.get(kind).is_some()
    }

    /// 终止一个效果（喝奶/清除，LivingEntity.removeEffect 语义）。
    pub fn remove(&mut self, kind: Kind) -> bool {
        let before = self.active.len();
        self.active.retain(|a| a.kind != kind);
        self.active.len() != before
    }

    pub fn clear(&mut self) {
        self.active.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Active> {
        self.active.iter()
    }

    /// 属性汇总面（MobEffect.addAttributeModifiers:167-175 的加/除修饰 +
    /// AttributeInstance.java:147-164 合成）。
    pub fn bundle(&self) -> Bundle {
        Bundle::merge(
            self.active
                .iter()
                .filter_map(|a| attribute_modifier(a.kind, a.amplifier)),
        )
    }
}

/// 效果查询谓词族（MobEffectUtil.java:25-49 的机制面；formatDuration :16-23
/// 为 HUD 文本格式化，归 UI 层）。
/// 挖掘加速判定：急迫或潮汐能量（MobEffectUtil.java:25-27）。
pub fn has_dig_speed(book: &EffectBook) -> bool {
    book.has(Kind::Haste) || book.has(Kind::ConduitPower)
}

/// 挖掘加速强度：两来源 amplifier 取大（MobEffectUtil.java:29-41）。
pub fn dig_speed_amplifier(book: &EffectBook) -> u8 {
    book.amplifier(Kind::Haste)
        .max(book.amplifier(Kind::ConduitPower))
        .unwrap_or(0)
}

/// 水下呼吸判定：水呼吸 ∪ 潮汐能量 ∪ 鹦鹉螺之息（MobEffectUtil.java:43-45）。
pub fn has_water_breathing(book: &EffectBook) -> bool {
    book.has(Kind::WaterBreathing)
        || book.has(Kind::ConduitPower)
        || book.has(Kind::BreathOfTheNautilus)
}

/// 氧气是否随效果回补（MobEffectUtil.java:47-49）。
pub fn refills_air_supply(book: &EffectBook) -> bool {
    !book.has(Kind::BreathOfTheNautilus)
        || book.has(Kind::WaterBreathing)
        || book.has(Kind::ConduitPower)
}

/// HUD 排序语义（MobEffectInstance.compareTo:339-352）：存在 ≤32147 tick 的
/// 一方且不同为环境时 → 非环境优先、有限时长优先、更短优先、再按颜色；
/// 否则按（环境、颜色）。false < true 对应原版 compareFalseFirst。
pub fn hud_order(a: &Active, b: &Active) -> Ordering {
    let cutoff = 32_147;
    let any_short = a.duration <= cutoff || b.duration <= cutoff;
    if any_short && !(a.ambient && b.ambient) {
        a.ambient
            .cmp(&b.ambient)
            .then((a.duration == INFINITE).cmp(&(b.duration == INFINITE)))
            .then(a.duration.cmp(&b.duration))
            .then_with(|| color(a.kind).cmp(&color(b.kind)))
    } else {
        a.ambient
            .cmp(&b.ambient)
            .then_with(|| color(a.kind).cmp(&color(b.kind)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target<'a, 'f>(
        health: &'a mut f32,
        absorb: &'a mut f32,
        food: Option<&'f mut FoodMut<'f>>,
        harms: &'f mut Vec<Harm>,
    ) -> TickTarget<'a, 'f> {
        TickTarget {
            health,
            max_health: 20.0,
            absorb,
            food,
            harms,
        }
    }

    /// 注册表数据逐项对拍（MobEffects.java:15-123 构造参数）。
    #[test]
    fn registry_category_color_and_modifier() {
        let expect = [
            (Kind::Speed, Category::Beneficial, 3402751u32),
            (Kind::Slowness, Category::Harmful, 9154528),
            (Kind::Haste, Category::Beneficial, 14270531),
            (Kind::MiningFatigue, Category::Harmful, 4866583),
            (Kind::Strength, Category::Beneficial, 16762624),
            (Kind::InstantHealth, Category::Beneficial, 16262179),
            (Kind::InstantDamage, Category::Harmful, 11101546),
            (Kind::JumpBoost, Category::Beneficial, 16646020),
            (Kind::Nausea, Category::Harmful, 5578058),
            (Kind::Regeneration, Category::Beneficial, 13458603),
            (Kind::Resistance, Category::Beneficial, 9520880),
            (Kind::FireResistance, Category::Beneficial, 16750848),
            (Kind::WaterBreathing, Category::Beneficial, 10017472),
            (Kind::Invisibility, Category::Beneficial, 16185078),
            (Kind::Blindness, Category::Harmful, 2039587),
            (Kind::NightVision, Category::Beneficial, 12779366),
            (Kind::Hunger, Category::Harmful, 5797459),
            (Kind::Weakness, Category::Harmful, 4738376),
            (Kind::Poison, Category::Harmful, 8889187),
            (Kind::Wither, Category::Harmful, 7561558),
            (Kind::HealthBoost, Category::Beneficial, 16284963),
            (Kind::Absorption, Category::Beneficial, 2445989),
            (Kind::Saturation, Category::Beneficial, 16262179),
            (Kind::Glowing, Category::Neutral, 9740385),
            (Kind::Levitation, Category::Harmful, 13565951),
            (Kind::Luck, Category::Beneficial, 5882118),
            (Kind::Unluck, Category::Harmful, 12624973),
            (Kind::SlowFalling, Category::Beneficial, 15978425),
            (Kind::ConduitPower, Category::Beneficial, 1950417),
            (Kind::DolphinsGrace, Category::Beneficial, 8954814),
            (Kind::BadOmen, Category::Neutral, 745784),
            (Kind::HeroOfTheVillage, Category::Beneficial, 4521796),
            (Kind::Darkness, Category::Harmful, 2696993),
            (Kind::TrialOmen, Category::Neutral, 1484454),
            (Kind::RaidOmen, Category::Neutral, 14565464),
            (Kind::WindCharged, Category::Harmful, 12438015),
            (Kind::Weaving, Category::Harmful, 7891290),
            (Kind::Oozing, Category::Harmful, 10092451),
            (Kind::Infested, Category::Harmful, 9214860),
            (Kind::BreathOfTheNautilus, Category::Beneficial, 65518),
        ];
        assert_eq!(expect.len(), ALL_KINDS.len());
        for (kind, cat, col) in expect {
            assert_eq!(category(kind), cat, "{kind:?}");
            assert_eq!(color(kind), col, "{kind:?}");
        }
        // 即时三件套（InstantenousMobEffect 子类）。
        for k in [Kind::InstantHealth, Kind::InstantDamage, Kind::Saturation] {
            assert!(is_instant(k), "{k:?}");
        }
        assert!(!is_instant(Kind::Poison));
    }

    /// 属性修饰模板 amount×(amplifier+1)（MobEffect.java:200-204）。
    #[test]
    fn modifier_template_scales_with_amplifier() {
        let cases = [
            (Kind::Speed, Attr::MoveSpeed, AttrOp::MultiplyTotal, 0.2),
            (
                Kind::Slowness,
                Attr::MoveSpeed,
                AttrOp::MultiplyTotal,
                -0.15,
            ),
            (Kind::Haste, Attr::AttackSpeed, AttrOp::MultiplyTotal, 0.1),
            (
                Kind::MiningFatigue,
                Attr::AttackSpeed,
                AttrOp::MultiplyTotal,
                -0.1,
            ),
            (Kind::Strength, Attr::AttackDamage, AttrOp::AddValue, 3.0),
            (
                Kind::JumpBoost,
                Attr::SafeFallDistance,
                AttrOp::AddValue,
                1.0,
            ),
            (Kind::Weakness, Attr::AttackDamage, AttrOp::AddValue, -4.0),
            (Kind::HealthBoost, Attr::MaxHealth, AttrOp::AddValue, 4.0),
            (Kind::Absorption, Attr::MaxAbsorption, AttrOp::AddValue, 4.0),
            (Kind::Luck, Attr::Luck, AttrOp::AddValue, 1.0),
            (Kind::Unluck, Attr::Luck, AttrOp::AddValue, -1.0),
            (
                Kind::Invisibility,
                Attr::WaypointTransmitRange,
                AttrOp::MultiplyTotal,
                -1.0,
            ),
        ];
        for (kind, attr, op, base) in cases {
            let d0 = attribute_modifier(kind, 0).unwrap();
            assert_eq!((d0.attr, d0.op), (attr, op), "{kind:?}");
            assert!((d0.amount - base).abs() < 1e-12, "{kind:?} amp0");
            let d2 = attribute_modifier(kind, 2).unwrap();
            assert!((d2.amount - base * 3.0).abs() < 1e-12, "{kind:?} amp2");
        }
        // 无修饰效果（MobEffects.java:55 resistance 等）。
        assert_eq!(attribute_modifier(Kind::Resistance, 3), None);
        assert_eq!(attribute_modifier(Kind::Regeneration, 0), None);
    }

    /// tick 间隔公式（25/50/40 >> amp）+ Java 移位 5 位掩码 + 相位锚定剩余时长。
    #[test]
    fn interval_fires_table() {
        // 毒：25>>0=25 → 相位 0/25/50 触发，24 不触发（PoisonMobEffect.java:23-26）。
        assert!(fires_this_tick(Kind::Poison, 0, 0));
        assert!(!fires_this_tick(Kind::Poison, 0, 24));
        assert!(fires_this_tick(Kind::Poison, 0, 25));
        assert!(fires_this_tick(Kind::Poison, 0, 50));
        // amp1 → 12；amp3 → 3。
        assert!(fires_this_tick(Kind::Poison, 1, 12));
        assert!(!fires_this_tick(Kind::Poison, 1, 11));
        assert!(fires_this_tick(Kind::Poison, 3, 3));
        // amp5 → 25>>5=0 → 每 tick（Java 移位出 0，interval>0 门失效）。
        assert!(fires_this_tick(Kind::Poison, 5, 7));
        // Java 移位掩码：amp=32 → 移 0 位 → 25（JLS 15.19）；amp=255 → 25>>31=0。
        assert!(fires_this_tick(Kind::Poison, 32, 25));
        assert!(!fires_this_tick(Kind::Poison, 32, 24));
        assert!(fires_this_tick(Kind::Poison, 255, 12345));
        // 再生 50>>amp（RegenerationMobEffect.java:21-24）。
        assert!(fires_this_tick(Kind::Regeneration, 0, 50));
        assert!(!fires_this_tick(Kind::Regeneration, 0, 49));
        assert!(fires_this_tick(Kind::Regeneration, 2, 12));
        // 凋零 40>>amp（WitherMobEffect.java:20-23）。
        assert!(fires_this_tick(Kind::Wither, 0, 40));
        assert!(!fires_this_tick(Kind::Wither, 0, 39));
        assert!(fires_this_tick(Kind::Wither, 1, 20));
        // 恒触发族。
        for k in [Kind::Hunger, Kind::Absorption, Kind::BadOmen] {
            assert!(fires_this_tick(k, 0, 5), "{k:?}");
        }
        // 突袭誓约只在剩余 1 tick 触发（RaidOmenMobEffect.java:15-17）。
        assert!(!fires_this_tick(Kind::RaidOmen, 0, 2));
        assert!(fires_this_tick(Kind::RaidOmen, 0, 1));
        // 即时效果存续期每 tick（InstantenousMobEffect.java:14-16）。
        assert!(fires_this_tick(Kind::InstantHealth, 0, 1));
        assert!(!fires_this_tick(Kind::InstantHealth, 0, 0));
        assert!(fires_this_tick(Kind::Saturation, 1, 3));
        // 无动作效果默认不触发（MobEffect.java:91-93）。
        assert!(!fires_this_tick(Kind::Speed, 0, 0));
        assert!(!fires_this_tick(Kind::Resistance, 4, 100));
    }

    /// 毒/再生/凋零的周期动作与护栏。
    #[test]
    fn poison_regeneration_wither_tick_actions() {
        let mut harms = Vec::new();
        let (mut hp, mut ab) = (1.5f32, 0.0f32);
        // 毒在 health>1.0 才掉血（PoisonMobEffect.java:15）。
        assert!(apply_tick(
            Kind::Poison,
            0,
            &mut target(&mut hp, &mut ab, None, &mut harms)
        ));
        assert_eq!(harms.len(), 1);
        assert_eq!(
            harms[0],
            Harm {
                amount: 1.0,
                wither: false
            }
        );
        // 残血 1.0 豁免（毒不致死）。
        harms.clear();
        let (mut hp, mut ab) = (1.0f32, 0.0f32);
        assert!(apply_tick(
            Kind::Poison,
            4,
            &mut target(&mut hp, &mut ab, None, &mut harms)
        ));
        assert!(harms.is_empty());
        // 再生：未满血回 1.0、封顶 max（RegenerationMobEffect.java:12-18）。
        let (mut hp, mut ab) = (19.5f32, 0.0f32);
        apply_tick(
            Kind::Regeneration,
            1,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert!((hp - 20.0).abs() < 1e-6, "min(19.5+1, 20)");
        let (mut hp, mut ab) = (20.0f32, 0.0f32);
        apply_tick(
            Kind::Regeneration,
            0,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert!((hp - 20.0).abs() < 1e-6, "满血不动");
        // 凋零：无条件 1.0、wither 源（WitherMobEffect.java:14-17）。
        harms.clear();
        let (mut hp, mut ab) = (5.0f32, 0.0f32);
        apply_tick(
            Kind::Wither,
            0,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert_eq!(
            harms,
            vec![Harm {
                amount: 1.0,
                wither: true
            }]
        );
    }

    /// 治愈/伤害二相与即时缩放（HealOrHarmMobEffect.java:17-47）。
    #[test]
    fn heal_or_harm_amounts() {
        let mut harms = Vec::new();
        // instant_health amp0/2 → +4/+16（4<<amp）。
        let (mut hp, mut ab) = (10.0f32, 0.0f32);
        apply_tick(
            Kind::InstantHealth,
            0,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert!((hp - 14.0).abs() < 1e-6);
        let (mut hp, mut ab) = (10.0f32, 0.0f32);
        apply_tick(
            Kind::InstantHealth,
            2,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert!((hp - 26.0).abs() < 1e-6);
        // instant_damage amp0/3 → 6/48 魔法伤害（6<<amp）。
        harms.clear();
        let (mut hp, mut ab) = (20.0f32, 0.0f32);
        apply_tick(
            Kind::InstantDamage,
            0,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert_eq!(
            harms.last().copied(),
            Some(Harm {
                amount: 6.0,
                wither: false
            })
        );
        harms.clear();
        apply_tick(
            Kind::InstantDamage,
            3,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert_eq!(
            harms.last().copied(),
            Some(Harm {
                amount: 48.0,
                wither: false
            })
        );
        // 治愈封顶 max（heal 语义）。
        let (mut hp, mut ab) = (19.0f32, 0.0f32);
        apply_tick(
            Kind::InstantHealth,
            1,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert!((hp - 20.0).abs() < 1e-6);
        // 即时缩放路径（:37/:40 (int)(scale·N + 0.5) 截断）。
        let (mut hp, mut ab) = (10.0f32, 0.0f32);
        apply_instant(
            Kind::InstantHealth,
            0,
            0.5,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert!((hp - 12.0).abs() < 1e-6, "(int)(0.5*4+0.5)=2");
        harms.clear();
        apply_instant(
            Kind::InstantDamage,
            0,
            1.5,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert_eq!(
            harms.last().copied(),
            Some(Harm {
                amount: 9.0,
                wither: false
            })
        );
        // 非治疗系即时无动作。
        let (mut hp, mut ab) = (10.0f32, 0.0f32);
        apply_instant(
            Kind::Saturation,
            0,
            1.0,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert!((hp - 10.0).abs() < 1e-6 && harms.is_empty());
    }

    /// 饱和/饥饿的食物面结算（SaturationMobEffect.java:13-19 + FoodData.add:19-22
    /// + FoodConstants.java:30-32 + HungerMobEffect.java:15 + FoodData.java:100-102）。
    #[test]
    fn saturation_and_hunger_food_side() {
        let mut harms = Vec::new();
        // saturation amp0：eat(1, 1.0) → food+1、sat+1×1×2=2，sat 钳 ≤ food。
        let (mut food, mut sat, mut ex) = (19.0f32, 0.0f32, 0.0f32);
        {
            let (mut hp, mut ab) = (20.0f32, 0.0f32);
            let mut fm = FoodMut {
                hunger: &mut food,
                saturation: &mut sat,
                exhaustion: &mut ex,
            };
            apply_tick(
                Kind::Saturation,
                0,
                &mut target(&mut hp, &mut ab, Some(&mut fm), &mut harms),
            );
        }
        assert!((food - 20.0).abs() < 1e-6);
        assert!((sat - 2.0).abs() < 1e-6);
        // amp3：eat(4, 1.0) → food+4（钳 20）、sat+8 钳 ≤ food。
        let (mut food, mut sat, mut ex) = (17.0f32, 1.0f32, 0.0f32);
        {
            let (mut hp, mut ab) = (20.0f32, 0.0f32);
            let mut fm = FoodMut {
                hunger: &mut food,
                saturation: &mut sat,
                exhaustion: &mut ex,
            };
            apply_tick(
                Kind::Saturation,
                3,
                &mut target(&mut hp, &mut ab, Some(&mut fm), &mut harms),
            );
        }
        assert!((food - 20.0).abs() < 1e-6, "17+4 → 20");
        assert!((sat - 9.0).abs() < 1e-6, "1+8=9 ≤ food 20");
        // 饥饿效果：每 tick exhaustion 0.005×(amp+1)（HungerMobEffect.java:15）。
        let (mut food, mut sat, mut ex) = (20.0f32, 5.0f32, 0.0f32);
        {
            let (mut hp, mut ab) = (20.0f32, 0.0f32);
            let mut fm = FoodMut {
                hunger: &mut food,
                saturation: &mut sat,
                exhaustion: &mut ex,
            };
            apply_tick(
                Kind::Hunger,
                0,
                &mut target(&mut hp, &mut ab, Some(&mut fm), &mut harms),
            );
            apply_tick(
                Kind::Hunger,
                2,
                &mut target(&mut hp, &mut ab, Some(&mut fm), &mut harms),
            );
        }
        assert!((ex - 0.02).abs() < 1e-9, "0.005 + 0.015");
        // exhaustion 上限 40（FoodData.java:100-101）。
        let (mut food, mut sat, mut ex) = (20.0f32, 5.0f32, 39.9999f32);
        {
            let (mut hp, mut ab) = (20.0f32, 0.0f32);
            let mut fm = FoodMut {
                hunger: &mut food,
                saturation: &mut sat,
                exhaustion: &mut ex,
            };
            apply_tick(
                Kind::Hunger,
                0,
                &mut target(&mut hp, &mut ab, Some(&mut fm), &mut harms),
            );
        }
        assert!((ex - 40.0).abs() < 1e-6);
        // mob 无食物面：饥饿效果空转不 panic。
        let mut harms = Vec::new();
        let (mut hp, mut ab) = (20.0f32, 0.0f32);
        apply_tick(
            Kind::Hunger,
            1,
            &mut target(&mut hp, &mut ab, None, &mut harms),
        );
        assert!(harms.is_empty());
    }

    /// 吸收盾入场与耗尽终止（AbsorptionMobEffect.java:12-25）。
    #[test]
    fn absorption_entrance_and_expiry() {
        let mut harms = Vec::new();
        // 入场：absorb = max(current, 4×(amp+1))（:22-25）。
        let mut book = EffectBook::default();
        book.apply_simple(Kind::Absorption, 200, 0);
        let mut ab = 0.0f32;
        book.entrance(&mut ab);
        assert!((ab - 4.0).abs() < 1e-6);
        book.apply_simple(Kind::Absorption, 200, 2);
        book.entrance(&mut ab);
        assert!((ab - 12.0).abs() < 1e-6, "amp2 → 4×3");
        // 已有更高吸收不被压低（max 语义）。
        book.apply_simple(Kind::Absorption, 200, 0);
        book.entrance(&mut ab);
        assert!((ab - 12.0).abs() < 1e-6);
        // 盾耗尽 → applyEffectTick 返回 false → 账本移除（:12-14）。
        let mut ab = 0.0f32;
        let (mut hp, mut food): (f32, Option<&mut FoodMut>) = (20.0, None);
        let harms = book.tick(0, &mut hp, 20.0, &mut ab, food.as_deref_mut());
        assert!(harms.is_empty());
        assert!(!book.has(Kind::Absorption), "吸收 0 → 效果终止");
        // 盾>0 → 存续。
        book.apply_simple(Kind::Absorption, 100, 0);
        let mut ab = 4.0f32;
        let (mut hp, mut food): (f32, Option<&mut FoodMut>) = (20.0, None);
        let _ = book.tick(0, &mut hp, 20.0, &mut ab, food.as_deref_mut());
        assert!(book.has(Kind::Absorption));
    }

    /// 账本推进：时长递减、到期移除、无限时长不衰减（MobEffectInstance.java:223-261）。
    #[test]
    fn book_tick_durations() {
        let mut book = EffectBook::default();
        book.apply_simple(Kind::Speed, 5, 0);
        book.apply_simple(Kind::Regeneration, INFINITE, 1);
        let (mut hp, mut ab) = (10.0f32, 0.0f32);
        let mut food: Option<&mut FoodMut> = None;
        // 5 tick：速度到期消失；再生无限存续；期间回血 5 点
        //（50>>1=25 间隔内 phase 1..5 不触发，10+5=15 无再生增量——先验证时长）。
        for t in 0..5 {
            assert!(book.has(Kind::Speed), "tick {t}");
            let _ = book.tick(0, &mut hp, 20.0, &mut ab, food.as_deref_mut());
        }
        assert!(!book.has(Kind::Speed));
        assert!(book.has(Kind::Regeneration));
        assert!(!book.is_empty());
        // 无限效果：时长恒 −1，相位锚定世界 tick（:228）。
        let active = book.get(Kind::Regeneration).unwrap();
        assert_eq!(active.duration, INFINITE);
        // 世界 tick 49→50 跨过 50>>1=25 的相位（50/25=2 触发点）。
        let (mut hp, mut ab) = (10.0f32, 0.0f32);
        let mut food: Option<&mut FoodMut> = None;
        assert!(
            book.tick(48, &mut hp, 20.0, &mut ab, food.as_deref_mut())
                .is_empty()
        );
        assert!(
            book.tick(49, &mut hp, 20.0, &mut ab, food.as_deref_mut())
                .is_empty()
        );
        book.tick(50, &mut hp, 20.0, &mut ab, food.as_deref_mut());
        // 相位 = 世界 tick：50 % 25 == 0 → 回血 1。
        assert!((hp - 11.0).abs() < 1e-6, "tick50 回血 1");
        book.clear();
        assert!(book.is_empty());
    }

    /// 替换语义三型（MobEffectInstance.update:132-175）。
    #[test]
    fn book_update_semantics() {
        // ① 更强恒替换（时长无所谓）。
        let mut book = EffectBook::default();
        book.apply_simple(Kind::Poison, 100, 0);
        assert!(book.apply_simple(Kind::Poison, 10, 1));
        let a = book.get(Kind::Poison).unwrap();
        assert_eq!((a.amplifier, a.duration), (1, 10));
        // ② 同强度更长 → 续时。
        assert!(book.apply_simple(Kind::Poison, 50, 1));
        assert_eq!(book.get(Kind::Poison).unwrap().duration, 50);
        // ③ 更强但更短 → 旧效果下潜隐藏链，到期回退（:139-146/:263-271）。
        let mut book = EffectBook::default();
        book.apply_simple(Kind::Regeneration, 10, 0);
        assert!(book.apply_simple(Kind::Regeneration, 5, 1));
        let (mut hp, mut ab) = (10.0f32, 0.0f32);
        let mut food: Option<&mut FoodMut> = None;
        // 前 5 tick amp1（50>>1=25 间隔不触发）；隐藏链同步递减 10→5。
        for _ in 0..5 {
            book.tick(0, &mut hp, 20.0, &mut ab, food.as_deref_mut());
        }
        assert!(
            !book.has(Kind::Regeneration) || book.get(Kind::Regeneration).unwrap().amplifier == 0,
            "amp1 5t 到期回退 amp0"
        );
        let a = book.get(Kind::Regeneration).unwrap();
        assert_eq!((a.amplifier, a.duration), (0, 5), "隐藏链同步走完 10−5");
        // ④ 更弱但更长 → 存隐藏待降级，活动效果不动（:152-156）。
        let mut book = EffectBook::default();
        book.apply_simple(Kind::Speed, 10, 2);
        assert!(
            !book.apply_simple(Kind::Speed, 30, 1),
            "弱化施加不算 changed"
        );
        let a = book.get(Kind::Speed).unwrap();
        assert_eq!((a.amplifier, a.duration), (2, 10));
        // 10 tick 后弱效果接续 20 tick。
        let (mut hp, mut ab) = (20.0f32, 0.0f32);
        let mut food: Option<&mut FoodMut> = None;
        for _ in 0..10 {
            book.tick(0, &mut hp, 20.0, &mut ab, food.as_deref_mut());
        }
        let a = book.get(Kind::Speed).unwrap();
        assert_eq!((a.amplifier, a.duration), (1, 20), "降级回退弱效果余量");
    }

    /// amplifier 钳位（MobEffectInstance.java:81 Mth.clamp(0, 255)）。
    #[test]
    fn amplifier_clamped() {
        let mut book = EffectBook::default();
        book.apply_simple(Kind::Speed, 10, 200);
        assert_eq!(book.amplifier(Kind::Speed), Some(200));
    }

    /// 属性汇总面（AttributeInstance.java:147-164 合成序：先加后乘）。
    #[test]
    fn bundle_combines_like_vanilla() {
        let mut book = EffectBook::default();
        book.apply_simple(Kind::Speed, 100, 1); // ×1.4
        book.apply_simple(Kind::Slowness, 100, 1); // ×0.7
        let b = book.bundle();
        assert!((b.move_speed_mult - 1.4 * 0.7).abs() < 1e-12, "乘子相乘");
        book.apply_simple(Kind::Strength, 100, 1); // +6.0
        book.apply_simple(Kind::Weakness, 100, 1); // −8.0
        let b = book.bundle();
        assert!((b.attack_damage_add - (-2.0)).abs() < 1e-12);
        assert!((b.attack_speed_mult - 1.0).abs() < 1e-12);
        book.clear();
        book.apply_simple(Kind::JumpBoost, 100, 1); // 安全坠落 +2
        book.apply_simple(Kind::HealthBoost, 100, 1); // 生命上限 +8
        book.apply_simple(Kind::Absorption, 100, 1); // 吸收上限 +8
        let b = book.bundle();
        assert!((b.safe_fall_add - 2.0).abs() < 1e-12);
        assert!((b.max_health_add - 8.0).abs() < 1e-12);
        assert!((b.max_absorption_add - 8.0).abs() < 1e-12);
        // 空账本 = 无修饰。
        book.clear();
        assert_eq!(book.bundle(), Bundle::default());
    }

    /// 混合因子（MobEffectInstance.java:384-402）：目标判定、步长、立即态。
    #[test]
    fn blend_step_table() {
        // nausea：blend_in 150 → 每 tick +1/150（MobEffects.java:53）。
        assert!((blend_step(0.0, 600, 150, 20, 60) - 1.0 / 150.0).abs() < 1e-9);
        // 剩余 ≤ 提前量 60 → 消退目标，blend_out 20 → −1/20。
        assert!((blend_step(1.0, 60, 150, 20, 60) - (1.0 - 1.0 / 20.0)).abs() < 1e-9);
        assert!(
            (blend_step(1.0, 61, 150, 20, 60) - 1.0).abs() < 1e-9,
            "61 > 60 仍全显"
        );
        // 无混合时长 → 直接目标（MobEffect.java 无 setBlendDuration）。
        assert_eq!(blend_step(0.0, 100, 0, 0, 0), 1.0);
        assert_eq!(blend_step(1.0, 100, 0, 0, 0), 1.0);
        // darkness 22/22/22（MobEffects.java:108）。
        assert!((blend_step(0.0, 500, 22, 22, 22) - 1.0 / 22.0).abs() < 1e-9);
    }

    /// 死亡/受击爆发数据（OozingMobEffect.java:30-32 / WindChargedMobEffect.java:23）。
    #[test]
    fn death_and_hurt_bursts() {
        assert_eq!(
            death_burst(Kind::Oozing),
            Some(DeathBurst::Slimes {
                requested: 2,
                size: 2
            })
        );
        assert_eq!(
            death_burst(Kind::Weaving),
            Some(DeathBurst::Cobwebs {
                count: (2, 3),
                attempts: 15,
                radius: 1
            })
        );
        assert_eq!(death_burst(Kind::WindCharged), Some(DeathBurst::WindBurst));
        assert_eq!(death_burst(Kind::Poison), None);
        // 粘液数量钳制表（:30-32）：min(max(cramming−nearby, 0), requested)。
        assert_eq!(slimes_to_spawn(8, 3, 2), 2, "余 5 → 上限 requested");
        assert_eq!(slimes_to_spawn(8, 7, 2), 1, "余 1 → 1");
        assert_eq!(slimes_to_spawn(8, 8, 2), 0, "无余量");
        assert_eq!(slimes_to_spawn(0, 99, 2), 2, "cramming<1 不限");
        assert_eq!(slimes_to_spawn(1, 0, 2), 1);
        // 风爆威力 3.0 + rand·2.0。
        assert!((wind_burst_power(0.0) - 3.0).abs() < 1e-6);
        assert!((wind_burst_power(1.0) - 5.0).abs() < 1e-6);
        // 蠹虫受击爆发：概率 0.1、数量 1..=2。
        assert_eq!(hurt_burst(Kind::Infested), Some((0.1, 1, 2)));
        assert_eq!(hurt_burst(Kind::Oozing), None);
    }

    /// HUD 排序（MobEffectInstance.compareTo:339-352）。
    #[test]
    fn hud_ordering() {
        let short = Active::new(Kind::Speed, 100, 0, false, true, true);
        let long = Active::new(Kind::Slowness, 40_000, 0, false, true, true);
        let inf = Active::new(Kind::Haste, INFINITE, 0, false, true, true);
        let ambient = Active::new(Kind::Poison, 100, 0, true, true, true);
        // 非环境 < 环境（compareFalseFirst）。
        assert_eq!(hud_order(&short, &ambient), Ordering::Less);
        // 有限 < 无限。
        assert_eq!(hud_order(&short, &inf), Ordering::Less);
        // 短 < 长。
        assert_eq!(hud_order(&short, &long), Ordering::Less);
        // 同段（双长且非环境）只比颜色。
        let long2 = Active::new(Kind::Slowness, 50_000, 0, false, true, true);
        assert_eq!(hud_order(&long, &long2), Ordering::Equal);
    }

    /// 查询谓词族（MobEffectUtil.java:25-49）。
    #[test]
    fn effect_util_queries() {
        let mut book = EffectBook::default();
        assert!(!has_dig_speed(&book));
        assert_eq!(dig_speed_amplifier(&book), 0);
        assert!(!has_water_breathing(&book));
        assert!(refills_air_supply(&book), "无鹦鹉螺之息 → 默认回补");
        book.apply_simple(Kind::Haste, 100, 2);
        assert!(has_dig_speed(&book));
        assert_eq!(dig_speed_amplifier(&book), 2);
        book.apply_simple(Kind::ConduitPower, 100, 4);
        assert_eq!(dig_speed_amplifier(&book), 4, "两源取大（:29-41）");
        book.clear();
        book.apply_simple(Kind::BreathOfTheNautilus, 100, 0);
        assert!(has_water_breathing(&book));
        assert!(
            !refills_air_supply(&book),
            "鹦鹉螺之息独占时暂停回补（:47-49）"
        );
        book.apply_simple(Kind::WaterBreathing, 100, 0);
        assert!(refills_air_supply(&book), "叠加水呼吸恢复回补");
    }

    /// 端到端 20 tick 模拟：毒 II 每 6 tick（25>>2）掉 1、再生 II 每 12 tick
    /// （50>>2）回 1、伤害走 Harm 队列、吸收盾先于血量（管线由调用方接）。
    #[test]
    fn twenty_tick_sim() {
        let mut book = EffectBook::default();
        book.apply_simple(Kind::Poison, 20, 2);
        book.apply_simple(Kind::Regeneration, 20, 2);
        let (mut hp, mut ab) = (10.0f32, 0.0f32);
        let mut food: Option<&mut FoodMut> = None;
        let mut total_harm = 0.0f32;
        let mut heals = 0;
        for w in 0..20 {
            for h in book.tick(w, &mut hp, 20.0, &mut ab, food.as_deref_mut()) {
                total_harm += h.amount;
            }
            // 再生相位：剩余 20,19,…,1 → 12、(50>>2=12) 12%12==0 → w 使剩余=12 即 w=8。
            if book.has(Kind::Regeneration) {
                let rem = book.get(Kind::Regeneration).map_or(0, |a| a.duration);
                if rem == 11 {
                    heals += 1;
                }
            }
        }
        // 毒相位：25>>2=6 → 剩余 18,12,6 命中 → 3 次伤害。
        assert!((total_harm - 3.0).abs() < 1e-6, "毒 II 20t 掉 3 HP");
        assert_eq!(heals, 1, "再生 II 单次回血");
        assert!(book.is_empty(), "20t 双双到期");
    }
}
