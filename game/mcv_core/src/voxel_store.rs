//! E7a：内存体素 palette 压缩存储 [`VoxelStore`]。
//!
//! 原版机制摘要（src-26.1 只读勘察，实装逐项对齐）：
//! - `chunk/LevelChunkSection.java:13-15,21`——每 section 16³=4096，
//!   竖直持 `PalettedContainer<BlockState>`；本仓 384 高柱 = 24 section
//!   （[`crate::SECTION_COUNT`]）。
//! - `chunk/PalettedContainer.java:75-79`——容器初始 0bit 单值档；
//!   :81-92 `createOrReuseData`——bits=0 走 ZeroBitStorage（零数据），
//!   否则 SimpleBitStorage(bits, entryCount)；:94-101 `onResize`——按
//!   新 palette 尺寸 ceillog2 建新档、`Data.copyFrom`（:312-319）按
//!   旧存储扫描序逐 entry 重编码（palette 插入序 = 首次出现序，对齐
//!   原版 ObjectList 语义）、只升不降（无缩档/compact）。
//! - `chunk/Strategy.java:33-48`（方块表）——0=单值；1-4=linear
//!   （**内存位宽一律 4**，case 1..4 全落 `FOUR_BITS_LINEAR` :14/:39）；
//!   5-8=hashmap；>8=global 回退（:44 `Configuration.Global`，
//!   `globalPaletteBitsInMemory = ceillog2(globalMap.size())` :28）。
//! - `SimpleBitStorage.java:240-301`——long 打包 LSB-first，
//!   valuesPerLong = 64/bits，cell = idx/vpl、bitoff = (idx%vpl)*bits。
//!
//! # 与原版的偏差（全部登记为债）
//! - **global 档位宽 = 16**：本仓体素值是全域 u16（bit0-11 方块 id +
//!   bit12-15 状态 nibble，见 `crate::ID_MASK`/`BlockId::with_state`），
//!   值域即 65536 → ceillog2 = 16；原版 IdMap 只数注册状态（~几千）。
//!   代价仅在 >256 值/section 的极端场景出现（正常地形不触达）。
//! - **from_flat_u16 直建最小档**：原版读盘走 unpack 按盘上 palette
//!   建档；本仓盘格式仍是平铺 u16（E7a 不动盘格式，盘 palette 化 =
//!   债），重建时按「section 首值起单值档、扫描遇新值才换档」得到
//!   最小驻留（比原版重放 set 序列更省，档位与解码字节仍确定性）。
//! - **hashmap 档位宽 5..8 的 `HashMap` 只做查找**：palette 值与 id
//!   一律存插入序 `Vec`（对齐原版 ObjectList），id 分配/解码只依赖
//!   扫描序——HashMap 的随机哈希序不外泄，构建确定性由测试锁定。
//!
//! 禁 unsafe；packed bit 读写为自写显式实现（无 MAGIC 乘除表，用
//! 整除/取余 + 移位，配 `debug_assert` 界断言）。

use std::collections::HashMap;

use crate::{BlockId, CHUNK_VOL, SECTION_COUNT, SECTION_VOL, WORLD_MIN_Y};

/// linear 档内存位宽（Strategy.java:14 FOUR_BITS_LINEAR——1..=16 值
/// 一律 4bit 存储）。
const LINEAR_BITS: usize = 4;
/// global 档位宽（值域 = u16 全集，见模块注释偏差条目）。
const GLOBAL_BITS: usize = 16;
/// 单 section 槽位数（LevelChunkSection.java:15 SECTION_SIZE）。
const SEC_SIZE: usize = SECTION_VOL; // 4096

/// 与 `Mth.ceillog2` 等价（Strategy.java:88-90）：ceil(log2(n))，
/// `ceillog2(1) = 0`（1 值可 0bit 表示）。
const fn ceillog2(n: usize) -> usize {
    if n <= 1 {
        0
    } else {
        usize::BITS as usize - (n - 1).leading_zeros() as usize
    }
}

/// palette 表示（值域 u16；id 从 0 递增，插入序 = 首现序）。
#[derive(Clone)]
enum Pal {
    /// SingleValuePalette：恒 0 号值，无存储。
    Single(u16),
    /// LinearPalette（≤16 值，线性扫描；对齐原版查找方式）。
    Linear(Vec<u16>),
    /// HashMapPalette（≤256 值）：`vals` 插入序（id = 下标），
    /// `index` 只做 O(1) 查找，不参与任何序语义。
    Hash {
        vals: Vec<u16>,
        index: HashMap<u16, u8>,
    },
    /// GlobalPalette：id = 值本身（u16 全值域），无表。
    Global,
}

