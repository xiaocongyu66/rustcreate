//! 黄金对拍的**唯一输入常量源**（共享常量源纪律）：场景构造、随机批
//! splitmix64、用例名与顺序、kind 集合都在本文件定义。tests/golden.rs 是
//! 唯一消费者；未来若需重建黄金数据（本机 C++ 驱动已随 cpp/ 删除），也
//! 必须以本文件为输入规格源本，杜绝「驱动对的是 A 输入、测试跑的是 B
//! 输入」的错配。

/// 黄金场景顺序（= mesh_scenes/*.bin 文件名；每个场景跑 opaque+water 两
/// pass，即 `<name>_k0.bin` / `<name>_k1.bin`）。
pub const SCENES: [(&str, bool); 7] = [
    ("plains", true), // 全 9 slot 加载
    ("cave_full9", true),
    ("cave_only_center", false), // 仅中心加载（8 邻 None = 不透明边界）
    ("water", true),
    ("noncube_mix", true),
    ("bounds", true),
    ("light_gradient", true),
];

/// 随机批：64 区块 × 2 pass = 128 行（mesh_random_batch.tsv 顺序与此一致）。
pub const RANDOM_BATCH_CHUNKS: usize = 64;

pub const VOL: usize = 65536;

pub type Chunk = (Box<[u16; VOL]>, Box<[u8; VOL]>);

/// FNV-1a 64（黄金 TSV 哈希列与本测试共用同一实现）。
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

pub fn chunk(id: u16, light: u8) -> Chunk {
    (Box::new([id; VOL]), Box::new([light; VOL]))
}

pub fn put(vox: &mut [u16; VOL], x: usize, y: usize, z: usize, id: u16) {
    vox[(y << 8) | (z << 4) | x] = id;
}

/// 按注册名查方块 id（不硬编码生成表 id）。
pub fn id_of(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .expect("unknown block name") as u16
}

/// All 9 slots loaded: center chunk plus 8 copies of `side` (usually air).
pub fn full9<'a>(center: &'a Chunk, side: &'a Chunk) -> [Option<crate::Slot<'a>>; 9] {
    let mut out = [Some(crate::Slot {
        voxels: &side.0[..],
        light: &side.1[..],
    }); 9];
    out[4] = Some(crate::Slot {
        voxels: &center.0[..],
        light: &center.1[..],
    });
    out
}

/// Only the center slot loaded; all 8 neighbours are `None` (opaque
/// boundary for the mesher).
pub fn only_center<'a>(center: &'a Chunk) -> [Option<crate::Slot<'a>>; 9] {
    let mut out: [Option<crate::Slot<'a>>; 9] = [None; 9];
    out[4] = Some(crate::Slot {
        voxels: &center.0[..],
        light: &center.1[..],
    });
    out
}

// ---- 确定性场景 -----------------------------------------------------------

/// 平原：草顶/土/石层 + 花 + 水塘 + 原木柱，含跨 x/z 光照变化。
pub fn scene_plains() -> Chunk {
    let mut c = chunk(0, 0xF0);
    let (grass, dirt, stone, water, flower, log) = (
        id_of("grass"),
        id_of("dirt"),
        id_of("stone"),
        id_of("water"),
        id_of("flower_red"),
        id_of("log"),
    );
    for x in 0..16 {
        for z in 0..16 {
            let h = 8 + (x ^ z) % 3; // 8..10
            for y in 0..h {
                let id = if y == h - 1 {
                    grass
                } else if y + 3 >= h {
                    dirt
                } else {
                    stone
                };
                put(&mut c.0, x, y, z, id);
            }
            if (x + z) % 5 == 0 {
                put(&mut c.0, x, h, z, flower);
            }
        }
    }
    // 水塘：凹地灌水，水面顶波动 + 水下地面剔除都要覆盖。
    for x in 3..7 {
        for z in 3..7 {
            put(&mut c.0, x, 8, z, 0);
            put(&mut c.0, x, 9, z, water);
        }
    }
    for (x, z) in [(2, 2), (12, 13)] {
        for y in 8..12 {
            put(&mut c.0, x, y, z, log);
        }
    }
    c
}

/// 洞穴边界：实心石 + 空腔 + 火把/栅栏/半砖，空腔壁面与洞内光照交互。
pub fn scene_cave() -> Chunk {
    let mut c = chunk(0, 0xF0);
    let (stone, torch, fence, slab, cobble) = (
        id_of("stone"),
        id_of("torch"),
        id_of("oak_fence"),
        id_of("oak_slab"),
        id_of("cobble"),
    );
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..14 {
                put(&mut c.0, x, y, z, stone);
            }
        }
    }
    for x in 4..12 {
        for z in 4..12 {
            for y in 5..11 {
                put(&mut c.0, x, y, z, 0);
            }
        }
    }
    put(&mut c.0, 7, 5, 7, torch);
    put(&mut c.0, 5, 5, 5, fence);
    put(&mut c.0, 6, 5, 5, fence);
    put(&mut c.0, 8, 5, 8, slab);
    put(&mut c.0, 9, 5, 8, cobble);
    c
}

