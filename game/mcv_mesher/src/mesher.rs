//! Pure-Rust 体素网格器——`cpp/src/mesher.cpp` 的机制移植（frozen oracle，
//! 任务板 #77 已删除；本文件是被黄金数据锚定的冻结基线）。
//!
//! 各函数注释以 `mesher.cpp:行号` 标注机制出处（引用出处≠复制表达，
//! docs/porting-conventions.md §1/§3：控制流与命名是本仓库自有表达）。
//! 输出是渲染 ABI（28 B 交错顶点 + u32 索引，基线 mesher.cpp:11-13,
//! 105-115；#191 顶点色属性 24→28 重锚），与黄金数据逐字节回归锁定
//! （tests/golden.rs；tint 输入为 None 时除 22..27 字节带外与旧 24B 基线
//! 语义同构，黄金件经 GOLDEN_UPDATE 通道重锚）。
//!
//! **病灶#2 平滑光照层（#153 审计 M9 重建波，LightCoordsUtil.java:61-89）**：
//! 顶点光 = 暴露层四格（两侧 + 对角 + 面中心）均值 + 借中心规则（26.1
//! 原版默认 Smooth Lighting ON；改动前每面单采 = M8 平采 ≈ Smooth OFF）。
//! 环境开关 `MCV_SMOOTH_LIGHT=0` 回落单采（输出与改动前逐位等价，回滚
//! 面），缺省开。黄金件随本层落地重锚（tests/golden/README.md 头注）。
//! 未移植债：非立方面的二级权重混合（`smoothWeightedBlend` 仅
//! facePartial 走 faceShape 权重，BlockModelLighter.java:163-178 的
//! `doNonCubicWeight` 分支）——盒面走与立方面同一 smoothBlend；cross 恒
//! 平采（无暴露层语义）。
//!
//! 顶点布局（28 B，#191 tint 波）：pos f32x3 | uv u16x2 | tex_layer u16 |
//! block_light u8 | sky_light u8 | ao u8 | flags u8（bit0-2 面档，bit3
//! 水顶波动）| pad 2B | tint_rgb u8x3（群系染色，None 场 = 白）| pad 1B；
//! 索引 u32，外视 CCW，每 quad (0,1,2)(0,2,3)。

use std::sync::OnceLock;

use mcv_core::{BlockDef, shape::Shape, tint};

/// 顶点步长（渲染 ABI；#191 tint 波 24→28：偏移 22/23/27 pad=0，
/// 24..27 = tint RGB8。旧基线 mesher.cpp:115 static_assert 24 随顶点色
/// 属性落地作废，黄金件经 CI GOLDEN_UPDATE 通道重锚）。
pub const VERTEX_STRIDE: usize = 28;

/// 未加载邻区块/世界底哨兵，视为不透明（mesher.cpp:24-28：置于 u16 顶端
/// 避免与注册 id 冲突）。
const BARRIER: u16 = 0xFFFF;
/// 体素 u16 打包（mesher.cpp:30-34）：bit0-11 = id，bit12-15 = 状态 nibble。
const ID_MASK: u16 = 0x0FFF;
/// 每格边 UV 单位 = 4096（一格边 = 一整 tile；shader 除 4096.0 得 tile 数，
/// sampler repeat 平铺，mesher.cpp:36-49）。单位是 tile 归一而非 u16 归一：
/// 合并面跨 N 块携带 N*4096，u16 容 N≤15（贪心合并上限 `MAX_MERGE` 由此来
/// 源）。65535/块的首版修法让 ≥2 块合并面溢出 u16，mod-65536 环绕毁掉顶点
/// 间线性插值 → 一 tile 拉伸铺满整个合并面（真机 round 2「马赛克拉伸」）。
const UV_PER_BLOCK: f32 = 4096.0;
/// 单轴贪心合并上限：15 * 4096 = 61440 < 65536（u16 安全）。
const MAX_MERGE: i32 = 15;
/// 水顶面下沉量（mesher.cpp:39）。
const WATER_TOP_SINK: f32 = 0.1;

/// 面档（mesher.cpp:41-48）；face = axis*2 + dir。
const FACE_PY: usize = 2;
const FACE_NY: usize = 3;

/// quad 顶点发射序（mesher.cpp:88-98）：(u,v) 角偏移，序选取得使
/// cross(c1-c0, c3-c0) 指向面外法线（CCW）。
const CORNER_ORDER: [[[i32; 2]; 4]; 6] = [
    // +X
    [[0, 0], [0, 1], [1, 1], [1, 0]],
    // -X
    [[0, 0], [1, 0], [1, 1], [0, 1]],
    // +Y
    [[0, 0], [0, 1], [1, 1], [1, 0]],
    // -Y
    [[0, 0], [1, 0], [1, 1], [0, 1]],
    // +Z
    [[0, 0], [1, 0], [1, 1], [0, 1]],
    // -Z
    [[0, 0], [0, 1], [1, 1], [1, 0]],
];

/// 原版非立方比例（mesher.cpp:379-386，16px 方块 → 0..1）。
const HALF: f32 = 0.5;
const TORCH_MIN: f32 = 0.4; // 火把柱 x/z 0.4..0.6
const TORCH_TOP: f32 = 0.625; // 火把柱 y 0..0.625
const FENCE_POST: f32 = 0.375; // 栅栏柱 x/z 0.375..0.625
const RAIL_MIN: f32 = 0.375; // 臂梁 y 0.375..0.5625
const RAIL_MAX: f32 = 0.5625;

/// 方块信息（mesher.cpp:68-75 BlockInfo）：opaque/liquid/geom/tiles/shape。
/// 数据吃 mcv_core::BLOCKS 生成表（与已删除的 C++ kBlocks 同一次
/// gen-blocks.py 生成，mesher.cpp:77-82），geom 是 C++ 表独有字段，按
/// cpp_geom 规则推导（见 [`block_geom`]）并受 tests/golden.rs 逐 id 对照
/// 黄金生成表锁定。
#[derive(Clone, Copy)]
pub(crate) struct Info {
    pub opaque: bool,
    pub geom: bool,
    pub tiles: [u16; 6], // [+X, -X, +Y, -Y, +Z, -Z]
    pub shape: u8,
}

/// 未注册 id（& 0xFFF 后越界）回退：全不透明（mesher.cpp:170-172 kUnknown，
/// 镜像 mcv_light::opacity 的保守方向）。
static UNKNOWN_INFO: Info = Info {
    opaque: true,
    geom: false,
    tiles: [0; 6],
    shape: 0,
};

