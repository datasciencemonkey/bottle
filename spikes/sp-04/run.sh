#!/usr/bin/env bash
# SP-04 (#44) rerun on reference hardware (M1 or M2 Mac). See docs/spikes/SP-04.md.
#
#   bash spikes/sp-04/run.sh            # from the repository root
#   SP04_PURGE=1 bash spikes/sp-04/run.sh   # also measure a cold-file-cache load (asks for sudo)
#
# Needs: Rust stable (1.88 or later), curl, shasum, say, /usr/bin/time. The files
# go into $SP04_WORK (default ~/sp04-work): 0.51 GB of downloads. Nothing from $SP04_WORK is committed.
# Exit code: 0 = every SP-04 criterion passes on this machine (10 s p95, tail p95, peak RSS, the
# in-process load and every fresh-process load in load.txt, cold load included), 1 = one or more
# fail, 2 = error (a download, build, test, load run or clip length failed). Any failed setup step
# (curl, shasum, tar, cargo, sudo purge) exits 2 through the ERR trap; tool codes are never passed through.
set -Eeuo pipefail
trap 'echo "error: a setup step failed (exit $?), overall exit=2"; exit 2' ERR
# norm: 0 and 1 keep their meaning; any other code (crash, signal, tool error) becomes 2.
norm() { case "$1" in 0|1) echo "$1" ;; *) echo 2 ;; esac; }

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="${SP04_WORK:-$HOME/sp04-work}"
RUNS="${SP04_RUNS:-20}"
VERSION="2.0.1"
LIBS_SHA="d7a9b14f337958e7373c8c650f046df33d210bbaff2f33cb8322dda663ee5e1c"
LIBS_NAME="sherpa-onnx-v${VERSION}-osx-arm64-static-lib.tar.bz2"
LIBS_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/v${VERSION}/${LIBS_NAME}"
MODEL_SHA="1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9"
MODEL_NAME="sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8"
MODEL_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/${MODEL_NAME}.tar.bz2"

[ "$(uname -s)" = "Darwin" ] && [ "$(uname -m)" = "arm64" ] || { echo "run.sh is for Apple silicon Macs; use run.ps1 on Windows"; exit 2; }
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p "$WORK/libs" "$WORK/model" "$WORK/audio" "$WORK/out"

fetch() { # url dest sha256
  if [ ! -f "$2" ] || [ "$(shasum -a 256 "$2" | cut -d' ' -f1)" != "$3" ]; then
    echo "download $1"
    curl -fL --retry 3 -C - -o "$2" "$1"
  fi
  local got; got="$(shasum -a 256 "$2" | cut -d' ' -f1)"
  [ "$got" = "$3" ] || { echo "SHA-256 mismatch for $2: got $got, want $3"; exit 2; }
  echo "sha256 OK $got $(basename "$2")"
}

echo "== 1. machine"
sysctl -n machdep.cpu.brand_string
P=$(sysctl -n hw.perflevel0.physicalcpu); E=$(sysctl -n hw.perflevel1.physicalcpu 2>/dev/null || echo 0)
ALL=$(sysctl -n hw.physicalcpu)
echo "P-cores=$P E-cores=$E physical=$ALL mem=$(( $(sysctl -n hw.memsize) / 1073741824 )) GB"
sw_vers | tr '\n' ' '; echo; uptime

echo "== 2. downloads (pinned SHA-256)"
fetch "$LIBS_URL" "$WORK/libs/$LIBS_NAME" "$LIBS_SHA"
fetch "$MODEL_URL" "$WORK/model/${MODEL_NAME}.tar.bz2" "$MODEL_SHA"
M="$WORK/model-x/$MODEL_NAME"
if [ ! -f "$M/encoder.int8.onnx" ]; then
  mkdir -p "$WORK/model-x"
  tar -xjf "$WORK/model/${MODEL_NAME}.tar.bz2" -C "$WORK/model-x"
fi

