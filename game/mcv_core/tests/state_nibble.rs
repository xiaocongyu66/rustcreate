//! 状态 nibble（体素 bit12-15）编解码回归锁。

use mcv_core::{BlockId, ID_MASK, Shape};

#[test]
fn with_state_id_roundtrip() {
    for id in [1u16, 5, 900, 1170] {
        for st in 0..=15u8 {
            let b = BlockId(id).with_state(st);
            assert_eq!(b.id(), id, "id {id} 状态 {st} 掩码回原 id");
            assert_eq!(b.state(), st, "id {id} 状态 {st} 提取一致");
            assert_eq!(b.def().name, mcv_core::BLOCKS[id as usize].name);
        }
    }
    // with_state 丢弃入参高位（只取低 4 位）
    assert_eq!(BlockId(26).with_state(0xF0).state(), 0);
    assert_eq!(BlockId(26).with_state(1).with_state(0).state(), 0);
}

#[test]
fn barrier_sentinel_survives_masking() {
    // C++ 侧 kBarrier=0xFFFF：掩码后是 0x0FFF（未注册 id，非任何真实方块），
    // 状态位读出 15。Rust 体素数组永不存 kBarrier（WorldView 对未加载区块
    // 返回 BlockId(1)），此处仅锁定掩码算术不产生意外别名。
    let k = BlockId(0xFFFF);
    assert_eq!(k.id(), 0x0FFF);
    assert_ne!(k.id(), BlockId(1170).id());
    assert_eq!(k.state(), 15);
    assert_eq!(ID_MASK, 0x0FFF);
}

#[test]
fn state_shapes_read_side() {
    // acacia_slab id=26：上半砖状态不影响形状/id 读取。
    let slab = BlockId(26).with_state(1);
    assert_eq!(mcv_core::shape::shape(slab.0), Shape::Slab);
    assert_eq!(slab.def().name, "acacia_slab");
    // 状态体素作为 u16 索引 BLOCKS 前必须掩码：id() 是唯一正确入口。
    assert!(slab.0 as usize >= mcv_core::BLOCKS.len());
}
