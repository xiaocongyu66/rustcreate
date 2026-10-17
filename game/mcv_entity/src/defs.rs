//! Mob registry. Stats from NOTES-2 (26.1 createAttributes per class).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobKind {
    Zombie,
    Skeleton,
    Creeper,
    Spider,
    Blaze,
    EnderMan,
    Creaking,
    Cow,
    Pig,
    Sheep,
    Chicken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobId(pub u8);

impl MobId {
    pub const ZOMBIE: MobId = MobId(0);
    pub const SKELETON: MobId = MobId(1);
    pub const CREEPER: MobId = MobId(2);
    pub const SPIDER: MobId = MobId(3);
    pub const BLAZE: MobId = MobId(4);
    pub const ENDERMAN: MobId = MobId(5);
    pub const CREAKING: MobId = MobId(6);
    pub const COW: MobId = MobId(7);
    pub const PIG: MobId = MobId(8);
    pub const SHEEP: MobId = MobId(9);
    pub const CHICKEN: MobId = MobId(10);

    pub const fn def(self) -> &'static MobDef {
        &MOBS[self.0 as usize]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MobDef {
    pub name: &'static str,
    pub kind: MobKind,
    pub health: f32,
    /// ATTACK_DAMAGE attribute (createAttributes).
    pub attack_damage: f32,
    /// ARMOR attribute.
    pub armor: f32,
    /// FOLLOW_RANGE（26.1 基础值 16，Mob.java:162；仅 Zombie 覆写 35，Zombie.java:132）
    pub follow_range: f32,
    /// SPEED attribute (movement_speed; ×43.17 ≈ m/s since player 0.1=4.317).
    pub speed_attr: f32,
    pub hostile: bool,
    /// XP dropped on death (Monster base 5).
    pub xp: u32,
    pub half_size: [f32; 3],
}

pub static MOBS: [MobDef; 11] = [
    // Zombie.java:130-137 — ATTACK 3.0, ARMOR 2.0, FOLLOW 35, SPEED 0.23
    MobDef {
        name: "zombie",
        kind: MobKind::Zombie,
        health: 20.0,
        attack_damage: 3.0,
        armor: 2.0,
        follow_range: 35.0,
        speed_attr: 0.23,
        hostile: true,
        xp: 5,
        half_size: [0.3, 0.95, 0.3],
    },
    // Skeleton.java/AbstractSkeleton.java — health 20(默认), ATTACK 2.0(弓, 近战兜底),
    // ARMOR 0, SPEED 0.25, FOLLOW 16(基础值, AbstractSkeleton 未覆写)
    MobDef {
        name: "skeleton",
        kind: MobKind::Skeleton,
        health: 20.0,
        attack_damage: 2.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.25,
        hostile: true,
        xp: 5,
        half_size: [0.35, 0.99, 0.35],
    },
    // Creeper.java:77-79 — health 20(默认), 近战无伤害(爆炸怪), SPEED 0.25, FOLLOW 16(基础值)
    MobDef {
        name: "creeper",
        kind: MobKind::Creeper,
        health: 20.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.25,
        hostile: true,
        xp: 5,
        half_size: [0.3, 0.85, 0.3],
    },
    // Spider.java:88-90 — health 16(MAX_HEALTH 覆写), ATTACK 2.0(Monster 默认, Attributes.java:14),
    // SPEED 0.3, FOLLOW 16(基础值)
    MobDef {
        name: "spider",
        kind: MobKind::Spider,
        health: 16.0,
        attack_damage: 2.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.3,
        hostile: true,
        xp: 5,
        half_size: [0.7, 0.45, 0.7],
    },
    // Blaze.java:54-56 — ATTACK 6.0, SPEED 0.23, FOLLOW 48；MAX_HEALTH 未
    // 覆写（Attributes.java:58-60 默认 20）；xpReward=10（Blaze.java:40）。
    // EntityType.java:277 — .sized(0.6F, 1.8F) → half [0.3, 0.9, 0.3]。
    MobDef {
        name: "blaze",
        kind: MobKind::Blaze,
        health: 20.0,
        attack_damage: 6.0,
        armor: 0.0,
        follow_range: 48.0,
        speed_attr: 0.23,
        hostile: true,
        xp: 10,
        half_size: [0.3, 0.9, 0.3],
    },
    // EnderMan.java:113-120 — MAX_HEALTH 40, SPEED 0.3, ATTACK 7.0,
    // FOLLOW 64；xpReward 未覆写 → Monster 默认 5。
    // EntityType.java:407-409 — .sized(0.6F, 2.9F) → half [0.3, 1.45, 0.3]。
    MobDef {
        name: "enderman",
        kind: MobKind::EnderMan,
        health: 40.0,
        attack_damage: 7.0,
        armor: 0.0,
        follow_range: 64.0,
        speed_attr: 0.3,
        hostile: true,
        xp: 5,
        half_size: [0.3, 1.45, 0.3],
    },
    // Creaking.java:64-69 + :127-134 — MAX_HEALTH 1, SPEED 0.4, ATTACK 3.0,
    // FOLLOW 32；xpReward=0（Creaking.java:92）。
    // EntityType.java:352-355 — .sized(0.9F, 2.7F) → half [0.45, 1.35, 0.45]。
    MobDef {
        name: "creaking",
        kind: MobKind::Creaking,
        health: 1.0,
        attack_damage: 3.0,
        armor: 0.0,
        follow_range: 32.0,
        speed_attr: 0.4,
        hostile: true,
        xp: 0,
        half_size: [0.45, 1.35, 0.45],
    },
    // Cow: health 10, passive
    // EntityType.java:351 — .sized(0.9F, 1.4F) → half [0.45, 0.7, 0.45]
    //（旧值 y=0.65 误用羊的尺寸，修正之）
    MobDef {
        name: "cow",
        kind: MobKind::Cow,
        health: 10.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.2,
        hostile: false,
        xp: 1,
        half_size: [0.45, 0.7, 0.45],
    },
    MobDef {
        name: "pig",
        kind: MobKind::Pig,
        health: 10.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.25,
        hostile: false,
        xp: 1,
        half_size: [0.45, 0.45, 0.45],
    },
    MobDef {
        name: "sheep",
        kind: MobKind::Sheep,
        health: 8.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.23,
        hostile: false,
        xp: 1,
        half_size: [0.45, 0.65, 0.45],
    },
    // Chicken.java:109-111 — MAX_HEALTH 4.0, MOVEMENT_SPEED 0.25；
    // EntityType.java:333 — .sized(0.4F, 0.7F) → half [0.2, 0.35, 0.2]
    MobDef {
        name: "chicken",
        kind: MobKind::Chicken,
        health: 4.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.25,
        hostile: false,
        xp: 1,
        half_size: [0.2, 0.35, 0.2],
    },
];

/// Passive speed converted to m/s (player 0.1 = 4.317 m/s).
pub fn speed_m_s(speed_attr: f32) -> f32 {
    speed_attr * 43.17
}