/// 水面：石盆 + 不同水位 + 叶下水面（wave=0 分支）+ 贴边水。
pub fn scene_water() -> Chunk {
    let mut c = chunk(0, 0xF0);
    let (stone, water, leaves) = (id_of("stone"), id_of("water"), id_of("leaves"));
    for x in 0..16 {
        for z in 0..16 {
            put(&mut c.0, x, 7, z, stone);
        }
    }
    for x in 2..10 {
        for z in 2..10 {
            let top = if (x + z) % 4 == 0 { 10 } else { 9 };
            for y in 8..top {
                put(&mut c.0, x, y, z, water);
            }
        }
    }
    // 叶下水面：+Y 邻格非空气 → wave=0。
    put(&mut c.0, 3, 9, 3, 0);
    put(&mut c.0, 3, 10, 3, leaves);
    // 贴区块边水（邻格走哨兵/空气两种语义）。
    for y in 8..10 {
        put(&mut c.0, 15, y, 0, water);
    }
    c
}

/// 非立方混合：半砖上下/叠放、楼梯全朝向+翻转、栅栏多向臂（贴石/贴火把/
/// 跨种）、花草、火把。全部状态 nibble 打包进体素值。
pub fn scene_noncube_mix() -> Chunk {
    let mut c = chunk(0, 0xF0);
    let (stone, slab, stairs, fence, spruce_fence, torch, flower, glass) = (
        id_of("stone"),
        id_of("oak_slab"),
        id_of("oak_stairs"),
        id_of("oak_fence"),
        id_of("spruce_fence"),
        id_of("torch"),
        id_of("flower_red"),
        id_of("glass"),
    );
    for x in 0..16 {
        for z in 0..16 {
            put(&mut c.0, x, 7, z, stone);
        }
    }
    // 半砖：下/上/叠放 + 中层面暴露。
    put(&mut c.0, 1, 8, 1, slab);
    put(&mut c.0, 2, 8, 1, slab | (1 << 12));
    put(&mut c.0, 3, 8, 1, slab);
    put(&mut c.0, 3, 9, 1, slab | (1 << 12));
    // 楼梯：facing 0..3 + top 翻转，各向贴邻。
    for f in 0..4u16 {
        put(&mut c.0, 5 + f as usize, 8, 3, stairs | (f << 12));
        put(&mut c.0, 5 + f as usize, 8, 4, stairs | ((f | 4) << 12));
    }
    // 栅栏：互连行 + 贴石臂 + 贴玻璃（opaque=false 整块）+ 跨种互连 +
    // 火把不连。
    for x in 1..4 {
        put(&mut c.0, x, 8, 8, fence);
    }
    put(&mut c.0, 4, 8, 8, stone);
    put(&mut c.0, 6, 8, 8, fence);
    put(&mut c.0, 7, 8, 8, spruce_fence);
    put(&mut c.0, 8, 8, 8, fence);
    put(&mut c.0, 9, 8, 8, torch);
    put(&mut c.0, 10, 8, 8, fence);
    put(&mut c.0, 11, 8, 8, glass);
    // 花草贴地 + 悬空火把（面剔到空气）。
    put(&mut c.0, 13, 8, 12, flower);
    put(&mut c.0, 14, 9, 13, torch);
    c
}

/// 越界/邻域缺失语义：y=0 实心层（-Y 走 y<0 哨兵）、y=255 顶柱（+Y 走
/// y>=256 空气、sky=15）、贴边方块对 None 邻（哨兵剔面）。
pub fn scene_bounds() -> Chunk {
    let mut c = chunk(0, 0xF0);
    let (stone, glass) = (id_of("stone"), id_of("glass"));
    for x in 0..16 {
        for z in 0..16 {
            put(&mut c.0, x, 0, z, stone);
        }
    }
    for y in 0..256 {
        put(&mut c.0, 8, y, 8, stone);
    }
    // 贴四边的透明方块：朝 None 邻的面被哨兵剔除，透明块自身也测。
    put(&mut c.0, 0, 5, 5, glass);
    put(&mut c.0, 15, 5, 5, glass);
    put(&mut c.0, 5, 5, 0, glass);
    put(&mut c.0, 5, 5, 15, glass);
    c
}

