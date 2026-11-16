#!/usr/bin/env bash
# Local macOS check before every push (NFR-13a). On pull_request, App CI runs on Windows only, so
# this script runs the commands of the App CI Checks job on your Mac. It runs every step, exits
# non-zero when any step fails, and prints a summary to paste into the PR.
#
# Usage: scripts/pre-push.sh
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1
export PATH="$HOME/.cargo/bin:$PATH"

results=()
failed=0

step() {
  local name="$1"
  shift
  echo
  echo "==> $name: $*"
  local start=$SECONDS
  if "$@"; then
    results+=("PASS  $name ($((SECONDS - start)) s)")
  else
    results+=("FAIL  $name ($((SECONDS - start)) s)")
    failed=1
  fi
}

step "pnpm install" pnpm install --frozen-lockfile
step "lint" pnpm lint
step "format check" pnpm format:check
step "typecheck" pnpm typecheck
step "TS tests" pnpm test:ci
step "cargo fmt" cargo fmt --all --check
step "clippy" cargo clippy --workspace --all-targets --locked -- -D warnings
if cargo nextest --version >/dev/null 2>&1; then
  step "Rust tests (nextest)" cargo nextest run --workspace --locked --profile ci
else
  step "Rust tests (cargo test, nextest missing)" cargo test --workspace --locked
fi
step "Rust doc tests" cargo test --workspace --locked --doc
step "repo checks" uv run --no-project python tools/ci_checks.py
step "script tests" uv run --no-project --with pytest==9.0.2 --with pyyaml==6.0.3 pytest scripts/tests -q

echo
echo "### pre-push.sh (macOS)"
echo
echo "- Commit: $(git rev-parse --short HEAD)"
echo "- Host: $(sw_vers -productName 2>/dev/null || uname -s) $(sw_vers -productVersion 2>/dev/null || uname -r), $(uname -m)"
echo "- Toolchain: $(rustc --version 2>/dev/null | cut -d' ' -f1-2), node $(node --version 2>/dev/null)"
echo
echo '```'
printf '%s\n' "${results[@]}"
echo '```'
if [ "$failed" -ne 0 ]; then
  echo "Result: FAIL"
  exit 1
fi
echo "Result: PASS"
