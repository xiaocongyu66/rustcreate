#!/usr/bin/env python3
# DEVELOP_ONLY 数据管道（同 ci/gen-blocks.py / assets/ 素材政策）：
#   从反编译 26.1 Java 源提取原版 SoundType 全表与「方块注册名 → SoundType」归属，
#   生成 engine/mcv_audio/src/sound_types.inc.rs（静态表，先例 blocks_gen.inc.rs）。
#
# 用法（仓库根目录）:
#   MCV_REF_SRC=<26.1解包树根> python3 ci/gen-soundtypes.py
#
# 数据源（仅本机参考、绝不入仓，路径经环境变量注入）：
#   $MCV_REF_SRC/net/minecraft/world/level/block/SoundType.java   —— 类型全表
#     （volume/pitch + dig/step/place/hit/fall 五事件，SoundEvents 常数引用）
#   $MCV_REF_SRC/net/minecraft/sounds/SoundEvents.java            —— 常数 → 事件名
#   $MCV_REF_SRC/net/minecraft/world/level/block/Blocks.java      —— 方块归属
#   $MCV_REF_SRC/net/minecraft/references/BlockIds.java           —— key 常数 → 注册名
#   $MCV_REF_SRC/net/minecraft/world/level/block/state/BlockBehaviour.java
#     —— 未显式 .sound() 的默认（:973 soundType = SoundType.STONE，仅取注释引用）
#
# 产物:
#   engine/mcv_audio/src/sound_types.inc.rs
#     SOUND_TYPES: [(&str, SoundType); N]（SoundType.java 声明序，含行号注释）
#     BLOCK_SOUND_TYPES: [(&str, u16); M]（注册名字典序升序，值 = SOUND_TYPES 下标；
#       未登记名由调用方按 BlockBehaviour.java:973 默认 STONE）
#   覆盖率自检：game/mcv_core/src/blocks_gen.inc.rs 全部名字必须直接或经旧名
#   别名命中（千块表 id 0..13 旧名见 ci/gen-blocks.py LEGACY），缺口即报错。
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REF = os.environ.get("MCV_REF_SRC")
if not REF:
    sys.exit("缺 MCV_REF_SRC=<26.1解包树根>（同 ci/gen-blocks.py）")
REF = Path(REF)
SOUND_TYPE_JAVA = REF / "net/minecraft/world/level/block/SoundType.java"
SOUND_EVENTS_JAVA = REF / "net/minecraft/sounds/SoundEvents.java"
BLOCKS_JAVA = REF / "net/minecraft/world/level/block/Blocks.java"
BLOCK_IDS_JAVA = REF / "net/minecraft/references/BlockIds.java"
GEN_BLOCKS = ROOT / "game/mcv_core/src/blocks_gen.inc.rs"
OUT = ROOT / "engine/mcv_audio/src/sound_types.inc.rs"

# 千块表 id 0..13 旧名 → 官方注册名（镜像 ci/gen-blocks.py LEGACY）。
LEGACY_ALIASES = {
    "grass": "grass_block",
    "snow_grass": "grass_block",
    "log": "oak_log",
    "leaves": "oak_leaves",
    "planks": "oak_planks",
    "cobble": "cobblestone",
    "flower_red": "poppy",
    "flower_yellow": "dandelion",
}


def read_lines(p: Path):
    return p.read_text(encoding="utf-8", errors="replace").splitlines()


# ---- SoundEvents.java：常数名 → 事件串 --------------------------------------
EVENT_RE = re.compile(r'public static final (?:SoundEvent|Holder\.Reference<SoundEvent>) (\w+) = register\w*\("([^"]+)"\)')
events = {}
for i, line in enumerate(read_lines(SOUND_EVENTS_JAVA), 1):
    m = EVENT_RE.search(line)
    if m:
        events[m.group(1)] = m.group(2)

