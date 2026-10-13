//! 配额防 OOM。常数与算法照原版 26.1 实物两件套:
//!
//! 1. [`NbtAccounter`] 本体(NbtAccounter.java):默认 quota 2Mi
//!    (`DEFAULT_NBT_QUOTA`)、非网络 100Mi(`UNCOMPRESSED_NBT_QUOTA`)、
//!    深度 512;`usage + size > quota` 即失败(NbtAccounter.java:38-51)。
//! 2. 各 TagType.load 的逐 tag 记账表(常数=实物 file:line):
//!    - 标量 self:byte 9(ByteTag.java:20)/ short 10(ShortTag.java:20)/
//!      int 12(IntTag.java:20)/ long 16(LongTag.java:20)/
//!      float 12(FloatTag.java:22)/ double 16(DoubleTag.java:22)
//!    - 定长数组:self 24 + 元素宽度×长度(ByteArrayTag.java:23-25 /
//!      IntArrayTag.java:23-25 / LongArrayTag.java:23-25)
//!    - String:self 36 + 2×UTF-16 char 数(StringTag.java:21-23)
//!    - List:self 36 + 4×count(ListTag.java:31-38)
//!    - Compound:self 48,每新增键 +36,键名单独记 28 + 2×char 数
//!      (CompoundTag.java:53/61/131-132;重复键不再计 36——
//!      `values.put(..)==null` 才加,同处)
//!
//! 与原版差异:原版超限抛异常,我们返回 [`NbtError`]。

use crate::error::NbtError;

/// 原版 `NbtAccounter.DEFAULT_NBT_QUOTA`:2 Mi「字符口径」字节数。
pub const DEFAULT_NBT_QUOTA: u64 = 2_097_152;
/// 原版 `NbtAccounter.UNCOMPRESSED_NBT_QUOTA`:100 Mi,读非网络来源用。
pub const UNCOMPRESSED_NBT_QUOTA: u64 = 104_857_600;
/// 原版 `NbtAccounter.MAX_STACK_DEPTH`。
pub const MAX_STACK_DEPTH: u32 = 512;
/// StringTag.load self(36) + 2×char(StringTag.java:21-23)。
pub(crate) const STRING_SELF_SIZE: u64 = 36;
pub(crate) const STRING_CHAR_SIZE: u64 = 2;
/// CompoundTag.loadCompound self(CompoundTag.java:53)。
pub(crate) const COMPOUND_SELF: u64 = 48;
/// 每个新键的 map entry(CompoundTag.java:61,重复键不计)。
pub(crate) const COMPOUND_ENTRY: u64 = 36;
/// 键名 self(CompoundTag.java:131)+ 2×char(:132)。
pub(crate) const KEY_SELF: u64 = 28;
/// ListTag.loadList self(ListTag.java:31)。
pub(crate) const LIST_SELF: u64 = 36;
/// 每槽位(ListTag.java:38)。
pub(crate) const LIST_SLOT: u64 = 4;
/// 定长数组 self(ByteArrayTag/IntArrayTag/LongArrayTag.java:23)。
pub(crate) const ARRAY_SELF: u64 = 24;
/// 各标量 self(ByteTag.java:20 … DoubleTag.java:22)。
pub(crate) const SELF_BYTE: u64 = 9;
pub(crate) const SELF_SHORT: u64 = 10;
pub(crate) const SELF_INT: u64 = 12;
pub(crate) const SELF_LONG: u64 = 16;
pub(crate) const SELF_FLOAT: u64 = 12;
pub(crate) const SELF_DOUBLE: u64 = 16;

/// Java `String.length()` 口径:UTF-16 code unit 数(补充平面 2 单元)。
#[must_use]
pub(crate) fn utf16_len(s: &str) -> u64 {
    s.chars().map(char::len_utf16).sum::<usize>() as u64
}

/// 读取期的字节配额 + 深度栈。用法见 [`crate::read`]。
#[derive(Debug, Clone)]
pub struct NbtAccounter {
    quota: u64,
    usage: u64,
    max_depth: u32,
    depth: u32,
}

impl Default for NbtAccounter {
    fn default() -> Self {
        Self::default_quota()
    }
}

impl NbtAccounter {
    /// 原版 `defaultQuota()` 语义(2Mi chars / 深度 512)。
    pub fn default_quota() -> Self {
        Self::new(DEFAULT_NBT_QUOTA, MAX_STACK_DEPTH)
    }

    /// 原版 `uncompressedQuota()` 语义(100Mi / 深度 512)。读本地结构/存档推荐。
    pub fn uncompressed_quota() -> Self {
        Self::new(UNCOMPRESSED_NBT_QUOTA, MAX_STACK_DEPTH)
    }

    /// 原版 `unlimitedHeap()` 语义:配额开满,深度上限保留。
    pub fn unlimited() -> Self {
        Self::new(u64::MAX, MAX_STACK_DEPTH)
    }

    /// 自定义配额与深度。
    pub fn new(quota: u64, max_depth: u32) -> Self {
        NbtAccounter {
            quota,
            usage: 0,
            max_depth,
            depth: 0,
        }
    }

    /// 记 `per * count` 字节账;溢出配额返回 [`NbtError::TooLarge`]。
    pub(crate) fn account(
        &mut self,
        per: u64,
        count: u64,
        what: &'static str,
    ) -> Result<(), NbtError> {
        let size = per.saturating_mul(count);
        self.account_bytes(size, what)
    }

    /// 记一笔字节账。
    pub(crate) fn account_bytes(&mut self, size: u64, what: &'static str) -> Result<(), NbtError> {
        match self.usage.checked_add(size) {
            Some(next) if next <= self.quota => {
                self.usage = next;
                Ok(())
            }
            _ => Err(NbtError::TooLarge {
                what,
                got: self.usage as i64 + size as i64,
                max: self.quota as i64,
            }),
        }
    }

    /// 进入一层 compound/list。原版 pushDepth。
    pub(crate) fn push_depth(&mut self) -> Result<(), NbtError> {
        if self.depth >= self.max_depth {
            return Err(NbtError::TooDeep {
                max_depth: self.max_depth,
            });
        }
        self.depth += 1;
        Ok(())
    }

    /// 退出一层。读取路径成对调用,不可能在顶层弹出。原版 popDepth。
    pub(crate) fn pop_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// 当前记账(测试用)。
    pub fn usage(&self) -> u64 {
        self.usage
    }
}
