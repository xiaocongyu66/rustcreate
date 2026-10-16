#!/bin/bash
# JGH1 dump worker: polls jobs/*.job, runs java dumper, marks done.
# job line: seed cx cz ymin ymax outname
set -u
cd /root/java-harness
CP="$(printf '%s:' libs/server-26.1.jar; find libs -name '*.jar' ! -name 'server-26.1.jar' | sort | tr '\n' ':')"
CP="${CP%:}"
OUT=/root/noise-fork/tools/fork-reanchor-dumps
JOBQ=/root/noise-fork/tools/fork-reanchor/jobs
DONE=/root/noise-fork/tools/fork-reanchor/done
mkdir -p "$OUT"
while true; do
  job=$(ls "$JOBQ"/*.job 2>/dev/null | head -1)
  if [ -z "$job" ]; then
    sleep 5
    continue
  fi
  read -r seed cx cz ymin ymax out < "$job"
  mv "$job" "$DONE/$(basename "$job").working"
  bin="$OUT/$out.bin"
  if [ -s "$bin" ]; then
    echo "SKIP $out (exists)" >> /root/noise-fork/tools/fork-reanchor/logs/worker.log
    mv "$DONE/$(basename "$job").working" "$DONE/$(basename "$job").done"
    continue
  fi
  if java -Xmx512m -cp "build:$CP" Main --seed "$seed" --chunk "$cx,$cz" --y-min "$ymin" --y-max "$ymax" -o "$bin" >> /root/noise-fork/tools/fork-reanchor/logs/worker.log 2>&1; then
    mv "$DONE/$(basename "$job").working" "$DONE/$(basename "$job").done"
    echo "OK $out" >> /root/noise-fork/tools/fork-reanchor/logs/worker.log
  else
    echo "FAIL $out" >> /root/noise-fork/tools/fork-reanchor/logs/worker.log
    mv "$DONE/$(basename "$job").working" "$DONE/$(basename "$job").failed"
  fi
done
