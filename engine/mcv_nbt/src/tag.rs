//! 通用值模型:13 种 tag 的内存表示。
//!
//! [`NbtCompound`] 保插入序(原版 Java 是 LinkedHashMap,SNBT 往返一致性与
//! 结构模板 palette 序都依赖保序语义)。类型 id 顺序照原版 TagTypes:
//! End=0 … LongArray=12。

use std::collections::HashMap;

/// 原版 TagTypes 表的类型字节常量。
pub const TAG_TYPE_BYTES: [u8; 13] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

/// tag 类型枚举(与字节 id 一一对应)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagType {
    /// 0,tag 树结束符/空 list 元素型。
    End,
    /// 1
    Byte,
    /// 2
    Short,
    /// 3
    Int,
    /// 4
    Long,
    /// 5
    Float,
    /// 6
    Double,
    /// 7
    ByteArray,
    /// 8
    String,
    /// 9
    List,
    /// 10
    Compound,
    /// 11
    IntArray,
    /// 12
    LongArray,
}

impl TagType {
    /// 字节 id → 类型;未知 id 返回 None。
    pub fn from_id(id: u8) -> Option<TagType> {
        match id {
            0 => Some(TagType::End),
            1 => Some(TagType::Byte),
            2 => Some(TagType::Short),
            3 => Some(TagType::Int),
            4 => Some(TagType::Long),
            5 => Some(TagType::Float),
            6 => Some(TagType::Double),
            7 => Some(TagType::ByteArray),
            8 => Some(TagType::String),
            9 => Some(TagType::List),
            10 => Some(TagType::Compound),
            11 => Some(TagType::IntArray),
            12 => Some(TagType::LongArray),
            _ => None,
        }
    }

    /// 类型 → 字节 id。
    pub fn id(self) -> u8 {
        match self {
            TagType::End => 0,
            TagType::Byte => 1,
            TagType::Short => 2,
            TagType::Int => 3,
            TagType::Long => 4,
            TagType::Float => 5,
            TagType::Double => 6,
            TagType::ByteArray => 7,
            TagType::String => 8,
            TagType::List => 9,
            TagType::Compound => 10,
            TagType::IntArray => 11,
            TagType::LongArray => 12,
        }
    }
}

/// 任意 tag 值。
#[derive(Debug, Clone)]
pub enum NbtValue {
    /// End,只出现在类型位,list 里不承载数据。
    End,
    /// TAG_Byte
    Byte(i8),
    /// TAG_Short
    Short(i16),
    /// TAG_Int
    Int(i32),
    /// TAG_Long
    Long(i64),
    /// TAG_Float
    Float(f32),
    /// TAG_Double
    Double(f64),
    /// TAG_Byte_Array
    ByteArray(Vec<i8>),
    /// TAG_Int_Array
    IntArray(Vec<i32>),
    /// TAG_Long_Array
    LongArray(Vec<i64>),
    /// TAG_String(合法输入不含孤立 surrogate,见 utf8 模块)
    String(String),
    /// TAG_List(同质序列,类型取首元素;空 list 类型字节 0)
    List(NbtList),
    /// TAG_Compound(保插入序)
    Compound(NbtCompound),
}

impl NbtValue {
    /// 本值的类型字节。
    pub fn tag_type(&self) -> TagType {
        match self {
            NbtValue::End => TagType::End,
            NbtValue::Byte(_) => TagType::Byte,
            NbtValue::Short(_) => TagType::Short,
            NbtValue::Int(_) => TagType::Int,
            NbtValue::Long(_) => TagType::Long,
            NbtValue::Float(_) => TagType::Float,
            NbtValue::Double(_) => TagType::Double,
            NbtValue::ByteArray(_) => TagType::ByteArray,
            NbtValue::IntArray(_) => TagType::IntArray,
            NbtValue::LongArray(_) => TagType::LongArray,
            NbtValue::String(_) => TagType::String,
            NbtValue::List(_) => TagType::List,
            NbtValue::Compound(_) => TagType::Compound,
        }
    }

    /// 数值宽化取 i32(照原版 NumericTag.asInt 的截断语义)。非数值 tag → None。
    pub fn as_i32(&self) -> Option<i32> {
        match *self {
            NbtValue::Byte(v) => Some(v as i32),
            NbtValue::Short(v) => Some(v as i32),
            NbtValue::Int(v) => Some(v),
            NbtValue::Long(v) => Some(v as i32),
            NbtValue::Float(v) => Some(clamp_cast_i64(v as f64) as i32),
            NbtValue::Double(v) => Some(clamp_cast_i64(v) as i32),
            _ => None,
        }
    }

