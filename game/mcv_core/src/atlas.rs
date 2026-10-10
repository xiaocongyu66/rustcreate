//! Terrain texture array: layers `0..MANIFEST_LAYERS-1` 来自 tiles_manifest.json
//! （字典序；运行时从资源根 `textures/block/*.png` 读盘，827 张真实贴图 +
//! 1 层 missing 哨兵），特殊层（挖掘裂纹）排在 `CRACK_BASE..`。
//!
//! 素材红线（2026-10 任务 #53）：**本 crate 不产生任何程序化假贴图**。
//! 旧版按贴图名逐层画噪声的 LEGACY_RECIPES 回退已全部删除——真实贴图
//! 缺失时该层保留原版 missing 贴图（[`fill_missing_marker`]，与
//! `MissingTextureAtlasSprite.generateMissingImage` 逐像素一致），调用方
//! 以 `log::error!` 显式报错，绝不伪装成真实贴图。裂纹层只读原版
//! `textures/block/destroy_stage_0..9.png`，缺失即为 missing 标记。

use crate::tiles;
use std::path::{Path, PathBuf};

pub const TILE_PX: usize = 16;
/// manifest 层数 = 827 张真实方块贴图（层 0..826，字典序）+ 1 层 missing
/// 哨兵（层 [`SENTINEL_LAYER`]，与 tiles_manifest.json 的
/// tile_index_to_file 长度一致）。
pub const MANIFEST_LAYERS: usize = 828;
/// 827 张真实方块贴图（不含哨兵）。完整性断言以此为准。
pub const REAL_TILE_COUNT: usize = MANIFEST_LAYERS - 1;
/// missing 哨兵层号（manifest 最后一项 `missing_no_texture`，无对应文件）。
/// 模型无贴图解析的方块（air/barrier/light/structure_void 等，见
/// ci/gen-blocks.py `tile_id`）指向该层，永远显示原版 missingno 品红标记。
pub const SENTINEL_LAYER: usize = MANIFEST_LAYERS - 1;
/// 哨兵贴图名（与 ci/gen-blocks.py SENTINEL_NAME 一致）。
pub const SENTINEL_TILE: &str = "missing_no_texture";
/// 挖掘裂纹叠加层数（10 档）。原版 26.1 为 destroy_stage_0..9 十张
/// （jar MANIFEST 名录），档位映射 `(int)(destroyProgress * 10)`（
/// MultiPlayerGameMode.java:551）。
pub const CRACK_LAYERS: usize = 10;
/// 裂纹 stage s 的数组层号。特殊层统一放在真实贴图之后。
pub const CRACK_BASE: usize = MANIFEST_LAYERS;
/// 图集总层数（= mcv_render::gpu 创建 texture array 的 depth_or_array_layers）。
pub const LAYERS: usize = MANIFEST_LAYERS + CRACK_LAYERS;
pub const MIP_LEVELS: u32 = 2;

const _: () = assert!(MANIFEST_LAYERS + CRACK_LAYERS <= u16::MAX as usize);

/// 原版 missing 贴图像素（MissingTextureAtlasSprite.java:19 `pink = -524040`
/// = ABGR 0xFFF800F8 → RGB (248, 0, 248)；另一分支 `-16777216` = 不透明黑）。
const MISSING_PINK: [u8; 4] = [248, 0, 248, 255];
const MISSING_BLACK: [u8; 4] = [0, 0, 0, 255];

/// texture_2d_array 拆分数组上限。GLES 3.0 规范下限 MAX_ARRAY_TEXTURE_LAYERS
/// = 256（wgpu `Limits::downlevel_defaults` 同值，wgpu-types-30.0.1
/// src/limits.rs:426），LAYERS=838 层 = 4 个数组即可装下；需要超过 4 份意味着
/// adapter 连规范下限都没给，回到截断兜底（见 [`split_layer_counts`]）。
pub const MAX_TEXTURE_ARRAYS: usize = 4;

/// 按单数组层数上限把 [`LAYERS`] 拆成 N 份（N ≤ [`MAX_TEXTURE_ARRAYS`]）。
///
/// 返回每份层数（有序；和 ≤ LAYERS）。上限 ≥ LAYERS 时返回 `[LAYERS]`
/// （单数组，桌面/Vulkan 主路径零变化）。设备上限过小导致
/// cdiv(LAYERS, per) > 4 份时，每份仍取设备上限、总和 < LAYERS（截断
/// 兜底），保证 create_texture 永不越设备验证。
pub fn split_layer_counts(max_layers_per_array: usize) -> Vec<usize> {
    let per = max_layers_per_array.max(1);
    if per >= LAYERS {
        return vec![LAYERS];
    }
    let n = LAYERS.div_ceil(per).min(MAX_TEXTURE_ARRAYS);
    // n 份正好装得下时均分（单份 ≤ per）；装不下（per 过小）按 per 截尾。
    let step = if LAYERS.div_ceil(n) <= per {
        LAYERS.div_ceil(n)
    } else {
        per
    };
    let mut counts = Vec::with_capacity(n);
    let mut left = LAYERS;
    for _ in 0..n {
        let c = step.min(left);
        if c == 0 {
            break;
        }
        counts.push(c);
        left -= c;
    }
    counts
}

/// 全局层号 → (数组下标, 数组内层号)。`counts` 为 [`split_layer_counts`]
/// 的输出。越界层钳到末数组末层（与 shader if 链兜底分支同语义；mesher
/// 只发合法层号，越界仅是防御）。
pub fn remap_layer(layer: usize, counts: &[usize]) -> (usize, usize) {
    assert!(!counts.is_empty(), "split_layer_counts 结果不能为空");
    let mut acc = 0usize;
    for (i, &c) in counts.iter().enumerate() {
        if layer < acc + c {
            return (i, layer - acc);
        }
        acc += c;
    }
    (counts.len() - 1, counts[counts.len() - 1] - 1)
}

