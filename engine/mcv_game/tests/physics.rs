//! 玩家物理测试。

use std::collections::HashMap;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkPos};
use mcv_game::physics::{Aabb, StepInput, step};
use mcv_game::{Player, VoxelAccess, interact, raycast};

const STONE: u16 = 1;
const WATER: u16 = 5;
const FLOWER_RED: u16 = 12;

/// 基于 HashMap 的测试世界：缺省 AIR，光照恒 15，chunk 恒已加载。
struct TestWorld {
    blocks: HashMap<(i32, i32, i32), u16>,
}

impl TestWorld {
    fn new() -> Self {
        Self {
            blocks: HashMap::new(),
        }
    }

    fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        self.blocks.insert((x, y, z), id);
    }

    /// 铺一整层 y（x/z 取 ±16 覆盖测试范围）。
    fn fill_layer(&mut self, y: i32, id: u16) {
        for x in -16..16 {
            for z in -16..16 {
                self.set(x, y, z, id);
            }
        }
    }
}

impl VoxelAccess for TestWorld {
    fn block(&self, p: BlockPos) -> BlockId {
        BlockId(*self.blocks.get(&(p.x, p.y, p.z)).unwrap_or(&0))
    }

    fn light(&self, _p: BlockPos) -> u8 {
        15
    }

    fn chunk_loaded(&self, _c: ChunkPos) -> bool {
        true
    }
}

fn ground_player(x: f32, y: f32, z: f32) -> Player {
    Player {
        pos: Vec3::new(x, y, z),
        ..Player::default()
    }
}

/// 1. 自由落体：y=20 起跳落到 y=0 整层 STONE 上，100 步内落地站稳。
#[test]
fn free_fall_lands() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let mut p = ground_player(0.5, 20.0, 0.5);

    for _ in 0..100 {
        step(&world, &mut p, &StepInput::default());
    }

    assert!(p.pos.y >= 1.0 && p.pos.y <= 1.001, "feet y = {}", p.pos.y);
    assert!(p.on_ground);
}

/// 2. 无地板 600 步：水中下沉终端 2 m/s，|vel.y| ≤ 4。
#[test]
fn terminal_velocity_bounded() {
    let world = TestWorld::new();
    let mut p = ground_player(0.5, 20.0, 0.5);

    for _ in 0..600 {
        step(
            &world,
            &mut p,
            &StepInput {
                in_water: true,
                ..StepInput::default()
            },
        );
    }

    assert!(p.vel.y.abs() <= 4.0, "vel.y = {}", p.vel.y);
    // 水中下沉终端 ≈ 2 m/s。
    assert!(p.vel.y >= -2.01, "vel.y = {}", p.vel.y);
}

/// 3. 平地跳：峰高（脚底抬升量）∈ (1.1, 1.35)。
#[test]
fn jump_peak() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let mut p = ground_player(0.5, 1.1, 0.5);

    // 先站稳。
    for _ in 0..10 {
        step(&world, &mut p, &StepInput::default());
    }
    assert!(p.on_ground);
    let y0 = p.pos.y;

    let mut peak = f32::MIN;
    for _ in 0..40 {
        step(
            &world,
            &mut p,
            &StepInput {
                jump: true,
                ..StepInput::default()
            },
        );
        peak = peak.max(p.pos.y);
    }

    let rise = peak - y0;
    assert!(rise > 1.1 && rise < 1.35, "rise = {}", rise);
}

/// 4. 30 m/s 水平冲 1 格厚墙：100 步零穿透。
#[test]
fn wall_no_tunneling() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    // 墙：x=3 一列（1 格厚），y=0..4。
    for y in 0..4 {
        world.set(3, y, 0, STONE);
    }
    let mut p = ground_player(0.5, 1.1, 0.5);
    p.vel.x = 30.0;

    for _ in 0..100 {
        step(&world, &mut p, &StepInput::default());
        let aabb = Aabb::from_player(p.pos);
        for y in -1..5 {
            for z in -1..2 {
                assert!(
                    !aabb.intersects_voxel(3, y, z),
                    "tunneled into wall at y={y}, z={z}, pos={}",
                    p.pos
                );
            }
        }
        assert!(p.pos.x < 3.0, "pos.x = {}", p.pos.x);
    }
}

