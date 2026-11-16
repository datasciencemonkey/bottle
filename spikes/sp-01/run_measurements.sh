#!/usr/bin/env bash
# SP-01: run the whole on-screen case matrix ONCE (small windows, bottom-right
# corner, each window closed when its case ends) plus the offscreen snapshot
# cases. Results go to $OUT (default out/run-<UTC time>).
#
# Usage: spikes/sp-01/run_measurements.sh <path-to-.app> <path-to-.riv>
# Needs: built .app (pnpm tauri build --bundles app), out/probe and
#        out/post_moves (swiftc, see probe-swift/), uv.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
APP_BUNDLE="${1:?path to .app}"
RIV="${2:?path to .riv}"
BIN="$APP_BUNDLE/Contents/MacOS/$(defaults read "$APP_BUNDLE/Contents/Info" CFBundleExecutable)"
OUT="${OUT:-$HERE/out/run-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
M() { uv run --no-project --with pillow python -I "$HERE/measure/sp01_measure.py" "$@"; }
front() { lsappinfo info -only name "$(lsappinfo front)" | sed 's/.*="\(.*\)"/\1/'; }
webkit_pids() { pgrep -f 'com.apple.WebKit' | sort -n || true; }

# Geometry: 240 px window, 20 px in from the bottom-right of the main display.
SIZE=240
read -r SW SH < <(osascript -l JavaScript -e 'ObjC.import("AppKit"); var f=$.NSScreen.mainScreen.frame; f.size.width+" "+f.size.height' 2>/dev/null || echo "1728 1117")
SW=${SW%.*}; SH=${SH%.*}
TX=$((SW - SIZE - 20)); TY=$((SH - SIZE - 20))   # top-left origin (Tauri, CGEvent)
BX=$TX; BY=20                                     # bottom-left origin (AppKit probe)
echo "{\"screen\":[$SW,$SH],\"rect_cg\":[$TX,$TY,$SIZE,$SIZE]}" > "$OUT/geometry.json"

sample() { # $1 label, $2 pid of app; 31 samples of 1 s over app + its WebKit pids
  local args=(-pid "$2")
  for p in $NEWWK; do args+=(-pid "$p"); done
  top -l 31 -s 1 -stats pid,cpu,mem "${args[@]}" > "$OUT/top-$1.txt"
  M top "$OUT/top-$1.txt" > "$OUT/cpu-$1.json"
}

hover() { # $1 label; 3 bursts of 200 moves at 10 ms
  for i in 1 2 3; do
    "$HERE/out/post_moves" "$TX" "$TY" "$SIZE" "$SIZE" 200 10 hid > "$OUT/sent-$1-$i.json"
    sleep 0.7
  done
}

hover_count() { # $1 label, $2 log
  local sent
  sent=$(uv run --no-project python -I -c "import json,glob,sys; print(sum(json.load(open(f))['sent'] for f in glob.glob(sys.argv[1])))" "$OUT/sent-$1-*.json")
  M hover "$2" --sent "$sent" > "$OUT/hover-$1.json"
}

# ---- Tauri cases (primary evidence) --------------------------------------
for RT in webgl2 canvas; do
  for MODE in animating idle; do
    L="tauri-$RT-$MODE"; LOG="$OUT/$L.jsonl"
    BEFORE_WK=$(webkit_pids); FRONT0=$(front)
    SP01_LOG="$LOG" SP01_RIV="$RIV" SP01_RUNTIME=$RT SP01_MODE=$MODE \
      SP01_SIZE=$SIZE SP01_X=$TX SP01_Y=$TY "$BIN" > "$OUT/$L.stdout" 2>&1 &
    PID=$!
    sleep 1; FRONT1=$(front)
    sleep 1.5
    NEWWK=$(comm -13 <(echo "$BEFORE_WK") <(webkit_pids) | tr '\n' ' ')
    echo "{\"front_before\":\"$FRONT0\",\"front_1s\":\"$FRONT1\",\"webkit_pids\":\"$NEWWK\"}" > "$OUT/front-$L.json"
    if [ "$MODE" = animating ]; then hover "$L"; sleep 2; fi
    sample "$L" "$PID"
    FRONT2=$(front)
    echo "{\"front_end\":\"$FRONT2\"}" >> "$OUT/front-$L.json"
    kill "$PID"; wait "$PID" 2>/dev/null || true
    [ "$MODE" = animating ] && hover_count "$L" "$LOG"
    sleep 1
  done
done