/// [`MissingTextureAtlasSprite.generateMissingImage`]（:13-32）的逐像素
/// 等价：16x16，四象限棋盘 `y < h/2 ^ x < w/2` → 左下/右上品红、其余黑。
/// 这是原版 missing 贴图的显式「缺失标记」（非程序化假贴图）。
fn paint_missing_marker(data: &mut [u8], layer: usize) {
    let base = layer * TILE_PX * TILE_PX * 4;
    for y in 0..TILE_PX as u32 {
        for x in 0..TILE_PX as u32 {
            let px = if (y < TILE_PX as u32 / 2) ^ (x < TILE_PX as u32 / 2) {
                MISSING_PINK
            } else {
                MISSING_BLACK
            };
            let off = base + ((y as usize * TILE_PX + x as usize) * 4);
            data[off..off + 4].copy_from_slice(&px);
        }
    }
}

/// Generates mip level 0 for all layers into `data` (LAYERS * 16*16*4 bytes).
/// 全层先铺原版 missing 贴图（MissingTextureAtlasSprite 语义）；真实贴图由
/// [`load_real_tiles`] 覆盖，裂纹层由 [`load_crack_stages`] 覆盖。哨兵层
/// [`SENTINEL_LAYER`] 无对应文件，永远保持 missing 标记——与原版对无贴图
/// 模型显示 missingno 一致。
pub fn generate_layers(data: &mut [u8]) {
    assert_eq!(data.len(), LAYERS * TILE_PX * TILE_PX * 4);
    for layer in 0..LAYERS {
        paint_missing_marker(data, layer);
    }
}

/// sRGB↔linear 查找表：逐行移植 `ARGB.java:9-28`（LINEAR_CHANNEL_DEPTH=1024；
/// `computeSrgbToLinear` :22-24 为 double pow、`computeLinearToSrgb` :26-28；
/// 正表 round(v*1023)，反表 round(v*255)；均值后按整数索引反表，
/// `linearChannelMean` :47-50）。
fn mip_tables() -> &'static ([u16; 256], [u8; 1024]) {
    static T: std::sync::OnceLock<([u16; 256], [u8; 1024])> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut s2l = [0u16; 256];
        for (i, slot) in s2l.iter_mut().enumerate() {
            let x = i as f32 / 255.0;
            let lin = if x >= 0.04045 {
                (((f64::from(x) + 0.055) / 1.055).powf(2.4)) as f32
            } else {
                x / 12.92
            };
            *slot = (lin * 1023.0 + 0.5) as u16;
        }
        let mut l2s = [0u8; 1024];
        for (i, slot) in l2s.iter_mut().enumerate() {
            let x = i as f32 / 1023.0;
            let srgb = if x >= 0.0031308 {
                (1.055 * f64::from(x).powf(0.416_666_666_666_666_7) - 0.055) as f32
            } else {
                12.92 * x
            };
            *slot = (srgb * 255.0 + 0.5) as u8;
        }
        (s2l, l2s)
    })
}

/// `ARGB.meanLinear`（:38-45）：RGB 走线性表（和/4 整数除 → 反表索引），
/// alpha 整数直均。`px4` = 2x2 四像素（顺序无关）。
fn mean_linear(px4: &[[u8; 4]; 4], s2l: &[u16; 256], l2s: &[u8; 1024]) -> [u8; 4] {
    let ch = |c: usize| -> u8 {
        let mean = px4
            .iter()
            .map(|p| u32::from(s2l[p[c] as usize]))
            .sum::<u32>()
            / 4;
        l2s[mean as usize]
    };
    let alpha = px4.iter().map(|p| u32::from(p[3])).sum::<u32>() / 4;
    [ch(0), ch(1), ch(2), alpha as u8]
}

/// `TextureUtil.solidify`（:130-184，CUTOUT mip 前置）：4 邻域 BFS
/// （DIRECTIONS :27），完全透明像素（alpha==0）取【最近不透明像素的 RGB】、
/// alpha 保持 0（`ARGB.color(0, nearest)` :177）——消除透明像素残留的黑色
/// RGB 参与 mip 平均造成的暗晕。不透明像素原样保留。全透明层原版读到
/// 默认黑（Java int 数组默认 0）；本函数防御性跳过（现存层无此形态，
/// 品红哨兵/裂纹均含不透明像素）。
fn solidify(img: &mut [u8], w: usize, h: usize) {
    let n = w * h;
    let mut nearest = vec![usize::MAX; n];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for i in 0..n {
        if img[i * 4 + 3] != 0 {
            nearest[i] = i;
            queue.push_back(i);
        }
    }
    // 原版松弛条件 `dist[nb] > dist[cur] + 1` 的 BFS（无权图=曼哈顿最近；
    // 种子先行入队，nearest 传播即 BFS 层序）。
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % w, i / w);
        let neighbors = [
            x.checked_sub(1).map(|v| v + y * w),
            if x + 1 < w { Some(x + 1 + y * w) } else { None },
            y.checked_sub(1).map(|v| x + v * w),
            if y + 1 < h {
                Some(x + (y + 1) * w)
            } else {
                None
            },
        ];
        for nb in neighbors.into_iter().flatten() {
            if nearest[nb] == usize::MAX {
                nearest[nb] = nearest[i];
                queue.push_back(nb);
            }
        }
    }
    // 写回：仅 alpha==0 像素换 RGB（alpha 仍 0）；来源像素本身不透明、
    // 永不被改写，故就地写安全。无来源（全透明层，防御）跳过。
    for i in 0..n {
        if img[i * 4 + 3] == 0 && nearest[i] != usize::MAX {
            let s = nearest[i] * 4;
            let (r, g, b) = (img[s], img[s + 1], img[s + 2]);
            img[i * 4] = r;
            img[i * 4 + 1] = g;
            img[i * 4 + 2] = b;
        }
    }
}

