#!/usr/bin/env bash
# 构建时从 Mojang 官方 CDN 拉全量原版音效到 sounds/（原版目录树，不落 git）。
# 仓库内只保留本脚本 + 少量扁平开发音效；dev 构建（push main）调用，
# release 路径不调用（DEVELOP_ONLY 约定：发行产物不得含 Mojang 资产）。
# 用法: ci/fetch-sounds.sh [输出目录=sounds]
set -euo pipefail
OUT="${1:-sounds}"
# MC 26.1 资产索引 index 30（piston-meta，按内容寻址 URL → 不可变、可复现）。
INDEX_URL="https://piston-meta.mojang.com/v1/packages/a1969c2dd99745486cbd873da16792b3b2381504/30.json"
CACHE="${FETCH_SOUNDS_CACHE:-ci/.cache/fetch-sounds}"
mkdir -p "$CACHE" "$OUT"
[ -f "$CACHE/30.json" ] || curl -fsSL --retry 3 "$INDEX_URL" -o "$CACHE/30.json"
python3 - "$CACHE/30.json" "$OUT" <<'PY'
import json, sys, os, hashlib, urllib.request, concurrent.futures as cf

index_path, out = sys.argv[1], sys.argv[2]
objects = json.load(open(index_path))["objects"]
tasks = []
for path, obj in objects.items():
    if path == "minecraft/sounds.json" or path.startswith("minecraft/sounds/"):
        rel = path[len("minecraft/"):]
        tasks.append((obj["hash"], obj["size"], os.path.join(out, rel)))

def fetch(task):
    sha, size, dest = task
    url = f"https://resources.download.minecraft.net/{sha[:2]}/{sha}"
    if os.path.isfile(dest) and os.path.getsize(dest) == size:
        with open(dest, "rb") as f:
            if hashlib.sha1(f.read()).hexdigest() == sha:
                return 0  # 已就位，跳过
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    for _ in range(3):
        try:
            with urllib.request.urlopen(url, timeout=60) as r:
                data = r.read()
            break
        except OSError:
            continue
    else:
        raise RuntimeError(f"download failed: {url}")
    if len(data) != size or hashlib.sha1(data).hexdigest() != sha:
        raise RuntimeError(f"sha1/size mismatch: {url}")
    tmp = dest + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, dest)
    return 1

with cf.ThreadPoolExecutor(max_workers=16) as pool:
    new = sum(pool.map(fetch, tasks))
print(f"sounds: {len(tasks)} objects ({new} downloaded, {len(tasks) - new} cached) -> {out}/")
PY
