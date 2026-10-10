//! 全表形状不变量：BLOCKS 每个 `solid=true` 条目必须能产出 ≥1 个非空
//! （非退化）碰撞 AABB；反之 Cross/Torch 形状（原版 noCollision 花草/
//! 火把族，碰撞恒空）必须 `solid=false`。
//!
//! 背景（沙穿透审计）：引擎的玩家碰撞谓词是
//! `blockshapes::push_boxes`（Cube 分支 = `def().solid → 整格盒`），
//! 它对位原版 26.1 的 `hasCollision ? getShape : Shapes.empty()`
//! （BlockBehaviour.java:333-334）。生成器曾把 `forceSolidOn`（只抬
//! isSolid，BlockBehaviour.java:482-487，不产生碰撞）并入 solid，
//! 造出「solid=true 但形状给不出碰撞盒」的自相矛盾条目（bamboo_sapling）
//! 与「原版可穿行、本引擎凭空全盒碰撞」的 115 块（招牌/压力板/旗帜/
//! 凋珊瑚/蛛网…）。本测试是这类表-形状脱节的常驻回归锁。

use std::collections::HashMap;

use mcv_core::{BlockId, BlockPos, ChunkPos, Shape};
use mcv_game::VoxelAccess;
use mcv_game::blockshapes::{EMPTY_AABB, MAX_SHAPE_BOXES, RayTarget, pick_boxes, push_boxes};

/// 全空气世界（栅栏臂等邻格查询都取不到连接对象）。
struct EmptyWorld;

impl VoxelAccess for EmptyWorld {
    fn block(&self, _p: BlockPos) -> BlockId {
        mcv_core::AIR
    }
    fn light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, _c: ChunkPos) -> bool {
        true
    }
}

/// HashMap 世界（定向条目用）。
struct Mem {
    m: HashMap<(i32, i32, i32), u16>,
}

impl VoxelAccess for Mem {
    fn block(&self, p: BlockPos) -> BlockId {
        BlockId(*self.m.get(&(p.x, p.y, p.z)).unwrap_or(&0))
    }
    fn light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn chunk_loaded(&self, _c: ChunkPos) -> bool {
        true
    }
}

fn by_name(name: &str) -> Option<u16> {
    (0..mcv_core::BLOCKS.len() as u16).find(|i| mcv_core::BLOCKS[*i as usize].name == name)
}

/// 非退化：三轴 max 严格大于 min。
fn degenerate(b: &mcv_game::physics::Aabb) -> bool {
    !(b.max.x > b.min.x && b.max.y > b.min.y && b.max.z > b.min.z)
}

