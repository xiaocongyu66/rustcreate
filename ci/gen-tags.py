#!/usr/bin/env python3
"""从原版 26.1 数据 JSON 生成 tag 静态表 tags_gen.inc.rs（#178 刀1）。

输入（只读本机参考树，绝不入仓；寻径见 genlib.input_root）：
    data/minecraft/tags/block/**/*.json   （约 248 个文件）
    game/mcv_core/src/blocks_gen.inc.rs   （GEN_BLOCKS 行序 = 方块 id）
    ci/gen-blocks.py                      （LEGACY 旧名→官方名映射）

构建期语义（照抄原版，运行期零解引用）：
    - TagFile.java:9        values + replace（默认 false，replace 先 clear）
    - TagEntry.java:17      required 默认 true
    - TagLoader.java:89     依赖拓扑排序后逐 tag 解引用（DependencySorter
                            遇环抛异常 → 本脚本检测环即报错退出）
    - TagLoader.java:96     `#` 引用解引用；required 缺失 → 报错退出
                            （原版记 error 弃整个 tag，本脚本从严）；
                            optional 缺失 → 静默跳过
    - 元素名→id：官方名经 LEGACY 别名（grass_block→grass 等）映射到
      GEN_BLOCKS 行序 id；GEN_BLOCKS 外且 required → 报错。

产物: game/mcv_core/src/tags_gen.inc.rs

用法（仓库根目录）:  python3 ci/gen-tags.py

TODO(#178 后续刀): item tags（data/minecraft/tags/item/**，registry="item"）
——item 注册表 id 域尚未入库，本刀只做 block tags（消费端全是方块）。
"""

import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from genlib import drift_guard, file_header, input_root

ROOT = Path(__file__).resolve().parent.parent
TAGS_REL = Path("data") / "minecraft" / "tags" / "block"
OUT_RS = ROOT / "game" / "mcv_core" / "src" / "tags_gen.inc.rs"

DEFAULT_NS = "minecraft"  # Identifier.java:59 DEFAULT_NAMESPACE


def parse_blocks_table():
    """GEN_BLOCKS 行序 id → 引擎名，以及 官方名 → id。"""
    src = (ROOT / "game" / "mcv_core" / "src" / "blocks_gen.inc.rs").read_text()
    engine = {}
    for m in re.finditer(r'/\*\s*(\d+)\s*\*/\s*\("([a-z0-9_]+)"', src):
        engine[int(m.group(1))] = m.group(2)
    # LEGACY 元组前两元素 (引擎旧名, 官方名)：从 gen-blocks.py 源码内
    # 提取 LEGACY = [...] 列表（不 import：该脚本顶层有 PIL 依赖与 exit 检查）。
    gen_src = (ROOT / "ci" / "gen-blocks.py").read_text()
    start = gen_src.index("LEGACY = [")
    depth, i = 0, start + len("LEGACY ")
    for j in range(i, len(gen_src)):
        if gen_src[j] == "[":
            depth += 1
        elif gen_src[j] == "]":
            depth -= 1
            if depth == 0:
                i = j
                break
    legacy = re.findall(r'\(\s*"([a-z0-9_]+)",\s*"([a-z0-9_]+)",', gen_src[start:i])
    engine2id = {v: k for k, v in engine.items()}
    official2id = {}
    for old, official in legacy:  # 先 LEGACY（grass 先于 snow_grass，先到优先）
        if old in engine2id:
            official2id.setdefault(official, engine2id[old])
    for bid, name in engine.items():  # 14+ 已是官方名
        official2id.setdefault(name, bid)
    return official2id


def split_id(raw):
    """Identifier.java:59 bySeparator：首个 ':' 分割，前空/无冒号 → minecraft。"""
    ns, sep, path = raw.partition(":")
    return (DEFAULT_NS, path) if (not sep or not ns) else (ns, path)


def parse_tag_file(path):
    """→ (entries, replace)。entries: (kind, raw_id, required)。"""
    doc = json.loads(path.read_text())
    replace = bool(doc.get("replace", False))  # TagFile.java:9 默认 false
    entries = []
    for v in doc["values"]:
        if isinstance(v, str):
            raw, required = v, True  # TagEntry.java:17 required 默认 true
        else:
            raw, required = v["id"], bool(v.get("required", True))
        kind = "#" if raw.startswith("#") else ""
        entries.append((kind, raw[1:] if kind else raw, required))
    return entries, replace


