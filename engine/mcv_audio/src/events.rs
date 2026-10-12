//! 已接线「非方块」音效的 26.1 事件名中心表。
//!
//! 调用点（mcv_logic/game.rs、mcv_app/app.rs）一律引用本表常量、不写
//! 字面量——改名/对齐 26.1 只动这一处。**完备性测试**
//! `tests/events_table.rs` 保证：映射键无重、映射音名 ⊆ [`WIRED`]、
//! [`WIRED`] 与映射表集合相等，且每个音名都存在于原版
//! `sounds.json`（资产索引 index 30；CI test job fetch 后真校验）。
//!
//! 方块音效族（SoundType 的 dig/step/place/break/fall/hit 字段）不走本表
//! ——那是 #136 波的数据驱动全表（`sound_types.inc.rs`，gen-soundtypes.py
//! 生成），事件名由方块材质派生，无字面量调用点。
//!
//! 26.1 唯一权威：SoundEvents.java 常量行号为 /root/mc-ref/src-26.1。

/// 玩家受伤（SoundEvents.java:1286 `PLAYER_HURT`；消费点
/// LivingEntity.java:2019 `playSound(hurtSound, …)`）。
pub const PLAYER_HURT: &str = "entity.player.hurt";

/// 入水溅落低速档（SoundEvents.java:1293 `PLAYER_SPLASH`；消费点
/// Entity.doWaterSplashEffect Entity.java:1594-1622，速度门在调用点）。
pub const PLAYER_SPLASH: &str = "entity.player.splash";

/// 入水溅落高速档（SoundEvents.java:1294；消费点同上）。
pub const PLAYER_SPLASH_HIGH_SPEED: &str = "entity.player.splash.high_speed";

/// 摔落伤害 >4 档（SoundEvents.java:1282 `PLAYER_BIG_FALL`；消费点
/// LivingEntity.java:1772 `playSound(getFallDamageSound(dmg), 1.0F, 1.0F)`，
/// 档位门 getFallDamageSound LivingEntity.java:2573）。
pub const PLAYER_FALL_BIG: &str = "entity.player.big_fall";

/// 摔落伤害 ≤4 档（SoundEvents.java:1292；消费点同上）。
pub const PLAYER_FALL_SMALL: &str = "entity.player.small_fall";

/// 工具/装备耐久耗尽破碎（SoundEvents.java:897 `ITEM_BREAK`；
/// 默认 BREAK_SOUND 组件 DataComponents.java:423，消费点
/// LivingEntity.breakItem LivingEntity.java:1412-1420：
/// volume 0.8、pitch 0.8 + random×0.4）。
///
/// **改名对齐（本波修正）**：旧接线写的是 1.20 前的 `random.break`——26.1
/// 的 sounds.json（index 30）已无该事件，旧调用实为静默死音。
pub const ITEM_BREAK: &str = "entity.item.break";

/// 进食咀嚼/完食（SoundEvents.java:691 `GENERIC_EAT`；默认 Consumable
/// sound 字段 Consumable.java:42，消费点 emitParticlesAndSounds
/// Consumable.java:104-117：音量 nextBoolean ? 0.5 : 1.0（:105，EAT 分支
/// :109）、音高 triangle(1.0, 0.2)（:106；RandomSource.java:63-65）。
/// 节拍门 shouldEmitParticlesAndSounds :119-124（已吃 > (int)(total×0.21875)
/// 且 remaining % 4 == 0），咀嚼拍经 LivingEntity.updateUsingItem :3408 →
/// ItemStack.onUseTick（ItemStack.java:1094-1098，count=5）；完食拍经
/// Consumable.onConsume :77-79（count=16）。本仓无 DRINK 动画食物，饮音
/// 分支不接）。
pub const GENERIC_EAT: &str = "entity.generic.eat";

/// 弓放箭（SoundEvents.java:111 `ARROW_SHOOT`；消费点
/// BowItem.releaseUsing BowItem.java:46-55：volume 1.0、pitch =
/// 1.0/(rand×0.4 + 1.2) + pow×0.5）。
pub const ARROW_SHOOT: &str = "entity.arrow.shoot";

/// 掉落物拾取确认（SoundEvents.java:898 `ITEM_PICKUP`；消费点客户端
/// take 实体包处理 ClientPacketListener.java:1002-1013：volume 0.2、
/// pitch (rand − rand)×1.4 + 2.0，位置 = 被拾实体）。经验球同型音在
/// :995（EXPERIENCE_ORB_PICKUP，volume 0.1、pitch (rand − rand)×0.35 +
/// 0.9）——本仓无经验球实体（击杀经验直接入账），留债不接。
pub const ITEM_PICKUP: &str = "entity.item.pickup";

/// UI 按钮按下（SoundEvents.java:1636 `UI_BUTTON_CLICK`；消费点
/// AbstractWidget.mouseClicked :106-118 命中即 playDownSound →
/// playButtonClickSound :163-166 `forUI(UI_BUTTON_CLICK, 1.0F)`；
/// 世界列表行同音 WorldSelectionList.java:579/:594；滑杆松开同音
/// AbstractSliderButton.onRelease :153-155）。app.rs 菜单/按钮接线。
pub const UI_BUTTON_CLICK: &str = "ui.button.click";

/// 全部已接事件名（无重复；与 `tests/events_table.rs` 的映射表集合相等）。
pub const WIRED: &[&str] = &[
    PLAYER_HURT,
    PLAYER_SPLASH,
    PLAYER_SPLASH_HIGH_SPEED,
    PLAYER_FALL_BIG,
    PLAYER_FALL_SMALL,
    ITEM_BREAK,
    GENERIC_EAT,
    ARROW_SHOOT,
    ITEM_PICKUP,
    UI_BUTTON_CLICK,
];
