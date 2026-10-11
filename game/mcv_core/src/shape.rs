//! 方块形状表：id → 形状（cross/火把/栅栏/半砖/楼梯/立方 + M 波 #138 全表：
//! 柵栏门/门/按钮/压力板/梯子/雪层）。
//!
//! 形状分类是**按注册名生成的逻辑**：规则与 ci/gen-blocks.py 的
//! `shape_of_name()` 和 C++ 侧 `blocks_gen.inc` 的 `shape` 字段逐字对齐
//! （三处同源、纯名字派生，无 Mojang 代码；几何常量为原版数值）。
//! 修改任何一侧规则必须同步另外两侧，否则 Rust 逻辑/C++ 渲染会漂移。
//!
//! # 模板盒单一数据源（M 波 #138）
//!
//! [`template_boxes`] 是非立方形状的几何常数**唯一**出处：渲染
//! （mcv_mesher `emit_shapes` 的 6..15 档）与拾取/碰撞
//! （engine/mcv_game/src/blockshapes.rs）都从这张表取盒。数值全部换算自
//! 26.1 反编译源码（`Block.java:163-202` 助手语义：`box(a..b)=[a/16,b/16]`、
//! `column(w,y0,y1)`=x/z 居中 w 宽、`cube(sx,sy,sz)`=x/z 居中+**y 居中**、
//! `boxZ(sizeX,sizeY,minZ,maxZ)`=x 居中 + y 居中 + z minZ..maxZ；
//! `Shapes.rotateHorizontal:399-420` 的参数名叫 `north`——**基准形对应
//! FACING=NORTH 条目**，因此贴边形（门/梯子/贴墙按钮/开活板门）统一
//! 「slab 位于 FACING **对侧**边」（支撑墙在 FACING.opposite 侧，
//! LadderBlock.java:52-55 canSurvive）。每个形状的出处见 match 臂内注释。

/// 方块形状（值写入 `BlockDef::shape` / C++ `BlockInfo::shape`）。
///
/// 本枚举的 `shape` 位与遮光表同源不同用：遮光唯一数据源是
/// `mcv_core::OPACITY`（`gen_opacity` 三段规则 + 显式例外，见该处
/// file:line），其规则④按生成表 `model_kind`（非立方渲染=1）判 0，
/// 与本枚举的非立方分类覆盖同一批方块（板/梯/栅栏/火把/花草），但
/// 玻璃类例外走 `TransparentBlock.java:34-37`（damp=0），与形状位无关。
/// 改本表不自动改遮光；两侧同改须各自核 `src-26.1`。形状感知遮光
/// `useShapeForLightOcclusion` 的“半影”细节登记于 mcv_light，见该处
/// KNOWN-DIVERGENCE 注释。
///
/// 编号纪律：**1..5 是黄金冻结档**（golden 场景/随机批含这五类，网格器
/// 输出逐字节锁定，分类规则与几何都不得改动）；6..9 与 10..15 允许在
/// golden 列冻结 0 下新增（mcv_mesher/tests/golden.rs shape 锁规则）。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Cube = 0,
    Cross = 1,
    Torch = 2,
    Fence = 3,
    Slab = 4,
    Stairs = 5,
    Carpet = 6,
    Trapdoor = 7,
    Pane = 8,
    Wall = 9,
    /// 柵栏门（FenceGateBlock）：整宽 4px 板横跨 FACING 轴；碰撞 24px 高、
    /// open 空。26.1 无栅栏门模板前的全盒占位由本变体替代。
    FenceGate = 10,
    /// 门（DoorBlock）：3px 薄板贴 FACING 对侧边；open 绕 hinge 转 90°。
    /// HALF（上/下半）不影响形状（getShape 无 half 分支），不占状态位。
    Door = 11,
    /// 按钮（ButtonBlock）：6×4×2px 贴附面小盒（floor/ceiling/wall）。
    Button = 12,
    /// 压力板（BasePressurePlateBlock）：14×1×14px 薄板；pressed 减半厚。
    PressurePlate = 13,
    /// 梯子（LadderBlock）：3px 薄板贴 FACING 对侧边（支撑墙同侧）。
    Ladder = 14,
    /// 雪层（SnowLayerBlock）：高度 = 层数×2px；原版注册名 "snow" 即雪层。
    SnowLayer = 15,
}

impl Shape {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Cross,
            2 => Self::Torch,
            3 => Self::Fence,
            4 => Self::Slab,
            5 => Self::Stairs,
            6 => Self::Carpet,
            7 => Self::Trapdoor,
            8 => Self::Pane,
            9 => Self::Wall,
            10 => Self::FenceGate,
            11 => Self::Door,
            12 => Self::Button,
            13 => Self::PressurePlate,
            14 => Self::Ladder,
            15 => Self::SnowLayer,
            _ => Self::Cube,
        }
    }
}

