//! 生长波行为锁（任务板 #146，红测试先行）——纯决策域：同 seed 确定性、
//! 树苗光照门、作物满 age 收获规格、random tick 只作用于目标方块、
//! 长树立地 = 苗原位。Java/loot 依据均为 src-26.1（断言注释附 file:line）。
//!
//! 引擎接线（GameRuntime.growth_tick 结算/掉落通道）的回归锁在
//! `tests/growth_runtime.rs`（需完整 crate 依赖树）。

use mcv_core::{BlockId, BlockPos, CHUNK_VOL, ChunkPos, WORLD_MAX_Y, WORLD_MIN_Y, vidx};
use mcv_logic::growth::{self, Growth, LightCell, SAPLING_GROW_ONE_IN, SAPLING_LIGHT_MIN, ids};
use mcv_worldgen::trees::{BlockSetter, Rng, TreeKind, TreesRng, place_tree};

// ---------------------------------------------------------------------------
// 脚手架：单区块体素画布 + 简单天空光照模型 + 脚本随机源
// ---------------------------------------------------------------------------

/// 单区块（cx, cz）体素画布：y<62 实心石、y=62 泥土、y=63 草方块顶面
/// （可立地面），上方空气。坐标为世界绝对坐标。
struct Canvas {
    cx: i32,
    cz: i32,
    v: Vec<BlockId>,
}

impl Canvas {
    fn new(cx: i32, cz: i32) -> Self {
        let mut v = vec![BlockId(0); CHUNK_VOL];
        for z in 0..16usize {
            for x in 0..16usize {
                for y in WORLD_MIN_Y..62 {
                    v[vidx(x, y, z)] = BlockId(1);
                }
                v[vidx(x, 62, z)] = BlockId(ids::DIRT);
                v[vidx(x, 63, z)] = BlockId(ids::GRASS);
            }
        }
        Self { cx, cz, v }
    }

    fn in_chunk(&self, p: BlockPos) -> bool {
        p.x.div_euclid(16) == self.cx && p.z.div_euclid(16) == self.cz
    }

    fn get(&self, p: BlockPos) -> BlockId {
        if !self.in_chunk(p) || !(WORLD_MIN_Y..WORLD_MAX_Y).contains(&p.y) {
            return BlockId(0);
        }
        let [lx, _, lz] = p.local();
        self.v[vidx(lx, p.y, lz)]
    }

    fn set(&mut self, p: BlockPos, id: BlockId) {
        assert!(self.in_chunk(p) && (WORLD_MIN_Y..WORLD_MAX_Y).contains(&p.y));
        let [lx, _, lz] = p.local();
        self.v[vidx(lx, p.y, lz)] = id;
    }

    fn id_at(&self, x: i32, y: i32, z: i32) -> u16 {
        self.get(BlockPos::new(x, y, z)).id()
    }

    /// 天空光照模型：本列 y' ≥ p.y 全非不透明 → 天光 15，否则 0；方块光
    /// 恒 0（fixture 无光源）。对「树苗上方无空气」测试：上方被实体块挡
    /// → 光 0 < 门 9（SaplingBlock.java:46 光照门的 fixture 化）。
    fn light(&self, p: BlockPos) -> LightCell {
        if !(WORLD_MIN_Y..WORLD_MAX_Y).contains(&p.y) {
            return LightCell { sky: 15, block: 0 };
        }
        for y in p.y..WORLD_MAX_Y {
            if self.get(BlockPos::new(p.x, y, p.z)).def().opaque {
                return LightCell { sky: 0, block: 0 };
            }
        }
        LightCell { sky: 15, block: 0 }
    }

    fn read(&self, p: BlockPos) -> BlockId {
        self.get(p)
    }
}

/// `trees::BlockSetter` 画布适配（世界绝对坐标写入）。
struct CanvasSetter<'a>(&'a mut Canvas);

impl BlockSetter for CanvasSetter<'_> {
    fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        self.0.set(BlockPos::new(x, y, z), BlockId(id));
    }
}

/// 脚本随机源：预烤门抽取序（in-range），记录消耗次数（短路序断言用）。
struct ScriptRng {
    draws: std::collections::VecDeque<u32>,
    consumed: usize,
}

