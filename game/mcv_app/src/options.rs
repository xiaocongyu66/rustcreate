//! 设置持久化：手写 `key=value` 行文本格式（同 `KeyMap::to_text` 风格，
//! 不用 serde）。`options.txt` 与 `keybindings.txt` 均放 saves 根目录，
//! 与世界文件夹同级。

use std::path::Path;

/// 渲染距离持久化钳制范围（与 app.rs 设置按钮步进范围一致）。
/// 原版对照：`IntRange(2, maxMemory≥1GB ? 32 : 16)`（Options.java:1407，
/// 档位常量 4/12/16/32 见 :81-84）。本仓 4..16 = 设置 UI 步进域裁剪
/// （原版下限 2 上限 32/16），差异登记为裁剪债（#115 刀 5b）。
const DIST_MIN: i32 = 4;
const DIST_MAX: i32 = 16;

/// 可持久化设置（对应 AppState 的 set_* 字段）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub render_dist: i32,
    pub sens: f32,
    /// 官方 CloudStatus：0=OFF 1=FAST 2=FANCY
    pub clouds: u8,
    /// 界面语言：0=English 1=中文
    pub lang: u8,
}

impl Default for Options {
    /// 桌面 8（引入持久化前的硬编码默认，headless 测试基线不动）；
    /// Android 6（#115 刀 5b：移动端内存预算裁剪——vd6 resident ≈ 63.4 MiB
    /// vs vd8 ≈ 101.7 MiB，算式见 docs/memory-budget.md）。
    /// 原版对照：默认恒 12（Options.java:1405-1409，IntRange(2, 大内存
    /// ?32:16)、无平台分支，弱机靠玩家自调）——本仓平台分支是移动端
    /// resident 预算的主动裁剪，登记为裁剪债。
    fn default() -> Self {
        Self {
            render_dist: if cfg!(target_os = "android") { 6 } else { 8 },
            sens: 1.0,
            clouds: 2,
            lang: 0,
        }
    }
}

/// 写文本文件：父目录不存在则自动创建。
pub fn write_text(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

impl Options {
    pub fn to_text(&self) -> String {
        format!(
            "render_dist={}\nsens={:.4}\nclouds={}\nlang={}\n",
            self.render_dist, self.sens, self.clouds, self.lang
        )
    }

    /// 解析：空行、`#` 注释、未知 key、格式错误 → 跳过该行；越界值 → 钳制。
    pub fn from_text(text: &str) -> Self {
        let mut o = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "render_dist" => {
                    if let Ok(n) = v.parse::<i32>() {
                        o.render_dist = n.clamp(DIST_MIN, DIST_MAX);
                    }
                }
                "sens" => {
                    if let Ok(f) = v.parse::<f32>()
                        && f.is_finite()
                    {
                        o.sens = f.clamp(0.25, 3.0);
                    }
                }
                "clouds" => {
                    if let Ok(n) = v.parse::<u8>() {
                        o.clouds = n % 3;
                    }
                }
                "lang" => {
                    if let Ok(n) = v.parse::<u8>() {
                        o.lang = n % 2;
                    }
                }
                _ => {}
            }
        }
        o
    }

    /// 从磁盘加载；文件缺失/不可读 → 全默认值。
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).map_or_else(|_| Self::default(), |t| Self::from_text(&t))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_text(path, &self.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        // render_dist=12 = 原版默认对照（Options.java:1409），锁「对照值
        // 可无损往返」；本仓桌面默认 8 / 移动默认 6 = 内存预算裁剪债
        // （Default 文档）。语义不许删弱：12 必须原样往返不落钳制。
        let o = Options {
            render_dist: 12,
            sens: 2.25,
            clouds: 1,
            lang: 1,
        };
        assert_eq!(Options::from_text(&o.to_text()), o);
        // 默认值也可无损往返
        assert_eq!(
            Options::from_text(&Options::default().to_text()),
            Options::default()
        );
    }

    #[test]
    fn malformed_lines_skipped() {
        let text = "# comment\n\nthis is garbage\nrender_dist=abc\nsens=\n=novalue\nlang=1\n";
        let o = Options::from_text(text);
        assert_eq!(o.lang, 1);
        assert_eq!(o.render_dist, Options::default().render_dist);
        assert_eq!(o.sens, Options::default().sens);
        assert_eq!(o.clouds, Options::default().clouds);
    }

    #[test]
    fn out_of_range_clamped() {
        let o = Options::from_text("render_dist=999\nsens=-5\nclouds=9\nlang=5\n");
        assert_eq!(o.render_dist, DIST_MAX);
        assert_eq!(o.sens, 0.25);
        assert_eq!(o.clouds, 0);
        assert_eq!(o.lang, 1);
        let o = Options::from_text("render_dist=1\nsens=99\n");
        assert_eq!(o.render_dist, DIST_MIN);
        assert_eq!(o.sens, 3.0);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let path = std::path::Path::new("/nonexistent/mcv-options-test/options.txt");
        assert_eq!(Options::load(path), Options::default());
    }

    #[test]
    fn save_load_roundtrip_on_disk() {
        // 按进程号隔离，避免并发测试互踩。
        // render_dist=6 与 Android 默认同值纯属巧合（本测锁的是磁盘
        // 往返语义，不依赖默认值；桌面默认 8，见 Default 文档）。
        let dir = std::env::temp_dir().join(format!("mcv_options_test_{}", std::process::id()));
        let path = dir.join("options.txt");
        let o = Options {
            render_dist: 6,
            sens: 0.5,
            clouds: 0,
            lang: 1,
        };
        o.save(&path).unwrap();
        assert_eq!(Options::load(&path), o);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
