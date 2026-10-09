use mcv_save::{LevelMeta, RegionFile, chunk_local, chunk_region, rle_decode, rle_encode};

/// Deterministic pseudo-random voxel buffer with mixed run lengths
/// (including >255 runs to exercise the escape path). Uses u16 ids beyond
/// 255 (900/1000) to pin the widened little-endian id encoding.
fn sample_voxels(seed: u8) -> Vec<u16> {
    let mut v = vec![0u16; 65536];
    let mut s = seed as u32 | 1;
    let mut i = 0;
    while i < v.len() {
        let lcg = |s: &mut u32| {
            *s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            *s
        };
        let table: [u16; 4] = [0, 3, 900, 1000];
        let id = table[(lcg(&mut s) % 4) as usize];
        let run = 1 + (lcg(&mut s) % 600) as usize; // up to 600 > 255
        let end = (i + run).min(v.len());
        for b in v[i..end].iter_mut() {
            *b = id;
        }
        i += run;
    }
    v
}

#[test]
fn rle_roundtrip_mixed_runs() {
    for seed in 0..8u8 {
        let vox = sample_voxels(seed);
        let enc = rle_encode(&vox);
        assert!(enc.len() < vox.len() * 2, "rle must compress this data");
        let mut dec = vec![0u16; 65536];
        rle_decode(&enc, &mut dec).expect("decode");
        assert_eq!(dec, vox);
    }
}

/// u16 边界：id = 1000（>255，锁死 LE 两字节编码）的 >255 长跑转义往返。
#[test]
fn rle_u16_escape_run_boundary_id() {
    let mut vox = vec![0u16; 1024];
    vox[..600].fill(1000); // escaped run, id beyond the old u8 range
    vox[600..].fill(258); // short run, also >255
    let enc = rle_encode(&vox);
    // escape: 0 + len(2) + id(2) = 5 bytes; short run: 1 + id(2) = 3 bytes
    assert_eq!(&enc[..5], &[0, 88, 2, 232, 3]); // len 600 LE (0x0258), id 1000 LE (0x03E8)
    let mut dec = vec![0u16; 1024];
    rle_decode(&enc, &mut dec).expect("decode");
    assert_eq!(dec, vox);
}

/// 状态位往返：RLE 按原始 u16 两字节编码，bit12-15 状态 nibble（上半砖/
/// 楼梯朝向）必须与方块 id 一起无损存读。
#[test]
fn rle_roundtrip_preserves_state_nibble() {
    let mut vox = vec![0u16; 65536];
    // acacia_slab(26) 上半 (state=1) | acacia_stairs(27) facing=2+top (state=6)
    let slab_top: u16 = 26 | (1 << 12);
    let stair: u16 = 27 | (6 << 12);
    vox[..300].fill(slab_top); // >255 长跑走转义路径
    vox[300..400].fill(stair);
    vox[500] = slab_top;
    let enc = rle_encode(&vox);
    let mut dec = vec![0u16; 65536];
    rle_decode(&enc, &mut dec).expect("decode");
    assert_eq!(dec, vox);
    assert_eq!(dec[0], slab_top);
    assert_eq!(dec[350] >> 12, 6);
}

#[test]
fn rle_rejects_truncated() {
    let mut dec = vec![0u16; 16];
    // truncated escape run (needs count + len2 + id2)
    assert!(rle_decode(&[0, 5, 0], &mut dec).is_err());
    // truncated short run (needs count + id2)
    assert!(rle_decode(&[5], &mut dec).is_err());
    // short run missing the high id byte
    assert!(rle_decode(&[5, 1], &mut dec).is_err());
    // decoded length mismatch: one complete 5x-256 run != 16 output cells
    assert!(rle_decode(&[5, 1, 0], &mut dec).is_err());
}

#[test]
fn region_roundtrip() {
    let dir = std::env::temp_dir().join("mcv_region_test");
    let _ = std::fs::remove_dir_all(&dir);
    let mut region = RegionFile::open(&dir, 0, 0).expect("open");

    let a = sample_voxels(3);
    let b = sample_voxels(9);
    region.save_chunk(chunk_local(0, 0), &a).expect("save a");
    region.save_chunk(chunk_local(5, 7), &b).expect("save b");
    assert!(region.has_chunk(chunk_local(0, 0)).unwrap());
    assert!(!region.has_chunk(chunk_local(1, 1)).unwrap());

    let mut out = vec![0u16; 65536];
    region
        .load_chunk(chunk_local(0, 0), &mut out)
        .expect("load a");
    assert_eq!(out, a);
    region
        .load_chunk(chunk_local(5, 7), &mut out)
        .expect("load b");
    assert_eq!(out, b);

    // Overwrite: smaller record reuses the slot.
    let small = vec![1u16; 65536];
    region
        .save_chunk(chunk_local(0, 0), &small)
        .expect("resave");
    region
        .load_chunk(chunk_local(0, 0), &mut out)
        .expect("reload");
    assert_eq!(out, small);
}

