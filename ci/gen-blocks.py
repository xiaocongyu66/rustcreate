#!/usr/bin/env python3
# DEVELOP_ONLY 数据管道（同 assets/ 素材政策，发布 tag 前删除素材与本脚本产物）：
#   从官方 26.1 解包资产（blockstates + models，含 parent 继承链）提取逐面贴卡路径，
#   从反编译 Blocks.java 提取硬度/发光/碰撞/液体属性，
#   生成大规模方块表 blocks_gen.inc.rs 与贴图清单 tiles_manifest.json。
#
# 用法（仓库根目录）:  python3 ci/gen-blocks.py
#
# 素材来源（主控指令：不访问 Mojang CDN，整包官方素材树已就位）：
#   ASSETS = /root/mcv-engine/assets/minecraft/  （与 /root/mc-ref/src-26.1/assets/
#   minecraft/ 同内容，含 textures/**（真实名 PNG）、models/**、blockstates/**；
#   该目录不存在时回退 mc-ref 解包树，仅本机参考、绝不入仓）。
#   贴图不另设拷贝：运行时直接从资源根 assets/minecraft/textures/block/
#   按虚拟路径加载；tiles_manifest.json 存 tile_index(字典序) → 贴图名/虚拟路径。
#
# 产物:
#   game/mcv_core/tiles_manifest.json   —— 层索引(字典序) → 贴图名 + 虚拟路径
#   game/mcv_core/src/blocks_gen.inc.rs —— GEN_BLOCKS 静态表（不改动 lib.rs，主控合并）
#
# id 兼容硬约束：现表 14 方块 id 0..13 保持不变（地形生成器依赖这些 id 产出）；
#   snow_grass 为独立 id 11（官方是 grass_block 的 snowy 状态，本引擎单方块模型，
#   直接采用 grass_block_snow 模型贴图）。新方块从 14 起按官方名字典序。
#   tiles 按 BlockId=u16 加宽目标输出 [u16;6]；tile 0 = 空/占位层。
import json
import re
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
REF_ASSETS = Path("/root/mc-ref/src-26.1/assets/minecraft")
VANILLA = ROOT / "assets/minecraft"
ASSETS = VANILLA if (VANILLA / "blockstates").is_dir() else REF_ASSETS
BLOCKSTATES = ASSETS / "blockstates"
MODELS = ASSETS / "models"
LOCAL_TEX = ASSETS / "textures"
BLOCKS_JAVA = Path("/root/mc-ref/src-26.1/net/minecraft/world/level/block/Blocks.java")

OUT_MANIFEST = ROOT / "game" / "mcv_core" / "tiles_manifest.json"
OUT_RS = ROOT / "game" / "mcv_core" / "src" / "blocks_gen.inc.rs"
OUT_CPP = ROOT / "cpp" / "src" / "blocks_gen.inc"

# 面序: [+X, -X, +Y, -Y, +Z, -Z]（同 BlockDef.tiles 注释）；east=+X … north=-Z
FACE_ORDER = ["east", "west", "up", "down", "south", "north"]