/// `MipmapGenerator.alphaTestCoverage`（:33-53）：像素域 (w-1)x(h-1)，
/// 每像素 4x4 子采样双线性 alpha > alphaRef 计覆盖。退化尺寸（<2）返回 0
/// （原版此时除零得 NaN，语义等价「无覆盖可保」；本管线恒 16x16/8x8 不达）。
fn alpha_test_coverage(img: &[u8], w: usize, h: usize, alpha_ref: f32, alpha_scale: f32) -> f32 {
    if w < 2 || h < 2 {
        return 0.0;
    }
    let mut coverage = 0f32;
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let a = |xx: usize, yy: usize| {
                (f32::from(img[(yy * w + xx) * 4 + 3]) / 255.0 * alpha_scale).clamp(0.0, 1.0)
            };
            let (a00, a10) = (a(x, y), a(x + 1, y));
            let (a01, a11) = (a(x, y + 1), a(x + 1, y + 1));
            let mut texel = 0f32;
            for sy in 0..4 {
                let fy = (sy as f32 + 0.5) / 4.0;
                for sx in 0..4 {
                    let fx = (sx as f32 + 0.5) / 4.0;
                    let v = a00 * (1.0 - fx) * (1.0 - fy)
                        + a10 * fx * (1.0 - fy)
                        + a01 * (1.0 - fx) * fy
                        + a11 * fx * fy;
                    if v > alpha_ref {
                        texel += 1.0;
                    }
                }
            }
            coverage += texel / 16.0;
        }
    }
    coverage / ((w - 1) * (h - 1)) as f32
}

/// `MipmapGenerator.scaleAlphaToCoverage`（:55-89）：5 次二分 alphaScale
/// ∈ [0,4] 逼近 mip0 原始 alpha-test 覆盖，随后逐像素
/// alpha = clamp(a*scale + bias + 0.025, 0, 1) 写回（floor(a*255)，
/// `ARGB.as8BitChannel` :225-227）。cutout mip 防「平均抬升 alpha →
/// 远处镂空变实心」。
fn scale_alpha_to_coverage(
    img: &mut [u8],
    w: usize,
    h: usize,
    desired: f32,
    alpha_ref: f32,
    bias: f32,
) {
    let (mut lo, mut hi) = (0.0f32, 4.0f32);
    let mut scale = 1.0f32;
    let (mut best_scale, mut best_err) = (1.0f32, f32::MAX);
    for _ in 0..5 {
        let cov = alpha_test_coverage(img, w, h, alpha_ref, scale);
        let err = (cov - desired).abs();
        if err < best_err {
            best_err = err;
            best_scale = scale;
        }
        if cov < desired {
            lo = scale;
        } else if cov > desired {
            hi = scale;
        } else {
            break;
        }
        scale = (lo + hi) * 0.5;
    }
    for i in 0..w * h {
        let o = i * 4 + 3;
        let a = f32::from(img[o]) / 255.0;
        let a = (a * best_scale + bias + 0.025).clamp(0.0, 1.0);
        img[o] = (a * 255.0) as u8;
    }
}

/// CPU mip level 1 (16 -> 8 px per side, per layer)——原版 MipmapGenerator
/// 语义逐行移植（audit #10：旧版 sRGB 域直平均四通道，半透明边缘远处出
/// 暗/脏晕）。策略 = AUTO（`MipmapGenerator.java:99-101`）：层内含
/// alpha==0 像素（`NativeImage.computeTransparency` :515-524 的
/// hasTransparent）→ CUTOUT，否则 MEAN；CUTOUT 先 solidify
/// （[`solidify`]，:102-105 对 mip0 工作副本），混合走 [`mean_linear`]
/// （线性域），随后每级 scaleAlphaToCoverage（[`scale_alpha_to_coverage`]，
/// :139-142，cutoutRef=0.5、alphaCutoffBias=0——block 无 texture mcmeta
/// 用例）。采样器现仍 Nearest/Nearest（gpu.rs），mip1 像素语义先对原版、
/// 三线性接线属渲染侧。
pub fn generate_mip1(mip0: &[u8], mip1: &mut [u8]) {
    assert_eq!(mip0.len(), LAYERS * TILE_PX * TILE_PX * 4);
    assert_eq!(mip1.len(), LAYERS * 8 * 8 * 4);
    let (s2l, l2s) = mip_tables();
    const S: usize = TILE_PX / 2;
    for layer in 0..LAYERS {
        let mut work = [0u8; TILE_PX * TILE_PX * 4];
        let off = layer * TILE_PX * TILE_PX * 4;
        // 长度表达式绝不能再借 work（copy_from_slice 的 &mut self 借用
        // 覆盖整个调用，E0502——CI run 38094574994）：用常量长度。
        work.copy_from_slice(&mip0[off..off + TILE_PX * TILE_PX * 4]);
        let cutout = work.chunks(4).any(|p| p[3] == 0);
        // 原始覆盖度只依赖 alpha，solidify 不改 alpha（alpha 保持 0），
        // 计算时机与原版先后均可（原版先 solidify 再测，:102-116）。
        let original_coverage = if cutout {
            alpha_test_coverage(&work, TILE_PX, TILE_PX, 0.5, 1.0)
        } else {
            0.0
        };
        if cutout {
            solidify(&mut work, TILE_PX, TILE_PX);
        }
        for y in 0..S {
            for x in 0..S {
                let mut q = [[0u8; 4]; 4];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let o = ((2 * y + dy) * TILE_PX + 2 * x + dx) * 4;
                        q[dy * 2 + dx].copy_from_slice(&work[o..o + 4]);
                    }
                }
                let m = mean_linear(&q, s2l, l2s);
                let d = (layer * S * S + y * S + x) * 4;
                mip1[d..d + 4].copy_from_slice(&m);
            }
        }
        if cutout {
            let off = layer * S * S * 4;
            scale_alpha_to_coverage(
                &mut mip1[off..off + S * S * 4],
                S,
                S,
                original_coverage,
                0.5,
                0.0,
            );
        }
    }
}