impl ScriptRng {
    fn lucky() -> Self {
        Self {
            draws: std::collections::VecDeque::new(),
            consumed: 0,
        }
    }
}

impl Rng for ScriptRng {
    fn next_int(&mut self, _bound: u32) -> u32 {
        self.consumed += 1;
        self.draws.pop_front().unwrap_or(0)
    }
    fn next_bool(&mut self) -> bool {
        self.next_int(2) == 1
    }
}

/// 收获流脚本源（`harvest_drops` 吃 `&mut dyn FnMut() -> u32`——与
/// `game.rs::spawn_rng` 同一消耗面）：预烤序列，耗尽后回 0。
fn script(draws: &[u32]) -> impl FnMut() -> u32 + '_ {
    let mut i = 0usize;
    move || {
        let v = draws.get(i).copied().unwrap_or(0);
        i += 1;
        v
    }
}

const OAK_SAPLING: BlockId = BlockId(ids::OAK_SAPLING);
const WHEAT: BlockId = BlockId(ids::WHEAT);

// ---------------------------------------------------------------------------
// 阶段位（状态 nibble）
// ---------------------------------------------------------------------------

#[test]
fn crop_age_and_sapling_stage_roundtrip_in_state_nibble() {
    // 小麦 age 0..7（CropBlock.java:29-30 AGE_7 → 状态 nibble 低 3 位）。
    for age in 0..=7u8 {
        let st = growth::with_crop_age(WHEAT, age);
        assert_eq!(st.id(), ids::WHEAT, "age {age} 不得改 id");
        assert_eq!(growth::crop_age(st), age, "age {age} roundtrip");
    }
    // 树苗 STAGE 0..1（BlockStateProperties.java:121 → nibble bit0）。
    for stage in 0..=1u8 {
        let st = growth::with_sapling_stage(OAK_SAPLING, stage);
        assert_eq!(st.id(), ids::OAK_SAPLING, "stage {stage} 不得改 id");
        assert_eq!(growth::sapling_stage(st), stage, "stage {stage} roundtrip");
    }
}

// ---------------------------------------------------------------------------
// 决策核：树苗门
// ---------------------------------------------------------------------------

/// 树苗上方无空气（被实体块挡 → fixture 光 0）→ 恒不长：原版门 =
/// `getMaxLocalRawBrightness(pos.above()) >= 9`（SaplingBlock.java:46），
/// 光 0 < 9 短路。幸运随机（门抽取恒 0）也必须被亮度门拦下。
#[test]
fn sapling_without_air_above_never_grows() {
    let mut c = Canvas::new(0, 0);
    let p = BlockPos::new(5, 64, 5);
    c.set(p, growth::with_sapling_stage(OAK_SAPLING, 1));
    c.set(BlockPos::new(5, 65, 5), BlockId(1)); // 上方石头
    for _ in 0..64 {
        let mut rng = ScriptRng::lucky();
        let g = growth::random_tick(
            c.get(p),
            p,
            c.light(p),
            c.light(BlockPos::new(p.x, p.y + 1, p.z)),
            0,
            &mut rng,
            &|p: BlockPos| c.read(p),
        );
        assert_eq!(g, Growth::None, "上方无空气必须不长");
    }
}

