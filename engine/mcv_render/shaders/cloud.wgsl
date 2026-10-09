// 体素云顶点/片元着色器 —— 机制翻译自 MC 26.1
// assets/minecraft/shaders/core/rendertype_clouds.{vsh,fsh}（见 mc-ref/NOTES-clouds.md）。
// 非原文复制：isamplerBuffer 面表改为实例属性 u32；动态 uniform 矩阵改为传入的 view_proj。

struct CloudUniform {
    view_proj : mat4x4<f32>,   // 视图*投影
    color     : vec4<f32>,     // CloudColor（CLOUD_COLOR 环境属性；本引擎用白色×不透明度）
    offset    : vec3<f32>,     // CloudOffset：(-xInCell, bottomY-camY, -zInCell)
    _pad0     : f32,
    cell_size : vec3<f32>,     // (12, 4, 12) × scale
    fog_end   : f32,           // FogCloudsEnd = 云距离（方块）
};

struct VsOut {
    @builtin(position) clip : vec4<f32>,
    @location(0) color : vec4<f32>,
    @location(1) dist : f32,
};

const FLAG_MASK_DIR : u32 = 7u;
const FLAG_INSIDE_FACE : u32 = 16u; // 内壁面（相机穿云），反转绕序
const FLAG_USE_TOP_COLOR : u32 = 32u; // FAST 模式底面用顶色

// 面方向编号同 MC Direction.values()：DOWN UP NORTH SOUTH WEST EAST
// 每面 4 角点（0..1 局部坐标），与 26.1 vsh 的 vertices 表一致。
const CORNERS : array<vec3<f32>, 24> = array<vec3<f32>, 24>(
    // Bottom face
    vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 0.0),
    // Top face
    vec3(0.0, 1.0, 0.0), vec3(0.0, 1.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(1.0, 1.0, 0.0),
    // North face
    vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0),
    // South face
    vec3(1.0, 0.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(0.0, 1.0, 1.0), vec3(0.0, 0.0, 1.0),
    // West face
    vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 1.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 0.0),
    // East face
    vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(1.0, 1.0, 1.0), vec3(1.0, 0.0, 1.0),
);

// 面固有明暗（26.1 vsh faceColors）：底 0.7 / 顶 1.0 / 南北 0.8 / 西东 0.9
const FACE_COLORS : array<f32, 6> = array<f32, 6>(0.7, 1.0, 0.8, 0.8, 0.9, 0.9);

@group(0) @binding(0) var<uniform> u : CloudUniform;

// 每实例一个面（u32 顶点属性，步进=instance）：
// bit23..16 = (x>>1) 的 8bit，bit15..8 = (z>>1) 的 8bit，
// bit7..0 = dir | flags（含 EXTRA_X/EXTRA_Z 的低位补位），对应 26.1 encodeFace。
fn s8(v : u32) -> i32 {
    // 8bit 符号扩展
    return bitcast<i32>(v << 24u) >> 24;
}

@vertex
fn vs_clouds(
    @builtin(vertex_index) quad_vertex : u32,
    @location(0) face_data : u32,
) -> VsOut {
    let packed = face_data;
    let flags = packed & 0xFFu;
    var cell_x = s8((packed >> 16u) & 0xFFu);
    var cell_z = s8((packed >> 8u) & 0xFFu);
    let direction = flags & FLAG_MASK_DIR;
    let is_inside_face = (flags & FLAG_INSIDE_FACE) != 0u;
    let use_top_color = (flags & FLAG_USE_TOP_COLOR) != 0u;
    // x = (x>>1)<<1 | EXTRA_X(bit7)；z 的补位在 bit6。
    // 位段是 u32、cell 是 s8 扩出的 i32，naga 拒绝混合符号性位或——显式转 i32。
    cell_x = (cell_x << 1) | i32((flags >> 7u) & 1u);
    cell_z = (cell_z << 1) | i32((flags >> 6u) & 1u);

    // 内壁面反转绕序（26.1: vertices[dir*4 + (inside ? 3-q : q)]）
    let q = select(quad_vertex, 3u - quad_vertex, is_inside_face);
    let face_vertex = CORNERS[direction * 4u + q];
    let pos = face_vertex * u.cell_size
        + vec3<f32>(f32(cell_x), 0.0, f32(cell_z)) * u.cell_size
        + u.offset;

    var out : VsOut;
    out.clip = u.view_proj * vec4<f32>(pos, 1.0);
    let shade = select(FACE_COLORS[direction], FACE_COLORS[1], use_top_color);
    out.color = vec4<f32>(u.color.rgb * shade, u.color.a);
    // fog_spherical_distance：pos 已是相机相对空间（offset 含 -camXZ 余量与
    // bottomY-camY，对应 26.1 ModelViewMat 空间），故球面距离 = |pos|。
    out.dist = length(pos);
    return out;
}

@fragment
fn fs_clouds(in : VsOut) -> @location(0) vec4<f32> {
    var color = in.color;
    // 26.1 fsh: color.a *= 1.0 - linear_fog_value(d, 0, FogCloudsEnd)
    color.a *= 1.0 - clamp(in.dist / max(u.fog_end, 1e-4), 0.0, 1.0);
    return color;
}
