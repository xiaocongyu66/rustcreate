//! 实体落盘专测（#222 批 A）：EntitiesTag 写侧（save_entities）+ 世界
//! 加载 mob 表回填（load_meta→restore_entities）+ playerdata 镜像面。
//! 共用 headless_rt 无头装配（对照 weather_tests）。

use super::*;
use mcv_nbt::{read_gzip, write_gzip};

/// 表面行：(id, pos, health)——回填等价的断言视图。
fn mob_rows(rt: &GameRuntime) -> Vec<(MobId, Vec3, f32)> {
    let kinds = rt.mobs_app.world.read::<MobKind>();
    let bodies = rt.mobs_app.world.read::<PhysBody>();
    let healths = rt.mobs_app.world.read::<Health>();
    let mut rows: Vec<_> = kinds
        .iter()
        .filter_map(|(e, k)| {
            let b = bodies.get(e)?;
            Some((k.0, b.pos, healths.get(e).map(|h| h.0).unwrap_or(0.0)))
        })
        .collect();
    rows.sort_by_key(|r| (r.0.0, (r.1.x * 8.0) as i32));
    rows
}

/// 存档恢复：两只怪（敌对 + 被动，跨两个区块域）写实体域档 → 全新
/// runtime 指向同一存档目录 load_meta → mob 表回填逐字段等价。
#[test]
fn mobs_survive_save_reload() {
    let mut rt = headless_rt("ent-save");
    let z = spawn_mob(
        &mut rt.mobs_app.world,
        MobId::ZOMBIE,
        Vec3::new(1.5, 70.0, -3.25),
    );
    // +20 格 → 不同区块域（r.1.0），域分文件面同时有证。
    let _c = spawn_mob(
        &mut rt.mobs_app.world,
        MobId::COW,
        Vec3::new(20.5, 64.0, 8.25),
    );
    *rt.mobs_app.world.write::<Health>().get_mut(z).unwrap() = Health(7.5);
    *rt.mobs_app.world.write::<Yaw>().get_mut(z).unwrap() = Yaw(1.25);
    rt.mobs_app
        .world
        .write::<PhysBody>()
        .get_mut(z)
        .unwrap()
        .on_ground = true;
    rt.mobs_app
        .world
        .write::<MobTicks>()
        .get_mut(z)
        .unwrap()
        .fire_ticks = 33;
    rt.save_entities();
    // 域档两份落盘（僵尸 r.0.0、牛 r.1.0）+ 玩家镜像。
    assert!(rt.save_dir.join("entities/r.0.0.mcrve").exists());
    assert!(rt.save_dir.join("entities/r.1.0.mcrve").exists());
    assert!(rt.save_dir.join("playerdata/player.dat").exists());

    // 模拟重进：全新 runtime（mob 表从零）同一目录 load_meta。
    let seed = rt.seed;
    let dir = rt.save_dir.clone();
    let mut rt2 = GameRuntime::new_headless(seed, dir, GameMode::Survival);
    assert_eq!(mob_rows(&rt2).len(), 0, "新 runtime 零怪起步");
    rt2.load_meta();
    let rows = mob_rows(&rt2);
    assert_eq!(rows.len(), 2, "两种 id 各一只随档回填");
    let zr = rows
        .iter()
        .find(|r| r.0 == MobId::ZOMBIE)
        .expect("僵尸回填");
    let cr = rows.iter().find(|r| r.0 == MobId::COW).expect("牛回填");
    assert_eq!(zr.1, Vec3::new(1.5, 70.0, -3.25));
    assert_eq!(zr.2, 7.5, "低血量随档（不回满）");
    assert_eq!(cr.1, Vec3::new(20.5, 64.0, 8.25));
    assert_eq!(cr.2, 10.0);
    // 朝向/着地/火账逐字段等价（经 world 组件读）。
    let e = rt2
        .mobs_app
        .world
        .read::<MobKind>()
        .iter()
        .find(|(_, k)| k.0 == MobId::ZOMBIE)
        .map(|(e, _)| e)
        .expect("僵尸实体");
    let yaw = rt2.mobs_app.world.read::<Yaw>().get(e).expect("yaw").0;
    assert!((yaw - 1.25).abs() < 1e-4, "弧度朝向随档 {yaw}");
    // RefMut 是语句级临时——get 的引用必须先落守卫绑定再断言
    //（E0716：临时值语句末即释放）。
    let bodies = rt2.mobs_app.world.read::<PhysBody>();
    let body = bodies.get(e).expect("body");
    assert!(body.on_ground);
    assert_eq!(body.vel, Vec3::ZERO, "静止怪 Motion 归零随档");
    let ticks_guard = rt2.mobs_app.world.read::<MobTicks>();
    let ticks = ticks_guard.get(e).expect("ticks");
    assert_eq!(ticks.fire_ticks, 33, "燃烧账随档");
    // 幂等：重复 load_meta 不翻倍。
    rt2.load_meta();
    assert_eq!(mob_rows(&rt2).len(), 2, "重复载入不产生副本");
}

