//! mcv_nbt 往返纪律测试:手工构造 13 种 tag → 写 → 读回逐字段相等;
//! 二次编码字节稳定;三压缩模式嗅探。

use mcv_nbt::{
    NbtAccounter, NbtCompound, NbtError, NbtList, NbtValue, parse_snbt, read_auto, read_gzip,
    read_raw, read_zlib, write_gzip, write_raw, write_snbt, write_zlib,
};

/// 覆盖 13 种 tag 的大 compound(键序 = 插入序)。
fn all_types() -> NbtCompound {
    let mut c = NbtCompound::new();
    c.insert("b", NbtValue::Byte(-1));
    c.insert("s", NbtValue::Short(-300));
    c.insert("i", NbtValue::Int(70_000));
    c.insert("l", NbtValue::Long(-9_000_000_000));
    c.insert("f", NbtValue::Float(1.5));
    c.insert("d", NbtValue::Double(-2.25));
    c.insert("ba", NbtValue::ByteArray(vec![1, -2, 3]));
    c.insert("ia", NbtValue::IntArray(vec![i32::MIN, 0, i32::MAX]));
    c.insert("la", NbtValue::LongArray(vec![i64::MIN, 7, i64::MAX]));
    c.insert("str", NbtValue::String("中文 \u{0} nul! \u{1f600}".into()));
    let mut inner = NbtCompound::new();
    inner.insert("nested", NbtValue::Byte(7));
    c.insert("cmp", NbtValue::Compound(inner));
    let mut list = NbtList::new();
    list.push(NbtValue::Compound({
        let mut e = NbtCompound::new();
        e.insert("x", NbtValue::Int(1));
        e
    }))
    .unwrap();
    list.push(NbtValue::Compound({
        let mut e = NbtCompound::new();
        e.insert("y", NbtValue::Int(2));
        e
    }))
    .unwrap();
    c.insert("list", NbtValue::List(list));
    // 空 list(类型字节 0)。
    c.insert("empty_list", NbtValue::List(NbtList::new()));
    c
}

#[test]
fn raw_roundtrip_fieldwise_and_byte_stable() {
    let c = all_types();
    let bytes = write_raw(&c).unwrap();
    let back = read_raw(&bytes).unwrap();
    assert_eq!(c, back, "读回逐字段(含键序)相等");
    // 字节稳定:二次编码字节相同。
    let bytes2 = write_raw(&back).unwrap();
    assert_eq!(bytes, bytes2, "二次编码字节稳定");
}

#[test]
fn compression_modes_sniff_and_roundtrip() {
    let c = all_types();
    let gz = write_gzip(&c).unwrap();
    let zl = write_zlib(&c).unwrap();
    let raw = write_raw(&c).unwrap();
    // 显式模式读。
    assert_eq!(read_gzip(&gz).unwrap(), c);
    assert_eq!(read_zlib(&zl).unwrap(), c);
    assert_eq!(read_raw(&raw).unwrap(), c);
    // 嗅探模式读全部命中。
    assert_eq!(read_auto(&gz).unwrap(), c);
    assert_eq!(read_auto(&zl).unwrap(), c);
    assert_eq!(read_auto(&raw).unwrap(), c);
    // 魔数前提。
    assert_eq!(&gz[..2], &[0x1f, 0x8b]);
    assert_eq!(zl[0], 0x78);
    assert_eq!(raw[0], 0x0a);
}

#[test]
fn empty_list_writes_type_byte_zero() {
    let mut c = NbtCompound::new();
    c.insert("e", NbtValue::List(NbtList::new()));
    let b = write_raw(&c).unwrap();
    // …9(类型) 0(元素类型 End) 00000000(长度)…
    let pos = b.windows(2).position(|w| w == [9, 0]).unwrap();
    assert_eq!(&b[pos + 2..pos + 6], &[0, 0, 0, 0]);
    assert_eq!(read_raw(&b).unwrap(), c);
}

#[test]
fn quota_too_large_triggers_without_panic() {
    // 一个小配额 accounter:塞进一个声明超大 IntArray 的输入必须报错,
    // 且不能在报错前分配超大缓冲(元素数记账在分配前)。
    let mut acc = NbtAccounter::new(64, 16);
    // compound { k: IntArray 长度 1<<30 }
    let mut b: Vec<u8> = vec![10, 0, 0]; // 根 compound + 空名
    b.extend([11, 0, 1, b'k']); // IntArray 键 "k"
    b.extend((1i32 << 30).to_be_bytes());
    b.push(0);
    let r = mcv_nbt::read_raw_with(&b, &mut acc);
    assert!(matches!(r, Err(NbtError::TooLarge { .. })), "{r:?}");
}

#[test]
fn string_length_quota_triggers() {
    let mut acc = NbtAccounter::new(48, 8);
    let mut b: Vec<u8> = vec![10, 0, 0]; // 根
    b.extend([8, 0, 1, b'k']); // String 键 "k"
    b.extend(65535u16.to_be_bytes()); // 值声明 65535 字节
    let r = mcv_nbt::read_raw_with(&b, &mut acc);
    // 记账(2×65535)先爆 TooLarge(48 配额)。
    assert!(matches!(r, Err(NbtError::TooLarge { .. })), "{r:?}");
}

#[test]
fn snbt_roundtrip_over_full_tree() {
    let c = all_types();
    let mut text = String::new();
    write_snbt(&NbtValue::Compound(c.clone()), &mut text);
    let back = parse_snbt(&text).unwrap();
    // SNBT 文本往返:标量后缀类型保真,树相等。
    assert_eq!(back, NbtValue::Compound(c));
}

#[test]
fn truncated_at_every_prefix_never_panics() {
    let bytes = write_raw(&all_types()).unwrap();
    for cut in 0..bytes.len() {
        let r = std::panic::catch_unwind(|| read_raw(&bytes[..cut]));
        match r {
            Ok(Err(_)) | Ok(Ok(_)) => {}
            Err(_) => panic!("read_raw panicked at cut={cut}"),
        }
        // 只有 0 前缀之外全部必须报错(树不可能在中间前缀完成)。
        if cut > 0 {
            assert!(
                read_raw(&bytes[..cut]).is_err(),
                "cut={cut} unexpectedly ok"
            );
        }
    }
}