/// 单个 16³ section 的 palette 压缩存储。
#[derive(Clone)]
pub struct VoxelSection {
    /// 存储位宽：0 | 4（linear 档）| 5..=8（hashmap 档）| 16（global）。
    bits: usize,
    /// LSB-first long 打包（SimpleBitStorage.java:240-301 同构），
    /// `bits == 0` 时为空 Vec（ZeroBitStorage 同语义）。
    data: Vec<u64>,
    pal: Pal,
}

impl VoxelSection {
    fn new_uniform(v: u16) -> Self {
        Self {
            bits: 0,
            data: Vec::new(),
            pal: Pal::Single(v),
        }
    }

    /// 存储位宽（0/4/5..8/16），行为锁口径。
    #[inline]
    pub fn bits(&self) -> usize {
        self.bits
    }

    /// palette 项数（global 档 = 0，无表；原版 GlobalPalette 无 size 语义）。
    pub fn palette_len(&self) -> usize {
        match &self.pal {
            Pal::Single(_) => 1,
            Pal::Linear(v) => v.len(),
            Pal::Hash { vals, .. } => vals.len(),
            Pal::Global => 0,
        }
    }

    /// 单值档（0bit）判定——原版 SingleValuePalette 专属；换档后即使
    /// 全挖回同值也不缩档（PalettedContainer.java:94-101 只升不降）。
    #[inline]
    pub fn is_uniform(&self) -> bool {
        matches!(self.pal, Pal::Single(_))
    }

    /// 单值档的常驻值；其余档 `None`。
    #[inline]
    pub fn uniform_value(&self) -> Option<u16> {
        match self.pal {
            Pal::Single(v) => Some(v),
            _ => None,
        }
    }

    #[inline]
    fn vpl(&self) -> usize {
        debug_assert!(self.bits > 0);
        // SimpleBitStorage.java:249 `(char)(64 / bits)`——整除。
        64 / self.bits
    }

    /// 打包读（自写显式实现，对照 SimpleBitStorage.java:295-301）。
    #[inline]
    fn packed_get(&self, idx: usize) -> u64 {
        debug_assert!(self.bits > 0 && idx < SEC_SIZE);
        let vpl = self.vpl();
        let word = idx / vpl;
        let off = (idx % vpl) * self.bits;
        debug_assert!(off + self.bits <= 64 && word < self.data.len());
        let mask = (1u64 << self.bits) - 1;
        (self.data[word] >> off) & mask
    }

    /// 打包写（对照 SimpleBitStorage.java:285-292）。
    #[inline]
    fn packed_set(&mut self, idx: usize, id: u64) {
        debug_assert!(self.bits > 0 && idx < SEC_SIZE);
        let vpl = self.vpl();
        let word = idx / vpl;
        let off = (idx % vpl) * self.bits;
        debug_assert!(off + self.bits <= 64 && word < self.data.len());
        let mask = (1u64 << self.bits) - 1;
        debug_assert!(id <= mask);
        self.data[word] = (self.data[word] & !(mask << off)) | ((id & mask) << off);
    }

    fn new_data(bits: usize) -> Vec<u64> {
        if bits == 0 {
            return Vec::new();
        }
        let vpl = 64 / bits;
        let len = SEC_SIZE.div_ceil(vpl);
        // 末字余量断言：最后一组残缺 entry 的位不得溢出 long（原版靠
        // 整除截断自然成立，此处显式锁死以防位宽常量演化破坏前提）。
        debug_assert!((SEC_SIZE % vpl) * bits <= 64);
        vec![0u64; len]
    }

    fn value_of(&self, id: u64) -> u16 {
        match &self.pal {
            Pal::Single(v) => {
                debug_assert_eq!(id, 0);
                *v
            }
            Pal::Linear(v) => v[id as usize],
            Pal::Hash { vals, .. } => vals[id as usize],
            Pal::Global => id as u16,
        }
    }

