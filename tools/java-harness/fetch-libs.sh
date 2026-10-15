#!/usr/bin/env python3
"""重建 libs/：piston-meta 26.1 精确清单（README 版本锚定段——DFU 9.0.19，
26.2 bundler 的 DFU 10 有 154 假错+静默语义风险）。逐 jar sha1 校验。
需要网络（piston-meta/piston-data）——网关间歇断时重跑即可（断点续传）。"""
import hashlib, json, os, sys, urllib.request

META = "https://piston-meta.mojang.com/v1/packages/8667c5278895a6ae31e05ab99ba044ef33fca092/26.1.json"
os.makedirs("libs", exist_ok=True)

meta = json.load(urllib.request.urlopen(META, timeout=60))
srv = meta["downloads"]["server"]["url"]
os.makedirs("libs/nested", exist_ok=True)
def fetch(url, dst):
    if os.path.exists(dst) and os.path.getsize(dst) > 0:
        return
    print("fetch", os.path.basename(dst))
    urllib.request.urlretrieve(url, dst)

fetch(srv, "libs/server-26.1.jar")
n = ok = 0
for lib in meta["libraries"]:
    if "artifact" not in lib:
        continue
    dl = lib["artifact"]
    if "linux" in lib.get("rules", []) and not any(r["os"]=="linux" for r in lib["rules"] if r.get("action")=="allow"):
        pass
    p = "libs/nested/" + dl["path"]
    os.makedirs(os.path.dirname(p), exist_ok=True)
    fetch(dl["url"], p)
    h = hashlib.sha1(open(p, "rb").read()).hexdigest()
    if h != dl["sha1"]:
        print("SHA1 MISMATCH", p); sys.exit(1)
    ok += 1
    n += 1
    print(f"\r{ok} ok", end="", flush=True)
print(f"\nlibs ready: server + {n} jars")
