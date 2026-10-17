//! 死亡掉落表（LootTable 简化，参照 NOTES-mobs.md §6）。
//!
//! 26.1 的 loot json 未随反编译提取（mc-ref/data 仅含 worldgen），
//! 数量取经典值（各怪 uniform[min,max]）。掉落不直接生成实体，
//! 以 [`DropEvent`] 枚举输出，由主控接线（生成掉落物实体 / 播放音效）。

use crate::defs::MobKind;
use glam::Vec3;

/// 一条掉落记录（主控据此生成 ItemEntity）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropEvent {
    /// 物品内核名（"rotten_flesh" 等，对应原版 loot table id）。
    pub item: &'static str,
    pub count: u32,
    pub pos: Vec3,
    /// 变体元数据透传（TropicalFish：packed variant 全量——pattern+底色+
    /// 花纹色，TropicalFish.packVariant；其余种恒 0）。物品栈组件持久化
    /// 待 mcv_item inventory schema 扩容（ AquaticSupp 模块头注）。
    pub variant: u32,
}

/// uniform[min, max]（含端点），由外部 rng 驱动。
fn uniform(min: u32, max: u32, rng: &mut impl FnMut() -> u32) -> u32 {
    min + rng() % (max - min + 1)
}

/// 鱼类掉落统一面（entity/{cod,salmon,tropical_fish}.json）：生鱼 ×1 +
/// 骨粉 5%（random_chance 0.05）。美西螈无 loot 表 → 恒空。
fn fish_drops(
    item: &'static str,
    variant: u32,
    pos: Vec3,
    rng: &mut impl FnMut() -> u32,
) -> Vec<DropEvent> {
    let mut out = vec![DropEvent {
        item,
        count: 1,
        pos,
        variant,
    }];
    if rng() % 100 < 5 {
        out.push(DropEvent {
            item: "bone_meal",
            count: 1,
            pos,
            variant: 0,
        });
    }
    out
}

/// 击杀掉落。`killed_by_player`：spider_eye 仅玩家击杀掉落（原版条件简化为
/// "玩家/驯服狼击杀"，这里取玩家）。难度不影响数量（26.1 四怪 loot 无难度项）。
pub fn death_drops(
    kind: MobKind,
    killed_by_player: bool,
    pos: Vec3,
    rng: &mut impl FnMut() -> u32,
) -> Vec<DropEvent> {
    death_drops_variant(kind, 0, killed_by_player, pos, rng)
}

/// 带变体的掉落（水生怪：TropicalFish 的 packed variant 随掉落透传）。
pub fn death_drops_variant(
    kind: MobKind,
    variant: u32,
    killed_by_player: bool,
    pos: Vec3,
    rng: &mut impl FnMut() -> u32,
) -> Vec<DropEvent> {
    let mut out = Vec::new();
    let push = |item: &'static str, count: u32, out: &mut Vec<DropEvent>| {
        if count > 0 {
            out.push(DropEvent {
                item,
                count,
                pos,
                variant: 0,
            });
        }
    };
    match kind {
        // entity/zombie.json：rotten_flesh 0–2（装备/胡萝卜/马铃薯/铁锭特殊项略）。
        MobKind::Zombie => push("rotten_flesh", uniform(0, 2, rng), &mut out),
        // entity/skeleton.json：bone 0–2、arrow 0–2（弓为装备掉落，接线侧处理）。
        MobKind::Skeleton => {
            push("bone", uniform(0, 2, rng), &mut out);
            push("arrow", uniform(0, 2, rng), &mut out);
        }
        // entity/creeper.json：gunpowder 0–2；唱片/头颅为特殊事件，略。
        MobKind::Creeper => push("gunpowder", uniform(0, 2, rng), &mut out),
        // entity/spider.json：string 0–2；spider_eye 0–1 仅玩家击杀。
        MobKind::Spider => {
            push("string", uniform(0, 2, rng), &mut out);
            if killed_by_player {
                push("spider_eye", uniform(0, 1, rng), &mut out);
            }
        }
        // 水生：生鱼 ×1 + 骨粉 5%（鱼 loot 表经典值）；美西螈无 loot 表。
        MobKind::Cod => out = fish_drops("cod", 0, pos, rng),
        MobKind::Salmon => out = fish_drops("salmon", 0, pos, rng),
        MobKind::TropicalFish => out = fish_drops("tropical_fish", variant, pos, rng),
        MobKind::Axolotl => {}
        // 被动怪：raw 肉类属后续内容，返回空。
        _ => {}
    }
    out
}
