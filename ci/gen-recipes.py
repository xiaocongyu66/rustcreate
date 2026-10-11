#!/usr/bin/env python3
# DEVELOP_ONLY 数据管道（同 ci/gen-soundtypes.py / ci/gen-blocks.py 政策）：
#   从 26.1 数据包配方 JSON 生成「凡输入输出都在本仓 ITEMS 注册表内」的配方
#   全表 game/mcv_item/src/recipes_gen.inc.rs（crafting.rs 手写表之外的生成表，
#   匹配走「手写 ∪ 生成」），并把其余配方登记进 ci/recipes-gap.md
#   （缺口 = 后续波次的输入：缺物品挡住的配方 + 未接线机制债）。
#
# 用法（仓库根目录）:
#   MCV_REF_SRC=<26.1解包树根> python3 ci/gen-recipes.py
#
# 数据源（仅本机参考、绝不入仓，路径经环境变量注入；零 Mojang JSON 入库）：
#   $MCV_REF_SRC/data/minecraft/recipe/*.json     —— 配方（JSON 即权威）
#   $MCV_REF_SRC/data/minecraft/tags/item/*.json  —— tag 展开源
#   game/mcv_item/src/lib.rs def! 表              —— ITEMS 注册名 → id 映射
#
# 翻译规则：
#   - 物品名 "minecraft:x" → 注册名（oak 族旧名经 LEGACY_ALIASES）→ ITEMS 下标；
#   - "#minecraft:<tag>" → 展开成员中已注册子集（宁缺勿错，同手写表语义）；
#   - 成员列表（alternatives）同 tag：取已注册子集，全缺才挡，部分缺记窄化；
#   - 凡任一输入或输出无已注册 id → 跳过该配方并记入缺口报告；
#   - 只接 crafting_shaped / crafting_shapeless / smelting（blasting/campfire/
#     smoking/stonecutting/smithing/special 等机制 = 债，登记不生成）。
# 产物:
#   game/mcv_item/src/recipes_gen.inc.rs —— GEN_SHAPED/GEN_SHAPELESS/GEN_COOKING
#   ci/recipes-gap.md                    —— 缺口报告（入库，测试锁存在性）
#   stdout 摘要（CI job 日志可读）
import json
import os
import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REF = os.environ.get("MCV_REF_SRC")
if not REF:
    sys.exit("缺 MCV_REF_SRC=<26.1解包树根>（同 ci/gen-soundtypes.py）")
REF = Path(REF)
RECIPES = REF / "data/minecraft/recipe"
TAGS = REF / "data/minecraft/tags/item"
LIB = ROOT / "game/mcv_item/src/lib.rs"
CRAFTING = ROOT / "game/mcv_item/src/crafting.rs"
OUT = ROOT / "game/mcv_item/src/recipes_gen.inc.rs"
GAP_MD = ROOT / "ci/recipes-gap.md"

COOK_TICKS_KEY = "cookingtime"

# 旧 14 方块名（id 0..13 旧位）→ 官方注册名（同 ci/gen-soundtypes.py LEGACY；
# 仅 oak 族 log/planks/leaves 出现在配方物品名，其余旧名不进配方 JSON）。
LEGACY_ALIASES = {
    "oak_log": "log",
    "oak_planks": "planks",
    "oak_leaves": "leaves",
}

# ---- ITEMS 注册表（def! 宏序 = id）------------------------------------------
DEF_RE = re.compile(r'def!\(\s*"([a-z0-9_/]+)"')
lib_src = LIB.read_text(encoding="utf-8")
tbl_start = lib_src.index("pub static ITEMS")
tbl = lib_src[tbl_start:lib_src.index("];", tbl_start)]
ITEM_NAMES = DEF_RE.findall(tbl)
assert ITEM_NAMES, "lib.rs ITEMS 表解析为空"
assert ITEM_NAMES[0] == "stick", f"ITEMS[0] 应为 stick，实为 {ITEM_NAMES[0]}"
NAME2ID = {n: i for i, n in enumerate(ITEM_NAMES)}