/// Full upload payload: mip0 + mip1 contiguous. 真实贴图从
/// `<assets_dir>/textures/block/*.png`（资源根 = assets/minecraft）读盘
/// 覆盖；缺失/解码失败的层保留原版 missing 标记并 `log::error!`（素材
/// 红线：不产出程序化假贴图）。挖掘裂纹层读原版 destroy_stage_0..9
/// （[`load_crack_stages`]）。
pub fn generate_payload_with_pack(assets_dir: Option<&Path>) -> Vec<u8> {
    let mut mip0 = vec![0u8; LAYERS * TILE_PX * TILE_PX * 4];
    generate_layers(&mut mip0);
    match assets_dir {
        Some(dir) => {
            let n = load_real_tiles(dir, &mut mip0);
            if n < REAL_TILE_COUNT as u32 {
                // 硬错误（一次性汇总）：列出示例缺失路径供诊断，完整清单
                // 用 missing_source_files 查询（AssetManager 侧汇总上报）。
                let miss = missing_source_files(dir);
                let sample: Vec<String> = miss
                    .iter()
                    .take(10)
                    .map(|p| p.display().to_string())
                    .collect();
                log::error!(
                    "atlas: {n}/{} real tiles from {}——缺失层将显示原版 missing 标记（无程序化回退）; 缺失示例: {}{}",
                    REAL_TILE_COUNT,
                    dir.display(),
                    sample.join(", "),
                    if miss.len() > 10 { ", …" } else { "" }
                );
            } else {
                log::info!(
                    "atlas: {n}/{} real tiles from {}",
                    REAL_TILE_COUNT,
                    dir.display()
                );
            }
            let c = load_crack_stages(dir, &mut mip0);
            if c < CRACK_LAYERS as u32 {
                log::error!(
                    "atlas: {c}/{} vanilla destroy stages——缺失档位显示 missing 标记",
                    CRACK_LAYERS
                );
            } else {
                log::info!("atlas: {c}/{} vanilla destroy stages", CRACK_LAYERS);
            }
        }
        None => {
            log::warn!("atlas: 无资源根——全部层为原版 missing 标记（仅无头测试模式）");
        }
    }
    let mut mip1 = vec![0u8; LAYERS * 8 * 8 * 4];
    generate_mip1(&mip0, &mut mip1);
    mip0.extend_from_slice(&mip1);
    mip0
}

/// 无资源路径（全部层 = 原版 missing 标记；仅供无头测试）。带真实贴图用
/// [`generate_payload_with_pack`]。
pub fn generate_payload() -> Vec<u8> {
    generate_payload_with_pack(None)
}

const MANIFEST_JSON: &str = include_str!("../tiles_manifest.json");
const BLOCKS_SUBDIR: &str = "textures/block";
// manifest 键存在性由 tests::parse_manifest_names 运行期锁（str::find 非 const）

/// 从嵌入的 tiles_manifest.json 抠出 `tile_index_to_file` 数组的文件名词表。
/// 手写极简解析（该数组在 JSON 中最靠前，取首个 `[...]` 内的全部引号串；
/// 生成器保证词表内无括号/逗号/引号字符）——零 serde 依赖。
fn parse_manifest_names() -> Vec<&'static str> {
    let key = MANIFEST_JSON
        .find("\"tile_index_to_file\"")
        .expect("manifest: missing tile_index_to_file");
    let open = MANIFEST_JSON[key..]
        .find('[')
        .expect("manifest: missing array open");
    let arr = &MANIFEST_JSON[key + open..];
    let close = arr.find(']').expect("manifest: missing array close");
    let mut names = Vec::with_capacity(MANIFEST_LAYERS);
    // split('"') 后奇数下标段即引号内内容（偶数段是分隔符/前缀）
    for seg in arr[..close].split('"').skip(1).step_by(2) {
        names.push(seg);
    }
    names
}

/// manifest 层号 → 贴图文件名（不含扩展名），下标即层号。
pub fn tile_file_names() -> &'static [&'static str] {
    static NAMES: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    let v = NAMES.get_or_init(parse_manifest_names);
    assert_eq!(
        v.len(),
        MANIFEST_LAYERS,
        "tiles_manifest.json 层数与 MANIFEST_LAYERS 不符"
    );
    v.as_slice()
}

