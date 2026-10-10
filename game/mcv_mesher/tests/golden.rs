//! 纯 Rust 网格器（src/mesher.rs）↔ 黄金数据逐字节回归锁——任务板 #77：
//! C++ frozen oracle（原 mcv_ffi/mcv_mesh_build → cpp/src/mesher.cpp）删除
//! 前，其输出在本机全量落盘固化（tests/golden/；确定性场景 14 个全量字节
//! 文件 + 64 区块随机批 128 行 FNV 哈希表）。
//!
//! **回归锁，非正确性标准**：黄金值是 frozen oracle 的输出，即旧贪心网格
//! 路径的近似语义——它不是「原版应该如此」的依据；26.1 的正确性门在
//! vanilla 后端 + tests/quality.rs（对标 src-26.1 反编译源码）。黄金文件
//! 唯一职责：C++ 删除后，Rust 网格器仍与删除前的 oracle 输出逐字节一致
//! （防重构手滑），对拍不过修 Rust 侧。
//!
//! 输入规格唯一来源见 tests/golden/cases.rs（场景构造/splitmix64/行序）。

#[path = "golden/cases.rs"]
mod cases;

use cases::{
    RANDOM_BATCH_CHUNKS, RANDOM_BATCH_SEED, Rng, SCENES, fnv1a, full9, only_center, random_chunk,
    scene_by_name,
};
use mcv_mesher::{MESH_OPAQUE, MESH_WATER, Mesher, Slot};

fn golden_dir() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden").to_string()
}

/// 场景黄金文件（mesh_scenes/）逐字节断言：u32 vc + u32 ic + 顶点 + 索引。
#[test]
fn deterministic_scenes_byte_match_golden() {
    let golden = golden_dir();
    let air = cases::chunk(0, 0xF0);
    let mesher = Mesher::new();
    for (name, loaded) in SCENES {
        let center = scene_by_name(name);
        let slots = if loaded {
            full9(&center, &air)
        } else {
            only_center(&center)
        };
        for kind in [MESH_OPAQUE, MESH_WATER] {
            let path = format!("{golden}/mesh_scenes/{name}_k{kind}.bin");
            let want = std::fs::read(&path).unwrap_or_else(|e| panic!("黄金件缺失 {path}: {e}"));
            let got = mesher.build(&slots, kind).expect("rust build");
            let mut blob = Vec::with_capacity(8 + got.vertices.len() + got.indices.len() * 4);
            blob.extend_from_slice(&got.counts().0.to_le_bytes());
            blob.extend_from_slice(&got.counts().1.to_le_bytes());
            blob.extend_from_slice(got.vertex_data());
            for i in got.indices() {
                blob.extend_from_slice(&i.to_le_bytes());
            }
            let (got_len, want_len) = (blob.len(), want.len());
            assert_eq!(
                got_len, want_len,
                "{name} k{kind}: 输出长度 {got_len} != 黄金 {want_len}"
            );
            for (i, (x, y)) in blob.iter().zip(want.iter()).enumerate() {
                assert_eq!(x, y, "{name} k{kind}: 首个字节差异在 {i}");
            }
        }
    }
}

