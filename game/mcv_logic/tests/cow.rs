//! E6 一致性锁：ECS 资源快照去克隆（COW，engine-roadmap.md §3-E6）。
//!
//! 固定步喂 ECS 的 `MobServices`/`DropWorld` 快照在 E6 前是两次**整表
//! HashMap 克隆**（每步逐节点复制，rd16 下 ~30 万表节点/s）；E6 后
//! `GameRuntime.chunks` 外层 `Arc` 化，快照 = 一次计数递增，主线程写路径
//! （stream 的插入/删除，流式事件级频率）走 [`GameRuntime::chunks_mut`]
//! 写时复刻（COW）。本文件锁四个时机：
//!
//! 1. 快照零拷贝：fixed_step 后 Resources 里的快照与主表共享同一 Arc；
//! 2. 无写稳态：主表分配不因快照更替而换（读路径永不复刻）；
//! 3. 写时复刻：快照存活时插入/删除使主表 Arc 指针变化，旧快照冻结在
//!    写前状态（读路径持旧快照安全），下一固定步快照跟上主表；
//! 4. 无共享原地写：无快照存活时写路径不复制整表。

use std::sync::Arc;

use glam::Vec3;
use mcv_core::{ChunkHandle, ChunkPos, Stage};
use mcv_entity::DropWorld;
use mcv_logic::game::{GameMode, GamePhase, GameRuntime, MobServices};

/// 唯一临时存档目录（镜像 game.rs `headless_rt` 的命名纪律）。
fn tmp_world(tag: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
    std::env::temp_dir().join(format!("mcv-cow-{}-{:x}", tag, h.finish()))
}

/// 拼一个已到 Uploaded 阶段的中心区块 + 玩家落位（fixed_step 可驱动）。
fn rt_with_center_chunk(tag: &str) -> GameRuntime {
    let mut rt = GameRuntime::new_headless(20261012, tmp_world(tag), GameMode::Survival);
    rt.phase = GamePhase::Playing;
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(0, 0)));
    h.advance_to(Stage::Uploaded);
    rt.chunks_mut().insert(ChunkPos::new(0, 0), h);
    rt.player.pos = Vec3::new(8.5, 70.0, 8.5);
    rt
}

/// 锁 1：快照零拷贝——`MobServices`/`DropWorld`/`AquaticServices` 与主表
/// 共享同一 Arc（E6 前这里是每步两次逐节点整表克隆；指针同一即证明克隆
/// 消除，且强计数 = 主表 + 三份快照 = 4，多一份/少一份都说明快照语义
/// 变了——M7c 起水生系统同快照注入，AquaticServices.chunks 同主表 Arc）。
#[test]
fn resource_snapshots_share_arc_with_main_table() {
    let mut rt = rt_with_center_chunk("share");
    rt.fixed_step(1.0 / 60.0);
    assert_eq!(
        Arc::strong_count(&rt.chunks),
        4,
        "主表 + MobServices 快照 + DropWorld 快照 + AquaticServices 快照 = 恰 4 份 Arc 持有"
    );
    let ms = rt
        .mobs_app
        .resources
        .get::<MobServices>()
        .expect("fixed_step 必注入 MobServices");
    assert!(
        Arc::ptr_eq(&ms.chunks, &rt.chunks),
        "MobServices 快照必须与主表共享同一 Arc（零拷贝）"
    );
    let dw = rt
        .mobs_app
        .resources
        .get::<DropWorld>()
        .expect("fixed_step 必注入 DropWorld");
    assert!(
        Arc::ptr_eq(&dw.chunks, &rt.chunks),
        "DropWorld 快照必须与主表共享同一 Arc"
    );
    assert!(
        Arc::ptr_eq(&ms.chunks, &dw.chunks),
        "两次快照 insert 互为同一 Arc（E6 前是两份独立整表）"
    );
}

