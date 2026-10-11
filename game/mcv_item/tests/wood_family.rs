//! 木族物品+蜂窝道具注册表测试（W 波，任务板 #137）。
//!
//! 对账基准（仓库外 /root/mc-ref/src-26.1）：
//! - `data/minecraft/recipe/<name>.json`（配方数值，见 crafting_wood.rs）
//! - `blocks_gen.inc.rs`（ci/gen-blocks.py 从官方 blockstates 生成，id 主键）
//! - `assets/minecraft/textures/item/*.png`（原版 item 贴图，图标层）
//!
//! oak 族的 log/planks 沿用旧 14 方块时代物品（LOG=31 块 6 "log"、PLANKS=26
//! 块 8 "planks"，tiles 即 oak_log*/oak_planks），不新造重复物品；其余五树种
//! 与全部stairs/slab/fence/gate/door/trapdoor/button/plate 按 vanilla 名新增。

use mcv_item::ItemKind;

/// 新增物品常量表：(常量, 注册名)。追加序 = id 序列化契约（同
/// items.rs::item_family_const_order_lock 锁法），id 从 90 起尾插。
const NEW_ITEMS: &[(u16, &str)] = &[
    // 五树种 planks（oak = 旧 PLANKS=26）
    (mcv_item::SPRUCE_PLANKS, "spruce_planks"),
    (mcv_item::BIRCH_PLANKS, "birch_planks"),
    (mcv_item::JUNGLE_PLANKS, "jungle_planks"),
    (mcv_item::ACACIA_PLANKS, "acacia_planks"),
    (mcv_item::DARK_OAK_PLANKS, "dark_oak_planks"),
    // stairs ×6
    (mcv_item::OAK_STAIRS, "oak_stairs"),
    (mcv_item::SPRUCE_STAIRS, "spruce_stairs"),
    (mcv_item::BIRCH_STAIRS, "birch_stairs"),
    (mcv_item::JUNGLE_STAIRS, "jungle_stairs"),
    (mcv_item::ACACIA_STAIRS, "acacia_stairs"),
    (mcv_item::DARK_OAK_STAIRS, "dark_oak_stairs"),
    // slab ×6
    (mcv_item::OAK_SLAB, "oak_slab"),
    (mcv_item::SPRUCE_SLAB, "spruce_slab"),
    (mcv_item::BIRCH_SLAB, "birch_slab"),
    (mcv_item::JUNGLE_SLAB, "jungle_slab"),
    (mcv_item::ACACIA_SLAB, "acacia_slab"),
    (mcv_item::DARK_OAK_SLAB, "dark_oak_slab"),
    // fence ×6
    (mcv_item::OAK_FENCE, "oak_fence"),
    (mcv_item::SPRUCE_FENCE, "spruce_fence"),
    (mcv_item::BIRCH_FENCE, "birch_fence"),
    (mcv_item::JUNGLE_FENCE, "jungle_fence"),
    (mcv_item::ACACIA_FENCE, "acacia_fence"),
    (mcv_item::DARK_OAK_FENCE, "dark_oak_fence"),
    // fence_gate ×6
    (mcv_item::OAK_FENCE_GATE, "oak_fence_gate"),
    (mcv_item::SPRUCE_FENCE_GATE, "spruce_fence_gate"),
    (mcv_item::BIRCH_FENCE_GATE, "birch_fence_gate"),
    (mcv_item::JUNGLE_FENCE_GATE, "jungle_fence_gate"),
    (mcv_item::ACACIA_FENCE_GATE, "acacia_fence_gate"),
    (mcv_item::DARK_OAK_FENCE_GATE, "dark_oak_fence_gate"),
    // door ×6
    (mcv_item::OAK_DOOR, "oak_door"),
    (mcv_item::SPRUCE_DOOR, "spruce_door"),
    (mcv_item::BIRCH_DOOR, "birch_door"),
    (mcv_item::JUNGLE_DOOR, "jungle_door"),
    (mcv_item::ACACIA_DOOR, "acacia_door"),
    (mcv_item::DARK_OAK_DOOR, "dark_oak_door"),
    // trapdoor ×6
    (mcv_item::OAK_TRAPDOOR, "oak_trapdoor"),
    (mcv_item::SPRUCE_TRAPDOOR, "spruce_trapdoor"),
    (mcv_item::BIRCH_TRAPDOOR, "birch_trapdoor"),
    (mcv_item::JUNGLE_TRAPDOOR, "jungle_trapdoor"),
    (mcv_item::ACACIA_TRAPDOOR, "acacia_trapdoor"),
    (mcv_item::DARK_OAK_TRAPDOOR, "dark_oak_trapdoor"),
    // button ×6
    (mcv_item::OAK_BUTTON, "oak_button"),
    (mcv_item::SPRUCE_BUTTON, "spruce_button"),
    (mcv_item::BIRCH_BUTTON, "birch_button"),
    (mcv_item::JUNGLE_BUTTON, "jungle_button"),
    (mcv_item::ACACIA_BUTTON, "acacia_button"),
    (mcv_item::DARK_OAK_BUTTON, "dark_oak_button"),
    // pressure_plate ×6
    (mcv_item::OAK_PRESSURE_PLATE, "oak_pressure_plate"),
    (mcv_item::SPRUCE_PRESSURE_PLATE, "spruce_pressure_plate"),
    (mcv_item::BIRCH_PRESSURE_PLATE, "birch_pressure_plate"),
    (mcv_item::JUNGLE_PRESSURE_PLATE, "jungle_pressure_plate"),
    (mcv_item::ACACIA_PRESSURE_PLATE, "acacia_pressure_plate"),
    (mcv_item::DARK_OAK_PRESSURE_PLATE, "dark_oak_pressure_plate"),
    // 蜂窝族 + 玻璃瓶（honey_bottle.json 的 honey_block/glass_bottle 配角）
    (mcv_item::HONEYCOMB, "honeycomb"),
    (mcv_item::HONEY_BOTTLE, "honey_bottle"),
    (mcv_item::GLASS_BOTTLE, "glass_bottle"),
    (mcv_item::HONEYCOMB_BLOCK, "honeycomb_block"),
    (mcv_item::HONEY_BLOCK, "honey_block"),
    (mcv_item::BEEHIVE, "beehive"),
    (mcv_item::BEE_NEST, "bee_nest"),
    (mcv_item::CRAFTING_TABLE, "crafting_table"),
    (mcv_item::CHEST, "chest"),
];