def resolve_item(vname: str):
    """官方注册名（可带 minecraft: 前缀）→ 本仓物品 id；未注册 None。"""
    name = vname.removeprefix("minecraft:")
    name = LEGACY_ALIASES.get(name, name)
    return NAME2ID.get(name)


def display(vname: str) -> str:
    return vname.removeprefix("minecraft:")


# ---- tag 展开（memo + 环守卫；只收编已注册成员，宁缺勿错）--------------------
_tag_memo = {}
_tag_in_progress = set()


def tag_members(tag: str):
    """tag 名（无 # 前缀）→ 已注册成员 id 排序去重；缺文件/环 = 空表。"""
    if tag in _tag_memo:
        return _tag_memo[tag]
    assert tag not in _tag_in_progress, f"tag 环引用: {tag}"
    _tag_in_progress.add(tag)
    members = []
    try:
        d = json.loads((TAGS / f"{tag}.json").read_text(encoding="utf-8"))
        for v in d.get("values", []):
            if v.startswith("#"):
                members += tag_members(v.removeprefix("#minecraft:"))
            else:
                i = resolve_item(v)
                if i is not None:
                    members.append(i)
    except FileNotFoundError:
        members = []
    _tag_in_progress.discard(tag)
    _tag_memo[tag] = sorted(set(members))
    return _tag_memo[tag]


def expand_ingredient(val):
    """ingredient（str / str 列表）→ (ids, missing, dropped)。

    ids = 已注册子集（可能窄化，保序去重）；missing = 全部成员都未注册时的
    挡路名单（tag 记 #tag 名，物品记注册名）；dropped = 部分窄化时被丢弃的
    未注册成员名（缺口报告「窄化」节，不影响该配方进表）。
    """
    vals = val if isinstance(val, list) else [val]
    ids = []
    missing = []
    dropped = []
    for v in vals:
        if not isinstance(v, str):
            missing.append(repr(v))
            continue
        if v.startswith("#"):
            tag = v.removeprefix("#minecraft:")
            got = tag_members(tag)
            if got:
                ids += got
            else:
                missing.append(f"#{tag}")
        else:
            i = resolve_item(v)
            if i is not None:
                ids.append(i)
            else:
                missing.append(display(v))
                dropped.append(display(v))
    seen = set()
    ids = [i for i in ids if not (i in seen or seen.add(i))]
    return ids, missing, dropped


def ingredient_label(val):
    """注释用来源标签（配方 JSON 的 key/ingredient 原文，去前缀；长列表截断）。"""
    if isinstance(val, str):
        if val.startswith("#"):
            return "#" + val.removeprefix("#minecraft:")
        return val.removeprefix("minecraft:")
    names = [display(v).removeprefix("minecraft:") for v in val if isinstance(v, str)]
    if len(names) > 4:
        return "+".join(names[:4]) + f"+…(共{len(names)}项)"
    return "+".join(names)


# ---- 手写表 id 集（重叠统计；wood_shape! 宏字面 id 与 id: "x" 都算）----------
crafting_src = CRAFTING.read_text(encoding="utf-8")
HAND_IDS = set(re.findall(r'id:\s*"([a-z0-9_]+)"', crafting_src))
HAND_IDS |= set(re.findall(r'wood_shape!\(\s*\w+,\s*"([a-z0-9_]+)"', crafting_src))
HAND_COOK_IDS = set(re.findall(r'id:\s*"([a-z0-9_]+)"[^}]*?cook_time', crafting_src, re.S))

# ---- 配方解析 ----------------------------------------------------------------
gen_shaped = []  # (recipe_id, category, pattern, keys[(ch, ids, label)], out_id, out_n)
gen_shapeless = []  # (recipe_id, category, ingredients[(ids, label)], out_id, out_n)
gen_smelting = []  # (recipe_id, category, ids, label, out_id, out_n, cook_ticks)
parsed = Counter()
skipped = Counter()  # type -> 因缺物品跳过数
debt = Counter()  # 未接线机制/形态 -> 数量
blocked = {}  # 物品名 -> [(recipe_id, role, type)]（挡路物品）
narrowed = Counter()  # 窄化来源 -> 丢弃成员数
narrowed_by = {}  # 窄化来源 -> set(丢弃未注册名)