/// 原版不可见方块：几何上不渲染（ci/gen-blocks.py INVISIBLE_GEOM，
/// mesher.cpp 生成表 geom=false 的来源注释）。
const INVISIBLE_GEOM: [&str; 6] = [
    "barrier",
    "light",
    "cave_air",
    "void_air",
    "structure_void",
    "bubble_column",
];

// kind==1（非立方模型）且非 solid/liquid/shape 的「占位整盒」名单，
// 静态快照自 cpp/src/blocks_gen.inc 的 geom 列（详见文件头注释）。
include!("geom_placeholder.rs");

/// cpp_geom 规则（ci/gen-blocks.py:498-506）的 Rust 表达：不透明 pass 是否
/// 产出几何。air/water 走水/空 pass；隐形方块 false；非立方模型（kind==1）
/// 中已有形状模板的由 `shape != 0` 覆盖，其余占位整盒（kind==1 的
/// signs/rails/banners/buttons 等无模板形状）按 C++ 表也出几何 → 名单快照；
/// 其余立方实体/其他液体 = solid or liquid。
pub fn block_geom(def: &BlockDef) -> bool {
    if def.name == "air" || def.name == "water" || INVISIBLE_GEOM.contains(&def.name) {
        return false;
    }
    def.solid
        || def.liquid
        || def.shape != Shape::Cube as u8
        || PLACEHOLDER_NONCUBE.contains(&def.name)
}

struct Tables {
    info: Vec<Info>,
    water_id: u16,
}

/// 进程级方块表（1171 项，一次性构建；构建期只读 mcv_core 静态表）。
fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| Tables {
        info: mcv_core::BLOCKS
            .iter()
            .map(|d| Info {
                opaque: d.opaque,
                geom: block_geom(d),
                tiles: d.tiles,
                shape: d.shape,
            })
            .collect(),
        water_id: mcv_core::BLOCKS
            .iter()
            .position(|d| d.name == "water")
            .unwrap_or(5) as u16,
    })
}

/// 注册表查询（mesher.cpp:174-177 block_info）：入参为已掩码 id；
/// 越界回退 [`UNKNOWN_INFO`]。
#[inline]
fn info_of(base: u16) -> &'static Info {
    let t = tables();
    if (base as usize) < t.info.len() {
        &t.info[base as usize]
    } else {
        &UNKNOWN_INFO
    }
}

/// 不透明判定（mesher.cpp:179-182 is_opaque）：哨兵先行；真 id 走表。
#[inline]
fn is_opaque(raw: u16) -> bool {
    raw == BARRIER || info_of(raw & ID_MASK).opaque
}

