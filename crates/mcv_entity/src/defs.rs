//! Mob registry. Stats from NOTES-2 (26.1 createAttributes per class).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobKind {
    Zombie,
    Skeleton,
    Creeper,
    Spider,
    Cow,
    Pig,
    Sheep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobId(pub u8);

impl MobId {
    pub const ZOMBIE: MobId = MobId(0);
    pub const SKELETON: MobId = MobId(1);
    pub const CREEPER: MobId = MobId(2);
    pub const SPIDER: MobId = MobId(3);
    pub const COW: MobId = MobId(4);
    pub const PIG: MobId = MobId(5);
    pub const SHEEP: MobId = MobId(6);

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
    /// FOLLOW_RANGE (Zombie 35).
    pub follow_range: f32,
    /// SPEED attribute (movement_speed; ×43.17 ≈ m/s since player 0.1=4.317).
    pub speed_attr: f32,
    pub hostile: bool,
    /// XP dropped on death (Monster base 5).
    pub xp: u32,
    pub half_size: [f32; 3],
}

pub static MOBS: [MobDef; 7] = [
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
    // Skeleton: health 20, ATTACK 2.0 (bow), ARMOR 0, SPEED 0.25
    MobDef {
        name: "skeleton",
        kind: MobKind::Skeleton,
        health: 20.0,
        attack_damage: 2.0,
        armor: 0.0,
        follow_range: 35.0,
        speed_attr: 0.25,
        hostile: true,
        xp: 5,
        half_size: [0.35, 0.99, 0.35],
    },
    // Creeper: health 20, no melee ATTACK (explosion), SPEED 0.25
    MobDef {
        name: "creeper",
        kind: MobKind::Creeper,
        health: 20.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 35.0,
        speed_attr: 0.25,
        hostile: true,
        xp: 5,
        half_size: [0.3, 0.85, 0.3],
    },
    // Spider: health 16, ATTACK 2.0(?), SPEED 0.3
    MobDef {
        name: "spider",
        kind: MobKind::Spider,
        health: 16.0,
        attack_damage: 2.0,
        armor: 0.0,
        follow_range: 35.0,
        speed_attr: 0.3,
        hostile: true,
        xp: 5,
        half_size: [0.7, 0.45, 0.7],
    },
    // Cow: health 10, passive
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
        half_size: [0.45, 0.65, 0.45],
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
];

/// Passive speed converted to m/s (player 0.1 = 4.317 m/s).
pub fn speed_m_s(speed_attr: f32) -> f32 {
    speed_attr * 43.17
}