# ---- SoundType.java：常数名 → (volume, pitch, 五事件, 行号) ------------------
# 声明形如 `public static final SoundType NAME = new SoundType(` … `);`，
# 参数无嵌套括号；行号取声明行。
ST_DECL = re.compile(r"public static final SoundType (\w+) = new SoundType\(")
sound_types = []  # (name, volume, pitch, [break, step, place, hit, fall], line)
lines = read_lines(SOUND_TYPE_JAVA)
i = 0
while i < len(lines):
    m = ST_DECL.search(lines[i])
    if m:
        name = m.group(1)
        body = []
        j = i
        while j < len(lines):
            body.append(lines[j])
            if ");" in lines[j]:
                break
            j += 1
        blob = " ".join(body)
        args = blob.split("new SoundType(", 1)[1].rsplit(");", 1)[0]
        parts = [a.strip() for a in args.split(",")]
        assert len(parts) == 7, f"SoundType {name}: 参数数 {len(parts)} != 7"
        vol, pitch = parts[0].removesuffix("F"), parts[1].removesuffix("F")
        evs = []
        for p in parts[2:]:
            cm = re.fullmatch(r"SoundEvents\.(\w+)", p)
            assert cm, f"SoundType {name}: 事件参数非 SoundEvents 常数: {p}"
            assert cm.group(1) in events, f"SoundType {name}: 未知事件常数 {cm.group(1)}"
            evs.append(events[cm.group(1)])
        sound_types.append((name, vol, pitch, evs, i + 1))
        i = j + 1
    else:
        i += 1
assert sound_types, "SoundType.java 解析为空"
st_index = {name: k for k, (name, *_) in enumerate(sound_types)}

# ---- Blocks.java：注册名 → SoundType 类型名 ---------------------------------
# 逐语句提取：`public static final Block CONST = …;`（括号深度 0 截断）。
# 优先级：显式 `.sound(SoundType.X)` > 助手参数裸 `SoundType.X`（logProperties/
# leavesProperties 等带参属性工厂）> ofLegacyCopy/ofFullCopy(BASE) 拷贝 >
# 助手 register（Bed→WOOD、StainedGlass→GLASS、Stair→BASE 拷贝）> 默认 STONE。
BLOCK_DECL = re.compile(r"public static final (?:Block|WeatheringCopperBlocks) (\w+) =")
WEATHERING_CREATE = re.compile(r"WeatheringCopperBlocks\.create\(")
# WeatheringCopperBlocks.create 注册 8 名（WeatheringCopperBlocks.java:14-46）：
# id / exposed_id / weathered_id / oxidized_id / waxed_id / waxed_exposed_id /
# waxed_weathered_id / waxed_oxidized_id（同一 properties 供应商 → 同一 SoundType）。
SOUND_DIRECT = re.compile(r"\.sound\(SoundType\.(\w+)\)")
SOUND_BARE = re.compile(r"SoundType\.(\w+)")
COPY_BASE = re.compile(r"of(?:Legacy|Full)Copy\((?:Blocks\.)?(\w+)\)")
STAIR_HELPER = re.compile(r"\bregister(?:Legacy)?Stair\(")
BED_HELPER = re.compile(r"\bregisterBed\(")
GLASS_HELPER = re.compile(r"\bregisterStainedGlass\(")

block_sound = {}  # 常数名 → 类型名
block_name = {}  # 常数名 → 注册名
block_lines = {}  # 常数名 → Blocks.java 行号
reg_line = {}  # 注册名 → Blocks.java 行号（首注册为准）
block_names_seen = set()

