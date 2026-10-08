use std::path::PathBuf;

use mcv_save::{chunk_local, chunk_region, rle_decode, rle_encode, LevelMeta, RegionFile};

/// Deterministic pseudo-random voxel buffer with mixed run lengths
/// (including >255 runs to exercise the escape path).
fn sample_voxels(seed: u8) -> Vec<u8> {
    let mut v = vec![0u8; 65536];
    let mut s = seed as u32 | 1;
    let mut i = 0;
    while i < v.len() {
        let lcg = |s: &mut u32| {
            *s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            *s
        };
        let id = (lcg(&mut s) % 4) as u8;
        let run = 1 + (lcg(&mut s) % 600) as usize; // up to 600 > 255
        for b in v[i..(i + run).min(v.len())].iter_mut() {
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
        assert!(enc.len() < vox.len(), "rle must compress this data");
        let mut dec = vec![0u8; 65536];
        rle_decode(&enc, &mut dec).expect("decode");
        assert_eq!(dec, vox);
    }
}

#[test]
fn rle_rejects_truncated() {
    let mut dec = vec![0u8; 16];
    assert!(rle_decode(&[0, 5, 0], &mut dec).is_err());
    assert!(rle_decode(&[5], &mut dec).is_err());
}

#[test]
fn region_roundtrip() {
    let dir = PathBuf::from(std::env::temp_dir().join("mcv_region_test"));
    let _ = std::fs::remove_dir_all(&dir);
    let mut region = RegionFile::open(&dir, 0, 0).expect("open");

    let a = sample_voxels(3);
    let b = sample_voxels(9);
    region.save_chunk(chunk_local(0, 0), &a).expect("save a");
    region.save_chunk(chunk_local(5, 7), &b).expect("save b");
    assert!(region.has_chunk(chunk_local(0, 0)).unwrap());
    assert!(!region.has_chunk(chunk_local(1, 1)).unwrap());

    let mut out = vec![0u8; 65536];
    region
        .load_chunk(chunk_local(0, 0), &mut out)
        .expect("load a");
    assert_eq!(out, a);
    region
        .load_chunk(chunk_local(5, 7), &mut out)
        .expect("load b");
    assert_eq!(out, b);

    // Overwrite: smaller record reuses the slot.
    let small = vec![1u8; 65536];
    region
        .save_chunk(chunk_local(0, 0), &small)
        .expect("resave");
    region
        .load_chunk(chunk_local(0, 0), &mut out)
        .expect("reload");
    assert_eq!(out, small);
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
        player: Some(mcv_save::PlayerMeta {
            x: 1.5,
            y: 97.0,
            z: -3.25,
            yaw: 90.0,
            pitch: -12.5,
            flying: true,
            sel_slot: 4,
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

    let bare = LevelMeta {
        seed: 7,
        name: "w".into(),
        day_time: 0,
        player: None,
    };
    let back = LevelMeta::decode(&bare.encode()).expect("decode");
    assert!(back.player.is_none());
}
