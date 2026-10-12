//! #172 刀B 调度器级增量锁：[`MeshScheduler::submit_incremental`] 的
//! 回传产物与整柱直呼 `build_mesh` **逐字节相同**，重放工作量（`work`）
//! 远低于整柱；产物缓存（Gate products）只在提交地板准入后写入，kill /
//! 直算派发即作废——游戏层「有缓存才窄行增量」前提的调度器侧证明。

#[path = "golden/cases.rs"]
mod cases;

use cases::{Chunk, chunk, put, scene_by_name};
use mcv_core::{CHUNK_VOL, ChunkPos};
use mcv_mesher::{
    MeshDone, MeshJob, MeshResults, MeshScheduler, Rows, SECTION_COUNT, TOTAL_SCANNED_ROWS,
    build_mesh, edit_rows, neighborhood_slots, subchunk_enabled_from,
};

const POS: ChunkPos = ChunkPos::new(3, -2);

fn flat(center: &Chunk, side: &Chunk) -> (Vec<u16>, Vec<u8>) {
    let mut vox = Vec::with_capacity(9 * CHUNK_VOL);
    let mut lit = Vec::with_capacity(9 * CHUNK_VOL);
    for i in 0..9 {
        let c = if i == 4 { center } else { side };
        vox.extend_from_slice(&c.0);
        lit.extend_from_slice(&c.1);
    }
    (vox, lit)
}

fn job(center: &Chunk, side: &Chunk) -> MeshJob {
    let (voxels, lights) = flat(center, side);
    MeshJob {
        pos: POS,
        origin: [0.0, 0.0, 0.0],
        voxels,
        lights,
    }
}

fn direct_build(center: &Chunk, side: &Chunk, kind: u32) -> (Vec<u8>, Vec<u32>) {
    let (vox, lit) = flat(center, side);
    let slots = neighborhood_slots(&vox, &lit);
    let mut v: [Option<&[u16]>; 9] = [None; 9];
    let mut l: [Option<&[u8]>; 9] = [None; 9];
    for i in 0..9 {
        v[i] = slots[i].map(|s| s.voxels);
        l[i] = slots[i].map(|s| s.light);
    }
    let m = build_mesh(&v, &l, kind);
    (m.vertices, m.indices)
}

fn wait_one(results: &mut MeshResults) -> MeshDone {
    for _ in 0..30_000 {
        let mut v = results.drain_committed();
        assert!(v.len() <= 1, "只投递了一笔");
        if v.len() == 1 {
            return v.pop().unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("worker 超时未回传");
}

fn bytes_of(d: &MeshDone, kind: u32) -> Option<(Vec<u8>, Vec<u32>)> {
    let b = if kind == 0 { &d.opaque } else { &d.water };
    b.as_ref().map(|m| (m.vertex.clone(), m.index.clone()))
}

/// 播种笔（全柱产物化）→ 窄行增量笔：字节同直呼、work 大降、缓存就位。
#[test]
fn seed_then_narrow_edit_bit_identical_with_low_work() {
    let (sched, mut results) = MeshScheduler::new(1);
    let side = chunk(0, 0xF0);
    let center_a = scene_by_name("cave");
    // 播种笔：rows=ALL、old=None —— 字节必须等于直呼，work = 整柱。
    sched.submit_incremental(job(&center_a, &side), true, Rows::ALL, None);
    let done = wait_one(&mut results);
    assert_eq!(done.work, TOTAL_SCANNED_ROWS + SECTION_COUNT);
    for kind in [0u32, 1] {
        assert_eq!(
            bytes_of(&done, kind),
            direct_build(&center_a, &side, kind),
            "播种笔 k{kind} 字节 != 直呼"
        );
    }
    assert!(done.products.is_some());
    assert!(sched.product_of(POS).is_some(), "准入后产物缓存就位");

    // 窄行增量笔：section 边界编辑（y=-49：-64 基准 section 1 内，
    // ±1 触界），脏行集外的桶全部复用。
    let mut center_b = (center_a.0.clone(), center_a.1.clone());
    put(&mut center_b.0, 8, -49, 8, 1);
    center_b.1[mcv_core::vidx(8, -49, 8)] ^= 0x11;
    let rows = edit_rows(8, -49, 8);
    sched.submit_incremental(job(&center_b, &side), false, rows, sched.product_of(POS));
    let done = wait_one(&mut results);
    assert!(
        done.work <= 2 * 9 + 3,
        "单格编辑重放量 = {}，应为 ≤21（18 扫描行 + ≤3 形状 section，\
         整柱 = {}）",
        done.work,
        TOTAL_SCANNED_ROWS + SECTION_COUNT
    );
    assert!(done.work < TOTAL_SCANNED_ROWS + SECTION_COUNT);
    for kind in [0u32, 1] {
        assert_eq!(
            bytes_of(&done, kind),
            direct_build(&center_b, &side, kind),
            "窄行增量笔 k{kind} 字节 != 直呼"
        );
    }
    assert!(sched.product_of(POS).is_some(), "增量提交后缓存刷新");
}

/// 缓存生命周期：直算派发即作废（保守防陈旧产物误用）；kill 同点作废；
/// 拒收（僵尸）产物不入缓存。
#[test]
fn product_cache_lifecycle() {
    let (sched, mut results) = MeshScheduler::new(1);
    let side = chunk(0, 0xF0);
    let center = scene_by_name("plains");
    // 播种 → 缓存 Some。
    sched.submit_incremental(job(&center, &side), false, Rows::ALL, None);
    let done = wait_one(&mut results);
    assert!(done.products.is_some());
    assert!(sched.product_of(POS).is_some());
    // 直算派发：提交后缓存必须为 None（下一笔绝不误用播种快照的产物）。
    sched.submit(job(&center, &side), false);
    assert!(sched.product_of(POS).is_none(), "直算派发即作废");
    let done = wait_one(&mut results);
    assert!(done.products.is_none());
    assert_eq!(done.work, TOTAL_SCANNED_ROWS + SECTION_COUNT);
    assert!(sched.product_of(POS).is_none());
    // kill：播种后不 drain 直接 kill —— 僵尸拒收且缓存已空。
    sched.submit_incremental(job(&center, &side), false, Rows::ALL, None);
    results.kill(POS);
    assert!(sched.product_of(POS).is_none());
    // 等回传落进通道（worker send 成功后置 pending=1），再 drain：
    // kill 地板（seq ≤ 当前 seq）必须全拒收。
    let mut spun = 0usize;
    while results.pending_done() == 0 && spun < 30_000 {
        spun += 1;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(spun < 30_000, "worker 超时未回传");
    let got = results.drain_committed();
    assert!(got.is_empty(), "kill 地板后的产物必须被拒收");
    assert!(sched.product_of(POS).is_none(), "僵尸产物不得入缓存");
}

/// 回滚开关纯函数面（对齐 worker_count_from 测试先例，env 不触真实环境）。
#[test]
fn subchunk_rollback_switch_semantics() {
    assert!(!subchunk_enabled_from(Some("0")));
    assert!(subchunk_enabled_from(None));
    assert!(subchunk_enabled_from(Some("1")));
    // 显式 0 才关（对齐 Integer.parseInt 不 trim 语义先例）。
    assert!(subchunk_enabled_from(Some(" 0")));
    assert!(subchunk_enabled_from(Some("x")));
}
