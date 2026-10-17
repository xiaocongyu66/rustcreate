//! 实体 ↔ NBT 编解码（#222 批 A）：26.1 `Entity.saveWithoutId`/`Entity.load`
//! 的官方键名面（`id`/`Pos`/`Motion`/`Rotation`/`fall_distance`/`Fire`/`Air`/
//! `OnGround`/`Invulnerable`/`PortalCooldown`/`UUID` + LivingEntity 的
//! `Health`，Entity.java:1979-2004、LivingEntity.java:724）。
//!
//! 恢复语义逐条对齐原版 Entity.load（:2064-2096）：**缺键走默认**（Pos/
//! Motion→ZERO、Rotation→ZERO、Fire→0、Air→getMaxAirSupply(300)、
//! OnGround→false、Health→getMaxHealth(LivingEntity.java:791)）、**未知 tag
//! 跳过**（ValueInput 只按键取值）与**未知实体 id 跳过不入表**（EntityType
//! by() 失败 → 实体不加载）。坐标域钳制（x/z 3.0000512E7、y 2E7）与速度
//! 分类（分量 |v|>10 → 0）同原版。差异登记 docs/KNOWN-DIVERGENCE.md
//! 「ENT-NBT」条目（常量填充键、UUID 合成、yaw 度映射、玩家档带 id）。
//!
//! codec 零文件 IO（落盘编排归 GameRuntime），只认 [`EntityNbt`] 快照 ↔
//! tag 树；ECS 类型不进本模块。

use crate::ai::wrap_angle;
use crate::defs::{MOBS, MobId};
use glam::Vec3;
use mcv_nbt::{NbtCompound, NbtList, NbtValue};

/// 玩家 id（playerdata 落盘自描述面；原版玩家档 playerdata/<uuid>.dat 无
/// `id` 键——本仓带 id 走同一条 codec 判别路，登记 ENT-NBT-PLAYER）。
pub const PLAYER_ID: &str = "minecraft:player";
/// 命名空间前缀（EntityType.save 的 `putString("id", id)`，id 形参含前缀）。
const NAMESPACE: &str = "minecraft:";
/// 玩家最大空气（LivingEntity 基值 300；读侧缺 `Air` 的回退）。
const MAX_AIR: i32 = 300;
/// 26.1 坐标钳制域（Entity.java:2072-2073 clamp 后 setPosRaw）。
const MAX_POS_XZ: f32 = 3.000_051_2e7_f32;
const MAX_POS_Y: f32 = 2.0e7_f32;

/// 落盘快照：ECS 组件/玩家状态 → NBT 的搬运面（与 ECS 类型解耦）。
pub struct EntityNbt {
    /// `id` 键全名（`minecraft:zombie` 等；玩家为 [`PLAYER_ID`]）。
    pub vanilla_id: String,
    pub pos: Vec3,
    /// `Motion`（Entity.deltaMovement）。
    pub motion: Vec3,
    /// 朝向（**弧度**，本仓约定 dir=(sin,−cos)，0=北）。
    pub yaw: f32,
    /// 俯仰（**弧度**，本仓 +上；原版 XPitch +下）。
    pub pitch: f32,
    /// `Health`。
    pub health: f32,
    /// 缺 `Health` 键的读回退（原版 getMaxHealth）。
    pub max_health: f32,
    /// `OnGround`。
    pub on_ground: bool,
    /// `Fire`（Entity.remainingFireTicks 面；≤0 未燃）。
    pub fire: i32,
    /// `Air`（满 300）。
    pub air: i32,
    /// `UUID`（IntArray×4）。
    pub uuid: [i32; 4],
}

impl EntityNbt {
    /// 玩家侧快照基准（vanilla_id 固定 [`PLAYER_ID`]；运动/生存字段
    /// 由 GameRuntime 覆写）。
    pub fn player() -> Self {
        Self {
            vanilla_id: PLAYER_ID.into(),
            ..Self::bench()
        }
    }

    /// 无状态基准快照（测试/占位默认）。
    pub fn bench() -> Self {
        Self {
            vanilla_id: String::new(),
            pos: Vec3::ZERO,
            motion: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            health: 20.0,
            max_health: 20.0,
            on_ground: false,
            fire: 0,
            air: MAX_AIR,
            uuid: [0; 4],
        }
    }
}

/// 本仓弧度朝向 → 原版 YRot 度（取值域 (−180, 180]）：YRot 0=南(+Z)、
/// 本仓 0=北(−Z)，差半圈（yaw_rad_to_deg/yaw_deg_to_rad 是一对圆周双射）。
fn yaw_rad_to_deg(yaw: f32) -> f32 {
    let d = yaw.to_degrees() + 180.0; // (0, 360]
    if d > 180.0 { d - 360.0 } else { d }
}

