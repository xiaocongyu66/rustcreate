#!/usr/bin/env python3
"""Java→Rust 机制移植账本生成器。

方法（防演绎作品定性，硬约束）：**只移植机制，不翻译表达**——从反编译
Java 提取常数/公式/执行顺序/边界条件形成机制规格（笔记留在本地
/root/mc-ref/NOTES-*.md，永不入库），再按我们框架自身的模块划分与
Rust 习惯法独立实现；验收=行为对拍测试，不是文件结构对应。产物表达
层零转录：不照抄类划分/方法分解/代码文本/注释。

账本：扫描 /root/mc-ref/src-26.1/net/minecraft 全部 .java，按域分桶、
标状态，产出 ci/port-manifest.csv。**清单只含文件名与状态，不含任何
反编译内容**（版权红线：仓库不接触 Mojang 代码本体）。

桶语义：
  logic      —— 机制移植目标（world/core/data/nbt/util 等逻辑层）
  adapt      —— client/server/network 层：机制由现有框架
                （mcv_render/mcv_app/mcv_save）承担等价职责，登记豁免
  exempt     —— 依赖 JVM 生态（netty/joml/fastutil/awt）不移植

状态：pending / ported(机制已独立实现+行为测试) / adapted(框架等价) /
exempt / blocked(缺上游)。已有状态从旧 CSV 继承，其余 pending。
"""
import csv
import os
import sys

SRC = os.environ.get("MCV_REF_SRC", "/root/mc-ref/src-26.1")
ROOT = os.path.join(SRC, "net", "minecraft")
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "port-manifest.csv")

ADAPT_PREFIX = {"client", "server", "network", "stats", "solver"}


def bucket_of(rel: str) -> str:
    top = rel.split("/", 1)[0]
    if top in ADAPT_PREFIX:
        return "adapt"
    if top in ("resources", "Tags", "advancements"):
        return "exempt"
    return "logic"


def load_old() -> dict:
    out = {}
    if os.path.exists(OUT):
        with open(OUT, newline="", encoding="utf-8") as f:
            for row in csv.DictReader(f):
                out[row["java_path"]] = (row["status"], row["rust_file"])
    return out


def main() -> None:
    if not os.path.isdir(ROOT):
        sys.exit(f"缺反编译源码树: {ROOT}（设 MCV_REF_SRC）")
    old = load_old()
    rows = []
    for dirpath, _, files in os.walk(ROOT):
        for fn in sorted(files):
            if not fn.endswith(".java"):
                continue
            rel = os.path.relpath(os.path.join(dirpath, fn), ROOT).replace(os.sep, "/")
            b = bucket_of(rel)
            status, rust = old.get(rel, ("pending", ""))
            if b == "adapt" and status == "pending":
                status = "exempt"
            rows.append({"java_path": rel, "bucket": b, "status": status, "rust_file": rust})
    rows.sort(key=lambda r: (r["bucket"], r["java_path"]))
    with open(OUT, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=["java_path", "bucket", "status", "rust_file"])
        w.writeheader()
        w.writerows(rows)
    n = {"logic": 0, "adapt": 0, "exempt": 0}
    done = 0
    for r in rows:
        n[r["bucket"]] = n.get(r["bucket"], 0) + 1
        if r["status"] in ("ported", "adapted"):
            done += 1
    print(f"total={len(rows)} logic={n['logic']} adapt={n['adapt']} done={done} -> {OUT}")


if __name__ == "__main__":
    main()
