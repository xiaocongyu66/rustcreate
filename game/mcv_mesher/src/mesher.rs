//! Pure-Rust 体素网格器——`cpp/src/mesher.cpp` 的机制移植（oracle）。
//!
//! 各函数注释以 `mesher.cpp:行号` 标注机制出处（引用出处≠复制表达，
//! docs/porting-conventions.md §1/§3：控制流与命名是本仓库自有表达）。
//! 输出是渲染 ABI（24 B 交错顶点 + u32 索引，mesher.cpp:11-13,105-115），
//! 与 C++ 路径逐字节对拍锁定（tests/parity.rs；对拍不过修本侧，禁改 C++）。
//!
//! 顶点布局（24 B）：pos f32x3 | uv u16x2 | tex_layer u16 | block_light u8 |
//! sky_light u8 | ao u8 | flags u8（bit0-2 面档，bit3 水顶波动）| pad 2B；
//! 索引 u32，外视 CCW，每 quad (0,1,2)(0,2,3)。

use std::sync::OnceLock;

use mcv_core::{BlockDef, shape::Shape};

/// 顶点步长（渲染 ABI；mesher.cpp:115 static_assert 24）。
pub const VERTEX_STRIDE: usize = 24;

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
/// 数据吃 mcv_core::BLOCKS 生成表（与 C++ kBlocks 同一次 gen-blocks.py 生成，
/// mesher.cpp:77-82），geom 是 C++ 表独有字段，按 cpp_geom 规则推导
/// （见 [`block_geom`]）并受 tests/parity.rs 逐 id 对照 C++ 表锁定。
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

/// 邻域坐标取方块（mesher.cpp:139-168 block_at）：y<0 → 哨兵（实心底），
/// y>=256 → 空气（世界顶开放）；x/z 越界折入邻区块，未加载 → 哨兵。
fn block_at(n: &Nb, x: i32, y: i32, z: i32) -> u16 {
    if y < 0 {
        return BARRIER;
    }
    if y >= 256 {
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
        Some(arr) => arr[((y << 8) | (z << 4) | x) as usize],
    }
}