    /// 数值宽化取 i64(截断语义同上)。非数值 tag → None。
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            NbtValue::Byte(v) => Some(v as i64),
            NbtValue::Short(v) => Some(v as i64),
            NbtValue::Int(v) => Some(v as i64),
            NbtValue::Long(v) => Some(v),
            NbtValue::Float(v) => Some(clamp_cast_i64(v as f64)),
            NbtValue::Double(v) => Some(clamp_cast_i64(v)),
            _ => None,
        }
    }

    /// 数值宽化取 f64。非数值 tag → None。
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            NbtValue::Byte(v) => Some(v as f64),
            NbtValue::Short(v) => Some(v as f64),
            NbtValue::Int(v) => Some(v as f64),
            NbtValue::Long(v) => Some(v as f64),
            NbtValue::Float(v) => Some(v as f64),
            NbtValue::Double(v) => Some(v),
            _ => None,
        }
    }

    /// String tag → &str,其余 → None。
    pub fn as_string(&self) -> Option<&str> {
        match self {
            NbtValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Compound tag 借用访问,缺失/类型不符 → None(不 panic)。
    pub fn as_compound(&self) -> Option<&NbtCompound> {
        match self {
            NbtValue::Compound(c) => Some(c),
            _ => None,
        }
    }

    /// List tag 借用访问。
    pub fn as_list(&self) -> Option<&NbtList> {
        match self {
            NbtValue::List(l) => Some(l),
            _ => None,
        }
    }

    /// ByteArray 借用访问。
    pub fn as_byte_array(&self) -> Option<&[i8]> {
        match self {
            NbtValue::ByteArray(a) => Some(a),
            _ => None,
        }
    }

    /// IntArray 借用访问。
    pub fn as_int_array(&self) -> Option<&[i32]> {
        match self {
            NbtValue::IntArray(a) => Some(a),
            _ => None,
        }
    }

    /// LongArray 借用访问。
    pub fn as_long_array(&self) -> Option<&[i64]> {
        match self {
            NbtValue::LongArray(a) => Some(a),
            _ => None,
        }
    }

    /// compound 便捷取子 compound。
    pub fn get_compound(&self, key: &str) -> Option<&NbtCompound> {
        self.as_compound()?.get_compound(key)
    }
}

/// Java `(int)/(long)` 强转浮点的饱和语义(±Inf/超界 → i64::MIN/MAX,NaN → 0)。
fn clamp_cast_i64(v: f64) -> i64 {
    if v.is_nan() {
        0
    } else if v >= i64::MAX as f64 {
        i64::MAX
    } else if v <= i64::MIN as f64 {
        i64::MIN
    } else {
        v as i64
    }
}

/// 保插入序的 compound(语义对照原版 LinkedHashMap 后端 CompoundTag)。
#[derive(Debug, Clone, Default)]
pub struct NbtCompound {
    order: Vec<String>,
    map: HashMap<String, NbtValue>,
}

impl NbtCompound {
    /// 空 compound。
    pub fn new() -> Self {
        NbtCompound {
            order: Vec::new(),
            map: HashMap::new(),
        }
    }

    /// 插入/覆盖。End 不允许作值(原版 compound 从不持 End)。
    /// 返回旧值(若有)。
    pub fn insert(&mut self, key: impl Into<String>, value: NbtValue) -> Option<NbtValue> {
        debug_assert!(!matches!(value, NbtValue::End), "compound 不得持有 End 值");
        let key: String = key.into();
        match self.map.insert(key.clone(), value) {
            Some(old) => Some(old),
            None => {
                self.order.push(key);
                None
            }
        }
    }

    /// 按键取借用。缺失 → None。
    pub fn get(&self, key: &str) -> Option<&NbtValue> {
        self.map.get(key)
    }

    /// 取子 compound。
    pub fn get_compound(&self, key: &str) -> Option<&NbtCompound> {
        self.get(key)?.as_compound()
    }

    /// 取子 list。
    pub fn get_list(&self, key: &str) -> Option<&NbtList> {
        self.get(key)?.as_list()
    }

    /// 取字符串。
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_string()
    }

    /// 取数值 i32(数值 tag 宽化)。
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key)?.as_i32()
    }

    /// 取数值 i64。
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key)?.as_i64()
    }

    /// 取 f64。
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }

    /// 取 IntArray。
    pub fn get_int_array(&self, key: &str) -> Option<&[i32]> {
        self.get(key)?.as_int_array()
    }

    /// 取 ByteArray。
    pub fn get_byte_array(&self, key: &str) -> Option<&[i8]> {
        self.get(key)?.as_byte_array()
    }

    /// 取 LongArray。
    pub fn get_long_array(&self, key: &str) -> Option<&[i64]> {
        self.get(key)?.as_long_array()
    }

    /// 是否含键。
    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// 删除并返回旧值,保持剩余键相对序。
    pub fn remove(&mut self, key: &str) -> Option<NbtValue> {
        let old = self.map.remove(key)?;
        if let Some(i) = self.order.iter().position(|k| k == key) {
            self.order.remove(i);
        }
        Some(old)
    }

    /// 迭代 (key, value),插入序。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &NbtValue)> {
        self.order.iter().map(move |k| (k.as_str(), &self.map[k]))
    }

    /// 键迭代,插入序。
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }

    /// 条目数。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// 同质 list。元素类型字节取首元素;空 list 类型字节 0(原版 ListTag 语义)。
