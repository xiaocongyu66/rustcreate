//! MC GUI 精灵表:从单一资源根 `assets/minecraft/textures/**` 读取原版
//! 按钮/HUD 精灵/logo(路径即 jar 内原版路径),运行时拼成一张 RGBA 纹理
//! (HUD 管线 tex=2)。核心 HUD 精灵任一文件缺失/解码失败都返回 None,调用方
//! 回退程序化绘制(安卓包不带素材也不能崩);物品图标缺失只跳过该条。
//!
//! 尺寸参照(见 mc-ref/NOTES-ui.md):button.png 200x20 九宫格
//! border=3(AbstractButton + mcmeta),hotbar.png 182x22、选中 24x23、
//! 心/饥饿 9x9、准星 15x15(Gui.java),logo 逻辑 256x64 取上 44
//! (LogoRenderer.LOGO_HEIGHT)。

use crate::gpu::HudQuad;

/// 核心 HUD 精灵条目:`textures/` 下的原版路径 → 逻辑尺寸。缺一整表弃用。
const CORE_SPRITES: [(&str, &str, u32, u32); 16] = [
    ("logo", "gui/title/minecraft.png", 256, 64),
    ("button", "gui/sprites/widget/button.png", 200, 20),
    (
        "button_hl",
        "gui/sprites/widget/button_highlighted.png",
        200,
        20,
    ),
    (
        "button_dis",
        "gui/sprites/widget/button_disabled.png",
        200,
        20,
    ),
    ("hotbar", "gui/sprites/hud/hotbar.png", 182, 22),
    ("hotbar_sel", "gui/sprites/hud/hotbar_selection.png", 24, 23),
    ("crosshair", "gui/sprites/hud/crosshair.png", 15, 15),
    (
        "heart_container",
        "gui/sprites/hud/heart/container.png",
        9,
        9,
    ),
    ("heart_full", "gui/sprites/hud/heart/full.png", 9, 9),
    ("heart_half", "gui/sprites/hud/heart/half.png", 9, 9),
    ("food_empty", "gui/sprites/hud/food_empty.png", 9, 9),
    ("food_full", "gui/sprites/hud/food_full.png", 9, 9),
    ("food_half", "gui/sprites/hud/food_half.png", 9, 9),
    // 空气泡三态（26.1 Gui.java:103-105 AIR/AIR_POPPING/AIR_EMPTY_SPRITE，
    // Gui.java:127 AIR_BUBBLE_SIZE=9）：extractAirBubbles 用，路径即 jar 内
    // 原版 hud/air*.png。
    ("air", "gui/sprites/hud/air.png", 9, 9),
    ("air_bursting", "gui/sprites/hud/air_bursting.png", 9, 9),
    ("air_empty", "gui/sprites/hud/air_empty.png", 9, 9),
];

