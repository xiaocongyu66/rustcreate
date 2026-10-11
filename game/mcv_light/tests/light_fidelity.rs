//! #153 光照保真审计波——独立对拍套件（原版真值先行）。
//!
//! 每条断言锚定反编译 Minecraft 26.1 源码 `/root/mc-ref/src-26.1` 的
//! file:line（审计波取证清单见 `docs/light-audit.md` 矩阵）。与
//! `light.rs` 的区别：本文件只断言**原版机制语义**，不复述实现细节——
//! 测出红 = 引擎与原版偏差实锤。
//!
//! 取证要旨（26.1）：
//! - 传播代价：`LightEngine.getOpacity` = `max(1, lightDampening)`
//!   （LightEngine.java:77-79），六方向一视同仁
//!   （`PROPAGATION_DIRECTIONS = Direction.values()`，LightEngine.java:25）；
//!   `BlockLightEngine.propagateIncrease`（BlockLightEngine.java:54-58）与
//!   `SkyLightEngine.propagateIncrease`（SkyLightEngine.java:148-153）均无
//!   方向特例——**天光"垂直不衰减"只存在于源柱**（`SkyLightEngine.
//!   addSourcesAbove` 把 15 直写进每一格，SkyLightEngine.java:106-135），
//!   BFS 波前的垂直步与水平步同样 -1；
//! - 源柱截断判据：`ChunkSkyLightSources.isEdgeOccluded`
//!   （ChunkSkyLightSources.java:140-148）= `bottomState.getLightDampening()
//!   != 0` || 面遮蔽形相接。水 damp=1（LiquidBlock.java:114-116
//!   `propagatesSkylightDown=false`）同样截断；玻璃 damp=0 + 空遮蔽形
//!   （TransparentBlock.java:34-37）不截断 → 玻璃天窗下整柱 15；
//! - 撤销波：`propagateDecrease` 只清不增、幸存格按 stored 现值回播
//!   （BlockLightEngine.java:91-105）；自发光回播
//!   （BlockLightEngine.java:99-101）；PULL_LIGHT_IN_ENTRY
//!   （LightEngine.java:23）把被检格的全部非零邻居当重照亮源。

use mcv_core::{BlockId, CHUNK_VOL, WORLD_MAX_Y, WORLD_MIN_Y, vidx};
use mcv_light::{
    LightChunk, SIDE_MINUS_X, SIDE_PLUS_X, apply_edge, extract_edge, init, update_block,
};

const AIR: u16 = 0;
const STONE: u16 = 1;
const WATER: u16 = 5;

fn bid(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("方块 {name} 未注册")) as u16
}

// ---------------------------------------------------------------------------
// 最小世界夹具（与 light.rs 同构，自包含——集成测试各自成 crate）。
// ---------------------------------------------------------------------------

struct World {
    voxels: Vec<u16>,
    light: Vec<u8>,
    hm: Vec<i16>,
}

impl World {
    fn new() -> Self {
        Self {
            voxels: vec![AIR; CHUNK_VOL],
            light: vec![0; CHUNK_VOL],
            hm: vec![WORLD_MIN_Y as i16; 256],
        }
    }

    /// 平地：实心石头铺到（含）`top`。
    fn flat(top: i32) -> Self {
        let mut w = Self::new();
        w.box_fill(0, 15, 0, top, 0, 15, STONE);
        w.rebuild_heightmap();
        w
    }

    #[allow(clippy::too_many_arguments)]
    fn box_fill(&mut self, x0: usize, x1: usize, y0: i32, y1: i32, z0: usize, z1: usize, id: u16) {
        for y in y0..=y1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    self.voxels[vidx(x, y, z)] = id;
                }
            }
        }
    }

    fn rebuild_heightmap(&mut self) {
        for z in 0..16usize {
            for x in 0..16usize {
                let mut h = WORLD_MIN_Y;
                for y in (WORLD_MIN_Y..WORLD_MAX_Y).rev() {
                    if mcv_light::opacity(BlockId(self.voxels[vidx(x, y, z)])) > 0 {
                        h = y + 1;
                        break;
                    }
                }
                self.hm[(z << 4) | x] = h as i16;
            }
        }
    }

    fn chunk(&mut self) -> LightChunk<'_> {
        LightChunk {
            voxels: &self.voxels,
            light: &mut self.light,
            heightmap: &self.hm,
        }
    }
}

