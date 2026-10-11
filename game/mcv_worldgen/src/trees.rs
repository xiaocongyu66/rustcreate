//! 26.1 树木结构移植——纯函数库（任务板 #135，T 波）。
//!
//! **不接 `generate()`/`game.rs`**（接线是 T5 合入后的后续刀）；本模块只提供
//! [`place_tree`]：给定随机源与落点，把一棵 26.1 规格的树写进任意
//! [`BlockSetter`]。原版 26.1 已把树木配置化：`TreeFeature`（骨架）+
//! trunk/foliage placer 族（算法）+ `TreeFeatures.java`（参数表），本文件逐
//! placer 移植，常数全部引注（路径缩写：`TJ` =
//! `/root/mc-ref/src-26.1/net/minecraft/data/worldgen/features/TreeFeatures.java`，
//! 其余按 `feature/…` 相对
//! `/root/mc-ref/src-26.1/net/minecraft/world/level/levelgen/`）。
//!
//! 骨架对拍：`TreeFeature.java:66-69 doPlace` 的采样顺序锁死
//! treeHeight → foliageHeight → leafRadius → placeTrunk → 逐 attachment
//! createFoliage。本仓每个树种函数按同一顺序消耗 [`Rng`]——这是「同 seed
//! 逐字节复现」契约面，改消耗序 = 破坏确定性。
//!
//! 有意裁剪（`docs/KNOWN-DIVERGENCE.md` TREES-1 登记）：
//! ① `getMaxFreeTreeHeight`（TreeFeature.java:119-134）+ `validTreePos`
//!    （:54-56）要读世界查空——纯函数无读面，假定立地开阔（树足迹内的
//!    「叶不盖本树原木」用本次调用自己的原木位置集模拟）；
//! ② 基下 dirt 恒放（原版是条件替换，TreeConfiguration.java:23-25）；
//! ③ decorators（树藤/可可/蜂巢）不在 #135 面（jungle 对齐
//!    JUNGLE_TREE_NO_VINE）；
//! ④ 叶 distance/persistent 与原木 axis 状态本仓未建模（一元 id）——这些
//!    placer 的原木恒 `defaultBlockState()`=axis_y（ForkingTrunkPlacer 的
//!    斜枝也是逐 y 竖杆），一元 id 语义无损。

use std::collections::HashSet;

/// 方块 id 常数——`game/mcv_core/src/blocks_gen.inc.rs` 引注（生成表；行号
/// 对应该文件当前版本，`tests/trees.rs::block_ids_lock_names` 锁名防漂移）。
pub mod ids {
    /// "log"（oak，旧 14 方块一元 id）
    pub const OAK_LOG: u16 = 6; // blocks_gen.inc.rs:27
    /// "leaves"（oak）
    pub const OAK_LEAVES: u16 = 7; // :28
    pub const DIRT: u16 = 2; // :23
    pub const ACACIA_LEAVES: u16 = 19; // :40
    pub const ACACIA_LOG: u16 = 20; // :41
    pub const BIRCH_LEAVES: u16 = 82; // :103
    pub const BIRCH_LOG: u16 = 83; // :104
    pub const DARK_OAK_LEAVES: u16 = 303; // :324
    pub const DARK_OAK_LOG: u16 = 304; // :325
    pub const JUNGLE_LEAVES: u16 = 495; // :516
    pub const JUNGLE_LOG: u16 = 496; // :517
    pub const SPRUCE_LEAVES: u16 = 947; // :968
    pub const SPRUCE_LOG: u16 = 948; // :969
}

/// `RandomSource.nextInt(bound)` 语义的整数随机面：uniform [0, bound)、
/// `bound ≥ 1` 且每次调用**必消耗**一次随机流（含 bound=1，
/// XoroshiroRandomSource.java:54-58 正界拒绝采样）。树种函数的消耗序 =
/// 原版 doPlace 顺序（模块 doc）。
pub trait Rng {
    fn next_int(&mut self, bound: u32) -> u32;
    /// `RandomSource.nextBoolean()` 独立抽取面（暗橡树 50% 顶盖层）。
    fn next_bool(&mut self) -> bool;
}

