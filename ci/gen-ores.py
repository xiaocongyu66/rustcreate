#!/usr/bin/env python3
"""从原版 26.1 worldgen JSON 生成矿石生成参数表 ores_gen.inc.rs（#178 刀3a）。

输入（只读本机参考树，绝不入仓；寻径见 genlib.input_root）：
    data/minecraft/worldgen/biome/<repo 10 群系>.json      （features[6] 表）
    data/minecraft/worldgen/placed_feature/ore_*.json      （placement 链）
    data/minecraft/worldgen/configured_feature/ore_*.json  （targets/size/discard）
    data/minecraft/tags/block/*.json + blocks_gen.inc.rs   （经 gen-tags.py
                                                            parse_blocks_table）

抽取规则（照抄原版数据链，零手工名单）：
    - GenerationStep.java:8-18 Decoration 枚举序，features 数组下标 6 =
      UNDERGROUND_ORES；本仓 10 群系（biomes.rs Biome 枚举序）各自的
      features[6] 取并集，行序 = plains 序（10 群系表实测同序，脚本断言）。
    - configured type != "minecraft:ore" → 跳过（OreFeature 执行器只吃
      ore 型；nether/末地群系不在 10 群系表 → quartz 等自动出局）。
    - placed 链实物模式 [count|rarity_filter, in_square, height_range,
      biome]（PlacedFeature.java:57-77 lazy flatMap 消费序的执行器对位在
      ore_pass.rs）；出现未知 modifier 类型即报错退出（从严，不静默丢数据）。
    - 高度锚 → 本仓世界 y（= 原生 y，min=-64/genDepth=384，
      VerticalAnchor.java:55/72/89：absolute=v；above_bottom=minGenY+v；
      below_top=minGenY+genDepth-1-v）。范围「越界」值（如 diamond
      above_bottom -80 → -144）按原版原样保留——越界位由
      OreFeature.java:132 isOutsideBuildHeight 兜住，分布形状是本体。
    - targets：OreConfiguration.java:45-57 {target tag_match, state}；
      tag 不抄名单，行内引 mcv_core::tags::TAG_BLOCK_*（#178 框架口径）。
      state Name 经 gen-tags.py 的官方名→id 映射；redstone lit=false 等
      Properties 在本仓单 id 模型下丢弃（同块）。

产物: game/mcv_worldgen/src/vanilla/ores_gen.inc.rs（struct/enum 定义在
      ores_data.rs，本文件只含数据行）

用法（仓库根目录）:  MCV_REF_SRC=<26.1解包树根> python3 ci/gen-ores.py
"""

import importlib.util
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from genlib import drift_guard, file_header, input_root

ROOT = Path(__file__).resolve().parent.parent
OUT_RS = ROOT / "game" / "mcv_worldgen" / "src" / "vanilla" / "ores_gen.inc.rs"

WG = Path("data") / "minecraft" / "worldgen"

# Biome 枚举序（biomes.rs:37-48）↔ 原版群系 json 名。位序 = 枚举判别值。
REPO_BIOMES = [
    ("plains", "Plains"),
    ("forest", "Forest"),
    ("birch_forest", "BirchForest"),
    ("desert", "Desert"),
    ("savanna", "Savanna"),
    ("taiga", "Taiga"),
    ("snowy_plains", "SnowyPlains"),
    ("ocean", "Ocean"),
    ("beach", "Beach"),
    ("river", "River"),
]

# GenerationStep.Decoration（GenerationStep.java:8-18）：下标 6 =
# UNDERGROUND_ORES（RAW_GENERATION/LAKES/LOCAL_MODIFICATIONS/
# UNDERGROUND_STRUCTURES/SURFACE_STRUCTURES/STRONGHOLDS 之后）。
ORE_STEP = 6

# 世界竖直锚（mcv_core::WORLD_MIN_Y=-64、WORLD_MAX_Y=320=genDepth 384；
# VerticalAnchor.java:55/72/89 换算）。
MIN_Y = -64
GEN_DEPTH = 384

DEFAULT_NS = "minecraft"


