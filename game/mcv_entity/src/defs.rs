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
    Chicken,
    Cod,
    Salmon,
    TropicalFish,
    Axolotl,
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
    pub const CHICKEN: MobId = MobId(7);
    pub const COD: MobId = MobId(8);
    pub const SALMON: MobId = MobId(9);
    pub const TROPICAL_FISH: MobId = MobId(10);
    pub const AXOLOTL: MobId = MobId(11);

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
    /// 水生生物（WaterAnimal 族：cod/salmon/tropical_fish + axolotl）。
    /// 刷怪走水通道、AI 走 aquatic_ai_system、物理吃 in_water 游泳律。
    pub aquatic: bool,
    /// 满空气账（tick）：鱼/美西螈离水窒息用——WaterAnimal 300
    ///（handleAirSupply 的默认 air=300）/Axolotl 6000（:86
    /// AXOLOTL_TOTAL_AIR_SUPPLY）。非水生恒 0。
    pub air_supply: u32,
    /// XP dropped on death (Monster base 5).
    pub xp: u32,
    pub half_size: [f32; 3],
}

pub static MOBS: [MobDef; 12] = [
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
        aquatic: false,
        air_supply: 0,
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
        aquatic: false,
        air_supply: 0,
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
        aquatic: false,
        air_supply: 0,
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
        aquatic: false,
        air_supply: 0,
        xp: 5,
        half_size: [0.7, 0.45, 0.7],
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
        aquatic: false,
        air_supply: 0,
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
        aquatic: false,
        air_supply: 0,
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
        aquatic: false,
        air_supply: 0,
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
        aquatic: false,
        air_supply: 0,
        xp: 1,
        half_size: [0.2, 0.35, 0.2],
    },
    // Cod（AbstractFish.createAttributes — 只覆写 MAX_HEALTH 3.0；
    // EntityType.java:339-341 — .sized(0.5F, 0.3F) → half [0.25, 0.15, 0.25]；
    // MobCategory.WATER_AMBIENT）。MOVEMENT_SPEED 未覆写 → 属性默认 0.7，
    // 但鱼的实际巡航由 travelInWater moveRelative(0.01)+阻尼决定（约
    // 1.2 m/s），引擎在 aquatic 系统用视角 wish 幅值表达，不走此换算。
    MobDef {
        name: "cod",
        kind: MobKind::Cod,
        health: 3.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.7, // Attributes.java:68 属性默认（fish 未覆写）
        hostile: false,
        aquatic: true,
        // WaterAnimal.handleAirSupply：!isInWater → air−1/t，空气上限
        // 300 tick（LivingEntity baseTick 同账）。
        air_supply: 300,
        // WaterAnimal.getBaseExperienceReward — 1 + random.nextInt(3)，
        // 表值取中位 2（掉落侧动态掷点见 xp 接线）。
        xp: 2,
        half_size: [0.25, 0.15, 0.25],
    },
    // Salmon（EntityType.java:854-856 — .sized(0.7F, 0.4F) →
    // half [0.35, 0.2, 0.35]）。变体尺寸（small/large 0.5/1.5 缩放）在
    // 渲染矩阵+表现层处理（vanilla refreshDimensions 的 AABB 同步未引入，
    // 碰撞盒取 MEDIUM 基准——KNOWN-DIVERGENCE）。
    MobDef {
        name: "salmon",
        kind: MobKind::Salmon,
        health: 3.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.7,
        hostile: false,
        aquatic: true,
        air_supply: 300,
        xp: 2,
        half_size: [0.35, 0.2, 0.35],
    },
    // TropicalFish（EntityType.java:1014-1016 — .sized(0.5F, 0.4F) →
    // half [0.25, 0.2, 0.25]）。变体打包（pattern|base<<16|patternColor<<24)
    // 见 aquatic.rs。
    MobDef {
        name: "tropical_fish",
        kind: MobKind::TropicalFish,
        health: 3.0,
        attack_damage: 0.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 0.7,
        hostile: false,
        aquatic: true,
        air_supply: 300,
        xp: 2,
        half_size: [0.25, 0.2, 0.25],
    },
    // Axolotl（Axolotl.java:383-388 — MAX_HEALTH 14.0, MOVEMENT_SPEED 1.0,
    // ATTACK_DAMAGE 2.0；EntityType.java:235-237 — .sized(0.75F, 0.42F) →
    // half [0.375, 0.21, 0.375]；MobCategory.AXOLOTLS）。离水窒息上限
    // 6000 tick（:86 AXOLOTL_TOTAL_AIR_SUPPLY）。速度属性 1.0 在引擎
    // 43.17 换算律下无意义（43 m/s），地面行走取玩家步速钳制（aquatic
    // 系统）——KNOWN-DIVERGENCE（引擎移动律差异）。
    MobDef {
        name: "axolotl",
        kind: MobKind::Axolotl,
        health: 14.0,
        attack_damage: 2.0,
        armor: 0.0,
        follow_range: 16.0,
        speed_attr: 1.0, // Axolotl.java:385 原值；行走面被 aquatic 钳制
        hostile: false,
        aquatic: true,
        air_supply: 6000,
        xp: 2,
        half_size: [0.375, 0.21, 0.375],
    },
];

/// Passive speed converted to m/s (player 0.1 = 4.317 m/s).
pub fn speed_m_s(speed_attr: f32) -> f32 {
    speed_attr * 43.17
}
