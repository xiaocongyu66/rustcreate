//! 二进制写出。布局照原版 NbtIo.write/各 TagType.write:
//! 根 = 类型字节 + 空名(u16 0) + compound payload;compound 以 End 终止;
//! list 声明元素类型字节 + i32 长度后逐元素写 payload。
//!
//! 与原版差异:原版 StringFallbackDataOutput 在字符串超 65535 mutf8 字节时
//! 静默写空串,我们直接报 [`NbtError::TooLarge`](更响,便于定位)。

use flate2::Compression;
use flate2::write::{GzEncoder, ZlibEncoder};
use std::io::Write;

use crate::error::NbtError;
use crate::tag::{NbtCompound, NbtList, NbtValue, TagType};
use crate::utf8;

/// 写出裸(未压缩)big-endian 根 compound。
pub fn write_raw(root: &NbtCompound) -> Result<Vec<u8>, NbtError> {
    let mut out = Vec::new();
    out.push(TagType::Compound.id());
    out.extend([0u8, 0u8]); // 空根名
    write_compound(root, &mut out)?;
    Ok(out)
}

/// GZIP 压缩写出(原版 writeCompressed 语义,默认压缩级)。
pub fn write_gzip(root: &NbtCompound) -> Result<Vec<u8>, NbtError> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&write_raw(root)?)?;
    Ok(enc.finish()?)
}

/// ZLIB 压缩写出。
pub fn write_zlib(root: &NbtCompound) -> Result<Vec<u8>, NbtError> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&write_raw(root)?)?;
    Ok(enc.finish()?)
}

pub(crate) fn write_string(s: &str, out: &mut Vec<u8>) -> Result<(), NbtError> {
    let len = utf8::encoded_len(s);
    if len > u16::MAX as usize {
        return Err(NbtError::TooLarge {
            what: "string utf8 bytes",
            got: len as i64,
            max: u16::MAX as i64,
        });
    }
    out.extend((len as u16).to_be_bytes());
    utf8::encode_into(s, out);
    Ok(())
}

fn write_payload(v: &NbtValue, out: &mut Vec<u8>) -> Result<(), NbtError> {
    match v {
        NbtValue::End => {}
        NbtValue::Byte(b) => out.push(*b as u8),
        NbtValue::Short(s) => out.extend(s.to_be_bytes()),
        NbtValue::Int(i) => out.extend(i.to_be_bytes()),
        NbtValue::Long(l) => out.extend(l.to_be_bytes()),
        NbtValue::Float(f) => out.extend(f.to_be_bytes()),
        NbtValue::Double(d) => out.extend(d.to_be_bytes()),
        NbtValue::ByteArray(a) => {
            out.extend(array_len(a.len())?.to_be_bytes());
            out.extend(a.iter().map(|b| *b as u8));
        }
        NbtValue::IntArray(a) => {
            out.extend(array_len(a.len())?.to_be_bytes());
            for i in a {
                out.extend(i.to_be_bytes());
            }
        }
        NbtValue::LongArray(a) => {
            out.extend(array_len(a.len())?.to_be_bytes());
            for l in a {
                out.extend(l.to_be_bytes());
            }
        }
        NbtValue::String(s) => write_string(s, out)?,
        NbtValue::List(l) => write_list(l, out)?,
        NbtValue::Compound(c) => write_compound(c, out)?,
    }
    Ok(())
}

fn array_len(n: usize) -> Result<i32, NbtError> {
    i32::try_from(n).map_err(|_| NbtError::TooLarge {
        what: "array length",
        got: n as i64,
        max: i32::MAX as i64,
    })
}

fn write_list(l: &NbtList, out: &mut Vec<u8>) -> Result<(), NbtError> {
    out.push(l.element_type().id());
    out.extend(array_len(l.len())?.to_be_bytes());
    for item in l.items() {
        // 原版 ListTag.write 同语义:声明类型字节一份,payload 逐元素自写。
        write_payload(item, out)?;
    }
    Ok(())
}

fn write_compound(c: &NbtCompound, out: &mut Vec<u8>) -> Result<(), NbtError> {
    for (k, v) in c.iter() {
        out.push(v.tag_type().id());
        write_string(k, out)?;
        write_payload(v, out)?;
    }
    out.push(0); // TAG_End
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_layout_matches_spec() {
        // compound { "a": Int(1) }
        let mut c = NbtCompound::new();
        c.insert("a", NbtValue::Int(1));
        let b = write_raw(&c).unwrap();
        assert_eq!(b, vec![10, 0, 0, 3, 0, 1, b'a', 0, 0, 0, 1, 0]);
    }

    #[test]
    fn nul_char_uses_modified_utf8() {
        let mut out = Vec::new();
        write_string("a\0b", &mut out).unwrap();
        assert_eq!(out, vec![0, 4, b'a', 0xc0, 0x80, b'b']);
    }
}
