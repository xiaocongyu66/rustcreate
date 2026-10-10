//! 黄金对拍的**唯一输入常量源**（共享常量源纪律）：seed 表、区块网格、
//! 全量落盘区块清单都在本文件定义；tests/golden.rs 是唯一消费者，未来
//! 若需重建黄金数据（本机驱动），也必须以本文件为输入规格源本，杜绝
//! 「驱动对的是 A 输入、测试跑的是 B 输入」的错配。

/// 黄金覆盖集的 4 个 seed：0 与 u64::MAX 是异或/回绕通路的两端边界。
pub const SEEDS: [u64; 4] = [0, 42, 0xDEAD_BEEF_CAFE_F00D, u64::MAX];

/// 每 seed 的网格偏移：保证负坐标区块进入 x/z 两轴。
pub const OFFSETS: [(i32, i32); 4] = [(0, 0), (-7, -3), (9, -12), (-15, 8)];

/// 网格半径：每 seed 覆盖 5×5 = 25 个区块。
pub const GRID: i32 = 2;

/// 全量二进制落盘的 4 个代表区块（seed 位型两端 + 负坐标）。
/// 文件名按 `terrain_s{seed:016x}_c{cx}_{cz}.bin` 拼接。
pub const DUMPS: [(u64, i32, i32); 4] = [
    (42, 0, 0),
    (0, -7, -3),
    (u64::MAX, 9, -12),
    (0xDEAD_BEEF_CAFE_F00D, -15, 8),
];

/// FNV-1a 64（黄金 TSV/全量文件的哈希列与本测试共用同一实现）。
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

/// 体素 65536×u16 的 LE 字节流（黄金哈希/落盘的字节口径）。
pub fn voxels_le(voxels: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(voxels.len() * 2);
    for v in voxels {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}