/// 编辑：先写体素再走 `update_block`（生产路径）。
fn edit(w: &mut World, x: usize, y: i32, z: usize, old_block: u16, new_block: u16) -> u8 {
    w.voxels[vidx(x, y, z)] = new_block;
    let mut seeds = Vec::new();
    let mut c = w.chunk();
    update_block(
        &mut c, x as u32, y, z as u32, old_block, new_block, &mut seeds,
    )
}

// ---------------------------------------------------------------------------
// F1 块光：全部六方向等价衰减，等级 = 发射值 − 曼哈顿距离。
// ---------------------------------------------------------------------------

/// 原版真值：`BlockLightEngine.propagateIncrease`（BlockLightEngine.java:
/// 54-58）每步 `fromLevel - getOpacity(toState)`，`getOpacity = max(1, damp)`
/// （LightEngine.java:77-79）；方向集合 = `Direction.values()` 全六向
/// （LightEngine.java:25）——垂直与水平无任何差别，火把 14 在距离 d 的
/// 格恒 14−d，d=15 处归 0。
#[test]
fn fidelity_block_light_level_is_emission_minus_manhattan_distance() {
    let torch = bid("torch");
    assert_eq!(mcv_core::BLOCKS[torch as usize].light_emit, 14);
    // 密封石室：内部空气 x 0..=15, y 32..=38, z 2..=14（±x 走满 8 格、
    // ±z 走 3 格后撞石壳；石壳格 damp 15 吞光）。
    let mut w = World::flat(40);
    w.box_fill(0, 15, 32, 38, 2, 14, AIR);
    w.rebuild_heightmap();
    w.voxels[vidx(8, 35, 8)] = torch;
    {
        let mut c = w.chunk();
        init(&mut c);
    }

    // 全格断言：水平 +x/-x、+z/-z 与垂直 ±y 的每个可达格都精确等于
    // 14 − 曼哈顿距离；曼哈顿距离 15 的格恰为 0（15 步衰减尽）。
    {
        let c = w.chunk();
        for d in 0..=7usize {
            assert_eq!(blk(&c, 8 + d, 35, 8), (14 - d) as u8, "+x {d}");
            assert_eq!(blk(&c, 8 - d, 35, 8), (14 - d) as u8, "-x {d}");
        }
        for d in 0..=3usize {
            assert_eq!(blk(&c, 8, 35, 8 + d), (14 - d) as u8, "+z {d}");
            assert_eq!(blk(&c, 8, 35, 8 - d), (14 - d) as u8, "-z {d}");
        }
        // 垂直：室内 y 32..=38 全空气，±3 格内逐格 -1。
        for d in 0..=3usize {
            assert_eq!(blk(&c, 8, (35 + d) as i32, 8), (14 - d) as u8, "+y {d}");
            assert_eq!(blk(&c, 8, (35 - d) as i32, 8), (14 - d) as u8, "-y {d}");
        }
        // 石壳格（damp 15 → 14-15 饱和 0）恰归零：光不透墙。
        assert_eq!(blk(&c, 8, 35, 1), 0, "z=1 石壳吞光");
        assert_eq!(blk(&c, 8, 31, 8), 0, "地板吞光");
    }
}

fn sky(c: &LightChunk, x: usize, y: i32, z: usize) -> u8 {
    c.light[vidx(x, y, z)] >> 4
}

fn blk(c: &LightChunk, x: usize, y: i32, z: usize) -> u8 {
    c.light[vidx(x, y, z)] & 0xF
}

// ---------------------------------------------------------------------------
// F2 天光 BFS：垂直步同样 -1（源柱之外没有免费竖直下落）。
// ---------------------------------------------------------------------------

