//! Synthetic-voxel tests for the BFS lighting engine.
//!
//! 光照规则参照反编译 Minecraft 26.1（机制与常数提取见仓库外笔记
//! `mc-ref/NOTES-light.md`）：
//! - 传播代价 = `max(1, lightDampening)`，六个方向一视同仁；
//! - 天光"垂直 15 不衰减"仅存在于源柱（全透空气柱），水/叶
//!   （damp=1）截断源柱并逐格 -1；
//! - 方块光火把参照值 14，每格 -1。

use mcv_core::{BlockId, CHUNK_VOL, vidx};
use mcv_light::{
    BorderSeed, LightChunk, SIDE_MINUS_X, SIDE_MINUS_Z, SIDE_PLUS_X, SIDE_PLUS_Z, apply_edge,
    extract_edge, init, opacity, propagate, update_block,
};

const AIR: u16 = 0;
const STONE: u16 = 1;
// 与 mcv_core::BLOCKS 顺序一致的 id 常量（黄金用例引用）。
const WATER: u16 = 5;
const LEAVES: u16 = 7;
const FLOWER_RED: u16 = 12;

struct World {
    voxels: Vec<u16>,
    light: Vec<u8>,
    hm: Vec<u8>,
}

impl World {
    fn new() -> Self {
        Self {
            voxels: vec![AIR; CHUNK_VOL],
            light: vec![0; CHUNK_VOL],
            hm: vec![0; 256],
        }
    }

    /// Flat ground: solid stone up to and including `top`.
    fn flat(top: usize) -> Self {
        let mut w = Self::new();
        w.box_fill(0, 15, 0, top, 0, 15, STONE);
        w.rebuild_heightmap();
        w
    }