/// 光照采样（mesher.cpp:184-224 light_at）：低 nibble=方块光、高=天空光；
/// 世界上方 sky=15/block=0，下方全 0，缺失数组全 0。返回 (sky, block)。
fn light_at(n: &Nb, x: i32, y: i32, z: i32) -> (u8, u8) {
    if y >= 256 {
        return (15, 0);
    }
    if y < 0 {
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
            let v = arr[((y << 8) | (z << 4) | x) as usize];
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

/// 扫描轴 → (格 u 数, 格 v 数, 层数)（mesher.cpp:243-256 slice_geom）。
/// axis0: u=z v=y；axis1: u=x v=z；axis2: u=x v=y。
fn slice_geom(axis: usize) -> (usize, usize, usize) {
    if axis == 1 {
        (16, 16, 256)
    } else {
        (16, 256, 16)
    }
}

/// (轴, 层, u, v) → 区块内 (x,y,z)（mesher.cpp:258-273 cell_coords）。
fn cell_coords(axis: usize, layer: i32, u: usize, v: usize) -> (i32, i32, i32) {
    match axis {
        0 => (layer, v as i32, u as i32),
        1 => (u as i32, layer, v as i32),
        _ => (u as i32, v as i32, layer),
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

/// 切片格（mesher.cpp:117-127 Cell）：ao4 打包 4 角 × 2bit，角序 (b*2+a)。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Cell {
    id: u16,
    tex: u16,
    sky: u8,
    blk: u8,
    ao4: u8,
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
        && a.wave == b.wave
}

/// 网格器输出（mesher.cpp:105-115 QuadVertex 平铺）：顶点 24B 步长、索引
/// u32。空网格保持 (0,0) 计数（池容量协商是 C++ 路径的池细节，不属 ABI）。
#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
}

impl MeshData {
    #[allow(clippy::too_many_arguments)]
    fn push_vertex(
        &mut self,
        x: f32,
        y: f32,
        z: f32,
        uu: u16,
        vv: u16,
        c: &Cell,
        ao: u8,
        flags: u8,
    ) {
        // mesher.cpp:303-320 push_vertex 的平铺表达：手写 LE 字节保证布局
        // 与 C++ 结构体逐字节一致（偏移 0/12/16/18/19/20/21，pad=0）。
        // 参数列与 oracle 同构（一条顶点的全部字段），不可再拆。
        self.vertices.extend_from_slice(&x.to_le_bytes());
        self.vertices.extend_from_slice(&y.to_le_bytes());
        self.vertices.extend_from_slice(&z.to_le_bytes());
        self.vertices.extend_from_slice(&uu.to_le_bytes());
        self.vertices.extend_from_slice(&vv.to_le_bytes());
        self.vertices.extend_from_slice(&c.tex.to_le_bytes());
        self.vertices.push(c.blk);
        self.vertices.push(c.sky);
        self.vertices.push(ao);
        self.vertices.push(flags);
        self.vertices.extend_from_slice(&[0, 0]);
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
/// 波动位进 flags bit3 并下沉平面。
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
        let (x, y, z) = match axis {
            0 => (plane, vv, uu),
            1 => (uu, plane, vv),
            _ => (uu, vv, plane),
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
        let ao = (anchor.ao4 >> (2 * (b as u8 * 2 + a as u8))) & 0x3;
        out.push_vertex(x, y, z, cu, cv, anchor, ao, flags);
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
/// （mesher.cpp:373-377 段注释语义）。
struct ShapeEmitter<'a, 'v> {
    n: &'a Nb<'a>,
    out: &'v mut MeshData,
    x: i32,
    y: i32,
    z: i32,
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
        let c = Cell {
            id: 0,
            tex: info.tiles[face],
            sky,
            blk,
            ao4: 0,
            wave: 0,
            visible: 1,
        };

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
            self.out.push_vertex(
                pos[0] + self.x as f32,
                pos[1] + self.y as f32,
                pos[2] + self.z as f32,
                cu,
                cv,
                &c,
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
    /// 光照取方块自身所在格（植物不遮挡所在格光照）。
    fn cross(&mut self, info: &Info) {
        let (sky, blk) = light_at(self.n, self.x, self.y, self.z);
        let c = Cell {
            id: 0,
            tex: info.tiles[FACE_PY],
            sky,
            blk,
            ao4: 0,
            wave: 0,
            visible: 1,
        };
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
                    &c,
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
    /// 连。KNOWN-DIVERGENCE 与 C++ 侧同（无 tag/blockstate 体系，见
    /// mesher.cpp:519-530），且 engine/mcv_game/src/blockshapes.rs::
    /// fence_connects 是同规则的第三份（渲染臂与碰撞臂一致性）。
    fn arm_connects(&self, nb: u16) -> bool {
        if nb == BARRIER {
            return false;
        }
        let i = info_of(nb & ID_MASK);
        i.shape == Shape::Fence as u8 || (i.shape == Shape::Cube as u8 && i.opaque)
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
}

/// 中心块逐格扫描发射非立方模板（mesher.cpp:614-669 emit_shapes）：仅
/// 不透明 pass 调用；生成表只出 shape 0..5，default 整盒为防御性回退。
fn emit_shapes(n: &Nb, out: &mut MeshData) {
    for y in 0..256 {
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
                let mut em = ShapeEmitter { n, out, x, y, z };
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
fn build_pass(n: &Nb, water_pass: bool, out: &mut MeshData, t: &Tables) {
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
                            // 水 vs 水（或哨兵）不出面（mesher.cpp:716-724）
                            if id == t.water_id && nb < BARRIER && (nb & ID_MASK) != t.water_id {
                                visible = true;
                                if face == FACE_PY {
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

                        let (sky, blk) = light_at(n, nx, ny, nz);
                        let mut c = Cell {
                            id,
                            tex: info_of(id).tiles[face],
                            sky,
                            blk,
                            ao4: 0,
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
                        emit_quad(out, water_pass, axis, dir, layer, u, v, wq, hq, &key);
                    }
                }
            }
        }
    }
}

/// 入口（mesher.cpp:814-861 mcv_mesh_build 的纯 Rust 等价，去池化）：先
/// 贪心 pass，不透明 pass 追加非立方形状模板（mesher.cpp:829-833）。
/// 邻域缺格/越界语义与 C++ 一致（block_at/light_at，见各函数出处）。
pub fn build_mesh(
    voxels: &[Option<&[u16]>; 9],
    lights: &[Option<&[u8]>; 9],
    kind: u32,
) -> MeshData {
    let n = Nb {
        voxels: *voxels,
        light: *lights,
    };
    let mut out = MeshData::default();
    build_pass(&n, kind == 1, &mut out, tables());
    if kind == 0 {
        emit_shapes(&n, &mut out);
    }
    out
}