/// 原版 [`AnimationMetadataSection.calculateFrameSize`]（:25-38）缺省分支：
/// 无显式 width/height 时帧 = min 边正方形。全库 block/ 54 张动画 mcmeta
/// 无一例显式 width/height（gen-blocks.py 转写时可验证），故运行期不读
/// mcmeta，直接取 min(w,h)。
///
/// 帧号 → 帧位：行主序（`SpriteContents.AnimatedTexture.getFrameX/Y`
/// :272-278，x = idx % rowSize、y = idx / rowSize，rowSize = 图宽 ÷ 帧宽）。
/// 帧取出后适配 16x16 层：
/// - 帧恰为 16x16（全部 52 张经典竖条）→ 逐像素精确复制（修复 audit #2：
///   旧实现整图重采样把 16xN 条带跨帧逐行混采，water_still 与帧 0 差
///   194/256 像素——tests 锁定）；
/// - 帧 > 16（32x32 shelf 族，audit #7）→ 取帧左上 16x16 裁切而非整图压扁；
/// - 帧 < 16（现存数据无）→ 最近邻放大兜底。
fn extract_frame_to_tile(src: &[u8], sw: u32, sh: u32, frame_index: u32, dst: &mut [u8]) {
    let fw = sw.min(sh);
    let fh = fw;
    let row = (sw / fw).max(1);
    let mut fx = (frame_index % row) * fw;
    let mut fy = (frame_index / row) * fh;
    if fx + fw > sw || fy + fh > sh {
        // 越界帧号回退帧 0（原版在 createAnimatedTexture 阶段已过滤非法
        // index，SpriteContents.java:137-146；此处仅防御）。
        fx = 0;
        fy = 0;
    }
    for dy in 0..TILE_PX as u32 {
        for dx in 0..TILE_PX as u32 {
            // 帧 ≥ TILE_PX：裁左上（dx/dy 直接落在帧内左上 16x16）；
            // 帧 < TILE_PX：帧内最近邻放大。
            let sx = fx
                + if fw >= TILE_PX as u32 {
                    dx
                } else {
                    dx * fw / TILE_PX as u32
                };
            let sy = fy
                + if fh >= TILE_PX as u32 {
                    dy
                } else {
                    dy * fh / TILE_PX as u32
                };
            for c in 0..4 {
                let d = ((dy * TILE_PX as u32 + dx) * 4 + c) as usize;
                let s = ((sy * sw + sx) * 4 + c) as usize;
                dst[d] = src[s];
            }
        }
    }
}

/// 动画贴图「初始上传帧」帧号（原版 = 帧列表首项 `frames[0].index`，缺省
/// 帧序 → 0；`SpriteContents.createAnimatedTexture` :113-163 + 首帧上传
/// 语义）。数据源 = manifest `animation.frame0_index`（gen-blocks.py 从
/// mcmeta 转写；全库 block/ 内非零项仅 fire_0/soul_fire_0 = 16）。
fn parse_frame0_index() -> Vec<(&'static str, u32)> {
    let Some(key) = MANIFEST_JSON.find("\"frame0_index\"") else {
        return Vec::new();
    };
    let rest = &MANIFEST_JSON[key..];
    let open = rest
        .find('{')
        .expect("manifest: animation.frame0_index 缺 {");
    let obj = &rest[open..];
    let close = obj
        .find('}')
        .expect("manifest: animation.frame0_index 缺 }");
    // 生成器保证扁平 {name: int} 映射：split('"') 奇数段=键，其后偶数段以
    // ": <整数>" 开头（同 parse_manifest_names 的零 serde 约定）。
    let segs: Vec<&str> = obj[..=close].split('"').collect();
    let mut v = Vec::new();
    let mut i = 1;
    while i + 1 < segs.len() {
        let val = segs[i + 1]
            .trim_start()
            .strip_prefix(':')
            .unwrap_or("")
            .trim_start();
        let num: u32 = val
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        v.push((segs[i], num));
        i += 2;
    }
    v
}

/// 贴图名 → 初始帧号（未列出的贴图 = 0）。
fn frame0_index(name: &str) -> u32 {
    static MAP: std::sync::OnceLock<Vec<(&'static str, u32)>> = std::sync::OnceLock::new();
    MAP.get_or_init(parse_frame0_index)
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(0, |(_, i)| *i)
}

/// 解码 PNG → 按原版帧语义取初始帧 → 16x16 RGBA → 写入 mip0 的指定层。
/// `frame_index` 见 [`frame0_index`]（非动画贴图恒 0；此时 min 边裁帧与
/// 原版无 mcmeta 时的 `FrameSize` 语义一致——竖条取顶帧、32x32 取左上）。
/// 失败返回 false（保留现有内容 = missing 标记）。
fn decode_into_layer(
    bytes: &[u8],
    src_path: &Path,
    layer: usize,
    layers: &mut [u8],
    frame_index: u32,
) -> bool {
    let Ok(img) = image::load_from_memory(bytes) else {
        log::warn!("atlas: failed to decode {}", src_path.display());
        return false;
    };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return false;
    }
    let mut tile = [0u8; TILE_PX * TILE_PX * 4];
    extract_frame_to_tile(rgba.as_raw(), w, h, frame_index, &mut tile);
    let off = layer * TILE_PX * TILE_PX * 4;
    layers[off..off + tile.len()].copy_from_slice(&tile);
    true
}

/// 从 `<dir>/textures/block/<name>.png`（单一资源根的原版路径）加载
/// 827 张真实方块贴图覆盖层 0..826。缺文件/解码失败该层保留原版
/// missing 标记并计数告警（调用方 `log::error!`，无程序化回退）；
/// missing 哨兵层（[`SENTINEL_LAYER`]）无对应文件，恒跳过。返回覆盖层数。
pub fn load_real_tiles(dir: &Path, layers: &mut [u8]) -> u32 {
    assert!(layers.len() >= MANIFEST_LAYERS * TILE_PX * TILE_PX * 4);
    let blocks = dir.join(BLOCKS_SUBDIR);
    if !blocks.is_dir() {
        return 0;
    }
    let mut count = 0u32;
    for (idx, name) in tile_file_names().iter().enumerate() {
        if idx == SENTINEL_LAYER {
            debug_assert_eq!(*name, SENTINEL_TILE);
            continue; // 哨兵层：无对应文件，保持 missing 标记
        }
        let path = blocks.join(format!("{name}.png"));
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if decode_into_layer(&bytes, &path, idx, layers, frame0_index(name)) {
            count += 1;
        }
    }
    count
}