/// 树苗门消耗序：亮度不足时不消耗随机流（SaplingBlock.java:46 `&&` 左
/// 操作数先行短路）；暗种 STAGE 0 → 先进位（:53 cycle(STAGE)）；
/// STAGE 1 且立地合格 → GrowTree（:55 growTree）。
#[test]
fn sapling_gate_short_circuit_and_two_stage_advance() {
    let c = Canvas::new(0, 0);
    let p = BlockPos::new(5, 64, 5);
    // ① 亮度门短路：光 0 → 不抽取。
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        growth::with_sapling_stage(OAK_SAPLING, 0),
        p,
        LightCell { sky: 0, block: 0 },
        LightCell { sky: 0, block: 0 },
        0,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    assert_eq!(g, Growth::None);
    assert_eq!(rng.consumed, 0, "亮度门未过不得消耗随机流");
    // ② 光足（15 ≥ 9）+ STAGE 0 → SaplingAdvance。
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        growth::with_sapling_stage(OAK_SAPLING, 0),
        p,
        c.light(p),
        LightCell { sky: 15, block: 0 },
        0,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    assert_eq!(g, Growth::SaplingAdvance);
    assert_eq!(rng.consumed, 1, "恰好消耗一次 nextInt(7)");
    // ③ STAGE 1 → GrowTree（oak 苗 → oak 树）。
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        growth::with_sapling_stage(OAK_SAPLING, 1),
        p,
        c.light(p),
        LightCell { sky: 15, block: 0 },
        0,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    assert_eq!(g, Growth::GrowTree(TreeKind::Oak));
    // ④ skyDarken 只压天光：暗夜 darken=11 → 天光 15−11=4 < 9 不长；
    //    火把 block=12 → max(12, 15−11)=12 ≥ 9 照长（LevelLightEngine
    //    .java:146-151 darken 不压方块光）。
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        growth::with_sapling_stage(OAK_SAPLING, 1),
        p,
        c.light(p),
        LightCell { sky: 15, block: 0 },
        11,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    assert_eq!(g, Growth::None, "暗夜无光不长");
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        growth::with_sapling_stage(OAK_SAPLING, 1),
        p,
        c.light(p),
        LightCell { sky: 15, block: 12 },
        11,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    assert_eq!(g, Growth::GrowTree(TreeKind::Oak), "方块光不被 darken 压");
}

/// 长树立地门 = 下方 ∈ TREE_GROUND（tree_pass.rs:87 白名单语义复用）；
/// 基座被挖（苗下石头）→ STAGE 1 也不长。
#[test]
fn sapling_grow_requires_tree_ground_below() {
    let mut c = Canvas::new(0, 0);
    let p = BlockPos::new(5, 64, 5);
    c.set(BlockPos::new(5, 63, 5), BlockId(1)); // 基座挖成石头
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        growth::with_sapling_stage(OAK_SAPLING, 1),
        p,
        c.light(p),
        LightCell { sky: 15, block: 0 },
        0,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    assert_eq!(g, Growth::None, "非土族基座不长树");
}

// ---------------------------------------------------------------------------
// 决策核：作物门
// ---------------------------------------------------------------------------

/// 作物亮度门 `getRawBrightness(pos, 0) >= 9`（CropBlock.java:80）：光 8
/// 短路不消耗随机流；满 age 不再生长（:74-76 isRandomlyTicking = !maxAge）。
#[test]
fn crop_gate_light_and_max_age() {
    let c = Canvas::new(0, 0);
    let p = BlockPos::new(8, 64, 8);
    // 光 8 < 9 → 不长不消耗。
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        growth::with_crop_age(WHEAT, 0),
        p,
        LightCell { sky: 8, block: 0 },
        c.light(p),
        0,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    assert_eq!(g, Growth::None);
    assert_eq!(rng.consumed, 0);
    // 满 age 7 → 不长不消耗（isRandomlyTicking 门）。
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        growth::with_crop_age(WHEAT, 7),
        p,
        LightCell { sky: 15, block: 0 },
        c.light(p),
        0,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    assert_eq!(g, Growth::None);
    assert_eq!(rng.consumed, 0, "满 age 不进 randomTick");
}