/// 本仓确定性随机流（xorshift64* + 乘高位缩减）：测试与未来 T5 接线的缺省
/// 实现。**不是**原版 Xoroshiro128++——原版树随机源按区块/位置从世界种子
/// 派生，跨实现字节级对拍不可能；本仓契约只有「同 seed 同字节」。
pub struct TreesRng {
    s: u64,
}

impl TreesRng {
    pub fn new(seed: u64) -> Self {
        // xorshift64* 状态禁 0；种子 |1 保低位熵（Saito & Matsumoto 2008）。
        Self { s: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.s;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.s = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

impl Rng for TreesRng {
    fn next_int(&mut self, bound: u32) -> u32 {
        assert!(bound >= 1, "nextInt bound must be positive");
        // Lemire 乘高位缩减 + 拒绝采样（与 XoroshiroRandomSource.java:54-77
        // 的 unbiased-bucket 思路同构；31 位抽取面）。
        let threshold = ((1u64 << 32) % u64::from(bound)) as u32;
        loop {
            let bits = (self.next_u64() >> 33) as u64;
            let m = bits * u64::from(bound);
            if (m as u32) >= threshold {
                return (m >> 32) as u32;
            }
        }
    }

    fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

/// 方块落点面（世界绝对坐标）：T5 接线把它适配到区块体素写，测试适配到
/// 画布。纯函数库不关心落点介质。
pub trait BlockSetter {
    fn set(&mut self, x: i32, y: i32, z: i32, id: u16);
}

/// 树种（#135 六种；fancy oak/mega 系/樱花/红树/苍白橡树不在本波面）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeKind {
    Oak,
    Birch,
    Spruce,
    Jungle,
    Acacia,
    DarkOak,
}

/// 全树种（测试表驱动与接线遍历用）。
pub const ALL_KINDS: [TreeKind; 6] = [
    TreeKind::Oak,
    TreeKind::Birch,
    TreeKind::Spruce,
    TreeKind::Jungle,
    TreeKind::Acacia,
    TreeKind::DarkOak,
];

impl TreeKind {
    /// (log_id, leaf_id)——引注见 [`ids`]；oak 走旧一元 log/leaves（26.1 的
    /// OAK_LOG 等价物，blocks_gen.inc.rs:27-28）。
    pub fn log_leaf_ids(self) -> (u16, u16) {
        match self {
            TreeKind::Oak => (ids::OAK_LOG, ids::OAK_LEAVES),
            TreeKind::Birch => (ids::BIRCH_LOG, ids::BIRCH_LEAVES),
            TreeKind::Spruce => (ids::SPRUCE_LOG, ids::SPRUCE_LEAVES),
            TreeKind::Jungle => (ids::JUNGLE_LOG, ids::JUNGLE_LEAVES),
            TreeKind::Acacia => (ids::ACACIA_LOG, ids::ACACIA_LEAVES),
            TreeKind::DarkOak => (ids::DARK_OAK_LOG, ids::DARK_OAK_LEAVES),
        }
    }

    /// 树干高度闭区间——按 `TrunkPlacer.getTreeHeight`（TrunkPlacer.java:
    /// 58-60）公式从 TJ 参数表推（各 SPEC 引注见树种函数）。
    pub fn trunk_height_range(self) -> (i32, i32) {
        match self {
            TreeKind::Oak => (4, 6),     // TJ:136 (4,2,0): 4+nextInt(3)+nextInt(1)
            TreeKind::Birch => (5, 7),   // TJ:181 (5,2,0)
            TreeKind::Spruce => (5, 9),  // TJ:433-441 (5,2,1)
            TreeKind::Jungle => (4, 12), // TJ:189 (4,8,0)
            TreeKind::Acacia => (5, 9),  // TJ:421 (5,2,2)
            TreeKind::DarkOak => (6, 9), // TJ:139-147 (6,2,1)
        }
    }
}

/// 世界坐标（原版 BlockPos 等价面，只带本库需要的算术）。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Pos {
    x: i32,
    y: i32,
    z: i32,
}

impl Pos {
    fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            z: self.z + dz,
        }
    }
}

