//! 方块 → 掉落所需工具（#178 刀2b：全量数据驱动，手写名单已删净）。
//!
//! 三真源（均为构建期生成物，运行期零解引用）：
//!
//! - **掉落门成员资格** = GEN_BLOCKS 第 9 列 `BlockDef::requires_tool`
//!   （26.1 `Properties.requiresCorrectToolForDrops()`，
//!   BlockBehaviour.java:1213 置位 + ofFullCopy/ofLegacyCopy :1054 继承闭包，
//!   ci/gen-blocks.py 提取；原版消费 Player.java:619）。
//! - **工具种类** = `mineable/pickaxe|shovel` tag（Item.java:466-478
//!   `Properties.pickaxe/shovel → minesAndDrops(MINEABLE_WITH_*)`；
//!   tags_gen.inc.rs，#178 刀1 生成）。gate=true 方块只出现在
//!   pickaxe/shovel 两 tag（axe/hoe 零交集，锁 tests/requires_tool_gen.rs）。
//! - **最低层级** = `needs_stone/iron/diamond_tool` tag（ToolMaterial.java
//!   的 INCORRECT_FOR_* deniesDrops 门槛；本表序木/金=1、石=2、铜=3、
//!   铁=4、钻/下界合金=5，与 `mcv_item::mining::mining_tier` 一致）。
//!   不在任何 needs tag = 任意镐/铲即可（incorrect_for_* 与 needs 并集
//!   逐 tag 相等，实测登记见刀2 交卷报告）。
//!
//! 特例（原版实物）：`gate=true ∧ ∉任何 mineable tag` 的 6 方块
//! （命令块族 5 + cobweb）返回 [`Requirement::Never`]——任何工具
//! `isCorrectForDrops` 均为 false（Tool.java:50-57 无规则匹配），
//! 详见 tests/tool_gate_snapshot.rs::VANILLA_CORRECTIONS。
//!
//! 行为快照锁：tests/tool_gate_snapshot.rs（1171 id 逐位对拍切换前
//! 手写名单，白名单外漂移即红）。

use crate::{BLOCKS, BlockId, tags};

/// 掉落所需工具种类（26.1 `mineable/pickaxe|shovel`——掉落门方块在
/// axe/hoe tag 零成员，26.1 实物锁定，故不扩变体）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolReq {
    Pickaxe,
    Shovel,
}

/// 掉落门要求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// 需要该种类 + 最低层级（`mcv_item::mining::mining_tier` 序）的工具。
    Tool { kind: ToolReq, tier: u8 },
    /// 原版 gate=true 且无任何 mineable tag：`Tool.isCorrectForDrops`
    /// 对一切工具返回 false（Tool.java:50-57），掉落只能走引擎未建模的
    /// 专用 Tool 组件（剪刀/剑）或根本无路（命令块族）。速度侧无
    /// minesAndDrops 规则 = 恒默认速度。
    Never,
}

/// 查询方块的掉落工具要求（按 [`BlockId`]，tag/生成列 id 域 = GEN_BLOCKS 行序）。
///
/// 返回 `None` = 无掉落门（26.1 无 `requiresCorrectToolForDrops` 属性，
/// 空手即"正确工具"，Player.java:619 `!requiresCorrectToolForDrops()`）。
pub fn requirement_by_id(id: BlockId) -> Option<Requirement> {
    let row = id.id();
    let def = BLOCKS.get(row as usize)?;
    if !def.requires_tool {
        return None;
    }
    // needs tag 互斥（stone<iron<diamond 层级链），序只影响效率不影响结果。
    let tier = if tags::TAG_BLOCK_NEEDS_DIAMOND_TOOL.set.contains(row) {
        5
    } else if tags::TAG_BLOCK_NEEDS_IRON_TOOL.set.contains(row) {
        4
    } else if tags::TAG_BLOCK_NEEDS_STONE_TOOL.set.contains(row) {
        2
    } else {
        1
    };
    if tags::TAG_BLOCK_MINEABLE__PICKAXE.set.contains(row) {
        Some(Requirement::Tool {
            kind: ToolReq::Pickaxe,
            tier,
        })
    } else if tags::TAG_BLOCK_MINEABLE__SHOVEL.set.contains(row) {
        Some(Requirement::Tool {
            kind: ToolReq::Shovel,
            tier,
        })
    } else {
        Some(Requirement::Never)
    }
}