/// 增长节奏常数：光足 + 空旷 → 速度 1.0 → `nextInt(26)==0`
/// （CropBlock.java:84 `(int)(25.0F/1.0)+1`）；中心干耕地 → 2.0 →
/// `nextInt(13)==0`（底 3×3：farmland 中心 1.0，无湿耕/邻耕）。
#[test]
fn crop_growth_rhythm_constants() {
    let c = Canvas::new(0, 0);
    let p = BlockPos::new(8, 64, 8); // 草方块上（非耕地，速度基值 1.0）
    // 幸运抽恒 0 → 每次命中 → age 逐级 +1（CropBlock.java:85）。
    let mut age = 0u8;
    for _ in 0..7 {
        let mut rng = ScriptRng::lucky();
        let g = growth::random_tick(
            growth::with_crop_age(WHEAT, age),
            p,
            LightCell { sky: 15, block: 0 },
            c.light(p),
            0,
            &mut rng,
            &|p: BlockPos| c.read(p),
        );
        match g {
            Growth::CropGrow { new_age } => age = new_age,
            other => panic!("光足空旷必须增长，得 {other:?}"),
        }
    }
    assert_eq!(age, 7, "七次幸运抽必满 age");
    // 节奏界：速度 1.0 → 界 26；中心干耕地（below y=63 改 farmland）→
    // 速度 2.0 → 界 13（CropBlock.java:84 (int)(25.0F/speed)+1）。
    let bare = growth::crop_growth_speed(ids::WHEAT, 8, 64, 8, &|x, y, z| {
        c.read(BlockPos::new(x, y, z))
    });
    assert_eq!(bare, 1.0, "草顶无耕无邻 = 基速 1.0");
    assert_eq!((growth::CROP_GROW_BASE / bare) as i32 + 1, 26);
    let mut c2 = Canvas::new(0, 0);
    c2.set(BlockPos::new(8, 63, 8), BlockId(ids::FARMLAND));
    let speed = growth::crop_growth_speed(ids::WHEAT, 8, 64, 8, &|x, y, z| {
        c2.read(BlockPos::new(x, y, z))
    });
    assert_eq!(speed, 2.0, "中心干耕地 = 1.0(基)+1.0(farmland)");
    assert_eq!((growth::CROP_GROW_BASE / speed) as i32 + 1, 13);
    // 常数锁（CropBlock.java:29 MAX_AGE / SaplingBlock.java:46 门）。
    assert_eq!(growth::CROP_MAX_AGE, 7);
    assert_eq!(SAPLING_LIGHT_MIN, 9);
    assert_eq!(growth::CROP_LIGHT_MIN, 9);
    assert_eq!(SAPLING_GROW_ONE_IN, 7);
}

// ---------------------------------------------------------------------------
// random tick 只作用于目标方块
// ---------------------------------------------------------------------------

/// 草方块/石头/泥土/耕地/空气被抽中 → 无操作（幸运抽也不动）：原版
/// isRandomlyTicking 面 = 树苗（randomTicks()）+ 小麦（!maxAge），
/// 其余方块 randomTick 无实现路径（草方块蔓延未建模，GROWTH-1 ③）。
#[test]
fn random_tick_only_acts_on_registered_targets() {
    let c = Canvas::new(0, 0);
    let p = BlockPos::new(8, 64, 8);
    for raw in [ids::GRASS, 1, ids::DIRT, ids::FARMLAND, 0] {
        let mut rng = ScriptRng::lucky();
        let g = growth::random_tick(
            BlockId(raw),
            p,
            LightCell { sky: 15, block: 0 },
            LightCell { sky: 15, block: 0 },
            0,
            &mut rng,
            &|p: BlockPos| c.read(p),
        );
        assert_eq!(g, Growth::None, "id {raw} 非 生长目标");
    }
}

// ---------------------------------------------------------------------------
// 长树：立地 = 苗原位
// ---------------------------------------------------------------------------

/// STAGE=1 的 oak 苗 + 合法立地 → 长树后：原木基 = 苗位、基下 = 泥土
/// （trunk_straight 基下 dirt，trees.rs:316-321；TreeGrower.java:169-171
/// 先清苗位再投影）且有叶。
#[test]
fn tree_grows_at_sapling_spot() {
    let mut c = Canvas::new(0, 0);
    let p = BlockPos::new(5, 64, 5);
    c.set(p, growth::with_sapling_stage(OAK_SAPLING, 1));
    let mut rng = ScriptRng::lucky();
    let g = growth::random_tick(
        c.get(p),
        p,
        c.light(p),
        c.light(BlockPos::new(p.x, p.y + 1, p.z)),
        0,
        &mut rng,
        &|p: BlockPos| c.read(p),
    );
    let kind = match g {
        Growth::GrowTree(k) => k,
        other => panic!("STAGE 1 幸运抽应长树，得 {other:?}"),
    };
    // 接线序（game.rs::grow_tree_at）：清苗 → place_tree。
    c.set(p, BlockId(0));
    let mut trng = TreesRng::new(0xA11CE);
    place_tree(&mut trng, p.x, p.y, p.z, kind, &mut CanvasSetter(&mut c));
    assert_eq!(c.id_at(5, 64, 5), 6, "树干基必须落在苗原位（log）");
    assert_eq!(c.id_at(5, 63, 5), ids::DIRT, "基下为泥土");
    let leaves = c.v.iter().filter(|b| b.id() == 7).count();
    assert!(leaves > 0, "冠层应有叶");
    // 苗位上方连续原木（直杆 oak 4..6）。
    let trunk: u16 = c.id_at(5, 65, 5);
    assert_eq!(trunk, 6);
}