/// 3x3 邻域（mesher.cpp:100-103 Neighborhood）：行主序 dz 外/dx 内，
/// 中心 = 4；None = 未加载。
#[derive(Clone, Copy)]
pub(crate) struct Nb<'a> {
    pub voxels: [Option<&'a [u16]>; 9],
    pub light: [Option<&'a [u8]>; 9],
}

/// 邻域坐标取方块（mesher.cpp:139-168 block_at）：y < WORLD_MIN_Y(−64) →
/// 哨兵（实心底：底面不渲染、不穿底）；y ≥ WORLD_MAX_Y(320) → 空气
/// （世界顶开放，顶面可见）；x/z 越界折入邻区块，未加载 → 哨兵。
/// v6：y 为**绝对**世界 y，存储索引走 `mcv_core::vidx`。
fn block_at(n: &Nb, x: i32, y: i32, z: i32) -> u16 {
    if y < mcv_core::WORLD_MIN_Y {
        return BARRIER;
    }
    if y >= mcv_core::WORLD_MAX_Y {
        return 0;
    }
    let (mut x, mut z) = (x, z);
    let mut cx = 0i32;
    let mut cz = 0i32;
    if x < 0 {
        cx = -1;
        x += 16;
    } else if x >= 16 {
        cx = 1;
        x -= 16;
    }
    if z < 0 {
        cz = -1;
        z += 16;
    } else if z >= 16 {
        cz = 1;
        z -= 16;
    }
    match n.voxels[((cz + 1) * 3 + (cx + 1)) as usize] {
        None => BARRIER,
        Some(arr) => arr[mcv_core::vidx(x as usize, y, z as usize)],
    }
}

/// 光照采样（mesher.cpp:184-224 light_at）：低 nibble=方块光、高=天空光；
/// 世界上方 sky=15/block=0，下方全 0，缺失数组全 0。返回 (sky, block)。
fn light_at(n: &Nb, x: i32, y: i32, z: i32) -> (u8, u8) {
    if y >= mcv_core::WORLD_MAX_Y {
        return (15, 0);
    }
    if y < mcv_core::WORLD_MIN_Y {
        return (0, 0);
    }
    let (mut x, mut z) = (x, z);
    let mut cx = 0i32;
    let mut cz = 0i32;
    if x < 0 {
        cx = -1;
        x += 16;
    } else if x >= 16 {
        cx = 1;
        x -= 16;
    }
    if z < 0 {
        cz = -1;
        z += 16;
    } else if z >= 16 {
        cz = 1;
        z -= 16;
    }
    match n.light[((cz + 1) * 3 + (cx + 1)) as usize] {
        None => (0, 0),
        Some(arr) => {
            let v = arr[mcv_core::vidx(x as usize, y, z as usize)];
            (v >> 4, v & 0x0F)
        }
    }
}

/// 单角 AO（mesher.cpp:226-241 corner_ao）：s1/s2/corner 是暴露层里贴住
/// 该角的三个格子；s1&&s2 → 0，否则 3-(s1+s2+c)。
// 参数列与 oracle 同构（邻格 + u/v 轴 + 角偏移），语义单元不可再拆。
#[allow(clippy::too_many_arguments)]
fn corner_ao(n: &Nb, nx: i32, ny: i32, nz: i32, u: [i32; 3], v: [i32; 3], a: i32, b: i32) -> u8 {
    let du = if a != 0 { 1 } else { -1 };
    let dv = if b != 0 { 1 } else { -1 };
    let s1 = is_opaque(block_at(n, nx + du * u[0], ny + du * u[1], nz + du * u[2]));
    let s2 = is_opaque(block_at(n, nx + dv * v[0], ny + dv * v[1], nz + dv * v[2]));
    let c = is_opaque(block_at(
        n,
        nx + du * u[0] + dv * v[0],
        ny + du * u[1] + dv * v[1],
        nz + du * u[2] + dv * v[2],
    ));
    if s1 && s2 {
        0
    } else {
        3 - (u8::from(s1) + u8::from(s2) + u8::from(c))
    }
}

/// 病灶#2 环境开关（#153 审计 M9 重建波）：`MCV_SMOOTH_LIGHT=0` 回落每面
/// 单采（M8 平采语义 = vanilla Smooth Lighting OFF，输出与改动前逐位
/// 等价——回滚面）；缺省/其他值 = 开（原版默认语义）。每次 build 读取
/// 一次（不入 OnceLock：测试需在同一进程内切换两分支对拍）。
fn smooth_light_enabled() -> bool {
    std::env::var("MCV_SMOOTH_LIGHT").ok().as_deref() != Some("0")
}

/// 26.1 `LightCoordsUtil.smoothBlend`（LightCoordsUtil.java:61-89）的
/// (sky, block) 双通道表达：中心任一通道 >2 时，零通道邻格借中心值
/// （借中心规则，:62-86）；四格按通道 floor 均值（:88 打包
/// `sum >> 2 & 0x00FF00FF`——通道位不相交，故与逐通道 `>>2` 等价）。
fn smooth_blend(n1: (u8, u8), n2: (u8, u8), n3: (u8, u8), center: (u8, u8)) -> (u8, u8) {
    let mut ns = [n1, n2, n3];
    if center.0 > 2 || center.1 > 2 {
        for nn in &mut ns {
            if nn.0 == 0 {
                nn.0 = center.0;
            }
            if nn.1 == 0 {
                nn.1 = center.1;
            }
        }
    }
    (
        (ns[0].0 + ns[1].0 + ns[2].0 + center.0) >> 2,
        (ns[0].1 + ns[1].1 + ns[2].1 + center.1) >> 2,
    )
}

/// 单角平滑光采样：与 [`corner_ao`] 同一三格几何（该角在暴露层的两侧格
/// 与对角格），blend 中心 = 面平采格 (c_sky, c_blk)。26.1
/// `BlockModelLighter.prepareQuadShape` 立方面分支
/// （BlockModelLighter.java:163-178）：顶点光 = smoothBlend(两侧格,
/// 对角格, lightCenter)，lightCenter 取面朝向格（:115-120）= 本仓既有的
/// `light_at(nx,ny,nz)` 平采点。
// 参数列与 corner_ao/corner_smooth 同构（邻格 + u/v 轴 + 角偏移），不可再拆。
#[allow(clippy::too_many_arguments)]
fn corner_smooth(
    n: &Nb,
    c_sky: u8,
    c_blk: u8,
    nx: i32,
    ny: i32,
    nz: i32,
    u: [i32; 3],
    v: [i32; 3],
    a: i32,
    b: i32,
) -> (u8, u8) {
    let du = if a != 0 { 1 } else { -1 };
    let dv = if b != 0 { 1 } else { -1 };
    let n1 = light_at(n, nx + du * u[0], ny + du * u[1], nz + du * u[2]);
    let n2 = light_at(n, nx + dv * v[0], ny + dv * v[1], nz + dv * v[2]);
    let nc = light_at(
        n,
        nx + du * u[0] + dv * v[0],
        ny + du * u[1] + dv * v[1],
        nz + du * u[2] + dv * v[2],
    );
    smooth_blend(n1, n2, nc, (c_sky, c_blk))
}

/// 扫描轴 → (格 u 数, 格 v 数, 层数)（mesher.cpp:243-256 slice_geom）。
/// axis0: u=z v=y；axis1: u=x v=z；axis2: u=x v=y。
fn slice_geom(axis: usize) -> (usize, usize, usize) {
    if axis == 1 {
        (16, 16, mcv_core::CHUNK_SY)
    } else {
        (16, mcv_core::CHUNK_SY, 16)
    }
}

/// (轴, 层, u, v) → 区块内 (x,y,z)（mesher.cpp:258-273 cell_coords）。
fn cell_coords(axis: usize, layer: i32, u: usize, v: usize) -> (i32, i32, i32) {
    // v6：y 分量 = 层内局部 v/layer + WORLD_MIN_Y（绝对世界 y，与
    // block_at/light_at/顶点位置的绝对域一致）。
    let y0 = v as i32 + mcv_core::WORLD_MIN_Y;
    match axis {
        0 => (layer, y0, u as i32),
        1 => (u as i32, layer + mcv_core::WORLD_MIN_Y, v as i32),
        _ => (u as i32, y0, layer),
    }
}

/// 切片网格 u/v 轴的 3D 单位向量（mesher.cpp:275-287 grid_axes）。
fn grid_axes(axis: usize) -> ([i32; 3], [i32; 3]) {
    match axis {
        0 => ([0, 0, 1], [0, 1, 0]),
        1 => ([1, 0, 0], [0, 0, 1]),
        _ => ([1, 0, 0], [0, 1, 0]),
    }
}

/// 格数 → u16 UV（mesher.cpp uv_coord）：*4096+0.5 截断。调用方保证
/// blocks ≤ MAX_MERGE（贪心）或 ≤1（形状模板），不会溢出 u16。不做
/// mod-wrap：顶点 uv 线性插值下环绕会把跨度折叠成一 tile 拉伸。
/// Rust `as` 截断语义与 C++ static_cast 一致（blocks≥0），f32 逐位同。
#[inline]
fn uv_coord(blocks: f32) -> u16 {
    (blocks * UV_PER_BLOCK + 0.5) as u16
}

/// 切片格（mesher.cpp:117-127 Cell）：ao4 打包 4 角 × 2bit，角序 (b*2+a)；
/// 病灶#2 平滑光照层增补 sl4/bl4（4 角 sky/block 均值，各 4bit/角，同一
/// 角序）——顶点光随角变化后，合并键必须锁全矩形的四角光场。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Cell {
    id: u16,
    tex: u16,
    sky: u8,
    blk: u8,
    ao4: u8,
    sl4: u16,
    bl4: u16,
    wave: u8,
    visible: u8,
}

/// 合并键（mesher.cpp:297-301 same_key）：双方可见且全字段相等。
fn same_key(a: &Cell, b: &Cell) -> bool {
    a.visible != 0
        && b.visible != 0
        && a.id == b.id
        && a.tex == b.tex
        && a.sky == b.sky
        && a.blk == b.blk
        && a.ao4 == b.ao4
        && a.sl4 == b.sl4
        && a.bl4 == b.bl4
        && a.wave == b.wave
}