/// 锁 2：无写稳态——固定步只读消费，主表分配不换。与锁 1 合起来钉死
/// 「零拷贝读」的实现形状：快照与主表同指针且指针跨步恒定 = 每步纯
/// 计数递增，rd16 下 ~30 万表节点/s 的克隆源被消除。
#[test]
fn steady_state_steps_keep_main_table_allocation() {
    let mut rt = rt_with_center_chunk("steady");
    rt.fixed_step(1.0 / 60.0);
    let p0 = Arc::as_ptr(&rt.chunks);
    for _ in 0..3 {
        rt.fixed_step(1.0 / 60.0);
    }
    assert_eq!(
        p0,
        Arc::as_ptr(&rt.chunks),
        "只读固定步不得更换主表分配（快照 insert 是计数递增）"
    );
}

/// 锁 3：写时复刻时机——快照存活时插入/删除使主表 Arc 换指针；旧快照
/// 冻结在写前状态（持旧快照的读路径安全）；下一固定步快照跟上主表
/// （「insert 时点快照」既有语义不变）。
#[test]
fn cow_write_replicates_table_only_while_snapshot_alive() {
    let mut rt = rt_with_center_chunk("cow");
    rt.fixed_step(1.0 / 60.0); // 快照进入 Resources（存活）
    let frozen = rt
        .mobs_app
        .resources
        .get::<MobServices>()
        .expect("第一步必注入快照")
        .chunks
        .clone(); // Arc 计数递增，非表克隆

    // 插入：主表换指针（复刻），旧快照冻结。
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(1, 0)));
    h.advance_to(Stage::Uploaded);
    rt.chunks_mut().insert(ChunkPos::new(1, 0), h);
    assert!(
        !Arc::ptr_eq(&frozen, &rt.chunks),
        "快照存活时写路径必须写时复刻（主表 Arc 指针变化）"
    );
    assert!(
        frozen.contains_key(&ChunkPos::new(0, 0)) && !frozen.contains_key(&ChunkPos::new(1, 0)),
        "旧快照必须冻结在写前状态（持旧快照读安全）"
    );
    assert!(rt.chunks.contains_key(&ChunkPos::new(1, 0)), "主表含增量");

    // 下一固定步：快照跟上主表。
    rt.fixed_step(1.0 / 60.0);
    let ms = rt
        .mobs_app
        .resources
        .get::<MobServices>()
        .expect("第二步必注入快照");
    assert!(Arc::ptr_eq(&ms.chunks, &rt.chunks), "写后快照与主表同 Arc");
    assert!(ms.chunks.contains_key(&ChunkPos::new(1, 0)), "快照见增量");

    // 删除：同样走写时复刻（冻结语义对称）。
    let frozen2 = ms.chunks.clone();
    rt.chunks_mut().remove(&ChunkPos::new(1, 0));
    assert!(
        !Arc::ptr_eq(&frozen2, &rt.chunks),
        "删除同样写时复刻（Arc 指针变化）"
    );
    assert!(
        frozen2.contains_key(&ChunkPos::new(1, 0)),
        "删除后旧快照仍含该块（冻结语义对称）"
    );
    assert!(!rt.chunks.contains_key(&ChunkPos::new(1, 0)));
}

