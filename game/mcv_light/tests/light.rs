//! Synthetic-voxel tests for the BFS lighting engine.
//!
//! 光照规则参照反编译 Minecraft 26.1（机制与常数提取见仓库外笔记
//! `/root/mc-ref/NOTES-light.md`）：
//! - 传播代价 = `max(1, lightDampening)`，六个方向一视同仁；
//! - 天光"垂直 15 不衰减"仅存在于源柱（全透空气柱），水/叶
//!   （damp=1）截断源柱并逐格 -1；
//! - 方块光火把参照值 14，每格 -1。

use mcv_core::{BlockId, CHUNK_VOL, vidx};
use mcv_light::{
    BorderSeed, LightChunk, apply_edge, extract_edge, init, opacity, propagate, update_block,
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

/// 26.1 `Blocks.TORCH.lightLevel(14)`（火把尚未注册进 BLOCKS，
/// 按参照发光值手工播种方块光通道）。
const TORCH_EMISSION: u8 = 14;

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
    {
        let mut c = w.chunk();
        init(&mut c);
        assert_eq!(sky(&c, 8, 34, 8), 0, "密封室内无天光");
        assert_eq!(blk(&c, 8, 34, 8), 0, "密封室内无方块光");
    }

    // 火把（发光 14）放在 (1,34,8)。
    w.light[vidx(1, 34, 8)] = TORCH_EMISSION;
    let mask = {
        let mut c = w.chunk();
        propagate(&mut c)
    };
    // 所有边界格距火把曼哈顿距离 ≥14，方块光够不到任何边界。
    assert_eq!(mask, 0);

    // 水平序列（沿 +x）：14,13,12,…,1，第 15 格衰减到 0。
    let sequence: Vec<u8> = (0..=14u8)
        .map(|d| TORCH_EMISSION.saturating_sub(d))
        .collect();
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

/// 黄金用例 3：玻璃透光 vs 石头挡光（材料柱对照表）。
/// 玻璃尚未注册进 BLOCKS；26.1 中玻璃 `noOcclusion →
/// propagatesSkylightDown → damp 0`，与花（CrossCollisionBlock）
/// 同类，此处用花作为玻璃类的透射替身。半透明水/叶 damp=1：
/// 截断源柱但每格只 -1；实心石头 damp=15：完全挡光。
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
/// `BlockBehaviour.java:395-397`）= 0（火把/栅栏/半砖/楼梯/花草/空气）；
/// 其余非实心 =1（水/玻璃：整盒形状 noOcclusion；叶=显式覆盖 1，
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
    // 花草：拾取形状整盒可命中，但碰撞/渲染非实心 → propagatesSkylightDown
    // → damp=0（区别于玻璃：玻璃拾取形状整盒 → damp=1，见下）。
    assert_eq!(opacity(BlockId(FLOWER_RED)), 0, "红花 damp=0（非整盒形状）");
    assert_eq!(opacity(BlockId(13)), 0, "黄花 damp=0");
    // 形状方块遮光（VERIFY-shapes C6）：非实心 + 形状非整立方 → 0。
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
    // 玻璃：整盒形状 noOcclusion → solidRender=false、
    // propagatesSkylightDown=false → damp=1（`BlockBehaviour.java:305-310,
    // 395-397`；玻璃柱逐格衰减为 26.1 实况，26.1 起无"玻璃全透"特例）。
    assert_eq!(opacity(BlockId(by_name("glass"))), 1, "玻璃 damp=1");
    // 未注册 id（≥BLOCKS.len()）保守按全挡。
    assert_eq!(
        opacity(BlockId(mcv_core::BLOCKS.len() as u16)),
        15,
        "未注册 id 保守按全挡"
    );

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
