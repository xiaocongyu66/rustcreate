//! 文本裁剪与折行（对照 26.1 `Font.java` 文本布局）：
//! - `clip_head/clip_tail` = `Font.plainSubstrByWidth`
//!   （Font.java:197-206，plainHeadByWidth/plainTailByWidth）：按 advance
//!   累计截取前/后缀；
//! - `wrap` = tooltip 折行：`Tooltip.MAX_WIDTH = 170`
//!   （Tooltip.java:17、splitTooltip :69-71 调 `font.split(message, 170)`）。
//!   原版 StringSplitter 优先按词断行；CJK 无空格回退逐字断行。

use mcv_render::text::text_width;

/// tooltip 折行宽度上限（Tooltip.java:17）。
pub const TOOLTIP_MAX_WIDTH: f32 = 170.0;

/// 按宽度截头：返回能放进 `max_w` 的最长前缀（空串/首字就超宽 → 空）。
pub fn clip_head(s: &str, max_w: f32, scale: f32) -> &str {
    let mut end = s.len();
    let mut acc = 0.0;
    for (i, ch) in s.char_indices() {
        let adv = text_width(ch.encode_utf8(&mut [0u8; 4]), scale);
        if acc + adv > max_w {
            end = i;
            break;
        }
        acc += adv;
        end = i + ch.len_utf8();
    }
    &s[..end]
}

/// 按宽度截尾（plainTailByWidth）：返回能放进 `max_w` 的最长后缀。
pub fn clip_tail(s: &str, max_w: f32, scale: f32) -> &str {
    let mut start = 0usize;
    let mut acc = 0.0;
    for (i, ch) in s.char_indices().rev() {
        let adv = text_width(ch.encode_utf8(&mut [0u8; 4]), scale);
        if acc + adv > max_w {
            start = i + ch.len_utf8();
            break;
        }
        acc += adv;
        start = i;
    }
    &s[start..]
}

/// 折行：词边界优先；单个词超宽时逐字硬断（CJK 无空格场景）。
pub fn wrap(s: &str, max_w: f32, scale: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        // 空词 = 连续空格或行首空格：保留一个空位
        if word.is_empty() {
            if !line.is_empty() {
                line.push(' ');
            }
            continue;
        }
        // 候选宽含连接空格（StringSplitter 量 line + ' ' + word；
        // 少算空格会放出超宽行）
        let joined = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        let candidate_w = text_width(&joined, scale);
        if line.is_empty() && text_width(word, scale) > max_w {
            // 词本身超宽 → 逐字硬断
            let mut chunk = String::new();
            for ch in word.chars() {
                let mut buf = [0u8; 4];
                let c = ch.encode_utf8(&mut buf);
                if !chunk.is_empty() && text_width(&format!("{chunk}{c}"), scale) > max_w {
                    lines.push(std::mem::take(&mut chunk));
                }
                chunk.push_str(c);
            }
            line = chunk;
        } else if !line.is_empty() && candidate_w > max_w {
            lines.push(std::mem::take(&mut line));
            line.push_str(word);
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    // 宽度期望全部用 text_width 动态测量（任务 #53 素材红线后回退
    // 8x8 字体已删：测试进程未装载 ascii.png 时宽度表为常数 9/字，
    // 装载后为 MC 原版 advance——两种环境断言均须成立，故不锚定
    // 具体宽度数值，阈值由被测函数同款 text_width 算出）。

    #[test]
    fn clip_head_ascii_and_cjk() {
        // 恰好放得下全宽 → 原样返回
        assert_eq!(
            clip_head("Restore Defaults", text_width("Restore Defaults", 1.0), 1.0),
            "Restore Defaults"
        );
        // 阈值取前 14 字符宽 → 截到该处（下一字符 advance > 0）
        let th = text_width("Restore Defaul", 1.0);
        assert_eq!(clip_head("Restore Defaults", th, 1.0), "Restore Defaul");
        // 首字符都放不下 → 空
        assert_eq!(clip_head("Restore", text_width("R", 1.0) - 1.0, 1.0), "");
        // 中文逐字断（中文溢出的统一解法：放不下就截）
        let cjk = "按键绑定界面标题";
        assert_eq!(clip_head(cjk, text_width("按键", 1.0), 1.0), "按键");
        assert_eq!(clip_head(cjk, text_width("按键绑", 1.0) - 1.0, 1.0), "按键");
        assert_eq!(clip_head(cjk, text_width(cjk, 1.0), 1.0), cjk);
    }

    #[test]
    fn clip_tail_keeps_suffix() {
        assert_eq!(
            clip_tail("Restore Defaults", text_width("Restore Defaults", 1.0), 1.0),
            "Restore Defaults"
        );
        assert_eq!(
            clip_tail("按键绑定界面标题", text_width("标题", 1.0), 1.0),
            "标题"
        );
        assert_eq!(
            clip_tail("按键绑定界面标题", text_width("面标题", 1.0) - 1.0, 1.0),
            "标题"
        );
        assert_eq!(clip_tail("Restore", text_width("e", 1.0) - 1.0, 1.0), "");
    }

    #[test]
    fn wrap_at_word_boundaries() {
        // 阈值 = 单词宽：一行一词（词间空格使两词连排必超宽）
        let th = text_width("aaaa", 1.0);
        let lines = wrap("aaaa bbbb cccc dddd", th, 1.0);
        assert_eq!(lines, vec!["aaaa", "bbbb", "cccc", "dddd"]);
        let one = wrap("aaaa", 400.0, 1.0);
        assert_eq!(one, vec!["aaaa"]);
        // 两词放得下 → 同行带空格
        let two = wrap("aaaa bbbb", text_width("aaaa bbbb", 1.0), 1.0);
        assert_eq!(two, vec!["aaaa bbbb"]);
        let empty = wrap("", 400.0, 1.0);
        assert_eq!(empty, vec![""]);
    }

    #[test]
    fn wrap_hard_breaks_oversized_cjk() {
        // 无空格长串 → 逐字断；阈值取 2 字宽（CJK advance 全宽均一，
        // 3 字必超）
        let th = text_width("一二", 1.0);
        let lines = wrap("一二三四五六七八九十", th, 1.0);
        assert_eq!(lines, vec!["一二", "三四", "五六", "七八", "九十"]);
        for l in &lines {
            assert!(text_width(l, 1.0) <= th);
        }
        assert_eq!(lines.concat(), "一二三四五六七八九十");
    }
}