#[derive(Debug, Clone)]
pub struct NbtList {
    element_type: TagType,
    items: Vec<NbtValue>,
}

impl Default for NbtList {
    fn default() -> Self {
        Self::new()
    }
}

impl NbtList {
    /// 空 list(元素类型 End,写入时类型字节 0)。
    pub fn new() -> Self {
        NbtList {
            element_type: TagType::End,
            items: Vec::new(),
        }
    }

    /// 由已知元素直接建 list。空切片 → 类型 End。
    pub fn from_vec(items: Vec<NbtValue>) -> Self {
        let element_type = items.first().map_or(TagType::End, |v| v.tag_type());
        NbtList {
            element_type,
            items,
        }
    }

    /// 读取路径专用:legacy 文件可含混型 list,类型字节与 items 原样保留。
    pub(crate) fn from_raw(element_type: TagType, items: Vec<NbtValue>) -> Self {
        NbtList {
            element_type,
            items,
        }
    }

    /// 追加元素;类型不符返回 Err(值被回传,不落入 list)。
    pub fn push(&mut self, value: NbtValue) -> Result<(), NbtValue> {
        let t = value.tag_type();
        if t == TagType::End {
            return Err(value);
        }
        if self.items.is_empty() {
            self.element_type = t;
        } else if t != self.element_type {
            return Err(value);
        }
        self.items.push(value);
        Ok(())
    }

    /// 元素类型字节(空 list = End)。
    pub fn element_type(&self) -> TagType {
        self.element_type
    }

    /// 按下标取。
    pub fn get(&self, i: usize) -> Option<&NbtValue> {
        self.items.get(i)
    }

    /// 元素个数。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 借用切片。
    pub fn items(&self) -> &[NbtValue] {
        &self.items
    }
}

/// 结构相等:浮点按位比较(NaN==NaN 为真),保证「round-trip 后相等」
/// 类测试的口径 = 「字节往返无损」口径。
impl PartialEq for NbtValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (NbtValue::End, NbtValue::End) => true,
            (NbtValue::Byte(a), NbtValue::Byte(b)) => a == b,
            (NbtValue::Short(a), NbtValue::Short(b)) => a == b,
            (NbtValue::Int(a), NbtValue::Int(b)) => a == b,
            (NbtValue::Long(a), NbtValue::Long(b)) => a == b,
            (NbtValue::Float(a), NbtValue::Float(b)) => a.to_bits() == b.to_bits(),
            (NbtValue::Double(a), NbtValue::Double(b)) => a.to_bits() == b.to_bits(),
            (NbtValue::ByteArray(a), NbtValue::ByteArray(b)) => a == b,
            (NbtValue::IntArray(a), NbtValue::IntArray(b)) => a == b,
            (NbtValue::LongArray(a), NbtValue::LongArray(b)) => a == b,
            (NbtValue::String(a), NbtValue::String(b)) => a == b,
            (NbtValue::List(a), NbtValue::List(b)) => a == b,
            (NbtValue::Compound(a), NbtValue::Compound(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for NbtValue {}

impl PartialEq for NbtList {
    fn eq(&self, other: &Self) -> bool {
        self.element_type == other.element_type && self.items == other.items
    }
}

impl Eq for NbtList {}

impl PartialEq for NbtCompound {
    /// 相等 = 键序 + 值都相等(保序是语义,往返测试依赖它)。
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order && self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl Eq for NbtCompound {}
