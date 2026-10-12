//! 二进制读取。布局照原版 NbtIo/各 TagType.load 实物:
//! - 根:type 字节 + 名称(跳读) + payload;根必须 compound。
//! - List:元素类型字节 + i32 长度 + 逐元素 payload(无名字)。
//! - Compound:键值对循环,End(0) 终止。
//! - 字符串:u16 长度 + modified UTF-8。
//!
//! 压缩嗅探:0x1F 0x8B → GZIP、0x78 → ZLIB、否则 RAW。

use std::io::Read;

use flate2::read::{GzDecoder, ZlibDecoder};

use crate::accounter::{NbtAccounter, STRING_SELF_SIZE};
use crate::error::NbtError;
use crate::tag::{NbtCompound, NbtList, NbtValue, TagType};
use crate::utf8;

/// 读取入口公共 trait:`&[u8]` 与压缩流都走它。
trait PayloadRead {
    fn read_u8(&mut self) -> Result<u8, NbtError>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), NbtError>;
}

impl PayloadRead for &[u8] {
    fn read_u8(&mut self) -> Result<u8, NbtError> {
        if self.is_empty() {
            return Err(NbtError::UnexpectedEof);
        }
        let b = self[0];
        *self = &self[1..];
        Ok(b)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), NbtError> {
        if self.len() < buf.len() {
            return Err(NbtError::UnexpectedEof);
        }
        buf.copy_from_slice(&self[..buf.len()]);
        *self = &self[buf.len()..];
        Ok(())
    }
}

fn read_i16<R: PayloadRead + ?Sized>(r: &mut R) -> Result<i16, NbtError> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(i16::from_be_bytes(b))
}

fn read_i32<R: PayloadRead + ?Sized>(r: &mut R) -> Result<i32, NbtError> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_be_bytes(b))
}

fn read_i64<R: PayloadRead + ?Sized>(r: &mut R) -> Result<i64, NbtError> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(i64::from_be_bytes(b))
}

/// 原版 `DataInput.readUTF`/`skipBytes(readUnsignedShort)` 口径:字符串
/// 长度是无符号 u16。
fn read_u16<R: PayloadRead + ?Sized>(r: &mut R) -> Result<u16, NbtError> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}

/// 跳读名字字段(原版 readUnnamedTag 里的 skipString)。
fn skip_name<R: PayloadRead + ?Sized>(r: &mut R) -> Result<(), NbtError> {
    let len = read_u16(r)?;
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)
}

fn read_string<R: PayloadRead + ?Sized>(
    r: &mut R,
    acc: &mut NbtAccounter,
) -> Result<String, NbtError> {
    acc.account(STRING_SELF_SIZE, 1, "string tag count")?;
    let len = read_u16(r)?;
    acc.account(2, len as u64, "string chars")?;
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    utf8::decode(&buf)
}

/// 数组长度的统一防御:负数/超配额直接报错,绝不先分配。
fn read_array_len<R: PayloadRead + ?Sized>(
    r: &mut R,
    acc: &mut NbtAccounter,
    per: u64,
    what: &'static str,
) -> Result<usize, NbtError> {
    let n = read_i32(r)?;
    if n < 0 {
        return Err(NbtError::TooLarge {
            what,
            got: n as i64,
            max: i32::MAX as i64,
        });
    }
    acc.account(per, n as u64, what)?;
    Ok(n as usize)
}

fn read_payload<R: PayloadRead + ?Sized>(
    r: &mut R,
    ty: TagType,
    acc: &mut NbtAccounter,
) -> Result<NbtValue, NbtError> {
    Ok(match ty {
        TagType::End => NbtValue::End,
        TagType::Byte => NbtValue::Byte(r.read_u8()? as i8),
        TagType::Short => NbtValue::Short(read_i16(r)?),
        TagType::Int => NbtValue::Int(read_i32(r)?),
        TagType::Long => NbtValue::Long(read_i64(r)?),
        TagType::Float => {
            let mut b = [0u8; 4];
            r.read_exact(&mut b)?;
            NbtValue::Float(f32::from_be_bytes(b))
        }
        TagType::Double => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            NbtValue::Double(f64::from_be_bytes(b))
        }
        TagType::ByteArray => {
            let n = read_array_len(r, acc, 1, "byte array")?;
            let mut buf = vec![0u8; n];
            r.read_exact(&mut buf)?;
            NbtValue::ByteArray(buf.into_iter().map(|b| b as i8).collect())
        }
        TagType::IntArray => {
            let n = read_array_len(r, acc, 4, "int array")?;
            let mut v = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                v.push(read_i32(r)?);
            }
            NbtValue::IntArray(v)
        }
        TagType::LongArray => {
            let n = read_array_len(r, acc, 8, "long array")?;
            let mut v = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                v.push(read_i64(r)?);
            }
            NbtValue::LongArray(v)
        }
        TagType::String => NbtValue::String(read_string(r, acc)?),
        TagType::List => {
            acc.push_depth()?;
            let inner = read_list(r, acc)?;
            acc.pop_depth();
            NbtValue::List(inner)
        }
        TagType::Compound => {
            acc.push_depth()?;
            let c = read_compound_payload(r, acc)?;
            acc.pop_depth();
            NbtValue::Compound(c)
        }
    })
}

