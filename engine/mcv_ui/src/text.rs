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
        let candidate_w = text_width(&format!("{line}{word}"), scale);
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

    // 宽度锚点（测试进程用 mcv_render 回退 8x8 字体，无 Renderer 干扰）：
    // 'R'=8 'e'=7 's'=7 't'=7 'o'=8 'r'=7 空格=4 'D'=8 'f'=7 'a'=8 'u'=8
    // 'l'=6 → "Restore Defaults" = 113、"Restore Defaul" = 99；
    // CJK 走 cjk.f16 全宽 advance=9（unifont.rs FULL_ADVANCE）。
    // ASCII 有 MC 素材环境不一致的风险，故锚定回退字体并在 CI 同源。

    #[test]
    fn clip_head_ascii_and_cjk() {
        assert_eq!(
            clip_head("Restore Defaults", 113.0, 1.0),
            "Restore Defaults"
        );
        assert_eq!(clip_head("Restore Defaults", 100.0, 1.0), "Restore Defaul");
        // 首字符都放不下 → 空
        assert_eq!(clip_head("Restore", 3.0, 1.0), "");
        // 中文逐字断（中文溢出的统一解法：放不下就截）
        assert_eq!(clip_head("按键绑定界面标题", 20.0, 1.0), "按键");
        assert_eq!(
            clip_head("按键绑定界面标题", 200.0, 1.0),
            "按键绑定界面标题"
        );
    }

    #[test]
    fn clip_tail_keeps_suffix() {
        assert_eq!(
            clip_tail("Restore Defaults", 400.0, 1.0),
            "Restore Defaults"
        );
        assert_eq!(clip_tail("按键绑定界面标题", 20.0, 1.0), "标题");
        assert_eq!(clip_tail("Restore", 3.0, 1.0), "");
    }

    #[test]
    fn wrap_at_word_boundaries() {
        // "aaaa"=32，"aaaa bbbb"=68 > 40 → 一行一词
        let lines = wrap("aaaa bbbb cccc dddd", 40.0, 1.0);
        assert_eq!(lines, vec!["aaaa", "bbbb", "cccc", "dddd"]);
        let one = wrap("aaaa", 400.0, 1.0);
        assert_eq!(one, vec!["aaaa"]);
        let empty = wrap("", 400.0, 1.0);
        assert_eq!(empty, vec![""]);
    }

    #[test]
    fn wrap_hard_breaks_oversized_cjk() {
        // 无空格长串 → 逐字断（每 2 字一行：18 ≤ 24 < 27）
        let lines = wrap("一二三四五六七八九十", 24.0, 1.0);
        assert_eq!(lines, vec!["一二", "三四", "五六", "七八", "九十"]);
        for l in &lines {
            assert!(text_width(l, 1.0) <= 24.0);
        }
        assert_eq!(lines.concat(), "一二三四五六七八九十");
    }
}