// ---------------------------------------------------------------------------
// 同 seed 确定性
// ---------------------------------------------------------------------------

/// scan_tick 同 (seed, tick) 恒同决策序：满画布 STAGE=0 树苗（进位决策
/// 不涉立地门，光照门吃画布天空模型）→ 大样本命中（72 尝试/块/tick 上段
/// section 高密度目标），两次扫描决策列表逐位相等；跨 tick/跨 seed 决策
/// 列表不同（随机域隔离的烟雾断言）。
#[test]
fn same_seed_scan_is_deterministic() {
    let mut c = Canvas::new(0, 0);
    // y ∈ [64, 300) 全部填 oak 苗（STAGE 0）→ 上段 15 个 section 高密度。
    for z in 0..16usize {
        for x in 0..16usize {
            for y in 64..300 {
                c.v[vidx(x, y, z)] = growth::with_sapling_stage(OAK_SAPLING, 0);
            }
        }
    }
    let chunks = [ChunkPos::new(0, 0)];
    let run = |seed: u64, tick: u64| {
        growth::scan_tick(&chunks, seed, tick, 0, &|p| c.read(p), &|p| c.light(p))
    };
    let a = run(0x5EED, 42);
    let b = run(0x5EED, 42);
    assert_eq!(a, b, "同 seed 同 tick 必须同决策序");
    assert!(
        !a.is_empty(),
        "高密度画布必须命中决策（实际 {} 项）",
        a.len()
    );
    assert!(
        a.iter().all(|(_, g)| matches!(g, Growth::SaplingAdvance)),
        "STAGE=0 画布的命中应全部为进位决策"
    );
    let diff = run(0x5EED, 43);
    assert_ne!(a, diff, "跨 tick 随机域必须扰动");
    let other = run(0xC0FFEE, 42);
    assert_ne!(a, other, "跨 seed 随机域必须隔离");
}

// ---------------------------------------------------------------------------
// 收获规格（26.1 loot_table/blocks/wheat.json）
// ---------------------------------------------------------------------------

/// 满 age 收获 = 1×wheat + wheat_seeds 0..3（binomial n=3、p=4/7）：
/// 幸运流（%7 恒 0 < 4）→ 3 粒种子；厄运流（恒 ≥4）→ 0 粒（只有麦粒）。
/// 未满 age → 仅 1×wheat_seeds（alternatives 第二分支）。非作物 → 空。
#[test]
fn harvest_drops_match_vanilla_wheat_loot() {
    let mature = growth::with_crop_age(WHEAT, 7);
    let mut lucky = || 0u32;
    let drops = growth::harvest_drops(mature, &mut lucky);
    assert_eq!(drops, vec![("wheat", 1), ("wheat_seeds", 3)], "幸运流满种");
    let mut unlucky = script(&[4, 5, 6]);
    let drops = growth::harvest_drops(mature, &mut unlucky);
    assert_eq!(drops, vec![("wheat", 1)], "厄运流零种子");
    // 种子数界：0..=3（binomial n=3）。
    let mut mixed = script(&[0, 6, 1]);
    let drops = growth::harvest_drops(mature, &mut mixed);
    assert_eq!(drops, vec![("wheat", 1), ("wheat_seeds", 2)]);
    // 未满 age → 1 种子（CropBlock.java:164-166 getBaseSeedId）。
    let young = growth::with_crop_age(WHEAT, 3);
    let mut any = || 6u32;
    assert_eq!(
        growth::harvest_drops(young, &mut any),
        vec![("wheat_seeds", 1)]
    );
    // 非作物 → 空。
    let mut any2 = || 0u32;
    assert!(growth::harvest_drops(OAK_SAPLING, &mut any2).is_empty());
    // 二项分布均值 ≈ n·p = 3×4/7 ≈ 1.714（大样本统计，容差 0.1）。
    let mut acc = 0f64;
    for i in 0..4000u32 {
        let draws = [i % 7, (i * 3 + 1) % 7, (i * 5 + 2) % 7];
        let mut rng = script(&draws);
        for (name, n) in growth::harvest_drops(mature, &mut rng) {
            if name == "wheat_seeds" {
                acc += f64::from(n);
            }
        }
    }
    let mean = acc / 4000.0;
    assert!(
        (mean - 12.0 / 7.0).abs() < 0.1,
        "种子均值 {mean} 应 ≈ 3×4/7 = {:.3}",
        12.0 / 7.0
    );
}