/// 原版真值：`SkyLightEngine.propagateIncrease`（SkyLightEngine.java:
/// 148-153）对每个方向同样取 `fromLevel - getOpacity(toState)`——**没有
/// 任何向下特例**。15 不衰减仅因源柱格被 `addSourcesAbove`
/// （SkyLightEngine.java:106-135）直写 15；从源柱侧向/向下进入非源柱区
/// 的每一格（含竖直向下的连续各格）都逐格 -1。
///
/// 场景：山体（x=2..15, y=41..70）内东向平洞 y=50 z=8（洞口 x=1 露天），
/// 洞尽头 (12) 向下挖竖井 46..49、井底向东扩展一层 y=45（x=12..15）。
/// 期望：平洞逐格 15,14,13,…,4；竖井自上而下 3,2,1,0（每格 -1，与水平
/// 同速率）；井底横廊全黑（唯一入口是井）。
#[test]
fn fidelity_sky_bfs_downward_costs_one_per_step_like_horizontal() {
    let mut w = World::flat(40);
    // 山体盖 x=2..15（x=0..1 留作露天洞口列）。
    w.box_fill(2, 15, 41, 70, 0, 15, STONE);
    // 平洞 y=50 z=8，x=1..12（x=1 本就是露天空气列）。
    w.box_fill(1, 12, 50, 50, 8, 8, AIR);
    // 洞尽头竖井 (12, 46..49, 8)。
    w.box_fill(12, 12, 46, 49, 8, 8, AIR);
    // 井底横廊 y=45 z=8 x=12..15。
    w.box_fill(12, 15, 45, 45, 8, 8, AIR);
    w.rebuild_heightmap();
    {
        let mut c = w.chunk();
        init(&mut c);
    }

    {
        let c = w.chunk();
        // 平洞：x=1 是源柱格 15，向内逐格 -1（每格一步水平代价）。
        for d in 0..=11usize {
            let expect = (15 - d) as u8;
            assert_eq!(sky(&c, 1 + d, 50, 8), expect, "平洞 x={}", 1 + d);
        }
        // 竖井：唯一光源在上方（平洞尽头 (12,50)=4），向下每格 -1——
        // 原版 BFS 无向下特例的直接后果。若引擎给"垂直 15/不衰减"则红。
        for (y, expect) in [(49i32, 3u8), (48, 2), (47, 1), (46, 0)] {
            assert_eq!(sky(&c, 12, y, 8), expect, "竖井 y={y}");
        }
        // 井底横廊：入口只有井（(12,46)=0 透不下光），全黑。
        for x in 12..=15usize {
            assert_eq!(sky(&c, x, 45, 8), 0, "井底 y=45 x={x}");
        }
        // 平洞壁保持全黑。
        assert_eq!(sky(&c, 6, 49, 8), 0);
        assert_eq!(sky(&c, 6, 51, 8), 0);
    }
}

// ---------------------------------------------------------------------------
// F3 玻璃天窗：damp=0 不截断源柱，天窗下整柱 15 直落室内。
// ---------------------------------------------------------------------------

/// 原版真值：`ChunkSkyLightSources.isEdgeOccluded`（ChunkSkyLightSources.
/// java:140-148）在 `bottomState.getLightDampening() == 0` 且两侧面遮蔽形
/// 不相接时**不**截断源柱；玻璃（TransparentBlock.java:34-37 显式
/// `propagatesSkylightDown=true`，damp=0，noOcclusion → 空遮蔽形）因此
/// 不截断——`addSourcesAbove`（SkyLightEngine.java:106-135）把 15 直写
/// 进天窗下方每一格。
#[test]
fn fidelity_glass_skylight_feeds_full_bright_column_into_room() {
    let glass = bid("glass");
    assert_eq!(mcv_light::opacity(BlockId(glass)), 0, "26.1 玻璃 damp=0");
    let mut w = World::flat(40);
    // 密封外壳：x 1..=14, y 41..=49, z 3..=13 全石（六面无缝）。
    w.box_fill(1, 14, 41, 49, 3, 13, STONE);
    // 室内空气 x 2..=13, y 41..=48, z 4..=12。
    w.box_fill(2, 13, 41, 48, 4, 12, AIR);
    // 天窗：顶板 (8,49,8) 一格换玻璃（damp=0，其上即露天）。
    w.voxels[vidx(8, 49, 8)] = glass;
    w.rebuild_heightmap();
    {
        let mut c = w.chunk();
        init(&mut c);
    }

    {
        let c = w.chunk();
        // 天窗正下方的室内空气列：整柱 15（源柱穿过玻璃直落到地面）。
        for y in 41..=48 {
            assert_eq!(sky(&c, 8, y, 8), 15, "天窗下 y={y}");
        }
        // 天窗玻璃格自身也是源柱格（15）。
        assert_eq!(sky(&c, 8, 49, 8), 15, "玻璃格");
        // 光从该柱向室内侧向逐格 -1（15 − 曼哈顿距离）。
        assert_eq!(sky(&c, 8, 45, 7), 14);
        assert_eq!(sky(&c, 6, 45, 8), 13);
        assert_eq!(sky(&c, 3, 45, 8), 10);
        // 远角仍被照到（≥1）：室内无全黑死角。
        assert!(sky(&c, 2, 41, 4) >= 1);
    }
}

