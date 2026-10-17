//! 实体 NBT 编解码三面测试（#222 批 A）：
//! 1. 字段 NBT 回环等价（12 官方键逐个点名 + gzip 字节域往返）；
//! 2. 未知容错（未知 tag 跳过、缺键走 26.1 恢复默认、未知 id 跳过）；
//! 3. 玩家面（playerdata 判别与运动/生存字段回环）。

use glam::Vec3;
use mcv_entity::defs::MobId;
use mcv_entity::nbt::{
    EntityNbt, PLAYER_ID, entity_from_nbt, entity_to_nbt, entity_uuid, mob_vanilla_id,
};
use mcv_nbt::{NbtCompound, NbtList, NbtValue, read_gzip, write_gzip};

/// Pos/Motion 面三联 Double 表。
fn list_d(v: [f64; 3]) -> NbtValue {
    NbtValue::List(NbtList::from_vec(v.map(NbtValue::Double).to_vec()))
}

/// Rotation 面二元 Float 表（YRot 后 XPitch，度）。
fn list_f(v: [f32; 2]) -> NbtValue {
    NbtValue::List(NbtList::from_vec(v.map(NbtValue::Float).to_vec()))
}

fn zombie() -> EntityNbt {
    EntityNbt {
        vanilla_id: mob_vanilla_id(MobId::ZOMBIE),
        pos: Vec3::new(1.5, 70.25, -3.125),
        motion: Vec3::new(0.1, -0.2, 0.03),
        yaw: 2.0,
        pitch: -0.4,
        health: 13.5,
        max_health: 20.0,
        on_ground: true,
        fire: 40,
        air: 300,
        uuid: entity_uuid(42, 7),
    }
}

/// 12 官方键全在、tag 形态正确（键名对齐 26.1 Entity.java:1979-2004 +
/// LivingEntity.java:724）。
#[test]
fn official_keys_and_shapes() {
    let mut c = NbtCompound::new();
    c.insert("unrelated_junk", NbtValue::Long(-7)); // 未知键共存能力
    let m = entity_to_nbt(&zombie());
    for (k, v) in m.iter() {
        c.insert(k, v.clone());
    }
    assert_eq!(c.get_string("id"), Some("minecraft:zombie"));
    let pos = c.get_list("Pos").expect("Pos list");
    assert_eq!(pos.len(), 3);
    assert!(pos.items().iter().all(|v| matches!(v, NbtValue::Double(_))));
    let rot = c.get_list("Rotation").expect("Rotation list");
    assert_eq!(rot.len(), 2);
    assert!(rot.items().iter().all(|v| matches!(v, NbtValue::Float(_))));
    assert!(matches!(c.get("fall_distance"), Some(NbtValue::Double(_))));
    assert!(matches!(c.get("Fire"), Some(NbtValue::Short(40))));
    assert!(matches!(c.get("Air"), Some(NbtValue::Short(300))));
    assert!(matches!(c.get("OnGround"), Some(NbtValue::Byte(1))));
    assert!(matches!(c.get("Invulnerable"), Some(NbtValue::Byte(0))));
    assert!(matches!(c.get("PortalCooldown"), Some(NbtValue::Int(0))));
    assert_eq!(c.get_int_array("UUID"), Some(&entity_uuid(42, 7)[..]));
    assert!(matches!(
        c.get("Health"),
        Some(NbtValue::Float(f) if f == 13.5)
    ));
}

/// 字段 NBT 回环等价（含 gzip 字节域）：写 → 压缩 → 解压 → 读，全部
/// 业务字段原样（旋转面按度存储允许 1e-4 rad 量级浮点误差——原版同以
/// 度存 Rotation，非位级 regret）。
#[test]
fn roundtrip_through_gzip_preserves_fields() {
    let e = zombie();
    let bytes = write_gzip(&entity_to_nbt(&e)).expect("gzip write");
    let back = entity_from_nbt(&read_gzip(&bytes).expect("gzip read")).expect("zombie loads");
    assert_eq!(back.mob, Some(MobId::ZOMBIE));
    assert_eq!(back.pos, e.pos);
    assert_eq!(back.motion, e.motion);
    assert_eq!(back.health, e.health);
    assert!(back.on_ground);
    assert_eq!(back.fire, 40);
    assert_eq!(back.air, 300);
    let yaw_err = (back.yaw - e.yaw).abs();
    let pitch_err = (back.pitch - e.pitch).abs();
    assert!(yaw_err < 1e-4, "yaw {yaw_err}");
    assert!(pitch_err < 1e-4, "pitch {pitch_err}");
}