/// 随机批黄金表逐行：`idx name kind in_hash vc ic vhash ihash`。
/// in_hash 锁输入规格（cases.rs 与黄金数据不同步即红），后三列锁输出。
#[test]
fn random_batch_matches_golden_table() {
    let text = std::fs::read_to_string(format!("{}/mesh_random_batch.tsv", golden_dir()))
        .expect("黄金表应随仓库存在");
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();
    assert_eq!(rows.len(), RANDOM_BATCH_CHUNKS * 2, "黄金行数漂移");

    let mesher = Mesher::new();
    let mut rng = Rng(RANDOM_BATCH_SEED);
    let mut ri = 0usize;
    for ci in 0..RANDOM_BATCH_CHUNKS {
        let center = random_chunk(&mut rng);
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
        // 输入哈希口径：与黄金落盘驱动一致——u32 kind LE + 9×(present 字节 +
        // 131072B 体素 LE + 65536B 光照)。
        for kind in [MESH_OPAQUE, MESH_WATER] {
            let row = &rows[ri];
            assert_eq!(
                row[0],
                format!("{:03}", ci * 2 + kind as usize + 14),
                "行号漂移 @ {ri}"
            );
            assert_eq!(row[1], format!("rand{ci}"), "用例名漂移 @ {ri}");
            assert_eq!(row[2], kind.to_string(), "kind 漂移 @ {ri}");
            let mut input = Vec::with_capacity(4 + 9 * (1 + 131072 + 65536));
            input.extend_from_slice(&kind.to_le_bytes());
            for slot in &slots {
                match slot {
                    Some(s) => {
                        input.push(1);
                        for v in s.voxels {
                            input.extend_from_slice(&v.to_le_bytes());
                        }
                        input.extend_from_slice(s.light);
                    }
                    None => input.push(0),
                }
            }
            assert_eq!(
                row[3],
                format!("{:016x}", fnv1a(&input)),
                "rand{ci} k{kind}: 输入哈希与黄金不符——cases.rs 输入规格漂移"
            );

            let got = mesher.build(&slots, kind).expect("rust build");
            let (vc, ic) = got.counts();
            assert_eq!(row[4], vc.to_string(), "rand{ci} k{kind}: 顶点数漂移");
            assert_eq!(row[5], ic.to_string(), "rand{ci} k{kind}: 索引数漂移");
            assert_eq!(
                row[6],
                format!("{:016x}", fnv1a(got.vertex_data())),
                "rand{ci} k{kind}: 顶点字节黄金哈希漂移"
            );
            let idx_bytes: Vec<u8> = got.indices().iter().flat_map(|i| i.to_le_bytes()).collect();
            assert_eq!(
                row[7],
                format!("{:016x}", fnv1a(&idx_bytes)),
                "rand{ci} k{kind}: 索引黄金哈希漂移"
            );
            ri += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// 表锁：geom 推导规则逐 id 对照黄金生成表（删除前 cpp/src/blocks_gen.inc
// 的逐字节副本；kind 无法从名字推导的漂移护栏）
// ---------------------------------------------------------------------------

#[test]
fn rust_geom_rule_matches_golden_generated_table() {
    let path = format!("{}/blocks_gen.inc", golden_dir());
    let text = std::fs::read_to_string(path).expect("黄金生成表应随仓库存在");
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
        // 初始化行以「},」收尾，剥壳后残留尾逗号 → 先去再切。
        let flat: Vec<&str> = compact
            .trim()
            .trim_end_matches(',')
            .split(',')
            .map(str::trim)
            .collect();
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
        // shape 0..5 逐 id 严格锁；6..9 族（carpet/trapdoor/pane/wall）容许
        // 黄金列冻结为 0：main 8a91368 撤销生成表重生成（C++ 退役中，黄金
        // 件即彼时快照），四族两侧均走全盒占位、网格字节逐位同。
        // 除「相等」或「Rust 6..9 且黄金 0」外的任何漂移仍然报错。
        assert!(
            shape == def.shape || (def.shape >= 6 && shape == 0),
            "id {count} shape: 黄金 shape 列与 Rust shape.rs 漂移（黄金={}，Rust={}；\
             6..9 族容许黄金冻结 0，见 main 8a91368）",
            shape,
            def.shape
        );
        assert!(
            (flat[2] == "true") == mcv_mesher::block_geom(def),
            "id {count} {}: geom 推导与黄金表漂移（黄金 geom={}）",
            def.name,
            flat[2]
        );
        count += 1;
    }
    assert_eq!(count, mcv_core::BLOCKS.len(), "两侧表行数应一致");
}

// ---------------------------------------------------------------------------
// 越界/邻域缺失语义（Rust 侧关键不变量，防与黄金同错）
// ---------------------------------------------------------------------------

#[test]
fn bounds_semantics_rust_invariants() {
    let c = scene_by_name("bounds");
    let mesher = Mesher::new();
    let buf = mesher.build(&only_center(&c), MESH_OPAQUE).unwrap();
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
// 性能护栏：单区块（opaque+water）实测毫秒（原 C++ 对照组随 oracle 删除）
// ---------------------------------------------------------------------------

/// 地形样区块：高度起伏 + 洞穴 + 水面，规模贴近真实生产输入。
fn perf_chunk() -> cases::Chunk {
    let mut c = cases::chunk(0, 0xF0);
    let (stone, dirt, grass, water) = (
        cases::id_of("stone"),
        cases::id_of("dirt"),
        cases::id_of("grass"),
        cases::id_of("water"),
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
                cases::put(&mut c.0, x, y, z, id);
            }
            for y in h..64 {
                cases::put(&mut c.0, x, y, z, water);
            }
            for y in 10..40 {
                if (x * 31 + y * 17 + z * 11) % 97 < 3 {
                    cases::put(&mut c.0, x, y, z, 0);
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
    let rust = Mesher::new();

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

    println!("单区块 mesh（opaque+water 均值/{N}）：{per_ms:.3} ms");
    // release 预算 10ms（任务要求）；CI test job 是 debug 构建，放宽到
    // 200ms 只做劣化哨兵。
    let budget_ms = if cfg!(debug_assertions) { 200.0 } else { 10.0 };
    assert!(
        per_ms < budget_ms,
        "单区块 mesh {per_ms:.3} ms 超预算 {budget_ms} ms（debug 构建属预期慢，若 release 超标需修）"
    );
}