# 旧 14 方块（id 0..13，顺序/名字/属性与旧表一致，地形生成器依赖）。
# tiles 逐面改为「官方贴图路径」引用（面序 [+X,-X,+Y,-Y,+Z,-Z]）。
# snow_grass = grass_block[snowy=true]（grass_block_snow 模型）在本引擎的独立 id。
LEGACY = [
    ("air", "air", None),
    ("stone", "stone", ["block/stone"] * 6),
    ("dirt", "dirt", ["block/dirt"] * 6),
    ("grass", "grass_block", ["block/grass_block_side", "block/grass_block_side",
                              "block/grass_block_top", "block/dirt",
                              "block/grass_block_side", "block/grass_block_side"]),
    ("sand", "sand", ["block/sand"] * 6),
    ("water", "water", ["block/water_still"] * 6),
    ("log", "oak_log", ["block/oak_log", "block/oak_log", "block/oak_log_top",
                        "block/oak_log_top", "block/oak_log", "block/oak_log"]),
    ("leaves", "oak_leaves", ["block/oak_leaves"] * 6),
    ("planks", "oak_planks", ["block/oak_planks"] * 6),
    ("cobble", "cobblestone", ["block/cobblestone"] * 6),
    ("bedrock", "bedrock", ["block/bedrock"] * 6),
    ("snow_grass", "grass_block", ["block/grass_block_snow", "block/grass_block_snow",
                                   "block/grass_block_top", "block/dirt",
                                   "block/grass_block_snow", "block/grass_block_snow"]),
    ("flower_red", "poppy", ["block/poppy"] * 6),   # 旧表 6 面同贴图，保持不变
    ("flower_yellow", "dandelion", ["block/dandelion"] * 6),
]
# 旧名 → (solid, opaque, liquid, light, hardness, kind)
LEGACY_ATTRS = {
    "air": (False, False, False, 0, 0.0, 0),
    "stone": (True, True, False, 0, 1.5, 0),
    "dirt": (True, True, False, 0, 0.5, 0),
    "grass": (True, True, False, 0, 0.6, 0),
    "sand": (True, True, False, 0, 0.5, 0),
    "water": (False, False, True, 0, 100.0, 0),
    "log": (True, True, False, 0, 2.0, 0),
    "leaves": (True, False, False, 0, 0.2, 0),
    "planks": (True, True, False, 0, 2.0, 0),
    "cobble": (True, True, False, 0, 2.0, 0),
    "bedrock": (True, True, False, 0, float("inf"), 0),
    "snow_grass": (True, True, False, 0, 0.6, 0),
    "flower_red": (False, False, False, 0, 0.0, 1),
    "flower_yellow": (False, False, False, 0, 0.0, 1),
}
LEGACY_OFFICIAL = {off for _, off, _ in LEGACY}

# 透明像素启发式之外，按名字判非 opaque 的词（玻璃/叶/冰/台阶/薄板等）。
NONOPAQUE_NAME_WORDS = ("glass", "leaves", "ice", "slab", "pane", "trapdoor", "grate")

stats = {
    "blockstates": 0, "cube": 0, "noncube": 0,
    "noncube_cross": 0, "noncube_multipart": 0, "noncube_other": 0,
    "tex_missing": 0, "blocks_no_tex": 0, "java_name_miss": 0,
    "multipart_blockstates": 0, "model_missing": 0,
}


# ---------------------------------------------------------------------------
# 1. blockstate → 默认模型 → parent 链 → 逐面贴卡路径
# ---------------------------------------------------------------------------

def default_model_of(bs):
    """默认变体："" / "normal" / 单键 / 键名升序第一个；multipart 取第一个 apply。"""
    if "multipart" in bs:
        stats["multipart_blockstates"] += 1
        for part in bs["multipart"]:
            m = part.get("apply")
            if isinstance(m, list):
                m = m[0] if m else None
            if isinstance(m, dict) and "model" in m:
                return m["model"], True
        return None, True
    variants = bs.get("variants", {})
    if not variants:
        return None, False
    val = None
    for key in ("", "normal"):
        if key in variants:
            val = variants[key]
            break
    if val is None:
        val = variants[min(variants)]  # 如 grass_block 的 "snowy=false"
    if isinstance(val, list):
        val = val[0]
    return (val.get("model"), False) if isinstance(val, dict) else (None, False)


_model_cache = {}


def load_model_chain(model_id):
    """→ (chain=[leaf..root] 的 JSON 列表, parent 末段名列表)；缺文件即截断。"""
    if model_id in _model_cache:
        return _model_cache[model_id]
    chain, parents, cur = [], [], model_id
    for _ in range(32):
        if cur is None:
            break
        if ":" in cur:
            dom, rel = cur.split(":", 1)
            if dom != "minecraft":
                break
        else:
            rel = cur
        parents.append(rel.split("/")[-1])
        path = MODELS / (rel + ".json")
        if not path.exists():
            break
        j = json.loads(path.read_text())
        chain.append(j)
        cur = j.get("parent")
    _model_cache[model_id] = (chain, parents)
    return chain, parents


