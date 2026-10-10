//! C++ oracle（FFI 后端）↔ 纯 Rust 网格器逐字节对拍（任务铁律：对拍不过
//! 修 Rust 侧，不许改 C++）。覆盖：确定性区块集（平原/洞穴边界/水面/非
//! 立方混合/越界语义）+ 64 区块随机 seed 批量 + geom 规则对 C++ 生成表的
//! 逐 id 锁 + 单区块性能护栏。

use mcv_mesher::{Backend, MESH_OPAQUE, MESH_WATER, Mesher, Slot};

const VOL: usize = 65536;

type Chunk = (Box<[u16; VOL]>, Box<[u8; VOL]>);

fn chunk(id: u16, light: u8) -> Chunk {
    (Box::new([id; VOL]), Box::new([light; VOL]))
}

fn put(vox: &mut [u16; VOL], x: usize, y: usize, z: usize, id: u16) {
    vox[(y << 8) | (z << 4) | x] = id;
}

/// 按注册名查方块 id（不硬编码生成表 id）。
fn id_of(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .expect("unknown block name") as u16
}

fn full9<'a>(center: &'a Chunk, side: &'a Chunk) -> [Option<Slot<'a>>; 9] {
    let mut out = [Some(Slot {
        voxels: &side.0[..],
        light: &side.1[..],
    }); 9];
    out[4] = Some(Slot {
        voxels: &center.0[..],
        light: &center.1[..],
    });
    out
}

/// 仅中心加载：8 邻 None（不透明边界语义）。
fn only_center<'a>(center: &'a Chunk) -> [Option<Slot<'a>>; 9] {
    let mut out: [Option<Slot<'a>>; 9] = [None; 9];
    out[4] = Some(Slot {
        voxels: &center.0[..],
        light: &center.1[..],
    });
    out
}

fn assert_bytes_eq(a: &[u8], b: &[u8], ctx: &str) {
    assert_eq!(
        a.len(),
        b.len(),
        "{ctx}: 顶点字节长度 {} vs {}",
        a.len(),
        b.len()
    );
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            x,
            y,
            "{ctx}: 首个差异在字节 {i}（顶点 {}，字段偏移 {}）",
            i / 24,
            i % 24
        );
    }
}

fn assert_indices_eq(a: &[u32], b: &[u32], ctx: &str) {
    assert_eq!(
        a.len(),
        b.len(),
        "{ctx}: 索引长度 {} vs {}",
        a.len(),
        b.len()
    );
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(x, y, "{ctx}: 首个差异在索引 {i}");
    }
}

