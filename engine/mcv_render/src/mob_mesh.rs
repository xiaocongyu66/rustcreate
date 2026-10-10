//! 生物网格管线：鸡/牛/羊/猪 + 僵尸/骷髅/苦力怕/蜘蛛八张原版模型（26.1
//! `client/model/animal/**`、`client/model/monster/**`、`HumanoidModel`）
//! 的部位盒定义 → 顶点数据 + 部位模型矩阵 + 贴图数组加载。
//!
//! 常数照反编译 Java 逐字提取（`createBodyLayer`/`createBase*Model`）：
//! - 鸡 `AdultChickenModel`：头 tex(0,0) box(-2,-6,-2,4,6,3) pivot(0,15,-4)，
//!   喙 tex(14,0)、红髯 tex(14,4) 是头的 **PartPose.ZERO 子部位**（共享头
//!   pivot、随头旋转，此处扁平化成同 pivot 部位）；躯干 tex(0,9)
//!   box(-3,-4,-3,6,8,6) pivot(0,16,0) 静态 xRot=π/2；腿 tex(26,0)
//!   box(-1,0,-3,3,5,3) pivot(-2/+1,19,1)（Java 右腿 x=-2、左腿 x=+1，
//!   盒宽 3px 非对称摆位）；翅 tex(24,13) box(±,0,-3,1,4,6)
//!   pivot(±4,13,0)。层定义 64x32。
//! - 牛 `CowModel`：头 pivot(0,4,-8) 含 4 盒（头 tex(0,0) box(-4,-4,-6,8,8,6)、
//!   鼻 tex(1,33) box(-3,1,-7,6,3,1)、双角 tex(22,0) box(±,-5,-5,1,3,1)）；
//!   躯干 tex(18,4) box(-6,-10,-7,12,18,10) + 乳房 tex(52,0) box(-2,2,-8,4,6,1)
//!   pivot(0,5,2) xRot=π/2；腿 tex(0,16) box(-2,0,-2,4,12,4) pivot(±4,12,7/−5)，
//!   左腿 mirror。层定义 64x64。
//! - 羊 `SheepModel`：头 tex(0,0) box(-3,-4,-6,6,6,8) pivot(0,6,-8)；躯干
//!   tex(28,8) box(-4,-10,-7,8,16,6) pivot(0,5,2) xRot=π/2；腿 tex(0,16)
//!   box(-2,0,-2,4,12,4) pivot(±3,12,7/−5)，**右**腿 mirror（createBodyMesh
//!   (12, false, true, NONE)）。层定义 64x32。剪毛前还有羊毛层
//!   `SheepFurModel`：头 box(-3,-4,-4,6,6,6) grow 0.6、躯干 grow 1.75、
//!   腿 box(…,4,6,4) grow 0.5，UV 与基模同矩形、贴图换 sheep_wool.png。
//! - 猪 `PigModel`（g=NONE，LayerDefinitions.java:229）：头 pivot(0,12,-6)
//!   含头 tex(0,0) box(-4,-4,-8,8,8,8) + 鼻 tex(16,16) box(-2,0,-9,4,3,1)；
//!   躯干/腿走 `QuadrupedModel.createBodyMesh(6, true, false, g)`——躯干
//!   tex(28,8) box(-5,-10,-7,10,16,8) pivot(0,11,2) xRot=π/2，腿
//!   box(-2,0,-2,4,6,4) pivot(±3,18,7/−5)，**左**腿 mirror。层定义 64x64。
//! - 僵尸 `ZOMBIE` 层 = `humanoidBodyLayer`（LayerDefinitions.java:510 =
//!   `HumanoidModel.createMesh(NONE, 0)`，与 Steve 1:1 同骨架，层定义
//!   64x64）：六部件头 tex(0,0) box(-4,-8,-4,8,8,8) pivot(0,0,0)、身
//!   tex(16,16) box(-4,0,-2,8,12,4)、臂 tex(40,16) box(-3,-2,-2,4,12,4)
//!   pivot(∓5,2,0)（左臂 mirror）、腿 tex(0,16) box(-2,0,-2,4,12,4)
//!   pivot(±1.9,12,0)。双臂走 `AnimationUtils.animateZombieArms` 的经典
//!   举臂（站立 armDrop = −π/2.25，攻击态 −π/1.5——本侧恒取站立值，
//!   且**覆盖**行走摆臂：HumanoidModel.setupAnim 先算摆臂、
//!   animateZombieArms 再覆写 xRot，原版僵尸走路手臂不摆）。
//! - 骷髅 `SkeletonModel.createBodyLayer`（= humanoid 骨架 + 细四肢，
//!   64x32）：头/身同僵尸；臂 tex(40,16) box(-1,-2,-1,2,12,2)
//!   pivot(∓5,2,0)、腿 tex(0,16) box(-1,0,-1,2,12,2) pivot(±2,12,0)，
//!   左侧 mirror。行走摆臂摆腿同 HumanoidModel（臂幅/腿幅 = 1.0/1.4）。
//! - 苦力怕 `CreeperModel.createBodyLayer(NONE)`（64x32）：头 tex(0,0)
//!   box(-4,-8,-4,8,8,8) pivot(0,6,0)、身 tex(16,16) box(-4,0,-2,8,12,4)
//!   pivot(0,6,0)、腿 tex(0,16) box(-2,0,-2,4,6,4) pivot(±2,18,±4)。
//!   四腿交替与四足同款对角同相（setupAnim：RH/LF = cos、LH/RF = cos+π）。
//! - 蜘蛛 `SpiderModel.createSpiderBodyLayer`（64x32）：头 tex(32,4)
//!   box(-4,-4,-8,8,8,8) pivot(0,15,−3)、body0 tex(0,0) box(-3,-3,-3,6,6,6)
//!   pivot(0,15,0)、body1 tex(0,12) box(-5,-4,-6,10,8,12) pivot(0,15,9)；
//!   八腿 tex(18,0) box(-15,-1,-1,16,2,2)（左腿 mirror 后 box(-1,-1,-1,…)
//!   ），静态姿态 pivot(±4,15,2/1/0/−1) + yRot(±π/4、±π/8) + zRot(∓π/4、
//!   ∓0.58119464)，行走 yRot 摆 = −cos(2·apos+off)·0.4·k、zRot 步进 =
//!   |sin(apos+off)|·0.4·k（右 + 左 −，四对相位 off = 0/π/π/2/3π/2）。
//!   盒模近似：腿不改用阶梯几何，静态 + 动态转角照抄原版数字。
//!
//! 四足行走摆腿（`QuadrupedModel.setupAnim`）：右后/左前 = cos(pos·0.6662)·1.4·k，
//! 左后/右前反相（对角同相）。本引擎 `phase` 已吸收 0.6662 因子（与
//! `player_mesh::update_walk_animation` 同源），幅值 `amount` 上限 0.88 →
//! 腿角 = cos(phase+shift)·amount·(1.4/0.88)，满速 ≈ 原版 ±1.4 rad。
//! 鸡翅膀（`ChickenModel.setupAnim`）：zRot = ±(sin(flap)+1)·flapSpeed，
//! 落地 flapSpeed→0（收拢），由游戏侧按 Chicken.java:117-129 推进后经
//! [`MobPose::wing_angle`] 传入。
//!
//! 坐标换算：四种生物脚底都在模型 y=24（与玩家同一 y↓ 根在头顶约定；
//! `ChickenModel::Y_OFFSET=16` 在 26.1 快照里无任何引用点，为死常数，
//! 不参与换算）。但**正面朝向不同**：MC 四足模型正面朝模型 −z（喙/鼻/
//! 脸贴图段都在 −z 面，ChickenModel 头 pivot z=−4 在躯干 z=2 之前），
//! 而玩家皮肤的脸贴图段同样落在引擎 −z（玩家变换 z 取反后 front 段挂在
//! bz=1 角，见 player_mesh::classic_faces 面 4）。因此生物顶点变换相对玩家
//! 少一次 z 取反：
//! `local = (-px, -py, +pz) × PX`（x 镜像 + y 翻转，z 原样——这是绕 z 的
//! 180° 旋转，proper、绕序不变），pivot 世界偏移 = `(-p.x, 24-p.y, +p.z)`
//! 再乘身体 yaw。MC 部位旋转须经共轭 R_eng = Q·R_mc·Q⁻¹（Q 为上述变换；
//! 绕 x 的 θ → −θ，绕 y 的 θ → −θ，绕 z 的 θ 不变号，见
//! mob_model_matrices 内注释）。
//! 相应地，经典条带展开的 face-with-eyes 段须从 bz=1 角（玩家）翻到 bz=0
//! 角（生物，喙/鼻一侧的 −z 面）——[`player_mesh::quadruped_faces`]，
//! 逐角 s 映射与 26.1 ModelPart.Cube 的顶点级 UV 指派共轭一致。
//! 左右镜像说明：x 取反把 MC 建模在 −x 侧的 "right_*" 部位映到引擎 +x
//! （左右互换）；生物几何双轴对称，视觉无差（与玩家管线同款约定）。

use crate::player_mesh::{PX, PlayerVertex, quadruped_faces};
use glam::{Mat4, Vec3};

