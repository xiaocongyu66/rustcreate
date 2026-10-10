#!/usr/bin/env python3
# DEVELOP_ONLY 数据管道（同 assets/ 素材政策，发布 tag 前删除素材与本脚本产物）：
#   从官方 26.1 解包资产（blockstates + models，含 parent 继承链）提取逐面贴卡路径，
#   从反编译 Blocks.java 提取硬度/发光/碰撞/液体属性，
#   生成大规模方块表 blocks_gen.inc.rs 与贴图清单 tiles_manifest.json。
#
# 用法（仓库根目录）:  python3 ci/gen-blocks.py
#
# 素材来源（主控指令：不访问 Mojang CDN，整包官方素材树已就位）：
#   ASSETS = <repo>/assets/minecraft/（含 textures/**（真实名 PNG）、models/**、
#   blockstates/**；不存在时回退环境变量 MCV_REF_SRC 指向的本机解包树，
#   仅本机参考、绝不入仓——路径一律不写死进仓库）。
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
#   tiles 按 BlockId=u16 加宽目标输出 [u16;6]；tile 层 0..N-1 = 真实贴图
#   （字典序），层 N = missing 哨兵（模型无贴图解析的方块；渲染为原版
#   missingno 品红标记，见 mcv_core::atlas::fill_missing_marker）。
import json
import os
import re
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
# 本机解包参考树根（含 assets/minecraft 与 net/minecraft/**），仅经环境变量
# 注入——本机路径绝不写死入仓。
REF_SRC = Path(os.environ["MCV_REF_SRC"]) if os.environ.get("MCV_REF_SRC") else None
VANILLA = ROOT / "assets/minecraft"
ASSETS = VANILLA if (VANILLA / "blockstates").is_dir() else (
    REF_SRC / "assets/minecraft" if REF_SRC else VANILLA)
BLOCKSTATES = ASSETS / "blockstates"
MODELS = ASSETS / "models"
LOCAL_TEX = ASSETS / "textures"
BLOCKS_JAVA = (REF_SRC / "net/minecraft/world/level/block/Blocks.java") if REF_SRC else None
if not BLOCKSTATES.is_dir():
    sys.exit(f"缺 blockstates 树：置 MCV_REF_SRC=<26.1解包树根> 或就位 {VANILLA}")
if BLOCKS_JAVA is None or not BLOCKS_JAVA.is_file():
    sys.exit("缺反编译 Blocks.java：置 MCV_REF_SRC=<26.1解包树根>（仅本机，勿入仓）")

OUT_MANIFEST = ROOT / "game" / "mcv_core" / "tiles_manifest.json"
OUT_RS = ROOT / "game" / "mcv_core" / "src" / "blocks_gen.inc.rs"
OUT_CPP = ROOT / "cpp" / "src" / "blocks_gen.inc"
# 官方注册表名字清单（tests/blocks_table.rs 覆盖率断言的数据源）
OUT_VANILLA = ROOT / "game" / "mcv_core" / "tests" / "vanilla_blocks_gen.inc"

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

# missing 哨兵贴图名（tiles_manifest.json 最后一层，无对应文件）。模型无贴图
# 解析的方块（air/barrier/light/structure_void 等）全部指向该层，运行时由
# mcv_core::atlas 固定为原版 missingno 品红标记（MissingTextureAtlasSprite）。
SENTINEL_NAME = "missing_no_texture"

# 透明像素启发式之外，按名字判非 opaque 的词（玻璃/叶/冰/台阶/薄板等）。
NONOPAQUE_NAME_WORDS = ("glass", "leaves", "ice", "slab", "pane", "trapdoor", "grate")

