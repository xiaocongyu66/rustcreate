本目录 OGG 为 Mojang 原版音效(取自 26.1 资源索引 index 30,按 sha1 校验自官方 CDN),仅限开发期使用;发布 tag 前必须删除整个 sounds/ 目录。

与 texturepack/ 同等待遇:入库仅作开发期便利,不得随发行版分发。

文件命名 = 原版资产路径扁平化(如 dig/stone1.ogg → dig_stone1.ogg),完整映射见 crates/mcv_audio 的 SoundId::vanilla_path()。注意 26.1 已无 dig/dirt、game/player/hurt、entity/generic/hurt:泥土复用 grass 音组、玩家/通用受伤对应 damage/hit* 组(详见 SoundId 文档注释)。
