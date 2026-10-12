//! NBT 解析/编码错误。手刻 enum,不引 thiserror(全仓纪律)。

use std::fmt;

/// 一切读/写/SNBT 失败的结构化原因。
#[derive(Debug)]
pub enum NbtError {
    /// 底层 IO 失败(文件、压缩流)。
    Io(std::io::Error),
    /// 输入提前结束(截断的 tag 树)。
    UnexpectedEof,
    /// 未知 tag 类型字节(不在 0..=12)。
    BadTagType(u8),
    /// modified UTF-8 解码失败(含非法序列与原版从不产生的孤立代理对)。
    BadUtf8,
    /// compound/list 嵌套超过配额器深度上限。
    TooDeep {
        /// 允许的最大深度。
        max_depth: u32,
    },
    /// 体量超过配额(字符串长度、数组长度、负长度均归此类)。
    TooLarge {
        /// 人类可读的原因片段。
        what: &'static str,
        /// 实际值(字节/字符/元素数)。
        got: i64,
        /// 配额上限;-1 表示无上限(仅 got 本身非法,如负数)。
        max: i64,
    },
    /// 根 tag 不是 compound(NBT 文件约定根为命名 compound)。
    BadRootType(u8),
    /// SNBT 语法错误,带出错字符偏移(0 起)。
    BadSnbt {
        /// 出错处字符偏移。
        offset: usize,
        /// 说明。
        msg: String,
    },
}

impl fmt::Display for NbtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NbtError::Io(e) => write!(f, "nbt io error: {e}"),
            NbtError::UnexpectedEof => write!(f, "unexpected end of nbt input"),
            NbtError::BadTagType(t) => write!(f, "unknown nbt tag type byte: {t}"),
            NbtError::BadUtf8 => write!(f, "invalid modified-utf8 string"),
            NbtError::TooDeep { max_depth } => write!(f, "nbt nesting deeper than {max_depth}"),
            NbtError::TooLarge { what, got, max } => {
                if *max < 0 {
                    write!(f, "nbt {what} out of range: {got}")
                } else {
                    write!(f, "nbt {what} too large: {got} > quota {max}")
                }
            }
            NbtError::BadRootType(t) => {
                write!(f, "root nbt tag must be a compound, got type {t}")
            }
            NbtError::BadSnbt { offset, msg } => write!(f, "snbt error at offset {offset}: {msg}"),
        }
    }
}

impl std::error::Error for NbtError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NbtError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for NbtError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            NbtError::UnexpectedEof
        } else {
            NbtError::Io(e)
        }
    }
}