fn read_list<R: PayloadRead + ?Sized>(
    r: &mut R,
    acc: &mut NbtAccounter,
) -> Result<NbtList, NbtError> {
    let raw_ty = r.read_u8()?;
    let elem_ty = TagType::from_id(raw_ty).ok_or(NbtError::BadTagType(raw_ty))?;
    let n = if elem_ty == TagType::End {
        // 原版空 list:类型 0 + 长度(通常 0)。长度仍要读掉。
        let n = read_i32(r)?;
        if n > 0 {
            // 类型 End 却给正长度:legacy 文件不会这样,保守拒绝。
            return Err(NbtError::TooLarge {
                what: "end-typed list length",
                got: n as i64,
                max: 0,
            });
        }
        0usize
    } else {
        // 每元素至少 1 字节(payload 前的最小尺寸),记账防超长声明。
        read_array_len(r, acc, 2, "list length")?
    };
    // legacy 文件可含混型:payload 一律按声明类型读,类型字节原样保留。
    let mut items = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        items.push(read_payload(r, elem_ty, acc)?);
    }
    Ok(NbtList::from_raw(elem_ty, items))
}

fn read_compound_payload<R: PayloadRead + ?Sized>(
    r: &mut R,
    acc: &mut NbtAccounter,
) -> Result<NbtCompound, NbtError> {
    let mut c = NbtCompound::new();
    loop {
        let raw_ty = r.read_u8()?;
        if raw_ty == 0 {
            return Ok(c);
        }
        let ty = TagType::from_id(raw_ty).ok_or(NbtError::BadTagType(raw_ty))?;
        let name = read_string(r, acc)?;
        let value = read_payload(r, ty, acc)?;
        c.insert(name, value);
    }
}

/// 读一个「无名 tag」:类型字节 + (跳读)名字 + payload。类型 0 → End。
/// 原版 NbtIo.readUnnamedTag 语义。
pub fn read_unnamed_tag(bytes: &[u8], acc: &mut NbtAccounter) -> Result<NbtValue, NbtError> {
    let mut r = bytes;
    let raw_ty = r.read_u8()?;
    if raw_ty == 0 {
        return Ok(NbtValue::End);
    }
    let ty = TagType::from_id(raw_ty).ok_or(NbtError::BadTagType(raw_ty))?;
    skip_name(&mut r)?;
    read_payload(&mut r, ty, acc)
}

/// 读任意「有名/无名 tag 载荷」用于 list 等已剥名字的上下文之外的通用入口:
/// 类型字节 + payload(无名字)。原版 NbtIo.readAnyTag 语义。
pub fn read_any_tag(bytes: &[u8], acc: &mut NbtAccounter) -> Result<NbtValue, NbtError> {
    let mut r = bytes;
    let raw_ty = r.read_u8()?;
    if raw_ty == 0 {
        return Ok(NbtValue::End);
    }
    let ty = TagType::from_id(raw_ty).ok_or(NbtError::BadTagType(raw_ty))?;
    read_payload(&mut r, ty, acc)
}

/// 从裸(big-endian、未压缩)字节读根 compound。
pub fn read_raw(bytes: &[u8]) -> Result<NbtCompound, NbtError> {
    let mut acc = NbtAccounter::uncompressed_quota();
    read_raw_with(bytes, &mut acc)
}

