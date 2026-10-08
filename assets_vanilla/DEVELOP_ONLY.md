# DEVELOP_ONLY — 官方资产整包（开发期素材）

来源：26.1.jar 内嵌 assets 树（assets/minecraft/**）完整复制，另按官方资产
索引（index 30）补齐缺失贴图。版权归 Mojang。

政策（同 texturepack/）：**发布 tag 前必须整目录删除**；rustcreate 公开仓库
的 release 产物绝不含本目录。开发期用于逐像素还原外观（方块/物品/GUI 精灵/
模型 JSON/blockstates），代码只按 MC 资产路径读取，不做逐文件搬运接线。

- textures/**  原版 PNG（block/item/gui/entity/particle/misc…）
- models/**     客户端模型 JSON（gen-blocks 管道解析逐面贴图）
- blockstates/** 方块状态 → 模型映射
- lang/**       语言表（i18n 数据源）
- atlases/**    图集清单
