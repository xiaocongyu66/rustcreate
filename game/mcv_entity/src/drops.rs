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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(events: &[DropEvent]) -> Vec<&'static str> {
        events.iter().map(|e| e.item).collect()
    }

    /// 对拍（data/minecraft/loot_table/entities/{cod,salmon}.json）：
    /// 池 1 = 本种生鱼 ×1（rolls 1.0，furnace_smelt 仅燃烧/火焰附加——
    /// 引擎无死亡燃烧面，不挂）；池 2 = bone_meal（random_chance 0.05）。
    #[test]
    fn cod_salmon_loot_parity() {
        let pos = Vec3::ZERO;
        // rng 恒 ≥5 → 骨粉 5% 池不中：只有本种 ×1。
        let mut never = || 42u32;
        assert_eq!(
            names(&death_drops(MobKind::Cod, false, pos, &mut never)),
            ["cod"]
        );
        assert_eq!(
            names(&death_drops(MobKind::Salmon, false, pos, &mut never)),
            ["salmon"]
        );
        // rng 恒 0 → 0%100 < 5：骨粉池必中（同池序：生鱼在前）。
        let mut always = || 0u32;
        assert_eq!(
            names(&death_drops(MobKind::Cod, false, pos, &mut always)),
            ["cod", "bone_meal"]
        );
        assert_eq!(
            names(&death_drops(MobKind::Salmon, false, pos, &mut always)),
            ["salmon", "bone_meal"]
        );
    }

    /// 热带鱼：loot 表唯一条目 tropical_fish（set_count 1），变体
    /// packed 全量透传到 [`DropEvent::variant`]（26.1 物品组件侧持久化
    /// 待 mcv_item inventory schema 扩容——模块头 KNOWN-DIVERGENCE）；
    /// cod/salmon 事件变体恒 0（无变体种）。
    #[test]
    fn tropical_fish_variant_passes_through() {
        let pos = Vec3::ZERO;
        let mut never = || 42u32;
        for v in [
            crate::aquatic::tropical::DEFAULT,
            crate::aquatic::tropical::COMMON_PACKED[0],
            crate::aquatic::tropical::pack(5, true, 14, 11),
        ] {
            let ev = death_drops(MobKind::TropicalFish, v, false, pos, &mut never);
            assert_eq!(names(&ev), ["tropical_fish"]);
            assert_eq!(ev[0].count, 1, "set_count 1.0（tropical_fish.json）");
            assert_eq!(ev[0].variant, v, "packed variant 原样透传");
        }
        let cod = death_drops(MobKind::Cod, 7, false, pos, &mut never);
        assert_eq!(cod[0].variant, 0, "无变体种恒 0");
    }

    /// 美西螈：loot 表无 pools（data/.../entities/axolotl.json 只有
    /// random_sequence 键）——恒空，掉任何东西都算错。
    #[test]
    fn axolotl_drops_nothing() {
        let pos = Vec3::ZERO;
        let mut always = || 0u32;
        assert!(death_drops(MobKind::Axolotl, 0, true, pos, &mut always).is_empty());
    }
}
