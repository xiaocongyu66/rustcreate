#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
方块覆盖审计：官方 26.1 blockstate 清单 vs 我方注册表，回答
“官方有而我们缺什么”，并把完全缺失名单写成 docs/missing-blocks.md。

可重复运行：任何时刻 `python3 ci/check-blocks.py` 都会重读两侧清单并
覆盖生成报告（开头带时间戳）。对并行的扩表代理保持防御式解析：
  - 我方现表：正则扫 game/mcv_core/src/lib.rs 里的 block!("name", ...)；
  - 生成表：若 game/mcv_core/src/blocks_gen.inc.rs 存在，正则扫其中
    ("name", ...) 元组首字段；文件不存在/格式变化都不报错，只降级为
    14 块基线。绝不编译 Rust、不跑 cargo/git。

用法：
  python3 ci/check-blocks.py                # 默认路径，写 docs/missing-blocks.md
  python3 ci/check-blocks.py --stdout-only  # 只打印摘要不写文件
  python3 ci/check-blocks.py --gen-file X --out Y  # 自定义路径（测试用）
退出码恒为 0（这是审计报表，不是 CI 门禁；扩表后复跑即可看覆盖率爬升）。
"""
import argparse
import datetime
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OFFICIAL_DEFAULT = "/root/mc-ref/src-26.1/assets/minecraft"

# 我方历史遗留名 -> 官方 26.1 规范名（仅现表用到的 oak-only 旧名）。
# GEN 表按规范名生成后直接精确匹配，别名只影响下面这几项。
OUR_ALIASES = {
    "grass": "grass_block",
    "snow_grass": "grass_block",
    "cobble": "cobblestone",
    "log": "oak_log",
    "planks": "oak_planks",
    "leaves": "oak_leaves",
    "flower_red": "poppy",
    "flower_yellow": "dandelion",
}

# ---------------------------------------------------------------- 我方清单
def strip_ns(name):
    return name.split(":", 1)[1] if name.startswith("minecraft:") else name

def parse_blocks_rs(path):
    """扫 lib.rs 的 BLOCKS：block!("name", ...) 首字段。文件缺失返回 None。"""
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8", errors="replace") as f:
        src = f.read()
    names = re.findall(r'block!\(\s*"([A-Za-z0-9_/.:-]+)"', src)
    if not names:  # 兜底：若宏被展开成 BlockDef 字面量
        names = re.findall(r'name:\s*"([A-Za-z0-9_/.:-]+)"', src)
    return [strip_ns(n) for n in names]

def parse_gen_inc(path):
    """扫 blocks_gen.inc.rs 的 GEN_BLOCKS：("name", 首字段。缺失返回 None。"""
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8", errors="replace") as f:
        src = f.read()
    names = re.findall(r'\(\s*"([A-Za-z0-9_/.:-]+)"\s*,', src)
    return [strip_ns(n) for n in names]

# ------------------------------------------------------- 官方清单 + 贴图链
class AssetIndex:
    """官方 blockstates/ models/ textures/ 的只读索引，带缓存与环保护。"""
    def __init__(self, root):
        self.root = root
        self.bs_dir = os.path.join(root, "blockstates")
        self.model_dir = os.path.join(root, "models")
        self.tex_dir = os.path.join(root, "textures")
        self._models = {}
        self._tex_ok = {}

    def blockstates(self):
        if not os.path.isdir(self.bs_dir):
            raise SystemExit(f"官方 blockstates 目录不存在: {self.bs_dir}")
        return sorted(f[:-5] for f in os.listdir(self.bs_dir)
                      if f.endswith(".json"))

    def _load_model(self, model_id):
        if model_id in self._models:
            return self._models[model_id]
        fp = os.path.join(self.model_dir, model_id.split(":", 1)[-1] + ".json")
        try:
            with open(fp, encoding="utf-8") as f:
                data = json.load(f)
            self._models[model_id] = data if isinstance(data, dict) else False
        except (OSError, ValueError):
            self._models[model_id] = False
        return self._models[model_id]

    def _tex_exists(self, tex_ref):
        if tex_ref in self._tex_ok:
            return self._tex_ok[tex_ref]
        rel = tex_ref.split(":", 1)[-1]
        ok = os.path.isfile(os.path.join(self.tex_dir, rel + ".png"))
        self._tex_ok[tex_ref] = ok
        return ok

    def _merged_texmap(self, model_id, visiting):
        """沿 parent 链自根向下合并 textures 变量表（子覆盖父）。
        返回 (merged_map, 是否有缺档/悬空引用)。"""
        if model_id in visiting or len(visiting) > 16:
            return {}, False
        m = self._load_model(model_id)
        if m is False:
            return {}, True
        texmap, bad = {}, False
        parent = m.get("parent")
        if isinstance(parent, str):
            pm, pb = self._merged_texmap(parent, visiting | {model_id})
            texmap.update(pm)
            bad = bad or pb
        if isinstance(m.get("textures"), dict):
            texmap.update(m["textures"])
        return texmap, bad

    def _texture_ids(self, model_id, visiting):
        """返回 (引用到的贴图 id 集合, 是否有解析失败)。"""
        texmap, bad = self._merged_texmap(model_id, visiting)
        out = set()
        def resolve(v, depth=0):
            vals = []
            if isinstance(v, str):
                if v.startswith("#"):
                    if depth < 8 and v[1:] in texmap:
                        return resolve(texmap[v[1:]], depth + 1)
                    return None  # 变量悬空
                vals.append(v)
            elif isinstance(v, list):
                for it in v:
                    r = resolve(it, depth + 1)
                    if r is None:
                        vals.extend([])
                    else:
                        vals.extend(r)
            elif isinstance(v, dict):
                return resolve(v.get("texture"), depth + 1) or []
            return vals
        for v in texmap.values():
            r = resolve(v)
            if r is None:
                bad = True
            else:
                out.update(x for x in r if isinstance(x, str))
        return out, bad

    def _models_of_blockstate(self, name):
        """从 blockstate JSON 收集所有 model 引用；解析失败返回 None。"""
        fp = os.path.join(self.bs_dir, name + ".json")
        try:
            with open(fp, encoding="utf-8") as f:
                data = json.load(f)
        except (OSError, ValueError):
            return None
        models, stack = set(), []
        vs = data.get("variants")
        if isinstance(vs, dict):
            for v in vs.values():
                stack.append(v)
        mp = data.get("multipart")
        if isinstance(mp, list):
            for part in mp:
                if isinstance(part, dict):
                    stack.append(part.get("apply"))
        while stack:
            v = stack.pop()
            if isinstance(v, list):
                stack.extend(v)
            elif isinstance(v, dict):
                if isinstance(v.get("model"), str):
                    models.add(v["model"])
                if "apply" in v:
                    stack.append(v["apply"])
        return models

    def texture_report(self, name, local_tiles):
        """返回 (贴图列文案, 备注)。local_tiles: 我方 texturepack 的 png 名集合。"""
        models = self._models_of_blockstate(name)
        if models is None:
            return "未知", "blockstate JSON 解析失败"
        refs, bad = set(), False
        for mid in models:
            r, b = self._texture_ids(mid, set())
            refs.update(r)
            bad = bad or b
        if not refs:
            note = "无模型/贴图引用" if not models else "模型无贴图（占位/空气类）"
            if bad:
                note += "；部分模型文件缺失"
                return "未知", note
            return "无", note
        exist = [t for t in refs if self._tex_exists(t)]
        local_hit = sorted({t.split(":")[-1].split("/")[-1] for t in exist}
                           & local_tiles)
        if exist:
            col = "是"
            note = f"官方贴图 {len(exist)}/{len(refs)} 张存在"
            if local_hit:
                note += f"；本地已有 {len(local_hit)} 张: {', '.join(local_hit[:4])}"
        else:
            col = "引用缺贴图"
            note = f"引用 {len(refs)} 张贴图均不在官方 textures/ 下"
        if bad and not exist:
            col = "未知"
            note += "；且模型链解析有缺档"
        return col, note

# ---------------------------------------------------------------- 类别粗分
# 顺序即优先级，首个命中即定型（关键词按 _ 分词后 token 匹配 + 少量前缀）。
FURNITURE = {"bed","chest","shelf","lectern","bookshelf","sign","ladder",
             "candle","lantern","brazier","chandelier","skull","head",
             "banner","table","loom","barrel","composter","hive","nest",
             "cauldron","campfire","furnace","pot","frame","painting",
             "door","trapdoor","jar","grinder","anvil"}
CROP = {"wheat","carrots","potato","potatoes","beetroot","pumpkin","melon",
        "stem","cocoa","bamboo","cane","cactus","berry","berries","wart",
        "kelp","seagrass","sapling","torchflower","pitcher","sprouts",
        "bush","flower"}
METAL = {"iron","gold","golden","copper","netherite","chain","debris","hopper",
         "rail"}
STONE_TOKENS = {"stone","sandstone","granite","diorite","andesite","tuff",
                "calcite","obsidian","bricks","brick","pale","mudstone"}
STONE_PREFIX = ("smooth_","polished_","mossy_","cobbled_","cracked_")

def classify(name):
    t = name.split("_")
    if t[-1] in ("slab", "stairs", "wall") or "stonecutter" in t \
       or name.startswith("chiseled_"):
        stonecut = True
    else:
        stonecut = False
    if FURNITURE & set(t):            return "furniture"
    if CROP & set(t):                 return "crop"
    if "ore" in t:                    return "ore"
    if METAL & set(t):                return "metal"
    if "wool" in t or "carpet" in t:  return "wool"
    if "concrete" in t:               return "concrete"
    if "terracotta" in t:             return "terracotta"
    if "glass" in t or "glasspane" in t: return "glass"
    if {"log","wood","stem","hyphae"} & set(t): return "log"
    if "planks" in t:                 return "planks"
    if stonecut:                      return "stonecutting"
    if (STONE_TOKENS & set(t)) or name.startswith(STONE_PREFIX):
        return "stone_variant"
    return "其他"

CATEGORY_ORDER = ["log","planks","wool","concrete","terracotta","glass",
                  "stone_variant","metal","ore","crop","furniture",
                  "stonecutting","其他"]

# ---------------------------------------------------------------- 主流程
def main():
    ap = argparse.ArgumentParser(description="方块覆盖审计")
    ap.add_argument("--official-dir", default=OFFICIAL_DEFAULT,
                    help="官方 assets/minecraft 目录（仅本机路径）")
    ap.add_argument("--blocks-rs",
                    default=os.path.join(REPO, "game/mcv_core/src/lib.rs"))
    ap.add_argument("--gen-file",
                    default=os.path.join(REPO, "game/mcv_core/src/blocks_gen.inc.rs"))
    ap.add_argument("--local-tiles",
                    default=os.path.join(REPO, "texturepack"))
    ap.add_argument("--out", default=os.path.join(REPO, "docs/missing-blocks.md"))
    ap.add_argument("--stdout-only", action="store_true")
    a = ap.parse_args()

    official = AssetIndex(a.official_dir).blockstates()
    base = parse_blocks_rs(a.blocks_rs) or []
    gen = parse_gen_inc(a.gen_file)          # None = 文件不存在
    ours_raw = list(dict.fromkeys(base + (gen or [])))
    ours = {OUR_ALIASES.get(n, n) for n in ours_raw}
    official_set = set(official)
    matched = ours & official_set
    missing = [n for n in official if n not in matched]

    local_tiles = set()
    if os.path.isdir(a.local_tiles):
        local_tiles = {f[:-4] for f in os.listdir(a.local_tiles)
                       if f.endswith(".png")}

    rows = []
    idx = AssetIndex(a.official_dir)
    for n in missing:
        tex, note = idx.texture_report(n, local_tiles)
        rows.append((n, classify(n), tex, note))

    cat_count = {c: 0 for c in CATEGORY_ORDER}
    for _, c, _, _ in rows:
        cat_count[c] += 1

    cov = 100.0 * len(matched) / len(official) if official else 0.0
    ts = datetime.datetime.now().astimezone().strftime("%Y-%m-%d %H:%M:%S %z")
    gen_desc = "不存在（仅 14 块基线）" if gen is None else f"{len(gen)} 条"

    # ---- stdout 摘要
    print(f"[check-blocks] 基线时间戳: {ts}")
    print(f"[check-blocks] 官方 blockstate: {len(official)}  "
          f"我方注册: {len(ours_raw)} (BLOCKS={len(base)}, GEN={gen_desc})  "
          f"名称匹配(含别名): {len(matched)}")
    print(f"[check-blocks] 覆盖率: {cov:.2f}%   完全缺失: {len(missing)}")
    print("[check-blocks] 缺失类别 top:")
    for c, n in sorted(cat_count.items(), key=lambda kv: -kv[1])[:5]:
        print(f"    {c:14s} {n:5d}")

    if a.stdout_only:
        return 0

    # ---- 写报告
    lines = []
    lines.append("# 方块覆盖审计 — 缺失清单基线")
    lines.append("")
    lines.append("> 由 `ci/check-blocks.py` 自动生成，勿手改。复跑："
                 "`python3 ci/check-blocks.py`。")
    lines.append("")
    lines.append(f"- 基线时间戳：{ts}")
    lines.append(f"- 官方 blockstate（26.1）：{len(official)}")
    lines.append(f"- 我方注册：BLOCKS 现表 {len(base)} + GEN_BLOCKS {gen_desc}"
                 f" = {len(ours_raw)} 条")
    alias_hits = {n for n in ours_raw if OUR_ALIASES.get(n, n) != n
                  and OUR_ALIASES.get(n, n) in matched}
    lines.append(f"- 名称匹配（含历史别名映射，如 cobble→cobblestone，"
                 f"命中 {len(alias_hits)} 个别名）：{len(matched)}")
    lines.append(f"- **覆盖率：{cov:.2f}%**；完全缺失：**{len(missing)}**")
    lines.append("")
    lines.append("## 按类别缺失统计（按名字关键词粗分，首命中优先）")
    lines.append("")
    lines.append("| 类别 | 缺失数 | 占缺失比例 |")
    lines.append("|---|---:|---:|")
    for c, n in sorted(cat_count.items(), key=lambda kv: -kv[1]):
        pct = 100.0 * n / len(rows) if rows else 0.0
        lines.append(f"| {c} | {n} | {pct:.1f}% |")
    lines.append("")
    lines.append("## 完全缺失名单")
    lines.append("")
    lines.append("| 官方名 | 类别 | 是否有贴图引用 | 备注 |")
    lines.append("|---|---|---|---|")
    for n, c, tex, note in rows:
        lines.append(f"| `{n}` | {c} | {tex} | {note} |")
    lines.append("")
    os.makedirs(os.path.dirname(a.out), exist_ok=True)
    with open(a.out, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    print(f"[check-blocks] 报告已写入: {a.out}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
