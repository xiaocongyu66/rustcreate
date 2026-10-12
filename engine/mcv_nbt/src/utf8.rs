//! Java `DataOutput.writeUTF` / `DataInput.readUTF` 的 modified UTF-8 编解码。
//!
//! 与 Rust String(标准 UTF-8)的差异:
//! - U+0000:modified UTF-8 编成 `0xC0 0x80`(标准 UTF-8 是单字节 0x00,且
//!   解码器必须拒绝该形式)。编码走 2 字节形式,解码两式都收(宽容)。
//! - 孤立 surrogate:Java 字符串可含,`writeUTF` 会编成 3 字节 `0xED 0xA0..`
//!   形式;Rust String 无法承载 → 解码报 [`NbtError::BadUtf8`]。原版合法
//!   数据(注册表名、SNBT、结构模板)从不出现孤立 surrogate,偏差登记为债。

use crate::error::NbtError;

/// 把 `s` 按 modified UTF-8 编码追加到 `out`,返回字节数(≤65535,由调用方
/// 在写出长度前缀前校验)。
pub(crate) fn encode_into(s: &str, out: &mut Vec<u8>) {
    for c in s.chars() {
        match c {
            '\u{0001}'..='\u{007f}' => out.push(c as u8),
            // U+0000:mutf8 独有的双字节形式(Java writeUTF 实物口径)。
            '\u{0}' => out.extend_from_slice(&[0xc0, 0x80]),
            // 2/3 字节与 UTF-8 同形;补充平面走标准 4 字节(见模块头
            // NBT-UTF8-CESU 债:原版为 CESU-8 surrogate 对)。
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
}

/// modified UTF-8 字节数(U+0000 计 2;续字节并入首字节计数)。
pub(crate) fn encoded_len(s: &str) -> usize {
    s.bytes().fold(0usize, |n, b| {
        n + match b {
            0x80..=0xbf => 0,
            0 => 2,
            0x01..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        }
    })
}

/// 解码 `data` 的 modified UTF-8。两式(U+0000 单字节/双字节)都接受;
/// 非法字节序列与孤立 surrogate → [`NbtError::BadUtf8`]。
pub(crate) fn decode(data: &[u8]) -> Result<String, NbtError> {
    // 快速路径:纯 ASCII 且不含 0xC0 0x80 双字节 NUL 之外形式的校验,
    // 统一走手写解码,口径只有一处。
    let mut out = String::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        match b0 {
            0x00 => {
                // 宽容:标准 UTF-8 形式的裸 NUL 也接受(原版从不产出)。
                out.push('\0');
                i += 1;
            }
            0x01..=0x7f => {
                out.push(b0 as char);
                i += 1;
            }
            // modified UTF-8 的 U+0000 双字节形式(C0 80)。
            0xc0 => {
                if data.get(i + 1) != Some(&0x80) {
                    return Err(NbtError::BadUtf8); // C0 其余组合 = 过短编码。
                }
                out.push('\0');
                i += 2;
            }
            0xc2..=0xdf => {
                let b1 = *data.get(i + 1).ok_or(NbtError::BadUtf8)?;
                if !is_cont(b1) {
                    return Err(NbtError::BadUtf8);
                }
                let cp = ((b0 as u32 & 0x1f) << 6 | (b1 as u32 & 0x3f)) as u16;
                // 过短编码拒绝;c0 c1 已被上面的 range 排除。
                if cp < 0x80 {
                    return Err(NbtError::BadUtf8);
                }
                let c = char::from_u32(cp as u32).ok_or(NbtError::BadUtf8)?;
                out.push(c);
                i += 2;
            }
            0xe0..=0xef => {
                let b1 = *data.get(i + 1).ok_or(NbtError::BadUtf8)?;
                let b2 = *data.get(i + 2).ok_or(NbtError::BadUtf8)?;
                if !is_cont(b1) || !is_cont(b2) {
                    return Err(NbtError::BadUtf8);
                }
                let cp = (b0 as u32 & 0x0f) << 12 | (b1 as u32 & 0x3f) << 6 | (b2 as u32 & 0x3f);
                // 拒绝孤立 surrogate(ED A0.. 形式)与过短编码。
                let c = char::from_u32(cp).ok_or(NbtError::BadUtf8)?;
                out.push(c);
                i += 3;
            }
            // 4 字节 UTF-8(补充平面):原版 writeUTF 从 Java string 产出的是
            // 两个 3 字节 surrogate(孤立 surrogate 上面已拒),4 字节形式
            // 原版从不产出——我们读侧宽容接受(自我往返需要),写侧也产出
            // 标准 UTF-8 4 字节而非 CESU-8 surrogate 对。字节级与原版不
            // 兼容(登记为债 NBT-UTF8-CESU),字符级语义一致。
            0xf0..=0xf4 => {
                let b1 = *data.get(i + 1).ok_or(NbtError::BadUtf8)?;
                let b2 = *data.get(i + 2).ok_or(NbtError::BadUtf8)?;
                let b3 = *data.get(i + 3).ok_or(NbtError::BadUtf8)?;
                if !is_cont(b1) || !is_cont(b2) || !is_cont(b3) {
                    return Err(NbtError::BadUtf8);
                }
                let cp = (b0 as u32 & 0x07) << 18
                    | (b1 as u32 & 0x3f) << 12
                    | (b2 as u32 & 0x3f) << 6
                    | (b3 as u32 & 0x3f);
                let c = char::from_u32(cp).ok_or(NbtError::BadUtf8)?;
                out.push(c);
                i += 4;
            }
            _ => return Err(NbtError::BadUtf8),
        }
    }
    Ok(out)
}

fn is_cont(b: u8) -> bool {
    b & 0xc0 == 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_two_byte_form_decodes() {
        assert_eq!(decode(&[b'a', 0xc0, 0x80, b'b']).unwrap(), "a\0b");
    }

    #[test]
    fn encode_decode_roundtrip_all_planes() {
        for s in ["", "plain", "中文😀", "a\0b\ncé"] {
            let mut buf = Vec::new();
            encode_into(s, &mut buf);
            assert_eq!(buf.len(), encoded_len(s));
            assert_eq!(decode(&buf).unwrap(), s, "roundtrip {s:?}");
        }
    }

    #[test]
    fn lone_surrogate_form_rejected() {
        // ED A0 80 = U+D800 直接编码(Java 孤立 surrogate),Rust 无法承载。
        assert!(matches!(
            decode(&[0xed, 0xa0, 0x80]),
            Err(NbtError::BadUtf8)
        ));
    }
}