#[test]
fn region_rejects_old_version() {
    // v1 (u8-id) records must be rejected outright: dev format, no compat.
    let dir = std::env::temp_dir().join("mcv_region_test_ver");
    let _ = std::fs::remove_dir_all(&dir);
    let region = RegionFile::open(&dir, 0, 0).expect("open");
    let mut out = vec![0u16; 65536];
    // Hand-write a fake v1 record and point the header at it.
    use std::io::{Seek, SeekFrom, Write};
    let off = region.path();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(off)
        .expect("reopen");
    file.seek(SeekFrom::Start(512 * 8)).expect("seek body");
    let mut record: Vec<u8> = Vec::new();
    record.extend_from_slice(&0u16.to_le_bytes()); // local
    record.push(1u8); // stale v1 version byte
    record.extend_from_slice(&3u32.to_le_bytes()); // rle_len (v1 encoding)
    record.extend_from_slice(&[10, 3, 10]); // 10x id 3, v1 style
    let len = record.len() as u32;
    file.write_all(&record).expect("write v1");
    // header slot 0: offset + len
    file.seek(SeekFrom::Start(0)).expect("seek head");
    file.write_all(&(512u32 * 8u32).to_le_bytes()).expect("off");
    file.write_all(&len.to_le_bytes()).expect("len");
    drop(file);

    let mut region = RegionFile::open(&dir, 0, 0).expect("reopen");
    let err = region
        .load_chunk(chunk_local(0, 0), &mut out)
        .expect_err("v1 must be rejected");
    assert!(
        err.to_string().contains("version"),
        "expected a version error, got: {err}"
    );
}

#[test]
fn region_coords() {
    let (rx, rz) = chunk_region(-1, -1);
    assert_eq!((rx, rz), (-1, -1));
    assert_eq!(chunk_local(-1, -1), 0xFF);
    let (rx, rz) = chunk_region(16, 32);
    assert_eq!((rx, rz), (1, 2));
    assert_eq!(chunk_local(16, 32), 0);
}

#[test]
fn meta_roundtrip() {
    let meta = LevelMeta {
        seed: 0xDEAD_BEEF_1234_5678,
        name: "新的世界".into(),
        day_time: 13_000,
        mode: 0,
        player: Some(mcv_save::PlayerMeta {
            x: 1.5,
            y: 97.0,
            z: -3.25,
            yaw: 90.0,
            pitch: -12.5,
            flying: true,
            sel_slot: 4,
            // 中间空槽以 (0,0,0) 占位，槽序即下标。
            hotbar: vec![(9, 1, 0), (0, 0, 0), (27, 64, 3)],
            main: vec![(0, 0, 0); 26].into_iter().chain([(1, 5, 0)]).collect(),
        }),
    };
    let bytes = meta.encode();
    let back = LevelMeta::decode(&bytes).expect("decode");
    assert_eq!(back.seed, meta.seed);
    assert_eq!(back.name, meta.name);
    assert_eq!(back.day_time, meta.day_time);
    let p = back.player.expect("player");
    assert_eq!(p.x, 1.5);
    assert_eq!(p.z, -3.25);
    assert!(p.flying);
    assert_eq!(p.sel_slot, 4);
    assert_eq!(p.hotbar.len(), 9, "v4 起快捷栏补齐 9 槽");
    assert_eq!(p.hotbar[..3], vec![(9, 1, 0), (0, 0, 0), (27, 64, 3)]);
    assert_eq!(p.main.len(), 27);
    assert_eq!(p.main[26], (1, 5, 0), "主背包尾槽保序");

    // 版本 5 必须拒绝。
    let mut future = bytes.clone();
    future[4] = 5;
    assert!(LevelMeta::decode(&future).is_err());

    let bare = LevelMeta {
        seed: 7,
        name: "w".into(),
        day_time: 0,
        mode: 0,
        player: None,
    };
    let back = LevelMeta::decode(&bare.encode()).expect("decode");
    assert!(back.player.is_none());
}

#[test]
fn meta_v2_backcompat() {
    // 手写 v2 字节流(无快捷栏字段):必须照常解码,hotbar 读为空。
    let mut v: Vec<u8> = Vec::new();
    v.extend_from_slice(b"MCV1");
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&42u64.to_le_bytes());
    v.extend_from_slice(&1234u64.to_le_bytes());
    v.extend_from_slice(&[1, b'x']);
    v.push(1); // player present
    for f in [1.0f32, 2.0, 3.0, 4.0, 5.0] {
        v.extend_from_slice(&f.to_le_bytes());
    }
    v.push(0); // flying
    v.push(7); // sel_slot
    v.push(1); // mode = creative（保持末尾字节约定）
    let back = LevelMeta::decode(&v).expect("v2 decodes");
    assert_eq!((back.seed, back.day_time, back.mode), (42, 1234, 1));
    let p = back.player.expect("player");
    assert_eq!(p.sel_slot, 7);
    assert!(p.hotbar.is_empty(), "v1/v2 读为空快捷栏");
    assert!(p.main.is_empty(), "v1/v2 读为空主背包");
}

#[test]
fn meta_v3_backcompat() {
    // 手写 v3 字节流(快捷栏存在、主背包字段不存在):必须照常解码,
    // main 读为空。
    let mut v: Vec<u8> = Vec::new();
    v.extend_from_slice(b"MCV1");
    v.extend_from_slice(&3u16.to_le_bytes());
    v.extend_from_slice(&9u64.to_le_bytes());
    v.extend_from_slice(&500u64.to_le_bytes());
    v.extend_from_slice(&[1, b'x']);
    v.push(1); // player present
    for f in [1.0f32, 2.0, 3.0, 4.0, 5.0] {
        v.extend_from_slice(&f.to_le_bytes());
    }
    v.push(0); // flying
    v.push(2); // sel_slot
    v.push(2); // hotbar len（v3 旧式：只写非空前缀）
    v.extend_from_slice(&7u16.to_le_bytes());
    v.push(1);
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&9u16.to_le_bytes());
    v.push(3);
    v.extend_from_slice(&2u16.to_le_bytes());
    v.push(0); // mode = survival（末尾字节约定）
    let back = LevelMeta::decode(&v).expect("v3 decodes");
    assert_eq!(back.mode, 0);
    let p = back.player.expect("player");
    assert_eq!(p.hotbar, vec![(7, 1, 0), (9, 3, 2)]);
    assert!(p.main.is_empty(), "v3 读为空主背包");
}
