//! i18n 基础设施：编译期静态翻译表 + key 查询。
//!
//! ## 数据来源（DEVELOP_ONLY）
//! compact 表由 `ci/gen-lang.py` 从 Mojang 官方语言文件
//! （`data/lang/en_us.json` / `zh_cn.json`，即 26.1.jar / 资产服务器抽出的
//! 原版文案）挑出引擎 UI 用到的 ≤40 个 key 生成，位于
//! `data/lang/compact/table.rs`。官方语言文件版权归 Mojang，政策与材质包
//! 素材相同：开发期素材，发布前删除或替换为自维护表（`mcv.` 前缀 key 已是
//! 自维护的 `data/lang/extra.json`）。改 key 后在仓库根目录重跑
//! `python3 ci/gen-lang.py` 再提交。
//!
//! ## 运行时策略
//! 表经 `include!` 编入二进制（编译期常量，无 JSON 运行时、无 lazy 初始化、
//! 无堆分配）；只嵌 compact 子集不嵌全量，版权面与体积都小（表 <2 KiB）。
//!
//! ## ⚠️ 中文字体阻塞项（接入前 t() 的 zh 值无法实际显示）
//! 当前引擎字体层（`mcv_render::font`，8x8 单色位图，font8x8 公版）只覆盖
//! U+0000..U+007F，**没有 CJK 字形**——`t(Lang::Zh, ...)` 返回的中文字符串
//! 送进 `mcv_render::text_quads` 会画不出字形（缺字）。
//! MC 官方 CJK 字形不在 jar 的 `assets/minecraft/textures/font/unifont*`
//! （26.1 已查证：该目录只有 ascii/nonlatin_european/accented 等拉丁字形）；
//! 26.1 的 unifont 走**资产系统**分发（asset index v30）：
//! - `minecraft/font/unifont.zip`（含 unifont_page_*.png 位图字形）
//! - `minecraft/font/unifont_jp.zip`、`minecraft/font/unifont_pua.zip`
//! - jar 内 `assets/minecraft/font/include/unifont.json`（26.1 里 providers
//!   为空壳，实际定义在资产 zip 与 default/uniform.json 字体定义中）
//!
//! 中文真正上屏需要字体层后续接入 unifont 字形图集（或自带开源 CJK 位图
//! 字体如 unifont 本尊，GPL+例外需评估）扩展 `mcv_render::font` 的码点覆盖。
//! 字体层就绪前，`t()` 的 zh 值仅对 ASCII 文案有显示意义；接线时中文界面
//! 显示为空的缺字属预期，勿当 i18n 表 bug。

/// 界面语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

// 编译期静态表，由 ci/gen-lang.py 生成（勿手改）。
// include! 相对本 .rs 文件解析路径，故用 CARGO_MANIFEST_DIR 定位。
mod table {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/lang/compact/table.rs"
    ));
}

/// 查 `key` 在 `lang` 下的文案。
///
/// 回退链：目标语言 → en → key 本身（永不 panic，未知 key 显示原始 key
/// 便于排查漏配）。表很小（≤40 项），线性扫比二分/哈希快。
///
/// 值里可能含 `%s` 占位（官方与 extra.json 同风格），调用方自行
/// `replace("%s", ...)`；带参文案也可优先选 extra.json 里的整句 key
/// （如 `mcv.options.renderDistanceChunk`）。
///
/// ⚠️ `Lang::Zh` 返回的中文字符串当前受字体层限制无法上屏，见模块头注释。
pub fn t(lang: Lang, key: &str) -> &str {
    let rows = table::TABLE.as_slice();
    // 先按 (key, lang) 精确找
    if let Some(row) = rows.iter().find(|(k, ..)| *k == key) {
        return match lang {
            Lang::En => row.1,
            Lang::Zh => row.2,
        };
    }
    // 表内 key 两语言都齐（gen-lang.py 保证 en/zh 同源同 key），故未命中
    // 即表外 key，直接回退 key 本身。
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    // 断言值全部抄自 data/lang/{en_us,zh_cn}.json 实际内容（26.1），勿凭记忆改。

    #[test]
    fn en_matches_official() {
        assert_eq!(t(Lang::En, "menu.singleplayer"), "Singleplayer");
        assert_eq!(t(Lang::En, "gameMode.survival"), "Survival Mode");
        assert_eq!(t(Lang::En, "menu.paused"), "Game Paused");
        assert_eq!(t(Lang::En, "options.renderDistance"), "Render Distance");
        assert_eq!(t(Lang::En, "gui.back"), "Back");
    }

    #[test]
    fn zh_matches_official() {
        assert_eq!(t(Lang::Zh, "menu.singleplayer"), "单人游戏");
        assert_eq!(t(Lang::Zh, "gameMode.survival"), "生存模式");
        assert_eq!(t(Lang::Zh, "menu.paused"), "游戏暂停");
        assert_eq!(t(Lang::Zh, "options.renderDistance"), "渲染距离");
        assert_eq!(t(Lang::Zh, "gui.back"), "返回");
    }

    #[test]
    fn extra_keys_present() {
        assert_eq!(t(Lang::Zh, "mcv.mode.hardcore"), "极限");
        assert_eq!(t(Lang::En, "mcv.mode.hardcore"), "Hardcore");
    }

    #[test]
    fn unknown_key_falls_back_to_key() {
        assert_eq!(t(Lang::En, "no.such.key.exists"), "no.such.key.exists");
        assert_eq!(t(Lang::Zh, "no.such.key.exists"), "no.such.key.exists");
    }

    #[test]
    fn table_is_sane() {
        // key 不得重复，且不得有值含 %s 而另一语言漏 %s
        let rows = table::TABLE.as_slice();
        for i in 0..rows.len() {
            for j in i + 1..rows.len() {
                assert_ne!(rows[i].0, rows[j].0, "duplicate key {}", rows[i].0);
            }
            assert_eq!(
                rows[i].1.matches("%s").count(),
                rows[i].2.matches("%s").count(),
                "%s 占位数不匹配: {}",
                rows[i].0
            );
        }
    }
}