blines = read_lines(BLOCKS_JAVA)
i = 0
while i < len(blines):
    m = BLOCK_DECL.search(blines[i])
    if not m:
        i += 1
        continue
    const = m.group(1)
    start = i
    depth = 0
    blob = []
    j = i
    while j < len(blines):
        line = blines[j]
        depth += line.count("(") - line.count(")")
        blob.append(line)
        if depth <= 0 and ";" in line:
            break
        j += 1
    stmt = " ".join(blob)
    # 注册名：语句内第一个字符串字面量；否则 BlockIds 常数。
    nm = re.search(r'"([a-z0-9_/]+)"', stmt)
    ids = re.search(r"register\(\s*BlockIds\.(\w+)", stmt)
    reg = nm.group(1) if nm else None
    if reg is None and ids:
        reg = ids.group(1).lower()  # BlockIds 常数名 = 注册名大写（references/BlockIds.java）
    # WeatheringCopperBlocks.create：展开 8 个派生注册名（同一 SoundType）。
    if WEATHERING_CREATE.search(stmt) and reg:
        d0 = SOUND_DIRECT.search(stmt)
        assert d0, f"WeatheringCopperBlocks.create {reg}: 未找到 .sound()"
        ty0 = d0.group(1)
        for pfx in [
            "",
            "exposed_",
            "weathered_",
            "oxidized_",
            "waxed_",
            "waxed_exposed_",
            "waxed_weathered_",
            "waxed_oxidized_",
        ]:
            derived = pfx + reg
            block_names_seen.add(derived)
            block_sound.setdefault(f"__wc__{derived}", ty0)
            block_name[f"__wc__{derived}"] = derived
            block_lines[f"__wc__{derived}"] = start + 1
            reg_line.setdefault(derived, start + 1)
        i = j + 1
        continue
    # 音效归属。
    ty = None
    d = SOUND_DIRECT.search(stmt)
    if d:
        ty = d.group(1)
    if ty is None:
        b = SOUND_BARE.search(stmt)
        if b:
            ty = b.group(1)
    if ty is None:
        c = COPY_BASE.search(stmt)
        if c:
            ty = block_sound.get(c.group(1))
    if ty is None and STAIR_HELPER.search(stmt):
        # registerStair("id", BASE) / registerLegacyStair：BASE 常数 = 第二参数。
        args = stmt[stmt.index("register"):]
        base = re.search(r",\s*(?:Blocks\.)?(\w+)\)", args)
        if base:
            ty = block_sound.get(base.group(1))
    if ty is None and BED_HELPER.search(stmt):
        ty = "WOOD"  # Blocks.java:5873-5885 registerBed → .sound(SoundType.WOOD)
    if ty is None and GLASS_HELPER.search(stmt):
        ty = "GLASS"  # Blocks.java:5916-5931 registerStainedGlass → GLASS
    if ty is None:
        ty = "STONE"  # BlockBehaviour.java:973 Properties 默认
    block_sound[const] = ty
    block_lines[const] = start + 1
    if reg:
        block_name[const] = reg
        block_names_seen.add(reg)
        reg_line.setdefault(reg, start + 1)
    i = j + 1

assert block_sound, "Blocks.java 解析为空"
missing_types = {v for v in block_sound.values()} - {n for n, *_ in sound_types}
assert not missing_types, f"Blocks.java 引用未知 SoundType: {missing_types}"

# ---- 覆盖率自检：千块表全名可解析 --------------------------------------------
gen = GEN_BLOCKS.read_text(encoding="utf-8", errors="replace")
gen_names = re.findall(r'\(\s*"([a-z0-9_]+)"', gen)
# 只取 GEN_BLOCKS 表内名字（前缀注释里也有字符串，收紧到静态表区域）。
tbl_start = gen.index("static GEN_BLOCKS")
gen_names = re.findall(r'/\*\s*\d+\s*\*/\s*\("([a-z0-9_]+)"', gen[tbl_start:])
assert gen_names, "blocks_gen.inc.rs 名字解析为空"
by_reg = {}
for cname, reg in block_name.items():
    by_reg[reg] = block_sound[cname]
uncovered = []
for n in gen_names:
    target = LEGACY_ALIASES.get(n, n)
    if target not in by_reg:
        uncovered.append((n, target))
# item_frame / glow_item_frame：26.1 无 Block 注册（ItemFrame 是实体，
# entity/decoration/ItemFrame.java:44；引擎表条目来自 blockstates 渲染模型）。
# 归属按 BlockBehaviour.java:973 未注册默认 STONE，显式登记防再生成时误报。
if [u for u in uncovered if u[1] not in ("item_frame", "glow_item_frame")]:
    sys.exit(f"千块表 {len(uncovered)} 名未覆盖（前 10）: {uncovered[:10]}")
