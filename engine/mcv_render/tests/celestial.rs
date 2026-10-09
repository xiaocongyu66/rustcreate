//! 天体（太阳/月相）CPU 侧验证：原版贴图加载 + 月相时间映射。
//! 不依赖 gpu-tests（无 GPU 也能跑，CI `cargo test --workspace` 即覆盖）。

use mcv_render::celestial;

/// 工作区 assets/minecraft（仓库内已提交的原版贴图，与 app.rs 桌面同源）。
fn assets() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/minecraft")
}

#[test]
fn vanilla_celestial_payload() {
    // 9 层 = sun + MoonPhase 序八相（SkyRenderer.java:125-127 太阳 quad、
    // :149-157 八相 moon/<serializedName> quad；AtlasProvider.java:162 图集
    // 源目录 environment/celestial）。层内非空、层间内容不同（各相贴图互异）。
    let payload = celestial::load_payload(&assets()).expect("原版天体贴图应可加载");
    assert_eq!(
        payload.len(),
        celestial::CELESTIAL_LAYERS * 32 * 32 * 4,
        "9 层 32x32 RGBA"
    );
    let layer = |l: usize| &payload[l * 32 * 32 * 4..(l + 1) * 32 * 32 * 4];
    for l in 0..celestial::CELESTIAL_LAYERS {
        assert!(
            layer(l).chunks(4).any(|px| px[3] > 200),
            "层 {l} 不应有全透明"
        );
    }
    // 满月(层1)与新月(层5)贴图必须不同（防错位/重复贴同一张）。
    assert_ne!(layer(1), layer(5));
}

#[test]
fn moon_phase_timeline() {
    // 26.1 月相周期 192000 tick、每日一相（data/minecraft/timeline/moon.json
    // visual/moon_phase 关键帧 0..168000 逐日：full_moon → waxing_gibbous），
    // 序号即 MoonPhase.java 枚举序。
    assert_eq!(celestial::moon_phase(0), 0, "第 0 日 = 满月");
    assert_eq!(celestial::moon_phase(23_999), 0);
    assert_eq!(celestial::moon_phase(24_000), 1, "第 1 日 = waning_gibbous");
    assert_eq!(celestial::moon_phase(96_000), 4, "第 4 日 = new_moon");
    assert_eq!(
        celestial::moon_phase(168_000),
        7,
        "第 7 日 = waxing_gibbous"
    );
    assert_eq!(celestial::moon_phase(191_999), 7);
    assert_eq!(celestial::moon_phase(192_000), 0, "8 日周期回绕满月");
}