/// 未知 tag 跳过 + 缺键走 26.1 恢复默认（Entity.load orElse 面）：Health
/// → getMaxHealth、Motion→ZERO、Rotation→ZERO（YRot 0=南）、OnGround→false、
/// Fire→0、Air→300。
#[test]
fn unknown_tags_skipped_missing_keys_defaulted() {
    let mut c = NbtCompound::new();
    c.insert("id", NbtValue::String("minecraft:cow".into()));
    // 未知键：标量/嵌套/列表三形态照收不理（ValueInput 按键取值面）。
    c.insert("SkyHook", NbtValue::Long(777));
    let mut nest = NbtCompound::new();
    nest.insert("a", NbtValue::Byte(1));
    c.insert("CustomStuff", NbtValue::Compound(nest));
    let mut tags = NbtList::new();
    tags.push(NbtValue::String("x".into())).expect("push");
    c.insert("Tags", NbtValue::List(tags));
    // Pos 给全，Motion/Rotation/Health 等全缺。
    c.insert("Pos", list_d([8.0, 64.5, -10.25]));

    let back = entity_from_nbt(&c).expect("cow loads");
    assert_eq!(back.mob, Some(MobId::COW));
    assert_eq!(back.pos, Vec3::new(8.0, 64.5, -10.25));
    assert_eq!(back.motion, Vec3::ZERO, "缺 Motion → ZERO（orElse 面）");
    assert!(
        (back.yaw - std::f32::consts::PI).abs() < 1e-6,
        "缺 Rotation → YRot 0（南）→ 本仓半圈差 = π 朝南"
    );
    assert_eq!(back.pitch, 0.0);
    assert_eq!(back.health, 10.0, "缺 Health → cow getMaxHealth");
    assert!(!back.on_ground);
    assert_eq!(back.fire, 0);
    assert_eq!(back.air, 300);
}

/// 恶意/损坏输入的恢复面：未知 id 跳过、非 finite 坐标不入表、速度分量
/// |v|>10 归 0（Entity.java:2068-2072）、形状坏的 Pos → ZERO、坐标钳制。
#[test]
fn corrupt_and_hostile_input_contained() {
    let mk = |id: &str, pos: Option<[f64; 3]>, motion: Option<[f64; 3]>| {
        let mut c = NbtCompound::new();
        c.insert("id", NbtValue::String(id.into()));
        if let Some(p) = pos {
            c.insert("Pos", list_d(p));
        }
        if let Some(m) = motion {
            c.insert("Motion", list_d(m));
        }
        c
    };
    // 未知 id → None（26.1：EntityType.by 失败实体跳过）。
    assert!(entity_from_nbt(&mk("minecraft:ender_dragon", Some([1.0, 64.0, 1.0]), None)).is_none());
    // 缺 id → None。
    assert!(entity_from_nbt(&mk("", Some([1.0, 64.0, 1.0]), None)).is_none());
    // 非 finite Pos → None（原版 IllegalStateException 面）。
    assert!(entity_from_nbt(&mk("minecraft:pig", Some([f64::NAN, 64.0, 1.0]), None)).is_none());
    // 形状坏 Pos（2 元素）→ orElse ZERO。
    let short = mk("minecraft:pig", None, None);
    let back = entity_from_nbt(&short).expect("缺 Pos 回退 ZERO");
    assert_eq!(back.pos, Vec3::ZERO);
    // 速度分量超 10 → 归 0（防瞬移）。
    let fast = mk(
        "minecraft:pig",
        Some([1.0, 64.0, 1.0]),
        Some([24.0, 0.0, -0.5]),
    );
    let back = entity_from_nbt(&fast).expect("速度超界实体仍加载");
    assert_eq!(back.motion, Vec3::new(0.0, 0.0, -0.5));
    // 坐标钳制到 26.1 世界域。
    let far = mk("minecraft:pig", Some([4.0e8, 5.0e7, -4.0e8]), None);
    let back = entity_from_nbt(&far).expect("越界坐标钳制加载");
    assert_eq!(back.pos.x, 3.000_051_2e7_f32);
    assert_eq!(back.pos.y, 2.0e7_f32);
    assert_eq!(back.pos.z, -3.000_051_2e7_f32);
    // 形状坏 Rotation（1 元）→ orElse(Vec2.ZERO) 同缺键：YRot 0=南 →
    // 本仓半圈差 yaw≈π（与缺键路径汇合，不允许 North 偏差）。
    let mut badrot = mk("minecraft:pig", Some([1.0, 64.0, 1.0]), None);
    badrot.insert("Rotation", list_f([90.0]));
    let back = entity_from_nbt(&badrot).expect("坏形状 Rotation 仍加载");
    assert!(
        (back.yaw - std::f32::consts::PI).abs() < 1e-6,
        "yaw {}",
        back.yaw
    );
    assert_eq!(back.pitch, 0.0);
}