/// 生物种类数（鸡/牛/羊/猪 + 僵尸/骷髅/苦力怕/蜘蛛）。
pub const MOB_KIND_COUNT: usize = 8;
/// 单体最大部位数（羊 = 6 基模 + 6 羊毛层；蜘蛛 11 也在限内）。
pub const MAX_MOB_PARTS: usize = 12;
/// 贴图数组层数（层序见 [`MOB_TEX_FILES`]；chicken/cow/pig 用 temperate 变体）。
pub const MOB_TEX_LAYERS: usize = 9;
/// 贴图统一 pad 到 64x64（鸡/羊/骷髅/苦力怕/蜘蛛原生 64x32，上半透明
/// ——2D 数组各层必须同尺寸）。
pub const MOB_TEX_PX: usize = 64;
/// 每种生物的部位数（下标 = [`MobModelKind`] 序）。
pub const MOB_PART_COUNTS: [usize; MOB_KIND_COUNT] = [8, 6, 12, 6, 6, 6, 6, 11];
/// 离屏/主控一次最多提交的生物实例数（uniform 槽位）。
pub const MOB_MAX_INSTANCES: usize = 64;

/// 行走腿摆增益：MC 满速腿摆 1.4 rad，本引擎 amount 满值 0.88（player_mesh
/// `update_walk_animation`），换算系数 = 1.4/0.88。
pub const WALK_GAIN: f32 = 1.4 / 0.88;

/// 渲染侧生物种类（与 `mcv_entity::MobId` 的映射在游戏层做，引擎不反向依赖）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobModelKind {
    Chicken = 0,
    Cow = 1,
    Sheep = 2,
    Pig = 3,
    Zombie = 4,
    Skeleton = 5,
    Creeper = 6,
    Spider = 7,
}

impl MobModelKind {
    pub fn idx(self) -> usize {
        self as usize
    }
    /// 贴图数组层（羊基模 2 / 羊毛 3，由部位表给出）。
    pub fn from_idx(i: u32) -> Option<MobModelKind> {
        match i {
            0 => Some(MobModelKind::Chicken),
            1 => Some(MobModelKind::Cow),
            2 => Some(MobModelKind::Sheep),
            3 => Some(MobModelKind::Pig),
            4 => Some(MobModelKind::Zombie),
            5 => Some(MobModelKind::Skeleton),
            6 => Some(MobModelKind::Creeper),
            7 => Some(MobModelKind::Spider),
            _ => None,
        }
    }
}

/// mob 贴图相对路径（相对 assets/minecraft/textures/）。全部原版素材，
/// 程序化占位是事故——缺文件时按缺素材降级（不渲染），绝不伪造。
pub const MOB_TEX_FILES: [&str; MOB_TEX_LAYERS] = [
    "entity/chicken/chicken_temperate.png",
    "entity/cow/cow_temperate.png",
    "entity/sheep/sheep.png",
    "entity/sheep/sheep_wool.png",
    "entity/pig/pig_temperate.png",
    "entity/zombie/zombie.png",
    "entity/skeleton/skeleton.png",
    "entity/creeper/creeper.png",
    "entity/spider/spider.png",
];

/// 部位动画角色。
#[derive(Clone, Copy)]
enum PartAnim {
    /// 静态（躯干转轴等，只带静态 xRot）。
    Fixed,
    /// 头部：xRot = head_pitch（羊再叠加吃草低头）。
    Head,
    /// 行走腿：xRot = cos(phase + shift)·amount·WALK_GAIN。
    WalkLeg { shift: f32 },
    /// 人形行走臂（HumanoidModel.setupAnim:223-226）：xRot =
    /// cos(phase + shift)·amount·(1.0/0.88)——幅值 = 腿 × (1.0/1.4)。
    WalkArm { shift: f32 },
    /// 僵尸举臂（AnimationUtils.animateZombieArms:75 `armDrop`）：MC xRot
    /// 恒为 `drop`（行走摆臂被覆写，原版僵尸走路手臂不摆）。
    ZombieArm { drop: f32 },
    /// 蜘蛛腿（SpiderModel.setupAnim:73-96）：静态姿态 y0/z0 + 行走
    /// yRot 摆 −cos(2·phase+off)·0.4·k、zRot 步进 |sin(phase+off)|·0.4·k，
    /// `side` = +1（右腿，MC yRot/zRot 增）/-1（左腿，减）。
    SpiderLeg {
        side: f32,
        off: f32,
        y0: f32,
        z0: f32,
    },
    /// 鸡翅：zRot = sign·wing_angle（右 + 左 −，ChickenModel.setupAnim:30-31）。
    Wing { sign: f32 },
}

/// 盒体（像素，MC 约定：min 相对部位 pivot，y 向下）。Copy 供静态
/// 初始化的 `&[SINGLE_BOX]` / `&[mirrored(..)]` 数组取用。
#[derive(Clone, Copy)]
struct MobBox {
    min: [f32; 3],
    size: [f32; 3],
    /// CubeDeformation：顶点外扩（UV 区域不变，26.1 ModelPart.Cube growX/Y/Z）。
    grow: f32,
    tex: [f32; 2],
    /// CubeListBuilder.mirror()：逐面水平镜像 UV（26.1 实现为顶点数组反转，
    /// 视觉等价于每个面 s→1−s；X 坐标 min/max 交换不改变盒体占据空间）。
    mirror: bool,
}

/// 部位定义：pivot（模型空间，y↓，脚底=24）+ 静态 xRot + 盒组 + 动画角色。
struct PartDef {
    pivot: [f32; 3],
    /// PartPose.offsetAndRotation 的 xRot（弧度；躯干 π/2）。
    rot_x: f32,
    tex_layer: u32,
    anim: PartAnim,
    boxes: &'static [MobBox],
}

const fn bx(min: [f32; 3], size: [f32; 3], tex: [f32; 2]) -> MobBox {
    MobBox {
        min,
        size,
        grow: 0.0,
        tex,
        mirror: false,
    }
}

const fn mirrored(mut b: MobBox) -> MobBox {
    b.mirror = true;
    b
}

const fn grown(mut b: MobBox, g: f32) -> MobBox {
    b.grow = g;
    b
}

// ---- 鸡（AdultChickenModel，64x32） -----------------------------------------

static CHICKEN_HEAD: MobBox = bx([-2.0, -6.0, -2.0], [4.0, 6.0, 3.0], [0.0, 0.0]);
static CHICKEN_BEAK: MobBox = bx([-2.0, -4.0, -4.0], [4.0, 2.0, 2.0], [14.0, 0.0]);
static CHICKEN_WATTLE: MobBox = bx([-1.0, -2.0, -3.0], [2.0, 2.0, 2.0], [14.0, 4.0]);
static CHICKEN_BODY: MobBox = bx([-3.0, -4.0, -3.0], [6.0, 8.0, 6.0], [0.0, 9.0]);
static CHICKEN_LEG: MobBox = bx([-1.0, 0.0, -3.0], [3.0, 5.0, 3.0], [26.0, 0.0]);
static CHICKEN_R_WING: MobBox = bx([0.0, 0.0, -3.0], [1.0, 4.0, 6.0], [24.0, 13.0]);
static CHICKEN_L_WING: MobBox = bx([-1.0, 0.0, -3.0], [1.0, 4.0, 6.0], [24.0, 13.0]);

// ---- 牛（CowModel，64x64） ---------------------------------------------------

static COW_HEAD: MobBox = bx([-4.0, -4.0, -6.0], [8.0, 8.0, 6.0], [0.0, 0.0]);
static COW_SNOUT: MobBox = bx([-3.0, 1.0, -7.0], [6.0, 3.0, 1.0], [1.0, 33.0]);
// 双角是两个独立盒体（CowModel.createBaseCowModel：right_horn box(-5,-5,-5)、
// left_horn box(4,-5,-5)，均 tex(22,0) 且无 mirror）——不能用单盒 ±x 复用。
static COW_HORN_R: MobBox = bx([-5.0, -5.0, -5.0], [1.0, 3.0, 1.0], [22.0, 0.0]);
static COW_HORN_L: MobBox = bx([4.0, -5.0, -5.0], [1.0, 3.0, 1.0], [22.0, 0.0]);
static COW_BODY: MobBox = bx([-6.0, -10.0, -7.0], [12.0, 18.0, 10.0], [18.0, 4.0]);
static COW_UDDER: MobBox = bx([-2.0, 2.0, -8.0], [4.0, 6.0, 1.0], [52.0, 0.0]);
static COW_LEG: MobBox = bx([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0]);

// ---- 羊（SheepModel 基模 64x32 + SheepFurModel 羊毛层同尺寸） ----------------

static SHEEP_HEAD: MobBox = bx([-3.0, -4.0, -6.0], [6.0, 6.0, 8.0], [0.0, 0.0]);
static SHEEP_BODY: MobBox = bx([-4.0, -10.0, -7.0], [8.0, 16.0, 6.0], [28.0, 8.0]);
static SHEEP_LEG: MobBox = bx([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0]);
static SHEEP_WOOL_HEAD: MobBox = grown(
    bx([-3.0, -4.0, -4.0], [6.0, 6.0, 6.0], [0.0, 0.0]),
    0.6, // SheepFurModel head CubeDeformation(0.6)
);
static SHEEP_WOOL_BODY: MobBox = grown(
    bx([-4.0, -10.0, -7.0], [8.0, 16.0, 6.0], [28.0, 8.0]),
    1.75, // SheepFurModel body CubeDeformation(1.75)
);
static SHEEP_WOOL_LEG: MobBox = grown(
    bx([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0]),
    0.5, // SheepFurModel leg CubeDeformation(0.5)
);

// ---- 猪（PigModel，64x64，g=NONE） -------------------------------------------

static PIG_HEAD: MobBox = bx([-4.0, -4.0, -8.0], [8.0, 8.0, 8.0], [0.0, 0.0]);
static PIG_SNOUT: MobBox = bx([-2.0, 0.0, -9.0], [4.0, 3.0, 1.0], [16.0, 16.0]);
static PIG_BODY: MobBox = bx([-5.0, -10.0, -7.0], [10.0, 16.0, 8.0], [28.0, 8.0]);
static PIG_LEG: MobBox = bx([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0]);

