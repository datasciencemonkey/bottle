#!/usr/bin/env bash
# SP-01 activation matrix, OFFSCREEN (window at x=-3000, no visible window).
# For each config: launch twice, sample the frontmost app at 0.3 s and 6 s,
# and read the app's own is_key / NSApp.isActive samples (0.5 s, 2 s, 5 s).
# Usage: spikes/sp-01/run_activation_offscreen.sh <path-to-.app> <path-to-.riv> [lsuielement-.app]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="${1:?path to .app}"; RIV="${2:?path to .riv}"; LSUI="${3:-}"
OUT="${OUT:-$HERE/out/activation-$(date -u +%Y%m%dT%H%M%SZ)}"; mkdir -p "$OUT"
front() { lsappinfo info -only name "$(lsappinfo front)" | sed 's/.*="\(.*\)"/\1/'; }
exe() { echo "$1/Contents/MacOS/$(defaults read "$1/Contents/Info" CFBundleExecutable)"; }

run_case() { # $1 label, $2 app, $3 launcher (exec|open), rest: env
  local label=$1 app=$2 how=$3; shift 3
  for i in 1 2; do
    local log="$OUT/$label-$i.jsonl"; rm -f "$log"
    local f0; f0=$(front)
    if [ "$how" = open ]; then
      local envargs=()
      for kv in SP01_LOG="$log" SP01_RIV="$RIV" SP01_X=-3000 SP01_Y=-3000 "$@"; do envargs+=(--env "$kv"); done
      open -g -n "${envargs[@]}" "$app"
    else
      env SP01_LOG="$log" SP01_RIV="$RIV" SP01_X=-3000 SP01_Y=-3000 "$@" "$(exe "$app")" > /dev/null 2>&1 &
    fi
    sleep 0.3; local f1; f1=$(front)
    sleep 5.7; local f2; f2=$(front)
    pkill -f "$(exe "$app")" || true; sleep 1
    local act key
    act=$(grep '"focus"' "$log" 2>/dev/null | grep -o '"app_active":[a-z]*' | cut -d: -f2 | tr '\n' ',' || true)
    key=$(grep '"focus"' "$log" 2>/dev/null | grep -o '"is_key":[a-z]*' | cut -d: -f2 | tr '\n' ',' || true)
    echo "{\"case\":\"$label\",\"run\":$i,\"front_before\":\"$f0\",\"front_0_3s\":\"$f1\",\"front_6s\":\"$f2\",\"app_active\":\"$act\",\"is_key\":\"$key\"}" | tee -a "$OUT/activation.jsonl"
  done
}

run_case accessory-aioa1 "$APP" exec SP01_POLICY=accessory SP01_AIOA=1
run_case accessory-aioa0 "$APP" exec SP01_POLICY=accessory SP01_AIOA=0
run_case prohibited-aioa1 "$APP" exec SP01_POLICY=prohibited SP01_AIOA=1
run_case prohibited-aioa0 "$APP" exec SP01_POLICY=prohibited SP01_AIOA=0
run_case regular-aioa0 "$APP" exec SP01_POLICY=regular SP01_AIOA=0
run_case accessory-aioa0-deactivate "$APP" exec SP01_POLICY=accessory SP01_AIOA=0 SP01_DEACTIVATE=1
run_case accessory-aioa0-panelswap "$APP" exec SP01_POLICY=accessory SP01_AIOA=0 SP01_PANEL_SWAP=1
if [ -n "$LSUI" ]; then
  run_case lsuielement-aioa1-exec "$LSUI" exec SP01_POLICY=accessory SP01_AIOA=1
  run_case lsuielement-aioa0-exec "$LSUI" exec SP01_POLICY=accessory SP01_AIOA=0
  run_case lsuielement-aioa0-open-g "$LSUI" open SP01_POLICY=accessory SP01_AIOA=0
fi
echo "results in $OUT/activation.jsonl"