# 形状分类（与 game/mcv_core/src/shape.rs::shape_of_name 逐字一致，三处同源，
# 第三处是下面写出的 cpp/src/blocks_gen.inc 的 shape 字段）。值 = Shape 枚举：
# 0 Cube, 1 Cross, 2 Torch, 3 Fence, 4 Slab, 5 Stairs, 6 Carpet, 7 Trapdoor,
# 8 Pane, 9 Wall。6..9 渲染暂走全盒占位（C++ emit_shapes default / Rust
# mcv_mesher 全盒路径），模板待 Rust 网格器 #77；碰撞/拾取几何真值在
# engine/mcv_game/src/blockshapes.rs（本任务已落）。
CROSS_PLANTS = {
    "flower_red", "flower_yellow", "allium", "azure_bluet", "blue_orchid",
    "cornflower", "lily_of_the_valley", "oxeye_daisy", "torchflower",
    "torchflower_crop", "wither_rose", "short_grass", "fern", "tall_grass",
    "large_fern", "rose_bush", "pink_petals", "wildflowers",
}


def shape_of_name(name):
    """注册名 → 形状编号；规则顺序与 shape.rs const fn 一致（勿改序）。"""
    if "potted" in name:
        return 0
    if "fence" in name and "gate" not in name:
        return 3
    if "slab" in name:
        return 4
    if "stairs" in name:
        return 5
    if "carpet" in name:
        return 6
    if "trapdoor" in name:
        return 7
    if "pane" in name:
        return 8
    if name.endswith("_wall"):
        return 9
    if name in CROSS_PLANTS or name.endswith("sapling"):
        return 1
    if "torch" in name and "wall" not in name:
        return 2
    return 0

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
            # 不识别 model_kind（渲染 6 面），且 missing 哨兵层是可见品红调试
            # 层，留空会闪品红；按“做不到就六面同贴图占位”处理。
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

CONST_RE = re.compile(r"public static final \w+ ([A-Z0-9_]+) = ")
PROPS_START_RE = re.compile(r"Properties\.of\w*\(|wallVariant\(")

# litBlockEmission 家族中【默认放置态即点亮】的成员（registerDefaultState
# LIT=true）：营火 CampfireBlock.java:84（soul 同块类，光级 10）、红石火把
# RedstoneTorchBlock.java:40（wall 变体经 wallVariant 复制同属性，
# Blocks.java:1597）。数值 = 各自 litBlockEmission(n)（Blocks.java:4624/
# 4636/1592）。其余成员默认 lit=false（炉族 :1064/4535/4545、红石矿 :1581、
# 红石灯 :2167、铜灯 CopperBulbBlock.java:30、蜡烛蛋糕 :5032）按 0。
DEFAULT_LIT_LIGHT = {
    "CAMPFIRE": 15,
    "SOUL_CAMPFIRE": 10,
    "REDSTONE_TORCH": 7,
    "REDSTONE_WALL_TORCH": 7,
}