#[test]
fn every_solid_block_yields_a_nonempty_collision_box() {
    let w = EmptyWorld;
    let p = BlockPos::new(0, 0, 0);
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    let mut offenders: Vec<String> = Vec::new();
    for (i, def) in mcv_core::BLOCKS.iter().enumerate() {
        let id = BlockId(i as u16);
        let n = push_boxes(id, &w, p, RayTarget::Collide, &mut boxes);
        let shapes = &boxes[..n];
        if def.solid && (shapes.is_empty() || shapes.iter().any(degenerate)) {
            offenders.push(format!(
                "id {i} {} shape={:?} solid=true 碰撞盒数 {n}（含退化盒：{}）",
                def.name,
                Shape::from_u8(def.shape),
                shapes.iter().filter(|b| degenerate(b)).count()
            ));
        }
        // 反向：Cross/Torch 无碰撞形状 → solid 必须 false（否则是
        // forceSolidOn 误并入碰撞谓词的复发）。
        if matches!(Shape::from_u8(def.shape), Shape::Cross | Shape::Torch) && def.solid {
            offenders.push(format!(
                "id {i} {} shape={:?} 无碰撞形状却 solid=true",
                def.name,
                Shape::from_u8(def.shape)
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "碰撞形状-表脱节（{} 条）：\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

#[test]
fn liquid_and_air_have_no_collision() {
    let w = EmptyWorld;
    let p = BlockPos::new(0, 0, 0);
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    for (i, def) in mcv_core::BLOCKS.iter().enumerate() {
        let is_air_family = def.name == "air" || def.name.ends_with("_air");
        if !(def.liquid || is_air_family) {
            continue;
        }
        let n = push_boxes(BlockId(i as u16), &w, p, RayTarget::Collide, &mut boxes);
        assert_eq!(n, 0, "{} 液体/空气占位不应有碰撞盒", def.name);
    }
}

#[test]
fn sand_regression_full_cube() {
    // 真机症状锚点：海滩沙必须给出整格非退化碰撞盒。
    let sand = by_name("sand").expect("sand 在表");
    let w = Mem {
        m: [((0, 0, 0), sand)].into_iter().collect(),
    };
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    let n = collision_at(&w, BlockPos::new(0, 0, 0), &mut boxes);
    assert_eq!(n, 1, "沙应有 1 个碰撞盒");
    assert!(!degenerate(&boxes[0]), "沙碰撞盒不得退化");
    assert_eq!(boxes[0].min.x, 0.0);
    assert_eq!(boxes[0].max.y, 1.0);
}

#[test]
fn force_solid_blocks_have_no_phantom_collision() {
    // 原版 noCollision+forceSolidOn 族（招牌/压力板/旗帜/凋珊瑚/蛛网/
    // 竹笋/萤火灌木/幽匿脉络）：isSolid≠有碰撞（BlockBehaviour.java:
    // 482-487 vs 333-334），本引擎必须可穿行。
    for name in [
        "oak_sign",
        "oak_wall_sign",
        "cherry_hanging_sign",
        "stone_pressure_plate",
        "white_banner",
        "dead_tube_coral",
        "cobweb",
        "bamboo_sapling",
        "sculk_vein",
        "firefly_bush",
    ] {
        let id = by_name(name).unwrap_or_else(|| panic!("{name} 不在表"));
        assert!(
            !mcv_core::BLOCKS[id as usize].solid,
            "{name} 原版 noCollision → solid(hasCollision) 必须 false"
        );
        let w = Mem {
            m: [((0, 0, 0), id)].into_iter().collect(),
        };
        let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
        let n = collision_at(&w, BlockPos::new(0, 0, 0), &mut boxes);
        assert_eq!(n, 0, "{name} 不应产出碰撞盒");
    }
}

#[test]
fn carpet_is_one_sixteenth_thin_box() {
    // CarpetBlock.java:17 column(16,0,1)：碰撞=拾取=全宽 y 0..1/16。
    let id = by_name("red_carpet").expect("red_carpet 在表");
    assert_eq!(
        Shape::from_u8(mcv_core::BLOCKS[id as usize].shape),
        Shape::Carpet
    );
    assert!(mcv_core::BLOCKS[id as usize].solid, "地毯原版有薄碰撞盒");
    let w = Mem {
        m: [((0, 0, 0), id)].into_iter().collect(),
    };
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    for mode in [RayTarget::Collide, RayTarget::Pick] {
        let n = push_boxes(BlockId(id), &w, BlockPos::new(0, 0, 0), mode, &mut boxes);
        assert_eq!(n, 1, "{mode:?} 地毯应 1 盒");
        assert_eq!(boxes[0].min.y, 0.0);
        assert_eq!(boxes[0].max.y, 0.0625);
        assert_eq!(boxes[0].max.x, 1.0);
    }
}

#[test]
fn trapdoor_closed_thin_plate_halves() {
    // TrapDoorBlock.java:48 boxZ(16,13,16)：上态 y 13..16px；下态 y 0..3px。
    let id = by_name("oak_trapdoor").expect("oak_trapdoor 在表");
    assert_eq!(
        Shape::from_u8(mcv_core::BLOCKS[id as usize].shape),
        Shape::Trapdoor
    );
    let w = Mem {
        m: [((0, 0, 0), id)].into_iter().collect(),
    };
    let p = BlockPos::new(0, 0, 0);
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];
    let n = push_boxes(
        BlockId(id).with_state(0),
        &w,
        p,
        RayTarget::Collide,
        &mut boxes,
    );
    assert_eq!(n, 1);
    assert_eq!(boxes[0].min.y, 0.0);
    assert_eq!(boxes[0].max.y, 0.1875);
    // bit2=1 上态（与楼梯 top 位同位约定）。
    let n = push_boxes(
        BlockId(id).with_state(4),
        &w,
        p,
        RayTarget::Pick,
        &mut boxes,
    );
    assert_eq!(n, 1);
    assert_eq!(boxes[0].min.y, 0.8125);
    assert_eq!(boxes[0].max.y, 1.0);
}

#[test]
fn pane_post_and_connection_arms() {
    // IronBarsBlock super(2,16,2,16,16)：柱 7..9px 全高；臂按邻格
    // （同板族 ∥ 不透明整立方近似 sturdy）从格边伸到中心。
    let pane = by_name("glass_pane").expect("glass_pane 在表");
    let stone = by_name("stone").expect("stone 在表");
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];

    // 孤立：仅柱。
    let mut m = std::collections::HashMap::new();
    m.insert((0, 0, 0), pane);
    let solo = Mem { m };
    let p = BlockPos::new(0, 0, 0);
    let n = collision_at(&solo, p, &mut boxes);
    assert_eq!(n, 1, "孤立板仅柱");
    assert_eq!(boxes[0].min.x, 0.4375);
    assert_eq!(boxes[0].max.x, 0.5625);
    assert_eq!(boxes[0].max.y, 1.0, "板无栅栏式 1.5 抬高");

    // +X 接另一板、-X 接石头 → 柱 + 两臂；拾取=碰撞。
    let mut m = std::collections::HashMap::new();
    m.insert((0, 0, 0), pane);
    m.insert((1, 0, 0), pane);
    m.insert((-1, 0, 0), stone);
    let w = Mem { m };
    for mode in [RayTarget::Collide, RayTarget::Pick] {
        let n = push_boxes(BlockId(pane), &w, p, mode, &mut boxes);
        assert_eq!(n, 3, "{mode:?} 柱+双臂");
        assert!(boxes[1..].iter().any(|b| b.max.x == 1.0 && b.min.x == 0.5));
        assert!(boxes[1..].iter().any(|b| b.min.x == 0.0 && b.max.x == 0.5));
    }
}

#[test]
fn wall_post_low_arms_and_collision_heights() {
    // WallBlock.java:66-74：拾取 post 高 1.0 + low 臂高 0.875（14px）；
    // 碰撞柱/臂均 1.5（24px 防抬）；臂断面 6px、自格边伸入 11px。
    let wall = by_name("cobblestone_wall").expect("cobblestone_wall 在表");
    let stone = by_name("stone").expect("stone 在表");
    assert_eq!(
        Shape::from_u8(mcv_core::BLOCKS[wall as usize].shape),
        Shape::Wall
    );
    let mut m = std::collections::HashMap::new();
    m.insert((0, 0, 0), wall);
    m.insert((0, 0, 1), wall); // +Z 同族连臂
    m.insert((1, 0, 0), stone); // +X sturdy 连臂
    let w = Mem { m };
    let p = BlockPos::new(0, 0, 0);
    let mut boxes = [EMPTY_AABB; MAX_SHAPE_BOXES];

    let n = collision_at(&w, p, &mut boxes);
    assert_eq!(n, 3, "碰撞：柱 + 双臂");
    assert_eq!(boxes[0].min.x, 0.25);
    assert_eq!(boxes[0].max.x, 0.75);
    assert_eq!(boxes[0].max.y, 1.5, "碰撞柱抬高防跳");
    // +Z 臂：z 0.3125..1（格边伸入 11px 的镜像端），y 碰撞也抬 1.5。
    let arm_z = boxes[1..].iter().find(|b| b.max.z == 1.0).expect("+Z 臂盒");
    assert_eq!(arm_z.min.z, 0.3125);
    assert_eq!(arm_z.min.x, 0.3125);
    assert_eq!(arm_z.max.x, 0.6875);
    assert_eq!(arm_z.max.y, 1.5);

    // 拾取：柱 1.0、臂 0.875（low，14px）。
    let n = pick_boxes(&w, p, &mut boxes);
    assert_eq!(n, 3);
    assert_eq!(boxes[0].max.y, 1.0);
    assert!(boxes[1..].iter().all(|b| b.max.y == 0.875));
}

fn collision_at(
    w: &dyn VoxelAccess,
    p: BlockPos,
    out: &mut [mcv_game::physics::Aabb; MAX_SHAPE_BOXES],
) -> usize {
    push_boxes(w.block(p), w, p, RayTarget::Collide, out)
}
