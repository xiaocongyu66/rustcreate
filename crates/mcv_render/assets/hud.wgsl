// HUD: orthographic pixel-space quads; font glyphs + terrain tile icons +
// solid tinted rects (via reserved solid-white font cell).

struct HudUniforms {
    screen: vec4<f32>, // x = width px, y = height px, z = time, w = free
};

@group(0) @binding(0) var<uniform> hud: HudUniforms;
@group(0) @binding(1) var font_tex: texture_2d<f32>;
@group(0) @binding(2) var hud_samp: sampler;
@group(0) @binding(3) var terrain_tex: texture_2d_array<f32>;

struct VtxIn {
    @location(0) pos: vec2<f32>, // pixels, top-left origin
    @location(1) uv: vec2<f32>,  // 0..1 inside source cell
    @location(2) color: vec4<f32>,
    @location(3) src: vec2<u32>, // x = texture id (0 font, 1 terrain), y = layer
};

struct VtxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) src: vec2<u32>,
};

@vertex
fn vs_hud(v: VtxIn) -> VtxOut {
    var out: VtxOut;
    let w = hud.screen.x;
    let h = hud.screen.y;
    let ndc = vec2<f32>(v.pos.x / w * 2.0 - 1.0, 1.0 - v.pos.y / h * 2.0);
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    out.src = v.src;
    return out;
}

@fragment
fn fs_hud(v: VtxOut) -> @location(0) vec4<f32> {
    var texel: vec4<f32>;
    if (v.src.x == 0u) {
        texel = textureSample(font_tex, hud_samp, v.uv);
    } else {
        texel = textureSample(terrain_tex, hud_samp, v.uv, v.src.y);
    }
    if (texel.a < 0.05) {
        discard;
    }
    // Font texture is white; tint colors everything. Icons keep full tint.
    return vec4<f32>(texel.rgb * v.color.rgb, texel.a * v.color.a);
}
