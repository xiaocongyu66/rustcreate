#!/usr/bin/env bash
# 构建时从 Mojang 官方 CDN 拉全量原版结构模板 .nbt 到 assets/minecraft/structure/
# （原版 jar 内 data/minecraft/structure*/ 树按相对路径铺开,不落 git）。
# 结构 .nbt 不在 index 30 资产清单里(那里只有 sounds/lang/font/textures),
# 只随版本 JAR 分发——因此本脚本走「piston-meta 版本清单(内容寻址)→
# client jar(sha1 校验+缓存)→ 本地解包」,其余纪律与 fetch-sounds.sh 一致。
# dev 构建（push main）调用，release 路径不调用（DEVELOP_ONLY 约定：
# 发行产物不得含 Mojang 资产）。
# 用法: ci/fetch-structures.sh [输出目录=assets/minecraft/structure]
set -euo pipefail
OUT="${1:-assets/minecraft/structure}"
# MC 26.1 版本清单(piston-meta,v1/packages 按 sha1 内容寻址 → 不可变、
# 可复现;清单内的 client jar URL 同样内容寻址,双重校验)。
VERSION_URL="https://piston-meta.mojang.com/v1/packages/a52c7511e13957149b53a175364c7ea7c4149bb8/26.1.json"
CACHE="${FETCH_STRUCTURES_CACHE:-ci/.cache/fetch-structures}"
mkdir -p "$CACHE" "$OUT"
[ -f "$CACHE/26.1.json" ] || curl -fsSL --retry 3 "$VERSION_URL" -o "$CACHE/26.1.json"
python3 - "$CACHE/26.1.json" "$OUT" "$CACHE" <<'PY'
import hashlib, json, os, sys, urllib.request, zipfile

version_manifest, out, cache = sys.argv[1], sys.argv[2], sys.argv[3]
meta = json.load(open(version_manifest))
client = meta["downloads"]["client"]
sha, size, url = client["sha1"], client["size"], client["url"]
jar = os.path.join(cache, f"{sha}.jar")
if not (os.path.isfile(jar) and os.path.getsize(jar) == size
        and hashlib.sha1(open(jar, "rb").read()).hexdigest() == sha):
    for _ in range(3):
        try:
            with urllib.request.urlopen(url, timeout=120) as r:
                data = r.read()
            break
        except OSError:
            continue
    else:
        raise RuntimeError(f"download failed: {url}")
    if len(data) != size or hashlib.sha1(data).hexdigest() != sha:
        raise RuntimeError(f"sha1/size mismatch: {url}")
    tmp = jar + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, jar)

PREFIXES = ("data/minecraft/structure/", "data/minecraft/structures/")
new = total = 0
with zipfile.ZipFile(jar) as z:
    names = [n for n in z.namelist()
             if n.endswith(".nbt") and n.startswith(PREFIXES)]
    for name in names:
        for p in PREFIXES:
            if name.startswith(p):
                rel = name[len(p):]
                break
        dest = os.path.join(out, rel)
        total += 1
        if os.path.isfile(dest) and os.path.getsize(dest) == z.getinfo(name).file_size:
            continue  # 已就位,跳过(jar 自身已 sha1 校验,内容可信)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        tmp = dest + ".tmp"
        with open(tmp, "wb") as f:
            f.write(z.read(name))
        os.replace(tmp, dest)
        new += 1
print(f"structures: {total} .nbt objects ({new} extracted, {total - new} cached) -> {out}/")
PY
