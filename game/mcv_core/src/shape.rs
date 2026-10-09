//! 方块形状表：id → 形状（cross/火把/栅栏/半砖/楼梯/立方）。
//!
//! 形状分类是**按注册名生成的逻辑**：规则与 ci/gen-blocks.py 的
//! `shape_of_name()` 和 C++ 侧 `blocks_gen.inc` 的 `shape` 字段逐字对齐
//! （三处同源、纯名字派生，无 Mojang 代码；几何常量为原版数值）。
//! 修改任何一侧规则必须同步另外两侧，否则 Rust 逻辑/C++ 渲染会漂移。

/// 方块形状（值写入 `BlockDef::shape` / C++ `BlockInfo::shape`）。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Cube = 0,
    Cross = 1,
    Torch = 2,
    Fence = 3,
    Slab = 4,
    Stairs = 5,
}

impl Shape {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Cross,
            2 => Self::Torch,
            3 => Self::Fence,
            4 => Self::Slab,
            5 => Self::Stairs,
            _ => Self::Cube,
        }
    }
}

/// 十字植物（花草类）精确名单；与 gen-blocks.py `CROSS_PLANTS` 一致。
/// 注：官方短草 blockstate 名 "grass" 与旧 id 3（草方块，本引擎旧名同名）
/// 冲突，无法按名区分，故不入名单（保持 Cube 整盒渲染）。
const CROSS_PLANTS: [&str; 18] = [
    "flower_red",
    "flower_yellow",
    "allium",
    "azure_bluet",
    "blue_orchid",
    "cornflower",
    "lily_of_the_valley",
    "oxeye_daisy",
    "torchflower",
    "torchflower_crop",
    "wither_rose",
    "short_grass",
    "fern",
    "tall_grass",
    "large_fern",
    "rose_bush",
    "pink_petals",
    "wildflowers",
];

const fn eq_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let mut j = 0;
        while j < needle.len() && hay[i + j] == needle[j] {
            j += 1;
        }
        if j == needle.len() {
            return true;
        }
        i += 1;
    }
    false
}

const fn ends_with(hay: &[u8], suf: &[u8]) -> bool {
    if suf.len() > hay.len() {
        return false;
    }
    let mut i = 0;
    while i < suf.len() {
        if hay[hay.len() - suf.len() + i] != suf[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// 名字 → 形状编号（`Shape` 的 u8 判别值）。const 供 `gen_def` 建表期使用。
/// 规则顺序（与 python 版逐字一致）：
/// `potted_*` → Cube（盆栽装饰，非植物本体）；含 `fence` 且不含 `gate` →
/// Fence；含 `slab` → Slab；含 `stairs` → Stairs；精确花草名或 `sapling`
/// 后缀 → Cross；含 `torch` 且不含 `wall` → Torch；否则 Cube。
pub const fn shape_of_name(name: &str) -> u8 {
    let n = name.as_bytes();
    if contains(n, b"potted") {
        return Shape::Cube as u8;
    }
    if contains(n, b"fence") && !contains(n, b"gate") {
        return Shape::Fence as u8;
    }
    if contains(n, b"slab") {
        return Shape::Slab as u8;
    }
    if contains(n, b"stairs") {
        return Shape::Stairs as u8;
    }
    let mut i = 0;
    while i < CROSS_PLANTS.len() {
        if eq_bytes(n, CROSS_PLANTS[i].as_bytes()) {
            return Shape::Cross as u8;
        }
        i += 1;
    }
    if ends_with(n, b"sapling") {
        return Shape::Cross as u8;
    }
    if contains(n, b"torch") && !contains(n, b"wall") {
        return Shape::Torch as u8;
    }
    Shape::Cube as u8
}

/// 体素值（含状态位）→ 形状；未注册 id 回退 Cube。
#[inline]
pub fn shape(id: u16) -> Shape {
    let real = (id & crate::ID_MASK) as usize;
    if real < crate::BLOCKS.len() {
        Shape::from_u8(crate::BLOCKS[real].shape)
    } else {
        Shape::Cube
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rules() {
        assert_eq!(shape_of_name("stone"), 0);
        assert_eq!(shape_of_name("grass"), 0); // 旧草方块名，非短草
        assert_eq!(shape_of_name("short_grass"), 1);
        assert_eq!(shape_of_name("flower_red"), 1);
        assert_eq!(shape_of_name("oak_sapling"), 1);
        assert_eq!(shape_of_name("torch"), 2);
        assert_eq!(shape_of_name("soul_wall_torch"), 0); // wall 变体保持 Cube
        assert_eq!(shape_of_name("oak_fence"), 3);
        assert_eq!(shape_of_name("oak_fence_gate"), 0);
        assert_eq!(shape_of_name("acacia_slab"), 4);
        assert_eq!(shape_of_name("bamboo_mosaic_stairs"), 5);
        assert_eq!(shape_of_name("potted_oak_sapling"), 0);
        assert_eq!(shape_of_name("torchflower"), 1); // 花草名单优先于 torch 词根
    }
}
