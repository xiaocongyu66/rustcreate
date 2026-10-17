// sprite2d：instanced 正交 2D 精灵批次（#222 批 A，bevy_sprite 对等通路）。
// - 正交像素→NDC 映射与 hud.wgsl vs_hud 逐字同式；
// - 图块采样走 MANIFEST_LAYERS 方块图集数组（gpu.rs atlas_arrays_wgsl 烘入
//   SPRITE2D_TERRAIN_ARRAYS 占位行，与 terrain/hud 同源生成，永不错位）；
// - 半透明混合、a<0.05 discard、输出乘 tint 与 fs_hud 逐字同式 —— 接线面
//   （HUD 方块物品图标）像素等价；
// - 本文件为**新增**文件（「新 WGSL 只加不改」红线）；占位行由 gpu.rs 替换，
//   勿手改生成段。

struct SpriteUniforms {
    screen: vec4<f32>, // x = width px, y = height px, z = time, w = free
};

@group(0) @binding(0) var<uniform> spr: SpriteUniforms;
// 图集数组声明 + sample_terrain_sprite(uv, layer) 由 gpu.rs 生成，占位独占整行。
// @@SPRITE2D_TERRAIN_ARRAYS@@
@group(0) @binding(5) var spr_samp: sampler;

struct VtxIn {
    // ---- per-instance（buffer slot 0，step_mode: instance）----
    @location(0) s_pos: vec2<f32>,    // 锚点屏幕坐标（px，左上原点）
    @location(1) s_size: vec2<f32>,   // 尺寸（px）
    @location(2) s_anchor: vec2<f32>, // bevy Anchor 模式：归一化枢轴（y 向下帧）
    @location(3) s_rot: f32,          // 绕锚点旋转（弧度）
    @location(4) s_layer: u32,        // 方块图集数组层号
    @location(5) s_tint: vec4<f32>,   // RGBA 乘色
    // ---- 静态单位 quad（buffer slot 1，step_mode: vertex）----
    @location(6) qcorner: vec2<f32>, // ±0.5（TL,TR,BR,BL 序，y 向下）
    @location(7) quv: vec2<f32>,     // 0/1 角 UV
};

struct VtxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) layer: u32,
};

@vertex
fn vs_sprite2d(v: VtxIn) -> VtxOut {
    var out: VtxOut;
    // 角点 = 锚点 + 旋转((c − anchor) ⊙ size)——Anchor 偏移与 flatten_hud
    // 的「中心 + 旋转(±half)」在 anchor=CENTER 或 rot=0 时逐顶点重合。
    let local = (v.qcorner - v.s_anchor) * v.s_size;
    let c = cos(v.s_rot);
    let s = sin(v.s_rot);
    let world = v.s_pos + vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    let w = spr.screen.x;
    let h = spr.screen.y;
    let ndc = vec2<f32>(world.x / w * 2.0 - 1.0, 1.0 - world.y / h * 2.0);
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = v.quv;
    out.color = v.s_tint;
    out.layer = v.s_layer;
    return out;
}

@fragment
fn fs_sprite2d(v: VtxOut) -> @location(0) vec4<f32> {
    let texel = sample_terrain_sprite(v.uv, v.layer);
    if (texel.a < 0.05) {
        discard;
    }
    // 与 fs_hud 图标分支同式：图集保留全 tint 乘算。
    return vec4<f32>(texel.rgb * v.color.rgb, texel.a * v.color.a);
}