def plain(v):
    """新版素材允许 textures 值为对象或数组；取纯引用。
    26.1 对象形式: {"sprite": "minecraft:block/x", "force_translucent": true}
    （旧字段名 name 亦兼容）。"""
    if isinstance(v, dict):
        return v.get("sprite") or v.get("name")
    if isinstance(v, list) and v:
        return plain(v[0])
    return v if isinstance(v, str) else None


def resolve_ref(chain, ref, depth=0):
    """把 "#key" 沿 parent 链解析成贴图路径（'block/x' / 'missingno' 等）。"""
    ref = plain(ref)
    if not ref or depth > 8:
        return None
    while ref.startswith("#") and depth <= 8:
        nxt = None
        for j in chain:
            v = plain(j.get("textures", {}).get(ref[1:]))
            if v is not None and v != ref:
                nxt = v
                break
        if nxt is None:
            return None
        ref = nxt
        depth += 1
    return ref


def normalize_tex(path):
    """'minecraft:block/x' → 'block/x'；missingno/非 block/ 命名空间 → None。"""
    if not path:
        return None
    if path.startswith("minecraft:"):
        path = path[len("minecraft:"):]
    if path.startswith("missingno") or path == "missingno":
        return None
    return path if path.startswith("block/") else None


def model_face_textures(model_id):
    """→ (faces: face→'block/x' 或 None, kind, cross)  kind: 0=纯立方, 1=非立方。"""
    chain, parents = load_model_chain(model_id)
    if not chain:
        stats["model_missing"] += 1
        return {f: None for f in FACE_ORDER}, 1, False

    merged = {}
    for j in reversed(chain):  # root→leaf，子覆盖父
        merged.update(j.get("textures", {}))
    particle = normalize_tex(resolve_ref(chain, merged.get("particle")))

    face_tex, full_box, has_elements, side_only = {}, True, False, True
    for j in chain:
        for el in j.get("elements", []):
            has_elements = True
            if el.get("rotation"):
                full_box = False
            if el.get("from", [0, 0, 0]) != [0, 0, 0] or el.get("to", [16, 16, 16]) != [16, 16, 16]:
                full_box = False
            faces = el.get("faces", {})
            if set(faces) & {"up", "down"}:
                side_only = False
            for face, f in faces.items():
                t = normalize_tex(resolve_ref(chain, f.get("texture")))
                if t:
                    face_tex[face] = t  # 后绘制元素（grass overlay 等）覆盖同面

    cross = any(p in ("cross", "cross_emissive") for p in parents)
    cube = (not has_elements) or (full_box and not side_only)

    out = {}
    if cube and not has_elements:
        # cube_all / cube / cube_column / orientable 等纯模板：查默认键
        for face in FACE_ORDER:
            out[face] = normalize_tex(resolve_ref(
                chain, merged.get(face) or merged.get("all")))
        cube = all(out.values())
    elif cube:
        for face in FACE_ORDER:  # 全盒 elements（grass_block 等）
            out[face] = face_tex.get(face)
        cube = all(out.values())
    if not cube:
        if cross:
            # 十字：自身贴图铺满 6 面。注：任务书建议 cross 顶底留空，但现网格器
            # 不识别 model_kind（渲染 6 面），且 tile 0 是可见品红调试层，留空会
            # 闪品红；按“做不到就六面同贴图占位”处理。
            t = face_tex.get("north") or face_tex.get("east") or particle
            for face in FACE_ORDER:
                out[face] = t
        else:
            # 占位：能给面给面，缺的面用代表贴图补（优先 side/up/particle）
            rep = (normalize_tex(resolve_ref(chain, merged.get("side")))
                   or out.get("up") or face_tex.get("up") or face_tex.get("down")
                   or face_tex.get("north") or face_tex.get("east") or particle)
            for face in FACE_ORDER:
                out[face] = out.get(face) or face_tex.get(face) or rep
    return out, (0 if cube else 1), cross


# ---------------------------------------------------------------------------
# 2. 贴图存在性/透明度（本地素材树，不下载）
# ---------------------------------------------------------------------------