/// 加载原版挖掘裂纹 10 档到 `CRACK_BASE + s` 层。原版 jar 内路径即
/// `textures/block/destroy_stage_0..9.png`（16x16，黑色裂纹 + alpha），
/// 档位公式 `(int)(destroyProgress * 10)`（MultiPlayerGameMode.java:551）。
/// 缺档该层保留 missing 标记（调用方 `log::error!`，无程序化回退）。
/// 返回覆盖档数。
pub fn load_crack_stages(dir: &Path, layers: &mut [u8]) -> u32 {
    assert!(layers.len() >= LAYERS * TILE_PX * TILE_PX * 4);
    let blocks = dir.join(BLOCKS_SUBDIR);
    if !blocks.is_dir() {
        return 0;
    }
    let mut count = 0u32;
    for s in 0..CRACK_LAYERS {
        let path = blocks.join(format!("destroy_stage_{s}.png"));
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if decode_into_layer(&bytes, &path, CRACK_BASE + s, layers, 0) {
            count += 1;
        }
    }
    count
}

/// 图集源贴图缺失清单（M8c 硬错误汇总入口）：真实层 0..826 的
/// `textures/block/<name>.png` + 裂纹 10 档 `destroy_stage_0..9.png`
/// 中所有不存在的**完整路径**（排序）。哨兵层无文件，不计缺失。
/// AssetManager 用它做一次性汇总上报；本模块自身的读盘循环与
/// [`generate_payload_with_pack`] 是图集这条线的既定读取路径（历史
/// 遗留的合法例外——它只读原版单张贴图拼层，不产生程序化像素）。
pub fn missing_source_files(dir: &Path) -> Vec<PathBuf> {
    let blocks = dir.join(BLOCKS_SUBDIR);
    let mut miss = Vec::new();
    for (idx, name) in tile_file_names().iter().enumerate() {
        if idx == SENTINEL_LAYER {
            continue;
        }
        let p = blocks.join(format!("{name}.png"));
        if !p.is_file() {
            miss.push(p);
        }
    }
    for s in 0..CRACK_LAYERS {
        let p = blocks.join(format!("destroy_stage_{s}.png"));
        if !p.is_file() {
            miss.push(p);
        }
    }
    miss
}

/// 标准 MC 资源包树（贴图名 1.13+ 起即 manifest 原名，无需翻译表）。
const MC_DIRS: [&str; 2] = ["assets/minecraft/textures/block", "textures/block"];

/// 纹理包旧图引擎名（历史 API，值即 manifest 贴图名，15 项）。
pub const PACK_TILE_NAMES: [&str; 15] = [
    "grass_block_top",
    "grass_block_side",
    "dirt",
    "stone",
    "sand",
    "water_still",
    "oak_log",
    "oak_log_top",
    "oak_leaves",
    "oak_planks",
    "cobblestone",
    "bedrock",
    "grass_block_snow",
    "poppy",
    "dandelion",
];

/// Overrides atlas layers with user texture-pack PNGs for the classic 15
/// tiles (files named after manifest texture names, e.g. `stone.png`, or the
/// standard MC pack tree). Each name resolves to its manifest layer, so pack
/// files replace the real content of exactly that layer.
///
/// Returns the number of layers overridden.
pub fn load_pack_over(dir: &std::path::Path, mip0: &mut [u8]) -> u32 {
    // 调用方（gpu.rs）传 mip0 子切片，只要求覆盖真实层区
    assert!(mip0.len() >= MANIFEST_LAYERS * TILE_PX * TILE_PX * 4);
    let names = tile_file_names();
    let mut count = 0u32;
    for name in PACK_TILE_NAMES {
        // 候选顺序：扁平引擎名 → 标准 MC 包树。首个命中生效。
        let candidates: Vec<std::path::PathBuf> = {
            let mut v = vec![dir.join(format!("{name}.png"))];
            for d in MC_DIRS {
                v.push(dir.join(d).join(format!("{name}.png")));
            }
            v
        };
        let Some((bytes, src_path)) = candidates
            .iter()
            .find_map(|p| std::fs::read(p).ok().map(|b| (b, p.clone())))
        else {
            continue;
        };
        let Some(idx) = names.iter().position(|f| *f == name) else {
            continue;
        };
        if decode_into_layer(&bytes, &src_path, idx, mip0, frame0_index(name)) {
            count += 1;
        }
    }
    count
}

/// 编译期一致性：tiles:: 常量必须是 manifest 里对应贴图的层号。
const _: () = {
    // 解析在 const 里做不了（需要 OnceLock），退化为范围断言 + 运行期
    // tests::gen_table_layer_indices_in_real_region 双重锁。
    assert!(tiles::GRASS_TOP as usize > 0 && (tiles::GRASS_TOP as usize) < MANIFEST_LAYERS);
    assert!((tiles::STONE as usize) < MANIFEST_LAYERS);
    assert!((tiles::FLOWER_YELLOW as usize) < MANIFEST_LAYERS);
};

#[cfg(test)]
mod frame_tests {
    use super::*;