for n in ("item_frame", "glow_item_frame"):
    if n not in by_reg:
        by_reg[n] = "STONE"
        block_names_seen.add(n)
print(f"SoundType {len(sound_types)} 类型；千块表 {len(gen_names)} 名全覆盖")

# ---- 产物 -------------------------------------------------------------------
def rs_str(s: str) -> str:
    return f'"{s}"'


out = []
out.append("// @generated by ci/gen-soundtypes.py — DO NOT EDIT.")
out.append("// DEVELOP_ONLY 数据管道（同 blocks_gen.inc.rs 资产政策）：从反编译 26.1")
out.append("// Java 源生成的原版 SoundType 全表与方块归属映射。数据源（本机路径经")
out.append("// MCV_REF_SRC 环境变量注入，绝不入仓）：")
out.append("//   net/minecraft/world/level/block/SoundType.java  —— 类型全表（行号见逐项注释）")
out.append("//   net/minecraft/sounds/SoundEvents.java           —— 常数 → 事件名（sounds.json 键）")
out.append("//   net/minecraft/world/level/block/Blocks.java     —— 方块归属（行号见逐项注释）")
out.append("//   未显式 .sound() 的注册默认 STONE（BlockBehaviour.java:973）。")
out.append("// 再生成: MCV_REF_SRC=<26.1解包树根> python3 ci/gen-soundtypes.py")
out.append("")
out.append("/// SoundType.java 声明序全表：(类型名, SoundType)。")
out.append("pub static SOUND_TYPES: [(&str, SoundType); %d] = [" % len(sound_types))
for name, vol, pitch, evs, ln in sound_types:
    out.append(
        "    /* S.java:%d */ (%s, SoundType { volume: %s, pitch: %s, dig: %s, step: %s, place: %s, hit: %s, fall: %s }),"
        % (ln, rs_str(name), vol, pitch, rs_str(evs[0]), rs_str(evs[1]), rs_str(evs[2]), rs_str(evs[3]), rs_str(evs[4]))
    )
out.append("];")
out.append("")
out.append("/// STONE 在 SOUND_TYPES 的下标（未知方块默认，BlockBehaviour.java:973）。")
out.append("pub const STONE_SOUND_TYPE_IDX: u16 = %d;" % st_index["STONE"])
out.append("")
rows = []
# 名集 = Blocks.java 全部注册名（含 WeatheringCopperBlocks 派生 8 名）
# ∪ 千块表 1171 名（旧名别名落官方名归属）——调用方两侧名字都能命中。
for n in sorted(set(block_names_seen) | set(gen_names)):
    target = LEGACY_ALIASES.get(n, n)
    rows.append((n, st_index[by_reg[target]], target))
rows.sort(key=lambda r: r[0])
out.append("/// 方块注册名 → SOUND_TYPES 下标（Blocks.java 注册名全集 ∪ 千块表")
out.append("/// 1171 名全覆盖；")
out.append("/// 旧名别名: %s）。按名字节序升序（binary_search 前提）。" %
           ", ".join(f"{a}→{b}" for a, b in sorted(LEGACY_ALIASES.items())))
out.append("pub static BLOCK_SOUND_TYPES: [(&str, u16); %d] = [" % len(rows))
for n, idx, target in rows:
    ln = reg_line.get(target, 0)
    src = f"Blocks.java:{ln}" if ln else "无 Block 注册（实体/渲染模型），BlockBehaviour.java:973 默认"
    out.append("    (%s, %d), // ← %s（%s）" % (rs_str(n), idx, target, src))
out.append("];")

OUT.write_text("\n".join(out) + "\n", encoding="utf-8")
print(f"写出 {OUT}（{len(sound_types)} 类型 / {len(rows)} 方块映射）")
