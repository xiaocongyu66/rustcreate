//! mcv_ecs:实体代语义、despawn 级联清理、分表借用、sparse-set 迭代。
use std::sync::atomic::{AtomicUsize, Ordering};

use mcv_ecs::{Entity, World};

#[derive(Clone, Copy, PartialEq, Debug)]
struct Pos(f32);
#[derive(Clone, Copy, PartialEq, Debug)]
struct Vel(f32);
#[derive(Clone, Copy)]
struct Hp(i32);

#[test]
fn spawn_insert_get_roundtrip() {
    let mut w = World::new();
    let a = w.spawn();
    let b = w.spawn();
    w.insert(a, Pos(1.0));
    w.insert(b, Pos(2.0));
    w.insert(b, Hp(10));
    assert_eq!(w.get_ref::<Pos>(a).unwrap().0, 1.0);
    assert_eq!(w.get_ref::<Pos>(b).unwrap().0, 2.0);
    assert!(w.get_ref::<Hp>(a).is_none(), "未插入的组件返回 None");
    assert_eq!(w.len(), 2);
    assert_eq!(w.component_count::<Pos>(), 2);
    assert_eq!(w.component_count::<Hp>(), 1);
}

#[test]
fn despawn_invalidates_handle_and_reuses_slot_with_new_gen() {
    let mut w = World::new();
    let a = w.spawn();
    w.insert(a, Pos(1.0));
    assert!(w.is_alive(a));
    assert!(w.despawn(a));
    assert!(!w.is_alive(a), "despawn 后句柄应失效");
    assert!(!w.despawn(a), "重复 despawn 返回 false");
    assert!(w.get_ref::<Pos>(a).is_none(), "死句柄读不到组件");

    // 槽位复用:新实体落在同一 idx,但 gen +1,旧句柄仍无效。
    let c = w.spawn();
    assert_eq!(c.idx(), a.idx());
    assert_ne!(c.gen(), a.gen());
    w.insert(c, Pos(9.0));
    assert!(w.get_ref::<Pos>(a).is_none());
    assert_eq!(w.get_ref::<Pos>(c).unwrap().0, 9.0);
}

/// despawn 必须把实体在所有组件表中的值 Drop 干净(不泄漏)。
#[test]
fn despawn_drops_components_in_all_tables() {
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    struct Probe;
    impl Probe {
        fn new() -> Self {
            LIVE.fetch_add(1, Ordering::SeqCst);
            Probe
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }

    let mut w = World::new();
    let e = w.spawn();
    w.insert(e, Probe::new());
    w.insert(e, Pos(1.0));
    assert_eq!(LIVE.load(Ordering::SeqCst), 1);
    w.despawn(e);
    assert_eq!(LIVE.load(Ordering::SeqCst), 0, "despawn 未排空组件");
    assert_eq!(w.component_count::<Probe>(), 0);
}

/// 覆盖插入:旧值必须被 Drop。
#[test]
fn insert_overwrite_drops_old_value() {
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    struct Probe;
    impl Probe {
        fn new() -> Self {
            LIVE.fetch_add(1, Ordering::SeqCst);
            Probe
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let mut w = World::new();
    let e = w.spawn();
    w.insert(e, Probe::new());
    w.insert(e, Probe::new());
    assert_eq!(
        LIVE.load(Ordering::SeqCst),
        1,
        "覆盖插入应 Drop 旧值只剩 1 个"
    );
    w.despawn(e);
    assert_eq!(LIVE.load(Ordering::SeqCst), 0);
}

/// 多表视图并行借用 + for_each 系统模式(写一表读他表)。
#[test]
fn multi_table_system_pattern() {
    let mut w = World::new();
    let (a, b, c) = (w.spawn(), w.spawn(), w.spawn());
    for e in [a, b, c] {
        w.insert(e, Pos(0.0));
    }
    w.insert(a, Vel(1.0));
    w.insert(c, Vel(3.0));
    w.insert(b, Hp(5));

    // 系统:pos += vel(vel 可选),跳过有 Hp 的实体。
    {
        let (mut pos, vel, hp) = (w.write::<Pos>(), w.read::<Vel>(), w.read::<Hp>());
        pos.for_each(|e, p| {
            if hp.get(e).is_some() {
                return;
            }
            if let Some(v) = vel.get(e) {
                p.0 += v.0;
            }
        });
    }
    assert_eq!(w.get_ref::<Pos>(a).unwrap().0, 1.0);
    assert_eq!(w.get_ref::<Pos>(b).unwrap().0, 0.0, "Hp 实体被跳过");
    assert_eq!(w.get_ref::<Pos>(c).unwrap().0, 3.0);
    assert_eq!(w.get_ref::<Pos>(a).unwrap().0, 1.0);

    // 表视图句柄语义:迭代顺序 = dense 插入序,内容 = sparse 现值。
    let snapshot: Vec<(Entity, f32)> = w.read::<Pos>().iter().map(|(e, p)| (e, p.0)).collect();
    assert_eq!(snapshot.len(), 3);
}

/// 空洞迭代:中间实体 despawn 后,迭代只含存活组件且无空洞。
#[test]
fn iteration_skips_holes_after_despawn() {
    let mut w = World::new();
    let ids: Vec<Entity> = (0..5)
        .map(|i| {
            let e = w.spawn();
            w.insert(e, Pos(i as f32));
            e
        })
        .collect();
    w.despawn(ids[1]);
    w.despawn(ids[3]);
    let vals: Vec<f32> = w.read::<Pos>().iter().map(|(_, p)| p.0).collect();
    assert_eq!(vals.len(), 3);
    assert!(vals.contains(&0.0) && vals.contains(&2.0) && vals.contains(&4.0));
    assert_eq!(w.component_count::<Pos>(), 3);
}

#[test]
fn remove_returns_component() {
    let mut w = World::new();
    let e = w.spawn();
    w.insert(e, Hp(7));
    assert_eq!(w.remove::<Hp>(e).unwrap().0, 7);
    assert!(w.remove::<Hp>(e).is_none(), "二次移除 None");
    assert!(w.get_ref::<Hp>(e).is_none());
    // 死句柄 remove 也返回 None 且不动表(本世界 spawn+despawn 得到
    // 世代已作废的合法构造句柄)。
    let ghost = w.spawn();
    w.despawn(ghost);
    assert!(w.remove::<Hp>(ghost).is_none());
    assert_eq!(w.component_count::<Hp>(), 0);
}

/// 已注册但从未插入的类型 = 空表视图;read/write 混用不同表合法。
#[test]
fn registered_empty_read_and_mixed_borrows_ok() {
    let mut w = World::new();
    let e = w.spawn();
    w.insert(e, Pos(1.0));
    w.register::<Vel>();
    // Vel 注册了但从未插入:read 得空表。
    assert!(w.read::<Vel>().is_empty());
    // 同时持 Pos 写视图 + Vel 读视图(都是 &self 借用、不同 RefCell,合法)。
    {
        let (mut pos, _vel) = (w.write::<Pos>(), w.read::<Vel>());
        pos.for_each(|_, p| p.0 += 100.0);
    }
    assert_eq!(w.get_ref::<Pos>(e).unwrap().0, 101.0);
}

/// 未注册类型 read/write panic(契约:防静默读空表掩盖漏注册)。
#[test]
#[should_panic(expected = "未注册")]
fn reading_unregistered_panics() {
    let w = World::new();
    drop(w.read::<Vel>());
}