def note_blocked(recipe_id, missing, role, rtype):
    for m in missing:
        blocked.setdefault(m, []).append((recipe_id, role, rtype))


def note_narrowed(label, dropped):
    if dropped:
        narrowed[label] += len(dropped)
        narrowed_by.setdefault(label, set()).update(dropped)


def result_of(d):
    """result → (官方物品名, count)；components 结果/异常形态 → (None, 0)。"""
    r = d.get("result")
    if not isinstance(r, dict) or "id" not in r:
        return None, 0
    if "components" in r:
        return None, 0  # 带组件（附魔/药水等自定义数据）= 机制债，非本波
    return r["id"], r.get("count", 1)


def shrink_pattern(pattern):
    """26.1 ShapedRecipePattern.shrink（:80-115）：配方 pattern 在 unpack 时
    裁掉首尾全空行/列——本仓 matches_shaped 对网格做 bbox 裁剪后要求图案
    bbox 紧致，生成表必须同样 shrink（26.1 全表 24 条受影响：shelf/boots/
    spyglass 等；本波可收子集恰无此形态，落表即错，故按原版实现保险）。"""
    rows = [row for row in pattern]
    # 首尾全空行。
    top = 0
    while top < len(rows) and not any(c != " " for c in rows[top]):
        top += 1
    bottom = len(rows)
    while bottom > top and not any(c != " " for c in rows[bottom - 1]):
        bottom -= 1
    rows = rows[top:bottom]
    if not rows:
        return []
    # 首尾全空列（各行等宽）。
    w = len(rows[0])
    left = 0
    while left < w and not any(row[left] != " " for row in rows):
        left += 1
    right = w
    while right > left and not any(row[right - 1] != " " for row in rows):
        right -= 1
    return [row[left:right] for row in rows]


files = sorted(p.name for p in RECIPES.glob("*.json"))
assert files, "recipe 目录解析为空"
for fname in files:
    d = json.loads((RECIPES / fname).read_text(encoding="utf-8"))
    rid = fname[: -len(".json")]
    rtype = d.get("type", "").removeprefix("minecraft:")
    parsed[rtype] += 1
    if rtype not in ("crafting_shaped", "crafting_shapeless", "smelting"):
        debt[rtype] += 1
        continue
    category = d.get("category", "-")
    out_v, out_n = result_of(d)
    if out_v is None:
        debt[f"{rtype}（components 结果）"] += 1
        continue
    out_id = resolve_item(out_v)
    if out_id is None:
        note_blocked(rid, [display(out_v)], "输出", rtype)
        skipped[rtype] += 1
        continue
    if rtype == "smelting":
        ids, missing, dropped = expand_ingredient(d["ingredient"])
        if not ids:
            note_blocked(rid, missing, "输入", rtype)
            skipped[rtype] += 1
            continue
        note_narrowed(f"smelting/{rid}", dropped)
        gen_smelting.append(
            (rid, category, ids, ingredient_label(d["ingredient"]), out_id, out_n, d.get(COOK_TICKS_KEY, 200))
        )
        continue
    if rtype == "crafting_shaped":
        pattern = d.get("pattern", [])
        keys = d.get("key", {})
        pattern = shrink_pattern(pattern)
        if not pattern:
            sys.exit(f"{fname}: pattern 全空（26.1 shrink 后为空 = 无效配方）")
        pw = len(pattern[0])
        if (
            not pattern
            or any(len(row) != pw for row in pattern)
            or pw > 3
            or len(pattern) > 3
        ):
            sys.exit(f"{fname}: pattern 超 3x3 或行宽不齐（26.1 MAX_SIZE=3）")
        if any(" " in k for k in keys):
            sys.exit(f"{fname}: key 含空格（' ' 是 26.1 EMPTY_SLOT 保留字符）")
        resolved = []
        missing_all = []
        for ch, val in keys.items():
            ids, missing, dropped = expand_ingredient(val)
            note_narrowed(f"{rid}[{ch}]", dropped)
            if not ids:
                missing_all += missing or ["?"]
                continue
            resolved.append((ch, ids, ingredient_label(val)))
        if len(resolved) != len(keys):
            note_blocked(rid, missing_all, "输入", rtype)
            skipped[rtype] += 1
            continue
        # 图案引用的字符必须有 key（26.1 unpack 缺 key = 配方反序列化失败）。
        if any(c != " " and c not in keys for row in pattern for c in row):
            sys.exit(f"{fname}: pattern 引用未定义 key")
        gen_shaped.append((rid, category, pattern, resolved, out_id, out_n))
        continue
    # crafting_shapeless
    ingredients = d.get("ingredients", [])
    resolved = []
    missing_all = []
    for val in ingredients:
        ids, missing, dropped = expand_ingredient(val)
        note_narrowed(f"{rid}", dropped)
        if not ids:
            missing_all += missing or ["?"]
            continue
        resolved.append((ids, ingredient_label(val)))
    if not resolved or len(resolved) != len(ingredients) or len(resolved) > 9:
        note_blocked(rid, missing_all, "输入", rtype)
        skipped[rtype] += 1
        continue
    gen_shapeless.append((rid, category, resolved, out_id, out_n))