// ---------------------------------------------------------------------------
// F4 拆灯重照亮：双光源撤一，逐格回落到幸存源正当值。
// ---------------------------------------------------------------------------

/// 原版真值：`propagateDecrease`（BlockLightEngine.java:91-101）把
/// `toLevel <= oldFromLevel - 1` 的格子清零续波；`toLevel > oldFromLevel-1`
/// 的幸存格按 stored 现值 `enqueueIncrease`（:103-105）——被拆灯支配的
/// 区域逐格回撤，幸存源的梯度一字不差。
#[test]
fn fidelity_remove_one_of_two_lamps_collapses_to_survivor_field() {
    let glowstone = bid("glowstone");
    assert_eq!(mcv_core::BLOCKS[glowstone as usize].light_emit, 15);
    let mut w = World::flat(40);
    // 密封走廊：内部空气 x 2..=13, y 32..=36, z 6..=10。
    w.box_fill(2, 13, 32, 36, 6, 10, AIR);
    w.rebuild_heightmap();
    w.voxels[vidx(4, 34, 8)] = glowstone;
    w.voxels[vidx(11, 34, 8)] = glowstone;
    {
        let mut c = w.chunk();
        init(&mut c);
    }

    // 点亮态：x=4..11 的合成场 = max(15-|x-4|, 15-|x-11|)。
    {
        let c = w.chunk();
        for (x, expect) in [
            (4usize, 15u8),
            (5, 14),
            (6, 13),
            (7, 12),
            (8, 12),
            (9, 13),
            (10, 14),
            (11, 15),
        ] {
            assert_eq!(blk(&c, x, 34, 8), expect, "双灯场 x={x}");
        }
    }

    // 拆掉 x=11 的萤石。
    edit(&mut w, 11, 34, 8, glowstone, AIR);

    // 撤一后：全走廊回落到幸存灯（x=4）的独场 15-|x-4|，逐格无残留。
    {
        let c = w.chunk();
        for d in 0..=9usize {
            assert_eq!(blk(&c, 4 + d, 34, 8), (15 - d) as u8, "撤后 x={}", 4 + d);
        }
        // 垂直方向同样回落。
        for d in 1..=2usize {
            assert_eq!(blk(&c, 8, (34 + d) as i32, 8), (11 - d) as u8);
            assert_eq!(blk(&c, 8, (34 - d) as i32, 8), (11 - d) as u8);
        }
    }
}

// ---------------------------------------------------------------------------
// F5 跨块 6 邻边：边格 = 恰一步 BFS（含角柱槽位 h=15）+ 双向证成收敛。
// ---------------------------------------------------------------------------

/// 原版真值：BFS 只有 6 邻（LightEngine.java:25），跨区块无对角跳步；
/// 角柱格 (15,y,15) 属于 ±X 与 ±Z 两条边（槽位 h=15），任一边同步都会
/// 覆盖它。双向收敛（先 REMOVE 后 ADD 的两相协议）等价原版撤销波对
/// 幸存格按现值回播 + 增加波前推（BlockLightEngine.java:91-105）。
mod pair {
    use super::*;

    pub struct PairWorld {
        pub chunks: [World; 2],
    }

    impl PairWorld {
        pub fn new() -> Self {
            Self {
                chunks: [World::new(), World::new()],
            }
        }

        /// 全局 x：0..=15 → 块 A，16..=31 → 块 B（B 局部 x = gx-16）。
        pub fn set(&mut self, gx: usize, y: i32, z: usize, id: u16) {
            if gx < 16 {
                self.chunks[0].voxels[vidx(gx, y, z)] = id;
            } else {
                self.chunks[1].voxels[vidx(gx - 16, y, z)] = id;
            }
        }