# WeatheringCopperBlocks.create("copper_bars", …) 一次注册 8 个铜风化变体
# （基础 + exposed/weathered/oxidized + waxed 前缀），属性共享同一 lambda。
COPPER_PREFIXES = (
    "", "exposed_", "weathered_", "oxidized_",
    "waxed_", "waxed_exposed_", "waxed_weathered_", "waxed_oxidized_",
)


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
        # 表行约定 =【默认放置态】（defaultBlockState）的发光。
        # litBlockEmission(n) 是【按 LIT 状态】的条件发光（Blocks.java:5853-5855
        # `lit ? n : 0`），正则不再匹配——家族中默认 lit=false 的成员
        # （炉/高炉/烟熏炉、红石矿/红石灯、铜灯族、蜡烛蛋糕族）自然落 0
        # （曾误按恒发光提取，把 furnace 记成 13；26.1 未点燃炉光级 0）；
        # 默认 lit=true 的成员（营火族、红石火把族）放置即点亮，查
        # DEFAULT_LIT_LIGHT 保留光级。
        lm = (re.search(r"lightLevel\(\s*\w+ -> (\d+)\s*\)", props)
              or re.search(r"lightLevel\((\d+)\)", props))
        if lm:
            light = int(lm.group(1))
        elif const in DEFAULT_LIT_LIGHT:
            light = DEFAULT_LIT_LIGHT[const]
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
            # 蜡烛发光：CandleBlock.LIGHT_EMISSION = lit ? 3*candles : 0
            # （CandleBlock.java:43），默认放置态 lit=false（:76）→ 放置即
            # 熄灭不发光，点亮是后续玩家行为、引擎无方块状态不建模。
            # （body 窗口会延伸到下一个常量之前的私有辅助方法——candleProperties()
            # 定义夹在 FIREFLY_BUSH 与后继常量之间，故仅限无显式 lightLevel
            # 的方块，避免把 firefly_bush 的恒发光 2 误覆盖。）
            if e["light"] is None:
                e["light"] = 0
        if "flowerPotProperties(" in body:
            e["instabreak"] = True
            e["no_occlusion"] = True
        if "shulkerBoxProperties(" in body:
            e["strength"] = 2.0
            e["no_occlusion"] = True
            e["force_solid"] = True  # 仅 isSolid 语义；碰撞由无 noCollision 保证
        if "registerBed(" in body:
            e["strength"] = 0.2
            e["no_occlusion"] = True
        if "registerStainedGlass(" in body:
            e["strength"] = 0.3
            e["no_occlusion"] = True
        if "buttonProperties(" in body:
            # buttonProperties(): noCollision().strength(0.5F)（Blocks.java:5970-5972）。
            # 石/木/竹/下界木全部按钮共用；此前漏提 → 误落 2.0 且 solid=true。
            e["strength"] = 0.5
            e["no_collision"] = True
        if "pistonProperties(" in body:
            # pistonProperties(): strength(1.5F)（Blocks.java:5960-5966）。
            e["strength"] = 1.5
        if "WeatheringCopperBlocks.create(" in body:
            # create(id, …) 一次注册 8 个变体，共享 lambda 里的属性
            # （copper_bars 5.0 / copper_chain 5.0 / copper_lantern 3.5+15 光，
            # 见 Blocks.java:1934/1944/4602）。基础名取 create 首个字符串。
            cm = re.search(r'WeatheringCopperBlocks\.create\(\s*"([a-z0-9_]+)"', body)
            base_id = cm.group(1) if cm else const.lower()
            for pfx in COPPER_PREFIXES:
                vn = pfx + base_id
                if vn not in entries:
                    entries[vn] = dict(e)
            continue
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
    # solid = 原版 hasCollision（Properties.noCollision() 置 false，
    # BlockBehaviour.java:1079-1082）。26.1 碰撞判据 =
    # `hasCollision ? getShape : empty`（BlockBehaviour.java:333-334），
    # 引擎消费方 blockshapes::push_boxes 的 Cube 分支以 solid 为碰撞谓词，
    # 语义必须对位 hasCollision。forceSolidOn 只抬 isSolid
    # （BlockBehaviour.java:482-487 calculateSolid，供 isSolidRender/实体
    # 投放等查询），不产生碰撞——此前误并入 solid，给招牌/压力板/旗帜/
    # 凋珊瑚/蛛网/竹笋等 noCollision+forceSolidOn 方块凭空造出全盒碰撞，
    # 且让 bamboo_sapling 落入「solid=true 而形状零碰撞」的自相矛盾条目。
    solid = not (e["no_collision"] or e["liquid"] or e["air"])
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
            if e["strength"] is None and not e["no_collision"]:
                solid = b["solid"]
    if instabreak:
        hardness = 0.0
    if hardness is None:
        # 原版 Properties.destroyTime 字段默认 0.0F（BlockBehaviour.java:976
        # `private float destroyTime;`，未调 .strength() 即 0）；此前误用 2.0
        # 兜底，曾把按钮/空气族/史莱姆/蜂蜜/虫蚀石等 40+ 块标成 2.0。
        hardness = 0.0
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
    """C++ kBlocks.geom：不透明 pass 是否产出几何。air/water 走水/空 pass；
    隐形方块（屏障/光源/空气族/结构空位/气泡柱）原版不可见 → false；
    非立方（kind=1，cross/楼梯/板…）占位渲染为全立方 → true；
    其余立方实体/其他液体（岩浆）→ solid or liquid。"""
    if name in INVISIBLE_GEOM or name in ("air", "water"):
        return False
    return bool(solid or liquid or kind == 1)


