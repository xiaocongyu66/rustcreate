//! FFI memory contract tests: version handshake, canary guards, double-free
//! detection, budget enforcement.

use mcv_ffi::{err, MemPool, ABI_VERSION};

#[test]
fn version_matches() {
    mcv_ffi::check_abi();
    assert_eq!(ABI_VERSION, 0x5255_4332);
}

#[test]
fn acquire_release_roundtrip() {
    let pool = MemPool::new(4 << 20).unwrap();
    let buf = pool.acquire(1024, 1024).expect("acquire");
    let (v, i) = buf.counts();
    assert_eq!((v, i), (0, 0));
    // Both regions are distinct and sized.
    assert!(buf.capacity_bytes() >= 1024 * 28);
    assert_eq!(buf.indices().len(), 0); // count is 0 until C++ fills it
    assert!(buf.capacity_bytes() >= 1024 * 24 + 1024 * 4);
    drop(buf);

    let (live, _bytes) = pool.stats();
    assert_eq!(live, 0, "release must decrement live count");

    // Second acquire must reuse the cached block (no failure, still bounded).
    let again = pool.acquire(1024, 1024).expect("reacquire");
    assert!(again.capacity_bytes() >= 1024 * 28);
    drop(again);
}

#[test]
fn canary_rejects_overflow() {
    let pool = MemPool::new(1 << 20).unwrap();
    let buf = pool.acquire(64, 64).expect("acquire");
    let mut raw = buf.into_raw();
    // Payload capacity is the whole class slot; the trailing canary lives
    // immediately after it. Overwrite it via the vertex pointer using the
    // class capacity: class 0 payload = 16 KiB.
    assert_eq!(raw.pool_class, 0);
    let payload_cap = 16384usize;
    unsafe {
        // indices sit at payload + vertex_cap*24; write past the payload end
        // relative to the true block end via indices pointer arithmetic is
        // fragile — poke the byte right after payload through vertex_data.
        let payload = raw.vertex_data;
        // indices offset = 64*24 = 1536; payload_cap = 16384; canary at
        // payload + 16384. Write via a pointer derived from vertex_data.
        std::ptr::write_bytes(payload.add(payload_cap), 0xFF, 1);
    }
    let rc = mcv_ffi::meshbuf_release_raw(&mut raw);
    assert_eq!(rc, err::CANARY_CORRUPT, "smashed canary must be detected");
}

#[test]
fn rejects_double_free() {
    let pool = MemPool::new(1 << 20).unwrap();
    let buf = pool.acquire(64, 64).expect("acquire");
    let mut raw = buf.into_raw();
    assert_eq!(mcv_ffi::meshbuf_release_raw(&mut raw), err::OK);
    let rc = mcv_ffi::meshbuf_release_raw(&mut raw);
    assert_eq!(rc, err::DOUBLE_FREE, "second release must be detected");
}

#[test]
fn budget_enforced() {
    // Budget smaller than one class-1 block (64 KiB payload + overhead).
    let pool = MemPool::new(16 * 1024).unwrap();
    // Class 0 total = header(24)+8+16384+8 = 16424 > 16384 budget → OOM.
    let rc = pool.acquire(600, 64).err().expect("expected OOM");
    assert_eq!(rc, err::OOM);
}

#[test]
fn oversize_rejected() {
    let pool = MemPool::new(256 << 20).unwrap();
    // 1M vertices * 24 B = 24 MiB > 4 MiB max class.
    let rc = pool.acquire(1_000_000, 0).err().expect("expected OOM");
    assert_eq!(rc, err::OOM);
}