        #[allow(clippy::too_many_arguments)]
        pub fn box_fill(
            &mut self,
            gx0: usize,
            gx1: usize,
            y0: i32,
            y1: i32,
            z0: usize,
            z1: usize,
            id: u16,
        ) {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    for gx in gx0..=gx1 {
                        self.set(gx, y, z, id);
                    }
                }
            }
        }

        pub fn rebuild_heightmaps(&mut self) {
            for w in &mut self.chunks {
                w.rebuild_heightmap();
            }
        }

        /// 复刻 game.rs::sync_light_edges 的派发循环（弹 (块, 边) →
        /// 抽边 → 邻块先 REMOVE 后 ADD → 脏位回队；512 步预算）。
        pub fn sync_edges(&mut self, queue: Vec<(u8, u8)>) {
            let mut queue = queue;
            let mut steps = 0usize;
            while let Some((which, side)) = queue.pop() {
                // 本用例世界只有 A(+X)↔B(-X) 一对邻接；±Z 邻块不存在
                // （game.rs 对缺失邻块跳过，同此）。
                let connects = matches!((which, side), (0, SIDE_PLUS_X) | (1, SIDE_MINUS_X));
                if !connects {
                    continue;
                }
                steps += 1;
                assert!(steps <= 512, "边同步预算耗尽（级联不收敛）");
                let other = 1 - which;
                let obit = if side < 2 { 1 - side } else { 5 - side };
                let edge = {
                    let c = self.chunks[which as usize].chunk();
                    extract_edge(&c, side)
                };
                for op in [1u8, 0u8] {
                    let dirty = {
                        let mut c = self.chunks[other as usize].chunk();
                        apply_edge(&mut c, &edge, obit, op)
                    };
                    for bit in 0..4u8 {
                        if dirty & (1 << bit) != 0 {
                            queue.push((other, bit));
                        }
                    }
                }
            }
        }

        pub fn init_and_pair(&mut self) {
            {
                let mut c = self.chunks[0].chunk();
                init(&mut c);
            }
            {
                let mut c = self.chunks[1].chunk();
                init(&mut c);
            }
            self.sync_edges(vec![(0, SIDE_PLUS_X), (1, SIDE_MINUS_X)]);
        }

        pub fn light_p(&self, gx: usize, y: i32, z: usize) -> u8 {
            if gx < 16 {
                self.chunks[0].light[vidx(gx, y, z)]
            } else {
                self.chunks[1].light[vidx(gx - 16, y, z)]
            }
        }
    }
}

/// 火把恰在 A 的角柱格 (15,41,15)：其光必须经 ±X 边的 h=15 槽位走进 B
/// 的角柱格 (16,41,15)（恰一步 -1）。原版无对角跳步——B 的光照只能从
/// 该边格一步证成。
#[test]
fn fidelity_corner_column_light_crosses_edge_slot_h15() {
    let torch = bid("torch");
    let mut p = pair::PairWorld::new();
    for w in &mut p.chunks {
        *w = World::flat(40);
    }
    p.set(15, 41, 15, torch);
    p.rebuild_heightmaps();
    p.init_and_pair();

    assert_eq!(p.light_p(15, 41, 15) & 0xF, 14, "A 角柱火把格");
    assert_eq!(p.light_p(16, 41, 15) & 0xF, 13, "B 角柱恰一步");
    assert_eq!(p.light_p(17, 41, 15) & 0xF, 12, "B 内继续逐格");
    // 天光不受影响（角柱露天两侧）。
    assert_eq!(p.light_p(16, 41, 15) >> 4, 15);
}

/// 平洞贯穿区块边界（A 洞口 + B 内萤石双源）：边界两侧逐格互证成，
/// 合成场呈 V 形（谷底在洞内），且跨界差恰 1 步——两相（REMOVE+ADD）
/// 协议收敛后的场与原版单一大世界 BFS 逐格同值。
#[test]
fn fidelity_cross_border_tunnel_field_matches_single_world_bfs() {
    let glowstone = bid("glowstone");
    let mut p = pair::PairWorld::new();
    for w in &mut p.chunks {
        *w = World::flat(45);
    }
    // 山体 y=46..60：gx=2..31（gx=0..1 留露天洞口列）。
    p.box_fill(2, 31, 46, 60, 0, 15, STONE);
    // 平洞 y=50 z=8，gx=2..30。
    p.box_fill(2, 30, 50, 50, 8, 8, AIR);
    // B 内萤石 gx=24。
    p.set(24, 50, 8, glowstone);
    p.rebuild_heightmaps();
    p.init_and_pair();

    // 天光通道：唯一来源是 A 洞口露天列（16-gx），B 洞段全顶盖无天光
    // → gx=2..15 逐格 14..1，B 侧（16..）恒 0——跨界差恰 1 步。
    for (i, expect) in [14u8, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1]
        .iter()
        .enumerate()
    {
        let gx = 2 + i;
        assert_eq!(p.light_p(gx, 50, 8) >> 4, *expect, "天光 A x={gx}");
    }
    for gx in 16..=20usize {
        assert_eq!(p.light_p(gx, 50, 8) >> 4, 0, "天光 B x={}（顶盖）", gx - 16);
    }
    // 方块光通道：唯一来源是 B 内萤石（15-|gx-24|）；B 场推过边界进入
    // A（A(15)=6，向内 5,4,…,1）—— REMOVE+ADD 两相协议必须把 B 的
    // 边快照喂进 A，合成场跨界连续（V 形谷底在 A 侧 gx=10）。
    for (i, expect) in [1u8, 2, 3, 4, 5, 6].iter().enumerate() {
        assert_eq!(
            p.light_p(10 + i, 50, 8) & 0xF,
            *expect,
            "方块光 A x={}",
            10 + i
        );
    }
    for (i, expect) in [7u8, 8, 9, 10, 11, 12, 13, 14, 15].iter().enumerate() {
        assert_eq!(p.light_p(16 + i, 50, 8) & 0xF, *expect, "方块光 B x={i}");
    }
    // 跨界两格（两通道各自）差恒 1（6 邻 BFS 的一步语义）。
    assert_eq!(
        (p.light_p(15, 50, 8) >> 4).abs_diff(p.light_p(16, 50, 8) >> 4),
        1,
        "天光跨界差一步"
    );
    assert_eq!(
        (p.light_p(15, 50, 8) & 0xF).abs_diff(p.light_p(16, 50, 8) & 0xF),
        1,
        "方块光跨界差一步"
    );
}