/// 按注册名查（等价 `requirement_by_id`，供名字入手的旧调用/测试使用；
/// 未注册名 = `None`）。
pub fn requirement(name: &str) -> Option<Requirement> {
    let row = BLOCKS.iter().position(|b| b.name == name)?;
    requirement_by_id(BlockId(row as u16))
}

#[cfg(test)]
mod tests {
    use super::{Requirement, ToolReq, requirement};

    fn t(name: &str) -> Option<(ToolReq, u8)> {
        match requirement(name) {
            Some(Requirement::Tool { kind, tier }) => Some((kind, tier)),
            Some(Requirement::Never) => None,
            None => None,
        }
    }

    /// 矿石/深板岩矿石族的层级门（26.1 needs_*_tool tag + Blocks.java 继承）。
    #[test]
    fn ore_families_gate_by_tier() {
        // 任意镐：煤矿族。
        assert_eq!(t("coal_ore"), Some((ToolReq::Pickaxe, 1)));
        assert_eq!(t("deepslate_coal_ore"), Some((ToolReq::Pickaxe, 1)));
        // 石镐级（needs_stone_tool）：铁/铜/青金 + 深板岩变体。
        for name in [
            "iron_ore",
            "deepslate_iron_ore",
            "copper_ore",
            "deepslate_copper_ore",
            "lapis_ore",
            "deepslate_lapis_ore",
        ] {
            assert_eq!(t(name), Some((ToolReq::Pickaxe, 2)), "{name} 石镐级");
        }
        // 铁镐级（needs_iron_tool）：金/红石/钻石/绿宝石 + 深板岩变体。
        for name in [
            "gold_ore",
            "deepslate_gold_ore",
            "redstone_ore",
            "deepslate_redstone_ore",
            "diamond_ore",
            "deepslate_diamond_ore",
            "emerald_ore",
            "deepslate_emerald_ore",
        ] {
            assert_eq!(t(name), Some((ToolReq::Pickaxe, 4)), "{name} 铁镐级");
        }
        // 钻镐级（needs_diamond_tool）：黑曜石族/远古残骸。
        for name in ["obsidian", "crying_obsidian", "ancient_debris"] {
            assert_eq!(t(name), Some((ToolReq::Pickaxe, 5)), "{name} 钻镐级");
        }
    }

    /// 石族任意镐；泥土/沙/木/叶/玻璃/砂砾/耕地不需工具；雪走铲。
    #[test]
    fn stone_family_and_tool_free_blocks() {
        for name in [
            "stone",
            "cobble",
            "deepslate",
            "cobbled_deepslate",
            "netherrack",
        ] {
            assert_eq!(t(name), Some((ToolReq::Pickaxe, 1)), "{name} 任意镐");
        }
        for name in [
            "dirt", "grass", "sand", "gravel", "glass", "farmland", "leaves", "log", "planks",
        ] {
            assert_eq!(t(name), None, "{name} 不需工具");
        }
        assert_eq!(t("snow"), Some((ToolReq::Shovel, 1)));
        assert_eq!(t("snow_block"), Some((ToolReq::Shovel, 1)));
    }

    /// 刀2b 原版修正代表：命令块族/cobweb = Never（任何工具不正确）；
    /// 铜栏杆/链 = 新入门任意镐。全集逐位锁在 tests/tool_gate_snapshot.rs。
    #[test]
    fn never_requirement_matches_vanilla() {
        for name in [
            "command_block",
            "chain_command_block",
            "repeating_command_block",
            "structure_block",
            "jigsaw",
            "cobweb",
        ] {
            assert_eq!(
                requirement(name),
                Some(Requirement::Never),
                "{name} 原版 = 任何工具都不正确"
            );
        }
        assert_eq!(
            t("copper_bars"),
            Some((ToolReq::Pickaxe, 1)),
            "copper_bars 属性带掉落门（Blocks.java:1934）"
        );
    }
}
