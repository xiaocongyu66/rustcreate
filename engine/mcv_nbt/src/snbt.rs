//! SNBT 最小集:parser + writer,语义对照原版 TagParser/SnbtPrinterTagVisitor
//! 的公开格式(非代码)。
//!
//! 支持:compound `{k:v,…}`(键可裸可引号)、list `[a,b]`、带后缀标量
//! `1b 2s 3L 4.5f 6.7d`(后缀 l/f/d 大小写均可)、无后缀 int/double(含
//! `1e3` 指数)、NaN/Infinity、单/双引号字符串(转义 `\\ \' \" \n \r \t
//! \uXXXX`)、裸串 `[0-9A-Za-z_\-.+]`、`:` 分隔、`,` 容忍尾逗号、`//` 行注释、
//! 数组糖 `[B;1b,2b]` `[I;…]` `[L;…]`。
//!
//! 已知缺口(登记为债,见 KNOWN-DIVERGENCE.md NBT-SNBT):十六进制整数
//! (`0x..`)、数字内下划线分隔符(`1_000`)、`[;]` 显式空数组类型声明。

use crate::error::NbtError;
use crate::tag::{NbtCompound, NbtList, NbtValue};

/// 解析完整 SNBT 文本(不允许尾随数据)。
pub fn parse_snbt(input: &str) -> Result<NbtValue, NbtError> {
    let chars: Vec<char> = input.chars().collect();
    let mut p = Parser { s: &chars, pos: 0 };
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos < p.s.len() {
        return Err(p.err_here("trailing data"));
    }
    Ok(v)
}

/// 解析并要求根为 compound。
pub fn parse_snbt_compound(input: &str) -> Result<NbtCompound, NbtError> {
    match parse_snbt(input)? {
        NbtValue::Compound(c) => Ok(c),
        other => Err(NbtError::BadSnbt {
            offset: 0,
            msg: format!("expected compound, got {:?}", other.tag_type()),
        }),
    }
}

/// 把 tag 写成 SNBT(与 [`parse_snbt`] 往返一致)。
pub fn write_snbt(v: &NbtValue, out: &mut String) {
    match v {
        NbtValue::End => out.push_str("0b"),
        NbtValue::Byte(b) => out.push_str(&format!("{b}b")),
        NbtValue::Short(s) => out.push_str(&format!("{s}s")),
        NbtValue::Int(i) => out.push_str(&i.to_string()),
        NbtValue::Long(l) => out.push_str(&format!("{l}L")),
        NbtValue::Float(f) => out.push_str(&float_text(*f as f64, 'f')),
        NbtValue::Double(d) => out.push_str(&float_text(*d, 'd')),
        NbtValue::ByteArray(a) => {
            if a.is_empty() {
                out.push_str("[]");
            } else {
                out.push_str("[B;");
                for (i, b) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&format!("{b}b"));
                }
                out.push(']');
            }
        }
        NbtValue::IntArray(a) => write_num_array(out, "I", a.iter().map(|v| v.to_string())),
        NbtValue::LongArray(a) => write_num_array(out, "L", a.iter().map(|v| format!("{v}L"))),
        NbtValue::String(s) => write_quoted(s, out),
        NbtValue::List(l) => {
            out.push('[');
            for (i, item) in l.items().iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_snbt(item, out);
            }
            out.push(']');
        }
        NbtValue::Compound(c) => {
            out.push('{');
            for (i, (k, val)) in c.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_key(k, out);
                out.push(':');
                write_snbt(val, out);
            }
            out.push('}');
        }
    }
}

fn write_num_array(out: &mut String, mark: &str, items: impl Iterator<Item = String>) {
    let parts: Vec<String> = items.collect();
    if parts.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str(&format!("[{mark};"));
    out.push_str(&parts.join(","));
    out.push(']');
}

/// 浮点 SNBT 文本:整数值的 float 必须带 `.0`(无后缀会被 parse 成 int),
/// inf/NaN 用原版拼写。后缀 `f`/`d`:`d` 可省(无后缀即 double)。
fn float_text(v: f64, suffix: char) -> String {
    let body = if v.is_nan() {
        "NaN".to_string()
    } else if v.is_infinite() {
        if v > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        }
    } else {
        let s = v.to_string();
        // "1"、"1e300" 这类无小数点的写法补 .0,保证 parse 回来仍是浮点。
        if s.chars().any(|c| c == '.' || c == 'e' || c == 'E') {
            s
        } else {
            format!("{s}.0")
        }
    };
    if suffix == 'f' {
        format!("{body}f")
    } else {
        body
    }
}