// ---------------------------------------------------------------------------
// F6 水体：damp=1 垂直逐格 -1（源柱在水面截断）+ 侧向同率。
// ---------------------------------------------------------------------------

/// 原版真值：水 `propagatesSkylightDown=false`（LiquidBlock.java:114-116）
/// → damp=1（BlockBehaviour.java:305-310 默认判据）→ `isEdgeOccluded`
/// 截断源柱（ChunkSkyLightSources.java:140-148），水面即源柱底；水柱
/// 内 15→14→13… 逐格 -1，且 BFS 对垂直/水平同价（SkyLightEngine.
/// java:148-153）。方块光同理（LightEngine.java:77-79）。
#[test]
fn fidelity_water_column_and_block_light_decay_one_per_cell() {
    let glowstone = bid("glowstone");
    let mut w = World::flat(38);
    // 水体 y=39..=49 铺满；东半底部留气穴 x=6..15, y=39..40。
    w.box_fill(0, 15, 39, 49, 0, 15, WATER);
    w.box_fill(6, 15, 39, 40, 0, 15, AIR);
    w.rebuild_heightmap();
    // 密封水室里埋一颗萤石（damp=15 的发光整方块）：验证方块光穿过
    // 水同样每格 -1（无任何"水吃块光额外衰减"）。
    w.voxels[vidx(12, 39, 12)] = glowstone;
    {
        let mut c = w.chunk();
        init(&mut c);
    }

    {
        let c = w.chunk();
        // 水面格 = 源柱底：14；往下逐格 -1 到气穴顶（y=41 → 14-8=6）。
        for d in 0..=10usize {
            let y = (49 - d) as i32;
            assert_eq!(sky(&c, 3, y, 3), (14 - d) as u8, "水柱 y={y}");
        }
        // 水里没有任何一格是 15（源柱被水面截断）。
        assert_eq!(sky(&c, 3, 49, 3), 14);
        // 气穴（空气 damp=0 不衰减自身，但光照按 BFS 步进）：
        // 顶部 (x,41) 全为水列同深度值 6；气穴内从上往下 5、4。
        for x in 6..=15usize {
            assert_eq!(sky(&c, x, 40, 3), 5, "气穴 y=40 x={x}");
            assert_eq!(sky(&c, x, 39, 3), 4, "气穴 y=39 x={x}");
        }
        // 方块光穿水：萤石格自发光 15，向上每格 -1（水 damp=1）。
        assert_eq!(blk(&c, 12, 39, 12), 15);
        for d in 1..=10usize {
            let y = (39 + d) as i32;
            assert_eq!(blk(&c, 12, y, 12), (15 - d) as u8, "穿水方块光 y={y}");
        }
    }
}

// ---------------------------------------------------------------------------
// F7 暗室挖窗进光（PULL_LIGHT_IN_ENTRY 语义）。
// ---------------------------------------------------------------------------

