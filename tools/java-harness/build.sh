#!/bin/sh
# java-gen-harness build: compile the copied 26.1 worldgen sources into build/
# classpath = official 26.1 server jar (libs/server-26.1.jar) + 107 exact-26.1
# dependency jars downloaded from piston-data (sha1-verified against
# https://piston-meta.mojang.com/v1/packages/8667c5278895a6ae31e05ab99ba044ef33fca092/26.1.json).
# NOTE: libs/* nested jars are NOT visible to a plain "libs/*" wildcard
# (java launcher globs one level only) -> build the list explicitly.
set -e
cd "$(dirname "$0")"
# bundler 结构：外层 server jar 的主类在内嵌 version jar（META-INF/versions）
VJAR="libs/bundle/META-INF/versions/26.1/server-26.1.jar"
CP="$(printf '%s:' "$VJAR" libs/server-26.1.jar; find libs -name '*.jar' ! -name 'server-26.1.jar' | sort | tr '\n' ':')"
CP="${CP%:}"
mkdir -p build
echo "classpath entries: $(printf '%s' "$CP" | tr ':' '\n' | wc -l)"
# shellcheck disable=SC2086
javac -nowarn -Xmaxerrs 600 -encoding UTF-8 -d build -cp "$CP" $(find src -name '*.java' ! -path 'src/main/HarnessFill.java')
echo "BUILD OK: $(find build -name '*.class' | wc -l) classes"
