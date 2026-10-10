use mcv_save::{
    CHUNK_VERSION, LevelMeta, RegionFile, chunk_local, chunk_region, rle_decode, rle_encode,
};

/// Deterministic pseudo-random voxel buffer with mixed run lengths
/// (including >255 runs to exercise the escape path). Uses u16 ids beyond
/// 255 (900/1000) to pin the widened little-endian id encoding.
fn sample_voxels(seed: u8) -> Vec<u16> {
    let mut v = vec![0u16; mcv_core::CHUNK_VOL];
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
        let mut dec = vec![0u16; mcv_core::CHUNK_VOL];
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
    let mut vox = vec![0u16; mcv_core::CHUNK_VOL];
    // acacia_slab(26) 上半 (state=1) | acacia_stairs(27) facing=2+top (state=6)
    let slab_top: u16 = 26 | (1 << 12);
    let stair: u16 = 27 | (6 << 12);
    vox[..300].fill(slab_top); // >255 长跑走转义路径
    vox[300..400].fill(stair);
    vox[500] = slab_top;
    let enc = rle_encode(&vox);
    let mut dec = vec![0u16; mcv_core::CHUNK_VOL];
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
    // v6：v3 记录尾带 heightmap i16[256]（绝对 y，可负）——往返同锁。
    let hm: [i16; 256] = std::array::from_fn(|i| (i as i32 - 64) as i16);
    region
        .save_chunk(chunk_local(0, 0), &a, &hm)
        .expect("save a");
    region
        .save_chunk(chunk_local(5, 7), &b, &hm)
        .expect("save b");
    assert!(region.has_chunk(chunk_local(0, 0)).unwrap());
    assert!(!region.has_chunk(chunk_local(1, 1)).unwrap());

    let mut out = vec![0u16; mcv_core::CHUNK_VOL];
    let mut hm_out = [0i16; 256];
    region
        .load_chunk(chunk_local(0, 0), &mut out, &mut hm_out)
        .expect("load a");
    assert_eq!(out, a);
    assert_eq!(hm_out, hm, "heightmap 落盘往返漂移");
    region
        .load_chunk(chunk_local(5, 7), &mut out, &mut hm_out)
        .expect("load b");
    assert_eq!(out, b);
    assert_eq!(hm_out, hm);

    // Overwrite: smaller record reuses the slot.
    let small = vec![1u16; mcv_core::CHUNK_VOL];
    region
        .save_chunk(chunk_local(0, 0), &small, &hm)
        .expect("resave");
    region
        .load_chunk(chunk_local(0, 0), &mut out, &mut hm_out)
        .expect("reload");
    assert_eq!(out, small);
}

/// v3 门：rle 完整但缺 512B heightmap 尾部的记录必须拒载（不崩）。
#[test]
fn region_rejects_truncated_heightmap_tail() {
    let dir = std::env::temp_dir().join("mcv_region_test_hmtail");
    let _ = std::fs::remove_dir_all(&dir);
    let region = RegionFile::open(&dir, 0, 0).expect("open");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(region.path())
        .expect("reopen");
    use std::io::{Seek, SeekFrom, Write};
    file.seek(SeekFrom::Start(512 * 8)).expect("seek body");
    let mut record: Vec<u8> = Vec::new();
    record.extend_from_slice(&0u16.to_le_bytes());
    record.push(mcv_save::CHUNK_VERSION);
    let vox = vec![7u16; mcv_core::CHUNK_VOL];
    let rle = rle_encode(&vox);
    record.extend_from_slice(&(rle.len() as u32).to_le_bytes());
    record.extend_from_slice(&rle); // 无 heightmap 尾
    let len = record.len() as u32;
    file.write_all(&record).expect("write");
    file.seek(SeekFrom::Start(0)).expect("seek head");
    file.write_all(&(512u32 * 8u32).to_le_bytes()).expect("off");
    file.write_all(&len.to_le_bytes()).expect("len");
    drop(file);

    let mut region = RegionFile::open(&dir, 0, 0).expect("reopen");
    let mut out = vec![0u16; mcv_core::CHUNK_VOL];
    let mut hm = [0i16; 256];
    assert!(
        region
            .load_chunk(chunk_local(0, 0), &mut out, &mut hm)
            .is_err(),
        "缺 heightmap 尾的 v3 记录必须拒载"
    );
}

#[test]
fn region_rejects_old_version() {
    // v1 (u8-id) 与 v2（256 高）都必须整拒：开发格式，无兼容义务。
    let dir = std::env::temp_dir().join("mcv_region_test_ver");
    let _ = std::fs::remove_dir_all(&dir);
    let region = RegionFile::open(&dir, 0, 0).expect("open");
    // Hand-write a fake old-version record and point the header at it.
    use std::io::{Seek, SeekFrom, Write};
    let off = region.path();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(off)
        .expect("reopen");
    file.seek(SeekFrom::Start(512 * 8)).expect("seek body");
    let mut record: Vec<u8> = Vec::new();
    record.extend_from_slice(&0u16.to_le_bytes()); // local
    record.push(1u8); // stale version byte（下方逐版覆写）
    record.extend_from_slice(&3u32.to_le_bytes()); // rle_len (v1 encoding)
    record.extend_from_slice(&[10, 3, 10]); // 10x id 3, v1 style
    let len = record.len() as u32;
    file.write_all(&record).expect("write v1");
    // header slot 0: offset + len
    file.seek(SeekFrom::Start(0)).expect("seek head");
    file.write_all(&(512u32 * 8u32).to_le_bytes()).expect("off");
    file.write_all(&len.to_le_bytes()).expect("len");
    drop(file);

    for stale in [1u8, 2u8, CHUNK_VERSION + 1] {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&off)
            .expect("reopen");
        file.seek(SeekFrom::Start(512 * 8 + 2))
            .expect("seek ver byte");
        file.write_all(&[stale]).expect("write ver byte");
        drop(file);
        let mut region = RegionFile::open(&dir, 0, 0).expect("reopen");
        let mut out = vec![0u16; mcv_core::CHUNK_VOL];
        let mut hm = [0i16; 256];
        let err = region
            .load_chunk(chunk_local(0, 0), &mut out, &mut hm)
            .expect_err("stale/future version must be rejected");
        assert!(
            err.to_string().contains("version"),
            "v{stale}: expected a version error, got: {err}"
        );
    }
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
            // v5 生存数值（非默认值，锁死往返）。
            health: 7.5,
            hunger: 14.0,
            saturation: 2.25,
            exhaustion: 12.5,
            air_supply: 240,
            difficulty: 1,
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
    // v5 生存数值往返。
    assert_eq!((p.health, p.hunger), (7.5, 14.0));
    assert_eq!((p.saturation, p.exhaustion), (2.25, 12.5));
    assert_eq!((p.air_supply, p.difficulty), (240, 1));

    // 版本 6（超前）必须拒绝。
    let mut future = bytes.clone();
    future[4] = 6;
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
