//! 首启把 APK assets/（CI 打包的单一资源根 `assets/`）解到 internal data 目录。
//!
//! manifest.txt 由 CI 在打 APK 前生成并随包携带；stamp 记录已解包的
//! manifest 内容，素材更新（manifest 变化）时自动重解。

use android_activity::AndroidApp;
use std::ffi::CString;
use std::io::Read;
use std::path::Path;

/// 只解这些前缀，防 manifest 被塞任意路径。
const ALLOW_PREFIXES: &[&str] = &["assets/"];

pub fn extract_assets(app: &AndroidApp, data: &Path) {
    let am = app.asset_manager();
    let read = |name: &str| -> Option<Vec<u8>> {
        let mut asset = am.open(&CString::new(name).ok()?)?;
        let mut buf = Vec::with_capacity(asset.length());
        asset.read_to_end(&mut buf).ok()?;
        Some(buf)
    };
    let Some(manifest) = read("manifest.txt") else {
        log::info!("no assets/manifest.txt in apk, skip extract");
        return;
    };
    let stamp = data.join("assets.stamp");
    if std::fs::read(&stamp).ok().as_deref() == Some(manifest.as_slice()) {
        return;
    }
    // 失败绝不落 stamp：否则首启解包残缺会被 stamp 永久缓存（下次启动
    // 直接命中缓存跳过重解，缺素材变成不可自愈的死局——2026-10-10 审计
    // 指出的静默降级洞）。
    let mut n = 0usize;
    let mut failed: Vec<&str> = Vec::new();
    for line in String::from_utf8_lossy(&manifest).lines() {
        let name = line.trim();
        if name.is_empty() || !ALLOW_PREFIXES.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let Some(bytes) = read(name) else {
            failed.push(name);
            continue;
        };
        let dest = data.join(name);
        if let Some(dir) = dest.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&dest, bytes).is_ok() {
            n += 1;
        } else {
            failed.push(name);
        }
    }
    if failed.is_empty() {
        let _ = std::fs::write(&stamp, &manifest);
        log::info!("extracted {n} asset files to {}", data.display());
    } else {
        log::error!(
            "asset extract INCOMPLETE: {n} ok, {} failed (first: {}); stamp NOT written, next launch retries",
            failed.len(),
            failed[0]
        );
    }
}