/// 本树写手：包一层 [`BlockSetter`]，携带本树原木位置集——在「树足迹」内
/// 模拟原版 `TreeFeature.validTreePos`（TreeFeature.java:54-56）的替换门：
/// 叶不得覆盖本树原木；世界级碰撞仍是接线方职责（偏差 ①）。
struct Scribe<'a, S: BlockSetter> {
    setter: &'a mut S,
    log_id: u16,
    logs: HashSet<(i32, i32, i32)>,
}

impl<S: BlockSetter> Scribe<'_, S> {
    fn place_log(&mut self, p: Pos) {
        self.logs.insert((p.x, p.y, p.z));
        self.setter.set(p.x, p.y, p.z, self.log_id);
    }

    fn place_dirt(&mut self, p: Pos) {
        self.setter.set(p.x, p.y, p.z, ids::DIRT);
    }
}

/// 水平四向表——`Direction.Plane.HORIZONTAL.getRandomDirection`
/// （Direction.java:588-590）= faces[nextInt(4)]。本仓排列序自定（均匀面
/// 等价，只锁自身确定性）。
const LEAN_DIRS: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// `TrunkPlacer.getTreeHeight`（TrunkPlacer.java:58-60）：恒消耗两次抽取
/// （A/B 为 0 时 nextInt(1) 同样消耗，与原版一致）。
fn sample_trunk_height<R: Rng>(rng: &mut R, spec: (i32, i32, i32)) -> i32 {
    let (base, a, b) = spec;
    base + rng.next_int((a + 1) as u32) as i32 + rng.next_int((b + 1) as u32) as i32
}

/// `StraightTrunkPlacer.placeTrunk`（StraightTrunkPlacer.java:34-46）：基下
/// dirt 打底 + 竖直原木列 [0, h)；冠附着点 = origin.above(h)（冠层刀用）。
fn trunk_straight<S: BlockSetter>(scribe: &mut Scribe<S>, origin: Pos, h: i32) {
    scribe.place_dirt(origin.offset(0, -1, 0)); // :36 placeBelowTrunkBlock
    for dy in 0..h {
        scribe.place_log(origin.offset(0, dy, 0)); // :38-40
    }
}

