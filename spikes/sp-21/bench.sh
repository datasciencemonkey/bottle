#!/bin/sh
# SP-21 (#34) measurement run. Usage: ./bench.sh [outdir]   (default /tmp/sp21-bench)
# Needs a release build: cargo build --release
set -eu
B="$(cd "$(dirname "$0")" && pwd)/target/release/bench"
OUT="${1:-/tmp/sp21-bench}"
mkdir -p "$OUT"
T="/usr/bin/time -l"
rss() { grep -E "maximum resident set size" | awk '{printf "  peak_rss=%.1f MB\n", $1/1048576}'; }

echo "== build (50,000 x 256, batches of 1,000 rows per transaction)"
for spec in "f32 l2 pk" "i8 l2 pk" "f32 cosine pk" "f32 cosine metadata"; do
  set -- $spec
  db="$OUT/$1-$2-$3.db"
  SP21_METRIC=$2 SP21_MODEL_COL=$([ "$3" = pk ] && echo partition || echo metadata) \
    $T "$B" build "$db" 50000 "$1" 2>"$OUT/time.txt"
  echo "  ($1 elem, $2 metric, embedding_model as $3)"; rss <"$OUT/time.txt"
done

echo "== cold: first query in a fresh process (5 processes each, OS file cache warm)"
for db in f32-l2-pk i8-l2-pk; do
  for i in 1 2 3 4 5; do "$B" query "$OUT/$db.db" 10 0 $((100 + i)) | sed "s/^/  [$db #$i] /"; done
done

echo "== warm: 1,000 random queries after the first"
for db in f32-l2-pk i8-l2-pk f32-cosine-pk f32-cosine-metadata; do
  for k in 10 50; do
    $T "$B" query "$OUT/$db.db" $k 1000 2>"$OUT/time.txt" | sed "s/^/  [$db] /"; rss <"$OUT/time.txt"
  done
done

echo "== warm with SQLite's default 2,000 KiB page cache"
for db in f32-l2-pk i8-l2-pk; do
  SP21_CACHE_KIB=2000 $T "$B" query "$OUT/$db.db" 10 1000 2>"$OUT/time.txt" | sed "s/^/  [$db cache=2000KiB] /"; rss <"$OUT/time.txt"
done

echo "== fallback: brute-force cosine over the f16 cache"
for k in 10 50; do
  $T "$B" brute "$OUT/f32-l2-pk.db" $k 1000 2>"$OUT/time.txt" | sed "s/^/  /"; rss <"$OUT/time.txt"
done

echo "== recall vs exact f32"
for k in 10 50; do "$B" recall "$OUT/f32-l2-pk.db" "$OUT/i8-l2-pk.db" $k 1000 | sed "s/^/  /"; done