/// 原版 YRot 度 → 本仓弧度朝向（[`yaw_rad_to_deg`] 的逆）。
fn yaw_deg_to_rad(yrot_deg: f32) -> f32 {
    wrap_angle(yrot_deg.to_radians() - std::f32::consts::PI)
}

/// MobId → 官方实体 id（`minecraft:` 前缀 + 注册表名）。
pub fn mob_vanilla_id(id: MobId) -> String {
    format!("{NAMESPACE}{}", id.def().name)
}

/// 官方实体 id → MobId（未知/无前缀 → None）。
fn mob_id_from_vanilla_id(id: &str) -> Option<MobId> {
    let name = id.strip_prefix(NAMESPACE)?;
    MOBS.iter()
        .position(|d| d.name == name)
        .map(|i| MobId(i as u8))
}

/// 确定性实体 UUID（IntArray×4）：本仓无运行时身份账，按 (seed, idx)
/// SplitMix64 合成——同实体同会话稳定（登记 ENT-NBT-UUID）。
pub fn entity_uuid(seed: u64, idx: u32) -> [i32; 4] {
    fn splitmix(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    let mut s = seed ^ (u64::from(idx) << 32 | 0x524D_4356);
    let hi = splitmix(&mut s);
    let lo = splitmix(&mut s);
    let stamp = splitmix(&mut s) as i32;
    [
        (hi >> 32) as i32,
        hi as i32,
        (lo >> 32) as i32,
        lo as i32 ^ (stamp & 0x7),
    ]
}

/// 实体 → NBT compound（12 核心键全部写入，键序对齐原版写入序）。
pub fn entity_to_nbt(e: &EntityNbt) -> NbtCompound {
    let mut c = NbtCompound::new();
    c.insert("id", NbtValue::String(e.vanilla_id.clone()));
    let d3 = |v: Vec3| {
        NbtValue::List(NbtList::from_vec(vec![
            NbtValue::Double(f64::from(v.x)),
            NbtValue::Double(f64::from(v.y)),
            NbtValue::Double(f64::from(v.z)),
        ]))
    };
    // 原版写入序（Entity.java:1991-2004）：Pos → Motion → Rotation →
    // fall_distance → Fire → Air → OnGround → Invulnerable →
    // PortalCooldown → UUID；Health 由 LivingEntity 追加（:724）。
    c.insert("Pos", d3(e.pos));
    c.insert("Motion", d3(e.motion));
    // Vec2.CODEC：YRot 后 XPitch（度）；本仓俯仰 +上 → 原版 +下取负。
    c.insert(
        "Rotation",
        NbtValue::List(NbtList::from_vec(vec![
            NbtValue::Float(yaw_rad_to_deg(e.yaw)),
            NbtValue::Float(-e.pitch.to_degrees()),
        ])),
    );
    // 本仓未跟踪坠落账——常量 0（读侧默认同值，登记 ENT-NBT-CONST）。
    c.insert("fall_distance", NbtValue::Double(0.0));
    c.insert(
        "Fire",
        NbtValue::Short(e.fire.clamp(i16::MIN as i32, i16::MAX as i32) as i16),
    );
    c.insert(
        "Air",
        NbtValue::Short(e.air.clamp(i16::MIN as i32, i16::MAX as i32) as i16),
    );
    c.insert("OnGround", NbtValue::Byte(i8::from(e.on_ground)));
    // 本仓无永久无敌态（MobTicks.invulnerable 是受击 i-frame，瞬态不落盘）。
    c.insert("Invulnerable", NbtValue::Byte(0));
    c.insert("PortalCooldown", NbtValue::Int(0));
    c.insert("UUID", NbtValue::IntArray(e.uuid.to_vec()));
    c.insert("Health", NbtValue::Float(e.health));
    c
}

/// NBT → 实体读回快照（Entity.load 的缺省/容错面）。
pub struct LoadedEntityNbt {
    /// 非玩家 → Some(MobId)；玩家面 → None。
    pub mob: Option<MobId>,
    pub pos: Vec3,
    pub motion: Vec3,
    /// 弧度（本仓约定）。
    pub yaw: f32,
    /// 弧度（本仓 +上）。
    pub pitch: f32,
    pub health: f32,
    pub on_ground: bool,
    pub fire: i32,
    pub air: i32,
}

/// 键下的三联数值表（Pos/Motion 共用）：缺键/形状不符 → None（调用方代
/// 表原版 orElse 缺省）。
fn vec3_at(c: &NbtCompound, key: &str) -> Option<[f64; 3]> {
    let l = c.get_list(key)?;
    if l.len() != 3 {
        return None;
    }
    Some([
        l.get(0)?.as_f64()?,
        l.get(1)?.as_f64()?,
        l.get(2)?.as_f64()?,
    ])
}

/// Pos 三联 → Vec3：缺失/形状不符 → 零（原版 `orElse(Vec3.ZERO)`，
/// Entity.java:2065）；非 finite → `None`（原版 `!isFinite` 抛
/// IllegalStateException 实体不入表，:2085-2087）。
fn pos_from(raw: Option<[f64; 3]>) -> Option<Vec3> {
    let [x, y, z] = raw.unwrap_or([0.0; 3]);
    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
        return None;
    }
    Some(Vec3::new(x as f32, y as f32, z as f32))
}