    fn assets_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/minecraft")
    }

    fn read_block(name: &str) -> image::RgbaImage {
        let p = assets_dir().join(format!("textures/block/{name}.png"));
        image::load_from_memory(&std::fs::read(&p).unwrap())
            .unwrap()
            .to_rgba8()
    }

    fn diff_px(a: &[u8], b: &[u8]) -> usize {
        (0..TILE_PX * TILE_PX)
            .filter(|i| a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4])
            .count()
    }

    /// 修复前的旧算法：整图最近邻重采样（保留作对照回归，锁定 audit #2
    /// 的「跨帧逐行混采 194/256 像素」量化值）。
    fn old_resample(src: &[u8], sw: u32, sh: u32, dst: &mut [u8]) {
        for y in 0..TILE_PX as u32 {
            for x in 0..TILE_PX as u32 {
                let sx = (u64::from(x) * u64::from(sw) / TILE_PX as u64) as u32;
                let sy = (u64::from(y) * u64::from(sh) / TILE_PX as u64) as u32;
                for c in 0..4 {
                    let d = ((y * TILE_PX as u32 + x) * 4 + c) as usize;
                    let s = ((sy * sw + sx) * 4 + c) as usize;
                    dst[d] = src[s];
                }
            }
        }
    }

    /// audit #2 主断言：water_still（16x512）帧 0 = 源图顶 16 行逐像素；
    /// 旧整图重采样与帧 0 差 194/256（PIL 实证对照）。
    #[test]
    fn water_still_frame0_exact_vs_old_mosaic() {
        let img = read_block("water_still");
        assert_eq!((img.width(), img.height()), (16, 512));
        let mut frame0 = [0u8; TILE_PX * TILE_PX * 4];
        extract_frame_to_tile(img.as_raw(), img.width(), img.height(), 0, &mut frame0);
        assert_eq!(diff_px(&frame0, &img.as_raw()[..frame0.len()]), 0);
        let mut old = [0u8; TILE_PX * TILE_PX * 4];
        old_resample(img.as_raw(), img.width(), img.height(), &mut old);
        assert_eq!(diff_px(&frame0, &old), 194);
    }

    /// 显式帧序（fire_0.mcmeta frames 首项 =16）：初始帧 = 帧 16（原版
    /// SpriteContents 初始上传帧），非帧 0；行主序帧位 y=16*16 行。
    #[test]
    fn fire0_uses_explicit_first_frame_index() {
        assert_eq!(frame0_index("fire_0"), 16);
        assert_eq!(frame0_index("soul_fire_0"), 16);
        assert_eq!(frame0_index("water_still"), 0);
        let img = read_block("fire_0");
        assert_eq!((img.width(), img.height()), (16, 512));
        let mut tile = [0u8; TILE_PX * TILE_PX * 4];
        extract_frame_to_tile(img.as_raw(), img.width(), img.height(), 16, &mut tile);
        let raw = img.as_raw();
        let off = 16 * 16 * 16 * 4; // 帧 16 起始字节（rowSize=1，帧高 16）
        assert_eq!(diff_px(&tile, &raw[off..off + tile.len()]), 0);
        assert_ne!(diff_px(&tile, &raw[..tile.len()]), 0);
    }

    /// audit #7：32x32 sprite（shelf 族 12 张 + dried_ghast_hydration_0_
    /// tentacles 共 13 张真实层）取左上 16x16 裁切，而非整图最近邻压扁。
    #[test]
    fn shelf_32x32_crops_top_left_not_squash() {
        let img = read_block("oak_shelf");
        assert_eq!((img.width(), img.height()), (32, 32));
        let mut tile = [0u8; TILE_PX * TILE_PX * 4];
        extract_frame_to_tile(img.as_raw(), img.width(), img.height(), 0, &mut tile);
        let raw = img.as_raw();
        for y in 0..TILE_PX {
            for x in 0..TILE_PX {
                let d = (y * TILE_PX + x) * 4;
                let s = (y * 32 + x) * 4;
                assert_eq!(&tile[d..d + 4], &raw[s..s + 4], "oak_shelf ({x},{y})");
            }
        }
    }

    /// manifest animation 节语义锁表（对照 assets 内 mcmeta 原件逐项核）：
    /// water_still frametime=2 无 frames（缺省帧序 0..31）、lava_still 显式
    /// 帧序 38 项、magma interpolate+frametime 8、fire_0 帧序首项 16、
    /// command_block 族 interpolate+frametime 10；动画条目总数 34。
    #[test]
    fn manifest_animation_table() {
        assert_eq!(
            parse_frame0_index(),
            vec![("fire_0", 16u32), ("soul_fire_0", 16u32)],
            "frame0_index 只应含显式 frames 且首帧号 ≠ 0 的条目"
        );
        let anim = &MANIFEST_JSON[MANIFEST_JSON.find("\"animation\"").unwrap()..];
        // 条目数：tiles 对象内 3 空格缩进的键行（gen-blocks.py
        // json.dumps(indent=1) 的输出形态）。
        let tiles = &anim[anim.find("\"tiles\": {").unwrap()..];
        assert_eq!(
            tiles.match_indices("\n   \"").count(),
            34,
            "manifest 内动画贴图应为 34 张"
        );
        let snippet = |name: &str| -> &'static str {
            let start = anim
                .find(&format!("\"{name}\": {{"))
                .unwrap_or_else(|| panic!("animation 缺 {name}"));
            // 普通字符串无 `{{}}` 转义：条目收尾就是换行 + 3 空格 + '}'。
            let end = anim[start..].find("\n   }").unwrap() + start + 4;
            &anim[start..end]
        };
        let ws = snippet("water_still");
        assert!(
            ws.contains("\"frametime\": 2") && !ws.contains("\"frames\""),
            "{ws}"
        );
        // frames 数组元素缩进 = 5 空格（json.dumps(indent=1)：tiles 条目
        // 3 空格、条目键 4 空格、数组元素 5 空格）。
        let ls = snippet("lava_still");
        assert!(
            ls.contains("\"frametime\": 2")
                && ls.contains("\"frames\"")
                && ls.contains("\n     0,")
                && !ls.contains("interpolate"),
            "{ls}"
        );
        assert_eq!(
            ls.matches("\n     ").count(),
            38,
            "lava_still 显式帧序应 38 项（回环 0..18..1）"
        );
        let mg = snippet("magma");
        assert!(
            mg.contains("\"frametime\": 8")
                && mg.contains("\"interpolate\": true")
                && mg.contains("\n     0,")
                && mg.contains("\n     2"),
            "{mg}"
        );
        let f0 = snippet("fire_0");
        assert!(
            f0.contains("\"frames\"") && f0.contains("\"frames\": [\n     16,"),
            "{f0}"
        );
        let cb = snippet("command_block_back");
        assert!(
            cb.contains("\"frametime\": 10")
                && cb.contains("\"interpolate\": true")
                && !cb.contains("\"frames\""),
            "{cb}"
        );
    }
}