fn is_bare_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

fn is_bare_key(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_bare_char)
}

fn write_key(k: &str, out: &mut String) {
    if is_bare_key(k) {
        out.push_str(k);
    } else {
        write_quoted(k, out);
    }
}

fn write_quoted(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    s: &'a [char],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn err_here(&self, msg: &str) -> NbtError {
        NbtError::BadSnbt {
            offset: self.pos,
            msg: msg.to_string(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.s.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        loop {
            while self.pos < self.s.len() && self.s[self.pos].is_whitespace() {
                self.pos += 1;
            }
            // `//` 行注释(原版 parseAsArgument 容忍的宽松面)。
            if self.s.get(self.pos) == Some(&'/') && self.s.get(self.pos + 1) == Some(&'/') {
                while self.pos < self.s.len() && self.s[self.pos] != '\n' {
                    self.pos += 1;
                }
            } else {
                return;
            }
        }
    }

    fn expect(&mut self, c: char) -> Result<(), NbtError> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err_here(&format!("expected `{c}`")))
        }
    }

    fn parse_value(&mut self) -> Result<NbtValue, NbtError> {
        match self.peek() {
            Some('{') => self.parse_compound(),
            Some('[') => self.parse_list_or_array(),
            Some('\'') | Some('"') => Ok(NbtValue::String(self.parse_quoted()?)),
            Some(c) if is_bare_char(c) => {
                let start = self.pos;
                while self.pos < self.s.len() && is_bare_char(self.s[self.pos]) {
                    self.pos += 1;
                }
                let raw: String = self.s[start..self.pos].iter().collect();
                Ok(parse_bare(&raw).unwrap_or(NbtValue::String(raw)))
            }
            other => Err(self.err_here(&format!("expected a value, got {other:?}"))),
        }
    }

    fn parse_compound(&mut self) -> Result<NbtValue, NbtError> {
        self.expect('{')?;
        let mut c = NbtCompound::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(NbtValue::Compound(c));
        }
        loop {
            self.skip_ws();
            let key = match self.peek() {
                Some('\'') | Some('"') => self.parse_quoted()?,
                Some(ch) if is_bare_char(ch) => {
                    let start = self.pos;
                    while self.pos < self.s.len() && is_bare_char(self.s[self.pos]) {
                        self.pos += 1;
                    }
                    self.s[start..self.pos].iter().collect()
                }
                _ => return Err(self.err_here("expected compound key")),
            };
            self.skip_ws();
            self.expect(':')?;
            self.skip_ws();
            let v = self.parse_value()?;
            c.insert(key, v);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                    self.skip_ws();
                    // 容忍尾逗号: `{a:1,}`。
                    if self.peek() == Some('}') {
                        self.pos += 1;
                        return Ok(NbtValue::Compound(c));
                    }
                }
                Some('}') => {
                    self.pos += 1;
                    return Ok(NbtValue::Compound(c));
                }
                _ => return Err(self.err_here("expected `,` or `}` in compound")),
            }
        }
    }

    fn parse_list_or_array(&mut self) -> Result<NbtValue, NbtError> {
        self.expect('[')?;
        // 数组糖:[B; / [I; / [L;(标记与分号间容忍空白)。
        if let Some(mark) = self.peek()
            && matches!(mark, 'B' | 'I' | 'L')
        {
            let mut probe = self.pos + 1;
            while self.s.get(probe) == Some(&' ') || self.s.get(probe) == Some(&'\t') {
                probe += 1;
            }
            if self.s.get(probe) == Some(&';') {
                self.pos = probe + 1;
                return self.parse_array_sugar(mark);
            }
        }
        // 普通 list。
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(NbtValue::List(NbtList::new()));
        }
        loop {
            self.skip_ws();
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(']') {
                        self.pos += 1;
                        break;
                    }
                }
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err_here("expected `,` or `]` in list")),
            }
        }
        // 混型字面量照原版:类型取首元素,后续类型不符仍保留。
        Ok(NbtValue::List(NbtList::from_vec(items)))
    }

    fn parse_array_sugar(&mut self, mark: char) -> Result<NbtValue, NbtError> {
        // 三个候选槽,只填 mark 对应的那条(结构直白,免二次映射丢元素)。
        let mut bytes: Vec<i8> = Vec::new();
        let mut ints: Vec<i32> = Vec::new();
        let mut longs: Vec<i64> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(match mark {
                'B' => NbtValue::ByteArray(bytes),
                'I' => NbtValue::IntArray(ints),
                _ => NbtValue::LongArray(longs),
            });
        }
        loop {
            self.skip_ws();
            let start = self.pos;
            while self.pos < self.s.len() && is_bare_char(self.s[self.pos]) {
                self.pos += 1;
            }
            if self.pos == start {
                return Err(self.err_here("expected a number in array"));
            }
            let raw: String = self.s[start..self.pos].iter().collect();
            let v = parse_bare(&raw).ok_or_else(|| self.err_here("bad number in array"))?;
            match mark {
                // 数组糖允许无后缀/异型后缀标量,但值必须在目标类型域内
                // (原版越域直接报错,同语义)。
                'B' => {
                    let n = v
                        .as_i32()
                        .ok_or_else(|| self.err_here("non-numeric in [B;]"))?;
                    if !(i8::MIN as i32..=i8::MAX as i32).contains(&n) {
                        return Err(self.err_here("[B;] element out of byte range"));
                    }
                    bytes.push(n as i8);
                }
                'I' => {
                    let n = v
                        .as_i64()
                        .ok_or_else(|| self.err_here("non-numeric in [I;]"))?;
                    let Ok(n32) = i32::try_from(n) else {
                        return Err(self.err_here("[I;] element out of int range"));
                    };
                    // 分数(如 1.5)经 as_i64 截断——原版也拒绝带小数元素,
                    // 但域内整数化偏差登记为宽容(见 KNOWN-DIVERGENCE)。
                    ints.push(n32);
                }
                _ => {
                    let n = v
                        .as_i64()
                        .ok_or_else(|| self.err_here("non-numeric in [L;]"))?;
                    longs.push(n);
                }
            }
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err_here("expected `,` or `]` in array")),
            }
        }
        Ok(match mark {
            'B' => NbtValue::ByteArray(bytes),
            'I' => NbtValue::IntArray(ints),
            _ => NbtValue::LongArray(longs),
        })
    }

    fn parse_quoted(&mut self) -> Result<String, NbtError> {
        let quote = self.s[self.pos];
        self.pos += 1;
        let mut out = String::new();
        loop {
            let c = *self
                .s
                .get(self.pos)
                .ok_or_else(|| self.err_here("unterminated string"))?;
            self.pos += 1;
            match c {
                q if q == quote => return Ok(out),
                '\\' => {
                    let e = *self
                        .s
                        .get(self.pos)
                        .ok_or_else(|| self.err_here("unterminated escape"))?;
                    self.pos += 1;
                    match e {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        '\\' => out.push('\\'),
                        '\'' => out.push('\''),
                        '"' => out.push('"'),
                        'u' => {
                            let mut cp: u32 = 0;
                            for _ in 0..4 {
                                let h = *self
                                    .s
                                    .get(self.pos)
                                    .ok_or_else(|| self.err_here("bad \\u escape"))?;
                                self.pos += 1;
                                cp = cp * 16
                                    + h.to_digit(16).ok_or_else(|| self.err_here("bad hex"))?;
                            }
                            let ch = char::from_u32(cp)
                                .ok_or_else(|| self.err_here("lone surrogate in \\u"))?;
                            out.push(ch);
                        }
                        other => return Err(self.err_here(&format!("bad escape `\\{other}`"))),
                    }
                }
                other => out.push(other),
            }
        }
    }
}