/// 锁 5：票表快照零拷贝（E6 同族收口，本波）——`MobServices.tickets`
/// 在收口前是每固定步一次整表克隆（TicketTable HashMap 逐节点 + 每块
/// 票 Vec）；收口后 = 一次计数递增。同时锁「变更才重建」：玩家锚点
/// （center, sim_dist）不变时票表分配跨帧恒定（每帧全量重建 → 变更
/// 重建，等价原版 addPlayer/removePlayer/updateViewDistance 才动票表，
/// DistanceManager.java:110-117,:152-157），锚点迁移则换分配且旧快照
/// 冻结（读路径持旧快照安全，级别场与逐帧重建逐块等价）。
#[test]
fn tickets_snapshot_shares_arc_and_rebuilds_only_on_change() {
    let mut rt = rt_with_center_chunk("tickets");
    rt.stream();
    let p0 = std::sync::Arc::as_ptr(&rt.tickets);
    rt.stream();
    assert_eq!(
        p0,
        std::sync::Arc::as_ptr(&rt.tickets),
        "锚点不变的稳态帧不得更换票表分配（每帧全量重建已废止）"
    );
    rt.fixed_step(1.0 / 60.0);
    let ms_tickets = rt
        .mobs_app
        .resources
        .get::<MobServices>()
        .expect("fixed_step 必注入 MobServices")
        .tickets
        .clone(); // Arc 计数递增，非表克隆
    assert!(
        std::sync::Arc::ptr_eq(&ms_tickets, &rt.tickets),
        "票表快照必须与主表共享同一 Arc（每步整表克隆已消除）"
    );
    // 「变更才重建 vs 恒重建」判别锁：快照存活（主表 Arc 强计数 ≥ 2）时
    // 锚点不变的稳态帧仍不换分配——恒 make_mut 的实现在这里必然换分配。
    rt.stream();
    assert!(
        std::sync::Arc::ptr_eq(&ms_tickets, &rt.tickets),
        "快照存活期稳态帧不得更换票表分配（变更才重建，非每帧复刻）"
    );
    // 语义保持：级别场 = 锚点解析式（ticket.rs 同式），快照读安全。
    let vd = rt.sim_dist();
    let sim_level = (mcv_logic::ticket::ENTITY_TICKING_LEVEL - vd).max(0);
    assert_eq!(
        ms_tickets.level(ChunkPos::new(0, 0)) as i32,
        sim_level,
        "快照内玩家块级别 = 模拟锚点级别（min-ticket）"
    );
    // 锚点迁移（跨区块边界）→ 变更重建：主表换新分配，旧快照冻结。
    rt.player.pos = Vec3::new(40.5, 70.0, 8.5);
    rt.stream();
    assert!(
        !std::sync::Arc::ptr_eq(&ms_tickets, &rt.tickets),
        "锚点迁移必须重建票表（换分配；旧快照冻结在新锚点之外）"
    );
    assert_eq!(
        ms_tickets.level(ChunkPos::new(2, 0)) as i32,
        (sim_level + 2).min(mcv_logic::ticket::NOT_LOADED as i32),
        "旧快照冻结在写前锚点（持旧快照读安全）"
    );
    assert_eq!(
        rt.tickets.level(ChunkPos::new(2, 0)) as i32,
        sim_level,
        "新表按新锚点 (2,0) 重建：玩家块级别 = 模拟锚点级别"
    );
}

/// 锁 4：无共享时写路径原地变（COW 另一面）：Resources 无快照存活，
/// `chunks_mut` 连续写不换主表分配（零复制）。
#[test]
fn write_in_place_when_no_snapshot_alive() {
    let mut rt = GameRuntime::new_headless(20261012, tmp_world("inplace"), GameMode::Survival);
    rt.phase = GamePhase::Playing;
    assert!(
        rt.mobs_app.resources.get::<MobServices>().is_none(),
        "前置：未跑固定步，快照不存在"
    );
    assert_eq!(Arc::strong_count(&rt.chunks), 1, "主表唯一持有");
    let p0 = Arc::as_ptr(&rt.chunks);
    let h = Arc::new(ChunkHandle::new(ChunkPos::new(0, 0)));
    h.advance_to(Stage::Uploaded);
    rt.chunks_mut().insert(ChunkPos::new(0, 0), h);
    rt.chunks_mut().insert(
        ChunkPos::new(1, 0),
        Arc::new(ChunkHandle::new(ChunkPos::new(1, 0))),
    );
    assert_eq!(
        p0,
        Arc::as_ptr(&rt.chunks),
        "无快照存活时写必须原地（Arc::make_mut 引用计数 1 不复制）"
    );
}