/// [`read_raw`] 的自定义配额版。
pub fn read_raw_with(bytes: &[u8], acc: &mut NbtAccounter) -> Result<NbtCompound, NbtError> {
    let mut r = bytes;
    let raw_ty = r.read_u8()?;
    if raw_ty == 0 {
        return Err(NbtError::BadRootType(0));
    }
    let ty = TagType::from_id(raw_ty).ok_or(NbtError::BadTagType(raw_ty))?;
    if ty != TagType::Compound {
        return Err(NbtError::BadRootType(raw_ty));
    }
    skip_name(&mut r)?;
    acc.push_depth()?;
    let c = read_compound_payload(&mut r, acc)?;
    acc.pop_depth();
    Ok(c)
}

/// 自动识别 RAW/GZIP/ZLIB 并读根 compound(魔数:0x1F 0x8B / 0x78 / 其余)。
pub fn read_auto(bytes: &[u8]) -> Result<NbtCompound, NbtError> {
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        read_gzip(bytes)
    } else if !bytes.is_empty() && bytes[0] == 0x78 {
        read_zlib(bytes)
    } else {
        read_raw(bytes)
    }
}

/// GZIP 压缩读(原版 readCompressed)。
pub fn read_gzip(bytes: &[u8]) -> Result<NbtCompound, NbtError> {
    let dec = GzDecoder::new(bytes);
    read_from_stream(dec)
}

/// ZLIB 压缩读(原版 NbtStreamTools 的 zlib 变体)。
pub fn read_zlib(bytes: &[u8]) -> Result<NbtCompound, NbtError> {
    let dec = ZlibDecoder::new(bytes);
    read_from_stream(dec)
}

fn read_from_stream<R: Read>(src: R) -> Result<NbtCompound, NbtError> {
    // 解压体量不可信,先整体读入再走配额版;读入上限 = 配额,防解压炸弹
    // 直接吃满内存。
    let mut acc = NbtAccounter::uncompressed_quota();
    let mut buf: Vec<u8> = Vec::new();
    let mut reader = std::io::BufReader::new(src);
    let mut chunk = [0u8; 65536];
    let cap = UNCOMPRESSED_DECOMPRESS_CAP;
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if buf.len() + n > cap {
            return Err(NbtError::TooLarge {
                what: "decompressed stream",
                got: (buf.len() + n) as i64,
                max: cap as i64,
            });
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    read_raw_with(&buf, &mut acc)
}

/// 解压流允许的字节上限(与原版 100Mi 解压口径对齐)。
const UNCOMPRESSED_DECOMPRESS_CAP: usize = 104_857_600;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_payload_reports_eof_not_panic() {
        // TAG_Compound + 空根名;体内一个 String 键(名长 0),值声明 8 字节
        // 但 payload 缺失 → UnexpectedEof。
        let b: Vec<u8> = vec![10, 0, 0, 8, 0, 0, 0, 8];
        assert!(matches!(read_raw(&b), Err(NbtError::UnexpectedEof)));
    }

    #[test]
    fn bad_root_type_rejected() {
        // 根为 Byte tag(类型 1)。
        let b = [1u8, 0, 0, 7];
        assert!(matches!(read_raw(&b), Err(NbtError::BadRootType(1))));
    }

    #[test]
    fn depth_limit_triggers() {
        // 嵌套 600 层 compound,远超 512 上限。键名一律空串(读侧允许覆盖)。
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend([10u8, 0, 0]); // 根 compound + 空名
        for _ in 0..600 {
            // 键:类型 Compound、空名、空名嵌套 compound 的 payload 稍后统一收尾。
            bytes.extend([10u8, 0, 0]);
        }
        // 每层 compound 需要一个 End;倒序补齐 End 与外层无关,读侧按 End 逐层弹栈。
        for _ in 0..600 {
            bytes.push(0);
        }
        let r = read_raw(&bytes);
        assert!(
            matches!(r, Err(NbtError::TooDeep { max_depth: 512 })),
            "{r:?}"
        );
    }

    #[test]
    fn negative_list_length_rejected() {
        // compound { k: List 类型 Int、长度 -1 }
        let b: Vec<u8> = vec![10, 0, 0, 9, 0, 1, b'k', 3, 0xff, 0xff, 0xff, 0xff, 0];
        assert!(matches!(
            read_raw(&b),
            Err(NbtError::TooLarge {
                what: "list length",
                got: -1,
                ..
            })
        ));
    }
}