/// 5. raycast：眼位向 -Y 命中脚下 STONE（法线 +Y）；空中向 +X 无命中；
///    水/空气穿透、花可命中。
#[test]
fn raycast_hits_and_misses() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.set(2, 20, 0, WATER);
    world.set(5, 20, 0, FLOWER_RED);

    let eye = Vec3::new(0.5, 1.0 + Player::EYE, 0.5);
    let (pos, normal) = raycast(&world, eye, Vec3::new(0.0, -1.0, 0.0), 10.0).unwrap();
    assert_eq!(pos, BlockPos::new(0, 0, 0));
    assert_eq!(normal, [0, 1, 0]);

    assert_eq!(
        raycast(
            &world,
            Vec3::new(0.5, 30.5, 0.5),
            Vec3::new(1.0, 0.0, 0.0),
            50.0
        ),
        None,
        "air ray should miss"
    );

    // 穿过水 (2,20,0)，命中花 (5,20,0)；从 +X 侧进入 → 法线 [-1,0,0]。
    let (pos, normal) = raycast(
        &world,
        Vec3::new(0.5, 20.5, 0.5),
        Vec3::new(1.0, 0.0, 0.0),
        50.0,
    )
    .unwrap();
    assert_eq!(pos, BlockPos::new(5, 20, 0));
    assert_eq!(normal, [-1, 0, 0]);
}

/// 测试用：按注册名查方块 id。
fn id_of(name: &str) -> u16 {
    (0..mcv_core::BLOCKS.len() as u16)
        .find(|i| mcv_core::BLOCKS[*i as usize].name == name)
        .unwrap()
}

/// 6a. 形状碰撞（C3）：下半砖（SlabBlock.java:35 column(16,0,8)）上表面
///     站人——落在 (0,70,0) 的 oak_slab 上，脚底 y=70.5。
#[test]
fn stand_on_bottom_slab() {
    let slab = id_of("oak_slab");
    let mut world = TestWorld::new();
    for x in 0..3 {
        for z in 0..3 {
            world.set(x, 70, z, slab); // state=0 → 下半砖
        }
    }
    let mut p = ground_player(0.5, 75.0, 0.5);
    for _ in 0..400 {
        step(&world, &mut p, &StepInput::default());
    }
    assert!(
        (p.pos.y - 70.5).abs() < 1e-3,
        "应站在半砖台面 70.5，实际 {}",
        p.pos.y
    );
    assert!(p.on_ground);
}

/// 6b. 栅栏防跳（C3）：碰撞柱+臂高 1.5（CrossCollisionBlock.java:45
/// collisionHeight=24），跳峰 1.1~1.35 不可越；且连接臂封住两柱间隙
/// （原版 CrossCollisionBlock.java:56-66 臂计入碰撞箱）。
#[test]
fn fence_blocks_jump_and_gap() {
    let fence = id_of("oak_fence");
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    // 栅栏线：x=3，z=0 与 x=4，z=0（同系互连出 +X/-X 臂）。
    world.set(3, 1, 0, fence);
    world.set(4, 1, 0, fence);

    // (a) 对准单柱正面跳挤：100 步后被挡在 x=3.375（柱 min 面）之前。
    let mut p = ground_player(0.5, 1.1, 0.5);
    let mut peak_y = 0.0f32;
    for _ in 0..120 {
        step(
            &world,
            &mut p,
            &StepInput {
                wish_dir: Vec3::X,
                jump: true,
                ..StepInput::default()
            },
        );
        peak_y = peak_y.max(p.pos.y);
        assert!(p.pos.x < 3.375, "越过栅栏柱：{}", p.pos.x);
    }
    // 跳峰低于碰撞顶 2.5（脚底 1 + 1.5）。
    assert!(peak_y < 2.5, "峰高穿过了 1.5 碰撞：{}", peak_y);

    // (b) 从两柱间隙（x=4.0，柱不覆盖 3.625..4.375）挤入：只有连接臂
    //     存在才会被挡——验证"臂计入碰撞"（防原版式穿缝）。
    let mut q = ground_player(4.0, 1.1, -2.0);
    for _ in 0..120 {
        step(
            &world,
            &mut q,
            &StepInput {
                wish_dir: Vec3::Z,
                ..StepInput::default()
            },
        );
        assert!(q.pos.z < 0.375, "从栅栏臂间隙穿过：{}", q.pos.z);
    }
}

/// 6c. 火把无实体碰撞（C3，Blocks.java torch noCollision；其
/// BaseTorchBlock.java:16 形状仅是拾取 outline）：玩家直接穿过火把格落地。
#[test]
fn torch_has_no_collision() {
    let torch = id_of("torch");
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    world.set(0, 1, 0, torch);
    world.set(0, 2, 0, torch);
    let mut p = ground_player(0.5, 5.0, 0.5);
    for _ in 0..200 {
        step(&world, &mut p, &StepInput::default());
    }
    assert!((p.pos.y - 1.0).abs() < 1e-3, "火把挡人：{}", p.pos.y);
}

