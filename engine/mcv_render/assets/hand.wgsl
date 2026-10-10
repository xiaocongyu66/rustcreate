// 第一人称手持物品图标 quad：独立小管线。顶点为世界空间坐标（CPU 侧由
// hand.rs 摆位），uniform 只带 view_proj；图标采 GUI 精灵表（原版物品
// png）。cutout 语义（alpha discard），不写深度（图标在手 pass 内最后画）。

struct HandUniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> hu: HandUniforms;
@group(0) @binding(1) var icon_tex: texture_2d<f32>;
@group(0) @binding(2) var icon_samp: sampler;

struct VtxIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,    // unorm8x2 → 0..1（精灵表空间）
    @location(2) pmeta: vec2<u32>, // 未用（与 player 顶点布局复用）
};

struct VtxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_hand_icon(v: VtxIn) -> VtxOut {
    var out: VtxOut;
    out.clip = hu.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    return out;
}

@fragment
fn fs_hand_icon(v: VtxOut) -> @location(0) vec4<f32> {
    let c = textureSample(icon_tex, icon_samp, v.uv);
    if (c.a < 0.1) {
        discard;
    }
    return vec4<f32>(c.rgb, c.a);
}