/// 十字植物（花草类）精确名单；与 gen-blocks.py `CROSS_PLANTS` 一致。
/// 注：官方短草 blockstate 名 "grass" 与旧 id 3（草方块，本引擎旧名同名）
/// 冲突，无法按名区分，故不入名单（保持 Cube 整盒渲染）。
const CROSS_PLANTS: [&str; 18] = [
    "flower_red",
    "flower_yellow",
    "allium",
    "azure_bluet",
    "blue_orchid",
    "cornflower",
    "lily_of_the_valley",
    "oxeye_daisy",
    "torchflower",
    "torchflower_crop",
    "wither_rose",
    "short_grass",
    "fern",
    "tall_grass",
    "large_fern",
    "rose_bush",
    "pink_petals",
    "wildflowers",
];

const fn eq_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let mut j = 0;
        while j < needle.len() && hay[i + j] == needle[j] {
            j += 1;
        }
        if j == needle.len() {
            return true;
        }
        i += 1;
    }
    false
}

const fn ends_with(hay: &[u8], suf: &[u8]) -> bool {
    if suf.len() > hay.len() {
        return false;
    }
    let mut i = 0;
    while i < suf.len() {
        if hay[hay.len() - suf.len() + i] != suf[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// 名字 → 形状编号（`Shape` 的 u8 判别值）。const 供 `gen_def` 建表期使用。
/// 规则顺序（与 python 版逐字一致）：
/// `potted_*` → Cube（盆栽装饰，非植物本体）；含 `fence` 且不含 `gate` →
/// Fence；含 `slab` → Slab；含 `stairs` → Stairs；含 `carpet` → Carpet；
/// 含 `trapdoor` → Trapdoor；含 `pane` → Pane；`_wall` 后缀 → Wall；
/// 精确花草名或 `sapling` 后缀 → Cross；含 `torch` 且不含 `wall` → Torch；
/// M 波追加（顺序在 trapdoor 之后，`_door` 词根不会误吞 trapdoor）：
/// `_fence_gate` 后缀 → FenceGate；`_door` 后缀 → Door；`_button` 后缀 →
/// Button；`_pressure_plate` 后缀 → PressurePlate；精确 `ladder` → Ladder；
/// 精确 `snow` → SnowLayer（原版注册名 "snow" 即雪层块，SnowLayerBlock）；
/// 否则 Cube。
///
/// `_wall` 后缀只命中 `cobblestone_wall` 类真墙名；`wall_torch`/
/// `oak_wall_sign` 等贴墙变体名不以 `_wall` 结尾，落回 Torch/Cube。
pub const fn shape_of_name(name: &str) -> u8 {
    let n = name.as_bytes();
    if contains(n, b"potted") {
        return Shape::Cube as u8;
    }
    if contains(n, b"fence") && !contains(n, b"gate") {
        return Shape::Fence as u8;
    }
    if contains(n, b"slab") {
        return Shape::Slab as u8;
    }
    if contains(n, b"stairs") {
        return Shape::Stairs as u8;
    }
    if contains(n, b"carpet") {
        return Shape::Carpet as u8;
    }
    if contains(n, b"trapdoor") {
        return Shape::Trapdoor as u8;
    }
    if contains(n, b"pane") {
        return Shape::Pane as u8;
    }
    if ends_with(n, b"_wall") {
        return Shape::Wall as u8;
    }
    let mut i = 0;
    while i < CROSS_PLANTS.len() {
        if eq_bytes(n, CROSS_PLANTS[i].as_bytes()) {
            return Shape::Cross as u8;
        }
        i += 1;
    }
    if ends_with(n, b"sapling") {
        return Shape::Cross as u8;
    }
    if contains(n, b"torch") && !contains(n, b"wall") {
        return Shape::Torch as u8;
    }
    // —— M 波 #138 追加族（全 1171 名模拟无意外命中，见 HANDOFF-SHAPES）——
    if ends_with(n, b"_fence_gate") {
        return Shape::FenceGate as u8;
    }
    if ends_with(n, b"_door") {
        return Shape::Door as u8;
    }
    if ends_with(n, b"_button") {
        return Shape::Button as u8;
    }
    if ends_with(n, b"_pressure_plate") {
        return Shape::PressurePlate as u8;
    }
    if eq_bytes(n, b"ladder") {
        return Shape::Ladder as u8;
    }
    if eq_bytes(n, b"snow") {
        return Shape::SnowLayer as u8;
    }
    Shape::Cube as u8
}

/// 体素值（含状态位）→ 形状；未注册 id 回退 Cube。
#[inline]
pub fn shape(id: u16) -> Shape {
    let real = (id & crate::ID_MASK) as usize;
    if real < crate::BLOCKS.len() {
        Shape::from_u8(crate::BLOCKS[real].shape)
    } else {
        Shape::Cube
    }
}

// ---------------------------------------------------------------------------
// 模板盒表（渲染 / 拾取 / 碰撞的单一数据源，M 波 #138）
// ---------------------------------------------------------------------------

/// 每形状最多盒数：栅栏/板/墙 = 立柱 1 + 四向臂 4。
pub const MAX_TEMPLATE_BOXES: usize = 5;

/// 格内局部盒（0..1；与 blockshapes::local_box / mesher Box3 同域）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShapeBox {
    pub x0: f32,
    pub y0: f32,
    pub z0: f32,
    pub x1: f32,
    pub y1: f32,
    pub z1: f32,
}

/// 盒用途：`Outline` = 渲染几何与拾取轮廓（26.1 `getShape`）；`Collision` =
/// 碰撞（`getCollisionShape`）。大多数形状两者同值；栅栏/墙（防跳抬到
/// 24px）、柵栏门（碰撞高 1.5 / open 空）按原版分离。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoxUse {
    Outline,
    Collision,
}

const fn bx(x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32) -> ShapeBox {
    ShapeBox {
        x0,
        y0,
        z0,
        x1,
        y1,
        z1,
    }
}

/// 臂盒：`dir` 0=+X 1=-X 2=+Z 3=-Z；断面横轴 c0..c1；沿臂向自格边
/// （+X/+Z 的 1.0、-X/-Z 的 0.0）伸入到 d（镜像端 1.0-d）。
const fn arm_box(dir: usize, c0: f32, c1: f32, d: f32, y0: f32, y1: f32) -> ShapeBox {
    match dir {
        0 => bx(d, y0, c0, 1.0, y1, c1),
        1 => bx(0.0, y0, c0, 1.0 - d, y1, c1),
        2 => bx(c0, y0, d, c1, y1, 1.0),
        _ => bx(c0, y0, 0.0, c1, y1, 1.0 - d),
    }
}

/// 四水平臂方向序（与 blockshapes 历史常数同一顺序，勿改）。
const ARM_DIRS: [usize; 4] = [0, 1, 2, 3];

/// 朝向四向编码（stairs/slab 既有约定）：0=+Z 1=-Z 2=+X 3=-X。
/// `facing_cw` = 指南针顺时针（N→E→S→W：1→2→0→3→1），
/// `facing_ccw` 逆时针。26.1 `Direction.getClockWise/getCounterClockWise`。
const fn facing_cw(f: u8) -> u8 {
    match f {
        1 => 2,
        2 => 0,
        0 => 3,
        _ => 1,
    }
}

const fn facing_ccw(f: u8) -> u8 {
    match f {
        1 => 3,
        3 => 0,
        0 => 2,
        _ => 1,
    }
}

/// 贴边薄板（门/梯子/开活板门/贴墙按钮共用语义）：位于 FACING **对侧**
/// 边（支撑墙侧，见模块头 rotateHorizontal 语义）。`t` = 厚度（px/16）。
const fn edge_slab(facing: u8, t: f32, y0: f32, y1: f32) -> ShapeBox {
    match facing {
        0 => bx(0.0, y0, 0.0, 1.0, y1, t),       // +Z → 贴 -Z 边
        1 => bx(0.0, y0, 1.0 - t, 1.0, y1, 1.0), // -Z → 贴 +Z 边
        2 => bx(0.0, y0, 0.0, t, y1, 1.0),       // +X → 贴 -X 边
        _ => bx(1.0 - t, y0, 0.0, 1.0, y1, 1.0), // -X → 贴 +X 边
    }
}

/// 非立方形状模板盒表：`shape`+状态 → 盒数组与有效数（尾部零填充）。
///
/// **消费方**：mcv_mesher `emit_shapes`（6..15 档渲染几何，Outline）、
/// mcv_game::blockshapes `push_boxes`（Pick=Outline / Collide=Collision）。
/// 1..5 档（cross/torch/fence/slab/stairs）网格器保持黄金冻结的既有实现，
/// 由对拍测试（mcv_game/tests/shapes_table.rs 渲染 AABB==拾取 AABB 全表）
/// 锁两侧同值；本表对 1..5 同样给出拾取/碰撞值供 blockshapes 消费。
///
/// 状态位约定（4bit nibble，各形状自用）：
/// - Slab：bit0 上半。Stairs：bit0-1 朝向、bit2 上半。
/// - Trapdoor：bit0-1 朝向、bit2 上半、bit3 open。
/// - FenceGate：bit0-1 朝向、bit2 open、bit3 in_wall。
/// - Door：bit0-1 朝向、bit2 open、bit3 hinge（0=LEFT 1=RIGHT）。
/// - Button：bit0-1 朝向、bit2-3 贴附面（0 地面 1 墙面 2 天花板）。
/// - PressurePlate：bit0 pressed。
/// - Ladder：bit0-1 朝向。SnowLayer：bit0-3 = 层数-1（≥7 视作满层 8）。
pub const fn template_boxes(
    sh: Shape,
    state: u8,
    use_: BoxUse,
) -> ([ShapeBox; MAX_TEMPLATE_BOXES], usize) {
    let mut out = [bx(0.0, 0.0, 0.0, 0.0, 0.0, 0.0); MAX_TEMPLATE_BOXES];
    let mut n = 0usize;
    // add：追加一盒（表容量内；超出的形状本就不该超过 5）。
    macro_rules! add {
        ($b:expr) => {{
            out[n] = $b;
            n += 1;
        }};
    }
    match sh {
        // 整立方由贪心路径/固体检出分支处理，模板表不产盒。
        Shape::Cube => {}
        // 十字植物：渲染走专用双面对角 quad（emit_cross）；拾取轮廓=整格
        // （blockshapes Cross 分支同值），无碰撞（Blocks.java noCollision）。
        Shape::Cross => {
            if matches!(use_, BoxUse::Outline) {
                add!(bx(0.0, 0.0, 0.0, 1.0, 1.0, 1.0));
            }
        }
        // 火把 outline：BaseTorchBlock.java:16 column(4,0,10)（x/z 6..10px、
        // y 0..10px）；无碰撞（Blocks.java torch noCollision）。渲染侧
        // （mesher 0.4..0.6 柱）是黄金冻结的 C++ 近似，不吃本表——差异
        // 由对拍测试记名豁免。
        Shape::Torch => {
            if matches!(use_, BoxUse::Outline) {
                add!(bx(0.375, 0.0, 0.375, 0.625, 0.625, 0.625));
            }
        }
        // 半砖：SlabBlock.java:35-36 column(16,0,8)/column(16,8,16)。
        Shape::Slab => {
            let y0 = if (state & 1) != 0 { 0.5 } else { 0.0 };
            add!(bx(0.0, y0, 0.0, 1.0, y0 + 0.5, 1.0));
        }
        // 楼梯：StairBlock.java:37-38,74-89——整格宽半高底座 + 踏步半盒
        // （整高半在 FACING 朝向侧）；bit2=top 翻转。
        Shape::Stairs => {
            let flipped = (state & 4) != 0;
            let (by0, sy0) = if flipped { (0.5, 0.0) } else { (0.0, 0.5) };
            add!(bx(0.0, by0, 0.0, 1.0, by0 + 0.5, 1.0));
            match state & 3 {
                0 => add!(bx(0.0, sy0, 0.5, 1.0, sy0 + 0.5, 1.0)),
                1 => add!(bx(0.0, sy0, 0.0, 1.0, sy0 + 0.5, 0.5)),
                2 => add!(bx(0.5, sy0, 0.0, 1.0, sy0 + 0.5, 1.0)),
                _ => add!(bx(0.0, sy0, 0.0, 0.5, sy0 + 0.5, 1.0)),
            }
        }
        // 地毯：CarpetBlock.java:17 column(16,0,1)（moss_carpet BASE 态同值
        // MossyCarpetBlock makeShapes；悬边 draped 态登记遗留）。
        Shape::Carpet => {
            add!(bx(0.0, 0.0, 0.0, 1.0, 0.0625, 1.0));
        }
        // 活板门：TrapDoorBlock.java:48-60——SHAPES=rotateAll(boxZ(16,13,16))
        // （3px 板）：关态 half=UP → y 0..3px、half=DOWN → y 13..16px；
        // 开态 getShape(FACING) → 竖板贴 FACING 对侧边（rotateHorizontal
        // base=north 语义，见模块头）。outline=collision（无覆写）。
        Shape::Trapdoor => {
            let facing = state & 3;
            if (state & 8) != 0 {
                add!(edge_slab(facing, 0.1875, 0.0, 1.0));
            } else if (state & 4) != 0 {
                add!(bx(0.0, 0.8125, 0.0, 1.0, 1.0, 1.0));
            } else {
                add!(bx(0.0, 0.0, 0.0, 1.0, 0.1875, 1.0));
            }
        }
        // 玻璃板：IronBarsBlock.java:29-31 super(2,16,2,16,16)（
        // CrossCollisionBlock）——柱 column(2,0,16)（x/z 7..9px）、臂
        // boxZ(2,0,16,0,8) 自格边伸到中心、全高；碰撞=拾取（无 1.5 抬高）。
        Shape::Pane => {
            add!(bx(0.4375, 0.0, 0.4375, 0.5625, 1.0, 0.5625));
            for dir in ARM_DIRS {
                add!(arm_box(dir, 0.4375, 0.5625, 0.5, 0.0, 1.0));
            }
        }
        // 墙：WallBlock.java:66-74 makeShapes(16,14)——柱 column(8,0,16)
        // （x/z 4..12px）、臂 boxZ(6,0,14,0,11)（断面 5..11px、自格边伸入
        // 11px、高 14px）；碰撞 makeShapes(24,24) 柱/臂全抬 1.5（防跳）。
        Shape::Wall => {
            let (hp, ha) = if matches!(use_, BoxUse::Collision) {
                (1.5f32, 1.5f32)
            } else {
                (1.0f32, 0.875f32)
            };
            add!(bx(0.25, 0.0, 0.25, 0.75, hp, 0.75));
            for dir in ARM_DIRS {
                add!(arm_box(dir, 0.3125, 0.6875, 0.3125, 0.0, ha));
            }
        }
        // 栅栏：CrossCollisionBlock.java:36-47 + FenceBlock.java:37,41
        // super(4,16,4,16,24)——outline 柱 column(4,0,16) + 臂 boxZ(4,0,16,
        // 0,8)（全高）；碰撞 makeShapes(4,24,4,0,24) 柱/臂全抬 24px。
        // （渲染臂是黄金冻结的薄梁 y 0.375..0.5625，不吃本表。）
        Shape::Fence => {
            let h = if matches!(use_, BoxUse::Collision) {
                1.5
            } else {
                1.0
            };
            add!(bx(0.375, 0.0, 0.375, 0.625, h, 0.625));
            for dir in ARM_DIRS {
                add!(arm_box(dir, 0.375, 0.625, 0.5, 0.0, h));
            }
        }
        // 柵栏门：FenceGateBlock.java:39-44,47-51——outline=cube(16,16,4)
        // （Block.java:175-178 = column(16,4,0,16) = box(0,0,6,16,16,10)，
        // 整宽 4px 板横跨 FACING 轴）；in_wall outline −3px（SHAPES_WALL=
        // gate−column(16,13,16)，:40-42）；碰撞 column(16,4,0,24)（:44-46，
        // 高 1.5 防跳、与 in_wall 无关）、open 空（:56-62）。
        Shape::FenceGate => {
            let axis_z = (state & 3) < 2; // 朝向 0/1=±Z → 板横跨 X
            let h = if matches!(use_, BoxUse::Collision) {
                1.5
            } else if (state & 8) != 0 {
                0.8125 // in_wall：outline y 0..13px
            } else {
                1.0
            };
            let open = matches!(use_, BoxUse::Collision) && (state & 4) != 0;
            if !open {
                if axis_z {
                    add!(bx(0.0, 0.0, 0.375, 1.0, h, 0.625));
                } else {
                    add!(bx(0.375, 0.0, 0.0, 0.625, h, 1.0));
                }
            }
        }
        // 门：DoorBlock.java:47-55——SHAPES=rotateHorizontal(boxZ(16,13,16))
        // （3px 板）；关门 slab 贴 FACING 对侧边；开门 doorDirection =
        // hinge RIGHT → CCW、LEFT → CW（:50-55），slab 贴对侧边。
        // outline=collision（无碰撞覆写）。
        Shape::Door => {
            let facing = state & 3;
            let dir = if (state & 4) != 0 {
                if (state & 8) != 0 {
                    facing_ccw(facing)
                } else {
                    facing_cw(facing)
                }
            } else {
                facing
            };
            add!(edge_slab(dir, 0.1875, 0.0, 1.0));
        }
        // 按钮：ButtonBlock.java:70-79 makeShapes——rotateAttachFace(
        // boxZ(6,4,8,16))（Block.java:194-197 = box(5,6,8,11,10,16) 基准
        // 南向半深杆）∩ cube(12)=box(2,2,2,14,14,14)（:71 ONLY_FIRST 相减）
        // → 6×4×2px 贴附小盒：地面 box(5,0,6,11,2,10)、天花板 y 14..16、
        // 墙面贴 FACING 对侧边（断面 x 5..11 / y 6..10px）。无碰撞
        // （Blocks.java button noCollission）。
        Shape::Button => {
            if matches!(use_, BoxUse::Outline) {
                let facing = state & 3;
                match (state >> 2) & 3 {
                    0 => add!(bx(0.3125, 0.0, 0.375, 0.6875, 0.125, 0.625)),
                    2 => add!(bx(0.3125, 0.875, 0.375, 0.6875, 1.0, 0.625)),
                    // 墙面（含 3 档回退=墙）：6 宽 × 4 高 × 2 厚，贴
                    // FACING 对侧边（断面 x 5..11px、y 6..10px）。
                    _ => match facing {
                        0 => add!(bx(0.3125, 0.375, 0.0, 0.6875, 0.625, 0.125)),
                        1 => add!(bx(0.3125, 0.375, 0.875, 0.6875, 0.625, 1.0)),
                        2 => add!(bx(0.0, 0.375, 0.375, 0.125, 0.625, 0.625)),
                        _ => add!(bx(0.875, 0.375, 0.375, 1.0, 0.625, 0.625)),
                    },
                }
            }
        }
        // 压力板：BasePressurePlateBlock.java:26-27——SHAPE=column(14,0,1)
        // （x/z 1..15px、y 0..1px）、SHAPE_PRESSED=column(14,0,0.5)；无碰撞
        // （Blocks.java pressure plate noCollission）。
        Shape::PressurePlate => {
            if matches!(use_, BoxUse::Outline) {
                let y1 = if (state & 1) != 0 { 0.03125 } else { 0.0625 };
                add!(bx(0.0625, 0.0, 0.0625, 0.9375, y1, 0.9375));
            }
        }
        // 梯子：LadderBlock.java:29,43——SHAPES=rotateHorizontal(boxZ(16,13,
        // 16))（3px 板贴 FACING 对侧边，支撑墙在 FACING.opposite，:52-55）。
        // 原版 ladder 无碰撞（noCollission）；gen 表 solid=true（
        // blocks_gen.inc.rs 冻结字段）+ shapes_table「solid ⇒ 非空碰撞盒」
        // 不变量 → 碰撞=拾取薄板，KNOWN-DIVERGENCE（原版实体可嵌入梯格）。
        Shape::Ladder => {
            add!(edge_slab(state & 3, 0.1875, 0.0, 1.0));
        }
        // 雪层：SnowLayerBlock.java:28,47-55——SHAPES[layer]=column(16,0,
        // layer*2)（每层 2px）；原版碰撞=SHAPES[layer-1]（低一层，1 层时
        // 空形），gen 表 solid=true + shapes_table 不变量 → 碰撞=outline
        // 高（1 层 2px 实体防穿），KNOWN-DIVERGENCE（原版 1 层雪可踩穿）。
        Shape::SnowLayer => {
            let layers = (state & 15).min(7) + 1; // 1..8
            let h = layers as f32 * 0.125;
            add!(bx(0.0, 0.0, 0.0, 1.0, h, 1.0));
        }
    }
    (out, n)
}

/// 臂连接判据（栅栏/板/墙共用，**渲染臂与碰撞臂同一规则**；原
/// mesher.rs `arm_connects` 与 blockshapes.rs `connects_family` 收拢至此）。
///
/// 26.1 近似移植（`FenceBlock.java:59-63` connectsTo 同族 ∥ faceSturdy）。
/// KNOWN-DIVERGENCE（无 tag / blockstate 体系，与 cpp/src/mesher.cpp
/// `fence_arm_connects` 同一近似，两侧必须同步）：
/// - 26.1 sturdy = 邻格支撑形在该面整面（`SupportType.java:11-16`）；此处以
///   "实体不透明整立方"近似——玻璃等 opaque=false 整方块原版会连，这里
///   不连（保守方向）；
/// - `BlockTags.FENCES` / `WOODEN_FENCES`（`FenceBlock.java:65-68`）以
///   "shape==同族" 近似——木质/非木质跨类（橡木↔下界砖栅栏）会误连；
/// - `isExceptionForConnection` 例外名单（`Block.java:255-262`）未移植。
#[inline]
pub fn connects(nb: u16, family: Shape) -> bool {
    let real = (nb & crate::ID_MASK) as usize;
    if real >= crate::BLOCKS.len() {
        // 未注册 id（含 0xFFFF 哨兵与空气 id 0）一律不连。
        return false;
    }
    let d = &crate::BLOCKS[real];
    if d.shape == family as u8 {
        return true;
    }
    d.solid && d.opaque && !d.liquid && d.shape == Shape::Cube as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rules() {
        assert_eq!(shape_of_name("stone"), 0);
        assert_eq!(shape_of_name("grass"), 0); // 旧草方块名，非短草
        assert_eq!(shape_of_name("short_grass"), 1);
        assert_eq!(shape_of_name("flower_red"), 1);
        assert_eq!(shape_of_name("oak_sapling"), 1);
        assert_eq!(shape_of_name("torch"), 2);
        assert_eq!(shape_of_name("soul_wall_torch"), 0); // wall 变体保持 Cube
        assert_eq!(shape_of_name("oak_fence"), 3);
        assert_eq!(shape_of_name("acacia_slab"), 4);
        assert_eq!(shape_of_name("bamboo_mosaic_stairs"), 5);
        assert_eq!(shape_of_name("potted_oak_sapling"), 0);
        assert_eq!(shape_of_name("torchflower"), 1); // 花草名单优先于 torch 词根
        assert_eq!(shape_of_name("red_carpet"), 6);
        assert_eq!(shape_of_name("oak_trapdoor"), 7);
        assert_eq!(shape_of_name("iron_trapdoor"), 7);
        assert_eq!(shape_of_name("glass_pane"), 8);
        assert_eq!(shape_of_name("cobblestone_wall"), 9);
        assert_eq!(shape_of_name("wall_torch"), 0); // 贴墙火把非墙（_wall 后缀）
        assert_eq!(shape_of_name("oak_wall_sign"), 0);
    }

    #[test]
    fn classify_rules_m_wave_families() {
        // M 波 #138：dispatch 全表——编号 ≥10（golden 锁规则允许新增档）。
        assert_eq!(shape_of_name("oak_fence_gate"), 10);
        assert_eq!(shape_of_name("nether_brick_fence_gate"), 10);
        assert_eq!(shape_of_name("oak_door"), 11);
        assert_eq!(shape_of_name("copper_door"), 11);
        assert_eq!(shape_of_name("oak_button"), 12);
        assert_eq!(shape_of_name("polished_blackstone_button"), 12);
        assert_eq!(shape_of_name("stone_pressure_plate"), 13); // plate ≠ pane
        assert_eq!(shape_of_name("heavy_weighted_pressure_plate"), 13);
        assert_eq!(shape_of_name("ladder"), 14);
        assert_eq!(shape_of_name("snow"), 15); // 原版 "snow" = 雪层（SnowLayerBlock）
        assert_eq!(shape_of_name("oak_trapdoor"), 7); // trapdoor 先于 _door 词根
        assert_eq!(shape_of_name("snow_grass"), 0); // 旧草方块雪顶态保持整块
    }

    /// 模板盒值锚（原版 px→格换算的抽样锁定；全量对拍在对拍测试里）。
    #[test]
    fn template_box_values() {
        // 半砖（SlabBlock.java:35-36）。
        let (b, n) = template_boxes(Shape::Slab, 0, BoxUse::Outline);
        assert_eq!(n, 1);
        assert_eq!(b[0], bx(0.0, 0.0, 0.0, 1.0, 0.5, 1.0));
        // 火把 outline（BaseTorchBlock.java:16）。
        let (b, n) = template_boxes(Shape::Torch, 0, BoxUse::Outline);
        assert_eq!(n, 1);
        assert_eq!(b[0], bx(0.375, 0.0, 0.375, 0.625, 0.625, 0.625));
        assert_eq!(template_boxes(Shape::Torch, 0, BoxUse::Collision).1, 0);
        // 地毯（CarpetBlock.java:17）。
        let (b, n) = template_boxes(Shape::Carpet, 0, BoxUse::Collision);
        assert_eq!(n, 1);
        assert_eq!(b[0].y1, 0.0625);
        // 柵栏碰撞柱 1.5（CrossCollisionBlock.java:45）/ outline 1.0。
        let (b, n) = template_boxes(Shape::Fence, 0, BoxUse::Collision);
        assert_eq!(n, 5, "柱+四臂");
        assert_eq!(b[0].y1, 1.5);
        assert_eq!(b[1], bx(0.5, 0.0, 0.375, 1.0, 1.5, 0.625)); // +X 臂
        let (b, _) = template_boxes(Shape::Fence, 0, BoxUse::Outline);
        assert_eq!(b[0].y1, 1.0);
        // 墙 outline 臂 0.875（14px）/ 碰撞 1.5（WallBlock.java:66-74）。
        let (b, _) = template_boxes(Shape::Wall, 0, BoxUse::Outline);
        assert_eq!(b[0].y1, 1.0);
        assert_eq!(b[1], bx(0.3125, 0.0, 0.3125, 1.0, 0.875, 0.6875));
        let (b, _) = template_boxes(Shape::Wall, 0, BoxUse::Collision);
        assert_eq!(b[0].y1, 1.5);
        // 玻璃板（IronBarsBlock super(2,16,2,16,16)）。
        let (b, n) = template_boxes(Shape::Pane, 0, BoxUse::Outline);
        assert_eq!(n, 5);
        assert_eq!(b[0], bx(0.4375, 0.0, 0.4375, 0.5625, 1.0, 0.5625));
        // 柵栏门：±Z 朝向板横跨 X；碰撞 1.5；open 碰撞空；in_wall 0.8125。
        let (b, n) = template_boxes(Shape::FenceGate, 0, BoxUse::Outline);
        assert_eq!(n, 1);
        assert_eq!(b[0], bx(0.0, 0.0, 0.375, 1.0, 1.0, 0.625));
        let (b, _) = template_boxes(Shape::FenceGate, 2, BoxUse::Outline);
        assert_eq!(b[0], bx(0.375, 0.0, 0.0, 0.625, 1.0, 1.0));
        let (b, _) = template_boxes(Shape::FenceGate, 0, BoxUse::Collision);
        assert_eq!(b[0].y1, 1.5);
        assert_eq!(template_boxes(Shape::FenceGate, 4, BoxUse::Collision).1, 0);
        let (b, _) = template_boxes(Shape::FenceGate, 8, BoxUse::Outline);
        assert_eq!(b[0].y1, 0.8125);
        // 门：关门贴 FACING 对侧边（0=+Z → -Z 边）；开 LEFT hinge → CW。
        let (b, _) = template_boxes(Shape::Door, 0, BoxUse::Outline);
        assert_eq!(b[0], bx(0.0, 0.0, 0.0, 1.0, 1.0, 0.1875));
        let (b, _) = template_boxes(Shape::Door, 1, BoxUse::Outline);
        assert_eq!(b[0], bx(0.0, 0.0, 0.8125, 1.0, 1.0, 1.0));
        // facing=1(-Z)，hinge LEFT(bit3=0)，open(bit2) → dir=cw(1)=2(+X)
        // → slab 贴 -X 边（DoorBlock.java:50-55：LEFT → getClockWise）。
        let (b, _) = template_boxes(Shape::Door, 0b0101, BoxUse::Outline);
        assert_eq!(b[0], bx(0.0, 0.0, 0.0, 0.1875, 1.0, 1.0));
        // hinge RIGHT(bit3=1) → dir=ccw(1)=3(-X) → slab 贴 +X 边。
        let (b, _) = template_boxes(Shape::Door, 0b1101, BoxUse::Outline);
        assert_eq!(b[0], bx(0.8125, 0.0, 0.0, 1.0, 1.0, 1.0));
        // 按钮：地面 6×2×4（box(5,0,6,11,2,10)px）；墙 −Z 朝向贴 +Z 边。
        let (b, _) = template_boxes(Shape::Button, 0, BoxUse::Outline);
        assert_eq!(b[0], bx(0.3125, 0.0, 0.375, 0.6875, 0.125, 0.625));
        let (b, _) = template_boxes(Shape::Button, 0b0101, BoxUse::Outline);
        assert_eq!(b[0], bx(0.3125, 0.375, 0.875, 0.6875, 0.625, 1.0));
        // 压力板（BasePressurePlateBlock.java:26-27）。
        let (b, _) = template_boxes(Shape::PressurePlate, 0, BoxUse::Outline);
        assert_eq!(b[0], bx(0.0625, 0.0, 0.0625, 0.9375, 0.0625, 0.9375));
        let (b, _) = template_boxes(Shape::PressurePlate, 1, BoxUse::Outline);
        assert_eq!(b[0].y1, 0.03125);
        // 梯子：贴 FACING 对侧边 3px 板。
        let (b, _) = template_boxes(Shape::Ladder, 2, BoxUse::Outline);
        assert_eq!(b[0], bx(0.0, 0.0, 0.0, 0.1875, 1.0, 1.0));
        // 雪层：state0=1 层 2px；state6=7 层 14px；state7+=满层。
        let (b, _) = template_boxes(Shape::SnowLayer, 0, BoxUse::Outline);
        assert_eq!(b[0].y1, 0.125);
        let (b, _) = template_boxes(Shape::SnowLayer, 6, BoxUse::Collision);
        assert_eq!(b[0].y1, 0.875);
        let (b, _) = template_boxes(Shape::SnowLayer, 9, BoxUse::Outline);
        assert_eq!(b[0].y1, 1.0);
        // 活板门：下/上关态 + 开态竖板（TrapDoorBlock.java:48-60）。
        let (b, _) = template_boxes(Shape::Trapdoor, 0, BoxUse::Outline);
        assert_eq!(b[0], bx(0.0, 0.0, 0.0, 1.0, 0.1875, 1.0));
        let (b, _) = template_boxes(Shape::Trapdoor, 4, BoxUse::Outline);
        assert_eq!(b[0], bx(0.0, 0.8125, 0.0, 1.0, 1.0, 1.0));
        let (b, _) = template_boxes(Shape::Trapdoor, 0b1001, BoxUse::Outline);
        assert_eq!(b[0], bx(0.0, 0.0, 0.8125, 1.0, 1.0, 1.0));
    }

    // 全表派生一致性由 mcv_core::lib.rs `shape_table_assignments` 覆盖
    // （逐 id 对照 BLOCKS.shape ↔ shape_of_name），此处不重复。
}
