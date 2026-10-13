//! 二进制读取。布局照原版 NbtIo/各 TagType.load 实物:
//! - 根:type 字节 + 名称(跳读) + payload;根必须 compound。
//! - List:元素类型字节 + i32 长度 + 逐元素 payload(无名字)。
//! - Compound:键值对循环,End(0) 终止。
//! - 字符串:u16 长度 + modified UTF-8。
//!
//! 压缩嗅探:0x1F 0x8B → GZIP、0x78 → ZLIB、否则 RAW。
//!
//! 实现口径:容器读为**显式栈状态机(零递归)**——原版 Java 是递归下降,
//! 靠 JVM 栈扛 512 层;Rust debug 帧大,同深度在 2 MiB 线程栈上直接 SIGABRT
//! (CI 实测)。栈外语义(深度上限、记账、错误类型)与原版一致。

use std::io::Read;

use flate2::read::{GzDecoder, ZlibDecoder};

use crate::accounter::NbtAccounter;
use crate::accounter::{
    ARRAY_SELF, COMPOUND_ENTRY, COMPOUND_SELF, KEY_SELF, LIST_SELF, LIST_SLOT, SELF_BYTE,
    SELF_DOUBLE, SELF_FLOAT, SELF_INT, SELF_LONG, SELF_SHORT, STRING_CHAR_SIZE, STRING_SELF_SIZE,
    utf16_len,
};
use crate::error::NbtError;
use crate::tag::{NbtCompound, NbtList, NbtValue, TagType};
use crate::utf8;

/// 读取入口 trait(切片源;解压流先整体读入再走同一状态机)。
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

/// 读一个字符串**值**(StringTag.java:11-23 的 `readAccounted` 口径:
/// 先记 self 36,读 UTF,再按 UTF-16 char 数记 2×len)。
fn read_string<R: PayloadRead + ?Sized>(
    r: &mut R,
    acc: &mut NbtAccounter,
) -> Result<String, NbtError> {
    acc.account(STRING_SELF_SIZE, 1, "string tag count")?;
    let s = read_utf_raw(r)?;
    acc.account(STRING_CHAR_SIZE, utf16_len(&s), "string chars")?;
    Ok(s)
}

/// 读一个 compound **键名**(CompoundTag.java:130-133 口径:readUTF 后
/// 记 28,再按 UTF-16 char 数记 2×len)。
fn read_key<R: PayloadRead + ?Sized>(
    r: &mut R,
    acc: &mut NbtAccounter,
) -> Result<String, NbtError> {
    let s = read_utf_raw(r)?;
    acc.account(KEY_SELF, 1, "compound key self")?;
    acc.account(STRING_CHAR_SIZE, utf16_len(&s), "key chars")?;
    Ok(s)
}

/// u16 长度 + modified UTF-8 原文(不含记账)。
fn read_utf_raw<R: PayloadRead + ?Sized>(r: &mut R) -> Result<String, NbtError> {
    let len = read_u16(r)?;
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    utf8::decode(&buf)
}

/// 定长数组头:先记 self 24(ByteArrayTag.java:23),再读长度(负数/超配额
/// 直接报错,绝不先分配),再按元素宽度全计(:25)。
fn read_array_len<R: PayloadRead + ?Sized>(
    r: &mut R,
    acc: &mut NbtAccounter,
    per: u64,
    what: &'static str,
) -> Result<usize, NbtError> {
    acc.account(ARRAY_SELF, 1, what)?;
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

/// 标量/定长数组 payload(平坦,零递归)。
fn read_scalar<R: PayloadRead + ?Sized>(
    r: &mut R,
    ty: TagType,
    acc: &mut NbtAccounter,
) -> Result<NbtValue, NbtError> {
    // 标量 self-size 先于 payload(各 *Tag.java load 口径,常数见 accounter)。
    match ty {
        TagType::Byte => {
            acc.account(SELF_BYTE, 1, "byte tag")?;
            Ok(NbtValue::Byte(r.read_u8()? as i8))
        }
        TagType::Short => {
            acc.account(SELF_SHORT, 1, "short tag")?;
            Ok(NbtValue::Short(read_i16(r)?))
        }
        TagType::Int => {
            acc.account(SELF_INT, 1, "int tag")?;
            Ok(NbtValue::Int(read_i32(r)?))
        }
        TagType::Long => {
            acc.account(SELF_LONG, 1, "long tag")?;
            Ok(NbtValue::Long(read_i64(r)?))
        }
        TagType::Float => {
            acc.account(SELF_FLOAT, 1, "float tag")?;
            let mut b = [0u8; 4];
            r.read_exact(&mut b)?;
            Ok(NbtValue::Float(f32::from_be_bytes(b)))
        }
        TagType::Double => {
            acc.account(SELF_DOUBLE, 1, "double tag")?;
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            Ok(NbtValue::Double(f64::from_be_bytes(b)))
        }
        TagType::ByteArray => {
            let n = read_array_len(r, acc, 1, "byte array")?;
            let mut buf = vec![0u8; n];
            r.read_exact(&mut buf)?;
            Ok(NbtValue::ByteArray(
                buf.into_iter().map(|b| b as i8).collect(),
            ))
        }
        TagType::IntArray => {
            let n = read_array_len(r, acc, 4, "int array")?;
            let mut v = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                v.push(read_i32(r)?);
            }
            Ok(NbtValue::IntArray(v))
        }
        TagType::LongArray => {
            let n = read_array_len(r, acc, 8, "long array")?;
            let mut v = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                v.push(read_i64(r)?);
            }
            Ok(NbtValue::LongArray(v))
        }
        TagType::String => Ok(NbtValue::String(read_string(r, acc)?)),
        other => unreachable!("read_scalar 只收标量/数组类型,收到 {other:?}"),
    }
}