# ---- 完整性断言 --------------------------------------------------------------
assert parsed, "未解析到任何配方"
for t in ("crafting_shaped", "crafting_shapeless", "smelting"):
    assert t in parsed, f"26.1 数据缺 {t}（异常，数据源对不上）"
for entries, out_idx in ((gen_shaped, 4), (gen_shapeless, 3), (gen_smelting, 4)):
    for e in entries:
        assert 0 <= e[out_idx] < len(ITEM_NAMES), f"产物 id 越界: {e}"
        if out_idx == 4:
            assert 1 <= e[5] <= 64, f"产物 count 异常: {e}"
# 生成表内 recipe id 唯一（文件名即 id）。
all_rids = [e[0] for e in gen_shaped] + [e[0] for e in gen_shapeless] + [e[0] for e in gen_smelting]
assert len(all_rids) == len(set(all_rids)), "生成表 recipe id 重复"


# ---- inc.rs 产物 -------------------------------------------------------------
def rs_ids(ids, label):
    """[u16] 字面量 + 尾注注册名（可读性；id = ITEMS 下标）。"""
    names = ", ".join(ITEM_NAMES[i] for i in ids)
    if len(ids) == 1 and label == ITEM_NAMES[ids[0]]:
        return f"&[{ids[0]}] /* {names} */"
    return f"&[{', '.join(str(i) for i in ids)}] /* {label}: {names} */"