/// 物品图标条目(名字 = mcv_item::ItemDef.name,快捷栏直接按名查)。
/// 与核心表不同:缺文件只跳过该条(sprite_full 返回 None,调用方回退),
/// 不拖垮整表。Block 类物品不走此表——用方块图集 tile 面。
/// lapis 原版贴图叫 lapis_lazuli.png,条目名保持物品名。
const ITEM_SPRITES: [(&str, &str, u32, u32); 355] = [
    ("stick", "item/stick.png", 16, 16),
    ("coal", "item/coal.png", 16, 16),
    ("iron_ingot", "item/iron_ingot.png", 16, 16),
    ("diamond", "item/diamond.png", 16, 16),
    ("lapis", "item/lapis_lazuli.png", 16, 16),
    ("book", "item/book.png", 16, 16),
    ("enchanted_book", "item/enchanted_book.png", 16, 16),
    ("wooden_sword", "item/wooden_sword.png", 16, 16),
    ("stone_sword", "item/stone_sword.png", 16, 16),
    ("iron_sword", "item/iron_sword.png", 16, 16),
    ("diamond_sword", "item/diamond_sword.png", 16, 16),
    ("golden_sword", "item/golden_sword.png", 16, 16),
    ("netherite_sword", "item/netherite_sword.png", 16, 16),
    ("wooden_pickaxe", "item/wooden_pickaxe.png", 16, 16),
    ("stone_pickaxe", "item/stone_pickaxe.png", 16, 16),
    ("iron_pickaxe", "item/iron_pickaxe.png", 16, 16),
    ("diamond_pickaxe", "item/diamond_pickaxe.png", 16, 16),
    ("golden_pickaxe", "item/golden_pickaxe.png", 16, 16),
    ("netherite_pickaxe", "item/netherite_pickaxe.png", 16, 16),
    ("wooden_axe", "item/wooden_axe.png", 16, 16),
    ("stone_axe", "item/stone_axe.png", 16, 16),
    ("iron_axe", "item/iron_axe.png", 16, 16),
    ("diamond_axe", "item/diamond_axe.png", 16, 16),
    ("iron_shovel", "item/iron_shovel.png", 16, 16),
    ("diamond_shovel", "item/diamond_shovel.png", 16, 16),
    ("wooden_shovel", "item/wooden_shovel.png", 16, 16),
    // #137 W 波木族+蜂窝追加：Material 物品无方块图集 tile 可借，走本表
    // （名字 = mcv_item::ItemDef.name；贴图名 = jar 内 item/*.png）。
    ("honeycomb", "item/honeycomb.png", 16, 16),
    ("honey_bottle", "item/honey_bottle.png", 16, 16),
    ("glass_bottle", "item/glass_bottle.png", 16, 16),
    // ---- 全方块计划·物品地基波（#144）追加：Material 物品图标全补——
    // 贴图名 = jar 内 item/*.png（26.1 时钟/指南针/十字弓为帧序列/姿态
    // 贴图，取 00/standby 帧；enchanted_golden_apple 原版模型直引
    // golden_apple）。缺文件条目由装配层跳过（best-effort 语义不变）。
    ("acacia_boat", "item/acacia_boat.png", 16, 16),
    ("acacia_chest_boat", "item/acacia_chest_boat.png", 16, 16),
    ("amethyst_shard", "item/amethyst_shard.png", 16, 16),
    (
        "angler_pottery_sherd",
        "item/angler_pottery_sherd.png",
        16,
        16,
    ),
    ("apple", "item/apple.png", 16, 16),
    (
        "archer_pottery_sherd",
        "item/archer_pottery_sherd.png",
        16,
        16,
    ),
    ("armadillo_scute", "item/armadillo_scute.png", 16, 16),
    ("armor_stand", "item/armor_stand.png", 16, 16),
    (
        "arms_up_pottery_sherd",
        "item/arms_up_pottery_sherd.png",
        16,
        16,
    ),
    ("arrow", "item/arrow.png", 16, 16),
    ("baked_potato", "item/baked_potato.png", 16, 16),
    ("bamboo_chest_raft", "item/bamboo_chest_raft.png", 16, 16),
    ("bamboo_raft", "item/bamboo_raft.png", 16, 16),
    ("beef", "item/beef.png", 16, 16),
    ("beetroot", "item/beetroot.png", 16, 16),
    ("beetroot_soup", "item/beetroot_soup.png", 16, 16),
    ("birch_boat", "item/birch_boat.png", 16, 16),
    ("birch_chest_boat", "item/birch_chest_boat.png", 16, 16),
    ("black_bundle", "item/black_bundle.png", 16, 16),
    ("black_dye", "item/black_dye.png", 16, 16),
    ("black_harness", "item/black_harness.png", 16, 16),
    (
        "blade_pottery_sherd",
        "item/blade_pottery_sherd.png",
        16,
        16,
    ),
    ("blaze_powder", "item/blaze_powder.png", 16, 16),
    ("blaze_rod", "item/blaze_rod.png", 16, 16),
    ("blue_bundle", "item/blue_bundle.png", 16, 16),
    ("blue_dye", "item/blue_dye.png", 16, 16),
    ("blue_egg", "item/blue_egg.png", 16, 16),
    ("blue_harness", "item/blue_harness.png", 16, 16),
    (
        "bolt_armor_trim_smithing_template",
        "item/bolt_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("bone", "item/bone.png", 16, 16),
    ("bone_meal", "item/bone_meal.png", 16, 16),
    (
        "bordure_indented_banner_pattern",
        "item/bordure_indented_banner_pattern.png",
        16,
        16,
    ),
    ("bowl", "item/bowl.png", 16, 16),
    ("bread", "item/bread.png", 16, 16),
    ("breeze_rod", "item/breeze_rod.png", 16, 16),
    (
        "brewer_pottery_sherd",
        "item/brewer_pottery_sherd.png",
        16,
        16,
    ),
    ("brick", "item/brick.png", 16, 16),
    ("brown_bundle", "item/brown_bundle.png", 16, 16),
    ("brown_dye", "item/brown_dye.png", 16, 16),
    ("brown_egg", "item/brown_egg.png", 16, 16),
    ("brown_harness", "item/brown_harness.png", 16, 16),
    ("brush", "item/brush.png", 16, 16),
    ("bucket", "item/bucket.png", 16, 16),
    ("bundle", "item/bundle.png", 16, 16),
    ("burn_pottery_sherd", "item/burn_pottery_sherd.png", 16, 16),
    ("carrot_on_a_stick", "item/carrot_on_a_stick.png", 16, 16),
    ("chainmail_boots", "item/chainmail_boots.png", 16, 16),
    (
        "chainmail_chestplate",
        "item/chainmail_chestplate.png",
        16,
        16,
    ),
    ("chainmail_helmet", "item/chainmail_helmet.png", 16, 16),
    ("chainmail_leggings", "item/chainmail_leggings.png", 16, 16),
    ("charcoal", "item/charcoal.png", 16, 16),
    ("cherry_boat", "item/cherry_boat.png", 16, 16),
    ("cherry_chest_boat", "item/cherry_chest_boat.png", 16, 16),
    ("chest_minecart", "item/chest_minecart.png", 16, 16),
    ("chicken", "item/chicken.png", 16, 16),
    ("chorus_fruit", "item/chorus_fruit.png", 16, 16),
    ("clay_ball", "item/clay_ball.png", 16, 16),
    ("clock", "item/clock_00.png", 16, 16),
    (
        "coast_armor_trim_smithing_template",
        "item/coast_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("cod", "item/cod.png", 16, 16),
    ("cod_bucket", "item/cod_bucket.png", 16, 16),
    ("compass", "item/compass_00.png", 16, 16),
    ("cooked_beef", "item/cooked_beef.png", 16, 16),
    ("cooked_chicken", "item/cooked_chicken.png", 16, 16),
    ("cooked_cod", "item/cooked_cod.png", 16, 16),
    ("cooked_mutton", "item/cooked_mutton.png", 16, 16),
    ("cooked_porkchop", "item/cooked_porkchop.png", 16, 16),
    ("cooked_rabbit", "item/cooked_rabbit.png", 16, 16),
    ("cooked_salmon", "item/cooked_salmon.png", 16, 16),
    ("cookie", "item/cookie.png", 16, 16),
    ("copper_axe", "item/copper_axe.png", 16, 16),
    ("copper_boots", "item/copper_boots.png", 16, 16),
    ("copper_chestplate", "item/copper_chestplate.png", 16, 16),
    ("copper_helmet", "item/copper_helmet.png", 16, 16),
    ("copper_hoe", "item/copper_hoe.png", 16, 16),
    ("copper_ingot", "item/copper_ingot.png", 16, 16),
    ("copper_leggings", "item/copper_leggings.png", 16, 16),
    ("copper_nugget", "item/copper_nugget.png", 16, 16),
    ("copper_pickaxe", "item/copper_pickaxe.png", 16, 16),
    ("copper_shovel", "item/copper_shovel.png", 16, 16),
    ("copper_spear", "item/copper_spear.png", 16, 16),
    ("copper_sword", "item/copper_sword.png", 16, 16),
    (
        "creeper_banner_pattern",
        "item/creeper_banner_pattern.png",
        16,
        16,
    ),
    ("crossbow", "item/crossbow_standby.png", 16, 16),
    ("cyan_bundle", "item/cyan_bundle.png", 16, 16),
    ("cyan_dye", "item/cyan_dye.png", 16, 16),
    ("cyan_harness", "item/cyan_harness.png", 16, 16),
    (
        "danger_pottery_sherd",
        "item/danger_pottery_sherd.png",
        16,
        16,
    ),
    ("dark_oak_boat", "item/dark_oak_boat.png", 16, 16),
    (
        "dark_oak_chest_boat",
        "item/dark_oak_chest_boat.png",
        16,
        16,
    ),
    ("diamond_boots", "item/diamond_boots.png", 16, 16),
    ("diamond_chestplate", "item/diamond_chestplate.png", 16, 16),
    ("diamond_helmet", "item/diamond_helmet.png", 16, 16),
    ("diamond_hoe", "item/diamond_hoe.png", 16, 16),
    ("diamond_leggings", "item/diamond_leggings.png", 16, 16),
    ("diamond_spear", "item/diamond_spear.png", 16, 16),
    ("disc_fragment_5", "item/disc_fragment_5.png", 16, 16),
    ("dried_kelp", "item/dried_kelp.png", 16, 16),
    (
        "dune_armor_trim_smithing_template",
        "item/dune_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("echo_shard", "item/echo_shard.png", 16, 16),
    ("egg", "item/egg.png", 16, 16),
    ("emerald", "item/emerald.png", 16, 16),
    ("enchanted_golden_apple", "item/golden_apple.png", 16, 16),
    ("end_crystal", "item/end_crystal.png", 16, 16),
    ("ender_eye", "item/ender_eye.png", 16, 16),
    ("ender_pearl", "item/ender_pearl.png", 16, 16),
    (
        "explorer_pottery_sherd",
        "item/explorer_pottery_sherd.png",
        16,
        16,
    ),
    (
        "eye_armor_trim_smithing_template",
        "item/eye_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("feather", "item/feather.png", 16, 16),
    (
        "fermented_spider_eye",
        "item/fermented_spider_eye.png",
        16,
        16,
    ),
    (
        "field_masoned_banner_pattern",
        "item/field_masoned_banner_pattern.png",
        16,
        16,
    ),
    ("fire_charge", "item/fire_charge.png", 16, 16),
    ("firework_rocket", "item/firework_rocket.png", 16, 16),
    ("fishing_rod", "item/fishing_rod.png", 16, 16),
    ("flint", "item/flint.png", 16, 16),
    ("flint_and_steel", "item/flint_and_steel.png", 16, 16),
    (
        "flow_armor_trim_smithing_template",
        "item/flow_armor_trim_smithing_template.png",
        16,
        16,
    ),
    (
        "flow_banner_pattern",
        "item/flow_banner_pattern.png",
        16,
        16,
    ),
    ("flow_pottery_sherd", "item/flow_pottery_sherd.png", 16, 16),
    (
        "flower_banner_pattern",
        "item/flower_banner_pattern.png",
        16,
        16,
    ),
    (
        "friend_pottery_sherd",
        "item/friend_pottery_sherd.png",
        16,
        16,
    ),
    ("furnace_minecart", "item/furnace_minecart.png", 16, 16),
    ("ghast_tear", "item/ghast_tear.png", 16, 16),
    (
        "glistering_melon_slice",
        "item/glistering_melon_slice.png",
        16,
        16,
    ),
    (
        "globe_banner_pattern",
        "item/globe_banner_pattern.png",
        16,
        16,
    ),
    ("glow_ink_sac", "item/glow_ink_sac.png", 16, 16),
    ("glow_item_frame", "item/glow_item_frame.png", 16, 16),
    ("glowstone_dust", "item/glowstone_dust.png", 16, 16),
    ("gold_ingot", "item/gold_ingot.png", 16, 16),
    ("gold_nugget", "item/gold_nugget.png", 16, 16),
    ("golden_apple", "item/golden_apple.png", 16, 16),
    ("golden_axe", "item/golden_axe.png", 16, 16),
    ("golden_boots", "item/golden_boots.png", 16, 16),
    ("golden_carrot", "item/golden_carrot.png", 16, 16),
    ("golden_chestplate", "item/golden_chestplate.png", 16, 16),
    ("golden_helmet", "item/golden_helmet.png", 16, 16),
    ("golden_hoe", "item/golden_hoe.png", 16, 16),
    ("golden_horse_armor", "item/golden_horse_armor.png", 16, 16),
    ("golden_leggings", "item/golden_leggings.png", 16, 16),
    (
        "golden_nautilus_armor",
        "item/golden_nautilus_armor.png",
        16,
        16,
    ),
    ("golden_shovel", "item/golden_shovel.png", 16, 16),
    ("golden_spear", "item/golden_spear.png", 16, 16),
    ("gray_bundle", "item/gray_bundle.png", 16, 16),
    ("gray_dye", "item/gray_dye.png", 16, 16),
    ("gray_harness", "item/gray_harness.png", 16, 16),
    ("green_bundle", "item/green_bundle.png", 16, 16),
    ("green_dye", "item/green_dye.png", 16, 16),
    ("green_harness", "item/green_harness.png", 16, 16),
    ("gunpowder", "item/gunpowder.png", 16, 16),
    (
        "guster_banner_pattern",
        "item/guster_banner_pattern.png",
        16,
        16,
    ),
    (
        "guster_pottery_sherd",
        "item/guster_pottery_sherd.png",
        16,
        16,
    ),
    ("heart_of_the_sea", "item/heart_of_the_sea.png", 16, 16),
    (
        "heart_pottery_sherd",
        "item/heart_pottery_sherd.png",
        16,
        16,
    ),
    (
        "heartbreak_pottery_sherd",
        "item/heartbreak_pottery_sherd.png",
        16,
        16,
    ),
    ("hopper_minecart", "item/hopper_minecart.png", 16, 16),
    (
        "host_armor_trim_smithing_template",
        "item/host_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("howl_pottery_sherd", "item/howl_pottery_sherd.png", 16, 16),
    ("ink_sac", "item/ink_sac.png", 16, 16),
    ("iron_boots", "item/iron_boots.png", 16, 16),
    ("iron_chestplate", "item/iron_chestplate.png", 16, 16),
    ("iron_helmet", "item/iron_helmet.png", 16, 16),
    ("iron_hoe", "item/iron_hoe.png", 16, 16),
    ("iron_leggings", "item/iron_leggings.png", 16, 16),
    ("iron_nugget", "item/iron_nugget.png", 16, 16),
    ("iron_spear", "item/iron_spear.png", 16, 16),
    ("item_frame", "item/item_frame.png", 16, 16),
    ("jungle_boat", "item/jungle_boat.png", 16, 16),
    ("jungle_chest_boat", "item/jungle_chest_boat.png", 16, 16),
    ("knowledge_book", "item/knowledge_book.png", 16, 16),
    ("lapis_lazuli", "item/lapis_lazuli.png", 16, 16),
    ("lead", "item/lead.png", 16, 16),
    ("leather", "item/leather.png", 16, 16),
    ("leather_boots", "item/leather_boots.png", 16, 16),
    ("leather_chestplate", "item/leather_chestplate.png", 16, 16),
    ("leather_helmet", "item/leather_helmet.png", 16, 16),
    (
        "leather_horse_armor",
        "item/leather_horse_armor.png",
        16,
        16,
    ),
    ("leather_leggings", "item/leather_leggings.png", 16, 16),
    ("light_blue_bundle", "item/light_blue_bundle.png", 16, 16),
    ("light_blue_dye", "item/light_blue_dye.png", 16, 16),
    ("light_blue_harness", "item/light_blue_harness.png", 16, 16),
    ("light_gray_bundle", "item/light_gray_bundle.png", 16, 16),
    ("light_gray_dye", "item/light_gray_dye.png", 16, 16),
    ("light_gray_harness", "item/light_gray_harness.png", 16, 16),
    ("lime_bundle", "item/lime_bundle.png", 16, 16),
    ("lime_dye", "item/lime_dye.png", 16, 16),
    ("lime_harness", "item/lime_harness.png", 16, 16),
    ("mace", "item/mace.png", 16, 16),
    ("magenta_bundle", "item/magenta_bundle.png", 16, 16),
    ("magenta_dye", "item/magenta_dye.png", 16, 16),
    ("magenta_harness", "item/magenta_harness.png", 16, 16),
    ("magma_cream", "item/magma_cream.png", 16, 16),
    ("mangrove_boat", "item/mangrove_boat.png", 16, 16),
    (
        "mangrove_chest_boat",
        "item/mangrove_chest_boat.png",
        16,
        16,
    ),
    ("map", "item/map.png", 16, 16),
    ("melon_slice", "item/melon_slice.png", 16, 16),
    ("milk_bucket", "item/milk_bucket.png", 16, 16),
    ("minecart", "item/minecart.png", 16, 16),
    (
        "miner_pottery_sherd",
        "item/miner_pottery_sherd.png",
        16,
        16,
    ),
    (
        "mojang_banner_pattern",
        "item/mojang_banner_pattern.png",
        16,
        16,
    ),
    (
        "mourner_pottery_sherd",
        "item/mourner_pottery_sherd.png",
        16,
        16,
    ),
    ("mushroom_stew", "item/mushroom_stew.png", 16, 16),
    ("music_disc_11", "item/music_disc_11.png", 16, 16),
    ("music_disc_13", "item/music_disc_13.png", 16, 16),
    ("music_disc_5", "item/music_disc_5.png", 16, 16),
    ("music_disc_blocks", "item/music_disc_blocks.png", 16, 16),
    ("music_disc_cat", "item/music_disc_cat.png", 16, 16),
    ("music_disc_chirp", "item/music_disc_chirp.png", 16, 16),
    ("music_disc_far", "item/music_disc_far.png", 16, 16),
    ("music_disc_mall", "item/music_disc_mall.png", 16, 16),
    ("music_disc_mellohi", "item/music_disc_mellohi.png", 16, 16),
    ("music_disc_stal", "item/music_disc_stal.png", 16, 16),
    ("music_disc_strad", "item/music_disc_strad.png", 16, 16),
    ("music_disc_wait", "item/music_disc_wait.png", 16, 16),
    ("music_disc_ward", "item/music_disc_ward.png", 16, 16),
    ("mutton", "item/mutton.png", 16, 16),
    ("name_tag", "item/name_tag.png", 16, 16),
    ("nautilus_shell", "item/nautilus_shell.png", 16, 16),
    ("nether_brick", "item/nether_brick.png", 16, 16),
    ("nether_star", "item/nether_star.png", 16, 16),
    ("netherite_axe", "item/netherite_axe.png", 16, 16),
    ("netherite_boots", "item/netherite_boots.png", 16, 16),
    (
        "netherite_chestplate",
        "item/netherite_chestplate.png",
        16,
        16,
    ),
    ("netherite_helmet", "item/netherite_helmet.png", 16, 16),
    ("netherite_hoe", "item/netherite_hoe.png", 16, 16),
    ("netherite_ingot", "item/netherite_ingot.png", 16, 16),
    ("netherite_leggings", "item/netherite_leggings.png", 16, 16),
    ("netherite_scrap", "item/netherite_scrap.png", 16, 16),
    ("netherite_shovel", "item/netherite_shovel.png", 16, 16),
    ("netherite_spear", "item/netherite_spear.png", 16, 16),
    (
        "netherite_upgrade_smithing_template",
        "item/netherite_upgrade_smithing_template.png",
        16,
        16,
    ),
    ("oak_boat", "item/oak_boat.png", 16, 16),
    ("oak_chest_boat", "item/oak_chest_boat.png", 16, 16),
    ("orange_bundle", "item/orange_bundle.png", 16, 16),
    ("orange_dye", "item/orange_dye.png", 16, 16),
    ("orange_harness", "item/orange_harness.png", 16, 16),
    ("painting", "item/painting.png", 16, 16),
    ("pale_oak_boat", "item/pale_oak_boat.png", 16, 16),
    (
        "pale_oak_chest_boat",
        "item/pale_oak_chest_boat.png",
        16,
        16,
    ),
    ("paper", "item/paper.png", 16, 16),
    (
        "piglin_banner_pattern",
        "item/piglin_banner_pattern.png",
        16,
        16,
    ),
    ("pink_bundle", "item/pink_bundle.png", 16, 16),
    ("pink_dye", "item/pink_dye.png", 16, 16),
    ("pink_harness", "item/pink_harness.png", 16, 16),
    (
        "plenty_pottery_sherd",
        "item/plenty_pottery_sherd.png",
        16,
        16,
    ),
    ("poisonous_potato", "item/poisonous_potato.png", 16, 16),
    (
        "popped_chorus_fruit",
        "item/popped_chorus_fruit.png",
        16,
        16,
    ),
    ("porkchop", "item/porkchop.png", 16, 16),
    (
        "prismarine_crystals",
        "item/prismarine_crystals.png",
        16,
        16,
    ),
    ("prismarine_shard", "item/prismarine_shard.png", 16, 16),
    (
        "prize_pottery_sherd",
        "item/prize_pottery_sherd.png",
        16,
        16,
    ),
    ("pufferfish", "item/pufferfish.png", 16, 16),
    ("pufferfish_bucket", "item/pufferfish_bucket.png", 16, 16),
    ("pumpkin_pie", "item/pumpkin_pie.png", 16, 16),
    ("purple_bundle", "item/purple_bundle.png", 16, 16),
    ("purple_dye", "item/purple_dye.png", 16, 16),
    ("purple_harness", "item/purple_harness.png", 16, 16),
    ("quartz", "item/quartz.png", 16, 16),
    ("rabbit", "item/rabbit.png", 16, 16),
    ("rabbit_foot", "item/rabbit_foot.png", 16, 16),
    ("rabbit_hide", "item/rabbit_hide.png", 16, 16),
    ("rabbit_stew", "item/rabbit_stew.png", 16, 16),
    (
        "raiser_armor_trim_smithing_template",
        "item/raiser_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("raw_copper", "item/raw_copper.png", 16, 16),
    ("raw_gold", "item/raw_gold.png", 16, 16),
    ("raw_iron", "item/raw_iron.png", 16, 16),
    ("recovery_compass", "item/recovery_compass_00.png", 16, 16),
    ("red_bundle", "item/red_bundle.png", 16, 16),
    ("red_dye", "item/red_dye.png", 16, 16),
    ("red_harness", "item/red_harness.png", 16, 16),
    ("redstone", "item/redstone.png", 16, 16),
    ("resin_brick", "item/resin_brick.png", 16, 16),
    (
        "rib_armor_trim_smithing_template",
        "item/rib_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("rotten_flesh", "item/rotten_flesh.png", 16, 16),
    ("saddle", "item/saddle.png", 16, 16),
    ("salmon", "item/salmon.png", 16, 16),
    ("salmon_bucket", "item/salmon_bucket.png", 16, 16),
    (
        "scrape_pottery_sherd",
        "item/scrape_pottery_sherd.png",
        16,
        16,
    ),
    (
        "sentry_armor_trim_smithing_template",
        "item/sentry_armor_trim_smithing_template.png",
        16,
        16,
    ),
    (
        "shaper_armor_trim_smithing_template",
        "item/shaper_armor_trim_smithing_template.png",
        16,
        16,
    ),
    (
        "sheaf_pottery_sherd",
        "item/sheaf_pottery_sherd.png",
        16,
        16,
    ),
    ("shears", "item/shears.png", 16, 16),
    (
        "shelter_pottery_sherd",
        "item/shelter_pottery_sherd.png",
        16,
        16,
    ),
    ("shulker_shell", "item/shulker_shell.png", 16, 16),
    (
        "silence_armor_trim_smithing_template",
        "item/silence_armor_trim_smithing_template.png",
        16,
        16,
    ),
    (
        "skull_banner_pattern",
        "item/skull_banner_pattern.png",
        16,
        16,
    ),
    (
        "skull_pottery_sherd",
        "item/skull_pottery_sherd.png",
        16,
        16,
    ),
    ("slime_ball", "item/slime_ball.png", 16, 16),
    (
        "snort_pottery_sherd",
        "item/snort_pottery_sherd.png",
        16,
        16,
    ),
    (
        "snout_armor_trim_smithing_template",
        "item/snout_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("snowball", "item/snowball.png", 16, 16),
    ("spectral_arrow", "item/spectral_arrow.png", 16, 16),
    ("spider_eye", "item/spider_eye.png", 16, 16),
    (
        "spire_armor_trim_smithing_template",
        "item/spire_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("spruce_boat", "item/spruce_boat.png", 16, 16),
    ("spruce_chest_boat", "item/spruce_chest_boat.png", 16, 16),
    ("spyglass", "item/spyglass.png", 16, 16),
    ("stone_hoe", "item/stone_hoe.png", 16, 16),
    ("stone_shovel", "item/stone_shovel.png", 16, 16),
    ("stone_spear", "item/stone_spear.png", 16, 16),
    ("string", "item/string.png", 16, 16),
    ("sugar", "item/sugar.png", 16, 16),
    (
        "tide_armor_trim_smithing_template",
        "item/tide_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("tnt_minecart", "item/tnt_minecart.png", 16, 16),
    ("trident", "item/trident.png", 16, 16),
    ("tropical_fish", "item/tropical_fish.png", 16, 16),
    (
        "tropical_fish_bucket",
        "item/tropical_fish_bucket.png",
        16,
        16,
    ),
    ("turtle_helmet", "item/turtle_helmet.png", 16, 16),
    ("turtle_scute", "item/turtle_scute.png", 16, 16),
    (
        "vex_armor_trim_smithing_template",
        "item/vex_armor_trim_smithing_template.png",
        16,
        16,
    ),
    (
        "ward_armor_trim_smithing_template",
        "item/ward_armor_trim_smithing_template.png",
        16,
        16,
    ),
    (
        "warped_fungus_on_a_stick",
        "item/warped_fungus_on_a_stick.png",
        16,
        16,
    ),
    (
        "wayfinder_armor_trim_smithing_template",
        "item/wayfinder_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("wheat", "item/wheat.png", 16, 16),
    ("white_bundle", "item/white_bundle.png", 16, 16),
    ("white_dye", "item/white_dye.png", 16, 16),
    ("white_harness", "item/white_harness.png", 16, 16),
    (
        "wild_armor_trim_smithing_template",
        "item/wild_armor_trim_smithing_template.png",
        16,
        16,
    ),
    ("wind_charge", "item/wind_charge.png", 16, 16),
    ("wolf_armor", "item/wolf_armor.png", 16, 16),
    ("wooden_hoe", "item/wooden_hoe.png", 16, 16),
    ("wooden_spear", "item/wooden_spear.png", 16, 16),
    ("writable_book", "item/writable_book.png", 16, 16),
    ("written_book", "item/written_book.png", 16, 16),
    ("yellow_bundle", "item/yellow_bundle.png", 16, 16),
    ("yellow_dye", "item/yellow_dye.png", 16, 16),
    ("yellow_harness", "item/yellow_harness.png", 16, 16),
];

/// 其他 best-effort 精灵(缺文件只跳过该条,不拖垮整表)。
/// menu_background:进世界加载画面背景(26.1 Screen.java:404-420
/// extractMenuBackgroundTexture 以 32×32 平铺全屏,LevelLoadingScreen
/// Reason.OTHER 背景的组成部分;panorama 立方渲染器本仓未做,见
/// loading_ui.rs 模块注释)。
const EXTRA_SPRITES: [(&str, &str, u32, u32); 1] =
    [("menu_background", "gui/menu_background.png", 32, 32)];

/// 容器面板条目类型：名字、路径、逻辑宽高、源裁剪矩形 (sx, sy, sw, sh)。
type PanelSprite = (&'static str, &'static str, u32, u32, u32, u32, u32, u32);

/// 容器面板(原版整幅 blit,非九宫格):名字、路径、逻辑尺寸、源裁剪矩形。
/// 原版渲染 = `graphics.blit(GUI_TEXTURED, <面板贴图>, xo, yo, 0, 0,
/// imageWidth, imageHeight, 256, 256)`(AbstractContainerScreen 定位
/// ((width-imageWidth)/2, (height-imageHeight)/2),:89-90;InventoryScreen.java:98、
/// CraftingScreen.java:35),即从 256x256 纹理取左上 176x166 画到面板左上角。
/// 面板贴图自带槽位凹槽——命中即不再程序化画槽底。缺文件跳过(调用方回退
/// 程序化面板)。
const PANEL_SPRITES: [PanelSprite; 2] = [
    (
        "inv_panel",
        "gui/container/inventory.png",
        176,
        166,
        0,
        0,
        176,
        166,
    ),
    (
        "craft_panel",
        "gui/container/crafting_table.png",
        176,
        166,
        0,
        0,
        176,
        166,
    ),
];

/// logo 实际绘制行数(MC 纹理 256x64 只显示上 44 行)。
pub const LOGO_VISIBLE_H: u32 = 44;

/// 一张拼好的 GUI 精灵表 + 名称→表内像素矩形。
pub struct SpriteSheet {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
    entries: Vec<(&'static str, u32, u32, u32, u32)>, // name, x, y, w, h
}

/// 近邻重采样 RGBA 到目标尺寸。
fn resample(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32, dst: &mut [u8]) {
    for y in 0..dh as usize {
        for x in 0..dw as usize {
            let sx = (x as u32 * sw / dw).min(sw - 1) as usize;
            let sy = (y as u32 * sh / dh).min(sh - 1) as usize;
            let s = (sy * sw as usize + sx) * 4;
            let d = (y * dw as usize + x) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

impl SpriteSheet {
    /// 从资源根(`assets/minecraft`)读 `textures/` 下原版精灵。
    /// 核心 HUD 精灵缺一即 None(调用方整体回退程序化绘制);
    /// 物品图标 best-effort:缺文件只 warn 并跳过该条。
    /// M8c：文件读取收编到 mcv_assets(缺失文件经 AssetError 显式 warn,含
    /// 完整路径),不再各自静默 `std::fs::read().ok()`。
    pub fn load(dir: &std::path::Path) -> Option<Self> {
        Self::load_via(&mcv_assets::AssetManager::new(dir))
    }

    /// 经 AssetManager 读取(统一缓存/缺素材登记),布局装配逻辑不变。
    pub fn load_via(assets: &mcv_assets::AssetManager) -> Option<Self> {
        let decode = |name: &'static str,
                      file: &str,
                      w: u32,
                      h: u32|
         -> Option<(&'static str, Vec<u8>, u32, u32)> {
            let rel = format!("textures/{file}");
            let bytes = assets.read_optional(&rel)?;
            let img = image::load_from_memory(&bytes)
                .map_err(|e| log::warn!("gui: decode textures/{file}: {e}"))
                .ok()?;
            let rgba = img.to_rgba8();
            let (sw, sh) = (rgba.width(), rgba.height());
            if sw == 0 || sh == 0 {
                return None;
            }
            let mut buf = vec![0u8; (w * h * 4) as usize];
            resample(rgba.as_raw(), sw, sh, w, h, &mut buf);
            Some((name, buf, w, h))
        };
        let mut loaded: Vec<(&'static str, Vec<u8>, u32, u32)> = Vec::new();
        for (name, file, w, h) in CORE_SPRITES {
            loaded.push(decode(name, file, w, h)?);
        }
        let items_before = loaded.len();
        for (name, file, w, h) in ITEM_SPRITES {
            loaded.extend(decode(name, file, w, h));
        }
        let items = loaded.len() - items_before;
        if items < ITEM_SPRITES.len() {
            log::warn!("gui: {}/{} item icons missing", items, ITEM_SPRITES.len());
        }
        for (name, file, w, h) in EXTRA_SPRITES {
            loaded.extend(decode(name, file, w, h));
        }
        // 容器面板 best-effort:源裁剪矩形内近邻取样(256x256 容器贴图取
        // 左上 176x166,同原版 blit 源矩形)。
        let panels_before = loaded.len();
        for (name, file, w, h, sx, sy, sw, sh) in PANEL_SPRITES {
            let rel = format!("textures/{file}");
            let Some(bytes) = assets.read_optional(&rel) else {
                continue;
            };
            let Ok(img) =
                image::load_from_memory(&bytes).map_err(|e| log::warn!("gui: decode {rel}: {e}"))
            else {
                continue;
            };
            let rgba = img.to_rgba8();
            let (iw, ih) = (rgba.width(), rgba.height());
            if sx + sw > iw || sy + sh > ih || sw == 0 || sh == 0 {
                log::warn!("gui: panel crop out of range: {rel}");
                continue;
            }
            let mut crop = vec![0u8; (sw * sh * 4) as usize];
            let raw = rgba.as_raw();
            let row_bytes = (sw * 4) as usize;
            for y in 0..sh {
                let s = (((sy + y) * iw + sx) * 4) as usize;
                let d = ((y * sw) * 4) as usize;
                crop[d..d + row_bytes].copy_from_slice(&raw[s..s + row_bytes]);
            }
            let mut buf = vec![0u8; (w * h * 4) as usize];
            resample(&crop, sw, sh, w, h, &mut buf);
            loaded.push((name, buf, w, h));
        }
        if loaded.len() - panels_before < PANEL_SPRITES.len() {
            log::warn!("gui: container panels missing, craft screen uses fallback");
        }
        // 竖排:宽度取最大(256),总高向上取整到 2 的幂
        let sheet_w = loaded.iter().map(|(_, _, w, _)| *w).max().unwrap_or(0);
        let total_h: u32 = loaded.iter().map(|(_, _, _, h)| *h).sum();
        let mut sheet_h = 32u32;
        while sheet_h < total_h {
            sheet_h *= 2;
        }
        let mut rgba = vec![0u8; (sheet_w * sheet_h * 4) as usize];
        let mut entries = Vec::with_capacity(loaded.len());
        let mut y = 0u32;
        for (name, buf, w, h) in loaded {
            for row in 0..h {
                let s = ((row * w * 4) as usize)..(((row + 1) * w * 4) as usize);
                let d =
                    (((y + row) * sheet_w * 4) as usize)..(((y + row) * sheet_w + w) * 4) as usize;
                rgba[d].copy_from_slice(&buf[s]);
            }
            entries.push((name, 0, y, w, h));
            y += h;
        }
        log::info!("gui: MC sprite sheet {sheet_w}x{sheet_h} loaded");
        Some(Self {
            rgba,
            w: sheet_w,
            h: sheet_h,
            entries,
        })
    }

    fn rect(&self, name: &str) -> Option<(u32, u32, u32, u32)> {
        self.entries
            .iter()
            .find(|(n, ..)| *n == name)
            .map(|(_, x, y, w, h)| (*x, *y, *w, *h))
    }

    /// 精灵在表内的 uv 角矩形 ((u0,v0),(u1,v1)，0..1 表空间)。
    /// 第一人称手持物品图标用（hand pass 直接采 sprite 表纹理）。
    pub fn sprite_uv(&self, name: &str) -> Option<([f32; 2], [f32; 2])> {
        let (rx, ry, rw, rh) = self.rect(name)?;
        let w = self.w as f32;
        let h = self.h as f32;
        Some((
            [rx as f32 / w, ry as f32 / h],
            [(rx + rw) as f32 / w, (ry + rh) as f32 / h],
        ))
    }

    fn uv_px(&self, x: f32, y: f32, w: f32, h: f32) -> [[f32; 2]; 2] {
        [
            [x / self.w as f32, y / self.h as f32],
            [(x + w) / self.w as f32, (y + h) / self.h as f32],
        ]
    }

    /// 整枚精灵贴图四边形(源矩形按比例取子区域,frac ∈ 0..1)。
    #[allow(clippy::too_many_arguments)] // 精灵绘制参数天然多（源矩形+目标矩形+着色）
    pub fn sprite(
        &self,
        name: &str,
        fx0: f32,
        fy0: f32,
        fx1: f32,
        fy1: f32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
    ) -> Option<HudQuad> {
        let (rx, ry, rw, rh) = self.rect(name)?;
        let u0 = rx as f32 + rw as f32 * fx0;
        let v0 = ry as f32 + rh as f32 * fy0;
        let u1 = rx as f32 + rw as f32 * fx1;
        let v1 = ry as f32 + rh as f32 * fy1;
        Some(HudQuad {
            x,
            y,
            w,
            h,
            uv: self.uv_px(u0, v0, u1 - u0, v1 - v0),
            color,
            tex: 2,
            layer: 0,
            rot: 0.0,
        })
    }

    /// 整枚精灵。
    pub fn sprite_full(
        &self,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
    ) -> Option<HudQuad> {
        self.sprite(name, 0.0, 0.0, 1.0, 1.0, x, y, w, h, color)
    }

    /// 九宫格拉伸(button.png 的 mcmeta:nine_slice border=3 源像素)。
    /// `scale` 为 GUI 整数缩放:目标圆角块取 border*scale,其余拉伸。
    #[allow(clippy::too_many_arguments)]
    pub fn nine_slice(
        &self,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        border_src: u32,
        scale: f32,
        color: [f32; 4],
    ) -> Vec<HudQuad> {
        let Some((rx, ry, rw, rh)) = self.rect(name) else {
            return Vec::new();
        };
        let sb = border_src.min(rw / 2).min(rh / 2) as f32;
        let bw = (sb * scale).min(w * 0.5);
        let bh = (sb * scale).min(h * 0.5);
        let (bx, by) = (w - bw * 2.0, h - bh * 2.0);
        let (sf, tf) = (rw as f32, rh as f32);
        let mut out = Vec::with_capacity(9);
        // (源 x, 源 y, 源 w, 源 h, 目标相对 x, 目标相对 y, 目标 w, 目标 h)
        let cells = [
            (0.0, 0.0, sb, sb, 0.0, 0.0, bw, bh),
            (sb, 0.0, sf - sb * 2.0, sb, bw, 0.0, bx, bh),
            (sf - sb, 0.0, sb, sb, w - bw, 0.0, bw, bh),
            (0.0, sb, sb, tf - sb * 2.0, 0.0, bh, bw, by),
            (sb, sb, sf - sb * 2.0, tf - sb * 2.0, bw, bh, bx, by),
            (sf - sb, sb, sb, tf - sb * 2.0, w - bw, bh, bw, by),
            (0.0, tf - sb, sb, sb, 0.0, h - bh, bw, bh),
            (sb, tf - sb, sf - sb * 2.0, sb, bw, h - bh, bx, bh),
            (sf - sb, tf - sb, sb, sb, w - bw, h - bh, bw, bh),
        ];
        for (sx, sy, sw, sh, dx, dy, dw, dh) in cells {
            if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
                continue;
            }
            out.push(HudQuad {
                x: x + dx,
                y: y + dy,
                w: dw,
                h: dh,
                uv: self.uv_px(rx as f32 + sx, ry as f32 + sy, sw, sh),
                color,
                tex: 2,
                layer: 0,
                rot: 0.0,
            });
        }
        out
    }

    /// 是否存在指定精灵。
    pub fn has(&self, name: &str) -> bool {
        self.rect(name).is_some()
    }
}