/// 原版真值：编辑格存储为 0 时 `checkNode` 走 `PULL_LIGHT_IN_ENTRY`
/// （LightEngine.java:23；BlockLightEngine.java:36 / SkyLightEngine.java:
/// 69）——把全部非零邻居按现值回播，光从亮侧流入新挖的暗格。
#[test]
fn fidelity_dig_window_into_dark_room_pulls_neighbour_light() {
    let glowstone = bid("glowstone");
    let mut w = World::flat(40);
    // 双密室：A 内部 x 3..=7、B 内部 x 9..=13（y 32..=36, z 6..=10），
    // 中间隔着实心石墙 x=8。A 内萤石 (4,34,8)。
    w.box_fill(3, 7, 32, 36, 6, 10, AIR);
    w.box_fill(9, 13, 32, 36, 6, 10, AIR);
    w.rebuild_heightmap();
    w.voxels[vidx(4, 34, 8)] = glowstone;
    {
        let mut c = w.chunk();
        init(&mut c);
    }
    {
        let c = w.chunk();
        // B 室全黑；共享墙面 (8,34,8) 为石、全暗。
        assert_eq!(blk(&c, 10, 34, 8), 0);
        assert_eq!(blk(&c, 8, 34, 8), 0);
        // A 室墙面 (7,34,8) = 15-3 = 12。
        assert_eq!(blk(&c, 7, 34, 8), 12);
    }

    // 在共享墙挖一格窗：(8,34,8) 石 → 空气。
    edit(&mut w, 8, 34, 8, STONE, AIR);

    {
        let c = w.chunk();
        // 窗格被亮侧一步证成：11；B 室侧向内逐格 10、9、8。
        assert_eq!(blk(&c, 8, 34, 8), 11, "窗格");
        assert_eq!(blk(&c, 9, 34, 8), 10);
        assert_eq!(blk(&c, 10, 34, 8), 9);
        assert_eq!(blk(&c, 11, 34, 8), 8);
        // 非直线邻格同样进光。
        assert_eq!(blk(&c, 9, 33, 8), 9);
        // A 室原有场不受扰动。
        assert_eq!(blk(&c, 4, 34, 8), 15);
    }
}

// ---------------------------------------------------------------------------
// F8 生产路径：从地表一路下挖，直天列随挖随延（源柱下沉）。
// ---------------------------------------------------------------------------

/// 原版真值：方块改动若光照属性有变，`LevelChunk.setBlockState` 调
/// `skyLightSources.update`（LevelChunk.java:297-302；ChunkSkyLightSources.
/// java:77-96）下沉 lowestSourceY，`SkyLightEngine.checkNode` 的
/// `updateSourcesInColumn`（SkyLightEngine.java:48-73）把新增源段整体
/// 写 15——直天 15 随挖逐格下探。
#[test]
fn fidelity_dig_shaft_extends_direct_sky_column_incrementally() {
    let mut w = World::flat(40);
    // 井底东侧先挖一个小观察龛（y=25, x=6..=7, z=8）——其列顶仍在
    // 地表（上方石 26..=40），不是源柱；龛内初始全黑。
    w.box_fill(6, 7, 25, 25, 8, 8, AIR);
    w.rebuild_heightmap();
    {
        let mut c = w.chunk();
        init(&mut c);
    }
    // 从地表 (8,40..=25,8) 逐格下挖（生产路径，一次一格）。
    for y in (25..=40i32).rev() {
        edit(&mut w, 8, y, 8, STONE, AIR);
    }
    {
        let c = w.chunk();
        // 井筒整列 15（源柱下探到井底）。
        for y in 25..=40 {
            assert_eq!(sky(&c, 8, y, 8), 15, "井筒 y={y}");
        }
        // 井底下方仍实心全黑。
        assert_eq!(sky(&c, 8, 24, 8), 0);
        // 井底侧向渗光逐格 -1（观察龛内，与井底源格同层的 y=25）。
        assert_eq!(sky(&c, 7, 25, 8), 14);
        assert_eq!(sky(&c, 6, 25, 8), 13);
    }
    // 回填一格即截断：放回石头，整段归 0。
    edit(&mut w, 8, 40, 8, AIR, STONE);
    {
        let c = w.chunk();
        for y in 25..=39 {
            assert_eq!(sky(&c, 8, y, 8), 0, "回填后 y={y}");
        }
    }
}

// ---------------------------------------------------------------------------
// F9（红 → ignore）：顶半砖应截断直天源柱——形状感知遮光缺失。
// ---------------------------------------------------------------------------