def main():
    data_root = input_root(str(TAGS_REL))
    tags_root = data_root / TAGS_REL
    files = sorted(tags_root.rglob("*.json"))
    official2id, errors = parse_blocks_table(), []

    # 读入并按包合并（TagLoader.java:61：本树单包，replace 语义仍照做）。
    builders = {}  # tag_path -> entries
    for f in files:
        tag_path = f.relative_to(tags_root).with_suffix("").as_posix()
        entries, replace = parse_tag_file(f)
        if replace:
            builders[tag_path] = []
        builders.setdefault(tag_path, []).extend(entries)

    # 拓扑解引用（TagLoader.java:89 语义），检测环即报错。
    resolved, state = {}, {}  # state: 1=visiting 2=done

    def resolve(tag_path):
        if state.get(tag_path) == 2:
            return resolved[tag_path]
        if state.get(tag_path) == 1:
            sys.exit(f"tag 引用环: {tag_path}")
        state[tag_path] = 1
        ids = set()
        for kind, raw, required in builders[tag_path]:
            ns, local = split_id(raw)
            if ns != DEFAULT_NS:  # 三方命名空间不在本表 → 按缺失处理
                if required:
                    errors.append(f"{tag_path}: 缺命名空间支持 {kind}{raw}")
                continue
            if kind == "#":
                if local not in builders:
                    if required:
                        errors.append(f"{tag_path}: 缺 tag 引用 #{local}")
                    continue
                ids.update(resolve(local))
            else:
                if local not in official2id:
                    if required:
                        errors.append(f"{tag_path}: 缺方块 {raw}")
                    continue  # optional 缺失：静默跳过
                ids.add(official2id[local])
        state[tag_path] = 2
        resolved[tag_path] = sorted(ids)
        return resolved[tag_path]

    for tag_path in sorted(builders):
        resolve(tag_path)
    if errors:
        print("\n".join("ERROR " + e for e in errors), file=sys.stderr)
        sys.exit(f"gen-tags: {len(errors)} 处 required 引用缺失/非法")

    def const_name(tag_path):
        return "TAG_BLOCK_" + tag_path.upper().replace("/", "__").replace(".", "_")

    out = [
        file_header(
            "gen-tags.py",
            [
                "id 域：GEN_BLOCKS 行序（方块 id）；数组排序去重，`#` 引用已",
                "在构建期解出——运行期零解引用（TagLoader.java:96 语义前置）。",
                "registry 名 \"block\"；item tags 见 ci/gen-tags.py TODO(#178)。",
                "消费端契约：mcv_registry::TagSet 二分查找要求升序（生成器保证）。",
            ],
        )
    ]
    out.append("use mcv_registry::{TagEntry, TagKey, TagSet};\n")
    refs = []
    for tag_path in sorted(resolved):
        cname = const_name(tag_path)
        ids = resolved[tag_path]
        if ids:
            body = "\n".join(
                "        " + ", ".join(str(i) for i in ids[k : k + 14]) + ","
                for k in range(0, len(ids), 14)
            )
            array = "    &[\n" + body + "\n    ]"
        else:
            array = "    &[]"
        out.append(
            f"pub const {cname}: TagEntry = TagEntry {{\n"
            f'    key: TagKey::new("block", "{tag_path}"),\n'
            f"    set: TagSet::new(\n{array},\n    ),\n}};\n"
        )
        refs.append(cname)
    out.append(
        "pub static ALL_TAGS: [&TagEntry; {}] = [&{}];\n".format(
            len(refs), ",\n    &".join(refs)
        )
    )
    OUT_RS.write_text("\n".join(out))

    counts = {
        "tag_files": len(files),
        "tags": len(resolved),
        "total_members": sum(len(v) for v in resolved.values()),
        "empty_tags": sum(1 for v in resolved.values() if not v),
    }
    for probe in (
        "base_stone_overworld",
        "replaceable",
        "features_cannot_replace",
        "lava_pool_stone_cannot_replace",
    ):
        counts[f"len[{probe}]"] = len(resolved.get(probe, []))
    drift_guard(counts)
    print(f"wrote {OUT_RS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
