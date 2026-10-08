# DEVELOP_ONLY — data/font

本目录内容为**开发期素材**,发布构建前必须删除或替换为自维护字体
(政策同 `texturepack/DEVELOP_ONLY.md`、`data/lang/`)。

- `cjk.f16`:Unifont 16x16 位图字体的中文子集(U+3000-303F、
  U+4E00-9FFF、U+FF00-FFEF,21296 字形,~768KiB)。
  由 `python3 ci/gen-unifont.py` 从 Unifont 16.0.04 生成。
- 授权:Unifont 采用 SIL Open Font License 1.1
  (https://unifoundry.com/unifont.html ,GNU 镜像
  https://ftpmirror.gnu.org/unifont/)。OFL 允许自由再分发,字体本身
  非 Mojang 版权内容;仍按本仓库政策归入 DEVELOP_ONLY,以便发布版
  统一替换为自维护字形表(可裁剪到常用 6-8k 字,约 250KiB)。
- 运行时消费方:`mcv_render::unifont`(解析 + 排版 + HUD quad 生成),
  数据格式说明见 `ci/gen-unifont.py` 头注释。