# 原版不可见方块（几何上不渲染；此前 kind=1 兜底让它们顶着「层 0 贴图」出全盒）。
# 依据：BarrierBlock/LightBlock/AirBlock/StructureVoidBlock/BubbleColumnBlock
# getShape=empty 或原版从不为它们生成渲染几何。
INVISIBLE_GEOM = {
    "barrier", "light", "cave_air", "void_air", "structure_void",
    "bubble_column",
}


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

    # missing 哨兵层：排在全部真实贴图之后，专供「模型无贴图解析」的方块
    # （air/barrier 等）指向；渲染层把它固定为原版 missingno 品红标记
    # （MissingTextureAtlasSprite），且不参与磁盘加载（无对应文件）。
    sentinel_index = len(tile_names)
    OUT_MANIFEST.write_text(json.dumps({
        "note": "tiles_manifest: 纹理数组层索引(按文件名字典序)→贴图。由 ci/gen-blocks.py 生成。"
                "贴图不入库，运行时从资源根 assets/minecraft/textures/block/ 按 virtual_path 加载（DEVELOP_ONLY）。"
                f"层 {sentinel_index} = missing 哨兵（无对应文件，运行时固定 missingno 品红标记）。",
        "texture_root": "assets/minecraft/textures/block",
        "tile_index_to_file": tile_names + [SENTINEL_NAME],
        "file_to_virtual_path": {nm: name2path[nm] + ".png" for nm in tile_names},
    }, ensure_ascii=False, indent=1) + "\n")

    def tile_id(tex):
        # 无贴图解析（tex 为空）或贴图不在素材树 → missing 哨兵层。
        return tile_index.get(renamed[tex], sentinel_index) if tex else sentinel_index

    # --- 官方注册表名字清单（覆盖率测试用；名字是事实，可入仓） ---
    vanilla_sorted = sorted(entries)
    OUT_VANILLA.write_text(
        "// @generated by ci/gen-blocks.py — DO NOT EDIT.\n"
        "// 官方 26.1 方块注册表名字清单：提取自反编译 Blocks.java 注册序列"
        "（含 registerBed/registerStair/registerStainedGlass 辅助与\n"
        "// WeatheringCopperBlocks.create 的 8 变体）。只含名字（事实），不复制代码。\n"
        f"static VANILLA_26_1_BLOCKS: [&str; {len(vanilla_sorted)}] = [\n"
        + "".join(f'    "{n}",\n' for n in vanilla_sorted)
        + "];\n"
    )
    print(f"wrote {OUT_VANILLA} ({len(vanilla_sorted)} names)")

    # --- 生成 Rust 表 ---
    total = len(LEGACY) + sum(1 for n in blocks if n not in LEGACY_OFFICIAL)
    lines = [
        "// @generated by ci/gen-blocks.py — DO NOT EDIT.",
        "// DEVELOP_ONLY：官方 26.1 blockstates/models/Blocks.java 生成（同 assets/ 素材政策）。",
        "// 元组: (name, solid, opaque, liquid, light_emit, tiles [+X,-X,+Y,-Y,+Z,-Z], hardness, model_kind)",
        "// model_kind: 0=纯立方（全支持） 1=非立方（cross/楼梯/板/栅栏/多部件…，占位代表贴图）",
        "// tiles 为 tiles_manifest.json 层索引；全部真实贴图层 0..N-1（字典序），",
        "// 层 N = missing 哨兵（模型无贴图解析的方块，渲染为原版 missingno 品红）。",
        "// 注意：层 0 也是真实贴图（字典序第一张 acacia_door_bottom），不是空层哨兵。",
        "// hardness=MC strength()，基岩=inf；未显式 strength() 的方块按原版",
        "// destroyTime 默认 0.0（BlockBehaviour.java:976）。",
        "// id 0..13 = 旧 14 方块（id/名字/属性与旧表一致，地形生成器依赖）；14+ 官方名字典序。",
        "// snow_grass(id 11): 官方是 grass_block 的 snowy 状态；本引擎单方块模型，独立 id，取 snow 模型贴图。",
        "// tintindex 生物群系染色（草顶/羊齿/树叶三族）已实现：渲染侧按 tile 层查",
        "// mcv_core::tint 注册表乘生物群系色（26.1 BlockColors 等价）。",
        "// cross/torch/fence/slab/stairs 由网格器 shape 模板路径按 state 出几何",
        "// （cpp/src/mesher.cpp emit_shapes）；其余 kind=1 仍整盒占位、六面给代表贴图。",
        "// carpet/trapdoor/pane/wall（shape 6..9）渲染暂全盒占位，模板待 Rust",
        "// 网格器 #77；碰撞/拾取几何已在 engine/mcv_game/src/blockshapes.rs 按",
        "// 原版数值落地。",
        "#[allow(clippy::type_complexity)]",
        f"static GEN_BLOCKS: [(&str, bool, bool, bool, u8, [u16; 6], f32, u8); {total}] = [",
    ]
    cpp_rows = []  # (name, opaque, liquid, geom, tiles, shape) —— C++ kBlocks 同序行
    idx = 0
    for lname, _off, tiles in LEGACY:
        solid, opaque, liquid, light, hard, kind = LEGACY_ATTRS[lname]
        t = [tile_id(x) for x in tiles] if tiles else [sentinel_index] * 6
        cmt = ""
        if lname == "snow_grass":
            cmt = "  // = grass_block[snowy=true]，独立 id（单方块模型）"
        if lname in ("flower_red", "flower_yellow"):
            cmt = "  // cross 占位：旧表 6 面同贴图，保持不变"
        cpp_rows.append((lname, opaque, liquid, cpp_geom(lname, solid, liquid, kind), t,
                         shape_of_name(lname)))
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
            # 仅引擎自建伪方块（item_frame/glow_item_frame：原版是实体）会走到这。
            stats["java_name_miss"] += 1
            attrs = {"hardness": 0.0, "light": 0, "solid": True,
                     "opaque": True, "liquid": False}
        solid, opaque, liquid = attrs["solid"], attrs["opaque"], attrs["liquid"]
        used = [b["faces"][f] for f in FACE_ORDER if b["faces"][f]]
        if any(has_alpha(x) for x in used) \
                or any(w in name for w in NONOPAQUE_NAME_WORDS) \
                or b["kind"] == 1:
            opaque = False
        cpp_rows.append((name, opaque, liquid, cpp_geom(name, solid, liquid, b["kind"]), t,
                         shape_of_name(name)))
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
        "// 字段: { opaque, liquid, geom, tiles[+X,-X,+Y,-Y,+Z,-Z], shape }",
        "// geom: 空气/水（水走水 pass）与隐形方块（屏障/光源/空气族/结构空位/",
        "// 气泡柱）= false；非立方(kind=1)仍入不透明 pass = true（形状模板网格",
        "// 器按 shape 出几何）；其余立方实体/其他液体 = solid or liquid。",
        "// shape: mcv_core::Shape 判别值 0立方 1十字 2火把 3栅栏 4半砖 5楼梯，",
        "// 按注册名分类（shape_of_name，与 game/mcv_core/src/shape.rs 同源）。",
    ]
    for name, opaque, liquid, geom, t, shape in cpp_rows:
        cpp_lines.append(
            f"    {{{'true' if opaque else 'false'}, {'true' if liquid else 'false'}, "
            f"{'true' if geom else 'false'}, {{{', '.join(map(str, t))}}}, {shape}}},  // {name}"
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
