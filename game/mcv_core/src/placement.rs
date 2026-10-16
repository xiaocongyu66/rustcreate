//! 放置状态属性门（26.1 getPlacementState 家族）：AXIS 属性逐块表。
//!
//! 26.1 语义：放置状态默认 **identity**——`BlockItem.getPlacementState`
//! （BlockItem.java:115-118）直接取 `getBlock().getStateForPlacement(context)`，
//! 而基类实现恒返回 `defaultBlockState()`（Block.java:465-467）；只有**覆写**
//! `getStateForPlacement` 的方块族才有放置朝向属性：
//! - RotatedPillarBlock 族：`AXIS = context.getClickedFace().getAxis()`
//!   （RotatedPillarBlock.java:54-56）——本模块的逐块表；
//! - `*_stairs`：FACING = 视线水平同向 + HALF 按命中面（StairBlock.java:
//!   97-105），消费侧见 mcv_logic `placement_state` 与网格器 emit_stairs；
//! - `*_slab`：HALF 按命中面（SlabBlock.java:68-80），同上。
//!
//! 名单是**逐块表驱动**（不按名后缀猜）：`melon_stem`/`pumpkin_stem`/
//! `attached_*_stem` 名含 "stem" 却不是 RotatedPillarBlock（BushBlock 系，
//! Blocks.java 注册行 `StemBlock::new`/`AttachedStemBlock::new`），后缀规则
//! 会把无 AXIS 属性的方块误套朝向；表则与 Blocks.java 类属一一对应。

use crate::GEN_BLOCKS;

/// 状态 nibble（bit12-15）里 axis 属性的编码（RotatedPillarBlock 族占用
/// bit0-1；与板 bit0 / 楼梯 bit0-1+bit2 按形状互斥，族间不重叠）。
/// 默认态 AXIS=Y（RotatedPillarBlock.java:23 `setValue(AXIS, Axis.Y)`）且
/// worldgen 原木全部竖置（mcv_worldgen trees.rs TREES-1），Y 编 0 使竖置
/// 体素值 = 纯 id，与旧数据逐位兼容。
pub const AXIS_Y: u8 = 0;
pub const AXIS_X: u8 = 1;
pub const AXIS_Z: u8 = 2;

/// 26.1 RotatedPillarBlock 类属注册名全表（Blocks.java 全部
/// `RotatedPillarBlock::new` 注册行，56 名；来源 grep Blocks.java
/// `-B1 RotatedPillarBlock::new`）。本仓注册表无官方名 `oak_log`——
/// 旧 14 行 `log` 即其对应块（ci/gen-blocks.py LEGACY 第 2 字段映射），
/// 故表内以 `log` 替换 `oak_log` 条目，其余 55 名照抄官方。
const PILLAR_NAMES: [&str; 56] = [
    "acacia_log",
    "acacia_wood",
    "bamboo_block",
    "basalt",
    "birch_log",
    "birch_wood",
    "bone_block",
    "cherry_log",
    "cherry_wood",
    "crimson_hyphae",
    "crimson_stem",
    "dark_oak_log",
    "dark_oak_wood",
    "deepslate",
    "jungle_log",
    "jungle_wood",
    "log", // 旧名 = 官方 oak_log（gen-blocks.py LEGACY 映射）
    "mangrove_log",
    "mangrove_wood",
    "muddy_mangrove_roots",
    "oak_wood",
    "ochre_froglight",
    "pale_oak_log",
    "pale_oak_wood",
    "pearlescent_froglight",
    "polished_basalt",
    "purpur_pillar",
    "quartz_pillar",
    "spruce_log",
    "spruce_wood",
    "stripped_acacia_log",
    "stripped_acacia_wood",
    "stripped_bamboo_block",
    "stripped_birch_log",
    "stripped_birch_wood",
    "stripped_cherry_log",
    "stripped_cherry_wood",
    "stripped_crimson_hyphae",
    "stripped_crimson_stem",
    "stripped_dark_oak_log",
    "stripped_dark_oak_wood",
    "stripped_jungle_log",
    "stripped_jungle_wood",
    "stripped_mangrove_log",
    "stripped_mangrove_wood",
    "stripped_oak_log",
    "stripped_oak_wood",
    "stripped_pale_oak_log",
    "stripped_pale_oak_wood",
    "stripped_spruce_log",
    "stripped_spruce_wood",
    "stripped_warped_hyphae",
    "stripped_warped_stem",
    "verdant_froglight",
    "warped_hyphae",
    "warped_stem",
];