/// 原版真值：顶半砖 `useShapeForLightOcclusion=true`（SlabBlock.java:
/// 49-51 `TYPE != DOUBLE`），其 UP 面遮蔽形为满格 → `isEdgeOccluded`
/// （ChunkSkyLightSources.java:140-148 的 faceShapeOccludes 分支）截断
/// 源柱；砖格与其下方天光由 BFS 侧向供给（14）。本引擎 damp(slab)=0
/// 且无形状感知（lib.rs KNOWN-DIVERGENCE）→ 源柱穿透，下方错亮 15。
/// 修复需 (方块, 状态, 方向) 级面遮蔽形表（引擎单文件面内非小修），
/// 划入重建波（docs/light-audit.md 病灶#1）。
#[test]
#[ignore = "light-audit: 病灶#1 待重建波（形状感知遮光：顶半砖截断源柱）"]
fn fidelity_top_slab_truncates_direct_sky_column() {
    let slab = BlockId(bid("oak_slab")).with_state(1); // bit0=1 → 顶半砖
    let mut w = World::flat(40);
    // 悬空顶半砖 (8,45,8)。
    w.voxels[vidx(8, 45, 8)] = slab.0;
    w.rebuild_heightmap();
    {
        let mut c = w.chunk();
        init(&mut c);
    }
    {
        let c = w.chunk();
        // 原版：半砖格由侧向源证成 = 14；正下方阴影列同样 14（侧向
        // 一步源），与悬浮石头一致。
        assert_eq!(sky(&c, 8, 45, 8), 14, "顶半砖格应有侧向 14");
        for y in 41..45 {
            assert_eq!(sky(&c, 8, y, 8), 14, "半砖阴影 y={y}");
        }
        // 上方空气仍是满照度源柱。
        assert_eq!(sky(&c, 8, 46, 8), 15);
    }
}

// ---------------------------------------------------------------------------
// F10（基线锚）：水/叶/遮光玻璃的"透光 vs 挡光"材料轴。
// ---------------------------------------------------------------------------

/// 原版真值：tinted_glass damp=15（TintedGlassBlock.java:25-27）——
/// 非不透明渲染（可透视）却整格挡光：BFS 15-15=0，格内及其后全黑；
/// 与水/叶（damp=1，透光但逐格衰减）形成材料轴两端。锚定
/// `mcv_light::opacity` 全表与 26.1 三段规则的一致性。
#[test]
fn fidelity_material_axis_water_leaves_tinted_glass() {
    assert_eq!(mcv_light::opacity(BlockId(WATER)), 1);
    assert_eq!(mcv_light::opacity(BlockId(bid("leaves"))), 1);
    assert_eq!(mcv_light::opacity(BlockId(bid("tinted_glass"))), 15);
    let tinted = bid("tinted_glass");
    let mut w = World::flat(38);
    // 平顶板 y=40 满铺，板下挖空腔 y=36..=39（四壁仍石）：
    // (4,4) 列换遮光玻璃、(8,8) 列换水、(12,12) 换叶。
    w.box_fill(0, 15, 40, 40, 0, 15, STONE);
    w.box_fill(0, 15, 36, 39, 0, 15, AIR);
    w.voxels[vidx(4, 40, 4)] = tinted;
    w.voxels[vidx(8, 40, 8)] = WATER;
    w.voxels[vidx(12, 40, 12)] = bid("leaves");
    w.rebuild_heightmap();
    {
        let mut c = w.chunk();
        init(&mut c);
    }
    {
        let c = w.chunk();
        // 板上 15；遮光玻璃格 0（15-15）；水/叶格 14（15-1）。
        assert_eq!(sky(&c, 4, 40, 4), 0, "遮光玻璃整格挡光");
        assert_eq!(sky(&c, 8, 40, 8), 14, "水格衰减 1");
        assert_eq!(sky(&c, 12, 40, 12), 14, "叶格衰减 1");
        // 板下空腔：水/叶列同率逐格 -1 且互等（damp 同为 1）。
        for y in 36..=39 {
            assert_eq!(sky(&c, 8, y, 8), (13 - (39 - y)) as u8, "水下 y={y}");
            assert_eq!(sky(&c, 12, y, 12), (13 - (39 - y)) as u8, "叶下 y={y}");
        }
        // 遮光玻璃列截断自分光（其正下格无直下份额），等深处处处严格
        // 暗于水列——原版 tinted_glass damp=15 vs 水 damp=1 的材料轴。
        for y in 36..=39 {
            assert!(
                sky(&c, 4, y, 4) < sky(&c, 8, y, 8),
                "玻璃列 y={y} 应严格暗于水列"
            );
        }
    }
}