// ---- 僵尸（humanoidBodyLayer = HumanoidModel.createMesh(NONE, 0)，64x64） ----

static HUMANOID_HEAD: MobBox = bx([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], [0.0, 0.0]);
static HUMANOID_BODY: MobBox = bx([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0]);
// HumanoidModel right_arm addBox(-3,-2,-2,4,12,4)（:101）；左臂
// mirror().addBox(-1,-2,-2,4,12,4)（:105）——mirror 只翻 UV，盒坐标
// 本身就比右臂内收 2px 再靠 pivot +5 外推，两臂不对称，不能共用右臂盒
// （共用会让左臂世界 x∈[2,6] 压进躯干 [−4,4]，原版是 [4,8]）。
static ZOMBIE_ARM: MobBox = bx([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0]);
static ZOMBIE_ARM_LEFT: MobBox =
    mirrored(bx([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0]));
static ZOMBIE_LEG: MobBox = bx([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0]);

// ---- 骷髅（SkeletonModel.createBodyLayer = humanoid 骨架细四肢，64x32） ------

// 骷髅头/身与 humanoid 同盒同 UV（createMesh 复用，createDefaultSkeletonMesh
// 只替换四肢）。
static SKELETON_ARM: MobBox = bx([-1.0, -2.0, -1.0], [2.0, 12.0, 2.0], [40.0, 16.0]);
static SKELETON_LEG: MobBox = bx([-1.0, 0.0, -1.0], [2.0, 12.0, 2.0], [0.0, 16.0]);

// ---- 苦力怕（CreeperModel.createBodyLayer(NONE)，64x32） ---------------------

static CREEPER_HEAD: MobBox = bx([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], [0.0, 0.0]);
static CREEPER_BODY: MobBox = bx([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0]);
static CREEPER_LEG: MobBox = bx([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0]);

// ---- 蜘蛛（SpiderModel.createSpiderBodyLayer，64x32） ------------------------

static SPIDER_HEAD: MobBox = bx([-4.0, -4.0, -8.0], [8.0, 8.0, 8.0], [32.0, 4.0]);
static SPIDER_BODY0: MobBox = bx([-3.0, -3.0, -3.0], [6.0, 6.0, 6.0], [0.0, 0.0]);
static SPIDER_BODY1: MobBox = bx([-5.0, -4.0, -6.0], [10.0, 8.0, 12.0], [0.0, 12.0]);
// 右腿 addBox(-15,-1,-1,16,2,2)；左腿 mirror（UV 镜像，盒体占位不同——
// mirror 不换 min，这里照 Java 两份盒体分别登记）。
static SPIDER_LEG_R: MobBox = bx([-15.0, -1.0, -1.0], [16.0, 2.0, 2.0], [18.0, 0.0]);
static SPIDER_LEG_L: MobBox = mirrored(bx([-1.0, -1.0, -1.0], [16.0, 2.0, 2.0], [18.0, 0.0]));
/// 蜘蛛四对腿静态 zRot（SpiderModel:56-63；0.58119464 = 2π·33/360 原版常数）。
const SPIDER_LEG_Z_MID: f32 = 0.58119464;