const fn name_eq(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    if x.len() != y.len() {
        return false;
    }
    let mut i = 0usize;
    while i < x.len() {
        if x[i] != y[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// 注册名 → 有 AXIS 属性（const 供 [`HAS_AXIS`] 建表期使用）。
pub const fn has_axis_state(name: &str) -> bool {
    let mut i = 0usize;
    while i < PILLAR_NAMES.len() {
        if name_eq(name, PILLAR_NAMES[i]) {
            return true;
        }
        i += 1;
    }
    false
}

/// 全 1171 块 AXIS 属性表（RotatedPillarBlock 类属）。越界 id → false。
#[inline]
pub fn has_axis(id: u16) -> bool {
    let real = (id & crate::ID_MASK) as usize;
    real < crate::BLOCKS.len() && HAS_AXIS[real]
}

/// 放置状态 nibble 的 AXIS 段：命中面轴（RotatedPillarBlock.java:55
/// `context.getClickedFace().getAxis()`——DDA 命中法线恒单位轴向量，
/// 非零分量即面轴）。
#[inline]
pub const fn axis_from_clicked_face(normal: [i32; 3]) -> u8 {
    if normal[0] != 0 {
        AXIS_X
    } else if normal[1] != 0 {
        AXIS_Y
    } else {
        AXIS_Z
    }
}

pub static HAS_AXIS: [bool; GEN_BLOCKS.len()] = {
    let mut t = [false; GEN_BLOCKS.len()];
    let mut i = 0usize;
    while i < GEN_BLOCKS.len() {
        t[i] = has_axis_state(GEN_BLOCKS[i].0);
        i += 1;
    }
    t
};

#[cfg(test)]
mod tests {
    use super::*;

    fn id_of(name: &str) -> u16 {
        crate::BLOCKS
            .iter()
            .position(|b| b.name == name)
            .unwrap_or_else(|| panic!("registry lacks {name}")) as u16
    }

    /// 表与 26.1 类属对拍（Prohibited 判据双向）：RotatedPillarBlock 族
    /// 全部有 AXIS；后缀近亲（stem 种植族/普通石材）一律没有——
    /// 「朝向只在有该 property 的方块生效」的门本身即受测对象。
    #[test]
    fn axis_property_gate_matches_official_class() {
        for n in [
            "log",
            "acacia_log",
            "spruce_log",
            "stripped_oak_log",
            "oak_wood",
            "stripped_birch_wood",
            "crimson_stem",
            "warped_stem",
            "stripped_crimson_hyphae",
            "deepslate",
            "basalt",
            "polished_basalt",
            "bone_block",
            "quartz_pillar",
            "purpur_pillar",
            "bamboo_block",
            "muddy_mangrove_roots",
            "ochre_froglight",
            "pearlescent_froglight",
            "verdant_froglight",
        ] {
            assert!(has_axis(id_of(n)), "{n} 应有 AXIS 属性");
        }
        for n in [
            "stone",
            "dirt",
            "furnace",
            "melon_stem",
            "pumpkin_stem",
            "attached_melon_stem",
            "attached_pumpkin_stem",
            "oak_stairs",
            "oak_slab",
        ] {
            assert!(!has_axis(id_of(n)), "{n} 不应有 AXIS 属性");
        }
    }

    /// 类属名单与官方注册行数锁定（Blocks.java 56 处
    /// `RotatedPillarBlock::new`；表内 oak_log→log 名单替换不改计数），
    /// 且注册表内逐名命中（名单漂移即红）。
    #[test]
    fn pillar_table_size_and_registry_coverage() {
        assert_eq!(PILLAR_NAMES.len(), 56);
        let mut hits = 0usize;
        for b in crate::BLOCKS.iter() {
            if has_axis_state(b.name) {
                assert!(
                    PILLAR_NAMES.contains(&b.name),
                    "{} 命中表却不在名单",
                    b.name
                );
                hits += 1;
            }
        }
        assert_eq!(hits, 56, "注册表内 AXIS 属性方块数应与类属名单相等");
    }
}