/// 6d. 形状拾取命中（C4，BaseTorchBlock.java:16 outline column(4,0,10)）：
///     火把可命中（侧/顶），瞄准火把上方空区穿透；上半砖下半区穿透；
///     石/花/空气行为不变（见 6 raycast 用例）。
#[test]
fn raycast_hits_shape_outlines() {
    let torch = id_of("torch");
    let slab = id_of("oak_slab");
    let mut world = TestWorld::new();
    world.set(5, 20, 0, torch);
    // 上半砖（state bit0=1）放 z=4 列：火把各射线取 z=0.5，互不串扰。
    world.set(7, 20, 4, slab | (1 << 12));

    // 水平命中火把柱（柱 y 0..0.625，取格内 y=0.3 高度）。
    let (pos, normal) = raycast(
        &world,
        Vec3::new(0.5, 20.3, 0.5),
        Vec3::new(1.0, 0.0, 0.0),
        50.0,
    )
    .unwrap();
    assert_eq!(pos, BlockPos::new(5, 20, 0));
    assert_eq!(normal, [-1, 0, 0]);

    // 瞄准火把柱上方（y=0.8 > 0.625）→ 穿透无命中。
    assert_eq!(
        raycast(
            &world,
            Vec3::new(0.5, 20.8, 0.5),
            Vec3::new(1.0, 0.0, 0.0),
            50.0
        ),
        None,
        "火把柱上方不应命中"
    );

    // 自上命中火把顶面（outline 顶 y=20.625 → 法线 +Y）。
    let (pos, normal) = raycast(
        &world,
        Vec3::new(5.5, 25.0, 0.5),
        Vec3::new(0.0, -1.0, 0.0),
        50.0,
    )
    .unwrap();
    assert_eq!(pos, BlockPos::new(5, 20, 0));
    assert_eq!(normal, [0, 1, 0]);

    // 上半砖：下半区（y=0.2）水平射线穿透；上半区（y=0.8）命中。
    assert_eq!(
        raycast(
            &world,
            Vec3::new(5.5, 20.2, 4.5),
            Vec3::new(1.0, 0.0, 0.0),
            50.0
        ),
        None,
        "上半砖下半区应穿透"
    );
    let (pos, normal) = raycast(
        &world,
        Vec3::new(5.5, 20.8, 4.5),
        Vec3::new(1.0, 0.0, 0.0),
        50.0,
    )
    .unwrap();
    assert_eq!(pos, BlockPos::new(7, 20, 4));
    assert_eq!(normal, [-1, 0, 0]);
}

/// 6e. 栅栏碰撞臂的拾取 outline：贴石头连臂（FenceBlock.java:63 sturdy
///     分支），outline 高 1.0（CrossCollisionBlock.java:46 wallHeight）。
#[test]
fn fence_outline_with_sturdy_arm() {
    let fence = id_of("oak_fence");
    let mut world = TestWorld::new();
    world.set(5, 20, 0, fence);
    world.set(4, 20, 0, STONE); // sturdy → 连臂（-X 臂 x 5..5.5）
    // 水平瞄准 -X 臂区域（y=20.3，柱区 5.375 之前 x=5.2 处只有臂）。
    let (pos, _) = raycast(
        &world,
        Vec3::new(3.5, 20.3, 0.5),
        Vec3::new(1.0, 0.0, 0.0),
        50.0,
    )
    .unwrap();
    // 先撞上石头格 (4,20,0) 的整盒边界 x=5.0——石头比臂更靠外，
    // 射线先命中石头；这同时证明拾取按 outline 而非跳过形状块。
    assert_eq!(pos, BlockPos::new(4, 20, 0));
    // 柱上方（y=20.9 < outline 顶 1.0=21.0）从 +Z 命中柱。
    let (pos, normal) = raycast(
        &world,
        Vec3::new(5.5, 20.9, 2.5),
        Vec3::new(0.0, 0.0, -1.0),
        50.0,
    )
    .unwrap();
    assert_eq!(pos, BlockPos::new(5, 20, 0));
    assert_eq!(normal, [0, 0, 1]);
    // y=21.0 以上（outline 高 1.0）穿透。
    assert_eq!(
        raycast(
            &world,
            Vec3::new(5.5, 21.2, 2.5),
            Vec3::new(0.0, 0.0, -1.0),
            50.0
        ),
        None
    );
}