/// 玩家面：`minecraft:player` 判别（mob=None）、俯仰符号反转（本仓 +上
/// ↔ 原版 XPitch +下）、缺 Health → 20。
#[test]
fn player_face_roundtrip() {
    let mut e = EntityNbt::player();
    e.pos = Vec3::new(0.5, 71.0, 2.5);
    e.yaw = -1.2;
    e.pitch = 0.8;
    e.health = 17.0;
    e.air = 250;
    e.on_ground = true;
    let c = entity_to_nbt(&e);
    assert_eq!(c.get_string("id"), Some(PLAYER_ID));
    // 原版 XPitch = 本仓 -pitch（+下坐标系）。
    let rot = c.get_list("Rotation").expect("rot");
    assert!(
        matches!(rot.get(1), Some(NbtValue::Float(p)) if (*p + 0.8f32.to_degrees()).abs() < 1e-4)
    );
    let back = entity_from_nbt(&c).expect("player loads");
    assert_eq!(back.mob, None, "玩家不是一个 mob");
    assert_eq!(back.pos, e.pos);
    assert_eq!(back.health, 17.0);
    assert_eq!(back.air, 250);
    assert!(back.on_ground);
    assert!((back.yaw - e.yaw).abs() < 1e-4);
    assert!((back.pitch - e.pitch).abs() < 1e-4);
    // 缺 Health 的玩家档 → 20.0 满值回退。
    let mut bare = NbtCompound::new();
    bare.insert("id", NbtValue::String(PLAYER_ID.into()));
    bare.insert("Pos", list_d([0.0, 70.0, 0.0]));
    let back = entity_from_nbt(&bare).expect("玩家缺 Health 走满值");
    assert_eq!(back.health, 20.0);
}

/// UUID 合成的确定性：同 (seed, idx) 同 uuid，slot/seed 变则变。
#[test]
fn uuid_deterministic_per_slot() {
    assert_eq!(entity_uuid(99, 3), entity_uuid(99, 3));
    assert_ne!(entity_uuid(99, 3), entity_uuid(99, 4));
    assert_ne!(entity_uuid(98, 3), entity_uuid(99, 3));
}

/// 空 Entities 列表的根 compound 走 gzip 往返（region 档壳）。
#[test]
fn entities_root_shell_roundtrip() {
    let mut root = NbtCompound::new();
    root.insert(
        "Entities",
        NbtValue::List(NbtList::from_vec(vec![entity_to_nbt(&zombie())])),
    );
    let bytes = write_gzip(&root).expect("write");
    let back = read_gzip(&bytes).expect("read");
    let l = back.get_list("Entities").expect("Entities");
    assert_eq!(l.len(), 1, "单实体壳往返");
    let item = l.get(0).and_then(|v| v.as_compound()).expect("compound");
    assert_eq!(item.get_string("id"), Some("minecraft:zombie"));
}