/// `DarkOakTrunkPlacer.placeTrunk`（DarkOakTrunkPlacer.java:33-96）：2x2
/// 打底与立柱、随机倾斜（leanHeight :45 / leanSteps :46）、外圈 1/3 概率
/// 下垂枝干（长 2..4，:75-80）。
fn trunk_dark_oak<R: Rng, S: BlockSetter>(
    rng: &mut R,
    scribe: &mut Scribe<S>,
    origin: Pos,
    h: i32,
) {
    let below = origin.offset(0, -1, 0);
    scribe.place_dirt(below); // :35-38 below / east / south / south.east()
    scribe.place_dirt(below.offset(1, 0, 0));
    scribe.place_dirt(below.offset(0, 0, 1));
    scribe.place_dirt(below.offset(1, 0, 1));
    let (ldx, ldz) = LEAN_DIRS[rng.next_int(4) as usize]; // :38 随机倾斜方向
    let lean_height = h - rng.next_int(4) as i32; // :45
    let mut lean_steps = 2i32 - rng.next_int(3) as i32; // :46 → 0..2
    let (mut tx, mut tz) = (origin.x, origin.z);
    let ey = origin.y + h - 1;
    for dy in 0..h {
        if dy >= lean_height && lean_steps > 0 {
            tx += ldx;
            tz += ldz;
            lean_steps -= 1;
        }
        // 原版此处过 isAirOrLeaves 门（:59）——无读面恒放（偏差 ①）。
        let p = Pos::new(tx, origin.y + dy, tz);
        scribe.place_log(p);
        scribe.place_log(p.offset(1, 0, 0)); // east
        scribe.place_log(p.offset(0, 0, 1)); // south
        scribe.place_log(p.offset(1, 0, 1)); // east.south()
    }
    // :71 主冠附着点 (tx, ey, tz)（doubleTrunk=true，冠层刀用）。
    // :73-85 外圈枝干：4x4 外环 12 格各 1/3 概率，柱长 nextInt(3)+2。
    for ox in -1..=2 {
        for oz in -1..=2 {
            if (ox < 0 || ox > 1 || oz < 0 || oz > 1) && rng.next_int(3) == 0 {
                let length = rng.next_int(3) + 2; // :76 → 2..4
                for by in 0..length {
                    scribe.place_log(Pos::new(origin.x + ox, ey - by - 1, origin.z + oz));
                }
                // :82 枝干冠附着点 (x+ox, ey, z+oz)（doubleTrunk=false）。
            }
        }
    }
}

/// `ForkingTrunkPlacer.placeTrunk`（ForkingTrunkPlacer.java:34-88）：主杆
/// 随机倾斜 + 反向斜枝（每步 y+1、xz±1 的竖杆序列，非对角连杆）。
fn trunk_forking<R: Rng, S: BlockSetter>(rng: &mut R, scribe: &mut Scribe<S>, origin: Pos, h: i32) {
    scribe.place_dirt(origin.offset(0, -1, 0)); // :36
    let (ldx, ldz) = LEAN_DIRS[rng.next_int(4) as usize]; // :38
    let lean_height = h - rng.next_int(4) as i32 - 1; // :41
    let mut lean_steps = 3i32 - rng.next_int(3) as i32; // :42 → 1..3
    let (mut tx, mut tz) = (origin.x, origin.z);
    for yo in 0..h {
        if yo >= lean_height && lean_steps > 0 {
            tx += ldx; // :50-54
            tz += ldz;
            lean_steps -= 1;
        }
        // :52-54 原版 placeLog 成功才记冠点 ey——本仓无读面恒成功。
        scribe.place_log(Pos::new(tx, origin.y + yo, tz));
    }
    let (bdx, bdz) = LEAN_DIRS[rng.next_int(4) as usize]; // :66 枝干方向
    if (bdx, bdz) != (ldx, ldz) {
        let branch_pos = lean_height - rng.next_int(2) as i32 - 1; // :69
        let mut branch_steps = 1i32 + rng.next_int(3) as i32; // :70 → 1..3
        let (mut bx, mut bz) = (origin.x, origin.z);
        let mut yo = branch_pos;
        while yo < h && branch_steps > 0 {
            if yo >= 1 {
                bx += bdx;
                bz += bdz;
                scribe.place_log(Pos::new(bx, origin.y + yo, bz));
            }
            branch_steps -= 1;
            yo += 1;
        }
    }
}

/// oak/birch/jungle：`createStraightBlobTree`（TJ:123-131）——直杆 +
/// `BlobFoliagePlacer(CI(radius), CI(0), height=3)` 膨冠。
/// 参数：oak (4,2,0)+r2（TJ:136）、birch (5,2,0)+r2（TJ:181）、
/// jungle (4,8,0)+r2（TJ:189）。
fn blob_tree<R: Rng, S: BlockSetter>(
    rng: &mut R,
    scribe: &mut Scribe<S>,
    origin: Pos,
    spec: (i32, i32, i32),
) {
    let h = sample_trunk_height(rng, spec);
    trunk_straight(scribe, origin, h);
    // 绿刀：BlobFoliagePlacer.createFoliage（BlobFoliagePlacer.java:44-53）——
    // yo = 0..-3 逐层 currentRadius = max(leafRadius - 1 - yo/2, 0)。
}