/// 玩家镜像面：playerdata/player.dat 是 save_entities 同拍的 NBT 口径
/// （id=minecraft:player、Pos/Health/Air 取值等价）；level.meta 仍是恢复
/// 权威（player.dat 只镜像不反哺——双源回退会打架，登记 KNOWN-DIVERGENCE）。
#[test]
fn player_nbt_mirror_written_with_entities_beat() {
    let mut rt = headless_rt("ent-player");
    rt.player.pos = Vec3::new(0.75, 71.5, 4.5);
    rt.player.health = 12.0;
    rt.air_supply = 250;
    rt.save_entities();
    let bytes = std::fs::read(rt.save_dir.join("playerdata/player.dat")).expect("player.dat");
    let root = read_gzip(&bytes).expect("gzip");
    let back = mcv_entity::nbt::entity_from_nbt(&root).expect("玩家档读回");
    assert_eq!(back.mob, None, "玩家面不是一个 mob");
    assert_eq!(back.pos, rt.player.pos);
    assert_eq!(back.health, 12.0);
    assert_eq!(back.air, 250);
}

/// 迁块回收：mob 从 r.0.0 迁 r.1.0 后再落盘，旧域档必须删除（不残留
/// 双根旧副本——否则重进世界同一只怪两份）。
#[test]
fn moved_mobs_leave_no_stale_region_copy() {
    let mut rt = headless_rt("ent-stale");
    let e0 = spawn_mob(
        &mut rt.mobs_app.world,
        MobId::PIG,
        Vec3::new(0.5, 70.0, 0.5),
    );
    rt.save_entities();
    assert!(rt.save_dir.join("entities/r.0.0.mcrve").exists());
    rt.mobs_app
        .world
        .write::<PhysBody>()
        .get_mut(e0)
        .unwrap()
        .pos
        .x = 17.5;
    rt.save_entities();
    assert!(
        !rt.save_dir.join("entities/r.0.0.mcrve").exists(),
        "迁块后旧域档回收"
    );
    assert!(rt.save_dir.join("entities/r.1.0.mcrve").exists());
}

/// 恢复容错端到端：档内未知 id 跳过、缺 Pos 走 ZERO、缺 Health 走满值
/// （26.1 Entity.load 语义在全链路成立——手拼档直灌）。
#[test]
fn restore_skips_unknown_ids_and_defaults_missing_fields() {
    use mcv_entity::nbt::{EntityNbt, entity_to_nbt};
    let mut rt = headless_rt("ent-tolerant");
    let mut entities = NbtList::new();
    // 一好：完整 zombie。
    entities
        .push(NbtValue::Compound(entity_to_nbt(&EntityNbt {
            vanilla_id: mcv_entity::nbt::mob_vanilla_id(MobId::ZOMBIE),
            pos: Vec3::new(2.0, 65.0, 6.0),
            motion: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            health: 20.0,
            max_health: 20.0,
            on_ground: true,
            fire: 0,
            air: 300,
            uuid: [1; 4],
        })))
        .expect("push");
    // 一坏：未知 id（26.1 EntityType.by 失败 → 跳过不入表）。
    let mut junk = NbtCompound::new();
    junk.insert("id", NbtValue::String("minecraft:ender_dragon".into()));
    junk.insert("Pos", NbtValue::List(NbtList::new()));
    entities.push(NbtValue::Compound(junk)).expect("push");
    // 一缺法：skeleton 只带 id（Pos→ZERO 默认、Health→满值默认）。
    let mut bare = NbtCompound::new();
    bare.insert("id", NbtValue::String("minecraft:skeleton".into()));
    entities.push(NbtValue::Compound(bare)).expect("push");

    let mut root = NbtCompound::new();
    root.insert("Entities", NbtValue::List(entities));
    std::fs::create_dir_all(rt.save_dir.join("entities")).unwrap();
    std::fs::write(
        rt.save_dir.join("entities/r.0.0.mcrve"),
        write_gzip(&root).expect("gzip"),
    )
    .unwrap();

    rt.load_meta();
    let rows = mob_rows(&rt);
    assert_eq!(rows.len(), 2, "未知 id 跳过，好实体 + 缺法实体入表");
    assert!(
        rows.iter()
            .any(|r| r.0 == MobId::ZOMBIE && r.1 == Vec3::new(2.0, 65.0, 6.0))
    );
    let sk = rows
        .iter()
        .find(|r| r.0 == MobId::SKELETON)
        .expect("skeleton");
    assert_eq!(sk.1, Vec3::ZERO, "缺 Pos → ZERO（orElse 面）");
    assert_eq!(sk.2, 20.0, "缺 Health → skeleton getMaxHealth");
    // 坏档不影响再落盘：save_entities 重写同一域档（含账面回填）。
    rt.save_entities();
    assert!(rt.save_dir.join("entities/r.0.0.mcrve").exists());
}
