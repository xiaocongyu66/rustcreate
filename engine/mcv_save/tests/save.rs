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
    assert_eq!(p.hotbar, vec![(9, 1, 0), (0, 0, 0), (27, 64, 3)]);

    // 版本 4 必须拒绝。
    let mut future = bytes.clone();
    future[4] = 4;
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
}
