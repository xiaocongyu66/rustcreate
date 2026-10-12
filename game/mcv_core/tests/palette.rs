//! E7a 红基线：内存体素 palette 压缩（`mcv_core::VoxelStore`）行为锁。
//!
//! 原版机制对齐（先读后写，file:line 为 src-26.1 只读快照）：
//! - PalettedContainer.java:81-92 `createOrReuseData`——bits=0 走
//!   ZeroBitStorage，否则 SimpleBitStorage(bits, 4096)；档位由
//!   Strategy 派生。:94-101 `onResize`——按 ceillog2(新 palette 尺寸)
//!   建新档、逐 entry copyFrom 重编码、只升不降。
//! - Strategy.java:33-48 方块档位表：0=单值；1-4=linear（内存一律
//!   4bit，FOUR_BITS_LINEAR，:14）；5-8=hashmap；>8=global 回退
//!   （:44 `Configuration.Global`）。
//! - SimpleBitStorage.java:240-264 LSB-first 打包（valuesPerLong =
//!   64/bits）；LevelChunkSection.java:13-15,21 按 16³=4096 section
//!   持容器。
//! 简化登记（债）：global 档内存位宽取 16（值域 = 全域 u16，含状态
//! nibble；原版 globalBits = ceillog2(IdMap 大小)）；linear 档在
//! 2..=16 值时同样占 4bit 存储（与原版一致，非简化）。

use mcv_core::{BlockId, CHUNK_VOL, WORLD_MAX_Y, WORLD_MIN_Y, vidx};

/// 第 k 个不同值落位后期望的 (bits, palette_len)，逐档对照
/// Strategy.java:37-45 的 switch + PalettedContainer.onResize 的
/// ceillog2 触发（16→5、32→6、64→7、128→8、256→global）。
fn expect_ladder(distinct: usize) -> (usize, usize) {
    match distinct {
        0..=1 => (0, distinct.max(1)),
        2..=16 => (4, distinct),
        17..=32 => (5, distinct),
        33..=64 => (6, distinct),
        65..=128 => (7, distinct),
        129..=256 => (8, distinct),
        n => (16, n),
    }
}

/// 在 section 0（y ∈ [WORLD_MIN_Y, WORLD_MIN_Y+16)）内逐个填入
/// `count` 个不同值（每值填满整 section 的一部分槽位，触发换档）。
fn fill_distinct(store: &mut mcv_core::VoxelStore, count: usize) {
    let mut i = 0usize;
    for v in 0..count {
        // 每值至少占一个槽位即可触发 palette 增长。
        let x = i % 16;
        let yl = (i / 16) % 16;
        let z = (i / 256) % 16;
        store.set(x, WORLD_MIN_Y + yl as i32, z, BlockId(v as u16 + 1));
        i += 1;
    }
}

#[test]
fn empty_store_is_zero_bit_single_value() {
    // 原版 PalettedContainer.java:75-79：初始 bits=0 单值表。
    let store = mcv_core::VoxelStore::new();
    for sy in 0..mcv_core::SECTION_COUNT {
        let s = store.section(sy);
        assert_eq!((s.bits(), s.palette_len()), (0, 1), "sy={sy} 全空柱必须 0bit");
        assert!(s.is_uniform());
        assert_eq!(s.uniform_value(), Some(0));
    }
    // 0bit 生效：全空气柱驻留是常数级（数据体为 0，只剩结构开销）。
    assert!(
        store.resident_bytes() <= 1024,
        "全空气柱驻留 {} B 超常数预算",
        store.resident_bytes()
    );
    assert_eq!(store.get(0, WORLD_MIN_Y, 0), BlockId(0));
    assert_eq!(store.decode_column().len(), CHUNK_VOL);
}

#[test]
fn ladder_matches_vanilla_strategy_table() {
    // 1..=16 值线性档一律 4bit（Strategy.java:39 FOUR_BITS_LINEAR，
    // 2 值 section 也占 4bit——原版语义，非本仓近似）；换档只在
    // 17/33/65/129/257 个不同值处发生（onResize ceillog2）。
    for n in [2usize, 3, 5, 16, 17, 32, 33, 64, 65, 128, 129, 256, 257, 300] {
        let mut store = mcv_core::VoxelStore::new();
        fill_distinct(&mut store, n);
        let s = store.section(0);
        let (bits, pal) = expect_ladder(n);
        assert_eq!(s.bits(), bits, "{n} 个不同值应落 {bits}bit 档");
        assert_eq!(s.palette_len(), pal, "{n} 值 palette 长度");
    }
}

#[test]
fn resize_never_shrinks() {
    // PalettedContainer 只升不降：256 值后全挖回空气，档位停在
    // 8bit（palette 保留），不做 compact。
    let mut store = mcv_core::VoxelStore::new();
    fill_distinct(&mut store, 200);
    assert_eq!(store.section(0).bits(), 8);
    for yl in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                store.set(x, WORLD_MIN_Y + yl, z, BlockId(0));
            }
        }
    }
    assert_eq!(store.section(0).bits(), 8, "回挖不得降档（对齐原版无缩档）");
    assert_eq!(store.get(0, WORLD_MIN_Y, 0), BlockId(0), "值仍须正确");
}

