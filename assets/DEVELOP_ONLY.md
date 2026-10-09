# DEVELOP_ONLY — 单一资源根（开发期 Mojang 素材）

本目录是引擎唯一资源根，目录布局 1:1 镜像官方 jar 的 `assets/` 命名空间树。
版权归 Mojang，**发布 tag 前必须整目录删除或替换**；rustcreate 公开仓库的
release 产物绝不含本目录内容。代码一律按 MC 资产路径读取
（`assets/minecraft/<与 jar 相同的路径>`），不做逐文件搬运接线。

## minecraft/（唯一命名空间）

26.1 原版资产树（textures/models/blockstates/lang/atlases/font/gui…）：
- textures/block|item|gui|entity|environment|font/** — 原版 PNG；GUI 精灵在
  `textures/gui/sprites/**`，ASCII 字体在 `textures/font/ascii.png`，云在
  `textures/environment/clouds.png`，玩家在 `textures/entity/player/{wide,slim}/`。
- textures/item/** — 物品图标（快捷栏/HUD 物品渲染数据源）。
- models/**、blockstates/** — gen-blocks 管道解析逐面贴图。
- lang/** — i18n 数据源。
- sounds/** + sounds.json — 全量原版音效树，**不入 git**，由
  `ci/fetch-sounds.sh` 在 dev 构建/CI 时按 index 30 从官方 CDN 拉取
  （sha1+size 双校验）。发布路径（workflow_call）不拉取。

## 退役目录（2026-10 合并）

- `assets_vanilla/` → 本目录 `minecraft/`。
- `texturepack/` → 全部内容为 `minecraft/` 树内文件的字节级拷贝
  （gui 精灵/云/皮肤/ASCII 字体/方块贴图），代码改读原版路径后整体删除。
- 顶层 `sounds/` → `minecraft/sounds/`；原 12 个扁平开发 ogg 已删，
  音频单测 fixture 改引用真实树路径（未 fetch 时测试打印 skip 通过）。
