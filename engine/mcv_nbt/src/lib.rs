//! mcv_nbt — 全仓共享的通用 NBT 序列化底座。
//!
//! 定位:二进制 13 种 tag 的读/写(big-endian)、GZIP/ZLIB/RAW 三模式嗅探、
//! 配额防 OOM、SNBT 文本往返。**crate 内零游戏类型**——物品组件、实体
//! 持久化、结构模板等都是消费方,本 crate 只认 tag 树。
//!
//! 机制对照原版 26.1(nbt/NbtIo、TagTypes、NbtAccounter、StringTag),
//! 差异与简化全部登记在 KNOWN-DIVERGENCE.md(NBT-* 条目)。

mod accounter;
mod error;
mod read;
mod snbt;
mod tag;
mod utf8;
mod write;

pub use accounter::NbtAccounter;
pub use error::NbtError;
pub use read::{
    read_any_tag, read_auto, read_gzip, read_raw, read_raw_with, read_unnamed_tag, read_zlib,
};
pub use snbt::{parse_snbt, parse_snbt_compound, write_snbt};
pub use tag::{NbtCompound, NbtList, NbtValue, TAG_TYPE_BYTES, TagType};
pub use write::{write_gzip, write_raw, write_zlib};
