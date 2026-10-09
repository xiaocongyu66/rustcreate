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
/// 26.1 `BlockBehaviour.getLightDampening` 默认规则：
/// 实心渲染整方块 = 15；非实心且 `propagatesSkylightDown`（玻璃、
/// 花、空气）= 0；其余非实心（水、叶、雪层等）= 1。
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
    assert_eq!(opacity(BlockId(FLOWER_RED)), 0, "红花 damp=0（玻璃同类）");
    assert_eq!(opacity(BlockId(13)), 0, "黄花 damp=0");
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
    // 粘液/蜂蜜块 Blocks.java:2562/4913、紫颂植株 PipeBlock.java:59-61。
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