out = []
out.append("// @generated by ci/gen-recipes.py — DO NOT EDIT.")
out.append("// DEVELOP_ONLY 数据管道（同 blocks_gen.inc.rs / sound_types.inc.rs 资产政策）：")
out.append("// 从 Mojang 26.1 数据包配方 JSON 生成的配方全表——只收「凡输入输出都在")
out.append("// 本仓 ITEMS 注册表内」的条目。数据源（本机路径经 MCV_REF_SRC 环境变量")
out.append("// 注入，绝不入仓；零 Mojang JSON 入库，只生成 Rust 常数表）：")
out.append("//   data/minecraft/recipe/*.json     —— 配方（JSON 即权威，逐条注释引文件名）")
out.append("//   data/minecraft/tags/item/*.json  —— tag 展开（只收已注册成员，宁缺勿错）")
out.append("// 缺 id 配方不进表，记 ci/recipes-gap.md（缺口 = 后续波次输入）。")
out.append("// 再生成: MCV_REF_SRC=<26.1解包树根> python3 ci/gen-recipes.py")
out.append("//")
out.append("// 合并语义（crafting.rs）：匹配走「手写表 ∪ 生成表」，手写优先。生成表是")
out.append("// JSON 直译的超集，与手写表（#137 历史先行件，保留不削）重叠的条目同 id 同")
out.append(
    "// 结果，材料面只会更宽（原版 tag 直译 ⊇ 手写手收子集；本表与手写表重叠 %d 条）。"
    % len((set(e[0] for e in gen_shaped) | set(e[0] for e in gen_shapeless) | set(e[0] for e in gen_smelting)) & HAND_IDS)
)
out.append("// 镜像：26.1 JSON 只含实际 pattern（ShapedRecipePattern.Data 无镜像字段），")
out.append("// 镜像在运行期由 matches_shaped 的 xFlip 分支覆盖（ShapedRecipePattern.java")
out.append("// :159-171 —— symmetrical==false 时 matches(input, true) 同样命中）。")
out.append("// 物品 id = game/mcv_item/src/lib.rs ITEMS 下标（u16），尾注 = 注册名。")
out.append("")
out.append(f"/// 26.1 crafting_shaped 可收子集（{len(gen_shaped)} 条；文件名字典序 = 配方 id 序）。")
out.append(f"pub static GEN_SHAPED: [ShapedRecipe; {len(gen_shaped)}] = [")
for rid, cat, pattern, keys, out_id, out_n in gen_shaped:
    overlap = "；手写表重叠" if rid in HAND_IDS else ""
    out.append(f"    /* recipe/{rid}.json（category={cat}{overlap}）*/")
    out.append("    ShapedRecipe {")
    pat = ", ".join(f'"{row}"' for row in pattern)
    out.append(f"        id: \"{rid}\",")
    out.append(f"        pattern: &[{pat}],")
    if len(keys) == 1:
        ch, ids, label = keys[0]
        out.append(f"        keys: &[('{ch}', {rs_ids(ids, label)})],")
    else:
        out.append("        keys: &[")
        for ch, ids, label in keys:
            out.append(f"            ('{ch}', {rs_ids(ids, label)}),")
        out.append("        ],")
    out.append(f"        result: ({out_id}, {out_n}), // {ITEM_NAMES[out_id]}")
    out.append("    },")
out.append("];")
out.append("")
out.append(f"/// 26.1 crafting_shapeless 可收子集（{len(gen_shapeless)} 条）。")
out.append(f"pub static GEN_SHAPELESS: [ShapelessRecipe; {len(gen_shapeless)}] = [")
for rid, cat, ingredients, out_id, out_n in gen_shapeless:
    overlap = "；手写表重叠" if rid in HAND_IDS else ""
    out.append(f"    /* recipe/{rid}.json（category={cat}{overlap}）*/")
    out.append("    ShapelessRecipe {")
    out.append(f"        id: \"{rid}\",")
    out.append("        ingredients: &[")
    for ids, label in ingredients:
        out.append(f"            {rs_ids(ids, label)},")
    out.append("        ],")
    out.append(f"        result: ({out_id}, {out_n}), // {ITEM_NAMES[out_id]}")
    out.append("    },")
out.append("];")
out.append("")
out.append(f"/// 26.1 smelting 可收子集（{len(gen_smelting)} 条；cookingtime 全表 200）。")
out.append("/// blasting/smoking/campfire_cooking 未接线（债，见 ci/recipes-gap.md）。")
out.append(f"pub static GEN_COOKING: [CookingRecipe; {len(gen_smelting)}] = [")
for rid, cat, ids, label, out_id, out_n, ticks in gen_smelting:
    out.append(f"    /* recipe/{rid}.json（category={cat}）*/")
    out.append("    CookingRecipe {")
    out.append(f"        id: \"{rid}\",")
    out.append(f"        input: {rs_ids(ids, label)},")
    out.append(f"        result: ({out_id}, {out_n}), // {ITEM_NAMES[out_id]}")
    out.append(f"        cook_time: {ticks},")
    out.append("    },")
out.append("];")

OUT.write_text("\n".join(out) + "\n", encoding="utf-8")

