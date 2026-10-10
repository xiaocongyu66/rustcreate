//! # mcv_assets —— 统一素材管线（M8c 资源管线统一）
//!
//! 全仓素材 IO 的唯一入口（CI 分层守卫下本 crate **不含 wgpu/winit**，
//! game 层与 engine 层都可安全依赖）：
//!
//! - **资源根解析**：单一 `assets/minecraft`（布局镜像原版 jar）。解析顺序
//!   `MCV_ASSETS_DIR` env → Android 解包目录（调用方传入）→ CWD →
//!   exe 同级（桌面 bundle 分发形态）。收编自 app.rs `assets_dir()` 与
//!   mcv_audio `default_sounds_dir()` 的散装解析。
//! - **缺素材 = 硬错误**：[`AssetManager::read`] 失败返回带**完整路径**的
//!   [`AssetError`]；每个路径只 `log::error!` 一次（去重），并登记进
//!   `missing` 表供 [`AssetManager::summary_report`] **一次性汇总上报**——
//!   杜绝各加载点各自静默/刷屏。显示层仍按素材红线以原版 missing 标记/
//!   透明纹理降级，但错误必须可见、可诊断（`missing()` 可查询）。
//! - **缓存去重**：同一路径只触一次磁盘（[`AssetManager::read`] 返回
//!   `Arc<[u8]>` 克隆即引用计数）。
//! - **图集重建**：[`AssetManager::rebuild_atlas_payload`] 把 mcv_core::atlas
//!   的「源贴图 → 838 层 payload（mip0+mip1）」接进来；GLES 层数拆分方案
//!   （`split_layer_counts`/`remap_layer`）留在 `mcv_core::atlas`（shader
//!   展开同源），调用方（mcv_render::gpu）消费。
//!
//! 素材红线不变：一切视觉素材必须来自原版 `assets/minecraft/**`，本 crate
//! 不产生任何像素。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use mcv_core::atlas;

/// 资源根相对 workspace/exe 目录的位置（镜像原版 jar 的 assets 树）。
pub const ROOT_REL: &str = "assets/minecraft";

/// 原版字体图集（相对资源根）。
pub const FONT_ASCII: &str = "textures/font/ascii.png";
/// 生物群系染色 colormap（相对资源根；缺失 → 上层禁用染色，不伪造颜色）。
pub const COLORMAP_FILES: [&str; 2] = [
    "textures/colormap/grass.png",
    "textures/colormap/foliage.png",
];
/// 天体太阳贴图（相对资源根）。
pub const CELESTIAL_SUN: &str = "textures/environment/celestial/sun.png";
/// 月相目录（相对资源根）。
pub const CELESTIAL_MOON_DIR: &str = "textures/environment/celestial/moon";
/// MoonPhase.java 枚举序的月相文件名（8 相，层序即数组层序 1..9）。
pub const CELESTIAL_MOON_FILES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];
/// 原版音效事件表（相对**音效根**；ci/fetch-sounds.sh 的布局：sounds.json
/// 在 sounds 根、变体相对其铺开）。
pub const SOUNDS_INDEX: &str = "sounds.json";

/// 素材缺失/损坏错误：`Display` 必含**完整路径**（硬错误的可诊断载体）。
#[derive(Debug, Clone)]
pub struct AssetError {
    pub path: PathBuf,
    pub reason: String,
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "素材缺失/损坏: {}: {}", self.path.display(), self.reason)
    }
}

impl std::error::Error for AssetError {}

/// 统一文件读取入口（散装 `std::fs::read` 的替代品）：错误含完整路径。
/// 不缓存、不打日志——日志/汇总归 [`AssetManager`]，本函数给无资源根上下文的
/// 调用点（如 mcv_audio 运行期懒加载）用。
pub fn read_file(path: &Path) -> Result<Vec<u8>, AssetError> {
    std::fs::read(path).map_err(|e| AssetError {
        path: path.to_path_buf(),
        reason: format!("读取失败: {e}"),
    })
}

