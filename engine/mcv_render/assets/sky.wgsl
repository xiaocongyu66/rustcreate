// Sky: fullscreen triangle; day/night gradient, sun/moon textured quads, hashed stars.

struct SkyUniforms {
    inv_view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,   // w = time
    sun_dir_day: vec4<f32>, // xyz = sun dir, w = day factor
    fog_horizon: vec4<f32>, // xyz = horizon color, w = free
    // 天体参数：x = 月相序（MoonPhase 序 0..7）、yzw = 自由（旧「素材缺失
    // → 程序化圆盘回退」开关已随素材红线删除——日月只画原版贴图 quad）。
    celestials: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sky: SkyUniforms;
@group(0) @binding(1) var celestial_tex: texture_2d_array<f32>;
@group(0) @binding(2) var celestial_samp: sampler;

struct VtxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> VtxOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VtxOut;
    out.clip = vec4<f32>(p[vi], 0.0, 1.0);
    out.ndc = p[vi];
    return out;
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = floor(p);
    let h = dot(q, vec3<f32>(127.1, 311.7, 74.7));
    return fract(sin(h) * 43758.5453);
}

// 原版天体几何解析化：贴图 quad 挂在过原点、法线为天体方向 s 的平面上，
// 距离 100（SkyRenderer.java:343-344 translate(0,100,0)），quad 在平面内
// [-extent, extent]^2，UV 0..1 铺满（buildCelestialQuad :134-140 的
// (-1,0,-1)..(1,0,1) 归一坐标 × scale）。返回纹理 uv；无命中返回 z<0。
fn quad_hit(dir: vec3<f32>, s: vec3<f32>, extent: f32, u_axis: vec3<f32>, v_axis: vec3<f32>) -> vec3<f32> {
    let d = dot(dir, s);
    if (d <= 0.0001) {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    // 视线与平面交点的平面内坐标。
    let p = dir * (100.0 / d);
    let x = dot(p, u_axis);
    let y = dot(p, v_axis);
    if (abs(x) > extent || abs(y) > extent) {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    return vec3<f32>(
        (x + extent) / (2.0 * extent),
        (y + extent) / (2.0 * extent),
        1.0,
    );
}

@fragment
fn fs_sky(v: VtxOut) -> @location(0) vec4<f32> {
    // Unproject far plane point to get the view ray.
    let far_pt = sky.inv_view_proj * vec4<f32>(v.ndc, 1.0, 1.0);
    let dir = normalize(far_pt.xyz / far_pt.w - sky.cam_pos.xyz);

    let day = sky.sun_dir_day.w;
    let up = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);

    // Day sky: zenith blue -> horizon pale; night: near-black blue.
    let day_top = vec3<f32>(0.32, 0.55, 0.95);
    let day_hor = vec3<f32>(0.62, 0.76, 0.95);
    let night_top = vec3<f32>(0.008, 0.012, 0.035);
    let night_hor = vec3<f32>(0.02, 0.03, 0.08);
    let top = mix(night_top, day_top, day);
    let hor = mix(night_hor, day_hor, day);
    var color = mix(hor, top, pow(up, 0.7));

    // Sun / moon：原版是贴图 quad（SkyRenderer.java:125-127 太阳、:149-157
    // 八相月亮，月亮 scale 20、太阳 scale 30、距离 100）。quad 平面基由
    // RotY(-90)·RotX(angle) 展开：太阳 U 轴（局部 +X）= 世界 +Z（南）、
    // V 轴（局部 +Z）= (-s.y, s.x, 0)；月亮路径同面反向（angle+180°），
    // 且 buildMoonPhases 的 UV 相对太阳横竖双镜像（:151-154 顶点 u0/u1、
    // v0/v1 对调）。U 轴对带 z 倾斜的 s 做 Gram-Schmidt 正交化保朝向稳定。
    let s = normalize(sky.sun_dir_day.xyz);
    let u_axis = normalize(vec3<f32>(0.0, 0.0, 1.0) - s * s.z);
    let v_axis = cross(u_axis, s);
    // 原版 CELESTIAL 管线混合 = BlendFunction.OVERLAY（RenderPipelines.java:643
    // → BlendFunction.java:9）：dst.rgb += src.rgb * src.a（加色）。sun.png /
    // moon/*.png 全图 alpha=1、四周暗色——加色下暗底≈无贡献，只亮出核心。
    // 素材缺失时纹理数组为全透明（无程序化圆盘回退）：加色 0，天空无天体。
    let sun_hit = quad_hit(dir, s, 15.0, u_axis, v_axis);
    if (sun_hit.z > 0.0) {
        // 太阳 quad 顶色白、alpha=rainBrightness（:347），本引擎无雨 → 1。
        let tex = textureSampleLevel(celestial_tex, celestial_samp, sun_hit.xy, 0, 0.0);
        color += tex.rgb * tex.a;
    }
    let m = -s;
    let moon_hit = quad_hit(dir, m, 10.0, u_axis, -v_axis);
    if (moon_hit.z > 0.0) {
        // 月亮 quad UV 双镜像（buildMoonPhases :151-154）+ 相位层选择
        // （baseVertex = moonPhase.index() * 4，:370-372）。
        let uv = vec2<f32>(1.0 - moon_hit.x, 1.0 - moon_hit.y);
        let phase = floor(sky.celestials.x + 0.5);
        let tex = textureSampleLevel(celestial_tex, celestial_samp, uv, i32(1.0 + phase), 0.0);
        color += tex.rgb * tex.a;
    }

    // Stars at night: hash lattice on the ray direction. 原版星星本身即
    // 程序生成（buildStars 种子 10842L 随机 1500 quad，SkyRenderer.java:182+），
    // 哈希点阵为等价程序化实现，非占位。
    let star_grid = floor(dir * 220.0);
    let star = step(0.9992, hash3(star_grid)) * (1.0 - day) * step(0.05, dir.y);
    color = mix(color, vec3<f32>(0.9, 0.92, 1.0), star * 0.8);

    return vec4<f32>(color, 1.0);
}