/// 光照异构：随 y 变化的天空/方块光 nibble，验证 light_at 高低 nibble 与
/// 邻格采样进入合并键。
pub fn scene_light_gradient() -> Chunk {
    let mut c = chunk(0, 0xF0);
    let (stone, glowstone, water) = (id_of("stone"), id_of("glowstone"), id_of("water"));
    for x in 0..16 {
        for z in 0..16 {
            put(&mut c.0, x, 6, z, stone);
            put(&mut c.0, x, 7, z, water);
        }
    }
    put(&mut c.0, 8, 8, 8, glowstone);
    for y in 0..16usize {
        for x in 0..16usize {
            for z in 0..16usize {
                let i = (y << 8) | (z << 4) | x;
                c.1[i] = ((((y * 13 + x * 7 + z * 3) % 16) << 4) | ((x + z) % 16)) as u8;
            }
        }
    }
    c
}

/// 场景名 → 区块构造（SCENES 顺序即黄金文件顺序）。
pub fn scene_by_name(name: &str) -> Chunk {
    match name {
        "plains" => scene_plains(),
        "cave_full9" | "cave_only_center" => scene_cave(),
        "water" => scene_water(),
        "noncube_mix" => scene_noncube_mix(),
        "bounds" => scene_bounds(),
        "light_gradient" => scene_light_gradient(),
        other => panic!("未知场景 {other}"),
    }
}

// ---- 随机批（splitmix64：确定性、无依赖）---------------------------------

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// 地形化随机区块（高度面 + 稀疏装饰 + 稀疏洞）。黄金行序：先 center/n1/
/// n2 三次构造，再按 slot 0..9 轮转（i==4 → center；否则 below(5)>0 时取
/// pool[below(3)]），最后每 pass 一行——顺序即黄金 TSV 行序，不可改动。
pub fn random_chunk(rng: &mut Rng) -> Chunk {
    let mut c = chunk(0, 0xF0);
    let solid = [
        id_of("stone"),
        id_of("dirt"),
        id_of("grass"),
        id_of("sand"),
        id_of("log"),
        id_of("leaves"),
        id_of("planks"),
    ];
    let deco = [
        id_of("oak_fence"),
        id_of("oak_slab"),
        id_of("oak_stairs"),
        id_of("flower_red"),
        id_of("torch"),
    ];
    for x in 0..16usize {
        for z in 0..16usize {
            let h = 40 + rng.below(24) as usize; // 40..63
            for y in 0..h {
                let id = solid[rng.below(solid.len() as u64) as usize];
                let raw = match mcv_core::shape::Shape::from_u8(mcv_core::BLOCKS[id as usize].shape)
                {
                    mcv_core::shape::Shape::Slab => id | ((rng.below(2) as u16) << 12),
                    mcv_core::shape::Shape::Stairs => id | ((rng.below(8) as u16) << 12),
                    _ => id,
                };
                c.0[(y << 8) | (z << 4) | x] = raw;
            }
            // 水面：洼地灌到 y=59（水顶波动 + 水下剔除 + 水间共面）。
            if h < 60 {
                for y in h..60 {
                    c.0[(y << 8) | (z << 4) | x] = id_of("water");
                }
            }
            // 稀疏装饰（约 1/8 列）：栅栏/半砖/楼梯/花草/火把，随机状态。
            if rng.below(8) == 0 && h < 255 {
                let id = deco[rng.below(deco.len() as u64) as usize];
                let raw = match mcv_core::shape::Shape::from_u8(mcv_core::BLOCKS[id as usize].shape)
                {
                    mcv_core::shape::Shape::Slab => id | ((rng.below(2) as u16) << 12),
                    mcv_core::shape::Shape::Stairs => id | ((rng.below(8) as u16) << 12),
                    _ => id,
                };
                c.0[(h << 8) | (z << 4) | x] = raw;
            }
            // 稀疏洞（约 1/16 列）：孤立空腔的六向剔除与 AO。
            if rng.below(16) == 0 && h > 6 {
                let y = rng.below((h - 2) as u64) as usize + 1;
                c.0[(y << 8) | (z << 4) | x] = 0;
            }
        }
    }
    // 光照：50% 经典 0xF0，否则随机 nibble 对（覆盖高低位与合并键）。
    for slot in c.1.iter_mut() {
        *slot = if rng.below(2) == 0 {
            0xF0
        } else {
            rng.below(256) as u8
        };
    }
    c
}

/// 随机批种子（与黄金数据落盘时的生成完全一致；改动即黄金重生成）。
pub const RANDOM_BATCH_SEED: u64 = 0x2026_1010_0C0B_0908;