echo "== 3. test audio (say, 16 kHz 16-bit mono)"
A="$WORK/audio"
printf '%s\n' "Open the sales dashboard for me now." > "$A/ref_2.txt"
printf '%s\n' "Please show me the total revenue for the last quarter by region and by product." > "$A/ref_5.txt"
printf '%s\n' "Show me the total revenue for the last quarter by region and compare it with the same quarter of the previous year for every product line, then sort the result by growth and highlight the top five accounts." > "$A/ref_10.txt"
printf '%s\n' "Good morning team. Today we will review the quarterly sales report for the western region. Revenue grew by twelve percent compared with last year, mostly because of the new enterprise contracts. However, the cost of support also went up, so the margin stayed flat. Please look at the churn numbers before our next meeting, and send me any questions about the forecast by Friday afternoon. We also need to decide on the hiring plan for the next two quarters, and the budget for the new data platform." > "$A/ref_30.txt"
for pair in 2:200 5:120 10:185 30:155; do
  n=${pair%%:*}; r=${pair##*:}
  say -v Samantha -r "$r" -o "$A/clip_${n}s.wav" --data-format=LEI16@16000 -f "$A/ref_$n.txt"
done

echo "== 4. build (vendored, hash-checked sherpa-onnx archive)"
( cd "$HERE" && SHERPA_ONNX_ARCHIVE_DIR="$WORK/libs" cargo build --release --locked )
BIN="$HERE/target/release/sp04-bench"
( cd "$HERE" && SHERPA_ONNX_ARCHIVE_DIR="$WORK/libs" SP04_MODEL_DIR="$M" SP04_CLIP="$A/clip_10s.wav" SP04_CLIP_REF="$A/ref_10.txt" cargo test --release --locked )

echo "== 5. model load, fresh process each time"
cd "$WORK/out"
if [ "${SP04_PURGE:-0}" = "1" ]; then
  if ! sudo purge; then echo "sudo purge failed: the cold load was not measured, overall exit=2"; exit 2; fi
  echo "file cache purged (cold load next)"
fi
: > load.txt
LOAD_RC=0
trap - ERR; set +e
for i in 1 2 3 4 5; do
  # Each run is a fresh process. Exit 1 = load over 3 s, 2 = load error.
  "$BIN" load --model-dir "$M" --threads "$P" | tee -a load.txt
  rc=${PIPESTATUS[0]}; rc=$(norm "$rc")
  if [ "$rc" -ne 0 ]; then echo "load run $i failed the criterion or errored (exit $rc)"; [ "$rc" -gt "$LOAD_RC" ] && LOAD_RC=$rc; fi
done
[ "$(grep -c '^load_ms=' load.txt)" -eq 5 ] || { echo "a load run printed no load_ms"; LOAD_RC=2; }
set -Ee; trap 'echo "error: a setup step failed (exit $?), overall exit=2"; exit 2' ERR

echo "== 6. bench, threads = P-cores ($P)"
trap - ERR; set +e
/usr/bin/time -l "$BIN" bench --model-dir "$M" --audio-dir "$A" --threads "$P" --runs "$RUNS" 2> time-p.txt | tee bench-p.txt
RC=${PIPESTATUS[0]}; RC=$(norm "$RC")
grep -E "maximum resident set size|real" time-p.txt
echo "== 7. bench, threads = all physical cores ($ALL), for the thread sensitivity"
/usr/bin/time -l "$BIN" bench --model-dir "$M" --audio-dir "$A" --threads "$ALL" --runs "$RUNS" --long-secs 0 2> time-all.txt | tee bench-all.txt
grep -E "maximum resident set size|real" time-all.txt
set -Ee; trap 'echo "error: a setup step failed (exit $?), overall exit=2"; exit 2' ERR
uptime
echo "outputs: $WORK/out (paste load.txt, bench-p.txt, bench-all.txt and time-*.txt into the issue)"
echo "pass rule: the 'SP04_RESULT' line of bench-p.txt is PASS and every load.txt load_ms <= 3000"
FINAL=$RC
[ "$LOAD_RC" -gt "$FINAL" ] && FINAL=$LOAD_RC
echo "bench exit=$RC load exit=$LOAD_RC overall exit=$FINAL (0 pass, 1 fail, 2 error)"
exit "$FINAL"