#[cfg(test)]
mod mip_tests {
    use super::*;

    fn payload_len_mip0() -> usize {
        LAYERS * TILE_PX * TILE_PX * 4
    }

    /// 线性域均值（`ARGB.meanLinear`）：黑白棋盘（2x2 内 2 黑 2 白）的
    /// mip1 = LINEAR_TO_SRGB[(0+0+1023+1023)/4=511] = 187；旧 sRGB 域直
    /// 平均得 127（audit #10 的「暗晕」方向）。alpha 全 255 → MEAN 路径
    /// （无 solidify/a2c），alpha 均值恒 255。
    #[test]
    fn mip1_averages_in_linear_domain() {
        let (s2l, l2s) = mip_tables();
        assert_eq!((s2l[0], s2l[255]), (0, 1023));
        assert_eq!(l2s[511], 187, "0.5 linear → sRGB 187（ARGB.java:15-20）");
        let mut mip0 = vec![0u8; payload_len_mip0()];
        for layer in 0..LAYERS {
            for y in 0..TILE_PX {
                for x in 0..TILE_PX {
                    let o = (layer * TILE_PX * TILE_PX + y * TILE_PX + x) * 4;
                    let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                    mip0[o] = v;
                    mip0[o + 1] = v;
                    mip0[o + 2] = v;
                    mip0[o + 3] = 255;
                }
            }
        }
        let mut mip1 = vec![0u8; LAYERS * 8 * 8 * 4];
        generate_mip1(&mip0, &mut mip1);
        for probe in [0usize, 7, 63] {
            let o = probe * 4;
            assert_eq!(
                &mip1[o..o + 4],
                &[187, 187, 187, 255],
                "mip1 px {probe}：期望线性域均值 187（旧 sRGB 均值得 127）"
            );
        }
    }

    /// cutout solidify（`TextureUtil.solidify:130-184`）：alpha==0 像素取
    /// 最近不透明像素 RGB（alpha 仍 0）→ mip1 红色不被透明像素的黑 RGB
    /// 拖暗（旧实现 4 像素均值出 (50,12,12)）。a2c 只改 alpha 不改 RGB。
    #[test]
    fn cutout_mip_solidifies_transparent_rgb() {
        let (s2l, l2s) = mip_tables();
        let expect = [
            l2s[s2l[200] as usize],
            l2s[s2l[50] as usize],
            l2s[s2l[50] as usize],
        ];
        assert_eq!(expect, [200, 50, 50], "sRGB 表往返应保序（1024 级足够）");
        const L: usize = 100;
        let mut mip0 = vec![0u8; payload_len_mip0()];
        // 其余层：不透明白（MEAN 路径，不参与本断言）。
        for p in mip0.chunks_mut(4) {
            p.copy_from_slice(&[255, 255, 255, 255]);
        }
        let base = L * TILE_PX * TILE_PX * 4;
        // L 层：仅 (0,0) 不透明红，其余全透明黑 RGB（PNG 常见残留形态）。
        for i in 0..TILE_PX * TILE_PX {
            mip0[base + i * 4..base + i * 4 + 4].copy_from_slice(&[0, 0, 0, 0]);
        }
        mip0[base..base + 4].copy_from_slice(&[200, 50, 50, 255]);
        let mut mip1 = vec![0u8; LAYERS * 8 * 8 * 4];
        generate_mip1(&mip0, &mut mip1);
        let o0 = (L * 8 * 8) * 4; // mip1 像素 (0,0)
        assert_eq!(
            &mip1[o0..o0 + 3],
            &[200, 50, 50],
            "solidify 后 2x2 均值 RGB 保持红（旧 sRGB 黑混得 (50,12,12)）"
        );
        // 邻块 (1,1)：4 透明像素也被 solidify 成红（alpha 0），RGB 不再是黑。
        let o1 = o0 + 4;
        assert_eq!(&mip1[o1..o1 + 3], &[200, 50, 50]);
        // a2c：desired=mip0 覆盖（1/225），best_scale=1，alpha += 0.025 偏置
        //（MipmapGenerator.java:82）→ 63→69 与 0→6，均 >0 且 <255。
        let a0 = mip1[o0 + 3];
        let a1 = mip1[o1 + 3];
        assert!(
            a0 > 0 && a0 < 255 && a1 > 0,
            "a2c alpha 写回 a0={a0} a1={a1}"
        );
    }

    /// 半透明（0<alpha<255、无 alpha==0）层 = MEAN 策略（Transparency
    /// 的 hasTransparent 只数 alpha==0，NativeImage.java:515-528）：
    /// 不 solidify，alpha 整数直均。
    #[test]
    fn translucent_layer_uses_mean_not_cutout() {
        const L: usize = 7;
        let mut mip0 = vec![0u8; payload_len_mip0()];
        for p in mip0.chunks_mut(4) {
            p.copy_from_slice(&[255, 255, 255, 255]);
        }
        let base = L * TILE_PX * TILE_PX * 4;
        for i in 0..TILE_PX * TILE_PX {
            mip0[base + i * 4..base + i * 4 + 4].copy_from_slice(&[255, 255, 255, 180]);
        }
        let mut mip1 = vec![0u8; LAYERS * 8 * 8 * 4];
        generate_mip1(&mip0, &mut mip1);
        let o = (L * 8 * 8) * 4;
        assert_eq!(
            &mip1[o..o + 4],
            &[255, 255, 255, 180],
            "水类半透明层 mip1 均值保语义"
        );
    }
}
