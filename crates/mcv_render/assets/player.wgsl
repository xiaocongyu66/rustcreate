// 玩家模型：独立管线。顶点局部坐标（脚底原点、pivot 相对），模型矩阵按
// 部位索引（meta.y）取自 uniform 数组；皮肤为 texture_2d_array（0=steve
// 1=alex），层号取 meta.x。自发光（无光照/无雾）：MC 第三人称玩家不受
// 区块光照影响（entity 渲染无 AO/雾），与任务选项 b 的语义一致但改动更小。

struct PlayerUniforms {
    view_proj: mat4x4<f32>,
    models: array<mat4x4<f32>, 12>,
};

@group(0) @binding(0) var<uniform> pu: PlayerUniforms;
@group(0) @binding(1) var skin_tex: texture_2d_array<f32>;
@group(0) @binding(2) var skin_samp: sampler;

struct VtxIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,      // unorm8x2
    @location(2) meta: vec2<u32>,    // x = 皮肤层, y = 部位索引
};

struct VtxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
};

@vertex
fn vs_player(v: VtxIn) -> VtxOut {
    var out: VtxOut;
    let model = pu.models[v.meta.y];
    out.clip = pu.view_proj * model * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.layer = v.meta.x;
    return out;
}

@fragment
fn fs_player(v: VtxOut) -> @location(0) vec4<f32> {
    // 第二层盒体的空像素（透明）直接丢弃；MC 皮肤为 cutout 语义。
    let c = textureSample(skin_tex, skin_samp, v.uv, v.layer);
    if (c.a < 0.1) {
        discard;
    }
    return c;
}