/// 树苗物品（旧 id 50-55，本波只把 kind 从 Material 转 Block 可放置）。
const SAPLINGS: &[(u16, &str)] = &[
    (mcv_item::OAK_SAPLING, "oak_sapling"),
    (mcv_item::BIRCH_SAPLING, "birch_sapling"),
    (mcv_item::SPRUCE_SAPLING, "spruce_sapling"),
    (mcv_item::JUNGLE_SAPLING, "jungle_sapling"),
    (mcv_item::ACACIA_SAPLING, "acacia_sapling"),
    (mcv_item::DARK_OAK_SAPLING, "dark_oak_sapling"),
];

/// 追加序 = id 契约：62 件从 90 起连续编号，名字双向可查。
#[test]
fn wood_family_append_order_lock() {
    for (i, (id, name)) in NEW_ITEMS.iter().enumerate() {
        assert_eq!(*id, 90 + i as u16, "第 {i} 件 {name} 应为 id {}", 90 + i);
        assert_eq!(mcv_item::ITEMS[*id as usize].name, *name, "id {id} 错位");
        assert_eq!(mcv_item::item_by_name(name), Some(*id), "{name} 反查");
    }
    // 表总长 = 旧 90 + 新 62（尾部追加，旧 id 序列化稳定）。
    assert_eq!(mcv_item::ITEMS.len(), 152, "ITEMS 总数 90+62");
}

/// ITEMS↔BLOCKS 映射齐：Block 物品指向同名方块（place→mine 回路自洽），
/// 全部六树种 ×11 形 + 蜂窝族方块件无一缺漏。
#[test]
fn wood_family_block_mapping() {
    for (id, name) in NEW_ITEMS.iter().chain(SAPLINGS.iter()) {
        let def = &mcv_item::ITEMS[*id as usize];
        let mcv_item::ItemKind::Block(b) = def.kind else {
            panic!("{name} 应为 Block 物品（可放置），实为 {:?}", def.kind);
        };
        assert_eq!(
            mcv_core::BLOCKS[b.id() as usize].name,
            *name,
            "{name} BlockId 错位"
        );
    }
    // 六树种 log/planks 用旧物品位，kind 也必须是 Block（oak=旧表）。
    let legacy: &[(u16, &str, &str)] = &[
        (mcv_item::LOG, "log", "log"),
        (mcv_item::PLANKS, "planks", "planks"),
        (mcv_item::BIRCH_LOG, "birch_log", "birch_log"),
        (mcv_item::SPRUCE_LOG, "spruce_log", "spruce_log"),
        (mcv_item::JUNGLE_LOG, "jungle_log", "jungle_log"),
        (mcv_item::ACACIA_LOG, "acacia_log", "acacia_log"),
        (mcv_item::DARK_OAK_LOG, "dark_oak_log", "dark_oak_log"),
    ];
    for (id, item_name, block_name) in legacy {
        let mcv_item::ItemKind::Block(b) = mcv_item::ITEMS[*id as usize].kind else {
            panic!("{item_name} 应为 Block 物品");
        };
        assert_eq!(mcv_core::BLOCKS[b.id() as usize].name, *block_name);
        assert_eq!(mcv_item::ITEMS[*id as usize].name, *item_name);
    }
}

/// 图标层存在：Block 物品走方块图集 tiles（层号须为真实贴图 < 827，
/// mcv_core::atlas::SENTINEL_LAYER）；Material 物品走 #125 items 精灵表，
/// 原版 item/<name>.png 须已在库（assets 全量随仓，无需运行时拉取）。
#[test]
fn wood_family_icon_layer_present() {
    let sentinel = mcv_core::atlas::SENTINEL_LAYER as u16;
    for (id, name) in NEW_ITEMS.iter().chain(SAPLINGS.iter()) {
        let def = &mcv_item::ITEMS[*id as usize];
        match def.kind {
            ItemKind::Block(b) => {
                let tiles = mcv_core::BLOCKS[b.id() as usize].tiles;
                assert!(
                    tiles.iter().all(|&t| t < sentinel),
                    "{name} 方块图集层含哨兵 {tiles:?}"
                );
            }
            _ => {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../../assets/minecraft/textures/item/{name}.png"));
                assert!(path.is_file(), "{} 缺 item 精灵 {}", name, path.display());
            }
        }
    }
}

/// 蜂窝族非方块件与木族全件均 64 堆（无耐久组件，inventory::max_stack 语义）。
#[test]
fn wood_family_stack_sizes() {
    for (id, name) in NEW_ITEMS.iter() {
        assert_eq!(
            mcv_item::inventory::Hotbar::max_stack(*id),
            64,
            "{name} 应 64 堆"
        );
    }
}