def load_block_table():
    """复用 gen-tags.py 的 官方名→GEN_BLOCKS 行序 id（同一映射源，零漂移）。"""
    spec = importlib.util.spec_from_file_location(
        "gen_tags_mod", ROOT / "ci" / "gen-tags.py"
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.parse_blocks_table()


def resolve_anchor(obj):
    """VerticalAnchor json → 本仓世界 y（VerticalAnchor.java 三实现）。"""
    (kind, v), = obj.items()
    if kind == "absolute":
        return v
    if kind == "above_bottom":
        return MIN_Y + v
    if kind == "below_top":
        return MIN_Y + GEN_DEPTH - 1 - v
    sys.exit(f"未知 VerticalAnchor: {obj}")


def parse_height(h):
    """HeightProvider json → (kind, lo, hi, plateau)（lo/hi 含端点，世界 y）。"""
    t = h.get("type", "minecraft:uniform")  # CODEC 常数分支已在 HeightProvider.java:14-19 归一：纯锚值 = constant
    if isinstance(h, (int, float)):
        y = resolve_anchor({"absolute": int(h)})
        return ("const", y, y, 0)
    if set(h.keys()) <= {"absolute", "above_bottom", "below_top"}:
        # HeightProvider.java:14-19 常数分支：裸 VerticalAnchor 对象
        y = resolve_anchor(h)
        return ("const", y, y, 0)
    if t == "minecraft:uniform":
        lo = resolve_anchor(h["min_inclusive"])
        hi = resolve_anchor(h["max_inclusive"])
        return ("uniform", lo, hi, 0)
    if t == "minecraft:trapezoid":
        lo = resolve_anchor(h["min_inclusive"])
        hi = resolve_anchor(h["max_inclusive"])
        return ("trapezoid", lo, hi, int(h.get("plateau", 0)))  # TrapezoidHeight.java:20 plateau 默认 0
    if t == "minecraft:constant":
        y = resolve_anchor(h["value"])
        return ("const", y, y, 0)
    sys.exit(f"本刀不支持的 height provider: {t}")


def parse_count(c):
    """CountPlacement 的 IntProvider → (kind, lo, hi)。int = ConstantInt 零随机。"""
    if isinstance(c, int):
        return ("fixed", c, c)
    t = c.get("type")
    if t in (None, "minecraft:uniform"):
        return ("uniform", c["min_inclusive"], c["max_inclusive"])
    if t == "minecraft:constant":
        return ("fixed", c["value"], c["value"])
    sys.exit(f"本刀不支持的 count provider: {t}")


def tag_const_name(tag_raw):
    """'minecraft:base_stone_overworld' → 'TAG_BLOCK_BASE_STONE_OVERWORLD'
    （gen-tags.py const_name 同款命名）。"""
    ns, _, path = tag_raw.partition(":")
    if ns and ns != DEFAULT_NS:
        sys.exit(f"非 minecraft 命名空间 tag: {tag_raw}")
    return "TAG_BLOCK_" + path.upper().replace("/", "__").replace(".", "_")


def main():
    data_root = input_root(str(WG / "configured_feature"))
    official2id = load_block_table()
    biome_dir = data_root / WG / "biome"
    placed_dir = data_root / WG / "placed_feature"
    config_dir = data_root / WG / "configured_feature"

    # 1. 群系 features[6] 表 → 行序（plains 序）+ 位掩码。
    per_biome = {}
    for jname, _variant in REPO_BIOMES:
        doc = json.loads((biome_dir / f"{jname}.json").read_text())
        steps = doc["features"]
        assert len(steps) == 11, f"{jname}: features 数组长度 {len(steps)} != 11（Decoration 枚举序漂移）"
        per_biome[jname] = [f.split(":")[-1] for f in steps[ORE_STEP]]
    order = per_biome["plains"]
    for jname, lst in per_biome.items():
        if lst != order:
            sys.exit(f"群系 {jname} 的 UNDERGROUND_ORES 表与 plains 序不一致：本刀行序模型需扩展（报告）")
    mask_of = {}
    for feat in order:
        m = 0
        for bit, (jname, _v) in enumerate(REPO_BIOMES):
            if feat in per_biome[jname]:
                m |= 1 << bit
        mask_of[feat] = m

    rows, skipped = [], []
    for feat in order:
        pf = json.loads((placed_dir / f"{feat}.json").read_text())
        cfg_name = pf["feature"].split(":")[-1]
        cfg = json.loads((config_dir / f"{cfg_name}.json").read_text())
        if cfg["type"] != "minecraft:ore":
            skipped.append((feat, cfg["type"]))
            continue
        conf = cfg["config"]
        size = conf["size"]
        discard = float(conf.get("discard_chance_on_air_exposure", 0.0))
        targets = []
        for t in conf["targets"]:  # OreConfiguration.java:45-57 顺序即试放序
            name = t["state"]["Name"].split(":")[-1]
            if name not in official2id:
                sys.exit(f"{cfg_name}: 方块 {name} 不在 GEN_BLOCKS（先扩 blocks 表）")
            pred = t["target"]
            pt = pred["predicate_type"]
            if pt != "minecraft:tag_match":
                sys.exit(f"{cfg_name}: 本刀只接 tag_match target（实物 {pt}）")
            targets.append((official2id[name], tag_const_name(pred["tag"])))

        count = ("fixed", 1, 1)
        rarity = 0
        have = []
        for m in pf["placement"]:
            mt = m["type"]
            if mt == "minecraft:count":
                count = parse_count(m["count"])
                have.append("count")
            elif mt == "minecraft:rarity_filter":
                rarity = int(m["chance"])
                have.append("rarity")
            elif mt == "minecraft:in_square":
                have.append("in_square")
            elif mt == "minecraft:height_range":
                height = parse_height(m["height"])
                have.append("height")
            elif mt == "minecraft:biome":
                have.append("biome")
            else:
                sys.exit(f"{feat}: 未接线 placement modifier {mt}（fail-fast，勿静默丢数据）")
        if have not in (
            ["count", "in_square", "height", "biome"],
            ["rarity", "in_square", "height", "biome"],
        ):
            sys.exit(f"{feat}: placement 链实物序异常 {have}（执行器只复刻此两型）")
        rows.append((feat, targets, count, rarity, size, discard, height, mask_of[feat]))

    # 2. 产码（具名 const + ORE_ROWS 表，供测试按名引用）。
    parts = [
        file_header(
            "gen-ores.py",
            [
                "行 = placed feature（configured 展平进 targets/size/discard）；",
                "行序 = biome features[6]（UNDERGROUND_ORES）实物序 = 应用序。",
                "高度已按 VerticalAnchor.java 换算世界 y（absolute/above_bottom/",
                "below_top），越界值原样保留（OreFeature.java:132 兜界）。",
                "biomes 位图 = REPO_BIOMES 位序（biomes.rs Biome 枚举判别值）。",
            ],
        )
    ]
    parts.append("use mcv_core::tags;\n")
    for i, (feat, targets, count, rarity, size, discard, height, mask) in enumerate(rows):
        tid = "\n".join(
            "        OreTarget { state: %d, tag: &tags::%s }," % (sid, tag)
            for sid, tag in targets
        )
        kind, lo, hi, plateau = height
        if kind == "uniform":
            hsrc = f"HeightDist::Uniform {{ lo: {lo}, hi: {hi} }}"
        elif kind == "trapezoid":
            hsrc = f"HeightDist::Trapezoid {{ lo: {lo}, hi: {hi}, plateau: {plateau} }}"
        else:
            hsrc = f"HeightDist::Const({lo})"
        ckind, clo, chi = count
        csrc = (
            f"OreCount::Fixed({clo})"
            if ckind == "fixed"
            else f"OreCount::Uniform {{ lo: {clo}, hi: {chi} }}"
        )
        parts.append(
            f"const ORE_ROW_{i}: OreRow = OreRow {{\n"
            f'    name: "{feat}",\n'
            f"    targets: &[\n{tid}\n    ],\n"
            f"    count: {csrc},\n"
            f"    rarity: {rarity},\n"
            f"    size: {size},\n"
            f"    discard_on_air: {discard}f32,\n"
            f"    height: {hsrc},\n"
            f"    biomes: 0x{mask:03X},\n"
            "};\n"
        )
    parts.append(
        "pub static ORE_ROWS: [&OreRow; {}] = [&{}];\n".format(
            len(rows), ",\n    &".join(f"ORE_ROW_{i}" for i in range(len(rows)))
        )
    )
    OUT_RS.write_text("\n".join(parts))

    import subprocess

    # 刀1 教训（99e32808）：产物必须 rustfmt 稳定——写完立即 rustfmt，
    # 并二次 --check 复核（幂等），不稳即红 exit。
    if subprocess.run(["rustfmt", "--edition", "2024", str(OUT_RS)]).returncode != 0:
        print("WARN: rustfmt 失败/缺失，提交前手动 rustfmt", file=sys.stderr)
    elif subprocess.run(["rustfmt", "--edition", "2024", "--check", str(OUT_RS)]).returncode != 0:
        sys.exit("rustfmt 输出不稳定（二次 check 漂移）")

    counts = {
        "rows": len(rows),
        "skipped_non_ore": len(skipped),
        "total_targets": sum(len(r[1]) for r in rows),
    }
    for probe in (
        "ore_coal_upper",
        "ore_iron_middle",
        "ore_diamond",
        "ore_copper",
        "ore_gold_lower",
        "ore_diamond_large",
    ):
        idx = [f for f, *_x in rows].index(probe)
        _f, _t, cnt, rar, size, disc, hgt, _m = rows[idx]
        counts[f"probe[{probe}]"] = (
            f"count={cnt} rarity={rar} size={size} discard={disc} height={hgt}"
        )
    drift_guard(counts)
    print(f"wrote {OUT_RS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