/// 显式栈容器帧。
enum Frame {
    /// compound 构造中;key = 刚读完键名、等值的挂起键。
    Compound {
        map: NbtCompound,
        key: Option<String>,
    },
    /// list 构造中;left = 还差几个元素(payload 全按 elem 类型读,
    /// legacy 混型文件 payload 类型不符时读出异型值原样保留——原版同语义)。
    List {
        elem: TagType,
        items: Vec<NbtValue>,
        left: u64,
    },
}

/// 零递归地读一个 `start_ty` 类型的 payload,返回完成值。
/// 深度/记账全走 accounter;栈占用 O(容器嵌套层数) 但全在堆上。
fn read_value_iter<R: PayloadRead + ?Sized>(
    r: &mut R,
    acc: &mut NbtAccounter,
    start_ty: TagType,
) -> Result<NbtValue, NbtError> {
    let mut stack: Vec<Frame> = Vec::new();
    // 双态驱动:`ty` = 待读 payload 类型;`value` = 已读完待挂到上层框架的值。
    let mut ty = Some(start_ty);
    let mut value: Option<NbtValue> = None;
    loop {
        // ① 有待挂值:挂到顶帧;栈空则整体完成。
        if let Some(v) = value.take() {
            match stack.last_mut() {
                None => return Ok(v),
                Some(Frame::Compound { map, key }) => {
                    // 原版 loadCompound(CompoundTag.java:60-62):put 返回
                    // null(新键)才补记 36;覆盖同名键不再计。
                    if map.insert(key.take().unwrap_or_default(), v).is_none() {
                        acc.account(COMPOUND_ENTRY, 1, "compound entry")?;
                    }
                }
                Some(Frame::List { items, .. }) => {
                    // left 只在 ③Elem 发出读取请求时扣减;此处仅收集。
                    items.push(v);
                }
            }
        }
        // ② 有待读类型:标量即完成;容器读头并压栈。
        if let Some(t) = ty.take() {
            match t {
                TagType::End => return Err(NbtError::BadTagType(0)),
                TagType::List => {
                    acc.push_depth()?;
                    // ListTag.loadList(ListTag.java:30-38):self 36 → 元素
                    // 类型字节 → 长度(负数拒)→ 类型 0 且长度>0 拒 → 4×count。
                    acc.account(LIST_SELF, 1, "list self")?;
                    let raw_ty = r.read_u8()?;
                    let elem = TagType::from_id(raw_ty).ok_or(NbtError::BadTagType(raw_ty))?;
                    let n = read_i32(r)?;
                    if n < 0 {
                        return Err(NbtError::TooLarge {
                            what: "list length",
                            got: n as i64,
                            max: i32::MAX as i64,
                        });
                    }
                    if elem == TagType::End && n > 0 {
                        // 原版 NbtFormatException("Missing type on ListTag")
                        // (ListTag.java:34-36)。
                        return Err(NbtError::TooLarge {
                            what: "end-typed list length",
                            got: n as i64,
                            max: 0,
                        });
                    }
                    acc.account(LIST_SLOT, n as u64, "list slots")?;
                    let n = n as usize;
                    stack.push(Frame::List {
                        elem,
                        items: Vec::with_capacity(n.min(1 << 16)),
                        left: n as u64,
                    });
                }
                TagType::Compound => {
                    acc.push_depth()?;
                    // CompoundTag.loadCompound(CompoundTag.java:53):self 48。
                    acc.account(COMPOUND_SELF, 1, "compound self")?;
                    stack.push(Frame::Compound {
                        map: NbtCompound::new(),
                        key: None,
                    });
                }
                s => {
                    value = Some(read_scalar(r, s, acc)?);
                }
            }
            continue;
        }
        // ③ 推进顶帧:读下一个条目,或完成(弹栈出值)。
        enum Next {
            CompoundEntry,
            Complete,
            Elem(TagType),
        }
        let next = match stack.last() {
            None => return Err(NbtError::UnexpectedEof),
            Some(Frame::Compound { .. }) => Next::CompoundEntry,
            Some(Frame::List { elem, left, .. }) => {
                if *left > 0 {
                    Next::Elem(*elem)
                } else {
                    Next::Complete
                }
            }
        };
        match next {
            Next::CompoundEntry => {
                let tb = r.read_u8()?;
                if tb == 0 {
                    let Some(Frame::Compound { map, .. }) = stack.pop() else {
                        unreachable!("Next::CompoundEntry 与栈顶型别同步");
                    };
                    acc.pop_depth();
                    value = Some(NbtValue::Compound(map));
                } else {
                    let t = TagType::from_id(tb).ok_or(NbtError::BadTagType(tb))?;
                    let name = read_key(r, acc)?;
                    if let Some(Frame::Compound { key, .. }) = stack.last_mut() {
                        *key = Some(name);
                    }
                    ty = Some(t);
                }
            }
            Next::Elem(t) => {
                if let Some(Frame::List { left, .. }) = stack.last_mut() {
                    *left -= 1;
                }
                ty = Some(t);
            }
            Next::Complete => match stack.pop() {
                Some(Frame::Compound { map, .. }) => {
                    acc.pop_depth();
                    value = Some(NbtValue::Compound(map));
                }
                Some(Frame::List { elem, items, .. }) => {
                    acc.pop_depth();
                    value = Some(NbtValue::List(NbtList::from_raw(elem, items)));
                }
                None => unreachable!("Next::Complete 时栈非空"),
            },
        }
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
    read_value_iter(&mut r, acc, ty)
}

/// 读「类型字节 + payload(无名字)」的通用入口。原版 NbtIo.readAnyTag 语义。
pub fn read_any_tag(bytes: &[u8], acc: &mut NbtAccounter) -> Result<NbtValue, NbtError> {
    let mut r = bytes;
    let raw_ty = r.read_u8()?;
    if raw_ty == 0 {
        return Ok(NbtValue::End);
    }
    let ty = TagType::from_id(raw_ty).ok_or(NbtError::BadTagType(raw_ty))?;
    read_value_iter(&mut r, acc, ty)
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
    match read_value_iter(&mut r, acc, TagType::Compound)? {
        NbtValue::Compound(c) => Ok(c),
        // read_value_iter 以 Compound 起步不可能返回他型。
        other => Err(NbtError::BadRootType(other.tag_type().id())),
    }
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
        // 嵌套 600 层 compound,远超 512 上限。显式栈读:零递归,超限必须
        // 返回 TooDeep 而不是栈溢出(CI 曾实测递归版 SIGABRT)。键名一律
        // 空串(读侧允许覆盖)。
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend([10u8, 0, 0]); // 根 compound + 空名
        for _ in 0..600 {
            bytes.extend([10u8, 0, 0]);
        }
        // 每层 compound 需要一个 End;读侧按 End 逐层弹栈。
        bytes.extend(std::iter::repeat_n(0u8, 600));
        let r = read_raw(&bytes);
        assert!(
            matches!(r, Err(NbtError::TooDeep { max_depth: 512 })),
            "{r:?}"
        );
    }

    #[test]
    fn deep_but_legal_nesting_parses_in_constant_stack() {
        // 511 层合法深度 + 一个 Int 叶子 + 逐层 End:必须完整读回。
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend([10u8, 0, 0]); // 根
        for _ in 0..510 {
            bytes.extend([10u8, 0, 0]);
        }
        bytes.extend([3u8, 0, 1, b'v', 0, 0, 0, 7]); // 键 v = Int(7)
        bytes.extend(std::iter::repeat_n(0u8, 511));
        let c = read_raw(&bytes).expect("511 层合法深度必须解析成功");
        // 逐层钻到叶子。
        let mut node = &c;
        let mut depth = 0;
        while let Some(inner) = node.get_compound("") {
            node = inner;
            depth += 1;
        }
        assert_eq!(node.get_i32("v"), Some(7));
        assert_eq!(depth, 510);
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

    #[test]
    fn accounting_exact_boundary_locks_vanilla_arithmetic() {
        // compound { "": Int(7) } 的原版口径总账:
        //   48(compound self)+28(空键 self)+0(键 char×2)+12(Int self)
        //   +36(新键 entry) = 124(根名 skipString 不记账,NbtIo.java:176)。
        //   恰等于配额放行(NbtAccounter.java:45 的判定是 usage+size > quota
        //   才炸),超 1 字节必拒。
        let b: Vec<u8> = vec![10, 0, 0, 3, 0, 0, 0, 0, 0, 7, 0];
        let mut ok = NbtAccounter::new(124, 8);
        assert!(read_raw_with(&b, &mut ok).is_ok(), "恰等于配额必须放行");
        assert_eq!(ok.usage(), 124);
        let mut tight = NbtAccounter::new(123, 8);
        assert!(
            matches!(
                read_raw_with(&b, &mut tight),
                Err(NbtError::TooLarge { .. })
            ),
            "恰超配额 1 字节必须拒"
        );
    }

    #[test]
    fn accounting_duplicate_key_skips_entry_charge() {
        // 同名键两次:第二次的 put 返回旧值,原版不再记 36
        // (CompoundTag.java:61 `values.put(..) == null`)。
        // 键名「k」每次 readString 记 28+2×1=30(CompoundTag.java:131-132)。
        // 总账 = 48 + (30+12+36) + (30+12) = 168。
        let b: Vec<u8> = vec![
            10, 0, 0, // 根 compound
            3, 0, 1, b'k', 0, 0, 0, 1, // k = Int(1)
            3, 0, 1, b'k', 0, 0, 0, 2, // k 覆盖 = Int(2)
            0,
        ];
        let mut acc = NbtAccounter::new(168, 8);
        assert!(read_raw_with(&b, &mut acc).is_ok());
        assert_eq!(acc.usage(), 168);
        let mut tight = NbtAccounter::new(167, 8);
        assert!(read_raw_with(&b, &mut tight).is_err());
    }

    #[test]
    fn accounting_string_uses_utf16_units_not_utf8_bytes() {
        // 原版 StringTag.java:23 记 2×UTF-16 char 数:「😀」= 2 单元 → 4;
        // 若按 UTF-8 字节(CESU-8 为 6)或字节数记账会偏离。
        // 总账:48 + 28(空键) + 36(string self) + 4(2 char×2) + 36(entry) = 152。
        let mut body: Vec<u8> = Vec::new();
        body.extend([10u8, 0, 0, 8, 0, 0]); // 根 + 键 "" + String
        body.extend(6u16.to_be_bytes()); // CESU-8 6 字节
        body.extend([0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]); // 😀
        body.push(0);
        let mut acc = NbtAccounter::new(152, 8);
        let c = read_raw_with(&body, &mut acc).expect("恰配额放行");
        assert_eq!(c.get_string("").unwrap(), "😀");
        assert_eq!(acc.usage(), 152);
        assert!(read_raw_with(&body, &mut NbtAccounter::new(151, 8)).is_err());
    }

    #[test]
    fn list_of_lists_roundtrip_shape() {
        // 显式栈的状态机对嵌套 list 的回归锁:[[1,2],[3]] 形态。
        let b: Vec<u8> = vec![
            10, 0, 0, // 根 compound
            9, 0, 1, b'l', // 键 l: List
            9,    // 元素类型 List
            0, 0, 0, 2, // 2 个内层 list
            // 内层 list 元素自带自己的元素类型字节(ListTag.write 语义),
            // 这里都是 Int → 3:
            3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, // [1,2]
            3, 0, 0, 0, 1, 0, 0, 0, 3, // [3]
            0, // 根 End
        ];
        let c = read_raw(&b).expect("嵌套 list 解析");
        let outer = c.get_list("l").unwrap();
        assert_eq!(outer.len(), 2);
        assert_eq!(outer.get(0).unwrap().as_list().unwrap().len(), 2);
        assert_eq!(outer.get(1).unwrap().as_list().unwrap().len(), 1);
    }
}