    /// Strategy.getConfigurationForBitCount（Strategy.java:36-46）：
    /// 0=单值、1..4→linear(4bit)、5..8→hashmap、≥9→global。
    /// 返回 (存储位宽, 新 palette)；档位请求只来自 ceillog2 阶梯，
    /// 故 2..=3 实际不可达（合并入 linear 分支即可，与原版同形）。
    fn configuration_for(bits: usize) -> (usize, Pal) {
        match bits {
            0 => (0, Pal::Single(0)),
            1..=4 => (
                LINEAR_BITS,
                Pal::Linear(Vec::with_capacity(1 << LINEAR_BITS)),
            ),
            5..=8 => (
                bits,
                Pal::Hash {
                    vals: Vec::with_capacity(1 << bits),
                    index: HashMap::new(),
                },
            ),
            _ => (GLOBAL_BITS, Pal::Global),
        }
    }

    /// PalettedContainer.onResize（:94-101）+ Data.copyFrom（:312-319）：
    /// 建 `target_bits` 新档，按旧存储扫描序把值灌入新 palette
    /// （插入序 = 首现序），最后把 `last_added` 追加进新 palette 并
    /// 返回其 id。只升不降。
    fn on_resize(&mut self, target_bits: usize, last_added: u16) -> u64 {
        let old_bits = self.bits;
        let old_data = std::mem::take(&mut self.data);
        let old_pal = std::mem::replace(&mut self.pal, Pal::Single(0));
        let (new_bits, new_pal) = Self::configuration_for(target_bits);
        let mut sec = VoxelSection {
            bits: new_bits,
            data: Self::new_data(new_bits),
            pal: new_pal,
        };
        // copyFrom（PalettedContainer.java:312-319 同构）：按旧存储
        // 扫描序把值灌入新 palette。旧档有表时用「旧 id → 新 id」
        // 直址表去重（≤256 项）；新档是 Global 时无表可建（新 id =
        // 值本身，u16 全值域塞不进 i16 直址表），逐格透传即可。
        let old_len = match &old_pal {
            Pal::Single(_) => 1,
            Pal::Linear(v) => v.len(),
            Pal::Hash { vals, .. } => vals.len(),
            Pal::Global => 0,
        };
        debug_assert_ne!(old_len, 0, "Global 是顶档，不可能作为重建的源档");
        let to_global = new_bits == GLOBAL_BITS;
        let mut remap: Vec<i16> = vec![-1; old_len.max(1)];
        for idx in 0..SEC_SIZE {
            let id = if old_bits == 0 {
                0
            } else {
                Self::packed_get_ref(old_bits, &old_data, idx)
            };
            let mapped = if to_global {
                debug_assert!(old_len > 0);
                Self::value_of_ref(&old_pal, id) as u64
            } else {
                let slot = &mut remap[id as usize];
                if *slot < 0 {
                    let v = Self::value_of_ref(&old_pal, id);
                    *slot = sec.push_palette_value(v) as i16;
                }
                *slot as u64
            };
            sec.packed_set(idx, mapped);
        }
        let id = sec.id_for(last_added);
        *self = sec;
        id
    }

    fn packed_get_ref(bits: usize, data: &[u64], idx: usize) -> u64 {
        let vpl = 64 / bits;
        let mask = (1u64 << bits) - 1;
        (data[idx / vpl] >> ((idx % vpl) * bits)) & mask
    }

    fn value_of_ref(pal: &Pal, id: u64) -> u16 {
        match pal {
            Pal::Single(v) => {
                debug_assert_eq!(id, 0);
                *v
            }
            Pal::Linear(v) => v[id as usize],
            Pal::Hash { vals, .. } => vals[id as usize],
            Pal::Global => id as u16,
        }
    }

    /// 把值按插入序放进当前（新建）palette，返回 id。仅 on_resize
    /// 内部使用（当前档必然未满：新档容量 ≥ 旧档项数）。
    fn push_palette_value(&mut self, v: u16) -> usize {
        match &mut self.pal {
            Pal::Single(_) => unreachable!("resize 后新档非 Single"),
            Pal::Linear(vals) => {
                debug_assert!(vals.len() < (1 << LINEAR_BITS));
                vals.push(v);
                vals.len() - 1
            }
            Pal::Hash { vals, index } => {
                debug_assert!(vals.len() < (1 << self.bits));
                index.insert(v, (vals.len()) as u8);
                vals.push(v);
                vals.len() - 1
            }
            Pal::Global => v as usize,
        }
    }