/// 裸 token → 标量值;不是合法数字则 None(调用侧按 String 处理)。
fn parse_bare(raw: &str) -> Option<NbtValue> {
    // 原版拼写的特殊常量(NaN/Infinity),可带 f/d 后缀。
    let (body, suffix) = split_suffix(raw);
    match body {
        "NaN" | "nan" => {
            return Some(match suffix {
                Some('f') => NbtValue::Float(f32::NAN),
                _ => NbtValue::Double(f64::NAN),
            });
        }
        "Infinity" | "-Infinity" | "+Infinity" => {
            let inf = if body.starts_with('-') {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
            return Some(match suffix {
                Some('f') => NbtValue::Float(inf as f32),
                _ => NbtValue::Double(inf),
            });
        }
        _ => {}
    }
    if !looks_numeric(body) {
        return None;
    }
    match suffix {
        Some('b') => body.parse::<i8>().ok().map(NbtValue::Byte),
        Some('s') => body.parse::<i16>().ok().map(NbtValue::Short),
        Some('l') => body.parse::<i64>().ok().map(NbtValue::Long),
        Some('f') => body.parse::<f32>().ok().map(NbtValue::Float),
        Some('d') => body.parse::<f64>().ok().map(NbtValue::Double),
        None => {
            if let Ok(i) = body.parse::<i32>() {
                return Some(NbtValue::Int(i));
            }
            // 无后缀:整数溢出 i32 时(如 9999999999)→ 按裸串处理(登记宽容)。
            body.parse::<f64>().ok().map(NbtValue::Double)
        }
        // split_suffix 已把后缀规范化为小写。
        _ => None,
    }
}

/// 拆尾部类型后缀;返回(主体, 规范化后缀)。后缀只在整体像数字时剥离,
/// 避免把 "abc"(b 结尾)、"123abc" 误判。
fn split_suffix(raw: &str) -> (&str, Option<char>) {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return (raw, None);
    }
    let last = raw[raw.len() - 1..].chars().next().unwrap();
    if !matches!(last, 'b' | 's' | 'l' | 'L' | 'f' | 'd' | 'D') {
        return (raw, None);
    }
    let body = &raw[..raw.len() - last.len_utf8()];
    let body_like_number = looks_numeric(body)
        || matches!(body, "NaN" | "nan" | "Infinity" | "-Infinity" | "+Infinity");
    if body_like_number {
        (body, Some(last.to_ascii_lowercase()))
    } else {
        (raw, None)
    }
}

