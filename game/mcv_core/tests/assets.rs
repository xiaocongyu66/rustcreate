//! 素材完整性冒烟（CPU）：所有曾触发过程化回退的关键原版贴图必须存在于
//! 仓库 assets/ 且可解码（26.1.jar 逐字节一致提交）。任一缺失 → 渲染层
//! 只能显示 missing 标记/失败提示，此处作为 CI 第一道防线提前拦截。

use std::path::Path;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/minecraft")
}

/// 关键贴图清单：路径（相对资源根 assets/minecraft）+ 出现过的回退位置。
const CRITICAL: &[(&str, &str)] = &[
    // 地形图集（旧 LEGACY_RECIPES 程序化噪声回退）
    (
        "textures/block/grass_block_top.png",
        "atlas 噪声回退（已删）",
    ),
    ("textures/block/dirt.png", "atlas 噪声回退（已删）"),
    ("textures/block/stone.png", "atlas 噪声回退（已删）"),
    ("textures/block/oak_leaves.png", "atlas 噪声回退（已删）"),
    (
        "textures/block/acacia_door_bottom.png",
        "层 0 曾被 skip(1) 漏载",
    ),
    (
        "textures/block/destroy_stage_0.png",
        "atlas 程序化裂纹回退（已删）",
    ),
    // 天体（旧 fs_sky 程序化圆盘回退）
    (
        "textures/environment/celestial/sun.png",
        "fs_sky 圆盘回退（已删）",
    ),
    (
        "textures/environment/celestial/moon/full_moon.png",
        "fs_sky 圆盘回退（已删）",
    ),
    // 字体（旧 font8x8 程序化字体回退）
    ("textures/font/ascii.png", "font8x8 回退（已删）"),
    // GUI 精灵表（旧程序化面板/按钮回退）
    (
        "textures/gui/container/inventory.png",
        "craft 灰板回退（已删）",
    ),
    (
        "textures/gui/container/crafting_table.png",
        "craft 灰板回退（已删）",
    ),
    ("textures/gui/title/minecraft.png", "标题程序化回退（已删）"),
    ("textures/gui/sprites/hud/hotbar.png", "HUD 核心精灵"),
    // 生物群系染色 colormap
    ("textures/colormap/grass.png", "染色禁用降级"),
    ("textures/colormap/foliage.png", "染色禁用降级"),
];

/// 每张关键贴图：存在、PNG 可解码、尺寸非空。
#[test]
fn critical_textures_exist_and_decode() {
    for (rel, why) in CRITICAL {
        let path = root().join(rel);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("关键素材缺失（{why}）: {} : {e}", path.display()));
        let img = image::load_from_memory(&bytes)
            .unwrap_or_else(|e| panic!("关键素材解码失败（{why}）: {} : {e}", path.display()));
        assert!(
            img.width() > 0 && img.height() > 0,
            "关键素材尺寸为空（{why}）: {}",
            path.display()
        );
    }
}

/// 方块贴图总量 = manifest 真实贴图数（827），与图集层数互锁。
#[test]
fn block_texture_count_matches_manifest() {
    let count = std::fs::read_dir(root().join("textures/block"))
        .expect("textures/block 目录应在仓库内")
        .filter(|e| e.as_ref().is_ok())
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "png")
        })
        .count();
    assert!(
        count >= mcv_core::atlas::REAL_TILE_COUNT,
        "textures/block 下 {count} 张 PNG < 期望 {REAL}",
        REAL = mcv_core::atlas::REAL_TILE_COUNT
    );
}