#[test]
fn flat_roundtrip_is_identity() {
    // 盘格式不动的前提：from_flat_u16 ∘ decode_column = id，含
    // >256 值/section（global 16bit 档）与全 u16 值域样本。
    let mut flat = vec![0u16; CHUNK_VOL];
    let mut x: u32 = 0x9E37_79B9;
    for (i, v) in flat.iter_mut().enumerate() {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        // 混合档：section0 全随机（>256 值），其余 section 低值域。
        let sec = i / mcv_core::SECTION_VOL;
        *v = if sec == 0 { (x >> 16) as u16 } else { ((x >> 20) % 12) as u16 };
    }
    let store = mcv_core::VoxelStore::from_flat_u16(&flat);
    assert_eq!(store.decode_column(), flat);
    // get_flat/get 与平铺 vidx 逐位一致。
    for _ in 0..2000 {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let xi = (x >> 27) as usize % 16;
        let zi = (x >> 20) as usize % 16;
        let yi = (x >> 8) % (mcv_core::CHUNK_SY as u32);
        let y = WORLD_MIN_Y + yi as i32;
        assert_eq!(store.get(xi, y, zi), store.get_flat(vidx(xi, y, zi)));
    }
}

#[test]
fn set_get_matches_flat_shadow_model() {
    // 随机 set 序列下与平铺影子数组逐位一致（含跨 section 换档）。
    let mut store = mcv_core::VoxelStore::new();
    let mut shadow = vec![0u16; CHUNK_VOL];
    let mut x: u32 = 0xDEAD_BEEF;
    for step in 0..20_000u32 {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let xi = (x >> 27) as usize % 16;
        let zi = (x >> 20) as usize % 16;
        let yi = (x >> 8) % (mcv_core::CHUNK_SY as u32);
        let y = WORLD_MIN_Y + yi as i32;
        // 值域逐步扩宽：早期小值域（linear），后期大值域（hashmap）。
        let wide = step > 10_000;
        let id = if wide { ((x >> 12) % 600) as u16 + 1 } else { ((x >> 12) % 20) as u16 + 1 };
        store.set(xi, y, zi, BlockId(id));
        shadow[vidx(xi, y, zi)] = id;
    }
    assert_eq!(store.decode_column(), shadow);
}

#[test]
fn build_is_deterministic() {
    // 同输入两次构建：逐 section 档位/palette 长度/解码字节全等
    // （hashmap 档查找用 HashMap、但 palette 插入序 = 扫描序，
    // 对齐原版 ObjectList 插入序语义，见模块注释与债登记）。
    let mut flat = vec![0u16; CHUNK_VOL];
    let mut x: u64 = 12345;
    for v in flat.iter_mut() {
        x = x
            .wrapping_mul(63_641_362_238_467_930_05)
            .wrapping_add(1_442_695_040_888_963_407);
        *v = ((x >> 16) % 300) as u16;
    }
    let a = mcv_core::VoxelStore::from_flat_u16(&flat);
    let b = mcv_core::VoxelStore::from_flat_u16(&flat);
    assert_eq!(a.decode_column(), b.decode_column());
    assert_eq!(a.resident_bytes(), b.resident_bytes());
    for sy in 0..mcv_core::SECTION_COUNT {
        assert_eq!(a.section(sy).bits(), b.section(sy).bits());
        assert_eq!(a.section(sy).palette_len(), b.section(sy).palette_len());
    }
}

#[test]
fn typical_generated_column_fits_32kib() {
    // 预算断言的本体口径：合成典型地形柱（半柱 3 值交界 + 半柱空气
    // + 一条水线）驻留 ≤ 32 KiB；真实生成柱的门在 memory_budget.rs。
    let mut flat = vec![0u16; CHUNK_VOL];
    for yl in 0..mcv_core::CHUNK_SY {
        for z in 0..16 {
            for x in 0..16 {
                let i = (yl << 8) | (z << 4) | x;
                flat[i] = if yl < 128 { 0 } else if yl < 190 { 1 } else { 3 };
            }
        }
    }
    let store = mcv_core::VoxelStore::from_flat_u16(&flat);
    assert!(
        store.resident_bytes() <= 32 * 1024,
        "典型柱驻留 {} B 超 32 KiB",
        store.resident_bytes()
    );
}

#[test]
#[should_panic]
fn section_index_out_of_range_panics() {
    let store = mcv_core::VoxelStore::new();
    let _ = store.section(mcv_core::SECTION_COUNT);
}

#[test]
#[should_panic]
fn get_out_of_world_y_panics() {
    let store = mcv_core::VoxelStore::new();
    let _ = store.get(0, WORLD_MAX_Y, 0);
}

#[test]
#[should_panic]
fn from_flat_wrong_len_panics() {
    let _ = mcv_core::VoxelStore::from_flat_u16(&[0u16; 10]);
}
