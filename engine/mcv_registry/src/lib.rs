//! 通用数据驱动注册表基建：标识符 / tag 键 / tag 集合。
//!
//! 零领域知识、零外部依赖：本 crate 不出现 BlockId/ItemDef 等游戏类型，
//! id 域统一为 `u16`（消费端自行保证与各自注册表行序一致）。
//! 数据行由 gen 脚本产出（`ci/gen-tags.py` → `tags_gen.inc.rs`），
//! 运行期只读二分查找，零解引用、零分配。
//!
//! ## 冻结语义偏差登记（显式决定，非遗漏）
//! 原版 `MappedRegistry.java:76-88` 的 freeze 是**运行期写保护**
//! （frozen 后 register 抛 `IllegalStateException`）；本引擎注册表与
//! tag 表全部是 `&'static` 编译期静态数据，写入通道在类型层面不存在，
//! 「天然冻结」——故不照抄 freeze/validateWrite 机制。

/// 命名空间标识符（原版 `Identifier.java` 等价）：`namespace:path`。
///
/// 字符集照抄原版：命名空间 `[a-z0-9_.-]`（`Identifier.java:24`，
/// 且整串不得为 `".."`），路径 `[a-z0-9/._-]`（`Identifier.java:253`
/// `validPathChar`）；`:` 分割、空命名空间缺省 `"minecraft"`
/// （`Identifier.java:59` `bySeparator`/`DEFAULT_NAMESPACE`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Identifier {
    namespace: &'static str,
    path: &'static str,
}

/// 命名空间字符集（`Identifier.java:245-247` `validNamespaceChar`）。
pub const fn valid_namespace_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.')
}

/// 路径字符集（`Identifier.java:252-254` `validPathChar`，多 `/`）。
pub const fn valid_path_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '/' | '.')
}

impl Identifier {
    /// 原版 `isValidNamespace`：字符集全合法且整串非 `".."`。
    /// 照抄原版：空串按「逐字符全称真」通过（构造入口另有非空要求处见 parse）。
    pub const fn is_valid_namespace(s: &str) -> bool {
        if s == ".." {
            return false;
        }
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            // 非 ASCII 直接判非法；ASCII 再按字符集核对。
            if !(b[i].is_ascii() && valid_namespace_char(b[i] as char)) {
                return false;
            }
            i += 1;
        }
        true
    }

    /// 原版 `isValidPath`（`Identifier.java:220-229`）：逐字符核对，空串为真。
    pub const fn is_valid_path(s: &str) -> bool {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if !(b[i].is_ascii() && valid_path_char(b[i] as char)) {
                return false;
            }
            i += 1;
        }
        true
    }

    /// 显式两参构造（原版 `tryBuild`，`Identifier.java:55-57`）；非法返 None。
    pub const fn new(namespace: &'static str, path: &'static str) -> Option<Self> {
        if Self::is_valid_namespace(namespace) && Self::is_valid_path(path) {
            Some(Self { namespace, path })
        } else {
            None
        }
    }

    /// 字符串解析（原版 `tryParse`+`bySeparator`，`Identifier.java:59-71`）：
    /// 按**首个** `:` 分割；`:` 前为空或无 `:` 时命名空间缺省 `minecraft`；
    /// 空输入、非法字符 → None。
    pub const fn parse(s: &'static str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b':' {
                // 首个冒号处分割（Java indexOf 语义）。
                let path = &s[i + 1..];
                if i == 0 {
                    return Self::new("minecraft", path);
                }
                return Self::new(&s[..i], path);
            }
            i += 1;
        }
        // 无冒号：整串为路径，命名空间缺省。
        Self::new("minecraft", s)
    }

    pub const fn namespace(&self) -> &'static str {
        self.namespace
    }

    pub const fn path(&self) -> &'static str {
        self.path
    }
}

impl core::fmt::Display for Identifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.path)
    }
}

/// tag 键：注册表名 + tag 路径（原版 TagKey 的静态化表示）。
/// const 可构造、全等比较。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TagKey {
    pub registry: &'static str,
    pub path: &'static str,
}

impl TagKey {
    pub const fn new(registry: &'static str, path: &'static str) -> Self {
        Self { registry, path }
    }
}