/// Motion 三联 → Vec3：缺失/形状不符 → 零；finite 分量按原版 >10 归零、
/// 非 finite 分量归 0（原版 NaN 可穿分类直进 setDeltaMovement，物理不可
/// 用——本仓收窄，登记 ENT-NBT-NAN）。
fn motion_from(raw: Option<[f64; 3]>) -> Vec3 {
    match raw {
        None => Vec3::ZERO,
        Some([x, y, z]) => {
            let cl = |v: f64| {
                if !v.is_finite() || v.abs() > 10.0 {
                    0.0
                } else {
                    v
                }
            };
            Vec3::new(cl(x) as f32, cl(y) as f32, cl(z) as f32)
        }
    }
}

pub fn entity_from_nbt(c: &NbtCompound) -> Option<LoadedEntityNbt> {
    // 26.1 id 必需；未知 id 跳过不入表（EntityType.by 失败面）。
    let id = c.get_string("id")?;
    let mob = if id == PLAYER_ID {
        None
    } else {
        Some(mob_id_from_vanilla_id(id)?)
    };
    let max_health = match mob {
        Some(m) => m.def().health,
        None => 20.0, // 玩家满值 20（Player::default）
    };
    let pos = pos_from(vec3_at(c, "Pos"))?;
    let motion = motion_from(vec3_at(c, "Motion"));
    // Rotation 读面（Vec2.CODEC YRot 后 XPitch，度）：表缺失/形状坏 →
    // orElse(Vec2.ZERO) 等价——YRot 0=南，经度→弧度半圈差换算即本仓
    // yaw≈π；表内分量非 finite → 该分量落 0 度（原版 NaN 可穿 setYRot
    // 是未定面，本仓收窄，登记 ENT-NBT-NAN 同族）。
    let (yrot, xrot) = match c.get_list("Rotation").filter(|l| l.len() == 2) {
        Some(l) => (
            l.get(0).and_then(|v| v.as_f64()).filter(|v| v.is_finite()),
            l.get(1).and_then(|v| v.as_f64()).filter(|v| v.is_finite()),
        ),
        // 缺表/形状坏：Vec2.ZERO（YRot 0 = 南）。
        None => (Some(0.0), Some(0.0)),
    };
    let yaw = yaw_deg_to_rad(yrot.unwrap_or(0.0) as f32);
    // 本仓俯仰 +上 → 原版 XPitch +下，读侧取负换回。
    let pitch = -((xrot.unwrap_or(0.0) as f32).to_radians());
    let pos = Vec3::new(
        pos.x.clamp(-MAX_POS_XZ, MAX_POS_XZ),
        pos.y.clamp(-MAX_POS_Y, MAX_POS_Y),
        pos.z.clamp(-MAX_POS_XZ, MAX_POS_XZ),
    );
    // Fire <0 是原版未燃值（remainingFireTicks 初值 -1），映射回本仓 0=未燃。
    let fire = c.get_i32("Fire").unwrap_or(0).max(0);
    let air = c.get_i32("Air").unwrap_or(MAX_AIR);
    let on_ground = c.get_i32("OnGround").unwrap_or(0) != 0;
    // LivingEntity.java:791 getFloatOr("Health", getMaxHealth()) → setHealth
    // 钳 0..max（setHealth 的 Mth.clamp 面）。
    let health =
        (c.get_f64("Health").unwrap_or(f64::from(max_health)) as f32).clamp(0.0, max_health);
    // fall_distance（本仓未跟踪）/PortalCooldown（无传送门）/UUID（无身份
    // 账）：读侧不消费，未知/多余键原样跳过 = 26.1 恢复语义。
    Some(LoadedEntityNbt {
        mob,
        pos,
        motion,
        yaw,
        pitch,
        health,
        on_ground,
        fire,
        air,
    })
}
