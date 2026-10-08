#!/usr/bin/env python3
# DEVELOP_ONLY 数据管道（同 texturepack / data/lang 素材政策）：
#   下载 Unifont（https://unifoundry.com/ ，SIL OFL 1.1 授权开源字体，
#   允许再分发，非 Mojang 版权内容），提取中文渲染所需字形，生成引擎
#   运行时位图字体数据 game/mcv_app/data/font/cjk.f16。
#   发布前按 DEVELOP_ONLY 政策删除或替换（见 data/font/DEVELOP_ONLY.md）。
#
# 用法（仓库根目录）:  python3 ci/gen-unifont.py [unifont_all.hex]
#   无参数时自动下载（缓存于 ci/.cache/，加入 .gitignore 语义同 gen-lang）；
#   也可传入已下载的 unifont hex 文本路径。
#
# 覆盖范围（MC 26.1 unifont 方案的中文子集）:
#   U+3000..U+303F  CJK 标点（。、《》等）
#   U+4E00..U+9FFF  CJK 统一表意文字（基本区全量 20992 字）
#   U+FF00..U+FFEF  半角/全角形式（！？（）ＡＢＣ等）
#   量化决策：三区共 ~21.3k 字形全部收录。全宽字形 32B 位图 + 5B 索引
#   ≈ 690KiB，与 MC 官方 jar 内 unifont.bin（518KiB，全 Unicode）同量级，
#   开发期可接受；如需发布期瘦身，把 CJK 区裁到常用 6-8k 字 ≈ 250KiB。
#   半宽字形（hex 32 字符 = 8 列宽，如 FF01 区的半角形式）位图减半存 16B。
#
# 输出格式 cjk.f16（全小端；位图行内保持 big-endian 位序，见下）:
#   头 16B: magic "FCF1" | u32 version=1 | u32 count | u32 half_count
#   索引区 count*5B，按 cp 升序（可整体二分）:
#     前 half_count 条为半宽条目(u8 flags=1)，其后为全宽条目(flags=0)。
#     条目 = (u32 cp_le, u8 flags)。位图偏移免表：
#     半宽条目 i（区序号）→ 半宽位图区 i*16；全宽条目 j → 全宽区 j*32。
#   半宽位图区: 每字形 16B = 16 行 × u8，bit7 = 最左像素（8 列）。
#   全宽位图区: 每字形 32B = 16 行 × u16，按 big-endian 字节书写，
#              首字节 bit7 = 最左像素（沿用 unifont/MC unifont.bin 惯例）。
import pathlib
import struct
import sys
import urllib.request
import gzip

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "game/mcv_app/data/font/cjk.f16"
CACHE = pathlib.Path(__file__).resolve().parent / ".cache"

VERSION = "16.0.04"
URLS = [
    f"https://ftpmirror.gnu.org/unifont/unifont-{VERSION}/unifont_all-{VERSION}.hex.gz",
    f"https://download.savannah.nongnu.org/releases/unifont/unifont-{VERSION}/unifont_all-{VERSION}.hex.gz",
    f"https://unifoundry.com/pub/unifont/unifont-{VERSION}/font-builds/unifont_all-{VERSION}.hex.gz",
]
RANGES = [(0x3000, 0x303F), (0x4E00, 0x9FFF), (0xFF00, 0xFFEF)]


def fetch_hex() -> str:
    if len(sys.argv) > 1:
        return pathlib.Path(sys.argv[1]).read_text(encoding="ascii")
    cached = CACHE / f"unifont_all-{VERSION}.hex"
    if cached.exists():
        return cached.read_text(encoding="ascii")
    CACHE.mkdir(parents=True, exist_ok=True)
    last = None
    for url in URLS:
        try:
            print(f"downloading {url}")
            raw = urllib.request.urlopen(url, timeout=60).read()
            text = gzip.decompress(raw).decode("ascii")
            cached.write_text(text, encoding="ascii")
            return text
        except Exception as e:  # noqa: BLE001 - 逐镜像重试
            last = e
            print(f"  failed: {e}")
    sys.exit(f"unifont 下载彻底失败（不生成假数据）: {last}")


def main() -> None:
    glyphs = {}   # cp -> (halfwidth, rows)，rows = 16 个 int，MSB=最左像素
    for line in fetch_hex().splitlines():
        line = line.strip()
        if not line:
            continue
        key, hexbits = line.split(":", 1)
        if "-" in key:  # 本版本无 range 行，防御性跳过
            continue
        cp = int(key, 16)
        if not any(a <= cp <= b for a, b in RANGES):
            continue
        if len(hexbits) == 32:      # 8x16 半宽 → 统一到 16 位行，高 8 位有效
            glyphs[cp] = (True, [int(hexbits[r * 2:r * 2 + 2], 16) << 8
                                 for r in range(16)])
        elif len(hexbits) == 64:    # 16x16 全宽
            glyphs[cp] = (False, [int(hexbits[r * 4:r * 4 + 4], 16)
                                  for r in range(16)])

    cps = sorted(glyphs)
    half = [cp for cp in cps if glyphs[cp][0]]
    full = [cp for cp in cps if not glyphs[cp][0]]

    out = bytearray()
    out += b"FCF1"
    out += struct.pack("<III", 1, len(cps), len(half))
    for cp in half:
        out += struct.pack("<IB", cp, 1)
    for cp in full:
        out += struct.pack("<IB", cp, 0)
    for cp in half:
        for row in glyphs[cp][1]:
            out.append(row >> 8)
    for cp in full:
        for row in glyphs[cp][1]:
            out += struct.pack(">H", row)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(bytes(out))
    print(f"wrote {OUT} ({len(out)} bytes): {len(half)} halfwidth + "
          f"{len(full)} fullwidth glyphs")


if __name__ == "__main__":
    main()