/// 网格器输出（mesher.cpp:105-115 QuadVertex 平铺）：顶点 24B 步长、索引
/// u32。空网格保持 (0,0) 计数（任务板 #77 前的池容量协商是 C++ 路径的池
/// 细节，随池一并拆除）。
#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    /// #191 群系 tint 场（None = 未装配 colormap/关闭染色 → 顶点色恒白，
    /// 乘算单位元，贴图原样显示）。世界原点方块列由 [`Self::tint_bx0/bz0`]
    /// 给出（= 中心区块世界方块坐标；场覆盖 [原点−R, 原点+16+R]，与
    /// [`mcv_core::tint::ChunkTint`] 的 ox/oz = 原点−R 契约同源）。
    tint: Option<tint::ChunkTint>,
    tint_bx0: i32,
    tint_bz0: i32,
}

impl MeshData {
    /// 顶点字节（28 B 步长交错）。
    pub fn vertex_data(&self) -> &[u8] {
        &self.vertices
    }

    /// 索引（u32，外视 CCW）。
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// (顶点数, 索引数)。
    pub fn counts(&self) -> (u32, u32) {
        (
            (self.vertices.len() / VERTEX_STRIDE) as u32,
            self.indices.len() as u32,
        )
    }

    /// 顶点群系 tint RGB8：按贴图层的 kind + 顶点世界方块列取色。
    /// - 无 tint 场（未加载 colormap / 关闭染色）→ [`TINT_WHITE`]（乘算
    ///   单位元，贴图原样，等价旧「素材缺失禁用」路径）；
    /// - colormap 族（草/叶）→ [`ChunkTint::color_at`] 盒均值（世界坐标，
    ///   顶点本地 x/z + 中心区块世界原点 bx0/bz0）；
    /// - 常量族（云杉/白桦）→ 烤色常数（与网格坐标无关）；
    /// - 不染色层 → [`TINT_WHITE`]。
    fn vertex_tint(&self, tex: u16, x: f32, z: f32) -> [u8; 3] {
        let Some(ct) = self.tint.as_ref() else {
            return tint::TINT_WHITE;
        };
        let kind = tint::tint_kind(tex);
        match kind {
            tint::TINT_GRASS | tint::TINT_FOLIAGE => {
                // 顶点本地 x/z ∈ [0,16]（含右/远边界 16.0——该角同时属于
                // 邻块的 0.0 角，两侧必须取**同一世界列**为混合窗中心，
                // 故 floor 不钳位：16.0 → 世界列 bx0+16，与邻块 0.0 →
                // 同列逐位同色，跨区块零接缝由构造保证）。
                let lx = x.floor() as i32;
                let lz = z.floor() as i32;
                ct.color_at(kind, self.tint_bx0 + lx, self.tint_bz0 + lz)
            }
            tint::TINT_SPRUCE => tint::SPRUCE_LEAF_RGB,
            tint::TINT_BIRCH => tint::BIRCH_LEAF_RGB,
            _ => tint::TINT_WHITE,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_vertex(
        &mut self,
        x: f32,
        y: f32,
        z: f32,
        uu: u16,
        vv: u16,
        tex: u16,
        blk: u8,
        sky: u8,
        ao: u8,
        flags: u8,
    ) {
        // mesher.cpp:303-320 push_vertex 的平铺表达：手写 LE 字节保证布局
        // 与 C++ 结构体逐字节一致（偏移 0/12/16/18/19/20/21，pad=0）。
        // 病灶#2 起 blk/sky 逐顶点取值（平滑层），由调用方按角解包传入；
        // 参数列与 oracle 同构（一条顶点的全部字段），不可再拆。
        // #191：24..27 写群系 tint RGB8（原版 BlockColors 顶点色同位属性，
        // 世界坐标采样跨块连续）；22/23/27 pad 恒 0。
        let tint_rgb = self.vertex_tint(tex, x, z);
        self.vertices.extend_from_slice(&x.to_le_bytes());
        self.vertices.extend_from_slice(&y.to_le_bytes());
        self.vertices.extend_from_slice(&z.to_le_bytes());
        self.vertices.extend_from_slice(&uu.to_le_bytes());
        self.vertices.extend_from_slice(&vv.to_le_bytes());
        self.vertices.extend_from_slice(&tex.to_le_bytes());
        self.vertices.push(blk);
        self.vertices.push(sky);
        self.vertices.push(ao);
        self.vertices.push(flags);
        self.vertices.extend_from_slice(&[0, 0]);
        self.vertices.extend_from_slice(&tint_rgb);
        self.vertices.push(0);
    }

    fn vertex_base(&self) -> u32 {
        (self.vertices.len() / VERTEX_STRIDE) as u32
    }

    fn push_quad_indices(&mut self, base: u32) {
        // mesher.cpp:365-370 / 457-462：外视 CCW，(0,1,2)(0,2,3)。
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// 发射一条合并 quad（mesher.cpp:322-371 emit_quad）：角序按
/// CORNER_ORDER[face]，AO 取锚格打包值（键相等 ⇒ 全矩形相等），水面顶
/// 波动位进 flags bit3 并下沉平面。病灶#2：平滑层顶点光按角取锚格
/// sl4/bl4 打包（键相等 ⇒ 远角格的四角光场与锚格逐位同，几何上大 quad
/// 的远角顶点 = 远格同序角）；`smooth == false` 或水 pass（水取光平采
/// 语义，M8/129-L，FluidRenderer.java:430-432）回落面单采值。
#[allow(clippy::too_many_arguments)]
fn emit_quad(
    out: &mut MeshData,
    water_pass: bool,
    axis: usize,
    dir: usize,
    layer: i32,
    u0: usize,
    v0: usize,
    wq: i32,
    hq: i32,
    anchor: &Cell,
    smooth: bool,
) {
    let face = axis * 2 + dir;
    let mut plane = layer as f32 + if dir == 0 { 1.0 } else { 0.0 };
    if water_pass && face == FACE_PY && anchor.wave != 0 {
        plane -= WATER_TOP_SINK;
    }
    let u_max = uv_coord(wq as f32);
    let v_max = uv_coord(hq as f32);
    let flags = (face as u8) | if anchor.wave != 0 { 0x08 } else { 0x00 };
    let base = out.vertex_base();
    for [a, b] in CORNER_ORDER[face] {
        let uu = (u0 as i32 + a * wq) as f32;
        let vv = (v0 as i32 + b * hq) as f32;
        // v6：顶点 y 为绝对世界 y（层/格局部量 + WORLD_MIN_Y；上传 origin
        // 的 y 分量恒 0，见 RustMesher）。
        let mine = mcv_core::WORLD_MIN_Y as f32;
        let (x, y, z) = match axis {
            0 => (plane, vv + mine, uu),
            1 => (uu, plane + mine, vv),
            _ => (uu, vv + mine, plane),
        };
        let cu = if a != 0 { u_max } else { 0 };
        // 侧面（v 轴=世界 +y）：v=0 必须在顶边——上传行序无翻转，row 0=贴图
        // 顶边（草裙），shader coord.y=0 采 row 0。旧约定 v 随 y 增 → 草裙
        // 落在方块底部（真机 round 2）。顶/底面（v 轴=z）方向不敏感。
        let cv = if axis != 1 {
            if b != 0 { 0 } else { v_max }
        } else if b != 0 {
            v_max
        } else {
            0
        };
        let k = b as u8 * 2 + a as u8;
        let ao = (anchor.ao4 >> (2 * k)) & 0x3;
        let (blk, sky) = if smooth && !water_pass {
            (
                ((anchor.bl4 >> (4 * k)) & 0xF) as u8,
                ((anchor.sl4 >> (4 * k)) & 0xF) as u8,
            )
        } else {
            (anchor.blk, anchor.sky)
        };
        out.push_vertex(x, y, z, cu, cv, anchor.tex, blk, sky, ao, flags);
    }
    out.push_quad_indices(base);
}

/// 方块内局部盒（mesher.cpp:388-391 Box3，0..1 局部坐标）。
#[derive(Clone, Copy)]
struct Box3 {
    x0: f32,
    y0: f32,
    z0: f32,
    x1: f32,
    y1: f32,
    z1: f32,
}

/// 面所在切片网格的 u/v 轴序号（mesher.cpp:393-403 face_uv_axes，与
/// grid_axes 一致；UV 与 AO 采样共用）。
fn face_uv_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (2, 1),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// 非立方形状发射器（mesher.cpp:408 起的形状模板段）：持有邻域与输出，
/// 方法对应 emit_box_face / emit_box / emit_cross / emit_fence /
/// emit_stairs。面剔除、光照、AO、UV 全部复用 Cube 路径函数
/// （mesher.cpp:373-377 段注释语义）。病灶#2：`smooth` 直通 box_face
/// 顶点光（平滑层）；cross 恒平采（植物不遮自身格，无暴露层语义）。
struct ShapeEmitter<'a, 'v> {
    n: &'a Nb<'a>,
    out: &'v mut MeshData,
    x: i32,
    y: i32,
    z: i32,
    smooth: bool,
}

impl ShapeEmitter<'_, '_> {
    /// 发射盒子的一个面（mesher.cpp:405-463 emit_box_face）。expose=true
    /// 强制发射（半砖中层面：邻格是本方块自身格的另一半，不属「不透明
    /// 邻格」判据）。UV 取盒子在格内的实际局部区间，与 Cube 同一 uv_coord
    /// 换算（mesher.cpp:449-450，整盒时与贪心路径逐位一致）；光照与 AO
    /// 采样暴露邻格 (nx,ny,nz)（mesher.cpp:437,451-452）。
    fn box_face(&mut self, info: &Info, b: &Box3, face: usize, expose: bool) {
        let axis = face / 2;
        let step: i32 = if face.is_multiple_of(2) { 1 } else { -1 };
        let nx = self.x + if axis == 0 { step } else { 0 };
        let ny = self.y + if axis == 1 { step } else { 0 };
        let nz = self.z + if axis == 2 { step } else { 0 };
        if !expose && is_opaque(block_at(self.n, nx, ny, nz)) {
            return;
        }
        let bmin = [b.x0, b.y0, b.z0];
        let bmax = [b.x1, b.y1, b.z1];
        let (u_axis, v_axis) = face_uv_axes(axis);
        let unit = |a: usize| {
            [
                if a == 0 { 1 } else { 0 },
                if a == 1 { 1 } else { 0 },
                if a == 2 { 1 } else { 0 },
            ]
        };
        let (u_ax, v_ax) = (unit(u_axis), unit(v_axis));

        let (sky, blk) = light_at(self.n, nx, ny, nz);

        let plane = if face.is_multiple_of(2) {
            bmax[axis]
        } else {
            bmin[axis]
        };
        let flags = face as u8;
        let base = self.out.vertex_base();
        for [a, bq] in CORNER_ORDER[face] {
            let mut pos = [0.0f32; 3];
            pos[axis] = plane;
            pos[u_axis] = if a != 0 { bmax[u_axis] } else { bmin[u_axis] };
            pos[v_axis] = if bq != 0 { bmax[v_axis] } else { bmin[v_axis] };
            let cu = uv_coord(pos[u_axis]);
            // 侧面：盒内 y 镜像，v=0 落在盒顶（约定同 emit_quad）。
            let v_uv_pos = if v_axis == 1 {
                bmax[v_axis] + bmin[v_axis] - pos[v_axis]
            } else {
                pos[v_axis]
            };
            let cv = uv_coord(v_uv_pos);
            let ao = corner_ao(self.n, nx, ny, nz, u_ax, v_ax, a, bq);
            // 病灶#2：非立方盒面顶点光与 Cube 同一平滑采样点（暴露格为
            // blend 中心）；回落 = 面单采。平滑权重二级混合（26.1
            // smoothWeightedBlend 仅 facePartial 走 faceShape 权重）未移植，
            // 登记债（见文件头注）。
            // corner_smooth/light_at 的返回序为 (sky, blk)，与 push_vertex
            // 的 (blk, sky) 入参序相反——此处按名绑定防换位。
            let (vsky, vblk) = if self.smooth {
                corner_smooth(self.n, sky, blk, nx, ny, nz, u_ax, v_ax, a, bq)
            } else {
                (sky, blk)
            };
            self.out.push_vertex(
                pos[0] + self.x as f32,
                pos[1] + self.y as f32,
                pos[2] + self.z as f32,
                cu,
                cv,
                info.tiles[face],
                vblk,
                vsky,
                ao,
                flags,
            );
        }
        self.out.push_quad_indices(base);
    }

    /// 六面发射；force_face >= 0 的面强制暴露（mesher.cpp:465-473 emit_box）。
    fn box_all(&mut self, info: &Info, b: &Box3, force_face: i32) {
        for f in 0..6 {
            self.box_face(info, b, f, f as i32 == force_face);
        }
    }

    /// 十字植物（mesher.cpp:475-513 emit_cross）：两条对角双面 quad，正面、
    /// 反面索引各 6（不透明管线开背面剔除）；ao 恒 3；flags 用 +Y 光照档；
    /// 光照取方块自身所在格（植物不遮挡所在格光照）——病灶#2 起 cross 不
    /// 参与平滑层（无暴露层，无四格语义，恒平采自身格）。
    fn cross(&mut self, info: &Info) {
        let (sky, blk) = light_at(self.n, self.x, self.y, self.z);
        let (fx, fy, fz) = (self.x as f32, self.y as f32, self.z as f32);
        let u_max = uv_coord(1.0);
        let v_max = uv_coord(1.0);
        let flags = FACE_PY as u8;
        // 每条对角线 4 角：底1 底2 顶2 顶1（u 沿对角线、v 沿 y）。
        const DIAGONALS: [[[f32; 3]; 4]; 2] = [
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 0.0],
            ],
            [
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
            ],
        ];
        // 底角（y=0）v=v_max、顶角 v=0：row 0=贴图顶边须落方块顶（同 emit_quad）。
        let cuv = [[0u16, v_max], [u_max, v_max], [u_max, 0], [0, 0]];
        for diag in DIAGONALS {
            let base = self.out.vertex_base();
            for k in 0..4 {
                self.out.push_vertex(
                    fx + diag[k][0],
                    fy + diag[k][1],
                    fz + diag[k][2],
                    cuv[k][0],
                    cuv[k][1],
                    info.tiles[FACE_PY],
                    blk,
                    sky,
                    3,
                    flags,
                );
            }
            // 正面 (0,1,2)(0,2,3) + 反面 (0,2,1)(0,3,2)（mesher.cpp:508-511）。
            let (i0, i1, i2, i3) = (base, base + 1, base + 2, base + 3);
            self.out
                .indices
                .extend_from_slice(&[i0, i1, i2, i0, i2, i3]);
            self.out
                .indices
                .extend_from_slice(&[i0, i2, i1, i0, i3, i2]);
        }
    }

    /// 栅栏臂连接判据（mesher.cpp:515-540 fence_arm_connects）：哨兵/越界
    /// 不连；同栅栏类别（shape==Fence）连；实体不透明整立方（sturdy 近似）
    /// 连。M 波 #138 起谓词唯一出处是 mcv_core::shape::connects（与
    /// engine/mcv_game/src/blockshapes.rs 碰撞臂同一函数，渲染臂与碰撞臂
    /// 一致性由构造保证）；本包装保留 oracle 出处注释与 BARRIER 早退。
    fn arm_connects(&self, nb: u16) -> bool {
        if nb == BARRIER {
            return false;
        }
        mcv_core::shape::connects(nb, Shape::Fence)
    }

    /// 栅栏（mesher.cpp:542-571 emit_fence）：中心立柱全高 + 水平四向臂
    /// （臂梁 y 0.375..0.5625，沿臂向从柱边到格边）；臂端面贴 sturdy 邻块
    /// 一侧由不透明面剔除收尾。
    fn fence(&mut self, info: &Info) {
        let post = Box3 {
            x0: FENCE_POST,
            y0: 0.0,
            z0: FENCE_POST,
            x1: 1.0 - FENCE_POST,
            y1: 1.0,
            z1: 1.0 - FENCE_POST,
        };
        self.box_all(info, &post, -1);
        for d in [[1i32, 0], [-1, 0], [0, 1], [0, -1]] {
            if !self.arm_connects(block_at(self.n, self.x + d[0], self.y, self.z + d[1])) {
                continue;
            }
            let mut arm = Box3 {
                x0: FENCE_POST,
                y0: RAIL_MIN,
                z0: FENCE_POST,
                x1: 1.0 - FENCE_POST,
                y1: RAIL_MAX,
                z1: 1.0 - FENCE_POST,
            };
            if d[0] == 1 {
                arm.x1 = 1.0;
            } else if d[0] == -1 {
                arm.x0 = 0.0;
            } else if d[1] == 1 {
                arm.z1 = 1.0;
            } else {
                arm.z0 = 0.0;
            }
            self.box_all(info, &arm, -1);
        }
    }

    /// 楼梯（mesher.cpp:573-612 emit_stairs）：底座整格宽半高盒 + 踏步
    /// （朝向侧半格、另半高盒），bit2=top 上下翻转。facing 0=+Z 1=-Z 2=+X
    /// 3=-X（与 mcv_core BlockId::state / blockshapes.rs 同一约定，勿按直觉
    /// 写成 +X 优先）。面剔除只用通用「邻格不透明」判据，两盒一律按各自
    /// 暴露面发射，宁多勿漏（盒间共面相对面由背面剔除消化）。
    fn stairs(&mut self, info: &Info, st: u8) {
        let facing = st & 3;
        let flipped = (st & 4) != 0;
        let (base, mut step) = if !flipped {
            (
                Box3 {
                    x0: 0.0,
                    y0: 0.0,
                    z0: 0.0,
                    x1: 1.0,
                    y1: HALF,
                    z1: 1.0,
                },
                Box3 {
                    x0: 0.0,
                    y0: HALF,
                    z0: 0.0,
                    x1: 1.0,
                    y1: 1.0,
                    z1: 1.0,
                },
            )
        } else {
            (
                Box3 {
                    x0: 0.0,
                    y0: HALF,
                    z0: 0.0,
                    x1: 1.0,
                    y1: 1.0,
                    z1: 1.0,
                },
                Box3 {
                    x0: 0.0,
                    y0: 0.0,
                    z0: 0.0,
                    x1: 1.0,
                    y1: HALF,
                    z1: 1.0,
                },
            )
        };
        match facing {
            0 => step.z0 = HALF, // +Z：踏步占 +Z 半格
            1 => step.z1 = HALF, // -Z
            2 => step.x0 = HALF, // +X
            _ => step.x1 = HALF, // -X
        }
        self.box_all(info, &base, -1);
        self.box_all(info, &step, -1);
    }

    /// M 波 #138：6..15 档统一模板发射——盒表唯一出处
    /// mcv_core::shape::template_boxes（渲染=拾取=碰撞同源，引注见该表）。
    /// 栅栏/板/墙的臂盒（表索引 1..5）按邻格连接判定裁剪，判据
    /// mcv_core::shape::connects 与碰撞侧同一函数（#62 对拍由构造保证）。
    /// 每盒六面走与 Cube 相同的暴露面剔除（宁多勿漏，同 emit_stairs）。
    fn template(&mut self, info: &Info, st: u8) {
        let sh = Shape::from_u8(info.shape);
        let (boxes, count) =
            mcv_core::shape::template_boxes(sh, st, mcv_core::shape::BoxUse::Outline);
        let armed = matches!(sh, Shape::Fence | Shape::Pane | Shape::Wall);
        for (i, b) in boxes.iter().enumerate().take(count) {
            if armed && i >= 1 {
                // 表臂序（mcv_core ARM_DIRS）：0=+X 1=-X 2=+Z 3=-Z。
                let (dx, dz) = match i {
                    1 => (1, 0),
                    2 => (-1, 0),
                    3 => (0, 1),
                    _ => (0, -1),
                };
                let nb = block_at(self.n, self.x + dx, self.y, self.z + dz);
                if !mcv_core::shape::connects(nb, sh) {
                    continue;
                }
            }
            self.box_all(
                info,
                &Box3 {
                    x0: b.x0,
                    y0: b.y0,
                    z0: b.z0,
                    x1: b.x1,
                    y1: b.y1,
                    z1: b.z1,
                },
                -1,
            );
        }
    }
}

/// 中心块逐格扫描发射非立方模板（mesher.cpp:614-669 emit_shapes）：仅
/// 不透明 pass 调用；生成表出 shape 0..15（1..5 黄金冻结实现、6..15 走
/// mcv_core::shape::template_boxes 模板，M 波 #138），default 整盒为
/// 防御性回退。
fn emit_shapes(n: &Nb, out: &mut MeshData, smooth: bool) {
    for y in mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y {
        for z in 0..16 {
            for x in 0..16 {
                let raw = block_at(n, x, y, z);
                if raw == BARRIER {
                    continue;
                }
                let info = info_of(raw & ID_MASK);
                if !info.geom || info.shape == 0 {
                    continue;
                }
                let st = ((raw >> 12) & 0xF) as u8;
                let mut em = ShapeEmitter {
                    n,
                    out,
                    x,
                    y,
                    z,
                    smooth,
                };
                match info.shape {
                    1 => em.cross(info),
                    2 => em.box_all(
                        info,
                        &Box3 {
                            x0: TORCH_MIN,
                            y0: 0.0,
                            z0: TORCH_MIN,
                            x1: 1.0 - TORCH_MIN,
                            y1: TORCH_TOP,
                            z1: 1.0 - TORCH_MIN,
                        },
                        -1,
                    ),
                    3 => em.fence(info),
                    4 => {
                        if (st & 1) != 0 {
                            // 上半砖：y 0.5..1，中层面（-Y）永远暴露
                            em.box_all(
                                info,
                                &Box3 {
                                    x0: 0.0,
                                    y0: HALF,
                                    z0: 0.0,
                                    x1: 1.0,
                                    y1: 1.0,
                                    z1: 1.0,
                                },
                                FACE_NY as i32,
                            );
                        } else {
                            // 下半砖：y 0..0.5，中层面（+Y）永远暴露
                            em.box_all(
                                info,
                                &Box3 {
                                    x0: 0.0,
                                    y0: 0.0,
                                    z0: 0.0,
                                    x1: 1.0,
                                    y1: HALF,
                                    z1: 1.0,
                                },
                                FACE_PY as i32,
                            );
                        }
                    }
                    5 => em.stairs(info, st),
                    // M 波 #138：6..15 档（carpet/trapdoor/pane/wall +
                    // fence_gate/door/button/pressure_plate/ladder/snow）
                    // 走 mcv_core::shape::template_boxes 模板（原全盒占位，
                    // 黄金场景不含这批方块，网格字节不受影响）。
                    6..=15 => em.template(info, st),
                    _ => em.box_all(
                        info,
                        &Box3 {
                            x0: 0.0,
                            y0: 0.0,
                            z0: 0.0,
                            x1: 1.0,
                            y1: 1.0,
                            z1: 1.0,
                        },
                        -1,
                    ),
                }
            }
        }
    }
}

/// 标准贪心扫描（mesher.cpp:671-803 build_pass）：每 (轴, 方向, 层) 先沿
/// 格 u 轴扩宽、再沿 v 轴扩高；仅整格 quad 键（方块/贴图/天空/方块光/
/// 打包 AO/波动）全等才合并。贪心仅限 shape==0（Cube）；非立方由
/// emit_shapes 另行发射。
fn build_pass(n: &Nb, water_pass: bool, out: &mut MeshData, t: &Tables, smooth: bool) {
    for axis in 0..3 {
        let (gu, gv, layers) = slice_geom(axis);
        let grid_len = gu * gv;
        let mut cells = vec![Cell::default(); grid_len];
        let mut visited = vec![0u8; grid_len];
        let (u_ax, v_ax) = grid_axes(axis);

        for dir in 0..2 {
            let face = axis * 2 + dir;
            let step: i32 = if dir == 0 { 1 } else { -1 };
            let nx_step = if axis == 0 { step } else { 0 };
            let ny_step = if axis == 1 { step } else { 0 };
            let nz_step = if axis == 2 { step } else { 0 };

            for layer in 0..layers as i32 {
                cells.iter_mut().for_each(|c| *c = Cell::default());

                for v in 0..gv {
                    for u in 0..gu {
                        let (x, y, z) = cell_coords(axis, layer, u, v);
                        let raw = block_at(n, x, y, z);
                        if raw == BARRIER {
                            continue;
                        }
                        let id = raw & ID_MASK;
                        let nx = x + nx_step;
                        let ny = y + ny_step;
                        let nz = z + nz_step;
                        let nb = block_at(n, nx, ny, nz);

                        let mut visible = false;
                        let mut wave = 0u8;
                        if water_pass {
                            // 水 vs 水（或哨兵）不出面（mesher.cpp:716-724）。
                            // 129-L/L2：贴满块遮蔽邻域的面按原版剔除——
                            // FluidRenderer.java:290 `renderCondition &&
                            // !isFaceOccludedByNeighbor(...)`；isFaceOccludedByState
                            //（:36-47）取邻态面遮蔽形：空形（玻璃/叶/冰等
                            // 非遮蔽体）不遮；满块形（本仓 `opaque` ⟺
                            // solidRender 满块 occlusionShape，gen-blocks.py:
                            // 563 / BlockBehaviour.java:512-513）下除 +Y 外
                            // 全遮（底/侧贴石不再发射），+Y 仅流体高度满
                            // 1.0 才遮——可发射顶面高 <1.0（原版 0.8889，
                            // 仓内空气上 0.9 波面），顶面保留。全表无
                            // 「opaque 且非立方」项（部分遮蔽形第三分支
                            // Shapes.blockOccludes 在仓内方块集无实例）。
                            if id == t.water_id && nb < BARRIER && (nb & ID_MASK) != t.water_id {
                                visible = face == FACE_PY || !is_opaque(nb);
                                if visible && face == FACE_PY {
                                    wave = u8::from((nb & ID_MASK) == 0);
                                }
                            }
                        } else {
                            let bi = info_of(id);
                            visible = bi.geom && bi.shape == 0 && !is_opaque(nb);
                        }
                        if !visible {
                            continue;
                        }

                        // 129-L/L1：流体面取光按原版语义——FluidRenderer.java
                        // :430-432 `getLightCoords = max(自身格, 上格)`；:168
                        // 顶面与 :234 侧面（水平循环外一次采样，**不采水平
                        // 邻格**）经此；:204 底面 = getLightCoords(pos.below())
                        // = max(下格, 自身)。修复前采对面格：贴石面采实心
                        // 内部光恒 0 = 黑水皮（129-C §5 首要根因）。
                        let (sky, blk) = if water_pass {
                            let (s0, b0) = light_at(n, x, y, z);
                            let (s1, b1) = if face == FACE_NY {
                                light_at(n, nx, ny, nz)
                            } else {
                                light_at(n, x, y + 1, z)
                            };
                            (s0.max(s1), b0.max(b1))
                        } else {
                            light_at(n, nx, ny, nz)
                        };
                        let mut c = Cell {
                            id,
                            tex: info_of(id).tiles[face],
                            sky,
                            blk,
                            ao4: 0,
                            sl4: 0,
                            bl4: 0,
                            wave,
                            visible: 1,
                        };
                        if water_pass {
                            c.ao4 = 0xFF; // 全角全亮（mesher.cpp:741-742）
                        } else {
                            for b in 0..2i32 {
                                for a in 0..2i32 {
                                    let ao = corner_ao(n, nx, ny, nz, u_ax, v_ax, a, b);
                                    c.ao4 |= ao << (2 * (b * 2 + a));
                                }
                            }
                            // 病灶#2：平滑层四角光场（与 corner_ao 同一角序
                            // (b*2+a) 打包，4bit/通道/角）——合并键锁全四角，
                            // emit_quad 按角解包为逐顶点光。回落（smooth=
                            // false）保持全 0，键与顶点取值均为改动前语义。
                            if smooth {
                                for b in 0..2i32 {
                                    for a in 0..2i32 {
                                        let (s, bl) = corner_smooth(
                                            n, sky, blk, nx, ny, nz, u_ax, v_ax, a, b,
                                        );
                                        let k = (b * 2 + a) as u32;
                                        c.sl4 |= (s as u16) << (4 * k);
                                        c.bl4 |= (bl as u16) << (4 * k);
                                    }
                                }
                            }
                        }
                        cells[v * gu + u] = c;
                    }
                }

                visited.iter_mut().for_each(|f| *f = 0);
                for v in 0..gv {
                    for u in 0..gu {
                        let i = v * gu + u;
                        if visited[i] != 0 || cells[i].visible == 0 {
                            continue;
                        }
                        let key = cells[i];

                        // MAX_MERGE 上限：uv 跨度 = 连长*4096 必须容于 u16
                        //（UV_PER_BLOCK 注释）。
                        let mut wq = 1i32;
                        while u + (wq as usize) < gu && wq < MAX_MERGE {
                            let j = v * gu + (u + wq as usize);
                            if visited[j] != 0 || !same_key(&key, &cells[j]) {
                                break;
                            }
                            wq += 1;
                        }

                        let mut hq = 1i32;
                        let mut grew = true;
                        while v + (hq as usize) < gv && grew && hq < MAX_MERGE {
                            for k in 0..(wq as usize) {
                                let j = (v + (hq as usize)) * gu + u + k;
                                if visited[j] != 0 || !same_key(&key, &cells[j]) {
                                    grew = false;
                                    break;
                                }
                            }
                            if grew {
                                hq += 1;
                            }
                        }

                        for dv in 0..(hq as usize) {
                            for du in 0..(wq as usize) {
                                visited[(v + dv) * gu + u + du] = 1;
                            }
                        }
                        emit_quad(
                            out, water_pass, axis, dir, layer, u, v, wq, hq, &key, smooth,
                        );
                    }
                }
            }
        }
    }
}

/// #191 tint 装配入参：中心区块世界方块原点 + 群系色场（场覆盖
/// `[block_x − R, block_x + 16 + R]`，R = [`tint::BLEND_RADIUS`]；构建方
/// 契约，见 [`tint::ChunkTint`]）。
#[derive(Debug)]
pub struct TintInput {
    /// 中心区块世界方块 x（= chunk_x × 16）。
    pub block_x: i32,
    /// 中心区块世界方块 z（= chunk_z × 16）。
    pub block_z: i32,
    pub tint: tint::ChunkTint,
}

/// 入口（mesher.cpp:814-861 mcv_mesh_build 的纯 Rust 等价，去池化）：先
/// 贪心 pass，不透明 pass 追加非立方形状模板（mesher.cpp:829-833）。
/// 邻域缺格/越界语义与 C++ 一致（block_at/light_at，见各函数出处）。
/// `tint`：群系染色场（仅不透明 pass 消费；水 pass 恒按原版流体常数色
/// 语义在着色器侧乘 WATER_TINT，顶点色写白）。
pub fn build_mesh(
    voxels: &[Option<&[u16]>; 9],
    lights: &[Option<&[u8]>; 9],
    kind: u32,
    tint_in: Option<TintInput>,
) -> MeshData {
    let n = Nb {
        voxels: *voxels,
        light: *lights,
    };
    // 病灶#2：顶点光走平滑光照层（原版默认语义）；`MCV_SMOOTH_LIGHT=0`
    // 回落每面单采（M8 平采语义，回滚面）。
    let smooth = smooth_light_enabled();
    let mut out = MeshData::default();
    if kind == 0 {
        if let Some(TintInput {
            block_x,
            block_z,
            tint,
        }) = tint_in
        {
            out.tint = Some(tint);
            out.tint_bx0 = block_x;
            out.tint_bz0 = block_z;
        }
    }
    build_pass(&n, kind == 1, &mut out, tables(), smooth);
    if kind == 0 {
        emit_shapes(&n, &mut out, smooth);
    }
    out
}
