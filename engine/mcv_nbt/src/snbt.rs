//! SNBT 最小集:parser + writer,语义对照原版 TagParser/SnbtPrinterTagVisitor
//! 的公开格式(非代码)。
//!
//! 支持:compound `{k:v,…}`(键可裸可引号)、list `[a,b]`、带后缀标量
//! `1b 2s 3L 4.5f 6.7d`(后缀 l/f/d 大小写均可)、无后缀 int/double(含
//! `1e3` 指数)、NaN/Infinity、单/双引号字符串(转义 `\\ \' \" \n \r \t
//! \uXXXX`)、裸串 `[0-9A-Za-z_\-.+]`、`:` 分隔、`,` 容忍尾逗号、`//` 行注释、
//! 数组糖 `[B;1b,2b]` `[I;…]` `[L;…]`(空数组显式声明 `[B;]` `[I;]` `[L;]`)。
//!
//! 数值面按 26.1 `SnbtGrammar`(packrat)实物:十六进制 `0x..`/二进制 `0b..`
//! (默认无符号、贪心读位、可组合 b/s/l 后缀)、数字内下划线(首尾非法、
//! 中间任意,NumberRunParseRule.java:36)、十进制整数禁前导零(cut 硬错)、
//! 无后缀超 i32 报错(原版 NumberFormatException「too big」)。数组域内
//! 元素只收整数字面量,小数/越域/非法后缀报错;后缀域矩阵按 ArrayPrefix
//! (B:{b} I:{b,s} L:{b,s,l},无后缀=数组类型)。
//!
//! 26.1 新增扩展面不在本底座子集(按 #177 口径不登记):`i`/`ub`/`ss` 等
//! 新后缀、`true`/`false` 布尔字面量、`bool()`/`uuid()` 内置调用;NaN/
//! Infinity 保留为本仓浮点字面量(26.1 packrat 已改判裸串,本仓 writer
//! 产 `NaN` 依赖其可读回,往返闭环自洽)。

use crate::error::NbtError;
use crate::tag::{NbtCompound, NbtList, NbtValue};