# ---- 缺口报告 ci/recipes-gap.md ----------------------------------------------
GAP_MD.parent.mkdir(parents=True, exist_ok=True)
md = []
md.append("<!-- @generated by ci/gen-recipes.py — DO NOT EDIT. 再生成: MCV_REF_SRC=<26.1解包树根> python3 ci/gen-recipes.py -->")
md.append("# 配方缺口报告（26.1 → 本仓 ITEMS）")
md.append("")
md.append("数据源：Mojang 26.1 数据包 `data/minecraft/recipe/*.json`（%d 个，本机路径经" % len(files))
md.append("`MCV_REF_SRC` 注入，不入仓）。进表规则：凡输入输出都在本仓 ITEMS 注册表内")
md.append("（`game/mcv_item/src/recipes_gen.inc.rs`，匹配走「手写 ∪ 生成」）；其余在此")
md.append("登记为缺口/债——**本报告是后续波次的输入清单**（物品注册另波，不扩 ITEMS）。")
md.append("")
md.append("## 概览")
md.append("")
md.append("| 机制 | 解析 | 进表 | 缺物品跳过 |")
md.append("|---|---|---|---|")
gen_count = {"crafting_shaped": len(gen_shaped), "crafting_shapeless": len(gen_shapeless), "smelting": len(gen_smelting)}
for t in ("crafting_shaped", "crafting_shapeless", "smelting"):
    md.append(f"| {t} | {parsed.get(t, 0)} | {gen_count[t]} | {skipped.get(t, 0)} |")
md.append(
    "（差值 = components 结果等机制债，见下；「缺物品跳过」只数输入/输出缺注册的条目。）"
)
md.append("")
md.append("## 挡路物品（缺注册 → 挡住的配方；按挡住数降序，同数按名字典序）")
md.append("")
md.append("| 物品 | 挡住数 | 配方（角色·机制） |")
md.append("|---|---|---|")
for name in sorted(blocked, key=lambda n: (-len(blocked[n]), n)):
    lst = blocked[name]
    shown = ", ".join(f"{rid}（{role}·{t.removeprefix('crafting_')}）" for rid, role, t in lst[:12])
    more = f" …等 {len(lst)} 条" if len(lst) > 12 else ""
    md.append(f"| {name} | {len(lst)} | {shown}{more} |")
md.append("")
md.append("## 窄化（tag/列表有已注册成员，未注册成员被丢弃——宁缺勿错，配方仍进表）")
md.append("")
if narrowed:
    md.append("| 来源 | 丢弃未注册成员 |")
    md.append("|---|---|")
    for label in sorted(narrowed):
        members = ", ".join(sorted(narrowed_by.get(label, set())))
        md.append(f"| {label} | {members} |")
else:
    md.append("（无）")
md.append("")
md.append("## 债（未接线的配方机制——本波只接 shaped/shapeless/smelting）")
md.append("")
md.append("| 机制 | 配方数 |")
md.append("|---|---|")
for t, n in sorted(debt.items(), key=lambda kv: (-kv[1], kv[0])):
    md.append(f"| {t} | {n} |")
md.append("")
md.append("blasting / smoking / campfire_cooking 的燃料值与 100 刻烹饪时间待「燃料表」波；")
md.append("stonecutting 待切石机；smithing_* 待锻造台；special_* 为代码特型（非数据表）。")
GAP_MD.write_text("\n".join(md) + "\n", encoding="utf-8")

# ---- stdout 摘要 -------------------------------------------------------------
print(f"解析 {len(files)} 配方：shaped 进表 {len(gen_shaped)}/{parsed['crafting_shaped']}、"
      f"shapeless {len(gen_shapeless)}/{parsed['crafting_shapeless']}、"
      f"smelting {len(gen_smelting)}/{parsed['smelting']}")
overlap = len((set(e[0] for e in gen_shaped) | set(e[0] for e in gen_shapeless) | set(e[0] for e in gen_smelting)) & HAND_IDS)
print(f"与手写表重叠 {overlap} 条（语义等价，匹配手写优先）；净新增 {len(all_rids) - overlap} 条")
print(f"缺物品挡住配方 {sum(len(v) for v in blocked.values())} 条（挡路物品 {len(blocked)} 种，"
      f"top3: {', '.join(sorted(blocked, key=lambda n: (-len(blocked[n]), n))[:3])}）")
print(f"债机制 {len(debt)} 类 {sum(debt.values())} 条（stonecutting {debt.get('stonecutting', 0)}、"
      f"blasting {debt.get('blasting', 0)} 等）")
print(f"写出 {OUT}")
print(f"写出 {GAP_MD}")