/// tag 集合：排序去重 `u16` id 的静态数组 + 二分查找。
/// id 域为不透明 `u16`——本 crate 不知道它指向方块还是物品。
#[derive(Clone, Copy, Debug)]
pub struct TagSet {
    ids: &'static [u16],
}

impl TagSet {
    /// 构造。约定 `ids` 已排序去重（gen 脚本保证；运行期零成本二分）。
    pub const fn new(ids: &'static [u16]) -> Self {
        Self { ids }
    }

    /// 二分命中（要求升序，见 [`TagSet::new`] 约定）。
    pub fn contains(&self, id: u16) -> bool {
        self.ids.binary_search(&id).is_ok()
    }

    pub const fn len(&self) -> usize {
        self.ids.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn iter(&self) -> core::slice::Iter<'static, u16> {
        self.ids.iter()
    }
}

/// 一条 tag 数据行：键 + 解出后的成员集（gen 产物单位）。
#[derive(Clone, Copy, Debug)]
pub struct TagEntry {
    pub key: TagKey,
    pub set: TagSet,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_valid_forms() {
        let id = Identifier::parse("minecraft:stone").unwrap();
        assert_eq!(id.namespace(), "minecraft");
        assert_eq!(id.path(), "stone");
        // 路径段允许 / . - _（Identifier.java:253）。
        let id = Identifier::parse("mcv:worldgen/tree/oak_1").unwrap();
        assert_eq!(id.namespace(), "mcv");
        assert_eq!(id.path(), "worldgen/tree/oak_1");
        // ":path" 形态冒号前为空 → 缺省命名空间（bySeparator :64-66）。
        let id = Identifier::parse(":x.y-z").unwrap();
        assert_eq!(id.namespace(), "minecraft");
        assert_eq!(id.to_string(), "minecraft:x.y-z");
    }

    #[test]
    fn identifier_default_namespace() {
        let id = Identifier::parse("stone").unwrap();
        assert_eq!(id.namespace(), "minecraft");
        assert_eq!(id.path(), "stone");
        assert_eq!(id.to_string(), "minecraft:stone");
    }

    #[test]
    fn identifier_invalid_forms() {
        // 空输入。
        assert!(Identifier::parse("").is_none());
        // 命名空间大写/非法字符（[a-z0-9_.-] 之外）。
        assert!(Identifier::parse("Minecraft:stone").is_none());
        assert!(Identifier::parse("mi de:stone").is_none());
        // 第二个冒号落在命名空间里 → 非法字符。
        assert!(Identifier::parse("a:b:c").is_none());
        // 路径非法字符（大写、空格、#）。
        assert!(Identifier::parse("minecraft:Stone").is_none());
        assert!(Identifier::parse("minecraft:mi de").is_none());
        // 命名空间整串 ".."（isValidNamespace 特判）。
        assert!(Identifier::new("..", "x").is_none());
        // 路径含冒号前的命名空间为空串 OK，但显式 new 空命名空间照抄原版
        // 逐字符真 → 允许；这里锁「非 .. 即可」的现状。
        assert!(Identifier::new("", "x").is_some());
    }

    #[test]
    fn tag_key_eq() {
        const K1: TagKey = TagKey::new("block", "mineable/pickaxe");
        const K2: TagKey = TagKey::new("block", "mineable/pickaxe");
        const K3: TagKey = TagKey::new("item", "mineable/pickaxe");
        assert_eq!(K1, K2);
        assert_ne!(K1, K3);
    }

    #[test]
    fn tag_set_binary_search() {
        const S: TagSet = TagSet::new(&[1, 5, 8, 9, 200]);
        assert!(S.contains(1));
        assert!(S.contains(8));
        assert!(S.contains(200));
        assert!(!S.contains(0));
        assert!(!S.contains(6));
        assert!(!S.contains(201));
        assert_eq!(S.len(), 5);
        assert_eq!(
            S.iter().copied().collect::<Vec<u16>>(),
            vec![1, 5, 8, 9, 200]
        );
    }

    #[test]
    fn tag_set_empty() {
        const S: TagSet = TagSet::new(&[]);
        assert!(S.is_empty());
        assert!(!S.contains(0));
        assert_eq!(S.len(), 0);
    }
}