/// 7. can_place_block：脚下格（STONE）不可放、面前一格可放、
///    与玩家重叠或已是实体方块不可放。
#[test]
fn can_place_block_rules() {
    let mut world = TestWorld::new();
    world.fill_layer(0, STONE);
    let p = ground_player(0.5, 1.0 + 1e-4, 0.5);

    // 脚下格是 STONE。
    assert!(!interact::can_place_block(
        &world,
        BlockPos::new(0, 0, 0),
        &p
    ));
    // 面前一格（+X，脚部高度）是 AIR 且不与玩家相交。
    assert!(interact::can_place_block(
        &world,
        BlockPos::new(1, 1, 0),
        &p
    ));
    // 目标已是 STONE。
    assert!(!interact::can_place_block(
        &world,
        BlockPos::new(1, 0, 0),
        &p
    ));
    // AIR 但与玩家身体重叠。
    assert!(!interact::can_place_block(
        &world,
        BlockPos::new(0, 1, 0),
        &p
    ));
}

/// 嵌入固体态的单轴推进：只应就近推出所嵌盒的最近面，绝不能隔着整块
/// 向反方向瞬移（沙坑挣扎回归锁：玩家嵌在 x∈[2,3] 的沙格内向 -X 移动，
/// 旧实现把「前方格 max.x=3.0」也当候选，`max` 选中 3.0 后钳位面落在
/// 玩家身后 1.5 m 处，delta 修正直接把玩家甩到沙块东侧——每帧在沙里
/// 被弹来弹去，即真机“老是能到沙子里面”的挣扎表现；新实现过滤位于
/// 自身反向边界之后的候选面，只就近推到 x≈2.0 的紧邻面）。
#[test]
fn embedded_player_ejects_nearest_face_not_through_block() {
    use mcv_game::physics::{Aabb, Axis, move_axis};

    let mut world = TestWorld::new();
    world.set(1, 1, 0, STONE);
    world.set(2, 1, 0, STONE);

    // 嵌入 [1,2]∪[2,3] 的沙面（脚底中心 x=1.9，盒 x∈[1.6,2.2]），向 -X 推：
    // 旧实现候选 {2.0, 3.0} 取 max=3.0 → 玩家被甩到 x≈3.0；新实现只接受
    // max.x ≤ 自身 max 边界的候选 → 面=2.0，就近推出。
    let mut p = ground_player(1.9, 1.0, 0.5);
    p.vel = Vec3::new(-4.0, 0.0, 0.0);
    let mut aabb = Aabb::from_player(p.pos);
    assert!(move_axis(&world, &mut p, &mut aabb, Axis::X, -0.066));
    assert!(
        aabb.min.x > 1.9 && aabb.min.x < 2.5,
        "嵌入态应向东邻格面 x=2.0 就近弹出，实得 min.x = {}（≈3.0 = 隔着整块瞬移）",
        aabb.min.x
    );

    // 镜像：嵌入 [2,3]∪[3,4]（脚底中心 x=2.75，盒 x∈[2.45,3.05]）向 +X
    // 推，候选 {2.0(身后，过滤), 3.0} → 面=3.0，应就近停在 x=3.0 以西；
    // 旧实现取 min=2.0 → 玩家被隔着两格甩到 x≈2.0 以西。
    world.set(3, 1, 0, STONE);
    let mut p = ground_player(2.75, 1.0, 0.5);
    p.vel = Vec3::new(4.0, 0.0, 0.0);
    let mut aabb = Aabb::from_player(p.pos);
    assert!(move_axis(&world, &mut p, &mut aabb, Axis::X, 0.066));
    assert!(
        aabb.max.x > 2.5 && aabb.max.x < 3.0,
        "嵌入态应向东邻格面 x=3.0 就近弹出，实得 max.x = {}（<2.5 = 隔着整块瞬移）",
        aabb.max.x
    );

    // 正常逼近路径回归：玩家从西侧接近石壁，必须钳在 x=2.0 面上。
    let mut world2 = TestWorld::new();
    world2.set(2, 1, 0, STONE);
    let mut p = ground_player(1.2, 1.0, 0.5);
    p.vel = Vec3::new(4.0, 0.0, 0.0);
    let mut aabb = Aabb::from_player(p.pos);
    // 中心 x=1.2（盒 x∈[0.9,1.5]）推进 0.6：子步 0.5 后 max.x=2.0 贴面
    // （严格重叠不含），再 0.1 才真重叠 → 钳位面 x=2.0。
    assert!(move_axis(&world2, &mut p, &mut aabb, Axis::X, 0.6));
    assert!(
        aabb.max.x <= 2.0,
        "逼近路径应钳在墙面 x=2.0，实得 max.x = {}",
        aabb.max.x
    );
}