    #[allow(clippy::too_many_arguments)]
    fn box_fill(
        &mut self,
        x0: usize,
        x1: usize,
        y0: usize,
        y1: usize,
        z0: usize,
        z1: usize,
        id: u16,
    ) {
        for y in y0..=y1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    self.voxels[vidx(x, y, z)] = id;
                }
            }
        }
    }

    /// Heightmap = highest light-blocking block y + 1 (0 = open column).
    fn rebuild_heightmap(&mut self) {
        for z in 0..16usize {
            for x in 0..16usize {
                let mut h = 0usize;
                for y in (0..256).rev() {
                    if opacity(BlockId(self.voxels[vidx(x, y, z)])) > 0 {
                        h = y + 1;
                        break;
                    }
                }
                self.hm[(z << 4) | x] = h as u8;
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

fn sky(c: &LightChunk, x: usize, y: usize, z: usize) -> u8 {
    c.light[vidx(x, y, z)] >> 4
}

fn blk(c: &LightChunk, x: usize, y: usize, z: usize) -> u8 {
    c.light[vidx(x, y, z)] & 0xF
}

fn sky_w(w: &World, x: usize, y: usize, z: usize) -> u8 {
    w.light[vidx(x, y, z)] >> 4
}

/// Place/dig through `update_block`, writing the voxel first (the engine's
/// voxel view is read-only; it assumes the edit is already applied).
fn edit(
    w: &mut World,
    x: usize,
    y: usize,
    z: usize,
    old_block: u16,
    new_block: u16,
) -> (u8, Vec<BorderSeed>) {
    w.voxels[vidx(x, y, z)] = new_block;
    let mut seeds = Vec::new();
    let mask = {
        let mut c = w.chunk();
        update_block(
            &mut c, x as u32, y as u32, z as u32, old_block, new_block, &mut seeds,
        )
    };
    (mask, seeds)
}

#[test]
fn flat_ground_direct_sky() {
    let mut w = World::flat(40);
    let mut c = w.chunk();
    let mask = init(&mut c);
    // Every open column is direct sunlight all the way down to the surface.
    for z in 0..16usize {
        for x in 0..16usize {
            for y in 41..256 {
                assert_eq!(sky(&c, x, y, z), 15, "sky at ({x},{y},{z})");
                assert_eq!(blk(&c, x, y, z), 0);
            }
            // Surface block itself and everything below stay dark.
            assert_eq!(sky(&c, x, 40, z), 0);
            assert_eq!(sky(&c, x, 0, z), 0);
        }
    }
    // All four borders carry light.
    assert_eq!(mask, 0b1111);
}

#[test]
fn sealed_room_block_light() {
    let mut w = World::flat(40);
    // Carve a sealed room out of the solid ground: interior air with a
    // one-block stone shell (interior x/z 3..=12, y 32..=38).
    w.box_fill(3, 12, 32, 38, 3, 12, AIR);
    w.rebuild_heightmap();
    let mut c = w.chunk();
    init(&mut c);
    // Sealed interior: no sky, no block light.
    assert_eq!(sky(&c, 5, 35, 5), 0);
    assert_eq!(blk(&c, 5, 35, 5), 0);

    // Hand-seed a block-light source at level 14 and converge.
    c.light[vidx(5, 35, 5)] = 14;
    let mask = propagate(&mut c);
    assert_eq!(mask, 0);
    for d in 0..=7usize {
        let expect = (14 - d) as u8;
        assert_eq!(blk(&c, 5 + d, 35, 5), expect, "x+{d}");
        assert_eq!(blk(&c, 5, 35, 5 + d), expect, "z+{d}");
    }
    // Interior height limits the vertical falloff to 3 cells.
    for d in 1..=3usize {
        let expect = (14 - d) as u8;
        assert_eq!(blk(&c, 5, 35 - d, 5), expect, "y-{d}");
        assert_eq!(blk(&c, 5, 35 + d, 5), expect, "y+{d}");
    }
    // Walls swallow the light: shell and the world beyond stay 0.
    assert_eq!(blk(&c, 13, 35, 5), 0);
    assert_eq!(blk(&c, 2, 35, 5), 0);
    assert_eq!(blk(&c, 5, 31, 5), 0);
    assert_eq!(blk(&c, 5, 39, 5), 0);
    // Sky nibble untouched by the block-light BFS.
    assert_eq!(sky(&c, 5, 35, 5), 0);
}

#[test]
fn place_and_dig_casts_shadow() {
    let mut w = World::flat(40);
    {
        let mut c = w.chunk();
        init(&mut c);
    }

    // Float an opaque block in the open sky at (8, 45, 8).
    let (mask, seeds) = edit(&mut w, 8, 45, 8, AIR, STONE);
    assert_eq!(mask, 0);
    assert!(seeds.is_empty());
    assert_eq!(sky_w(&w, 8, 45, 8), 0);
    // Direct column below is cut: shadow filled at 14 from the sides.
    for y in 41..45 {
        assert_eq!(sky_w(&w, 8, y, 8), 14, "shadow at y={y}");
    }
    // Neighbour columns keep full sunlight.
    assert_eq!(sky_w(&w, 7, 44, 8), 15);
    assert_eq!(sky_w(&w, 9, 44, 8), 15);
    assert_eq!(sky_w(&w, 8, 46, 8), 15);

    // Dig it back out: direct column is restored.
    let (mask, seeds) = edit(&mut w, 8, 45, 8, STONE, AIR);
    assert_eq!(mask, 0);
    assert!(seeds.is_empty());
    for y in 41..=45 {
        assert_eq!(sky_w(&w, 8, y, 8), 15, "restored at y={y}");
    }
}

#[test]
fn edge_add_feeds_neighbour() {
    let mut a = World::flat(40);
    let mut b = World::flat(40);
    let edge = {
        let mut ca = a.chunk();
        init(&mut ca);
        extract_edge(&ca, 0)
    };
    let mut cb = b.chunk();
    init(&mut cb);
    // Simulate an unlit B: wipe its sky channel through the live view.
    cb.light.iter_mut().for_each(|l| *l &= 0x0F);

    // A's +X edge feeds B's -X edge.
    for y in 41..256 {
        assert_eq!(edge[((y << 4) + 3) as usize], 0xF0);
    }
    let mask = apply_edge(&mut cb, &edge, 1, 0); // side 1 = -X, ADD
    assert_eq!(sky(&cb, 0, 50, 0), 14);
    assert_eq!(sky(&cb, 1, 50, 0), 13);
    assert_eq!(sky(&cb, 3, 50, 0), 11);
    assert_eq!(sky(&cb, 0, 41, 7), 14);
    // Opaque ground stays dark; far side of B untouched.
    assert_eq!(sky(&cb, 0, 40, 0), 0);
    assert_eq!(sky(&cb, 15, 50, 8), 0);
    // The synced side is never reported back; the z-borders are.
    assert_eq!(mask & 0b0011, 0);
    assert_eq!(mask & 0b1100, 0b1100);
}

#[test]
fn edge_remove_retracts_light() {
    let mut a = World::flat(40);
    let mut b = World::flat(40);
    let edge = {
        let mut ca = a.chunk();
        init(&mut ca);
        extract_edge(&ca, 0)
    };
    let mut cb = b.chunk();
    init(&mut cb);
    cb.light.iter_mut().for_each(|l| *l &= 0x0F);

    apply_edge(&mut cb, &edge, 1, 0);
    assert_eq!(sky(&cb, 0, 50, 0), 14);

    // The neighbour's light went away: retract the whole gradient.
    let dark = [0u8; 4096];
    apply_edge(&mut cb, &dark, 1, 1); // REMOVE
    assert_eq!(sky(&cb, 0, 50, 0), 0);
    assert_eq!(sky(&cb, 1, 50, 0), 0);
    assert_eq!(sky(&cb, 5, 50, 0), 0);
    assert_eq!(sky(&cb, 0, 42, 7), 0);
    assert_eq!(sky(&cb, 13, 50, 8), 0);
}

#[test]
fn tunnel_light_falloff() {
    let mut w = World::flat(39);
    // Mountain slab over x/z 2..=15, y 40..=60; columns below stay open.
    w.box_fill(2, 15, 40, 60, 2, 15, STONE);
    // East-west tunnel at y=50, z=8, mouth at x=2 (x=1 is open air).
    w.box_fill(2, 12, 50, 50, 8, 8, AIR);
    w.rebuild_heightmap();
    let mut c = w.chunk();
    init(&mut c);

    // The mouth cell is lit by the open column beside it, then light
    // decays by exactly 1 per tunnel cell.
    assert_eq!(sky(&c, 1, 50, 8), 15);
    for d in 0..=10usize {
        let expect = (14 - d) as u8;
        assert_eq!(sky(&c, 2 + d, 50, 8), expect, "tunnel depth {d}");
    }
    // Tunnel walls stay dark.
    assert_eq!(sky(&c, 6, 49, 8), 0);
    assert_eq!(sky(&c, 6, 51, 8), 0);
    assert_eq!(sky(&c, 6, 50, 7), 0);
}

// ---------------------------------------------------------------------------
// 与反编译 Minecraft 26.1 对拍的黄金用例（表格断言）。
// ---------------------------------------------------------------------------

/// 26.1 `Blocks.TORCH.lightLevel(14)`（Blocks.java:963-971）。火把已注册进
/// `mcv_core::BLOCKS`（gen 全表 1171 块、93 发光方块），发光值直查真相源
/// 而非手工常量——新增发光方块在源头登记后本表自动跟随（MINOR-1）。
fn torch_emission() -> u8 {
    mcv_core::BLOCKS[bid("torch") as usize].light_emit
}

/// 黄金用例 1：火把方块光衰减序列 14,13,…,1,0。
/// 参照 `BlockLightEngine.propagateIncrease`：六向每格
/// `-max(1, damp(target))`，垂直方向没有任何特例——竖直向下同样是
/// 13、12、…（区别于天光源柱）。
#[test]
fn golden_torch_decay_sequence() {
    // 密封石室：内部空气 x 1..=15, y 32..=36, z 6..=10，顶板 y37..=40。
    let mut w = World::flat(40);
    w.box_fill(1, 15, 32, 36, 6, 10, AIR);
    w.rebuild_heightmap();
    let torch_emit = torch_emission();
    assert_eq!(torch_emit, 14, "火把 26.1 发光值");
    {
        let mut c = w.chunk();
        init(&mut c);
        assert_eq!(sky(&c, 8, 34, 8), 0, "密封室内无天光");
        assert_eq!(blk(&c, 8, 34, 8), 0, "密封室内无方块光");
    }

    // 火把（发光 14）放在 (1,34,8)。
    w.light[vidx(1, 34, 8)] = torch_emit;
    let mask = {
        let mut c = w.chunk();
        propagate(&mut c)
    };
    // 所有边界格距火把曼哈顿距离 ≥14，方块光够不到任何边界。
    assert_eq!(mask, 0);

    // 水平序列（沿 +x）：14,13,12,…,1，第 15 格衰减到 0。
    let sequence: Vec<u8> = (0..=14u8).map(|d| torch_emit.saturating_sub(d)).collect();
    let mut got = Vec::new();
    {
        let c = w.chunk();
        for d in 0..=14usize {
            got.push(blk(&c, 1 + d, 34, 8));
        }
    }
    assert_eq!(got, sequence, "火把水平衰减序列");

    // 垂直方向同样逐格 -1（26.1 方块光无垂直特例）。
    {
        let c = w.chunk();
        for (y, expect) in [(32usize, 12u8), (33, 13), (34, 14), (35, 13), (36, 12)] {
            assert_eq!(blk(&c, 1, y, 8), expect, "火把垂直 y={y}");
        }
        // 墙体吞光：石头 damp=15，14-15 饱和为 0，不透射。
        assert_eq!(blk(&c, 0, 34, 8), 0, "侧墙");
        assert_eq!(blk(&c, 1, 31, 8), 0, "地板");
        assert_eq!(blk(&c, 1, 37, 8), 0, "天花板");
    }
}

/// 黄金用例 2：天光源柱透射。
/// 参照 `ChunkSkyLightSources.lowestSourceY` + `SkyLightEngine`：
/// damp=0 的连续空气柱整体保持 15（垂直不衰减）；树叶（damp=1）
/// 截断源柱，从 15 起每格 -1；实心石台完全挡光；柱底侧向渗入
/// 平台下方的洞窟时每格 -1。
#[test]
fn golden_sky_source_column_transmission() {
    let mut w = World::new();
    // 悬空石台 y 60..=61 铺满整个 chunk（避免外圈全空列从边缘漏光）。
    w.box_fill(0, 15, 60, 61, 0, 15, STONE);
    // 竖井 (5,5)：整列打通 → 全透空气柱。
    w.box_fill(5, 5, 60, 61, 5, 5, AIR);
    // 树叶柱 (8,8)：仅在 y=61 塞一格树叶截断源柱（damp=1）。
    w.voxels[vidx(8, 61, 8)] = LEAVES;
    w.voxels[vidx(8, 60, 8)] = AIR;
    w.rebuild_heightmap();
    let mut c = w.chunk();
    init(&mut c);

    // 全透竖井整列 15：垂直下落穿透透明介质不衰减（源柱机制）。
    for y in [62usize, 61, 60, 59, 58, 30, 0] {
        assert_eq!(sky(&c, 5, y, 5), 15, "空气源柱 y={y} 应保持 15");
    }

    // 树叶截断源柱：15 → 14（入叶格）→ 13 → 12 …
    for (y, expect) in [
        (62u32, 15u8),
        (61, 14),
        (60, 13),
        (59, 12),
        (58, 11),
        (57, 10),
    ] {
        assert_eq!(sky(&c, 8, y as usize, 8), expect, "树叶柱 y={y}");
    }

    // 石台本体挡光；其上方仍是满照度。
    assert_eq!(sky(&c, 3, 61, 3), 0);
    assert_eq!(sky(&c, 3, 60, 3), 0);
    assert_eq!(sky(&c, 3, 62, 3), 15);

    // 井底侧向渗入平台下方洞窟：横向每格恰好 -1。
    for d in 0..=5usize {
        assert_eq!(sky(&c, 5 + d, 59, 5), 15 - d as u8, "洞窟横向第 {d} 格");
    }
}

/// 黄金用例 3：透射 vs 挡光（材料柱对照表）。damp 判据见
/// `mcv_core::OPACITY`：26.1 玻璃 `TransparentBlock.java:34-37` 显式覆写
/// `propagatesSkylightDown=true → damp 0`（全透），花（VegetationBlock
/// 系，`VegetationBlock.java:50-52`）同 0，此处用例以花为透射代表
/// （玻璃直查断言见 `opacity_full_table_matches_26_1`）。半透明水/叶
/// damp=1：截断源柱但每格只 -1；实心石头 damp=15：完全挡光。
#[test]
fn golden_sky_through_material_columns() {
    // (材料, 期望的天光序列 [(y, level)])：y=51 为板上方，50 为材料
    // 格本身，49..46 为板下洞窟中的柱内序列。
    const CASES: [(u16, [(u32, u8); 6]); 4] = [
        // 石头：板上方 15，材料格及其下全挡（15-15=0）。
        (
            STONE,
            [(51, 15), (50, 0), (49, 0), (48, 0), (47, 0), (46, 0)],
        ),
        // 树叶 damp=1：15 → 14 → 13 → …
        (
            LEAVES,
            [(51, 15), (50, 14), (49, 13), (48, 12), (47, 11), (46, 10)],
        ),
        // 水 damp=1（26.1 LiquidBlock；不是旧版的每格 -3）。
        (
            WATER,
            [(51, 15), (50, 14), (49, 13), (48, 12), (47, 11), (46, 10)],
        ),
        // 花（玻璃类 damp=0）：不截断源柱，整列 15 全透射。
        (
            FLOWER_RED,
            [(51, 15), (50, 15), (49, 15), (48, 15), (47, 15), (46, 15)],
        ),
    ];

    for (material, expected) in CASES {
        let mut w = World::new();
        // y=50 整层实心石台（16x16 全覆盖），仅 (4,4) 列换测材。
        w.box_fill(0, 15, 50, 50, 0, 15, STONE);
        w.voxels[vidx(4, 50, 4)] = material;
        w.rebuild_heightmap();
        let mut c = w.chunk();
        init(&mut c);

        for (y, expect) in expected {
            assert_eq!(
                sky(&c, 4, y as usize, 4),
                expect,
                "材料 id={material} 在 y={y} 的天光"
            );
        }

        if material == STONE {
            // 石头世界：板下洞窟无任何光源入口，处处全黑。
            assert_eq!(sky(&c, 8, 45, 8), 0);
            assert_eq!(sky(&c, 12, 40, 12), 0);
        }
        if material == FLOWER_RED {
            // 玻璃类透射柱向洞窟侧向渗光：横向每格 -1。
            assert_eq!(sky(&c, 5, 45, 4), 14);
            assert_eq!(sky(&c, 6, 45, 4), 13);
        }
    }
}

/// 黄金用例 4：发光/透光常数表与 26.1 对照。
/// 26.1 `BlockBehaviour.getLightDampening`（`BlockBehaviour.java:305-310`）
/// 默认规则：实心渲染整方块（solidRender）=15；否则
/// `propagatesSkylightDown`（默认=拾取形状非整立方且无流体，
/// `BlockBehaviour.java:395-397`）= 0（火把/栅栏/半砖/楼梯/花草/空气；
/// 玻璃另有 `TransparentBlock.java:34-37` 显式覆写）；
/// 其余非实心 =1（水：`LiquidBlock.java:114-116`；叶=显式覆盖 1，
/// `LeavesBlock.java:83-85`）。
/// 发光侧：当前 BLOCKS 子集（天然建材+水+花）在 26.1 全部不发光。
#[test]
fn golden_light_constants_match_26_1() {
    // ---- 透光（lightDampening → mcv_light::opacity 查表）----
    let opaque = [
        1u16, 2, 3, 4, 6, 8, 9, 10,
        11, // stone dirt grass sand log planks cobble bedrock snow_grass
    ];
    for id in opaque {
        assert_eq!(opacity(BlockId(id)), 15, "id={id} 实心整方块应 damp=15");
    }
    assert_eq!(opacity(BlockId(0)), 0, "空气 damp=0");
    assert_eq!(
        opacity(BlockId(WATER)),
        1,
        "水 damp=1（LiquidBlock 截断源柱）"
    );
    assert_eq!(
        opacity(BlockId(LEAVES)),
        1,
        "树叶 damp=1（LeavesBlock 覆盖值）"
    );
    // 花草：拾取形状可命中，但碰撞/渲染非实心 → propagatesSkylightDown
    // → damp=0（区别于玻璃：玻璃靠 TransparentBlock 显式覆写得 0，见下）。
    assert_eq!(opacity(BlockId(FLOWER_RED)), 0, "红花 damp=0（非整盒形状）");
    assert_eq!(opacity(BlockId(13)), 0, "黄花 damp=0");
    // 形状方块遮光（VERIFY-shapes C6）：非整方块渲染形状 → 0，全表
    // （mcv_core::OPACITY 规则④）与本引擎形状表同源成立。
    let by_name = |want: &str| {
        mcv_core::BLOCKS
            .iter()
            .position(|b| b.name == want)
            .unwrap() as u16
    };
    for name in [
        "torch",
        "oak_fence",
        "oak_slab",
        "oak_stairs",
        "short_grass",
    ] {
        assert_eq!(opacity(BlockId(by_name(name))), 0, "{name} damp=0");
    }
    // 玻璃：整盒形状但 TransparentBlock 覆写 propagatesSkylightDown=true
    // → damp=0（`TransparentBlock.java:34-37`，全透不衰减；
    // 早前 C6 草稿按"整盒形状 noOcclusion → 1"推断为 1，系误判）。
    assert_eq!(opacity(BlockId(by_name("glass"))), 0, "玻璃 damp=0");
    // 状态位掩蔽：带状态 nibble 的体素按基础方块查表（上半砖仍 0，
    // 有状态的石头仍 15），防止 OPACITY 直查原始 u16 越界误判 15。
    let slab = BlockId(by_name("oak_slab")).with_state(1);
    assert_eq!(opacity(slab), 0, "上半砖（带状态位）damp=0");
    let stone_st = BlockId(1).with_state(7);
    assert_eq!(opacity(stone_st), 15, "带状态位石头 damp=15");
    // 千块表接入后 id 99 已是注册方块（black_carpet，damp≠全挡），
    // "未注册保守全挡"只对越界 id 成立（C2：全表见 mcv_core::OPACITY）。
    assert_eq!(opacity(BlockId(60000)), 15, "越界 id 保守按全挡");

    // ---- 发光（light_emit）----
    // 26.1 参照：这些方块全部 lightLevel 0；将来注册发光方块时按
    // 参照值填：TORCH/WALL_TORCH=14，COPPER_TORCH=14，GLOWSTONE=15，
    // SEA_LANTERN/SHROOMLIGHT/LANTERN/JACK_O_LANTERN/FIRE/LAVA=15，
    // FURNACE(燃)=13（litBlockEmission），MAGMA_BLOCK=3，SOUL_TORCH=10，
    // END_ROD=14，GLOW_LICHEN=7，REDSTONE_ORE(亮)=9，CRYING_OBSIDIAN=10。
    const GOLDEN_EMIT: [(&str, u8); 14] = [
        ("air", 0),
        ("stone", 0),
        ("dirt", 0),
        ("grass", 0),
        ("sand", 0),
        ("water", 0),
        ("log", 0),
        ("leaves", 0),
        ("planks", 0),
        ("cobble", 0),
        ("bedrock", 0),
        ("snow_grass", 0),
        ("flower_red", 0),
        ("flower_yellow", 0),
    ];
    for (i, (name, emit)) in GOLDEN_EMIT.into_iter().enumerate() {
        let def = &mcv_core::BLOCKS[i];
        assert_eq!(def.name, name, "id={i} 方块顺序变了，需同步本对照表");
        assert_eq!(def.light_emit, emit, "{name} 发光值应为 {emit}（26.1）");
    }
}

/// C2：全表 opacity 抽查（`mcv_core::OPACITY`，三段规则
/// `solidRender?15:(propagatesSkylightDown?0:1)`，BlockBehaviour.java:305-310）。
/// 修复前仅 6 个 legacy id 有值，火把/玻璃/板/楼梯/格栅全部被误判 15。
#[test]
fn opacity_full_table_matches_26_1() {
    let id = |name: &str| -> u16 {
        mcv_core::BLOCKS
            .iter()
            .position(|b| b.name == name)
            .unwrap_or_else(|| panic!("方块 {name} 未注册")) as u16
    };
    // 实心整方块（solidRender）= 15。
    for n in ["stone", "deepslate", "bedrock", "glowstone"] {
        assert_eq!(opacity(BlockId(id(n))), 15, "{n} 应为 15");
    }
    // 非实心且 propagatesSkylightDown（火把/玻璃/板/梯/栅栏/格栅…）= 0：
    // 玻璃 Blocks.java:505-507 TransparentBlock（TransparentBlock.java:34-37），
    // 板/梯/火把走默认判据（非整方块形状，BlockBehaviour.java:395-397）。
    for n in [
        "glass",
        "white_stained_glass",
        "copper_grate",
        "glass_pane",
        "torch",
        "acacia_pressure_plate",
        "acacia_stairs",
        "acacia_fence",
        "activator_rail",
    ] {
        assert_eq!(opacity(BlockId(id(n))), 0, "{n} 应为 0");
    }
    // 非实心但截断源柱（damp=1）：水/岩浆（LiquidBlock.java:114-116）、
    // 叶（LeavesBlock.java:84-86）、整方块形状的透光块（冰族，默认规则）。
    for n in [
        "water",
        "lava",
        "leaves",
        "acacia_leaves",
        "flowering_azalea_leaves",
        "ice",
        "packed_ice",
        "blue_ice",
        "frosted_ice",
    ] {
        assert_eq!(opacity(BlockId(id(n))), 1, "{n} 应为 1");
    }
    // 遮光玻璃例外 15——源码为 TintedGlassBlock.java:25-27 `return 15`，
    // 不是审计草稿推测的 1。
    assert_eq!(opacity(BlockId(id("tinted_glass"))), 15, "遮光玻璃应 15");
    // 整方块形状 + noOcclusion 的 damp=1 例外（表内 model_kind=1，规则④
    // 会误给 0，gen_opacity 显式覆盖）：潜影盒 ShulkerBoxBlock.java:159-161、
    // 粘液/蜂蜜块 Blocks.java:2561/4913、紫颂植株 PipeBlock.java:59-61。
    for n in [
        "shulker_box",
        "white_shulker_box",
        "black_shulker_box",
        "slime_block",
        "honey_block",
        "chorus_plant",
    ] {
        assert_eq!(opacity(BlockId(id(n))), 1, "{n} 应为 1");
    }
    // 屏障/光源 = 0（整方块形状但覆写 propagatesSkylightDown=true：
    // BarrierBlock.java:40-42、LightBlock.java:71-73）。
    assert_eq!(opacity(BlockId(id("barrier"))), 0, "barrier 应 0");
    assert_eq!(opacity(BlockId(id("light"))), 0, "light 应 0");
    // 空气 0；越界 id 保守 15。
    assert_eq!(opacity(BlockId(0)), 0);
    assert_eq!(opacity(BlockId(60000)), 15);
    // firefly_bush 发光数据修正 3→2（Blocks.java:5840-5846 lightLevel→2；
    // 旧值 3 是 gen-blocks.py 烛例外误伤），形状按植被 damp=0。
    let fb = id("firefly_bush");
    assert_eq!(
        mcv_core::BLOCKS[fb as usize].light_emit,
        2,
        "firefly_bush 发光"
    );
    assert_eq!(opacity(BlockId(fb)), 0, "firefly_bush damp");
}

/// C3：海洋水柱源柱截断。26.1 `ChunkSkyLightSources.isEdgeOccluded`
/// （ChunkSkyLightSources.java:140-148）：bottomState 的 `dampening != 0`
/// 即截断源柱——水 damp=1（LiquidBlock.java:114-116）同样截断；
/// `SkyLightEngine.addSourcesAbove`（SkyLightEngine.java:106-131）只在截断点
/// 以上铺 15。修复前 init 按「不计水」的地形 heightmap 播种，整条水柱 15。
#[test]
fn ocean_water_column_falls_off_below_surface() {
    let mut w = World::new();
    // 海床石头到 y=39，整 chunk 水体 y40..=55，其上空气。
    w.box_fill(0, 15, 0, 39, 0, 15, STONE);
    w.box_fill(0, 15, 40, 55, 0, 15, WATER);
    w.rebuild_heightmap();
    // init 不再读 heightmap（播种判据=voxels/column_top，M4 统一判据）：
    // 故意抹成全 0，旧实现会把整柱（含水）种满 15，新实现结果不变。
    w.hm.fill(0);
    let mut c = w.chunk();
    init(&mut c);
    // 水面以上的空气是源柱：整段 15。
    for y in 56..256usize {
        assert_eq!(sky(&c, 4, y, 4), 15, "水面上方 y={y}");
    }
    // 水面格 = 源柱底：15 - max(1, 水 damp=1) = 14，往下每格恰好 -1。
    for d in 0..=15usize {
        let y = 55 - d;
        assert_eq!(
            sky(&c, 4, y, 4),
            14u8.saturating_sub(d as u8),
            "水柱 y={y} 应逐格递减"
        );
    }
    // 海床石面（实心 15）不透光。
    assert_eq!(sky(&c, 4, 39, 4), 0);
}

/// C4：removal 波回播自发光。26.1 `BlockLightEngine.propagateDecrease`
/// （BlockLightEngine.java:91-101）：清零邻居后读取该格自身 `toEmission`，
/// `toEmission < toLevel` 才让移除波继续携带旧亮度，`toEmission > 0` 则把
/// 该格按发射值重新压入 increase 队列。修复前弱光源被更强的相邻光源移除时
/// 清零且不再回填，永久熄灭。
#[test]
fn removal_wave_replays_weak_emitters() {
    let id = |name: &str| -> u16 {
        mcv_core::BLOCKS
            .iter()
            .position(|b| b.name == name)
            .unwrap_or_else(|| panic!("方块 {name} 未注册")) as u16
    };
    let (glowstone, torch, magma) = (id("glowstone"), id("torch"), id("magma_block"));
    assert_eq!(mcv_core::BLOCKS[glowstone as usize].light_emit, 15);
    assert_eq!(mcv_core::BLOCKS[torch as usize].light_emit, 14);
    assert_eq!(mcv_core::BLOCKS[magma as usize].light_emit, 3);

    // 密封石室走廊 y=34：萤石(4) - 火把(5) - 空气(6) - 岩浆岩(7)。
    let mut w = World::flat(40);
    w.box_fill(2, 13, 32, 36, 6, 10, AIR);
    w.rebuild_heightmap();
    w.voxels[vidx(4, 34, 8)] = glowstone;
    w.voxels[vidx(5, 34, 8)] = torch;
    w.voxels[vidx(7, 34, 8)] = magma;
    {
        let mut c = w.chunk();
        init(&mut c);
        assert_eq!(blk(&c, 4, 34, 8), 15, "萤石格自身发光");
        assert_eq!(blk(&c, 5, 34, 8), 14, "火把格：发射 14（与萤石投影同值）");
        assert_eq!(blk(&c, 6, 34, 8), 13, "走廊空气格");
        assert_eq!(blk(&c, 7, 34, 8), 3, "岩浆岩：实心格只保留自发光");
    }

    // 挖掉萤石：removal 波会依次触碰火把格(14<15)、空气格、岩浆岩格(3<…)。
    let (_mask, _seeds) = edit(&mut w, 4, 34, 8, glowstone, AIR);
    {
        let c = w.chunk();
        // 火把（emit=14==被清零的存储值，vanilla 边界 `toEmission < toLevel`
        // 不成立 → 不续波但必须回播）——修复前这里是 0（永久熄灭）。
        assert_eq!(blk(&c, 5, 34, 8), 14, "火把光必须存活");
        assert_eq!(blk(&c, 6, 34, 8), 13, "走廊由火把重新照亮");
        assert_eq!(blk(&c, 7, 34, 8), 3, "岩浆岩自发光回播");
        assert_eq!(blk(&c, 4, 34, 8), 13, "萤石位由火把照到 13");
    }
}

// ---------------------------------------------------------------------------
// MINOR-1：发光表默认态——mcv_core::BLOCKS 是唯一真相源。
// ---------------------------------------------------------------------------

/// 按名字查注册 id（BLOCKS 表 1171 块全注册）。
fn bid(name: &str) -> u16 {
    mcv_core::BLOCKS
        .iter()
        .position(|b| b.name == name)
        .unwrap_or_else(|| panic!("方块 {name} 未注册")) as u16
}

/// 发光值黄金表：火把族 + 主力光源逐类对照 26.1 `Blocks.java`（发光不再
/// 存在任何「手工登记/静默 0」旁路——`mcv_light::light_emit` 是
/// `mcv_core::BLOCKS` 的 const 镜像，新增方块在源头登记即自动生效；
/// 本用例锁死火把类回归基线）。
#[test]
fn golden_emit_table_torch_family_matches_26_1() {
    // (方块名, 26.1 发光值, Blocks.java 依据 file:line)
    const GOLDEN: [(&str, u8, &str); 17] = [
        ("torch", 14, "Blocks.java:966 lightLevel(statex -> 14)"),
        (
            "wall_torch",
            14,
            "Blocks.java:971 wallVariant(TORCH).lightLevel(14)",
        ),
        ("copper_torch", 14, "Blocks.java:1726-1730 lightLevel(14)"),
        (
            "copper_wall_torch",
            14,
            "Blocks.java:1731-1734 lightLevel(14)",
        ),
        ("soul_torch", 10, "Blocks.java:1716-1720 lightLevel(10)"),
        (
            "soul_wall_torch",
            10,
            "Blocks.java:1721-1724 lightLevel(10)",
        ),
        (
            "redstone_torch",
            7,
            "Blocks.java:1589-1593 litBlockEmission(7) 按 LIT 态条件发光\
             （Blocks.java:5853-5855），但默认放置态 LIT=true\
             （RedstoneTorchBlock.java:40 registerDefaultState）→ 7",
        ),
        (
            "redstone_wall_torch",
            7,
            "Blocks.java:1594-1597 wallVariant 复制同属性，默认点亮同上 → 7",
        ),
        ("glowstone", 15, "Blocks.java:1736-1744 lightLevel(15)"),
        ("sea_lantern", 15, "Blocks.java:2619-2627 lightLevel(15)"),
        ("shroomlight", 15, "Blocks.java:4728-4729 lightLevel(15)"),
        ("jack_o_lantern", 15, "Blocks.java:1756-1764 lightLevel(15)"),
        ("end_rod", 14, "Blocks.java:3562-3565 lightLevel(14)"),
        ("lantern", 15, "Blocks.java:4578-4587 lightLevel(15)"),
        ("soul_lantern", 10, "Blocks.java:4590-4599 lightLevel(10)"),
        ("fire", 15, "Blocks.java:973-981 lightLevel(15)"),
        ("soul_fire", 10, "Blocks.java:985-992 lightLevel(10)"),
    ];
    for (name, emit, src) in GOLDEN {
        let id = bid(name) as usize;
        assert_eq!(
            mcv_core::BLOCKS[id].light_emit,
            emit,
            "{name} 发光值应为 {emit}（{src}）"
        );
    }
}

/// MINOR-1 端到端：id≥14 的发光方块经 `update_block` 放置即从 BLOCKS
/// 真相源播种（生产路径，非手工灌 light），挖掘后逐格回撤到 0。
#[test]
fn torch_placement_seeds_block_light_via_update_block() {
    let torch = bid("torch");
    assert!(torch >= 14, "火把是 id≥14 的新注册方块（gen 字典序）");
    let mut w = World::flat(40);
    // 密封石室：内部空气 x 3..=12, y 32..=36, z 6..=10。
    w.box_fill(3, 12, 32, 36, 6, 10, AIR);
    w.rebuild_heightmap();
    {
        let mut c = w.chunk();
        init(&mut c);
    }

    // 放置火把：update_block 读 BLOCKS 播种并 BFS。
    let (mask, _seeds) = edit(&mut w, 5, 34, 8, AIR, torch);
    assert_eq!(mask, 0, "光照被石室吞掉，不触边");
    {
        let c = w.chunk();
        assert_eq!(blk(&c, 5, 34, 8), 14, "火把格自发光（真相源播种）");
        // 水平 +x 逐格 -1，墙（x=13 起 damp 15）吞光。
        for d in 0..=7usize {
            assert_eq!(blk(&c, 5 + d, 34, 8), 14 - d as u8, "+x 第 {d} 格");
        }
        assert_eq!(blk(&c, 13, 34, 8), 0, "东墙");
        // -x 与垂直同样逐格 -1。
        assert_eq!(blk(&c, 4, 34, 8), 13);
        assert_eq!(blk(&c, 3, 34, 8), 12);
        assert_eq!(blk(&c, 2, 34, 8), 0, "西墙");
        for (y, expect) in [(33usize, 13u8), (32, 12), (35, 13), (36, 12)] {
            assert_eq!(blk(&c, 5, y, 8), expect, "垂直 y={y}");
        }
        assert_eq!(blk(&c, 5, 31, 8), 0, "地板");
        assert_eq!(blk(&c, 5, 37, 8), 0, "天花板");
    }

    // 挖掉火把：撤销波清场，逐格回 0。
    let (_mask, _seeds) = edit(&mut w, 5, 34, 8, torch, AIR);
    {
        let c = w.chunk();
        for d in 0..=7usize {
            assert_eq!(blk(&c, 5 + d, 34, 8), 0, "回撤 +x 第 {d} 格");
        }
        assert_eq!(blk(&c, 5, 33, 8), 0);
        assert_eq!(blk(&c, 5, 35, 8), 0);
    }
}

// ---------------------------------------------------------------------------
// MINOR-2：跨区块边界撤销判据。PairWorld 复刻 game.rs::sync_light_edges
// 的派发循环（抽边 → REMOVE 后 ADD → 脏位回队级联，512 步预算）。
// ---------------------------------------------------------------------------

const CHUNK_A: u8 = 0;
const CHUNK_B: u8 = 1;

/// 两区块世界：全局 x 0..=15 属 A，16..=31 属 B（B 局部 x = gx-16）。
struct PairWorld {
    chunks: [World; 2],
}

impl PairWorld {
    fn flat(top: usize) -> Self {
        Self {
            chunks: [World::flat(top), World::flat(top)],
        }
    }

    fn set(&mut self, gx: usize, y: usize, z: usize, id: u16) {
        if gx < 16 {
            self.chunks[0].voxels[vidx(gx, y, z)] = id;
        } else {
            self.chunks[1].voxels[vidx(gx - 16, y, z)] = id;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn box_fill(
        &mut self,
        gx0: usize,
        gx1: usize,
        y0: usize,
        y1: usize,
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

    fn rebuild_heightmaps(&mut self) {
        for w in &mut self.chunks {
            w.rebuild_heightmap();
        }
    }

    /// 经 `update_block` 的编辑（先写体素，同生产路径）。
    fn edit(
        &mut self,
        which: u8,
        x: usize,
        y: usize,
        z: usize,
        old: u16,
        new: u16,
    ) -> (u8, Vec<BorderSeed>) {
        let w = &mut self.chunks[which as usize];
        w.voxels[vidx(x, y, z)] = new;
        let mut seeds = Vec::new();
        let mask = {
            let mut c = w.chunk();
            update_block(&mut c, x as u32, y as u32, z as u32, old, new, &mut seeds)
        };
        (mask, seeds)
    }

    /// `game.rs::sync_light_edges` 同构：弹 (块, 边)，抽源边快照，对邻块
    /// 先 REMOVE 后 ADD，脏位回队级联。只派发 A+X / B-X 这对真实相邻边
    /// （±Z 邻块不在本用例世界内，同 game.rs 对缺失邻块的跳过）。
    fn sync_edges(&mut self, queue: Vec<(u8, u8)>) {
        let mut queue = queue;
        let mut steps = 0usize;
        while let Some((which, side)) = queue.pop() {
            let connects_pair = match side {
                SIDE_PLUS_X => which == CHUNK_A,
                SIDE_MINUS_X => which == CHUNK_B,
                // ±Z 的邻块不在本两块世界内（game.rs 会同步真实 ±Z 邻块；
                // 此处同其对缺失邻块的跳过路径）。
                SIDE_PLUS_Z | SIDE_MINUS_Z => false,
                _ => false,
            };
            if !connects_pair {
                continue;
            }
            steps += 1;
            assert!(steps <= 512, "边界同步预算耗尽（级联不收敛）");
            let other = if which == CHUNK_A { CHUNK_B } else { CHUNK_A };
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

    /// 双块 init + 初始成对边同步（同 game.rs border pairing）。
    fn init_and_pair(&mut self) {
        {
            let mut c = self.chunks[0].chunk();
            init(&mut c);
        }
        {
            let mut c = self.chunks[1].chunk();
            init(&mut c);
        }
        self.sync_edges(vec![(CHUNK_A, SIDE_PLUS_X), (CHUNK_B, SIDE_MINUS_X)]);
    }
}

fn blk_p(p: &PairWorld, gx: usize, y: usize, z: usize) -> u8 {
    let (which, x) = if gx < 16 {
        (0usize, gx)
    } else {
        (1usize, gx - 16)
    };
    p.chunks[which].light[vidx(x, y, z)] & 0xF
}

fn sky_p(p: &PairWorld, gx: usize, y: usize, z: usize) -> u8 {
    let (which, x) = if gx < 16 {
        (0usize, gx)
    } else {
        (1usize, gx - 16)
    };
    p.chunks[which].light[vidx(x, y, z)] >> 4
}

/// 用例 1：火把恰在边界格（A 的 +X 边），挖掉后邻区整列回撤。
/// 参照 26.1 `BlockLightEngine.propagateDecrease`（BlockLightEngine.java:
/// 91-101）：撤销只减不增，A 边值归零后 B 侧不再被证成的格子逐格清零。
#[test]
fn border_torch_dug_retracts_neighbour_per_nibble() {
    let torch = bid("torch");
    let mut p = PairWorld::flat(40);
    p.set(15, 45, 8, torch); // A 的 +X 边界格
    p.rebuild_heightmaps();
    p.init_and_pair();

    // 点亮态：A 边界格自身 14，B 侧 13,12,11,… 逐格 -1；天光满照度。
    assert_eq!(blk_p(&p, 15, 45, 8), 14, "火把格");
    for d in 0..=7usize {
        assert_eq!(blk_p(&p, 16 + d, 45, 8), 13 - d as u8, "B 侧 +{d}");
        assert_eq!(sky_p(&p, 16 + d, 45, 8), 15, "天光 +{d}");
    }

    // 挖掉火把：A 本地撤销 + 边推送 → B 侧整列回撤为 0。
    let (mask, _seeds) = p.edit(CHUNK_A, 15, 45, 8, torch, AIR);
    assert_eq!(mask & (1 << SIDE_PLUS_X), 1 << SIDE_PLUS_X, "+X 脏位");
    p.sync_edges(vec![(CHUNK_A, SIDE_PLUS_X)]);
    assert_eq!(blk_p(&p, 15, 45, 8), 0, "A 边界格归零");
    for d in 0..=7usize {
        assert_eq!(blk_p(&p, 16 + d, 45, 8), 0, "B 侧回撤 +{d}");
        assert_eq!(sky_p(&p, 16 + d, 45, 8), 15, "天光不受方块光撤销影响 +{d}");
    }
}

/// 用例 2（修复「错压/漏照」）：B 侧自有萤石时挖掉边界火把——火把份额
/// 撤销、萤石份额按邻区现值存活，且编辑块（A）侧必须由 B 的现值回喂：
/// 26.1 跨 section 撤销波对 `toLevel >= oldFromLevel` 的幸存格按其
/// stored 现值重播种（BlockLightEngine.java:103-105）；本协议等价实现
/// 是 REMOVE 回报边变化 → game.rs 级联反向推送（拉）。修复前 A 侧
/// 永久漏照（0 而非 11）。
#[test]
fn border_torch_dug_keeps_neighbour_justified_light() {
    let (torch, glowstone) = (bid("torch"), bid("glowstone"));
    let mut p = PairWorld::flat(40);
    p.set(15, 45, 8, torch); // A 边界格
    p.set(19, 45, 8, glowstone); // B 局部 x=3
    p.rebuild_heightmaps();
    p.init_and_pair();

    // 点亮态：B(16)=13（火把 13 与萤石 12 取大）、B(17)=13、B(18)=14、
    // B(19)=15；A(15)=14。
    assert_eq!(blk_p(&p, 15, 45, 8), 14);
    assert_eq!(blk_p(&p, 16, 45, 8), 13);
    assert_eq!(blk_p(&p, 17, 45, 8), 13);
    assert_eq!(blk_p(&p, 18, 45, 8), 14);
    assert_eq!(blk_p(&p, 19, 45, 8), 15);

    let _ = p.edit(CHUNK_A, 15, 45, 8, torch, AIR);
    p.sync_edges(vec![(CHUNK_A, SIDE_PLUS_X)]);

    // 萤石份额存活（撤销波触到 B(16) 后由 ≥ 波前的萤石梯度回播）。
    assert_eq!(blk_p(&p, 19, 45, 8), 15, "萤石自发光存活");
    assert_eq!(blk_p(&p, 18, 45, 8), 14);
    assert_eq!(blk_p(&p, 17, 45, 8), 13);
    assert_eq!(blk_p(&p, 16, 45, 8), 12, "B 边界格回落到萤石正当值");
    // 编辑块侧回喂：A(15) = B(16) 现值 - 1，向内逐格 -1（修复前恒 0）。
    assert_eq!(blk_p(&p, 15, 45, 8), 11, "A 边界格由邻区现值回喂");
    assert_eq!(blk_p(&p, 14, 45, 8), 10);
    assert_eq!(blk_p(&p, 13, 45, 8), 9);
    // 天光满照度不受影响。
    assert_eq!(sky_p(&p, 15, 45, 8), 15);
    assert_eq!(sky_p(&p, 16, 45, 8), 15);
}

/// 用例 3（修复「错压邻区」）：发光体恰在 B 的边界格、A 侧同列是暗格
/// （n_blk=0）——修复前 REMOVE 推送把贴边萤石清零且从不回播（apply_edge
/// 的边界清零不走 removal_channel 的自发光回播路径），邻区光源被永久
/// 压灭。26.1 `checkNode`：stored > emission → 清零 + decrease(stored)
/// + increase(emission)（BlockLightEngine.java:30-41）。
#[test]
fn border_emitter_survives_neighbour_dark_edge() {
    let glowstone = bid("glowstone");
    let mut p = PairWorld::flat(40);
    p.set(16, 45, 8, glowstone); // B 的 -X 边界格
    p.rebuild_heightmaps();
    p.init_and_pair();

    assert_eq!(blk_p(&p, 16, 45, 8), 15, "贴边萤石不被邻区暗边压灭");
    assert_eq!(blk_p(&p, 17, 45, 8), 14);
    assert_eq!(blk_p(&p, 18, 45, 8), 13);

    // 再吃一次全暗 REMOVE 推送仍不熄灭（A 侧没有任何方块光来源）。
    p.sync_edges(vec![(CHUNK_A, SIDE_PLUS_X)]);
    assert_eq!(blk_p(&p, 16, 45, 8), 15, "重复 REMOVE 后萤石仍在");
    assert_eq!(blk_p(&p, 17, 45, 8), 14);
    // 天光 nibble：萤石是不透明整方块（OPACITY 规则① damp=15），所在格
    // 天光恒 0（源柱在其上方截断）；相邻空气格保持满照度。
    assert_eq!(sky_p(&p, 16, 45, 8), 0, "不透明发光体格内无天光");
    assert_eq!(sky_p(&p, 17, 45, 8), 15, "相邻空气格天光满照度");
}

/// 用例 4：玻璃隧道跨界、遮光体（收口石壁）在亮侧（B）挖开——B 的边界
/// 字节 0→9 变化，正常推送即可把光喂进暗侧 A。逐格 9,8,7,…（山体内
/// 天光 nibble 恒 0，整字节 == 方块光 nibble）。
#[test]
fn glass_tunnel_blocker_dug_on_lit_side_feeds_dark_side() {
    let torch = bid("torch");
    let mut p = PairWorld::flat(49);
    // 石山 y50..=60 全覆盖；隧道 y=50 z=8：B 半段 gx 16..=23（火把 gx=21），
    // A 半段 gx 9..=15（无光源全暗），收口石壁恰在 B 的边界格 gx=16。
    p.box_fill(0, 31, 50, 60, 0, 15, STONE);
    p.box_fill(16, 23, 50, 50, 8, 8, AIR);
    p.box_fill(9, 15, 50, 50, 8, 8, AIR);
    p.set(16, 50, 8, STONE);
    p.set(21, 50, 8, torch);
    p.rebuild_heightmaps();
    p.init_and_pair();

    assert_eq!(blk_p(&p, 21, 50, 8), 14, "火把格");
    assert_eq!(blk_p(&p, 20, 50, 8), 13);
    assert_eq!(blk_p(&p, 17, 50, 8), 10);
    assert_eq!(blk_p(&p, 16, 50, 8), 0, "收口石壁不透光");
    assert_eq!(blk_p(&p, 15, 50, 8), 0, "A 侧隧道全暗");

    // 在亮侧挖开收口：B 边界字节 0→9 → 推送 → A 吸收。
    let (mask, _seeds) = p.edit(CHUNK_B, 0, 50, 8, STONE, AIR);
    assert_eq!(mask & (1 << SIDE_MINUS_X), 1 << SIDE_MINUS_X, "-X 脏位");
    p.sync_edges(vec![(CHUNK_B, SIDE_MINUS_X)]);

    assert_eq!(blk_p(&p, 16, 50, 8), 9, "B 边界格点亮");
    for d in 0..=6usize {
        let (gx, expect) = (15 - d, 8 - d as u8);
        assert_eq!(blk_p(&p, gx, 50, 8), expect, "A 隧道第 {d} 格");
        // 逐 nibble：山体内天光 0，整字节 == 方块光。
        let (which, x) = if gx < 16 { (0usize, gx) } else { (1, gx - 16) };
        assert_eq!(
            p.chunks[which].light[vidx(x, 50, 8)],
            expect,
            "整字节 gx={gx}"
        );
    }
}

/// 用例 5：玻璃隧道跨界、遮光体在暗侧（A）的边界格挖开——编辑块自身
/// 光照字节 0→0 不变，纯字节 diff 漏报；`update_block` 现按「边界格遮光
/// 等级下降」补报脏位（本用例断言该位）。但补报驱动的是「推」方向
/// （A→B），B 的光要回流进 A 还需调用方对该边补一次反向同步
/// （game.rs::sync_light_edges 接线需求，见任务报告）——这里按接线后
/// 的语义补推反向边，断言拉取后的逐格预期。
#[test]
fn glass_tunnel_blocker_dug_on_dark_side_pulls_via_reverse_sync() {
    let torch = bid("torch");
    let mut p = PairWorld::flat(49);
    p.box_fill(0, 31, 50, 60, 0, 15, STONE);
    p.box_fill(16, 23, 50, 50, 8, 8, AIR);
    p.box_fill(9, 15, 50, 50, 8, 8, AIR);
    p.set(15, 50, 8, STONE); // 遮光体恰在 A 的 +X 边界格
    p.set(21, 50, 8, torch);
    p.rebuild_heightmaps();
    p.init_and_pair();

    assert_eq!(blk_p(&p, 16, 50, 8), 9, "B 边界格亮");
    assert_eq!(blk_p(&p, 15, 50, 8), 0, "A 侧被收口挡住");

    let (mask, _seeds) = p.edit(CHUNK_A, 15, 50, 8, STONE, AIR);
    assert_eq!(blk_p(&p, 15, 50, 8), 0, "本地无源仍暗（字节 0→0）");
    assert_eq!(
        mask & (1 << SIDE_PLUS_X),
        1 << SIDE_PLUS_X,
        "遮光下降补报 +X 脏位"
    );

    // 接线后的完整语义：正向（A→B，无害空转）+ 反向（B→A，拉取）。
    p.sync_edges(vec![(CHUNK_A, SIDE_PLUS_X), (CHUNK_B, SIDE_MINUS_X)]);
    assert_eq!(blk_p(&p, 16, 50, 8), 9);
    for d in 0..=6usize {
        let (gx, expect) = (15 - d, 8 - d as u8);
        assert_eq!(blk_p(&p, gx, 50, 8), expect, "拉取后 A 隧道第 {d} 格");
        let (which, x) = if gx < 16 { (0usize, gx) } else { (1, gx - 16) };
        assert_eq!(
            p.chunks[which].light[vidx(x, 50, 8)],
            expect,
            "整字节 gx={gx}"
        );
    }
}
