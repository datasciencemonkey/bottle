#!/usr/bin/env bash
# The macOS gate (NFR-13a, ADR 0045). App CI runs on Windows and Linux only, so run this script
# on a Mac before every push. It runs the commands of the App CI jobs: Checks (with the pinned
# cargo-nextest and requirement coverage, NFR-23a), Offline build (AT-37) and Adversarial golden
# set (SEC-26). It runs every step, exits non-zero when any step fails, and prints a summary to
# paste into the PR. A macOS-only break is caught only here.
#
# Usage: scripts/pre-push.sh
# Run it on each `git push`:  ln -sf ../../scripts/pre-push.sh "$(git rev-parse --git-common-dir)/hooks/pre-push"
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

# App CI pins cargo-nextest (NEXTEST_VERSION in app-ci.yml). Install the pin with
# `cargo install cargo-nextest --locked --version <pin>`, or take the prebuilt binary:
# `curl -LsSf https://get.nexte.st/<pin>/mac | tar zxf - -C ~/.cargo/bin`.
NEXTEST_VERSION=$(sed -n 's/^ *NEXTEST_VERSION: *"\(.*\)"/\1/p' .github/workflows/app-ci.yml)
nextest_pinned() {
  local have
  have=$(cargo nextest --version 2>/dev/null | head -n 1)
  echo "want cargo-nextest $NEXTEST_VERSION, have: ${have:-none}"
  [ -n "$NEXTEST_VERSION" ] && [[ "$have" == "cargo-nextest $NEXTEST_VERSION"* ]]
}
rust_tests() {
  cargo nextest run --workspace --locked --profile ci || return 1
  mkdir -p reports && cp target/nextest/ci/junit.xml reports/junit-rust.xml
}
step "nextest version" nextest_pinned
step "Rust tests (nextest)" rust_tests
step "Rust doc tests" cargo test --workspace --locked --doc
step "repo checks" uv run --no-project python tools/ci_checks.py
step "script tests" uv run --no-project --with pytest==9.0.2 --with pyyaml==6.0.3 \
  pytest scripts/tests -q --junitxml=reports/junit-py.xml
step "requirement coverage" uv run --no-project scripts/req_coverage.py \
  --requirements research/final-requirements.md \
  --junit reports/junit-ts.xml reports/junit-rust.xml reports/junit-py.xml \
  --out reports/requirements-coverage.md --json reports/requirements-coverage.json

# Offline build (AT-37): fetch with the locked lockfiles, then build with the network off.
offline_build() {
  pnpm install --frozen-lockfile && cargo fetch --locked || return 1
  rm -rf node_modules
  pnpm install --offline --frozen-lockfile && pnpm build &&
    CARGO_NET_OFFLINE=true cargo build --workspace --locked --offline
}
step "offline build" offline_build

# Adversarial golden set (SEC-26).
step "golden set (TS)" pnpm exec vitest run -t "SEC-26" --passWithNoTests
step "golden set (Rust)" cargo test --locked -p mog-core -p mog-core-ffi sec_26

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