/// 读 + 解码 PNG → RGBA8（返回 (像素, w, h)）。解码失败错误含完整路径。
pub fn read_png_rgba(path: &Path) -> Result<(Vec<u8>, u32, u32), AssetError> {
    let bytes = read_file(path)?;
    let img = image::load_from_memory(&bytes)
        .map_err(|e| AssetError {
            path: path.to_path_buf(),
            reason: format!("PNG 解码失败: {e}"),
        })?
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err(AssetError {
            path: path.to_path_buf(),
            reason: "图像尺寸为空".to_string(),
        });
    }
    Ok((img.into_raw(), w, h))
}

/// 资源根持有者：路径解析 + 类别加载 + 缺素材登记 + 读缓存。
#[derive(Debug, Default)]
pub struct AssetManager {
    root: PathBuf,
    /// 路径 → 原始字节（读盘去重缓存）。
    cache: RefCell<HashMap<PathBuf, Arc<[u8]>>>,
    /// 已判定缺失的文件（完整路径）；每个只 error 一次，汇总查表用。
    missing: RefCell<HashSet<PathBuf>>,
}

impl AssetManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            cache: RefCell::new(HashMap::new()),
            missing: RefCell::new(HashSet::new()),
        }
    }

    // ---- 资源根解析（收编 app.rs assets_dir / mcv_audio default_sounds_dir）----

    /// 纯解析函数（单测入口）：按序返回首个存在且含 `textures/` 的候选根。
    /// `base` = Android 解包出的 internal data 目录（调用方在 android cfg 里传），
    /// `cwd`/`exe_dir` = 桌面两形态的工作目录与 exe 同级目录。
    pub fn resolve_root_in(
        base: Option<&Path>,
        cwd: &Path,
        exe_dir: Option<&Path>,
    ) -> Option<PathBuf> {
        let mut cands: Vec<PathBuf> = Vec::new();
        if let Ok(env) = std::env::var("MCV_ASSETS_DIR") {
            cands.push(PathBuf::from(env));
        }
        if let Some(base) = base {
            cands.push(base.join(ROOT_REL));
        }
        cands.push(cwd.join(ROOT_REL));
        if let Some(exe) = exe_dir {
            cands.push(exe.join(ROOT_REL));
        }
        // 有效性门：资源根必须带 textures/（原版树骨架），防空目录假命中。
        cands
            .into_iter()
            .find(|c| c.is_dir() && c.join("textures").is_dir())
    }

    /// 运行期解析：以当前 CWD 与 exe 目录调 [`Self::resolve_root_in`]。
    /// 未解析到返回 None——资源根缺失是最高级别的素材硬错误，调用方必须
    /// 显式 `log::error!`（见 app.rs `assets_dir` 调用点），不得静默。
    pub fn resolve_root(android_data: Option<&Path>) -> Option<PathBuf> {
        let cwd = std::env::current_dir().unwrap_or_default();
        let exe_dir = std::env::current_exe()
            .ok()?
            .parent()
            .map(Path::to_path_buf);
        Self::resolve_root_in(android_data, &cwd, exe_dir.as_deref())
    }

    /// 音效根：`root/sounds`（原版 jar 布局；ci/fetch-sounds.sh 产物）。
    /// 另兼容 `MCV_SOUNDS_DIR` 显式覆盖（开发机自选目录）。
    pub fn resolve_sounds_root(root: Option<&Path>) -> Option<PathBuf> {
        if let Ok(env) = std::env::var("MCV_SOUNDS_DIR") {
            let p = PathBuf::from(env);
            if p.is_dir() {
                return Some(p);
            }
        }
        root.map(|r| r.join("sounds")).filter(|p| p.is_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 资源根下相对路径 → 完整路径（错误信息/登记一律用完整路径）。
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    // ---- 读取（缓存去重 + 硬错误登记）----

    /// 必需素材读取：缓存命中零磁盘；缺失 → `AssetError`（含完整路径）+
    /// 一次性 `log::error!` + 登记 `missing`（`summary_report` 汇总）。
    pub fn read(&self, rel: &str) -> Result<Arc<[u8]>, AssetError> {
        self.read_path(&self.path(rel))
    }

    /// best-effort 素材（物品图标/容器面板/粒子帧等）：缺失返回 None，
    /// 一次性 `log::warn!`（含完整路径）+ 登记 `missing`。上层按素材红线
    /// 降级（跳过该条目/占位），绝不伪造。
    pub fn read_optional(&self, rel: &str) -> Option<Arc<[u8]>> {
        let path = self.path(rel);
        let hit = self.cache.borrow().get(&path).cloned();
        if let Some(b) = hit {
            return Some(b);
        }
        match read_file(&path) {
            Ok(bytes) => {
                let arc: Arc<[u8]> = Arc::from(bytes);
                self.cache.borrow_mut().insert(path, arc.clone());
                Some(arc)
            }
            Err(_) => {
                if self.missing.borrow_mut().insert(path.clone()) {
                    log::warn!(
                        "可选素材缺失（该条目降级，无程序化回退）: {}",
                        path.display()
                    );
                }
                None
            }
        }
    }

    /// 完整路径版读取（atlas 等按 PathBuf 遍历的加载器共用缓存与登记）。
    pub fn read_path(&self, path: &Path) -> Result<Arc<[u8]>, AssetError> {
        let hit = self.cache.borrow().get(path).cloned();
        if let Some(b) = hit {
            return Ok(b);
        }
        let bytes = read_file(path).inspect_err(|e| {
            // 硬错误：每个路径只报一次（去重防刷屏），summary_report 再汇总。
            if self.missing.borrow_mut().insert(path.to_path_buf()) {
                log::error!("{e}");
            }
        })?;
        let arc: Arc<[u8]> = Arc::from(bytes);
        self.cache
            .borrow_mut()
            .insert(path.to_path_buf(), arc.clone());
        Ok(arc)
    }

    // ---- 类别加载 ----

    /// **图集重建（rebuild() 语义）**：源贴图 → `atlas::LAYERS`(838) 层
    /// payload（mip0+mip1 连续，`atlas::TILE_PX`=16）。缺失源贴图由
    /// mcv_core::atlas 保留原版 missing 标记并计数报错（素材红线），
    /// 完整缺失路径另可用 [`atlas::missing_source_files`] 查询。
    /// 调用方（mcv_render::gpu）按 `split_layer_counts` 拆数组换纹理；
    /// GPU 热替换薄接口见 `Renderer::rebuild_atlas`（留桩，注明）。
    pub fn rebuild_atlas_payload(&self) -> Vec<u8> {
        atlas::generate_payload_with_pack(Some(&self.root))
    }

    /// 方块图集缺失源贴图清单（827 真实层 + 10 裂纹档，完整路径）。
    pub fn atlas_missing(&self) -> Vec<PathBuf> {
        atlas::missing_source_files(&self.root)
    }

    /// 天体 payload（1 太阳 + 8 月相 → 9 层 32x32 RGBA）。任一必需文件
    /// 缺失/解码失败返回 None（错误路径已由 `read` 登记/报错）。
    /// 布局与解码实现在 [`celestial`] 模块（本 crate，纯 IO/解码）。
    pub fn celestial_payload(&self) -> Option<Vec<u8>> {
        celestial::load_payload_via(self)
    }

    /// 生物群系染色 colormap 原始 PNG（grass/foliage）。
    pub fn colormap(&self, file: &str) -> Result<Arc<[u8]>, AssetError> {
        self.read(&format!("textures/colormap/{file}"))
    }

    /// 本资源根的音效目录（不存在 → None，mcv_audio 侧静默降级由调用方
    /// 显式 log，见 app.rs `sounds_dir`）。
    pub fn sounds_dir(&self) -> Option<PathBuf> {
        Self::resolve_sounds_root(Some(&self.root))
    }

    // ---- 缺失汇总（一次性上报，杜绝各处各自静默）----

    /// 已登记缺失文件的排序快照（诊断/断言用）。
    pub fn missing(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.missing.borrow().iter().cloned().collect();
        v.sort();
        v
    }

    /// 一次性汇总上报：全部缺失路径（前 20 条展开）+ 总数，单条
    /// `log::error!`。初始化尾部（Renderer 构造后 / app 资产初始化后）调一次。
    /// 返回缺失总数（0 = 全部就位）。
    pub fn summary_report(&self) -> usize {
        let miss = self.missing();
        if miss.is_empty() {
            log::info!(
                "AssetManager: 素材全部就位（资源根 {}）",
                self.root.display()
            );
            return 0;
        }
        let shown: Vec<String> = miss
            .iter()
            .take(20)
            .map(|p| p.display().to_string())
            .collect();
        log::error!(
            "AssetManager: 缺失素材 {}/∞（硬错误，素材红线无程序化回退；资源根 {}）: {}{}",
            miss.len(),
            self.root.display(),
            shown.join(", "),
            if miss.len() > 20 { ", …" } else { "" }
        );
        miss.len()
    }
}

/// 天体贴图（太阳/月相）payload 装配——从 mcv_render::celestial 下沉
/// （M8c：纯 IO/解码，无 wgpu）。原版 26.1 依据保留在 mcv_render 模块注释：
/// 太阳/月亮是贴图 quad（SkyRenderer.java:125-127/:149-157），26.1 月相为
/// 每相独立文件（moon/<serializedName>.png，图集目录 environment/celestial，
/// AtlasProvider.java:162 + celestials.json）。
pub mod celestial {
    use super::{AssetManager, CELESTIAL_MOON_DIR, CELESTIAL_MOON_FILES, CELESTIAL_SUN};

    /// 纹理数组层数：1 太阳 + 8 月相。
    pub const CELESTIAL_LAYERS: usize = 9;
    /// 每层边长（原版 sun.png / moon/*.png 均 32x32）。
    pub const CELESTIAL_PX: usize = 32;
    /// 太阳层数组层号。
    pub const SUN_LAYER: u32 = 0;
    /// 月相 p（MoonPhase.index()）的数组层号。
    pub const MOON_LAYER_BASE: u32 = 1;

    /// 近邻重采样 RGBA 到 32x32。
    fn resample_to_layer(src: &[u8], sw: u32, sh: u32, layer: usize, dst: &mut [u8]) {
        let w = CELESTIAL_PX as u32;
        for y in 0..w {
            for x in 0..w {
                let sx = (u64::from(x) * u64::from(sw) / u64::from(w)) as u32;
                let sy = (u64::from(y) * u64::from(sh) / u64::from(w)) as u32;
                let s = ((sy * sw + sx) * 4) as usize;
                let d = layer * CELESTIAL_PX * CELESTIAL_PX * 4 + ((y * w + x) * 4) as usize;
                dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
            }
        }
    }

    /// 读原版天体贴图拼 mip0 载荷（9 层 32x32 RGBA 连续）。经
    /// [`AssetManager`]（缓存 + 缺素材登记/硬错误）。任一必需文件缺失/
    /// 解码失败返回 None（调用方按素材红线上传透明占位，不画假天体）。
    pub fn load_payload_via(assets: &AssetManager) -> Option<Vec<u8>> {
        let mut payload = vec![0u8; CELESTIAL_LAYERS * CELESTIAL_PX * CELESTIAL_PX * 4];
        let sun = assets.read(CELESTIAL_SUN).ok()?;
        if !decode_layer(&sun, SUN_LAYER as usize, &mut payload) {
            return None;
        }
        for (i, name) in CELESTIAL_MOON_FILES.iter().enumerate() {
            let rel = format!("{CELESTIAL_MOON_DIR}/{name}.png");
            let bytes = assets.read(&rel).ok()?;
            if !decode_layer(&bytes, MOON_LAYER_BASE as usize + i, &mut payload) {
                return None;
            }
        }
        Some(payload)
    }

    /// 旧签名兼容壳（mcv_render::celestial 与测试用）：dir 上临时建 manager。
    pub fn load_payload(assets_dir: &std::path::Path) -> Option<Vec<u8>> {
        load_payload_via(&AssetManager::new(assets_dir))
    }

    fn decode_layer(bytes: &[u8], layer: usize, payload: &mut [u8]) -> bool {
        let Ok(img) = image::load_from_memory(bytes) else {
            log::warn!("celestial: PNG 解码失败");
            return false;
        };
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        if w == 0 || h == 0 {
            return false;
        }
        resample_to_layer(rgba.as_raw(), w, h, layer, payload);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("mcv_assets_{tag}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        dir
    }

    #[test]
    fn resolve_root_in_order_and_validity() {
        // 空目录（无 textures/）不算有效根；首个有效候选生效；base 优先于 cwd。
        let base = tmp_dir("r_base");
        let cwd = tmp_dir("r_cwd");
        std::fs::create_dir_all(base.join(ROOT_REL).join("textures")).unwrap();
        std::fs::create_dir_all(cwd.join(ROOT_REL)).unwrap(); // 无 textures → 无效
        let hit = AssetManager::resolve_root_in(Some(&base), &cwd, None).expect("base 根应命中");
        assert!(hit.starts_with(&base));
        // 全无效 → None
        let bad = tmp_dir("r_bad");
        assert!(AssetManager::resolve_root_in(None, &bad, None).is_none());
        for d in [base, cwd, bad] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn read_missing_is_hard_error_with_full_path() {
        // 缺素材硬错误断言：临时空目录 → Err 且错误文本含完整路径。
        let root = tmp_dir("hard");
        std::fs::create_dir_all(root.join("textures")).unwrap();
        let am = AssetManager::new(&root);
        let e = am
            .read("textures/block/stone.png")
            .expect_err("空目录读取必须硬错误");
        let msg = e.to_string();
        assert!(
            msg.contains(root.join("textures/block/stone.png").to_str().unwrap()),
            "错误必须含完整路径: {msg}"
        );
        assert_eq!(am.missing().len(), 1);
        // 去重登记：再读不翻倍。
        let _ = am.read("textures/block/stone.png");
        assert_eq!(am.missing().len(), 1);
        assert_eq!(am.summary_report(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn empty_root_lists_every_atlas_source_as_missing() {
        // 空资源根（仅 textures/ 骨架）：atlas 缺失清单 = 827 真实层 + 10 裂纹档。
        let root = tmp_dir("miss_all");
        std::fs::create_dir_all(root.join("textures")).unwrap();
        let am = AssetManager::new(&root);
        let miss = am.atlas_missing();
        assert_eq!(
            miss.len(),
            atlas::REAL_TILE_COUNT + atlas::CRACK_LAYERS,
            "空根应列出全部源贴图缺失"
        );
        assert!(miss.iter().all(|p| p.starts_with(&root)));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_cache_dedups() {
        // 内部缓存去重：读两次后磁盘内容改写，仍返回首次缓存字节。
        let root = tmp_dir("cache");
        std::fs::create_dir_all(root.join("t")).unwrap();
        let f = root.join("t/a.png");
        std::fs::write(&f, b"first").unwrap();
        let am = AssetManager::new(&root);
        let a = am.read("t/a.png").unwrap();
        std::fs::write(&f, b"second!").unwrap();
        let b = am.read("t/a.png").unwrap();
        assert_eq!(&*a, &*b);
        assert_eq!(&*b, b"first");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn atlas_rebuild_on_repo_assets() {
        // 仓库资源根（已提交原版贴图）：rebuild 出 838 层 payload，
        // 且图集源贴图零缺失（素材红线守护：真实贴图全覆盖）。
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/minecraft");
        let am = AssetManager::new(&root);
        assert!(
            am.atlas_missing().is_empty(),
            "仓库内图集源贴图应零缺失: {:?}",
            am.atlas_missing().first()
        );
        let payload = am.rebuild_atlas_payload();
        let mip0 = atlas::LAYERS * atlas::TILE_PX * atlas::TILE_PX * 4;
        let mip1 = atlas::LAYERS * 8 * 8 * 4;
        assert_eq!(payload.len(), mip0 + mip1);
        // split 方案与 payload 层数自洽（GLES 拆分接口留在此处一起锁）。
        assert_eq!(
            atlas::split_layer_counts(atlas::LAYERS),
            vec![atlas::LAYERS]
        );
        assert!(am.missing().is_empty());
    }
}
