//! Java `DataOutput.writeUTF` / `DataInput.readUTF` 的 modified UTF-8 编解码
//! (口径 = OpenJDK `DataOutputStream.writeUTF`/`DataInputStream.readUTF` 实物):
//!
//! - U+0000:modified UTF-8 编成 `0xC0 0x80`(标准 UTF-8 是单字节 0x00,Java
//!   解码器只认双字节形式;本仓读侧对裸 0x00 保持宽容——原版从不产出,
//!   接受它不改变与 Java 互读的合法集)。
//! - 非 BMP 码位:Java `String` 以 surrogate 对承载,`writeUTF` 对每个
//!   surrogate 各发一个 3 字节 `0xED 0xA0..`/`0xED 0xB0..` 形式(CESU-8,
//!   共 6 字节)。本仓写侧同形(补充平面 char → surrogate 对 2×3 字节);
//!   读侧把「高 surrogate 紧跟低 surrogate」的合法 CESU-8 对合成单一
//!   码位,与 Java `readUTF` 一致。
//! - 标准 UTF-8 4 字节形式:原版读写都不产出,`DataInputStream` 的
//!   UTF-16 解码器对它直接判 malformed → 本仓同样拒绝(`BadUtf8`)。
//! - **平台限制(登记 KNOWN-DIVERGENCE)**:孤立 surrogate(无配对)在
//!   Java `String` 中可持有,`readUTF` 成功;Rust `String` 表示层无法
//!   承载 → 本仓解码报 [`NbtError::BadUtf8`]。这是 Rust 字符串表示的
//!   硬限制而非简化:原版合法数据(注册表名/SNBT/结构模板)从不出现
//!   孤立 surrogate。

use crate::error::NbtError;

/// 把 `s` 按 modified UTF-8 编码追加到 `out`,返回字节数(≤65535,由调用方
/// 在写出长度前缀前校验)。补充平面码位写为 CESU-8 surrogate 对(2×3 字节),
/// 与 Java `writeUTF` 字节级一致。
pub(crate) fn encode_into(s: &str, out: &mut Vec<u8>) {
    for c in s.chars() {
        match c {
            '\u{0001}'..='\u{007f}' => out.push(c as u8),
            // U+0000:mutf8 独有的双字节形式(Java writeUTF 实物口径)。
            '\u{0}' => out.extend_from_slice(&[0xc0, 0x80]),
            c if (c as u32) <= 0xFFFF => {
                let v = c as u32;
                if v < 0x800 {
                    out.push(0xc0 | (v >> 6) as u8);
                    out.push(0x80 | (v & 0x3f) as u8);
                } else {
                    out.push(0xe0 | (v >> 12) as u8);
                    out.push(0x80 | ((v >> 6) & 0x3f) as u8);
                    out.push(0x80 | (v & 0x3f) as u8);
                }
            }
            // 补充平面 → surrogate 对,每个 surrogate 各编 3 字节
            // (OpenJDK DataOutputStream.writeUTF 对 char 单元逐个编码的实物口径)。
            c => {
                let v = c as u32 - 0x10000;
                encode_surrogate(0xD800 + (v >> 10), out);
                encode_surrogate(0xDC00 + (v & 0x3FF), out);
            }
        }
    }
}

fn encode_surrogate(unit: u32, out: &mut Vec<u8>) {
    out.push(0xe0 | (unit >> 12) as u8);
    out.push(0x80 | ((unit >> 6) & 0x3f) as u8);
    out.push(0x80 | (unit & 0x3f) as u8);
}

/// modified UTF-8 字节数(U+0000 计 2,补充平面计 6=CESU-8 代理对;
/// 续字节并入首字节计数)。
pub(crate) fn encoded_len(s: &str) -> usize {
    // char 口径更直白:代理对展开后每码位 6 字节。
    s.chars().fold(0usize, |n, c| {
        n + match c as u32 {
            0 => 2,
            1..=0x7F => 1,
            0x80..=0x7FF => 2,
            0x800..=0xFFFF => 3,
            _ => 6,
        }
    })
}