def tex_exists(tex):
    return (LOCAL_TEX / (tex + ".png")).exists()


def has_alpha(tex):
    try:
        im = Image.open(LOCAL_TEX / (tex + ".png")).convert("RGBA")
        return im.getextrema()[3][0] < 255
    except Exception:  # noqa: BLE001
        return False


# ---------------------------------------------------------------------------
# 3. Blocks.java 属性提取（Vineflower 反编译，只提取数值，不复制代码进仓库）
# ---------------------------------------------------------------------------

CONST_RE = re.compile(r"public static final Block ([A-Z0-9_]+) = ")
PROPS_START_RE = re.compile(r"Properties\.of\w*\(|wallVariant\(")


def parse_java_blocks():
    src = BLOCKS_JAVA.read_text()
    consts = [(m.group(1), m.start()) for m in CONST_RE.finditer(src)]
    entries, const2name = {}, {}
    for i, (const, pos) in enumerate(consts):
        end = consts[i + 1][1] if i + 1 < len(consts) else len(src)
        body = src[pos:end]
        m = re.search(r'register\(\s*\n?\s*"([a-z0-9_]+)"', body)
        name = m.group(1) if m else const.lower()
        if name in entries:
            continue
        pm = PROPS_START_RE.search(body)
        props = body[pm.start():] if pm else ""
        base = None
        bm = re.search(r"Properties\.(?:ofFullCopy|ofLegacyCopy)\((\w+)\)", props)
        if bm:
            base = bm.group(1)  # base 的 Block 常量名
        else:
            wm = re.search(r"^wallVariant\((\w+),", props)
            sm = re.search(r"register(?:Legacy)?Stair\(\"[a-z0-9_]+\",\s*(\w+)\)", body)
            if wm:
                base = wm.group(1)
            elif sm:
                base = sm.group(1)
        strength = None
        stm = re.search(r"\.strength\((-?\d+(?:\.\d+)?)F?(?:,\s*-?\d+(?:\.\d+)?F?)?\)", props)
        if stm:
            strength = float(stm.group(1))
        light = None
        lm = (re.search(r"lightLevel\(\s*\w+ -> (\d+)\s*\)", props)
              or re.search(r"litBlockEmission\((\d+)\)", props)
              or re.search(r"lightLevel\((\d+)\)", props))
        if lm:
            light = int(lm.group(1))
        e = {
            "const": const, "base_const": base,
            "strength": strength, "light": light,
            "no_collision": ".noCollision()" in props,
            "force_solid": ".forceSolidOn()" in props,
            "no_occlusion": ".noOcclusion()" in props,
            "liquid": ".liquid()" in props,
            "air": ".air()" in props,
            "instabreak": ".instabreak()" in props,
        }
        # 私有辅助 Properties（数值对照 Blocks.java 尾部方法体人工核对）
        if "logProperties(" in body and e["strength"] is None:
            e["strength"] = 2.0
        if "leavesProperties(" in body:
            e["strength"] = 0.2
            e["no_occlusion"] = True
        if "netherStemProperties(" in body:
            e["strength"] = 2.0
        if "candleProperties(" in body:
            e["strength"] = 0.1
            e["no_occlusion"] = True
            e["light"] = 3  # CandleBlock.LIGHT_EMISSION = 3 * candles
        if "flowerPotProperties(" in body:
            e["instabreak"] = True
            e["no_occlusion"] = True
        if "shulkerBoxProperties(" in body:
            e["strength"] = 2.0
            e["no_occlusion"] = True
            e["force_solid"] = True
        if "registerBed(" in body:
            e["strength"] = 0.2
            e["no_occlusion"] = True
        if "registerStainedGlass(" in body:
            e["strength"] = 0.3
            e["no_occlusion"] = True
        entries[name] = e
        const2name[const] = name
    return entries, const2name