    /// 取值的 palette id，必要时先换档（PalettedContainer.getAndSet
    /// :117-121 的 idFor 侧）。返回写入存储用的 id。
    fn id_for(&mut self, v: u16) -> u64 {
        // 先在当前档内无副作用/就地追加地解出 id；解不出（满档）时
        // 记下 onResize 目标位宽，出借用区后再调 on_resize。
        let bits = self.bits;
        let need = match &mut self.pal {
            Pal::Single(u) => {
                if *u == v {
                    return 0;
                }
                // 原版 SingleValuePalette.onResize(ceillog2(2)=1)
                // → FOUR_BITS_LINEAR。
                ceillog2(2)
            }
            Pal::Linear(vals) => {
                if let Some(pos) = vals.iter().position(|&x| x == v) {
                    return pos as u64;
                }
                if vals.len() < (1 << LINEAR_BITS) {
                    vals.push(v);
                    return (vals.len() - 1) as u64;
                }
                ceillog2(vals.len() + 1) // 16 → 5
            }
            Pal::Hash { vals, index } => {
                let cap = 1usize << bits;
                if let Some(&id) = index.get(&v) {
                    return id as u64;
                }
                if vals.len() < cap {
                    let id = vals.len() as u8;
                    index.insert(v, id);
                    vals.push(v);
                    return id as u64;
                }
                ceillog2(vals.len() + 1) // 256 → 9 → Global
            }
            Pal::Global => return v as u64,
        };
        self.on_resize(need, v)
    }

    /// 读 section 内索引（vanilla `getIndex` 同构：`(yl<<8)|(z<<4)|x`）。
    #[inline]
    pub fn get(&self, local: usize) -> u16 {
        debug_assert!(local < SEC_SIZE);
        if self.bits == 0 {
            return self.value_of(0);
        }
        let id = self.packed_get(local);
        self.value_of(id)
    }

    /// 写 section 内索引：idFor（可换档）+ 打包写。
    #[inline]
    pub fn set(&mut self, local: usize, v: u16) {
        debug_assert!(local < SEC_SIZE);
        if self.bits == 0 {
            if let Pal::Single(u) = &self.pal
                && *u == v
            {
                return;
            }
            // 换档后 copyFrom 已灌满旧值，这里写新 id 即可。
            let id = self.id_for(v);
            self.packed_set(local, id);
            return;
        }
        let id = self.id_for(v);
        self.packed_set(local, id);
    }

    /// 展开为 4096 个 u16（打包扫描 + 小直址翻译表，O(1)/体素，
    /// 无逐体素 palette 查找——热路径口径见派单约束 4）。
    fn unpack_into(&self, out: &mut [u16]) {
        debug_assert_eq!(out.len(), SEC_SIZE);
        if self.bits == 0 {
            out.fill(self.value_of(0));
            return;
        }
        // 直址翻译表：id 上界 = min(2^bits, palette 项数)；global 档
        // 位宽 16 时 id = 值本身，直接透传。
        if self.bits == GLOBAL_BITS {
            let vpl = self.vpl();
            let mut i = 0;
            for &word in &self.data {
                let mut w = word;
                for _ in 0..vpl {
                    if i >= SEC_SIZE {
                        break;
                    }
                    out[i] = w as u16;
                    w >>= GLOBAL_BITS;
                    i += 1;
                }
            }
            debug_assert_eq!(i, SEC_SIZE);
            return;
        }
        let mut xlat = [0u16; 256];
        match &self.pal {
            Pal::Linear(vals) => {
                for (i, &v) in vals.iter().enumerate() {
                    xlat[i] = v;
                }
            }
            Pal::Hash { vals, .. } => {
                for (i, &v) in vals.iter().enumerate() {
                    xlat[i] = v;
                }
            }
            Pal::Single(v) => xlat[0] = *v,
            Pal::Global => unreachable!("global 档走上面分支"),
        }
        let vpl = self.vpl();
        let mask = (1u64 << self.bits) - 1;
        let mut i = 0;
        for &word in &self.data {
            let mut w = word;
            for _ in 0..vpl {
                if i >= SEC_SIZE {
                    break;
                }
                out[i] = xlat[(w & mask) as usize];
                w >>= self.bits;
                i += 1;
            }
        }
        debug_assert_eq!(i, SEC_SIZE);
    }

