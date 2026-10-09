本目录 OGG 为 Mojang 原版音效(取自 26.1 资源索引 index 30,按 sha1 校验自官方 CDN),仅限开发期使用;发布 tag 前必须删除整个 sounds/ 目录。

与 texturepack/ 同等待遇:入库仅作开发期便利,不得随发行版分发。

文件命名 = 原版资产路径扁平化(如 dig/stone1.ogg → dig_stone1.ogg),原版资产路径对应 26.1 sounds.json 事件名的变体(如 dig/stone1 属 block.stone.break/place/hit)。引擎按原版事件名寻址(engine/mcv_audio 的 SoundTable 解析 sounds.json);本目录 12 个扁平文件仅作单测 fixture 与开发期兜底,全量原版音效树(原版目录树 + sounds.json)由 ci/fetch-sounds.sh 构建时拉取,不入 git。注意 26.1 已无 dig/dirt、game/player/hurt、entity/generic/hurt:泥土复用 grass 音组、玩家受伤对应 damage/hit* 组。
