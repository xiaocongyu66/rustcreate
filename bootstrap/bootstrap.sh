#!/usr/bin/env bash
# java-gen-harness bootstrap——从本地参考树生成工作区（Mojang 反编译源
# **绝不入仓**：本脚本按 copy_manifest.txt 把 26.1 levelgen 子树拷贝到
# src/net/**，随后应用 decompile-fixes.patch（入库的第一手原创修型）并
# 覆盖自研 overlay（HarnessFill 桥）。前置：本地已解包 26.1 反编译源
# （本项目固定路径 /root/mc-ref/src-26.1，可用 MC_REF 覆盖）。
set -euo pipefail
cd "$(dirname "$0")/.."
MC_REF="${MC_REF:-/root/mc-ref/src-26.1}"
[[ -d "$MC_REF/net/minecraft/world/level/levelgen" ]] || { echo "缺参考树: $MC_REF"; exit 1; }
n=0
while IFS= read -r rel; do
    src="$MC_REF/$rel"; dst="src/$rel"
    [[ -f "$src" ]] || { echo "缺文件: $rel"; exit 1; }
    mkdir -p "$(dirname "$dst")"
    cp "$src" "$dst"
    n=$((n+1))
done < bootstrap/copy_manifest.txt
echo "copied $n files"
# 自研 overlay（包内 protected 桥，非 vanilla 件）
cp src/main/HarnessFill.java src/net/minecraft/world/level/levelgen/HarnessFill.java
git apply --check bootstrap/harness-patches/decompile-fixes.patch
git apply bootstrap/harness-patches/decompile-fixes.patch
echo "patched 10 files (HARNESS-PATCH decompile artifacts)"