/// 解析完整 SNBT 文本(不允许尾随数据)。
pub fn parse_snbt(input: &str) -> Result<NbtValue, NbtError> {
    let chars: Vec<char> = input.chars().collect();
    let mut p = Parser {
        s: &chars,
        pos: 0,
        depth: 0,
    };
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
            // 空数组写显式类型声明(SnbtPrinterTagVisitor.visitByteArray:
            // 89-102 实物:空也输出 `[B;]`,`[]` 会读回空 list 破坏往返)。
            if a.is_empty() {
                out.push_str("[B;]");
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
    // 空数组显式 `[I;]`/`[L;]`(同 visitByteArray 空数组实物)。
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

/// SNBT 文本嵌套深度上限(与二进制侧 NbtAccounter 512 对齐)。
const MAX_SNBT_DEPTH: u32 = 512;

struct Parser<'a> {
    s: &'a [char],
    pos: usize,
    depth: u32,
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
        // 递归深度护栏:深层嵌套文本(`{{{{…`)不炸栈(原版 TagParser 亦
        // 递归,JVM 有栈护栏;本仓显式限深,错误路径不回退计数——解析
        // 已整体中止,计数无后续消费者)。
        if self.depth >= MAX_SNBT_DEPTH {
            return Err(self.err_here("nesting too deep"));
        }
        self.depth += 1;
        let v = self.parse_value_inner();
        self.depth -= 1;
        v
    }

    fn parse_value_inner(&mut self) -> Result<NbtValue, NbtError> {
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
                match classify_bare(&raw, NumKind::Int) {
                    Ok(Bare::Value(v)) => Ok(v),
                    Ok(Bare::Str) => Ok(NbtValue::String(raw)),
                    Err(msg) => Err(self.err_here(&msg)),
                }
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
        // 数组糖:[B; / [I; / [L;('[' 后、标记与分号间均容忍空白,原版
        // StringReader 终端逐项 skipWhitespace)。
        let mut probe = self.pos;
        while self.s.get(probe).is_some_and(char::is_whitespace) {
            probe += 1;
        }
        if let Some(mark) = self.s.get(probe).copied()
            && matches!(mark, 'B' | 'I' | 'L')
        {
            probe += 1;
            while self.s.get(probe).is_some_and(char::is_whitespace) {
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
        // 无后缀元素的默认目标类型=数组本身(SnbtGrammar ArrayPrefix
        // .computeType:SnbtGrammar.java:781)。
        let default = match mark {
            'B' => NumKind::Byte,
            'I' => NumKind::Int,
            _ => NumKind::Long,
        };
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
            // 数组域=intArrayEntries(纯 integerLiteral,intArrayEntries 用
            // integerLiteralRule:SnbtGrammar.java:639):小数/引号串/裸词
            // 一律拒绝(原版截断宽容已撤销)。
            let v = match classify_bare(&raw, default) {
                Ok(Bare::Value(
                    v @ (NbtValue::Byte(_)
                    | NbtValue::Short(_)
                    | NbtValue::Int(_)
                    | NbtValue::Long(_)),
                )) => v,
                Ok(Bare::Value(_)) => {
                    return Err(
                        self.err_here(&format!("`{raw}` is not an integer literal in array"))
                    );
                }
                Ok(Bare::Str) | Err(_) => {
                    return Err(self.err_here(&format!("bad integer literal `{raw}` in array")));
                }
            };
            // 后缀域矩阵(ArrayPrefix.isAllowed:SnbtGrammar.java:763):
            // B:{b}; I:{b,s,(i),无}; L:{b,s,(i),l,无}。`i` 后缀不在本仓子集。
            let kind = NumKind::of(&v);
            let allowed = match mark {
                'B' => kind == NumKind::Byte,
                'I' => matches!(kind, NumKind::Byte | NumKind::Short | NumKind::Int),
                _ => true,
            };
            if !allowed {
                return Err(self.err_here(&format!(
                    "invalid array element type for [{mark};] in `{raw}`"
                )));
            }
            match mark {
                'B' => bytes.push(v.as_i32().unwrap_or(0) as i8),
                'I' => ints.push(v.as_i32().unwrap_or(0)),
                _ => longs.push(v.as_i64().unwrap_or(0)),
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

/// 标量整数类型(SnbtGrammar TypeSuffix 的整数四元子集)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumKind {
    Byte,
    Short,
    Int,
    Long,
}

impl NumKind {
    fn of(v: &NbtValue) -> Self {
        match v {
            NbtValue::Byte(_) => NumKind::Byte,
            NbtValue::Short(_) => NumKind::Short,
            NbtValue::Int(_) => NumKind::Int,
            _ => NumKind::Long,
        }
    }
}

/// 裸 token 分类结果:数值 / 裸串 / 数值面非法(报错)。
enum Bare {
    Value(NbtValue),
    Str,
}

/// 裸 token → 标量。原版语义(SnbtGrammar.createParser):首字符为符号/
/// 数字/`.` 的 token 走数值分支,不合法即整体报错(cut 后回退 unquoted
/// 分支又因 canStartNumber 报 invalid_unquoted_start——两路都是错);
/// 字母/下划线开头只能是裸串(canStartNumber:SnbtGrammar.java:164)。
fn classify_bare(raw: &str, default: NumKind) -> Result<Bare, String> {
    if raw.is_empty() {
        return Ok(Bare::Str);
    }
    // 原版拼写的特殊常量(NaN/Infinity),可带 f/d 后缀(本仓保留字面量,
    // 见模块头注)。
    if let Some(v) = special_const(raw) {
        return Ok(Bare::Value(v));
    }
    let first = raw.as_bytes()[0];
    if !(first == b'+' || first == b'-' || first == b'.' || first.is_ascii_digit()) {
        return Ok(Bare::Str);
    }
    Ok(Bare::Value(parse_number(raw, default)?))
}

fn special_const(raw: &str) -> Option<NbtValue> {
    let (body, suffix) = match raw.as_bytes().last() {
        Some(c) if matches!(c, b'f' | b'F' | b'd' | b'D') => (&raw[..raw.len() - 1], Some(*c)),
        _ => (raw, None),
    };
    match body {
        "NaN" | "nan" => Some(match suffix {
            Some(b'f') | Some(b'F') => NbtValue::Float(f32::NAN),
            _ => NbtValue::Double(f64::NAN),
        }),
        "Infinity" | "+Infinity" => Some(match suffix {
            Some(b'f') | Some(b'F') => NbtValue::Float(f32::INFINITY),
            _ => NbtValue::Double(f64::INFINITY),
        }),
        "-Infinity" => Some(match suffix {
            Some(b'f') | Some(b'F') => NbtValue::Float(f32::NEG_INFINITY),
            _ => NbtValue::Double(f64::NEG_INFINITY),
        }),
        _ => None,
    }
}

/// 数字主入口:sign? 后分 hex(`0x`)、binary(`0b`)、十进制(整数/浮点)。
fn parse_number(raw: &str, default: NumKind) -> Result<NbtValue, String> {
    let b = raw.as_bytes();
    let mut i = 0;
    let minus = b[0] == b'-';
    if b[0] == b'+' || b[0] == b'-' {
        i = 1;
    }
    let body = &b[i..];
    if body.is_empty() {
        return Err("expected digits after sign".to_string());
    }
    if body[0] == b'0' && body.len() >= 2 && (body[1] | 0x20) == b'x' {
        return radix_int(body, minus, 16, 2, default, "hex");
    }
    if body[0] == b'0' && body.len() >= 2 && (body[1] | 0x20) == b'b' {
        // 原版 `0b`+非二进位回退为 marker「0」+后缀 b(SnbtGrammar.java:357
        // 二进位支无 cut,失败回退 :359 marker);整 token 恰为 `0b` 时
        // 即 Byte(0),如 `0b5` 则残留 → 错。
        let (digits, j, err) = scan_run(body, 2, |c| c == b'0' || c == b'1');
        if let Some(e) = err {
            return Err(e.to_string());
        }
        if digits.is_empty() {
            return if body.len() == 2 {
                Ok(NbtValue::Byte(0))
            } else {
                Err("expected binary digits after `0b`".to_string())
            };
        }
        // 二进位默认无符号(SignedPrefix.UNSIGNED:SnbtGrammar.java:804),
        // 负号即错。
        return finish_radix(&digits, j, body, minus, 2, default, "binary");
    }
    decimal_or_float(body, minus, default)
}

/// 最长数字串:accepted 字符+`_` 贪婪;空串→(空,原位,None);首尾 `_`
/// →错(NumberRunParseRule.java:36 实物——中间下划线任意,含连续)。
/// 返回的下划线已剥离。
fn scan_run(
    b: &[u8],
    start: usize,
    accept: fn(u8) -> bool,
) -> (String, usize, Option<&'static str>) {
    let mut i = start;
    while i < b.len() && (accept(b[i]) || b[i] == b'_') {
        i += 1;
    }
    if i == start {
        return (String::new(), start, None);
    }
    if b[start] == b'_' || b[i - 1] == b'_' {
        return (
            String::new(),
            i,
            Some("underscore not allowed at numeral edges"),
        );
    }
    let digits: String = b[start..i]
        .iter()
        .map(|&c| c as char)
        .filter(|&c| c != '_')
        .collect();
    (digits, i, None)
}

fn int_suffix_kind(c: u8) -> Result<NumKind, String> {
    match c {
        b'b' | b'B' => Ok(NumKind::Byte),
        b's' | b'S' => Ok(NumKind::Short),
        b'l' | b'L' => Ok(NumKind::Long),
        _ => Err(format!("bad integer suffix `{}`", c as char)),
    }
}

fn radix_int(
    body: &[u8],
    minus: bool,
    radix: u32,
    start: usize,
    default: NumKind,
    label: &str,
) -> Result<NbtValue, String> {
    let (digits, j, err) = scan_run(body, start, |c| c.is_ascii_hexdigit());
    if let Some(e) = err {
        return Err(e.to_string());
    }
    if digits.is_empty() {
        return Err(format!("expected {label} digits"));
    }
    finish_radix(&digits, j, body, minus, radix, default, label)
}

/// hex/二进位共用:后缀(可选、必须吃掉整个尾巴)、非负检查、无符号构建。
/// `0b` 的空位支已在调用侧回退,这里 digits 恒非空。
fn finish_radix(
    digits: &str,
    j: usize,
    body: &[u8],
    minus: bool,
    radix: u32,
    default: NumKind,
    label: &str,
) -> Result<NbtValue, String> {
    let kind = match body.get(j) {
        None => default,
        Some(&c) => {
            if j + 1 != body.len() {
                return Err("trailing garbage after radix integer".to_string());
            }
            int_suffix_kind(c)?
        }
    };
    // BINARY/HEX 默认 SignedPrefix.UNSIGNED(SnbtGrammar.java:798-807):
    // 带负号即 ERROR_EXPECTED_NON_NEGATIVE_NUMBER。
    if minus {
        return Err(format!("{label} literal must be non-negative"));
    }
    let wrapped = |v: u64| match kind {
        NumKind::Byte => NbtValue::Byte(v as u8 as i8),
        NumKind::Short => NbtValue::Short(v as u16 as i16),
        NumKind::Int => NbtValue::Int(v as u32 as i32),
        NumKind::Long => NbtValue::Long(v as i64),
    };
    let width = match kind {
        NumKind::Byte => 8,
        NumKind::Short => 16,
        NumKind::Int => 32,
        NumKind::Long => 64,
    };
    // 无符号域检查:Integer.parseUnsignedInt 等要求值 < 2^width。
    let v = u64::from_str_radix(digits, radix)
        .map_err(|_| format!("{label} value out of range for {kind:?}"))?;
    if width < 64 && v >= 1u64 << width {
        return Err(format!("{label} value out of range for {kind:?}"));
    }
    Ok(wrapped(v))
}

/// 十进制主体:整数字面量(sign? decnumeral suffix?)或浮点(floatLiteral
/// 四支:SnbtGrammar.java:403-425)。
fn decimal_or_float(body: &[u8], minus: bool, default: NumKind) -> Result<NbtValue, String> {
    let (whole, j, err) = scan_run(body, 0, |c| c.is_ascii_digit());
    if let Some(e) = err {
        return Err(e.to_string());
    }
    if whole.is_empty() {
        // 支 2:`.` 后小数位必需(floatLiteral SnbtGrammar.java:414-419,
        // decimalNumeral 非 optional)。
        if body[0] != b'.' {
            return Err("expected digits".to_string());
        }
        let (frac, j, err) = scan_run(body, 1, |c| c.is_ascii_digit());
        if let Some(e) = err {
            return Err(e.to_string());
        }
        if frac.is_empty() {
            return Err("expected digits after `.`".to_string());
        }
        return float_tail(body, minus, &format!("0.{frac}"), j);
    }
    match body.get(j) {
        None => build_signed_decimal(&whole, minus, default),
        Some(&c) if c == b'.' => {
            // 支 1:小数位可空(`5.` 合法,Double.parseDouble 同义)。
            let (frac, k, err) = scan_run(body, j + 1, |c| c.is_ascii_digit());
            if let Some(e) = err {
                return Err(e.to_string());
            }
            let frac = if frac.is_empty() {
                "0".to_string()
            } else {
                frac
            };
            float_tail(body, minus, &format!("{whole}.{frac}"), k)
        }
        // 支 3/4:整数位直接进浮点尾巴(指数或 f/d 后缀至少其一)。
        Some(&c) if (c | 0x20) == b'e' || matches!(c, b'f' | b'F' | b'd' | b'D') => {
            float_tail(body, minus, &format!("{whole}.0"), j)
        }
        Some(&c) if matches!(c, b'b' | b'B' | b's' | b'S' | b'l' | b'L') => {
            if j + 1 != body.len() {
                return Err("trailing garbage after integer suffix".to_string());
            }
            build_signed_decimal(&whole, minus, int_suffix_kind(c)?)
        }
        Some(_) => Err("malformed numeric literal".to_string()),
    }
}

/// 浮点尾巴:`(e sign? expdigits)? (f|d)?` 直到 token 末尾;mantissa 已
/// 规范化为 `digits.digits`(floatLiteral 支 3 的 exponent 必需、支 4 的
/// 后缀必需,由调用点保证进入本函数的 token 至少其一为真)。
fn float_tail(body: &[u8], minus: bool, mantissa: &str, mut i: usize) -> Result<NbtValue, String> {
    let mut exp_part = String::new();
    if i < body.len() && (body[i] | 0x20) == b'e' {
        let mut k = i + 1;
        let eminus = body.get(k) == Some(&b'-');
        if matches!(body.get(k), Some(&b'+') | Some(&b'-')) {
            k += 1;
        }
        let (exp, j, err) = scan_run(body, k, |c| c.is_ascii_digit());
        if let Some(e) = err {
            return Err(e.to_string());
        }
        if exp.is_empty() {
            return Err("expected exponent digits".to_string());
        }
        exp_part = format!("e{}{exp}", if eminus { "-" } else { "" });
        i = j;
    }
    let suffix = if i >= body.len() {
        None
    } else {
        let c = body[i];
        if !matches!(c, b'f' | b'F' | b'd' | b'D') || i + 1 != body.len() {
            return Err("trailing garbage after float literal".to_string());
        }
        Some(c)
    };
    build_float(
        &format!("{}{mantissa}{exp_part}", if minus { "-" } else { "" }),
        suffix,
    )
}

fn build_float(text: &str, suffix: Option<u8>) -> Result<NbtValue, String> {
    match suffix {
        Some(b'f') | Some(b'F') => text
            .parse::<f32>()
            .map(NbtValue::Float)
            .map_err(|_| format!("invalid float `{text}`")),
        _ => text
            .parse::<f64>()
            .map(NbtValue::Double)
            .map_err(|_| format!("invalid double `{text}`")),
    }
}

/// 十进制整数:原版 grammar 层禁前导零(cut:SnbtGrammar.java:358),
/// 有符号域检查(Byte.parseByte 等);无后缀超 i32 →「too big, use L」
/// (NBT-NEGINT-LAX 撤销裸串宽容)。
fn build_signed_decimal(digits: &str, minus: bool, kind: NumKind) -> Result<NbtValue, String> {
    if digits.len() > 1 && digits.starts_with('0') {
        return Err("leading zeros not allowed".to_string());
    }
    let sign = if minus { "-" } else { "" };
    let text = format!("{sign}{digits}");
    match kind {
        NumKind::Byte => text
            .parse::<i8>()
            .map(NbtValue::Byte)
            .map_err(|_| format!("value `{text}` out of byte range")),
        NumKind::Short => text
            .parse::<i16>()
            .map(NbtValue::Short)
            .map_err(|_| format!("value `{text}` out of short range")),
        NumKind::Int => text
            .parse::<i32>()
            .map(NbtValue::Int)
            .map_err(|_| format!("value `{text}` too big, use `L` suffix")),
        NumKind::Long => text
            .parse::<i64>()
            .map(NbtValue::Long)
            .map_err(|_| format!("value `{text}` out of long range")),
    }
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

    fn num(src: &str) -> NbtValue {
        parse_snbt(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"))
    }

    fn bad(src: &str) {
        let e = parse_snbt(src).unwrap_or_else(|| panic!("{src:?} must be rejected"));
        assert!(matches!(e, NbtError::BadSnbt { .. }), "{src:?} → {e}");
    }

    #[test]
    fn hex_binary_literals_vanilla_semantics() {
        // 无符号默认+宽度回绕(Integer.parseUnsignedInt:SnbtGrammar.java:856)。
        assert_eq!(num("0xFF"), NbtValue::Int(255));
        assert_eq!(num("0xFFFFFFFF"), NbtValue::Int(-1));
        assert_eq!(num("0xF000s"), NbtValue::Short(-4096)); // 61440 → i16 回绕
        assert_eq!(num("0xFFFFs"), NbtValue::Short(-1));
        assert_eq!(num("0x1L"), NbtValue::Long(1));
        // 贪心 HEX_NUMERAL:后缀 b 本身是十六进制位,被位流吸收(原版同)。
        assert_eq!(num("0xffb"), NbtValue::Int(0xffb));
        // 二进制:UNSIGNED 默认(SnbtGrammar.java:804);`0b` 空位回退
        // marker「0」+后缀 b。
        assert_eq!(num("0b101"), NbtValue::Int(5));
        assert_eq!(num("0b11b"), NbtValue::Byte(3));
        assert_eq!(num("0b"), NbtValue::Byte(0));
        assert_eq!(num("0b1_0"), NbtValue::Int(2));
        // 无符号面拒负号(ERROR_EXPECTED_NON_NEGATIVE_NUMBER)。
        bad("-0x1");
        bad("-0b1");
        bad("0b2"); // 非二进位
        bad("0xZZ");
        bad("0x100000000"); // 超 u32 域
        bad("0xF0000s"); // 超 u16 域
    }

    #[test]
    fn underscore_separators_edge_rules() {
        // NumberRunParseRule.java:36——串首尾 `_` 非法,中间任意(含连续)。
        assert_eq!(num("1_000"), NbtValue::Int(1000));
        assert_eq!(num("1__0"), NbtValue::Int(10)); // 剥离全部 `_` → "10"
        assert_eq!(num("-1_0"), NbtValue::Int(-10));
        assert_eq!(num("0x1_2"), NbtValue::Int(0x12));
        assert_eq!(num("1_0.5_5e1_0"), NbtValue::Double(105_500_000_000.0));
        assert_eq!(num(".5_5"), NbtValue::Double(0.55));
        // 首字符非数字启动集 → 裸串(原版 canStartNumber:SnbtGrammar.java:164)。
        assert_eq!(num("_1"), NbtValue::String("_1".into()));
        bad("1_");
        bad("0x_FF");
        bad("1e_5");
        bad("1._5");
    }

    #[test]
    fn leading_zeros_and_int_overflow_rejected() {
        // 前导零:grammar 层 cut(SnbtGrammar.java:358);浮点面不受限。
        bad("07");
        bad("00");
        bad("07s");
        assert_eq!(num("0"), NbtValue::Int(0));
        assert_eq!(num("-0"), NbtValue::Int(0));
        assert_eq!(num("0.5"), NbtValue::Double(0.5));
        assert_eq!(num("07.5"), NbtValue::Double(7.5));
        // NBT-NEGINT-LAX:无后缀超 i32 报错(原版 NumberFormatException,
        // 撤销裸串/Double 宽容)。
        bad("9999999999");
        bad("2147483648");
        bad("-2147483649");
        assert_eq!(num("2147483647"), NbtValue::Int(i32::MAX));
        assert_eq!(num("9223372036854775807L"), NbtValue::Long(i64::MAX));
        // 数字开头的非数字 token:原版 invalid_unquoted_start,同为错。
        bad("123abc");
        bad("-");
    }

    #[test]
    fn array_domain_matrix_and_empty_decls() {
        // ArrayPrefix.isAllowed(SnbtGrammar.java:763):B:{b};I:{b,s};L:{b,s,l}。
        assert_eq!(num("[B;]"), NbtValue::ByteArray(vec![]));
        assert_eq!(num("[I;]"), NbtValue::IntArray(vec![]));
        assert_eq!(num("[L;]"), NbtValue::LongArray(vec![]));
        bad("[;]"); // 原版无裸 `[;]`,arrayPrefix 只有 B/L/I
        assert_eq!(num("[I;0xff,2]"), NbtValue::IntArray(vec![255, 2]));
        assert_eq!(num("[L;1,2b,3s,4l]"), NbtValue::LongArray(vec![1, 2, 3, 4]));
        assert_eq!(num("[B;0xff]"), NbtValue::ByteArray(vec![-1]));
        assert_eq!(num("[B;127]"), NbtValue::ByteArray(vec![127]));
        // 撤销截断宽容:小数元素按原版拒绝(intArrayEntries=纯整数字面量)。
        bad("[I;1.5]");
        bad("[L;1.0]");
        bad("[B;255]"); // Byte.parseByte 域检查
        bad("[B;300]");
        bad("[B;1s]"); // 后缀域
        bad("[I;1l]"); // 后缀域
        // '[' 后/标记与分号间容忍空白(StringReader 终端 skipWhitespace)。
        assert_eq!(num("[ B; 1b ]"), NbtValue::ByteArray(vec![1]));
    }

    #[test]
    fn empty_typed_arrays_roundtrip_vanilla_style() {
        // 原版 printer 空数组写 `[B;]`(SnbtPrinterTagVisitor.java:89-102),
        // 写 `[]` 会读回空 list 破坏往返。
        roundtrip("{a:[B;],b:[I;],c:[L;]}");
        let mut t = String::new();
        write_snbt(&NbtValue::ByteArray(vec![]), &mut t);
        assert_eq!(t, "[B;]");
    }

    #[test]
    fn deep_nesting_reports_error_not_stack_overflow() {
        let src = "{".repeat(1000);
        let e = parse_snbt(&src).unwrap_err();
        assert!(matches!(e, NbtError::BadSnbt { .. }), "{e}");
    }
}