/// 解码 `data` 的 modified UTF-8(CESU-8 兼容):合法 surrogate 对合成单一
/// 码位;孤立 surrogate、4 字节标准 UTF-8、过短编码 → [`NbtError::BadUtf8`]。
pub(crate) fn decode(data: &[u8]) -> Result<String, NbtError> {
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
                let cp = decode_3(data, i)?;
                match cp {
                    // 高 surrogate:必须紧跟低 surrogate 的 3 字节形式,合成
                    // 单一码位(Java readUTF 对 CESU-8 对的实物行为)。
                    hi @ 0xD800..=0xDBFF => {
                        let lo = decode_3(data, i + 3)?;
                        if !(0xDC00..=0xDFFF).contains(&lo) {
                            return Err(NbtError::BadUtf8); // 无配对低 surrogate。
                        }
                        let c = char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                            .ok_or(NbtError::BadUtf8)?;
                        out.push(c);
                        i += 6;
                    }
                    // 低 surrogate 先行 / 孤立 surrogate:Rust String 不可表示。
                    0xD800..=0xDFFF => return Err(NbtError::BadUtf8),
                    other => {
                        let c = char::from_u32(other).ok_or(NbtError::BadUtf8)?;
                        out.push(c);
                        i += 3;
                    }
                }
            }
            // 0xf0..(标准 UTF-8 4 字节)与 0xc0/0xc1 过短形式、裸续字节等
            // 全部落入拒绝分支:Java UTF-16 解码器对 4 字节形式同样判 malformed。
            _ => return Err(NbtError::BadUtf8),
        }
    }
    Ok(out)
}

/// 解一个 3 字节序列,返回 16 位单元(允许 surrogate 值——调用侧决定配对)。
/// 结构校验口径与既有实现一致(三字节 + 续字节;E0 系过短形式按原宽容面
/// 保留,写侧永远产出 canonical 形式)。
fn decode_3(data: &[u8], i: usize) -> Result<u32, NbtError> {
    let b0 = *data.get(i).ok_or(NbtError::BadUtf8)?;
    let b1 = *data.get(i + 1).ok_or(NbtError::BadUtf8)?;
    let b2 = *data.get(i + 2).ok_or(NbtError::BadUtf8)?;
    if !is_cont(b1) || !is_cont(b2) {
        return Err(NbtError::BadUtf8);
    }
    Ok((b0 as u32 & 0x0f) << 12 | (b1 as u32 & 0x3f) << 6 | b2 as u32 & 0x3f)
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

    /// 与 Java `DataOutputStream.writeUTF` 编码规则手推对照表(自造向量,
    /// 规则:每个 UTF-16 code unit 独立编码;补充平面 code unit 即 surrogate):
    /// U+1F600 😀 → D83D DE00 → `ED A0 BD` `ED B8 80`;
    /// U+1D54F 𝕏 → D835 DD4F → `ED A0 B5` `ED B5 8F`;
    /// U+10348 𐍈 → D800 DF48 → `ED A0 80` `ED BD 88`;
    /// U+4E2D 中 → E4 B8 AD;U+0000 → C0 80。
    #[test]
    fn cesu8_golden_vectors_match_java_rules() {
        let cases: [(&str, &[u8]); 6] = [
            ("😀", &[0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80][..]),
            ("𝕏", &[0xED, 0xA0, 0xB5, 0xED, 0xB5, 0x8F][..]),
            ("𐍈", &[0xED, 0xA0, 0x80, 0xED, 0xBD, 0x88][..]),
            ("中", &[0xE4, 0xB8, 0xAD][..]),
            ("\0", &[0xC0, 0x80][..]),
            (
                "a😀\0",
                &[b'a', 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80, 0xC0, 0x80][..],
            ),
        ];
        for (s, want) in cases {
            let mut buf = Vec::new();
            encode_into(s, &mut buf);
            assert_eq!(buf, want, "encode {s:?}");
            assert_eq!(encoded_len(s), want.len(), "len {s:?}");
            assert_eq!(decode(want).unwrap(), s, "decode {s:?}");
        }
    }

    #[test]
    fn surrogate_pairs_merge_back_to_back() {
        // 两个补充平面相邻(12 字节 CESU-8)→ 两个码位。
        let b = [
            0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80, // 😀
            0xED, 0xA0, 0xB5, 0xED, 0xB5, 0x8F, // 𝕏
        ];
        assert_eq!(decode(&b).unwrap(), "😀𝕏");
    }

    #[test]
    fn lone_surrogate_form_rejected() {
        // 孤立高 surrogate(无低配对)、孤立低 surrogate:Rust String 不可
        // 表示(平台限制,见模块头)。
        assert!(matches!(
            decode(&[0xed, 0xa0, 0x80]),
            Err(NbtError::BadUtf8)
        ));
        assert!(matches!(
            decode(&[0xed, 0xa0, 0x80, b'a']),
            Err(NbtError::BadUtf8)
        ));
        assert!(matches!(
            decode(&[0xed, 0xb8, 0x80]),
            Err(NbtError::BadUtf8)
        ));
    }

    #[test]
    fn four_byte_standard_utf8_rejected() {
        // F0 9F 98 80 = 标准 UTF-8 的 😀:Java readUTF 判 malformed,本仓同拒。
        assert!(matches!(
            decode(&[0xf0, 0x9f, 0x98, 0x80]),
            Err(NbtError::BadUtf8)
        ));
    }
}