def resolve_attrs(name, entries, const2name, seen=None):
    """合并继承（ofFullCopy/ofLegacyCopy/registerStair/wallVariant 的 base）。"""
    e = entries.get(name)
    if e is None:
        return None
    seen = seen or set()
    hardness = e["strength"]
    if hardness is not None and hardness < 0:
        hardness = float("inf")  # 基岩 strength(-1) → INFINITY（沿用引擎约定）
    solid = not (e["no_collision"] or e["liquid"] or e["air"]) or e["force_solid"]
    no_occ = e["no_occlusion"]
    liquid, air = e["liquid"], e["air"]
    light = e["light"]
    instabreak = e["instabreak"]
    base = e["base_const"]
    if base and base not in seen and base in const2name:
        seen.add(base)
        b = resolve_attrs(const2name[base], entries, const2name, seen)
        if b:
            if hardness is None:
                hardness = b["hardness"]
            if light is None:
                light = b["light"]
            liquid = liquid or b["liquid"]
            air = air or b.get("air", False)
            instabreak = instabreak or b.get("instabreak")
            no_occ = no_occ or not b["opaque"]
            if e["strength"] is None and not e["no_collision"] and not e["force_solid"]:
                solid = b["solid"]
    if instabreak:
        hardness = 0.0
    if hardness is None:
        hardness = 2.0  # 缺数据默认
    if light is None:
        light = 0
    solid = solid and not liquid and not air
    opaque = not no_occ and solid and not liquid and not air
    return {"hardness": hardness, "light": min(light, 15), "solid": solid,
            "opaque": opaque, "liquid": liquid, "air": air}


# ---------------------------------------------------------------------------
# 4. 主流程
# ---------------------------------------------------------------------------

def rb(v):
    return "true" if v else "false"


def rust_f32(v):
    return "f32::INFINITY" if v == float("inf") else f"{v:.10g}f32"


def cpp_geom(name, solid, liquid, kind):
    """C++ kBlocks.geom：不透明 pass 是否产出几何。air 无几何；water 走水
    pass；非立方（kind=1，cross/楼梯/板…）占位渲染为全立方 → true；
    其余立方实体/其他液体（岩浆）→ solid or liquid。"""
    if name in ("air", "water"):
        return False
    return bool(solid or liquid or kind == 1)