/// 同输入跑两侧后端，逐字节断言顶点 + 索引，返回 (顶点数, 索引数) 供
/// 附加断言。
fn build_both(slots: &[Option<Slot<'_>>; 9], kind: u32, ctx: &str) -> (u32, u32) {
    let ffi = Mesher::with_backend(256 << 20, Backend::Ffi).unwrap();
    let rust = Mesher::with_backend(256 << 20, Backend::Rust).unwrap();
    let a = ffi.build(slots, kind).expect("ffi build");
    let b = rust.build(slots, kind).expect("rust build");
    assert_eq!(
        a.counts(),
        b.counts(),
        "{ctx} kind {kind}: 计数 ffi={:?} rust={:?}",
        a.counts(),
        b.counts()
    );
    assert_bytes_eq(
        a.vertex_data(),
        b.vertex_data(),
        &format!("{ctx} kind {kind}"),
    );
    assert_indices_eq(a.indices(), b.indices(), &format!("{ctx} kind {kind}"));
    b.counts()
}

/// 两 pass 都对拍一遍，返回末 pass（water）计数。
fn build_both_passes(slots: &[Option<Slot<'_>>; 9], ctx: &str) -> (u32, u32) {
    build_both(slots, MESH_OPAQUE, ctx);
    build_both(slots, MESH_WATER, ctx)
}

// ---------------------------------------------------------------------------
// 表锁：geom 推导规则逐 id 对照 C++ 生成表（kind 无法从名字推导的漂移护栏）
// ---------------------------------------------------------------------------

#[test]
fn rust_geom_rule_matches_cpp_generated_table() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../cpp/src/blocks_gen.inc");
    let text = std::fs::read_to_string(path).expect("C++ 生成表应随仓库存在");
    let mut count = 0usize;
    for (i, line) in text.lines().enumerate() {
        let Some((body, name)) = line.split_once("//") else {
            continue;
        };
        let body = body.trim();
        if !body.starts_with('{') {
            continue;
        }
        let compact: String = body.chars().filter(|c| *c != '{' && *c != '}').collect();
        let flat: Vec<&str> = compact.split(',').map(str::trim).collect();
        assert_eq!(flat.len(), 10, "line {}: 意外字段数 {:?}", i + 1, flat);
        let def = &mcv_core::BLOCKS[count];
        assert_eq!(name.trim(), def.name, "id {count}: 表名漂移");
        assert_eq!(flat[0] == "true", def.opaque, "id {count} opaque");
        assert_eq!(flat[1] == "true", def.liquid, "id {count} liquid");
        let tiles: [u16; 6] = flat[3..9]
            .iter()
            .map(|t| t.parse::<u16>().expect("tile 层"))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        assert_eq!(tiles, def.tiles, "id {count} tiles");
        let shape: u8 = flat[9].parse().expect("shape");
        assert_eq!(shape, def.shape, "id {count} shape");
        assert!(
            (flat[2] == "true") == mcv_mesher::block_geom(def),
            "id {count} {}: geom 推导与 C++ 表漂移（C++ geom={}）",
            def.name,
            flat[2]
        );
        count += 1;
    }
    assert_eq!(count, mcv_core::BLOCKS.len(), "两侧表行数应一致");
}

// ---------------------------------------------------------------------------
// 确定性区块集
// ---------------------------------------------------------------------------

/// 平原：草顶/土/石层 + 花 + 水塘 + 原木柱，含跨 x/z 光照变化。
fn scene_plains() -> Chunk {
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
fn scene_cave() -> Chunk {
    let mut c = chunk(0, 0xF0);
    let (stone, torch, fence, slab, cobble) = (
        id_of("stone"),
        id_of("torch"),
        id_of("oak_fence"),
        id_of("oak_slab"),
        id_of("cobblestone"),
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
fn scene_water() -> Chunk {
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
fn scene_noncube_mix() -> Chunk {
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
    // 栅栏：互连行 + 贴石臂 + 贴玻璃（opaque=false 整块，C++ 近似不连）+
    // 跨种互连 + 火把不连。
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
fn scene_bounds() -> Chunk {
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
fn scene_light_gradient() -> Chunk {
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

#[test]
fn deterministic_scenes_byte_parity() {
    let air = chunk(0, 0xF0);
    let scenes: [(&str, Chunk, bool); 6] = [
        ("plains", scene_plains(), true),
        ("cave_full9", scene_cave(), true),
        ("cave_only_center", scene_cave(), false),
        ("water", scene_water(), true),
        ("noncube_mix", scene_noncube_mix(), true),
        ("bounds", scene_bounds(), true),
    ];
    for (name, center, loaded) in scenes {
        let (vc, ic) = if loaded {
            build_both_passes(&full9(&center, &air), name)
        } else {
            build_both_passes(&only_center(&center), name)
        };
        println!("{name}: opaque+water 全对拍通过（末 pass {vc} 顶点 / {ic} 索引）");
    }
    // 光照梯度场景单独跑（构造后修改了 light 数组）。
    let lg = scene_light_gradient();
    build_both_passes(&full9(&lg, &air), "light_gradient");
}

#[test]
fn bounds_semantics_shared_by_both_backends() {
    // 越界/邻域缺失语义的显式行为断言（任务 4）：C++ 与 Rust 对 y<0 / y>=256
    // / None 邻给出同一几何。scene_bounds 已对拍；这里补 Rust 侧的关键
    // 不变量，防两侧同错。
    let c = scene_bounds();
    let rust = Mesher::with_backend(256 << 20, Backend::Rust).unwrap();
    let buf = rust.build(&only_center(&c), MESH_OPAQUE).unwrap();
    // y=0 层无 -Y 面（y<0 哨兵剔除）：查 flags bit0-2 != 3 且 pos.y == 0。
    let mut ny_at_y0 = 0;
    for b in buf.vertex_data().chunks(24) {
        let y = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        let flags = b[21];
        if y == 0.0 && (flags & 7) == 3 {
            ny_at_y0 += 1;
        }
    }
    assert_eq!(ny_at_y0, 0, "y=0 层不应有 -Y 面（y<0 = 实心）");
    // y=255 柱顶 +Y 面存在且 sky=15（y>=256 = 空气、sky 15）。
    let mut py_top_sky = None;
    for b in buf.vertex_data().chunks(24) {
        let y = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        let flags = b[21];
        if y == 256.0 && (flags & 7) == 2 {
            py_top_sky = Some(b[19]);
        }
    }
    assert_eq!(py_top_sky, Some(15), "世界顶 +Y 面应采到 sky=15");
}

// ---------------------------------------------------------------------------
// 64 区块随机 seed 批量
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    /// splitmix64：确定性、无依赖。
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_chunk(rng: &mut Rng) -> Chunk {
    let mut c = chunk(0, 0xF0);
    let palette: [(usize, usize); 12] = [
        // (id, 权重)
        (0, 55),
        (id_of("stone") as usize, 12),
        (id_of("dirt") as usize, 6),
        (id_of("grass") as usize, 6),
        (id_of("water") as usize, 6),
        (id_of("log") as usize, 3),
        (id_of("leaves") as usize, 3),
        (id_of("oak_fence") as usize, 2),
        (id_of("oak_slab") as usize, 2),
        (id_of("oak_stairs") as usize, 2),
        (id_of("flower_red") as usize, 2),
        (id_of("torch") as usize, 1),
    ];
    let total: usize = palette.iter().map(|p| p.1).sum();
    for y in 0..256 {
        for z in 0..16 {
            for x in 0..16 {
                if rng.below(4) == 0 {
                    continue; // 保持大片空气/实体结构，避免纯噪声
                }
                let mut pick = rng.below(total as u64) as usize;
                let mut id = 0u16;
                for (bid, w) in palette {
                    if pick < w {
                        id = bid as u16;
                        break;
                    }
                    pick -= w;
                }
                // 半砖上下/楼梯朝向+top 的随机状态 nibble。
                let raw = match mcv_core::shape::Shape::from_u8(mcv_core::BLOCKS[id as usize].shape)
                {
                    mcv_core::shape::Shape::Slab => id | ((rng.below(2) as u16) << 12),
                    mcv_core::shape::Shape::Stairs => id | ((rng.below(8) as u16) << 12),
                    _ => id,
                };
                c.0[(y << 8) | (z << 4) | x] = raw;
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

#[test]
fn random_batch_64_chunks_byte_parity() {
    let mut rng = Rng(0x2026_1010_0C0B_0908);
    for ci in 0..64 {
        let center = random_chunk(&mut rng);
        // 两个共享邻块轮换 + 随机 None，覆盖跨格 id 交互与哨兵边界。
        let n1 = random_chunk(&mut rng);
        let n2 = random_chunk(&mut rng);
        let pool = [&center, &n1, &n2];
        let mut slots: [Option<Slot<'_>>; 9] = [None; 9];
        for (i, slot) in slots.iter_mut().enumerate() {
            if i == 4 {
                *slot = Some(Slot {
                    voxels: &center.0[..],
                    light: &center.1[..],
                });
            } else if rng.below(5) > 0 {
                let pick = &pool[rng.below(3) as usize];
                *slot = Some(Slot {
                    voxels: &pick.0[..],
                    light: &pick.1[..],
                });
            }
        }
        build_both_passes(&slots, &format!("rand#{ci}"));
    }
}

// ---------------------------------------------------------------------------
// 性能护栏：单区块（opaque+water）实测毫秒
// ---------------------------------------------------------------------------

/// 地形样区块：高度起伏 + 洞穴 + 水面，规模贴近真实生产输入。
fn perf_chunk() -> Chunk {
    let mut c = chunk(0, 0xF0);
    let (stone, dirt, grass, water) = (
        id_of("stone"),
        id_of("dirt"),
        id_of("grass"),
        id_of("water"),
    );
    for x in 0..16 {
        for z in 0..16 {
            let h = 60 + ((x * 13 + z * 7 + (x ^ z)) % 9);
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
            for y in h..64 {
                put(&mut c.0, x, y, z, water);
            }
            for y in 10..40 {
                if (x * 31 + y * 17 + z * 11) % 97 < 3 {
                    put(&mut c.0, x, y, z, 0);
                }
            }
        }
    }
    c
}

#[test]
fn rust_mesher_perf_single_chunk() {
    let c = perf_chunk();
    let slots = only_center(&c);
    let rust = Mesher::with_backend(256 << 20, Backend::Rust).unwrap();

    // 预热（页/分支缓存）后取 5 次均值：opaque+water 一整个区块。
    let _ = rust.build(&slots, MESH_OPAQUE).unwrap();
    let _ = rust.build(&slots, MESH_WATER).unwrap();
    const N: u32 = 5;
    let t0 = std::time::Instant::now();
    for _ in 0..N {
        let _ = rust.build(&slots, MESH_OPAQUE).unwrap();
        let _ = rust.build(&slots, MESH_WATER).unwrap();
    }
    let per_ms = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(N);

    // 对照组：C++ oracle 同输入耗时（信息性打印，便于回归感知）。
    let ffi = Mesher::with_backend(256 << 20, Backend::Ffi).unwrap();
    let _ = ffi.build(&slots, MESH_OPAQUE).unwrap();
    let t1 = std::time::Instant::now();
    for _ in 0..N {
        let _ = ffi.build(&slots, MESH_OPAQUE).unwrap();
        let _ = ffi.build(&slots, MESH_WATER).unwrap();
    }
    let ffi_ms = t1.elapsed().as_secs_f64() * 1000.0 / f64::from(N);

    println!("单区块 mesh（opaque+water 均值/{N}）：rust={per_ms:.3} ms, cxx={ffi_ms:.3} ms");
    // release 预算 10ms（任务要求）；CI test job 是 debug 构建，放宽到
    // 200ms 只做劣化哨兵。
    let budget_ms = if cfg!(debug_assertions) { 200.0 } else { 10.0 };
    assert!(
        per_ms < budget_ms,
        "单区块 mesh {per_ms:.3} ms 超预算 {budget_ms} ms（debug 构建属预期慢，若 release 超标需修）"
    );
}
