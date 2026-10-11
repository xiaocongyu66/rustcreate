//! lang 双语名（W 波 #137 刀3）：ci/gen-lang.py 物品名表产物判据。
//!
//! 数据源：game/mcv_app/data/lang/{en_us,zh_cn}.json（Mojang 官方全量，
//! 键名 block.minecraft.<注册名> / item.minecraft.<注册名>）；生成器按
//! mcv_item::ITEMS 注册名与 kind 推键，产 compact/items_table.rs
//! （(注册名, en, zh) 行）+ compact/{en,zh}_items.json（人工审阅用）。
//! 产物随仓提交（DEVELOP_ONLY 版权政策同 compact 主表）。

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// 重跑生成器后读取产物（测试同时验证脚本可复现，产物入库且与源同步）。
/// OnceLock：多个测试并发跑同一 test binary，脚本 write_text 截断文件的
/// 瞬间另一线程读到空表会假红（CI run 38156491631 实锤）——生成+同步断言
/// 全程只执行一次。
fn items_table_text() -> &'static str {
    static TABLE: OnceLock<String> = OnceLock::new();
    TABLE.get_or_init(items_table_regenerated)
}

fn items_table_regenerated() -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("mcv_app 位于 game/ 下一层");
    let table = root.join("game/mcv_app/data/lang/compact/items_table.rs");
    let committed = std::fs::read_to_string(&table).unwrap_or_default();
    let script = root.join("ci/gen-lang.py");
    let out = Command::new("python3")
        .arg(&script)
        .current_dir(root)
        .output()
        .expect("运行 ci/gen-lang.py");
    assert!(
        out.status.success(),
        "gen-lang.py 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let regenerated =
        std::fs::read_to_string(&table).unwrap_or_else(|e| panic!("读 {}: {e}", table.display()));
    assert_eq!(
        committed, regenerated,
        "items_table.rs 与生成器输出不同步：改 ITEMS 后须重跑 ci/gen-lang.py 再提交"
    );
    regenerated
}

/// 生成器必须已产物品名表且覆盖木族新件（表随源重生成）。
#[test]
fn items_table_covers_wood_family() {
    let t = items_table_text();
    for (name, en, zh) in [
        ("oak_stairs", "Oak Stairs", "橡木楼梯"),
        ("spruce_planks", "Spruce Planks", "云杉木板"),
        (
            "dark_oak_pressure_plate",
            "Dark Oak Pressure Plate",
            "深色橡木压力板",
        ),
        ("honeycomb", "Honeycomb", "蜜脾"),
        ("honey_bottle", "Honey Bottle", "蜂蜜瓶"),
        ("beehive", "Beehive", "蜂箱"),
        ("crafting_table", "Crafting Table", "工作台"),
        ("chest", "Chest", "箱子"),
    ] {
        let row = format!(r#"("{name}", "{en}", "{zh}")"#);
        assert!(t.contains(&row), "items_table 缺行 {row}");
    }
}

/// 表行数 = ITEMS 全量减 legacy 名（无官方 key 的回退件不产行）——
/// 至少要覆盖 62 件新件与全部 modern 名旧件。
#[test]
fn items_table_covers_modern_names() {
    let t = items_table_text();
    let n = t.matches('(').count();
    assert!(n >= 100, "items_table 行数过少: {n}");
}

/// legacy 物品（旧 14 方块名）无官方 key——生成器跳过，运行时回退注册名；
/// 这里锁"不产错行"：不得为 legacy 名伪造 key 行。
#[test]
fn items_table_skips_legacy_names() {
    let t = items_table_text();
    // grass 是官方键（block.minecraft.grass="Grass"/"草"，26.1 保留旧名），
    // 不在 legacy 名之列；log/planks/leaves 无官方 key，运行时回退注册名。
    for legacy in ["(\"log\",", "(\"planks\",", "(\"leaves\","] {
        assert!(
            !t.contains(legacy),
            "legacy 名 {legacy} 不该有官方 key 行（运行时回退注册名）"
        );
    }
}