def main():
    print(f"素材树: {ASSETS}")
    entries, const2name = parse_java_blocks()
    print(f"Blocks.java: {len(entries)} registered blocks")

    # --- 解析全部 blockstate ---
    blocks = {}
    for p in sorted(BLOCKSTATES.glob("*.json")):
        name = p.stem
        model, is_mp = default_model_of(json.loads(p.read_text()))
        if model is None:
            faces, kind, cross = {f: None for f in FACE_ORDER}, 1, False
        else:
            faces, kind, cross = model_face_textures(model)
        blocks[name] = {"faces": faces, "kind": kind, "cross": cross, "mp": is_mp}
        stats["cube" if kind == 0 else "noncube"] += 1
        if kind:
            stats["noncube_cross" if cross else
                  ("noncube_multipart" if is_mp else "noncube_other")] += 1
    stats["blockstates"] = len(blocks)
    print(f"blockstates {stats['blockstates']}: cube={stats['cube']} "
          f"noncube={stats['noncube']} (cross={stats['noncube_cross']} "
          f"multipart={stats['noncube_multipart']} other={stats['noncube_other']})")

    # --- 收集贴图（含旧 14 方块引用），存在性检查 + 同名去重 ---
    wanted = set()
    for b in blocks.values():
        wanted.update(t for t in b["faces"].values() if t)
    for _, _, tiles in LEGACY:
        if tiles:
            wanted.update(tiles)
    missing = sorted(t for t in wanted if not tex_exists(t))
    stats["tex_missing"] = len(missing)
    for t in missing:
        print(f"[warn] 缺贴图: {t}")
    wanted -= set(missing)

    by_leaf = {}
    for t in wanted:
        by_leaf.setdefault(t.split("/")[-1], set()).add(t)
    renamed = {}
    for leaf, paths in by_leaf.items():
        for t in paths:
            renamed[t] = leaf if len(paths) == 1 else "block_" + leaf.replace("/", "_")
    conflicts = sorted(k for k, v in by_leaf.items() if len(v) > 1)
    if conflicts:
        print("[info] 同名冲突加 block_ 前缀:", conflicts)

    tile_names = sorted({renamed[t] for t in wanted})
    name2path = {renamed[t]: t for t in wanted}
    tile_index = {nm: i for i, nm in enumerate(tile_names)}
    print(f"唯一贴图 {len(tile_names)}，缺 {stats['tex_missing']}")
    if len(tile_names) >= 2048:
        print(f"[warn] 贴图数 {len(tile_names)} ≥ 2048 层预算（GLES 上限抬到 adapter 值但 <2048）!")

    OUT_MANIFEST.write_text(json.dumps({
        "note": "tiles_manifest: 纹理数组层索引(按文件名字典序)→贴图。由 ci/gen-blocks.py 生成。"
                "贴图不入库，运行时从资源根 assets/minecraft/textures/block/ 按 virtual_path 加载（DEVELOP_ONLY）。",
        "texture_root": "assets/minecraft/textures/block",
        "tile_index_to_file": tile_names,
        "file_to_virtual_path": {nm: name2path[nm] + ".png" for nm in tile_names},
    }, ensure_ascii=False, indent=1) + "\n")

    def tile_id(tex):
        return tile_index.get(renamed[tex], 0) if tex else 0

    # --- 生成 Rust 表 ---
    total = len(LEGACY) + sum(1 for n in blocks if n not in LEGACY_OFFICIAL)
    lines = [
        "// @generated by ci/gen-blocks.py — DO NOT EDIT.",
        "// DEVELOP_ONLY：官方 26.1 blockstates/models/Blocks.java 生成（同 assets/ 素材政策）。",
        "// 元组: (name, solid, opaque, liquid, light_emit, tiles [+X,-X,+Y,-Y,+Z,-Z], hardness, model_kind)",
        "// model_kind: 0=纯立方（全支持） 1=非立方（cross/楼梯/板/栅栏/多部件…，占位代表贴图）",
        "// tiles 为 tiles_manifest.json 层索引；0=空/占位层。hardness=MC strength()，基岩=inf。",
        "// id 0..13 = 旧 14 方块（id/名字/属性与旧表一致，地形生成器依赖）；14+ 官方名字典序。",
        "// snow_grass(id 11): 官方是 grass_block 的 snowy 状态；本引擎单方块模型，独立 id，取 snow 模型贴图。",
        "// 已知限制：tintindex 生物群系染色（草侧面 overlay/叶）未由渲染管线实现，贴图按原样入表；",
        "// cross/非立方暂渲染为全方块（网格器未实现 model_kind），六面给代表贴图。",
        "#[allow(clippy::type_complexity)]",
        f"static GEN_BLOCKS: [(&str, bool, bool, bool, u8, [u16; 6], f32, u8); {total}] = [",
    ]
    cpp_rows = []  # (name, opaque, liquid, geom, tiles) —— C++ kBlocks 同序行
    idx = 0
    for lname, _off, tiles in LEGACY:
        solid, opaque, liquid, light, hard, kind = LEGACY_ATTRS[lname]
        t = [tile_id(x) for x in tiles] if tiles else [0] * 6
        cmt = ""
        if lname == "snow_grass":
            cmt = "  // = grass_block[snowy=true]，独立 id（单方块模型）"
        if lname in ("flower_red", "flower_yellow"):
            cmt = "  // cross 占位：旧表 6 面同贴图，保持不变"
        cpp_rows.append((lname, opaque, liquid, cpp_geom(lname, solid, liquid, kind), t))
        lines.append(f"    /* {idx:4} */ ({json.dumps(lname)}, {rb(solid)}, {rb(opaque)}, "
                     f"{rb(liquid)}, {light}, [{', '.join(map(str, t))}], "
                     f"{rust_f32(hard)}, {kind}),{cmt}")
        idx += 1

    for name in sorted(blocks):
        if name in LEGACY_OFFICIAL:
            continue
        b = blocks[name]
        t = [tile_id(b["faces"][f]) for f in FACE_ORDER]
        if all(v == 0 for v in t) and name != "air":
            stats["blocks_no_tex"] += 1
        attrs = resolve_attrs(name, entries, const2name)
        if attrs is None:
            stats["java_name_miss"] += 1
            attrs = {"hardness": 2.0, "light": 0, "solid": True,
                     "opaque": True, "liquid": False}
        solid, opaque, liquid = attrs["solid"], attrs["opaque"], attrs["liquid"]
        used = [b["faces"][f] for f in FACE_ORDER if b["faces"][f]]
        if any(has_alpha(x) for x in used) \
                or any(w in name for w in NONOPAQUE_NAME_WORDS) \
                or b["kind"] == 1:
            opaque = False
        cpp_rows.append((name, opaque, liquid, cpp_geom(name, solid, liquid, b["kind"]), t))
        lines.append(f"    /* {idx:4} */ ({json.dumps(name)}, {rb(solid)}, {rb(opaque)}, "
                     f"{rb(liquid)}, {attrs['light']}, [{', '.join(map(str, t))}], "
                     f"{rust_f32(attrs['hardness'])}, {b['kind']}),")
        idx += 1
    lines.append("];")
    OUT_RS.write_text("\n".join(lines) + "\n")
    print(f"wrote {OUT_RS} ({idx} entries)")

    # --- C++ 侧 kBlocks 表（同一次生成保证与 BLOCKS 逐 id 同步） ---
    cpp_lines = [
        "// @generated by ci/gen-blocks.py — DO NOT EDIT.",
        "// mesher.cpp 的 kBlocks 初始化行；与 blocks_gen.inc.rs 同一次生成，",
        "// id 顺序/标志/tile 层索引与 mcv_core::BLOCKS 按构造一致。",
        "// 字段: { opaque, liquid, geom, tiles[+X,-X,+Y,-Y,+Z,-Z] }",
        "// geom: 空气/水（水走水 pass）= false；非立方(kind=1)占位渲染为",
        "// 全立方 = true；其余立方实体/其他液体 = solid or liquid。",
    ]
    for name, opaque, liquid, geom, t in cpp_rows:
        cpp_lines.append(
            f"    {{{'true' if opaque else 'false'}, {'true' if liquid else 'false'}, "
            f"{'true' if geom else 'false'}, {{{', '.join(map(str, t))}}}}},  // {name}"
        )
    OUT_CPP.write_text("\n".join(cpp_lines) + "\n")
    print(f"wrote {OUT_CPP} ({len(cpp_rows)} rows)")
    print(f"wrote {OUT_MANIFEST} ({len(tile_names)} tiles)")
    print("stats:", json.dumps(stats))

    # --- 抽查样本（供 docs/blocks-report.md） ---
    samples = ["grass_block", "oak_log", "glass", "poppy", "sandstone_stairs",
               "oak_slab", "oak_fence", "glass_pane", "torch", "water",
               "bedrock", "oak_leaves", "crafting_table", "furnace", "dandelion"]
    print("\n== 抽查（tiles → 贴图名）==")
    for s in samples:
        if s in LEGACY_OFFICIAL:
            pos = [i for i, (n, o, _) in enumerate(LEGACY) if o == s]
            src = f"legacy#{pos[0] if pos else '?'}"
            tiles = next((tl for n, o, tl in LEGACY if o == s and n != "snow_grass"),
                         next(tl for n, o, tl in LEGACY if n == "snow_grass"))
            faces = {f: (renamed.get(x, "∅") if x else None)
                     for f, x in zip(FACE_ORDER, tiles)} if tiles else {}
            kind = LEGACY_ATTRS[next(n for n, o, _ in LEGACY
                                     if o == s and n != "snow_grass")][5]
        else:
            src = "gen"
            b = blocks[s]
            faces = {f: (renamed.get(b["faces"][f], "∅") if b["faces"][f] else None)
                     for f in FACE_ORDER}
            kind = b["kind"]
        print(f"{s:20s} kind={kind} src={src}")
        for f in FACE_ORDER:
            print(f"   {f:6s} -> {faces.get(f)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