/// 树写入门与槽位判别（tree_pass.rs:169-189 ChunkClipper 同表）：
/// 原木进空气/叶、不进石头；叶进空气/水/叶；基下泥土条件替换（草 → 替、
/// 泥土 → 跳过）。
#[test]
fn tree_write_gates_match_tree_pass_clipper() {
    let air = BlockId(0);
    let leaf = BlockId(7);
    use growth::TreeSlot::{Leaf, Log, Soil};
    // Log 门：空气/叶进；石头/泥土/水不进（TrunkPlacer.java:93-99 →
    // TreeFeature.java:53-55；DarkOakTrunkPlacer.java:63 交集）。
    assert!(growth::tree_write_gate(Log, air));
    assert!(growth::tree_write_gate(Log, leaf));
    assert!(!growth::tree_write_gate(Log, BlockId(1)));
    assert!(!growth::tree_write_gate(Log, BlockId(ids::DIRT)));
    assert!(!growth::tree_write_gate(Log, BlockId(ids::WATER)));
    // Leaf 门：空气/水/叶进（FoliagePlacer.java:171-172）。
    assert!(growth::tree_write_gate(Leaf, air));
    assert!(growth::tree_write_gate(Leaf, BlockId(ids::WATER)));
    assert!(growth::tree_write_gate(Leaf, leaf));
    assert!(!growth::tree_write_gate(Leaf, BlockId(1)));
    // Soil 门：非泥土替换（TreeConfiguration.java:20-26；草/雪草 → 替、
    // 泥土 → 跳过）。
    assert!(growth::tree_write_gate(Soil, BlockId(ids::GRASS)));
    assert!(growth::tree_write_gate(Soil, BlockId(ids::SNOW_GRASS)));
    assert!(!growth::tree_write_gate(Soil, BlockId(ids::DIRT)));
    // 槽位判别覆盖六树种 log/leaf + dirt。
    for id in [6u16, 20, 83, 304, 496, 948] {
        assert_eq!(growth::tree_slot_of(id), Some(Log), "log id {id}");
    }
    for id in [7u16, 19, 82, 303, 495, 947] {
        assert_eq!(growth::tree_slot_of(id), Some(Leaf), "leaf id {id}");
    }
    assert_eq!(growth::tree_slot_of(ids::DIRT), Some(Soil));
    assert_eq!(growth::tree_slot_of(ids::GRASS), None);
}

/// 苗 id → 树种映射全覆盖（#135 六树种面）。
#[test]
fn sapling_to_tree_kind_table() {
    assert_eq!(growth::tree_kind_of(ids::OAK_SAPLING), Some(TreeKind::Oak));
    assert_eq!(
        growth::tree_kind_of(ids::BIRCH_SAPLING),
        Some(TreeKind::Birch)
    );
    assert_eq!(
        growth::tree_kind_of(ids::SPRUCE_SAPLING),
        Some(TreeKind::Spruce)
    );
    assert_eq!(
        growth::tree_kind_of(ids::JUNGLE_SAPLING),
        Some(TreeKind::Jungle)
    );
    assert_eq!(
        growth::tree_kind_of(ids::ACACIA_SAPLING),
        Some(TreeKind::Acacia)
    );
    assert_eq!(
        growth::tree_kind_of(ids::DARK_OAK_SAPLING),
        Some(TreeKind::DarkOak)
    );
    assert_eq!(growth::tree_kind_of(0), None);
    for s in growth::SAPLINGS {
        assert!(growth::is_sapling(s));
    }
    assert!(growth::is_crop(ids::WHEAT));
    assert!(!growth::is_crop(ids::FARMLAND));
}