# ---- WKWebView + NSPanel probe (not Tauri): hover variants ----------------
for TRACK in 0 2; do
  L="probe-panel-track$TRACK"; LOG="$OUT/$L.jsonl"
  SP01_WEB="$HERE/web" SP01_LOG="$LOG" SP01_RIV="$RIV" SP01_KIND=panel SP01_TRACK=$TRACK \
    SP01_SIZE=$SIZE SP01_X=$BX SP01_Y=$BY "$HERE/out/probe" > /dev/null 2>&1 &
  PID=$!
  sleep 2.5; hover "$L"; sleep 0.5
  kill "$PID"; wait "$PID" 2>/dev/null || true
  hover_count "$L" "$LOG"
  sleep 1
done

# ---- Offscreen probe snapshots: alpha, Delta E, halo, early frames --------
M checker "$OUT/checker.png" --w 480 --h 480 --cell 16
for RT in webgl2 canvas; do
  D="$OUT/snap-$RT"; mkdir -p "$D/flash"
  SP01_WEB="$HERE/web" SP01_LOG="$OUT/snap-$RT.jsonl" SP01_RIV="$RIV" SP01_RUNTIME=$RT SP01_SIZE=$SIZE \
    SP01_X=-3000 SP01_Y=-3000 SP01_SNAP_DIR="$D" SP01_SNAP_AT=3 SP01_FLASH_DIR="$D/flash" \
    SP01_QUIT_AFTER=3.5 "$HERE/out/probe" > /dev/null 2>&1 || true
  M composite "$D/snap_3000.png" "$OUT/checker.png" "$D/comp.png" > "$D/alpha.json"
  M transparency "$OUT/checker.png" "$D/comp.png" > "$D/transparency.json"
  # Early frames: composite each over black, then look for near-white frames.
  mkdir -p "$D/flash-comp"
  uv run --no-project --with pillow python -I -c "
import sys,glob,os
from PIL import Image
for f in glob.glob(sys.argv[1]+'/*.png'):
    im=Image.open(f).convert('RGBA'); bg=Image.new('RGBA',im.size,(0,0,0,255))
    Image.alpha_composite(bg,im).convert('RGB').save(os.path.join(sys.argv[2],os.path.basename(f)))
" "$D/flash" "$D/flash-comp"
  M flash "$D/flash-comp" > "$D/flash.json"
done

# ---- Screen capture (needs Screen Recording for the terminal) -------------
# With the permission: desktop-only reference shot, then rapid shots from launch
# (white flash), then a settled shot (Delta E of transparent pixels, halo).
if screencapture -x -R"$TX,$TY,$SIZE,$SIZE" "$OUT/screencapture-test.png" 2> "$OUT/screencapture.err"; then
  echo '{"screencapture":"ok"}' > "$OUT/screencapture.json"
  for RT in webgl2 canvas; do
    D="$OUT/screen-$RT"; mkdir -p "$D/flash"
    screencapture -x -R"$TX,$TY,$SIZE,$SIZE" "$D/ref.png"
    SP01_LOG="$D/log.jsonl" SP01_RIV="$RIV" SP01_RUNTIME=$RT SP01_SIZE=$SIZE SP01_X=$TX SP01_Y=$TY \
      "$BIN" > /dev/null 2>&1 &
    PID=$!; T0=$(uv run --no-project python -I -c "import time;print(time.time())")
    for i in $(seq 1 40); do
      T=$(uv run --no-project python -I -c "import time,sys;print(int((time.time()-float(sys.argv[1]))*1000))" "$T0")
      screencapture -x -R"$TX,$TY,$SIZE,$SIZE" "$D/flash/f_$T.png"
    done
    sleep 2; screencapture -x -R"$TX,$TY,$SIZE,$SIZE" "$D/test.png"
    kill "$PID"; wait "$PID" 2>/dev/null || true
    M transparency "$D/ref.png" "$D/test.png" > "$D/transparency.json"
    M flash "$D/flash" > "$D/flash.json"
    sleep 1
  done
else
  echo "{\"screencapture\":\"failed\",\"stderr\":\"$(tr -d '\n' < "$OUT/screencapture.err")\"}" > "$OUT/screencapture.json"
fi

echo "results in $OUT"
for f in "$OUT"/front-*.json "$OUT"/hover-*.json "$OUT"/cpu-*.json "$OUT"/snap-*/alpha.json \
         "$OUT"/snap-*/transparency.json "$OUT"/snap-*/flash.json "$OUT"/screen-*/*.json "$OUT"/screencapture.json; do
  [ -f "$f" ] || continue
  echo "== ${f#$OUT/}"; tr -d '\n' < "$f"; echo
done
