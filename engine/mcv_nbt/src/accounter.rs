//! 配额防 OOM。常数与算法照原版 26.1 NbtAccounter 实物:
//! 默认 quota 2Mi(按字符串字符数口径,字符串记账 36 + 2×len)、
//! 解压/无压缩口径 quota 100Mi、最大嵌套深度 512。
//! 与原版差异:原版超限抛异常,我们返回 [`NbtError`]。

use crate::error::NbtError;

/// 原版 `NbtAccounter.DEFAULT_NBT_QUOTA`:2 Mi「字符口径」字节数。
pub const DEFAULT_NBT_QUOTA: u64 = 2_097_152;
/// 原版 `NbtAccounter.UNCOMPRESSED_NBT_QUOTA`:100 Mi,读非网络来源用。
pub const UNCOMPRESSED_NBT_QUOTA: u64 = 104_857_600;
/// 原版 `NbtAccounter.MAX_STACK_DEPTH`。
pub const MAX_STACK_DEPTH: u32 = 512;
/// 原版 StringTag 记账:每个字符串 tag 固定 36 + 2×len(Java char 2 字节口径)。
pub(crate) const STRING_SELF_SIZE: u64 = 36;

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