    /// 堆驻留估算（预算门口径）：打包数据 + palette 表；`HashMap`
    /// 按容量 × 元素大小估（内部元数据不计，登记为口径偏差）。
    fn resident_bytes(&self) -> usize {
        let data = self.data.capacity() * std::mem::size_of::<u64>();
        let pal = match &self.pal {
            Pal::Single(_) | Pal::Global => 0,
            Pal::Linear(v) => v.capacity() * std::mem::size_of::<u16>(),
            Pal::Hash { vals, index } => {
                vals.capacity() * std::mem::size_of::<u16>()
                    + index.capacity() * (std::mem::size_of::<u16>() + std::mem::size_of::<u8>())
            }
        };
        data + pal
    }
}

/// 一柱体素（24 × 16³ section，palette 压缩驻留）。
///
/// 磁盘/调度器接口仍是平铺 u16（E7a 不动盘格式与快照 payload）：
/// [`Self::from_flat_u16`] 装载、[`Self::decode_column`] 展开。
pub struct VoxelStore {
    sections: Box<[VoxelSection; SECTION_COUNT]>,
}

impl Clone for VoxelStore {
    fn clone(&self) -> Self {
        Self {
            sections: self.sections.clone(),
        }
    }
}

impl std::fmt::Debug for VoxelStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoxelStore")
            .field(
                "sections",
                &self.sections.iter().map(|s| s.bits()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl VoxelStore {
    /// 全空气柱（24 个 0bit 单值 section）。
    pub fn new() -> Self {
        Self {
            sections: Box::new(std::array::from_fn(|_| VoxelSection::new_uniform(0))),
        }
    }

    /// 从平铺 u16（`crate::vidx` 布局，长 [`CHUNK_VOL`]）装载：
    /// 逐 section「首值起单值档、扫描遇新值换档」直建最小档
    /// （偏差登记见模块注释）。
    ///
    /// # Panics
    /// `flat.len() != CHUNK_VOL`。
    pub fn from_flat_u16(flat: &[u16]) -> Self {
        assert_eq!(
            flat.len(),
            CHUNK_VOL,
            "flat voxel array must be CHUNK_VOL long"
        );
        let mut store = Self::new();
        for sy in 0..SECTION_COUNT {
            let base = sy * SEC_SIZE;
            let sec = &mut store.sections[sy];
            *sec = VoxelSection::new_uniform(flat[base]);
            for local in 1..SEC_SIZE {
                let v = flat[base + local];
                // 与当前单值相同则免表操作。
                if let Pal::Single(u) = &sec.pal
                    && *u == v
                {
                    continue;
                }
                // Single 档等值路径已被上面分支排除——走到这里
                // id_for 要么直接给 id（非 Single 档），要么已换档。
                let id = sec.id_for(v);
                sec.packed_set(local, id);
            }
        }
        store
    }

    /// 展开整柱为平铺 u16（`crate::vidx` 布局；save/快照/heightmap
    /// 重算等整段消费方使用，勿逐体素解码）。
    pub fn decode_column(&self) -> Vec<u16> {
        let mut out = vec![0u16; CHUNK_VOL];
        for sy in 0..SECTION_COUNT {
            self.sections[sy].unpack_into(&mut out[sy * SEC_SIZE..(sy + 1) * SEC_SIZE]);
        }
        out
    }

    /// 第 `sy` 个 section 只读视图（[`crate::SECTION_COUNT`] 界断言）。
    ///
    /// # Panics
    /// `sy >= SECTION_COUNT`。
    pub fn section(&self, sy: usize) -> &VoxelSection {
        &self.sections[sy]
    }

    /// 读（x/z 区块内 0..16，`y_abs` 绝对世界 y）。越界行为同
    /// [`crate::vidx`]（debug_assert + 界外 section 索引 panic）。
    #[inline]
    pub fn get(&self, x: usize, y_abs: i32, z: usize) -> BlockId {
        BlockId(self.get_u16(x, y_abs, z))
    }

    #[inline]
    fn get_u16(&self, x: usize, y_abs: i32, z: usize) -> u16 {
        debug_assert!(x < 16 && z < 16 && y_abs >= WORLD_MIN_Y);
        let yl = (y_abs - WORLD_MIN_Y) as usize;
        // 界外 y：sy 越界 → 数组索引 panic（coords 审计 P2 同则）。
        self.sections[yl >> 4].get(((yl & 15) << 8) | (z << 4) | x)
    }

    /// 写（坐标域同 [`Self::get`]）。
    #[inline]
    pub fn set(&mut self, x: usize, y_abs: i32, z: usize, id: BlockId) {
        debug_assert!(x < 16 && z < 16 && y_abs >= WORLD_MIN_Y);
        let yl = (y_abs - WORLD_MIN_Y) as usize;
        let local = ((yl & 15) << 8) | (z << 4) | x;
        // 界外 y → 索引 panic（同上）。
        if yl >> 4 >= SECTION_COUNT {
            panic!("y_abs {y_abs} outside world vertical range");
        }
        self.sections[yl >> 4].set(local, id.0);
    }

    /// 平铺 `crate::vidx` 索引读（机械替换 `read()[flat]` 用）。
    ///
    /// # Panics
    /// `flat >= CHUNK_VOL`。
    #[inline]
    pub fn get_flat(&self, flat: usize) -> BlockId {
        let sy = flat >> 12;
        BlockId(self.sections[sy].get(flat & (SEC_SIZE - 1)))
    }

    /// 平铺 `crate::vidx` 索引写。
    ///
    /// # Panics
    /// `flat >= CHUNK_VOL`。
    #[inline]
    pub fn set_flat(&mut self, flat: usize, id: BlockId) {
        let sy = flat >> 12;
        self.sections[sy].set(flat & (SEC_SIZE - 1), id.0);
    }

    /// 按平铺 `crate::vidx` 布局惰性遍历全柱（旧平铺数组
    /// `.iter()` 的等价替换；计数/比较等整段消费方使用）。
    pub fn iter(&self) -> impl Iterator<Item = BlockId> + '_ {
        (0..CHUNK_VOL).map(move |i| self.get_flat(i))
    }

    /// 堆驻留估算总和（预算门口径，见
    /// [`VoxelSection::resident_bytes`]）。
    pub fn resident_bytes(&self) -> usize {
        self.sections.iter().map(VoxelSection::resident_bytes).sum()
    }
}

impl Default for VoxelStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceillog2_matches_mth() {
        // Mth.ceillog2：Strategy.java:88-90（ceillog2(1)=0、(2)=1、
        // (3)=2、(256)=8、(257)=9）。
        assert_eq!(ceillog2(0), 0);
        assert_eq!(ceillog2(1), 0);
        assert_eq!(ceillog2(2), 1);
        assert_eq!(ceillog2(3), 2);
        assert_eq!(ceillog2(16), 4);
        assert_eq!(ceillog2(17), 5);
        assert_eq!(ceillog2(256), 8);
        assert_eq!(ceillog2(257), 9);
    }

    #[test]
    fn packed_roundtrip_all_tiers() {
        for bits in [4usize, 5, 6, 7, 8, 16] {
            let (new_bits, pal) = VoxelSection::configuration_for(bits);
            assert_eq!(new_bits, if bits <= 4 { LINEAR_BITS } else { bits });
            let mut sec = VoxelSection {
                bits: new_bits,
                data: VoxelSection::new_data(new_bits),
                pal,
            };
            let mask = (1u64 << new_bits) - 1;
            // 写入满掩码值再逐位读回（含打包边界：跨 long 项）。
            let mut seed = 7u32;
            for i in 0..SEC_SIZE {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (seed >> 8) as u64 & mask;
                sec.packed_set(i, v);
            }
            let mut seed = 7u32;
            for i in 0..SEC_SIZE {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (seed >> 8) as u64 & mask;
                assert_eq!(sec.packed_get(i), v, "bits={bits} idx={i}");
            }
        }
    }

    #[test]
    fn resize_preserves_values_and_order() {
        // 换档后旧值全保留、新档 palette 首现序 = 扫描序。
        let mut sec = VoxelSection::new_uniform(9);
        sec.set(0, 5); // 换档到 linear 4bit
        sec.set(10, 7);
        assert_eq!(sec.bits(), 4);
        sec.set(11, 5);
        assert_eq!(sec.get(0), 9);
        assert_eq!(sec.get(10), 7);
        assert_eq!(sec.get(11), 5);
        // 16 值满档 → 第 17 值进 hashmap 5bit，旧值不丢。
        for i in 0..15u16 {
            sec.set(100 + i as usize, 100 + i);
        }
        assert_eq!(sec.bits(), 4);
        sec.set(200, 120);
        assert_eq!(sec.bits(), 5);
        assert_eq!(sec.get(0), 9);
        assert_eq!(sec.get(100), 100);
        assert_eq!(sec.get(200), 120);
    }
}