/// 全部位表（下标序 = 鸡 8 / 牛 6 / 羊 12 / 猪 6 / 僵尸 6 / 骷髅 6 /
/// 苦力怕 6 / 蜘蛛 11，与 [`MOB_PART_COUNTS`] 一致；
/// 各部位 pivot/UV 全部照 Java 数字，来源行见模块头）。
static PARTS: [&[PartDef]; MOB_KIND_COUNT] = [
    // 鸡
    &[
        PartDef {
            pivot: [0.0, 15.0, -4.0],
            rot_x: 0.0,
            tex_layer: 0,
            anim: PartAnim::Head,
            boxes: &[CHICKEN_HEAD],
        },
        // 喙/红髯 = 头的 PartPose.ZERO 子部位：同 pivot、随头转（扁平化）。
        PartDef {
            pivot: [0.0, 15.0, -4.0],
            rot_x: 0.0,
            tex_layer: 0,
            anim: PartAnim::Head,
            boxes: &[CHICKEN_BEAK],
        },
        PartDef {
            pivot: [0.0, 15.0, -4.0],
            rot_x: 0.0,
            tex_layer: 0,
            anim: PartAnim::Head,
            boxes: &[CHICKEN_WATTLE],
        },
        PartDef {
            pivot: [0.0, 16.0, 0.0],
            rot_x: std::f32::consts::FRAC_PI_2,
            tex_layer: 0,
            anim: PartAnim::Fixed,
            boxes: &[CHICKEN_BODY],
        },
        PartDef {
            pivot: [-2.0, 19.0, 1.0],
            rot_x: 0.0,
            tex_layer: 0,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[CHICKEN_LEG],
        },
        PartDef {
            pivot: [1.0, 19.0, 1.0],
            rot_x: 0.0,
            tex_layer: 0,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[CHICKEN_LEG],
        },
        PartDef {
            pivot: [-4.0, 13.0, 0.0],
            rot_x: 0.0,
            tex_layer: 0,
            anim: PartAnim::Wing { sign: 1.0 },
            boxes: &[CHICKEN_R_WING],
        },
        PartDef {
            pivot: [4.0, 13.0, 0.0],
            rot_x: 0.0,
            tex_layer: 0,
            anim: PartAnim::Wing { sign: -1.0 },
            boxes: &[CHICKEN_L_WING],
        },
    ],
    // 牛
    &[
        PartDef {
            pivot: [0.0, 4.0, -8.0],
            rot_x: 0.0,
            tex_layer: 1,
            anim: PartAnim::Head,
            boxes: &[COW_HEAD, COW_SNOUT, COW_HORN_R, COW_HORN_L],
        },
        PartDef {
            pivot: [0.0, 5.0, 2.0],
            rot_x: std::f32::consts::FRAC_PI_2,
            tex_layer: 1,
            anim: PartAnim::Fixed,
            boxes: &[COW_BODY, COW_UDDER],
        },
        PartDef {
            pivot: [-4.0, 12.0, 7.0],
            rot_x: 0.0,
            tex_layer: 1,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[COW_LEG],
        },
        PartDef {
            pivot: [4.0, 12.0, 7.0],
            rot_x: 0.0,
            tex_layer: 1,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[mirrored(COW_LEG)],
        },
        PartDef {
            pivot: [-4.0, 12.0, -5.0],
            rot_x: 0.0,
            tex_layer: 1,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[COW_LEG],
        },
        PartDef {
            pivot: [4.0, 12.0, -5.0],
            rot_x: 0.0,
            tex_layer: 1,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[mirrored(COW_LEG)],
        },
    ],
    // 羊（0..5 基模 + 6..11 羊毛层；右腿 mirror）
    &[
        PartDef {
            pivot: [0.0, 6.0, -8.0],
            rot_x: 0.0,
            tex_layer: 2,
            anim: PartAnim::Head,
            boxes: &[SHEEP_HEAD],
        },
        PartDef {
            pivot: [0.0, 5.0, 2.0],
            rot_x: std::f32::consts::FRAC_PI_2,
            tex_layer: 2,
            anim: PartAnim::Fixed,
            boxes: &[SHEEP_BODY],
        },
        PartDef {
            pivot: [-3.0, 12.0, 7.0],
            rot_x: 0.0,
            tex_layer: 2,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[mirrored(SHEEP_LEG)],
        },
        PartDef {
            pivot: [3.0, 12.0, 7.0],
            rot_x: 0.0,
            tex_layer: 2,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[SHEEP_LEG],
        },
        PartDef {
            pivot: [-3.0, 12.0, -5.0],
            rot_x: 0.0,
            tex_layer: 2,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[mirrored(SHEEP_LEG)],
        },
        PartDef {
            pivot: [3.0, 12.0, -5.0],
            rot_x: 0.0,
            tex_layer: 2,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[SHEEP_LEG],
        },
        PartDef {
            pivot: [0.0, 6.0, -8.0],
            rot_x: 0.0,
            tex_layer: 3,
            anim: PartAnim::Head,
            boxes: &[SHEEP_WOOL_HEAD],
        },
        PartDef {
            pivot: [0.0, 5.0, 2.0],
            rot_x: std::f32::consts::FRAC_PI_2,
            tex_layer: 3,
            anim: PartAnim::Fixed,
            boxes: &[SHEEP_WOOL_BODY],
        },
        PartDef {
            pivot: [-3.0, 12.0, 7.0],
            rot_x: 0.0,
            tex_layer: 3,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[SHEEP_WOOL_LEG],
        },
        PartDef {
            pivot: [3.0, 12.0, 7.0],
            rot_x: 0.0,
            tex_layer: 3,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[SHEEP_WOOL_LEG],
        },
        PartDef {
            pivot: [-3.0, 12.0, -5.0],
            rot_x: 0.0,
            tex_layer: 3,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[SHEEP_WOOL_LEG],
        },
        PartDef {
            pivot: [3.0, 12.0, -5.0],
            rot_x: 0.0,
            tex_layer: 3,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[SHEEP_WOOL_LEG],
        },
    ],
    // 猪（左腿 mirror）
    &[
        PartDef {
            pivot: [0.0, 12.0, -6.0],
            rot_x: 0.0,
            tex_layer: 4,
            anim: PartAnim::Head,
            boxes: &[PIG_HEAD, PIG_SNOUT],
        },
        PartDef {
            pivot: [0.0, 11.0, 2.0],
            rot_x: std::f32::consts::FRAC_PI_2,
            tex_layer: 4,
            anim: PartAnim::Fixed,
            boxes: &[PIG_BODY],
        },
        PartDef {
            pivot: [-3.0, 18.0, 7.0],
            rot_x: 0.0,
            tex_layer: 4,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[PIG_LEG],
        },
        PartDef {
            pivot: [3.0, 18.0, 7.0],
            rot_x: 0.0,
            tex_layer: 4,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[mirrored(PIG_LEG)],
        },
        PartDef {
            pivot: [-3.0, 18.0, -5.0],
            rot_x: 0.0,
            tex_layer: 4,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[PIG_LEG],
        },
        PartDef {
            pivot: [3.0, 18.0, -5.0],
            rot_x: 0.0,
            tex_layer: 4,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[mirrored(PIG_LEG)],
        },
    ],
    // 僵尸（humanoid 六部件；举臂 = animateZombieArms 站立 armDrop=−π/2.25，
    // MC xRot 负 = 前举 → 本空间取 −drop 正角前摆；走路手臂原版不摆）
    &[
        PartDef {
            pivot: [0.0, 0.0, 0.0],
            rot_x: 0.0,
            tex_layer: 5,
            anim: PartAnim::Head,
            boxes: &[HUMANOID_HEAD],
        },
        PartDef {
            pivot: [0.0, 0.0, 0.0],
            rot_x: 0.0,
            tex_layer: 5,
            anim: PartAnim::Fixed,
            boxes: &[HUMANOID_BODY],
        },
        PartDef {
            pivot: [-5.0, 2.0, 0.0],
            rot_x: 0.0,
            tex_layer: 5,
            anim: PartAnim::ZombieArm {
                drop: -std::f32::consts::PI / 2.25,
            },
            boxes: &[ZOMBIE_ARM],
        },
        PartDef {
            pivot: [5.0, 2.0, 0.0],
            rot_x: 0.0,
            tex_layer: 5,
            anim: PartAnim::ZombieArm {
                drop: -std::f32::consts::PI / 2.25,
            },
            boxes: &[ZOMBIE_ARM_LEFT],
        },
        PartDef {
            pivot: [-1.9, 12.0, 0.0],
            rot_x: 0.0,
            tex_layer: 5,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[ZOMBIE_LEG],
        },
        PartDef {
            pivot: [1.9, 12.0, 0.0],
            rot_x: 0.0,
            tex_layer: 5,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[mirrored(ZOMBIE_LEG)],
        },
    ],
    // 骷髅（humanoid 骨架细四肢；行走摆臂摆腿同 HumanoidModel.setupAnim）
    &[
        PartDef {
            pivot: [0.0, 0.0, 0.0],
            rot_x: 0.0,
            tex_layer: 6,
            anim: PartAnim::Head,
            boxes: &[HUMANOID_HEAD],
        },
        PartDef {
            pivot: [0.0, 0.0, 0.0],
            rot_x: 0.0,
            tex_layer: 6,
            anim: PartAnim::Fixed,
            boxes: &[HUMANOID_BODY],
        },
        PartDef {
            pivot: [-5.0, 2.0, 0.0],
            rot_x: 0.0,
            tex_layer: 6,
            // HumanoidModel.setupAnim:223-226：右臂 = cos(pos·0.6662+π)
            // （与右腿 shift 0 反相）；左臂 = cos(pos·0.6662)（与左腿反相）。
            anim: PartAnim::WalkArm {
                shift: std::f32::consts::PI,
            },
            boxes: &[SKELETON_ARM],
        },
        PartDef {
            pivot: [5.0, 2.0, 0.0],
            rot_x: 0.0,
            tex_layer: 6,
            anim: PartAnim::WalkArm { shift: 0.0 },
            boxes: &[mirrored(SKELETON_ARM)],
        },
        PartDef {
            pivot: [-2.0, 12.0, 0.0],
            rot_x: 0.0,
            tex_layer: 6,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[SKELETON_LEG],
        },
        PartDef {
            pivot: [2.0, 12.0, 0.0],
            rot_x: 0.0,
            tex_layer: 6,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[mirrored(SKELETON_LEG)],
        },
    ],
    // 苦力怕（头/身同 pivot(0,6,0)；四腿对角同相，与四足 QuadrupedModel 同式）
    &[
        PartDef {
            pivot: [0.0, 6.0, 0.0],
            rot_x: 0.0,
            tex_layer: 7,
            anim: PartAnim::Head,
            boxes: &[CREEPER_HEAD],
        },
        PartDef {
            pivot: [0.0, 6.0, 0.0],
            rot_x: 0.0,
            tex_layer: 7,
            anim: PartAnim::Fixed,
            boxes: &[CREEPER_BODY],
        },
        PartDef {
            pivot: [-2.0, 18.0, 4.0],
            rot_x: 0.0,
            tex_layer: 7,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[CREEPER_LEG],
        },
        PartDef {
            pivot: [2.0, 18.0, 4.0],
            rot_x: 0.0,
            tex_layer: 7,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[mirrored(CREEPER_LEG)],
        },
        PartDef {
            pivot: [-2.0, 18.0, -4.0],
            rot_x: 0.0,
            tex_layer: 7,
            anim: PartAnim::WalkLeg {
                shift: std::f32::consts::PI,
            },
            boxes: &[CREEPER_LEG],
        },
        PartDef {
            pivot: [2.0, 18.0, -4.0],
            rot_x: 0.0,
            tex_layer: 7,
            anim: PartAnim::WalkLeg { shift: 0.0 },
            boxes: &[mirrored(CREEPER_LEG)],
        },
    ],
    // 蜘蛛（头 + 双体段 + 八腿；静态 y/z 转角与行走摆动照 SpiderModel 数字）
    &[
        PartDef {
            pivot: [0.0, 15.0, -3.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::Head,
            boxes: &[SPIDER_HEAD],
        },
        PartDef {
            pivot: [0.0, 15.0, 0.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::Fixed,
            boxes: &[SPIDER_BODY0],
        },
        PartDef {
            pivot: [0.0, 15.0, 9.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::Fixed,
            boxes: &[SPIDER_BODY1],
        },
        PartDef {
            pivot: [-4.0, 15.0, 2.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::SpiderLeg {
                side: 1.0,
                off: 0.0,
                y0: std::f32::consts::FRAC_PI_4,
                z0: -std::f32::consts::FRAC_PI_4,
            },
            boxes: &[SPIDER_LEG_R],
        },
        PartDef {
            pivot: [4.0, 15.0, 2.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::SpiderLeg {
                side: -1.0,
                off: 0.0,
                y0: -std::f32::consts::FRAC_PI_4,
                z0: std::f32::consts::FRAC_PI_4,
            },
            boxes: &[SPIDER_LEG_L],
        },
        PartDef {
            pivot: [-4.0, 15.0, 1.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::SpiderLeg {
                side: 1.0,
                off: std::f32::consts::PI,
                y0: std::f32::consts::FRAC_PI_8,
                z0: -SPIDER_LEG_Z_MID,
            },
            boxes: &[SPIDER_LEG_R],
        },
        PartDef {
            pivot: [4.0, 15.0, 1.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::SpiderLeg {
                side: -1.0,
                off: std::f32::consts::PI,
                y0: -std::f32::consts::FRAC_PI_8,
                z0: SPIDER_LEG_Z_MID,
            },
            boxes: &[SPIDER_LEG_L],
        },
        PartDef {
            pivot: [-4.0, 15.0, 0.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::SpiderLeg {
                side: 1.0,
                off: std::f32::consts::FRAC_PI_2,
                y0: -std::f32::consts::FRAC_PI_8,
                z0: -SPIDER_LEG_Z_MID,
            },
            boxes: &[SPIDER_LEG_R],
        },
        PartDef {
            pivot: [4.0, 15.0, 0.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::SpiderLeg {
                side: -1.0,
                off: std::f32::consts::FRAC_PI_2,
                y0: std::f32::consts::FRAC_PI_8,
                z0: SPIDER_LEG_Z_MID,
            },
            boxes: &[SPIDER_LEG_L],
        },
        PartDef {
            pivot: [-4.0, 15.0, -1.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::SpiderLeg {
                side: 1.0,
                off: 3.0 * std::f32::consts::FRAC_PI_2,
                y0: -std::f32::consts::FRAC_PI_4,
                z0: -std::f32::consts::FRAC_PI_4,
            },
            boxes: &[SPIDER_LEG_R],
        },
        PartDef {
            pivot: [4.0, 15.0, -1.0],
            rot_x: 0.0,
            tex_layer: 8,
            anim: PartAnim::SpiderLeg {
                side: -1.0,
                off: 3.0 * std::f32::consts::FRAC_PI_2,
                y0: std::f32::consts::FRAC_PI_4,
                z0: std::f32::consts::FRAC_PI_4,
            },
            boxes: &[SPIDER_LEG_L],
        },
    ],
];

/// 每帧姿态（由游戏侧计算后传入渲染；与 `PlayerPose` 同约定：脚底原点、
/// yaw 与 `Camera::dir` 同型）。
#[derive(Clone, Copy, Debug)]
pub struct MobPose {
    /// 脚底世界坐标。
    pub pos: Vec3,
    /// 身体 yaw（弧度）。
    pub yaw: f32,
    /// 行走摆动相位/幅值（`player_mesh::update_walk_animation` 的输出直传）。
    pub phase: f32,
    pub amount: f32,
    /// 头俯仰（弧度，正 = 抬头，同 player.pitch 约定）。
    pub head_pitch: f32,
    /// 鸡翅展开角（弧度；= (sin(flap)+1)·flapSpeed，右翼 + 左翼 −）。
    pub wing_angle: f32,
    /// 羊吃草低头量 0..1（SheepModel：head.y += scale·9 像素）。
    pub head_drop: f32,
    /// 羊吃草头部 xRot（弧度，MC 语义正 = 低头，getHeadEatAngleScale）。
    pub head_eat_angle: f32,
}

impl Default for MobPose {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            yaw: 0.0,
            phase: 0.0,
            amount: 0.0,
            head_pitch: 0.0,
            wing_angle: 0.0,
            head_drop: 0.0,
            head_eat_angle: 0.0,
        }
    }
}

/// 合并网格：四种生物顶点共用一个缓冲（`meta.x` = 贴图层、`meta.y` = 部位
/// 在**单种内**的序号，每 kind 从 0 起，< [`MAX_MOB_PARTS`]——与
/// `mob_model_matrices` 输出的局部槽位和 shader `models[12]` 数组同构），
/// 逐 kind 连续切片。
pub struct MobMesh {
    pub verts: Vec<PlayerVertex>,
    pub indices: Vec<u32>,
    /// `slices[kind]` = 该生物全部部位（含羊毛层）的索引区间。
    pub slices: [std::ops::Range<u32>; MOB_KIND_COUNT],
}

/// UV 水平镜像：该角 s → 1−s（见 [`MobBox::mirror`] 注释）。
fn remap(st: (f32, f32), mirror: bool) -> (f32, f32) {
    if mirror { (1.0 - st.0, st.1) } else { st }
}

/// 展开一个部位的所有盒体为 24 顶点/盒 + 索引。`part_index` = 该部位在
/// 单种内的序号（meta.y，模型矩阵数组下标；每 kind 从 0 重置）。
fn push_part(m: &mut MobMesh, def: &PartDef, part_index: usize) {
    for b in def.boxes {
        let (min, size) = if b.grow != 0.0 {
            (
                [b.min[0] - b.grow, b.min[1] - b.grow, b.min[2] - b.grow],
                [
                    b.size[0] + 2.0 * b.grow,
                    b.size[1] + 2.0 * b.grow,
                    b.size[2] + 2.0 * b.grow,
                ],
            )
        } else {
            (b.min, b.size)
        };
        let faces = quadruped_faces(b.tex[0], b.tex[1], b.size[0], b.size[1], b.size[2]);
        let base = m.verts.len() as u32;
        for face in faces.iter() {
            for k in 0..4 {
                let ci = face.corner(k);
                let (su, tv) = remap(face.st(k), b.mirror);
                let px = min[0] + if ci & 1 != 0 { size[0] } else { 0.0 };
                let py = min[1] + if ci & 2 != 0 { size[1] } else { 0.0 };
                let pz = min[2] + if ci & 4 != 0 { size[2] } else { 0.0 };
                let uu = face.uv[0] + (face.uv[2] - face.uv[0]) * su;
                let vv = face.uv[1] + (face.uv[3] - face.uv[1]) * tv;
                m.verts.push(PlayerVertex {
                    // 四足约定：x 镜像 + y 翻转，z 保持（见模块头）。
                    pos: [-px * PX, -py * PX, pz * PX],
                    uv: [
                        (uu * 255.0 / MOB_TEX_PX as f32).round() as u8,
                        (vv * 255.0 / MOB_TEX_PX as f32).round() as u8,
                    ],
                    _pad: [0; 2],
                    meta: [def.tex_layer, part_index as u32],
                });
            }
        }
        for f in 0..6u32 {
            let vb = base + f * 4;
            m.indices
                .extend_from_slice(&[vb, vb + 1, vb + 2, vb, vb + 2, vb + 3]);
        }
    }
}

/// 内部累积器（`part_index` 在部位间推进）。
struct MobMeshBuilder {
    mesh: MobMesh,
    part_index: usize,
}

impl MobMeshBuilder {
    fn part(&mut self, def: &PartDef) {
        push_part(&mut self.mesh, def, self.part_index);
        self.part_index += 1;
    }
}

/// 构建四种生物的合并网格。
pub fn build_mob_mesh() -> MobMesh {
    let cap_boxes: usize = PARTS
        .iter()
        .map(|parts| parts.iter().map(|p| p.boxes.len()).sum::<usize>())
        .sum();
    let mut b = MobMeshBuilder {
        mesh: MobMesh {
            verts: Vec::with_capacity(cap_boxes * 24),
            indices: Vec::with_capacity(cap_boxes * 36),
            slices: std::array::from_fn(|_| 0..0),
        },
        part_index: 0,
    };
    for (k, parts) in PARTS.iter().enumerate() {
        debug_assert_eq!(parts.len(), MOB_PART_COUNTS[k]);
        // meta.y 必须是 kind 内局部序：shader `models[meta.y]` 数组仅
        // MAX_MOB_PARTS=12 槽，实例 uniform 由 mob_model_matrices 按
        // 局部序填充（跨种全局序会让羊/猪越界读零矩阵 → 部位塌缩）。
        debug_assert!(parts.len() <= MAX_MOB_PARTS);
        b.part_index = 0;
        let start = b.mesh.indices.len() as u32;
        for def in parts.iter() {
            b.part(def);
        }
        b.mesh.slices[k] = start..b.mesh.indices.len() as u32;
    }
    b.mesh
}

/// 计算一个生物实例的部位模型矩阵（只填前 `MOB_PART_COUNTS[kind]` 个，
/// 其余单位阵——shader 按 `meta.y` 取，不会访问到）。
///
/// yaw 用 `RotY(−yaw)`：四足正面（喙/鼻/脸段）在模型 −z，本引擎朝向角约定
/// `f(yaw) = (sin yaw, 0, −cos yaw)`（与 `Camera::dir` 一致）——把正面方向
/// −ẑ 旋到 f(yaw) 需要 θ = −yaw。（玩家管线用 `RotY(+yaw)` 且脸段同样在
/// 引擎 −z，其水平朝向与本约定互为镜像——既有行为，见报告
/// 「派单说法 vs 实况」；生物侧以 `Camera::dir` 为准取 −yaw。）
pub fn mob_model_matrices(kind: MobModelKind, pose: &MobPose) -> [Mat4; MAX_MOB_PARTS] {
    let k = kind.idx();
    let parts = PARTS[k];
    let ry = Mat4::from_axis_angle(Vec3::Y, -pose.yaw);
    // 腿摆 = cos(phase+shift)·amount·增益（QuadrupedModel.setupAnim 的
    // pos·0.6662 已被 phase 吸收，见模块头）。
    let leg = |shift: f32| (pose.phase + shift).cos() * pose.amount * WALK_GAIN;
    // 蜘蛛速度因子：MC walkAnimationSpeed（0..1）≈ amount/0.88（与
    // WALK_GAIN 同一套幅值换算）。
    let spd = pose.amount / 0.88;
    let mut out = [Mat4::IDENTITY; MAX_MOB_PARTS];
    for (p, def) in parts.iter().enumerate() {
        // 部位旋转的引擎局部三轴角（ax, ay, az）：MC rotationZYX(z,y,x)
        // 经共轭 Q·R_mc·Q⁻¹（Q: (x,y,z)→(−x,−y,+z)）逐轴映射——绕 MC x 的
        // θ → 绕本空间 x 的 −θ；绕 MC y 的 θ → −θ（Q 把 MC +ŷ 映到 −ŷ）；
        // 绕 z 的 θ 不变号；乘序保持 ⇒ 本空间 R_eng = Rz(z)·Ry(y)·Rx(x)。
        let (ax, ay, az) = match def.anim {
            // 静态转轴：Java 的 rot_x=π/2（QuadrupedModel.createBodyMesh
            // 等）在本空间取 −π/2——符号错了躯干会整段悬空翻转（牛躯干
            // 将到 y 16..26px，正确为 12..22px，见 cow_body_lies_horizontal
            // 测试）。
            PartAnim::Fixed => (-def.rot_x, 0.0, 0.0),
            // 头俯仰（正=抬头）；MC 头 xRot 正=低头，符号相反恰好抵消，
            // 故 head_pitch 直传。羊吃草叠加低头（MC 语义正=低头 → 取负）。
            PartAnim::Head => (pose.head_pitch - pose.head_eat_angle, 0.0, 0.0),
            // 腿摆与玩家管线同约定（+ 角 = 前摆，player_mesh model_matrices
            // rots 表同款）；对角同相 pairing 与 Java 一致，cos 对称下与严格
            // 共轭（−θ）仅差相位原点，视觉等价。
            PartAnim::WalkLeg { shift } => (leg(shift), 0.0, 0.0),
            // 人形臂（HumanoidModel.setupAnim:223-226）：MC 摆幅 1.0·k =
            // 腿 1.4·k × (1/1.4)，与同侧腿反相（shift 差 π，见 PARTS 表）。
            PartAnim::WalkArm { shift } => (leg(shift) / 1.4, 0.0, 0.0),
            // 僵尸举臂（animateZombieArms:75 armDrop，站立 = −π/2.25）：
            // MC xRot 负 = 前举 → 本空间 −drop 正角。
            PartAnim::ZombieArm { drop } => (-drop, 0.0, 0.0),
            // 蜘蛛腿（SpiderModel.setupAnim:73-96）：yRot 摆
            // = −cos(2·apos+off)·0.4·k（右 + 左 −）、zRot 步进
            // = |sin(apos+off)|·0.4·k（右 + 左 −），apos=phase、k=spd；
            // 静态 y0/z0 照 Java。本空间 ay = −y_mc、az = z_mc。
            PartAnim::SpiderLeg { side, off, y0, z0 } => {
                let swing = -(2.0 * pose.phase + off).cos() * 0.4 * spd;
                let step = (pose.phase + off).sin().abs() * 0.4 * spd;
                (0.0, -(y0 + side * swing), z0 + side * step)
            }
            // 鸡翅绕局部 z（ChickenModel.setupAnim:30-31：右 + 左 −）。
            // Q 不改变 z 轴 ⇒ 绕 z 的旋转角不变号。
            PartAnim::Wing { sign } => (0.0, 0.0, sign * pose.wing_angle),
        };
        // 羊吃草头部下沉（SheepModel.setupAnim：head.y += scale·9 像素）。
        let drop = if matches!(def.anim, PartAnim::Head) {
            pose.head_drop * 9.0
        } else {
            0.0
        };
        let off = Vec3::new(
            -def.pivot[0] * PX,
            (24.0 - def.pivot[1] - drop) * PX,
            def.pivot[2] * PX, // 四足 z 不取反（正面朝 −z，见模块头）
        );
        let world_piv = pose.pos + ry.transform_point3(off);
        // 旋转必须绕**引擎局部轴**构造、再被 ry 左乘：out = T·ry·Rz·Ry·Rx。
        // 旧实现把轴先旋到世界（Rot(ry·x̂, θ)），得 out = T·ry²·R·ry⁻¹——
        // 部位自转被按 2·yaw 共轭，yaw≠0 时腿摆方向侧翻（被动模型在
        // Scene.mobs 常空时代从未以 yaw≠0 上屏，故未暴露；接通 mobs 后
        // 由 mob_gpu 的 yaw 回归测试锁死）。
        let r = Mat4::from_axis_angle(Vec3::Z, az)
            * Mat4::from_axis_angle(Vec3::Y, ay)
            * Mat4::from_axis_angle(Vec3::X, ax);
        out[p] = Mat4::from_translation(world_piv) * ry * r;
    }
    out
}

/// 鸡的扑翼推进（Chicken.java:117-129，tick 语义；游戏侧每 tick 调一次）。
/// flapSpeed 账本与 Java 逐字一致：`+(onGround?-1:4)×0.3` 后 clamp 0..1
/// （117-118）。`flap` 相位为**简化账本**：Java 另有 `flapping` 状态
/// （空中钳到 1 后 ×0.9 → 每 tick 实增 1.8；落地 ×0.9ⁿ 渐停，119-123、
/// 129），本实现不跟踪 flapping，取空中恒 +2.0、落地恒 0——起停比原版
/// 略硬，wing_angle 幅值域不变（M7b 不要求动画完整，见任务报告）。
/// 返回 (新 flap, 新 flapSpeed)；wing_angle = (sin(flap)+1)·flapSpeed。
pub fn update_wing_animation(flap: f32, flap_speed: f32, on_ground: bool) -> (f32, f32) {
    let mut fs = flap_speed + (if on_ground { -1.0 } else { 4.0 }) * 0.3;
    fs = fs.clamp(0.0, 1.0);
    let flapping = if !on_ground { 1.0 } else { 0.0 };
    let f = flap + flapping * 2.0;
    (f, fs)
}

/// 从资源根加载并解码 mob 贴图数组（全部 pad 到 64x64x9 RGBA）。
/// 任一文件缺失/解码失败/尺寸不符 → None（上层按缺素材降级不渲染；
/// 绝不用程序化占位顶替原版贴图）。M8c：读取经 AssetManager（缺文件
/// 硬错误显式 log，不再静默 `.ok()?` 吞掉）。
pub fn load_mob_payload(assets_dir: Option<&std::path::Path>) -> Option<Vec<u8>> {
    let dir = assets_dir?;
    load_mob_payload_via(&mcv_assets::AssetManager::new(dir))
}

/// 经 AssetManager 加载（统一缓存/缺素材登记；Renderer 构造路径用）。
pub fn load_mob_payload_via(assets: &mcv_assets::AssetManager) -> Option<Vec<u8>> {
    let mut out = vec![0u8; MOB_TEX_LAYERS * MOB_TEX_PX * MOB_TEX_PX * 4];
    for (layer, file) in MOB_TEX_FILES.iter().enumerate() {
        let Ok(bytes) = assets.read(&format!("textures/{file}")) else {
            log::error!("mob 贴图缺失: textures/{file}（相对资源根）——生物不渲染（无程序化占位）");
            return None;
        };
        let Ok(img) = image::load_from_memory(&bytes) else {
            log::error!("mob 贴图解码失败: textures/{file}——生物不渲染");
            return None;
        };
        let img = img.to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        // 原版尺寸门：鸡/羊 64x32、牛/猪 64x64（LayerDefinition.create 的
        // (w,h) 参数）。pad 到 64x64 后 v 坐标不变（v=像素/64 归一化）。
        let ok = matches!((w, h), (64, 32) | (64, 64));
        if !ok {
            log::warn!("mob texture {file}: unexpected {w}x{h}");
            return None;
        }
        // into_raw 只取一次（消费 img），循环内按行拷贝 pad。
        let raw = img.into_raw();
        for y in 0..h {
            for x in 0..w {
                let s = (y * w + x) * 4;
                let d = ((y * MOB_TEX_PX) + x) * 4;
                out[layer * MOB_TEX_PX * MOB_TEX_PX * 4 + d..][..4].copy_from_slice(&raw[s..s + 4]);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_table_matches_java_counts() {
        let m = build_mob_mesh();
        for (k, r) in m.slices.iter().enumerate() {
            let boxes = PARTS[k].iter().map(|p| p.boxes.len()).sum::<usize>();
            assert_eq!(r.len(), boxes * 36, "kind {k} index span");
        }
        // 总盒数：鸡 8 + 牛 10（头 4 含双角 + 躯干 2 含乳房 + 腿 4）
        // + 羊 12 + 猪 7（头 2 含鼻 + 躯干 1 + 腿 4）
        // + 僵尸 6（humanoid 六部件）+ 骷髅 6 + 苦力怕 6 + 蜘蛛 11
        // = 66
        assert_eq!(m.verts.len(), 66 * 24);
        assert_eq!(m.indices.len(), 66 * 36);
        // kind 切片连续
        let mut prev = 0u32;
        for r in m.slices.iter() {
            assert_eq!(r.start, prev);
            prev = r.end;
        }
    }

    #[test]
    fn part_matrices_rigidly_covary_with_yaw() {
        // 矩阵合成回归锁（旧缺陷 T·ry²·Rx·ry⁻¹ 在 yaw≠0 把腿摆侧翻）：
        // 标准式 out = T(world_piv)·ry·Rz·Ry·Rx 下部位旋转必须满足刚性
        // 协变 matrix3(yaw) = Ry(−yaw)·matrix3(0)，即部位自转与朝向解耦、
        // 整体像刚体绕 y 轴旋转。旧共轭式不满足（自转被 2·yaw 共轭）。
        let m3 = |m: &Mat4| {
            glam::Mat3::from_cols(m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate())
        };
        let mk = |yaw: f32| {
            mob_model_matrices(
                MobModelKind::Zombie,
                &MobPose {
                    yaw,
                    phase: 0.9,
                    amount: 0.88,
                    ..Default::default()
                },
            )
        };
        let base = mk(0.0);
        for yaw in [std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
            let ry3 = m3(&Mat4::from_axis_angle(Vec3::Y, -yaw));
            let rot = mk(yaw);
            for (p, (b, r)) in base.iter().zip(rot.iter()).enumerate() {
                let want = ry3 * m3(b);
                let got = m3(r);
                for (i, row) in want.to_cols_array_2d().iter().enumerate() {
                    for (j, w) in row.iter().enumerate() {
                        assert!(
                            (w - got.to_cols_array_2d()[i][j]).abs() < 1e-4,
                            "yaw {yaw} part {p} col {i} row {j}: {w} vs {}",
                            got.to_cols_array_2d()[i][j]
                        );
                    }
                }
            }
            // 物理兜底：最大摆幅下腿尖仍垂在髋下（竖直分量不消失 = 不横躺）。
            let hip = rot[4].transform_point3(Vec3::ZERO);
            let tip = rot[4].transform_point3(Vec3::new(0.0, -0.75, 0.0));
            assert!(
                tip.y < hip.y - 0.1 && tip.y > hip.y - 0.76,
                "yaw {yaw} leg tip {tip} hip {hip}"
            );
        }
    }

    #[test]
    fn zombie_left_arm_is_independent_asymmetric_box() {
        // 26.1 HumanoidModel.java:101 right_arm addBox(-3,-2,-2,…)、
        // :105 left_arm mirror().addBox(-1,-2,-2,…)——mirror 只翻 UV，
        // 盒坐标不对称。共用右臂盒会让左臂世界 x∈[2,6]px 压进躯干。
        assert_eq!(ZOMBIE_ARM.min[0], -3.0);
        assert_eq!(ZOMBIE_ARM_LEFT.min[0], -1.0);
        let m = build_mob_mesh();
        let pose = MobPose::default();
        let mm = mob_model_matrices(MobModelKind::Zombie, &pose);
        let r = m.slices[MobModelKind::Zombie as usize];
        // 部位序：头0 身1 右臂2 左臂3，各 24 顶点。
        let x_span = |part: usize| {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for v in &m.verts[r.start as usize + part * 24..r.start as usize + (part + 1) * 24] {
                let w = mm[part].transform_point3(Vec3::from(v.pos));
                lo = lo.min(w.x);
                hi = hi.max(w.x);
            }
            (lo, hi)
        };
        let (rlo, rhi) = x_span(2);
        let (llo, lhi) = x_span(3);
        // 世界米制：右臂 [−8,−4]px、左臂 [4,8]px，对 y 轴镜像对称。
        assert!((rlo - -0.5).abs() < 1e-5 && (rhi - -0.25).abs() < 1e-5, "{rlo} {rhi}");
        assert!((llo - 0.25).abs() < 1e-5 && (lhi - 0.5).abs() < 1e-5, "{llo} {lhi}");
    }

    #[test]
    fn chicken_head_matches_java_numbers() {
        // AdultChickenModel：头 box(-2,-6,-2,4,6,3) pivot(0,15,-4)。
        // 静止姿态世界范围：x ±0.125；y (24-15)/16..(24-9)/16 = 0.5625..0.9375；
        // z：pivot −4px + 盒 z(−2..1) → −0.375..−0.1875（头在原点前方）。
        let m = build_mob_mesh();
        let pose = MobPose::default();
        let mm = mob_model_matrices(MobModelKind::Chicken, &pose);
        // 顶点序：部位 0（头）前 24 顶点
        let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
        for v in &m.verts[0..24] {
            assert_eq!(v.meta, [0, 0], "meta.y 必须是 kind 内局部序");
            let w = mm[0].transform_point3(Vec3::from(v.pos));
            lo = lo.min(w);
            hi = hi.max(w);
        }
        assert!((lo.x - -0.125).abs() < 1e-5 && (hi.x - 0.125).abs() < 1e-5);
        assert!((lo.y - 0.5625).abs() < 1e-5 && (hi.y - 0.9375).abs() < 1e-5);
        assert!(
            (lo.z - -0.375).abs() < 1e-5 && (hi.z - -0.1875).abs() < 1e-5,
            "{lo} {hi}"
        );
        // 正脸（−z 侧，face-with-eyes 段）UV = [u0+d, v0+d, u0+d+w, v0+d+h]
        // = (3,3)-(7,9)（tex(0,0) 4x6x3）。
        for v in &m.verts[16..20] {
            assert!(v.pos[2] < 0.0, "face corners must sit at −z");
            let u = v.uv[0] as f32 / 255.0 * 64.0;
            let t = v.uv[1] as f32 / 255.0 * 64.0;
            assert!(u > 2.9 && u < 7.1, "chicken face u {u}");
            assert!(t > 2.9 && t < 9.1, "chicken face v {t}");
        }
    }

    #[test]
    fn cow_body_lies_horizontal() {
        // CowModel 躯干 xRot=π/2（共轭后本空间绕 x 取 −π/2，y↔z 互换）：
        // box(-6,-10,-7,12,18,10) 转平。世界 y = (24-5) + 盒 pz（-7..3）
        // → 12..22px = 0.75..1.375；乳房盒 pz∈-8..-7 → 11..12px → 0.6875。
        // x = ±6px = ±0.375。
        let m = build_mob_mesh();
        let pose = MobPose::default();
        let mm = mob_model_matrices(MobModelKind::Cow, &pose);
        // 部位顶点按 (部位, 面, 角) 顺序：羊之前 = 鸡 8 部位 + 牛头（4 盒
        // × 24 = 96 顶点）→ 牛躯干（含乳房，2 盒 48 顶点）在 192+96=288 起。
        let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
        for v in &m.verts[288..288 + 48] {
            assert_eq!(
                v.meta[1] as usize, 1,
                "牛躯干部位序（kind 内局部：头 0 躯干 1）"
            );
            let w = mm[1].transform_point3(Vec3::from(v.pos));
            lo = lo.min(w);
            hi = hi.max(w);
        }
        assert!((lo.x - -0.375).abs() < 1e-5 && (hi.x - 0.375).abs() < 1e-5);
        assert!(
            (lo.y - 0.6875).abs() < 1e-5 && (hi.y - 1.375).abs() < 1e-5,
            "{lo} {hi}"
        );
    }

    #[test]
    fn sheep_wool_layer_uses_wool_texture() {
        let m = build_mob_mesh();
        // 羊基模 6 部位（层 2），羊毛 6 部位（层 3）。顶点偏移按「盒」数
        // 累积：鸡 8 盒 + 牛 10 盒（头 4 含双角 + 躯干 2 含乳房 + 腿 4）
        // = 18 盒 → 羊起于 18×24=432；羊 12 盒 → 基模 432..576、羊毛
        // 576..720。
        let base = (8 + 10) * 24;
        for (i, v) in m.verts[base..base + 6 * 24].iter().enumerate() {
            assert_eq!(v.meta[0], 2, "羊基模层号");
            assert_eq!(v.meta[1] as usize, i / 24, "羊基模局部部位序");
        }
        for (i, v) in m.verts[base + 6 * 24..base + 12 * 24].iter().enumerate() {
            assert_eq!(v.meta[0], 3, "羊毛层层号");
            assert_eq!(v.meta[1] as usize, 6 + i / 24, "羊毛层局部部位序");
        }
    }

    #[test]
    fn pig_snout_pokes_forward() {
        // PigModel 鼻 box(-2,0,-9,4,3,1)：比头（盒 z -8..0）更前 1px，
        // 世界 -z。头 pivot z=-6px：世界 z = -6 + 盒 z → 鼻尖 -15px。
        let m = build_mob_mesh();
        let pose = MobPose::default();
        let mm = mob_model_matrices(MobModelKind::Pig, &pose);
        // 顶点偏移按「盒」数累积：鸡 8 + 牛 10 + 羊 12 = 30 盒 → 猪起于
        // 30×24=720；头部位（2 盒）占 720..768。z 最小值 = -15px = -0.9375。
        let base = (8 + 10 + 12) * 24;
        let mut min_z = f32::MAX;
        for v in &m.verts[base..base + 48] {
            let w = mm[0] * Vec3::from(v.pos).extend(1.0);
            min_z = min_z.min(w.z);
        }
        assert!((min_z - -0.9375).abs() < 1e-5, "pig snout z {min_z}");
    }

    #[test]
    fn walk_legs_swing_opposite_and_bounded() {
        let pose = MobPose {
            phase: 0.0,
            amount: 0.88,
            ..Default::default()
        };
        let mm = mob_model_matrices(MobModelKind::Cow, &pose);
        // 牛右后腿（部位 2，pivot z=+7px）前摆、左后腿（部位 3）后摆
        // （对角同相，QuadrupedModel.setupAnim）。
        let tip = |p: usize| mm[p].transform_point3(Vec3::new(0.0, -0.75, 0.0)).z;
        assert!(tip(2) < 0.0 && tip(3) > 0.0);
        // 满速摆幅 = 1.4 rad（0.88·WALK_GAIN）：tip z = pivot 7px −
        // sin(1.4)·0.75（pivot 平移 + 摆动分量都核算）。
        let swing = 0.75 * 1.4f32.sin();
        assert!(
            (tip(2) - (0.4375 - swing)).abs() < 1e-4,
            "right hind tip {}",
            tip(2)
        );
        // 反相：phase π 时对调
        let pose2 = MobPose {
            phase: std::f32::consts::PI,
            amount: 0.88,
            ..Default::default()
        };
        let mm2 = mob_model_matrices(MobModelKind::Cow, &pose2);
        assert!(mm2[2].transform_point3(Vec3::new(0.0, -0.75, 0.0)).z > 0.0);
    }

    #[test]
    fn wing_flaps_and_rests() {
        // 空中：flapSpeed→1，flap 每 tick +2；wing_angle ∈ 0..2。
        let (mut f, mut fs) = (0.0f32, 0.0f32);
        for _ in 0..10 {
            (f, fs) = update_wing_animation(f, fs, false);
        }
        assert_eq!(fs, 1.0);
        let angle = (f.sin() + 1.0) * fs;
        assert!((0.0..=2.0).contains(&angle));
        // 落地：flapSpeed 收拢到 0、flap 停止推进。
        for _ in 0..10 {
            (f, fs) = update_wing_animation(f, fs, true);
        }
        assert_eq!(fs, 0.0);
        let f2 = update_wing_animation(f, fs, true).0;
        assert_eq!(f2, f, "落地 flap 不再推进");
    }

    #[test]
    fn sheep_eat_drops_head() {
        let base = MobPose::default();
        let eating = MobPose {
            head_drop: 1.0,
            head_eat_angle: 1.0,
            ..Default::default()
        };
        let mm0 = mob_model_matrices(MobModelKind::Sheep, &base);
        let mm1 = mob_model_matrices(MobModelKind::Sheep, &eating);
        let y = |mm: &[Mat4; MAX_MOB_PARTS]| mm[0].transform_point3(Vec3::ZERO).y;
        // 精确下沉量 = 9px·PX（SheepModel：head.y += scale·9）。
        assert!(
            (y(&mm0) - y(&mm1) - 9.0 * PX).abs() < 1e-5,
            "吃草时头 pivot 应精确下沉 9px"
        );
    }

    #[test]
    fn uv_pixels_within_texture() {
        let m = build_mob_mesh();
        for v in &m.verts {
            for c in v.uv {
                let px = c as f32 / 255.0 * MOB_TEX_PX as f32;
                assert!((0.0..=MOB_TEX_PX as f32).contains(&px));
            }
            assert!((v.meta[0] as usize) < MOB_TEX_LAYERS);
        }
    }

    // ---- 敌对四怪 -----------------------------------------------------------

    /// kind 首部位（头）的顶点区间起点：切点 = 索引切片起点 / 36 盒 × 24。
    fn head_vert_range(m: &MobMesh, k: usize) -> std::ops::Range<usize> {
        let base = m.slices[k].start as usize / 36 * 24;
        base..base + 24
    }

    /// 断言盒体正脸段（face-with-eyes，UV = [u+d, v+d, u+d+w, v+d+h]）：
    /// 四角顶点必须落在 −z 面、UV 像素坐标落在给定矩形内。容差 0.14 px =
    /// u8 量化的半 LSB（64/255/2 ≈ 0.126）：rect 边界角如 v=16 编码为
    /// round(16·255/64)=64 → 回读 16.063。
    fn assert_face_rect(m: &MobMesh, k: usize, rect: [f32; 4]) {
        let r = head_vert_range(m, k);
        for v in &m.verts[r.start + 16..r.start + 20] {
            assert!(v.pos[2] < 0.0, "face corners must sit at −z");
            let u = v.uv[0] as f32 / 255.0 * MOB_TEX_PX as f32;
            let t = v.uv[1] as f32 / 255.0 * MOB_TEX_PX as f32;
            assert!(
                u >= rect[0] - 0.14 && u <= rect[2] + 0.14,
                "face u {u} not in {}..{}",
                rect[0],
                rect[2]
            );
            assert!(
                t >= rect[1] - 0.14 && t <= rect[3] + 0.14,
                "face t {t} not in {}..{}",
                rect[1],
                rect[3]
            );
        }
    }

    #[test]
    fn hostile_head_face_rects_match_java() {
        let m = build_mob_mesh();
        // 僵尸/骷髅/苦力怕头 tex(0,0) 8x8x8 → 正脸段 (8,8)-(16,16)
        // （HumanoidModel.createMesh:94 / CreeperModel:34）。
        for k in [4, 5, 6] {
            assert_face_rect(&m, k, [8.0, 8.0, 16.0, 16.0]);
        }
        // 蜘蛛头 tex(32,4) 8x8x8（SpiderModel:47）→ (40,12)-(48,20)。
        assert_face_rect(&m, 7, [40.0, 12.0, 48.0, 20.0]);
        // 层号：头顶点 meta.x = 各自贴图层（5..8）。
        for (k, layer) in [(4usize, 5u32), (5, 6), (6, 7), (7, 8)] {
            for v in &m.verts[head_vert_range(&m, k)] {
                assert_eq!(v.meta[0], layer);
            }
        }
    }

    #[test]
    fn hostile_foot_lines_up_at_y_zero() {
        // 脚底统一在模型 y=24（与玩家/四足同一根约定）：僵尸腿 pivot y=12
        // 盒 12px、骷髅同、苦力怕腿 pivot y=18 盒 6px、蜘蛛 body1 底
        // y=15+4=19……腿部位静止姿态最低点都应触地（y=0）。
        let m = build_mob_mesh();
        let pose = MobPose::default();
        for k in [4usize, 5, 6] {
            let mm = mob_model_matrices(MobModelKind::from_idx(k as u32).unwrap(), &pose);
            let base = m.slices[k].start as usize / 36 * 24;
            let end = base + m.slices[k].len() / 36 * 24;
            let mut min_y = f32::MAX;
            for v in &m.verts[base..end] {
                let w = mm[v.meta[1] as usize].transform_point3(Vec3::from(v.pos));
                min_y = min_y.min(w.y);
            }
            assert!(min_y.abs() < 1e-5, "kind {k} 脚底应触地，min_y {min_y}");
        }
    }

    #[test]
    fn zombie_arms_raised_forward_legs_swing() {
        let pose = MobPose {
            phase: 0.0,
            amount: 0.88,
            ..Default::default()
        };
        let mm = mob_model_matrices(MobModelKind::Zombie, &pose);
        // 举臂：animateZombieArms 站立 armDrop = −π/2.25 → 本空间 +π/2.25，
        // 臂尖（局部 (0,−0.75,0)）指向前方 −z 且明显高于垂臂。
        let arm_tip = mm[2].transform_point3(Vec3::new(0.0, -0.75, 0.0));
        assert!(arm_tip.z < -0.6, "僵尸臂应前举，tip {arm_tip}");
        assert!(arm_tip.y > 1.0, "僵尸臂应抬起，tip {arm_tip}");
        // 行走腿：右腿（部位 4，shift 0）前摆、左腿后摆。
        let tip = |p: usize| mm[p].transform_point3(Vec3::new(0.0, -0.75, 0.0)).z;
        assert!(tip(4) < 0.0 && tip(5) > 0.0);
    }

    #[test]
    fn skeleton_arms_swing_opposite_legs() {
        let pose = MobPose {
            phase: 0.0,
            amount: 0.88,
            ..Default::default()
        };
        let mm = mob_model_matrices(MobModelKind::Skeleton, &pose);
        // HumanoidModel.setupAnim：右臂与右腿反相（右臂 shift π、右腿 0）。
        let z = |p: usize| mm[p].transform_point3(Vec3::new(0.0, -0.75, 0.0)).z;
        assert!(z(4) * z(2) < 0.0, "右臂应与右腿反相");
        assert!(z(5) * z(3) < 0.0, "左臂应与左腿反相");
        // 臂角/腿角 = 1/1.4（pivot z 同为 0，由尖端位移反解摆角核算；
        // 部位序：2=右臂 3=左臂 4=右腿 5=左腿）。
        let ang = |zz: f32| (zz.abs() / 0.75).asin();
        let (leg, arm) = (ang(z(4)), ang(z(2)));
        assert!((arm * 1.4 - leg).abs() < 1e-4, "arm {arm} leg {leg}");
    }

    #[test]
    fn creeper_legs_diagonal_and_body_stack() {
        let m = build_mob_mesh();
        let pose = MobPose {
            phase: 0.0,
            amount: 0.88,
            ..Default::default()
        };
        let mm = mob_model_matrices(MobModelKind::Creeper, &pose);
        // 四腿对角同相（CreeperModel.setupAnim:52-55：RH/LF = cos、LH/RF 反相）。
        let tip = |p: usize| mm[p].transform_point3(Vec3::new(0.0, -0.375, 0.0)).z;
        assert!(tip(2) < 0.0 && tip(4) > 0.0, "右后前摆 / 右前后摆");
        assert!(tip(3) > 0.0 && tip(5) < 0.0, "左后后摆 / 左前前摆");
        // 头顶高度：头 pivot(0,6,0) + 盒顶 8px → 世界 y = (24-6+8)/16 = 1.625。
        let base = m.slices[6].start as usize / 36 * 24;
        let mut max_y = f32::MIN;
        for v in &m.verts[base..base + 24] {
            let w = mm[0].transform_point3(Vec3::from(v.pos));
            max_y = max_y.max(w.y);
        }
        assert!((max_y - 1.625).abs() < 1e-5, "creeper 头顶 {max_y}");
    }

    #[test]
    fn spider_legs_splay_wide_and_swing() {
        let m = build_mob_mesh();
        let static_pose = MobPose::default();
        let mm0 = mob_model_matrices(MobModelKind::Spider, &static_pose);
        // 静态八腿外张（y0=±π/4 等）：模型横向跨度远超身体半宽 5px。
        let base = m.slices[7].start as usize / 36 * 24;
        let end = base + m.slices[7].len() / 36 * 24;
        let mut max_x = 0.0f32;
        for v in &m.verts[base..end] {
            let w = mm0[v.meta[1] as usize].transform_point3(Vec3::from(v.pos));
            max_x = max_x.max(w.x.abs());
        }
        assert!(max_x > 0.8 && max_x < 1.2, "蜘蛛腿展 {max_x}");
        // 行走摆动改变腿位（yRot 摆 + zRot 步进，右 + 左 −）。
        let walk = MobPose {
            phase: 1.1,
            amount: 0.88,
            ..Default::default()
        };
        let mm1 = mob_model_matrices(MobModelKind::Spider, &walk);
        let moved = (0..MAX_MOB_PARTS)
            .filter(|p| {
                mm0[*p]
                    .transform_point3(Vec3::new(0.0, -0.125, 0.0))
                    .distance(mm1[*p].transform_point3(Vec3::new(0.0, -0.125, 0.0)))
                    > 1e-4
            })
            .count();
        // 头/身静止，八腿全动。
        assert_eq!(moved, 8, "八条腿都应摆动，moved {moved}");
    }

    #[test]
    fn leg_swing_stays_in_facing_plane_at_yaw() {
        // yaw 共轭回归：部位旋转必须绕引擎局部轴构造再被 ry 左乘。旧实现
        // （轴先旋到世界）会得 T·ry²·R·ry⁻¹，yaw≠0 时腿摆侧翻到行走面之外。
        let pose = MobPose {
            pos: Vec3::new(5.0, 64.0, -7.0),
            yaw: std::f32::consts::FRAC_PI_2, // 面朝 +x（f(yaw)=(1,0,0)）
            phase: 0.0,
            amount: 0.88,
            ..Default::default()
        };
        let mm = mob_model_matrices(MobModelKind::Zombie, &pose);
        // 右腿前摆（shift 0 → 满幅 1.4 rad 前摆）：尖端应沿面朝方向 +x
        // 位移，横向（z）只允许静态 pivot 偏移（腿 pivot x=1.9px 经
        // RotY(−π/2) 落到 +z，摆动分量不得混入）。
        let tip = mm[4].transform_point3(Vec3::new(0.0, -0.75, 0.0));
        let swing = 0.75 * 1.4f32.sin();
        assert!(
            (tip.x - (pose.pos.x + swing)).abs() < 1e-4,
            "前摆应在 +x，tip {tip}"
        );
        assert!(
            (tip.z - pose.pos.z - 1.9 * PX).abs() < 1e-4,
            "腿摆不得侧翻出面向平面，tip {tip}"
        );
    }
}