/// spruce：TJ:433-441——`StraightTrunkPlacer(5,2,1)` +
/// `SpruceFoliagePlacer(U(2,3), U(0,2), U(1,2))` 双层裙摆与尖顶。
fn spruce_tree<R: Rng, S: BlockSetter>(rng: &mut R, scribe: &mut Scribe<S>, origin: Pos) {
    let h = sample_trunk_height(rng, (5, 2, 1));
    trunk_straight(scribe, origin, h);
    // 绿刀：SpruceFoliagePlacer.createFoliage（SpruceFoliagePlacer.java:38-56）
    // ——轮生半径 0/1 起步、reset 后 minRadius=1、maxRadius 逐层爬到 leafRadius。
}

/// acacia：TJ:417-425——`ForkingTrunkPlacer(5,2,2)` +
/// `AcaciaFoliagePlacer(CI(2), CI(0))` 平顶+十字臂。分支结构已全文移植，
/// 无 acacia_branching_debt（绿刀冠层落位后收口）。
fn acacia_tree<R: Rng, S: BlockSetter>(rng: &mut R, scribe: &mut Scribe<S>, origin: Pos) {
    let h = sample_trunk_height(rng, (5, 2, 2));
    trunk_forking(rng, scribe, origin, h);
    // 绿刀：AcaciaFoliagePlacer.createFoliage（AcaciaFoliagePlacer.java:33-54）
    // ——主冠 radiusOffset=1、枝冠 0；行 (-1, r+ro) / (0, r-1) / (0, r+ro-1)。
}

/// dark oak：TJ:139-147——`DarkOakTrunkPlacer(6,2,1)` +
/// `DarkOakFoliagePlacer(CI(0), CI(0))` 圆柱冠（2x2 三层 + 50% 顶盖）。
fn dark_oak_tree<R: Rng, S: BlockSetter>(rng: &mut R, scribe: &mut Scribe<S>, origin: Pos) {
    let h = sample_trunk_height(rng, (6, 2, 1));
    trunk_dark_oak(rng, scribe, origin, h);
    // 绿刀：DarkOakFoliagePlacer.createFoliage（DarkOakFoliagePlacer.java:33-52）
    // ——主冠 (-1,r+2)/(0,r+3)/(+1,r+2)/50%(+2,r)；枝冠 (-1,r+2)/(0,r+1)。
}

/// 种一棵树。`base_y` = 树基格（原版 Feature.origin / 苗位）的**世界绝对 y**；
/// 树干占 [base_y, base_y+树干高度)，基下一格放 dirt（dark oak 为 2x2 立柱
/// +2x2 dirt）。随机消耗序与原版 doPlace 一致（模块 doc）。
pub fn place_tree<R: Rng>(
    rng: &mut R,
    x: i32,
    base_y: i32,
    z: i32,
    kind: TreeKind,
    setter: &mut impl BlockSetter,
) {
    let origin = Pos::new(x, base_y, z);
    let mut scribe = Scribe {
        setter,
        log_id: kind.log_leaf_ids().0,
        logs: HashSet::new(),
    };
    match kind {
        TreeKind::Oak => blob_tree(rng, &mut scribe, origin, (4, 2, 0)),
        TreeKind::Birch => blob_tree(rng, &mut scribe, origin, (5, 2, 0)),
        TreeKind::Spruce => spruce_tree(rng, &mut scribe, origin),
        TreeKind::Jungle => blob_tree(rng, &mut scribe, origin, (4, 8, 0)),
        TreeKind::Acacia => acacia_tree(rng, &mut scribe, origin),
        TreeKind::DarkOak => dark_oak_tree(rng, &mut scribe, origin),
    }
}