/// 是否形如 `[+-]?digits(.digits)?([eE][+-]?digits)?`,且至少一位数字。
fn looks_numeric(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mut digits = false;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits = true;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits = true;
        }
    }
    if digits && i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let mut exp_digits = false;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            exp_digits = true;
        }
        if !exp_digits {
            return false;
        }
    }
    digits && i == b.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(src: &str) {
        let v = parse_snbt(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"));
        let mut text = String::new();
        write_snbt(&v, &mut text);
        let v2 =
            parse_snbt(&text).unwrap_or_else(|e| panic!("reparse {text:?} (from {src:?}): {e}"));
        assert_eq!(v, v2, "roundtrip text: {text:?}");
    }

    #[test]
    fn scalar_suffixed_and_bare() {
        roundtrip("{a:1b,b:2s,c:3,d:4L,e:4.5f,g:6.7d,h:-8,i:1e3,j:\"x k\",k:abc}");
    }

    #[test]
    fn lists_and_arrays() {
        roundtrip("{a:[1,2,3],b:[I;1,2,3],c:[B;1b,2b],d:[L;5L,6L],e:[],f:[[1],[2]]}");
    }

    #[test]
    fn comments_and_trailing_commas() {
        let v = parse_snbt("{\n // comment\n a:1, // trailing comma ok\n b:[1,2,],\n}").unwrap();
        assert_eq!(v.as_compound().unwrap().get_i32("a"), Some(1));
        assert_eq!(v.as_compound().unwrap().get_list("b").unwrap().len(), 2);
    }

    #[test]
    fn nested_quotes_and_escapes() {
        roundtrip(r#"{s:"he said \"hi\"\n\ttab\\",t:'sq \u00e9'}"#);
    }

    #[test]
    fn nan_infinity() {
        roundtrip("{a:NaN,b:-Infinity,c:1.5f}");
    }

    #[test]
    fn bad_input_reports_offset() {
        let e = parse_snbt("{a:}").unwrap_err();
        assert!(matches!(e, NbtError::BadSnbt { .. }), "{e}");
    }
}
