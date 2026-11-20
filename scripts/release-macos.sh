#!/usr/bin/env bash
# Build, sign and package the macOS release DMG on the owner's Mac. It never runs in CI: the
# signing key is the self-signed keychain identity "Bottle Local Dev Signing" and never leaves
# this Mac. The designated requirement stays `identifier "com.databricks.bottle" and certificate
# leaf = H"<sha1>"` across releases, so macOS keeps the TCC grants. Users see Gatekeeper once
# (the build is not notarized) and click Open Anyway.
#
# Usage: scripts/release-macos.sh <version> [--skip-build [--skip-git-check]]
#   --skip-build      DRY RUN: do not run pnpm/tauri; sign the app at $BOTTLE_APP (a stub app).
#                     The output name gets the -DRYRUN suffix and no gh command is printed.
#   --skip-git-check  skip the clean main == origin/main check (needs --skip-build)
# Output: dist/release/Bottle-<version>-macos-arm64.dmg and its .sha256. It prints the gh command
# and does not run it. Architecture: arm64 only. The vendored sherpa-onnx archives are pinned per
# target (osx-arm64 and osx-x64), so a universal build needs two archive sets and lipo.
set -euo pipefail

IDENTITY=58001E9C83E58ED8B44F6EC76C7F5774F84E34FB
BUNDLE_ID=com.databricks.bottle
APP_NAME=Bottle

version="${1:-}"
[ -n "$version" ] && shift || { echo "usage: $0 <version> [--skip-build] [--skip-git-check]" >&2; exit 2; }
skip_build=0
skip_git=0
for a in "$@"; do
  case "$a" in
    --skip-build) skip_build=1 ;;
    --skip-git-check) skip_git=1 ;;
    *) echo "unknown flag: $a" >&2; exit 2 ;;
  esac
done
[ "$skip_git" = 0 ] || [ "$skip_build" = 1 ] || { echo "--skip-git-check needs --skip-build" >&2; exit 2; }
dry=""
if [ "$skip_build" = 1 ]; then
  dry="-DRYRUN"
  echo "################ DRY RUN: stub app, not a release build. Do not publish the output. ################"
fi
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 0.1.0" >&2; exit 2; }
[ "$(uname -s)" = Darwin ] || { echo "release-macos.sh runs on macOS only" >&2; exit 1; }
[ "$(uname -m)" = arm64 ] || { echo "release-macos.sh builds arm64; run it on Apple Silicon" >&2; exit 1; }

cd "$(git rev-parse --show-toplevel)"
export PATH="$HOME/.cargo/bin:$PATH"

# 1. Inputs.
if [ "$skip_git" = 0 ]; then
  git fetch origin main
  [ "$(git rev-parse --abbrev-ref HEAD)" = main ] || { echo "not on main" >&2; exit 1; }
  [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || { echo "main is not at origin/main" >&2; exit 1; }
  [ -z "$(git status --porcelain)" ] || { echo "the tree is not clean" >&2; exit 1; }
fi

notes="docs/release/notes-v${version}.md"
[ -f "$notes" ] || { echo "missing $notes" >&2; exit 1; }

pkg_version=$(uv run --no-project python -I -c 'import json,sys;print(json.load(open(sys.argv[1]))["version"])' package.json)
tauri_version=$(uv run --no-project python -I -c 'import json,sys;print(json.load(open(sys.argv[1]))["version"])' src-tauri/tauri.conf.json)
[ "$pkg_version" = "$version" ] || { echo "package.json is $pkg_version, not $version" >&2; exit 1; }
[ "$tauri_version" = "$version" ] || { echo "tauri.conf.json is $tauri_version, not $version" >&2; exit 1; }
bad=$(cargo metadata --no-deps --format-version 1 | uv run --no-project python -I -c '
import json, sys
v = sys.argv[1]
bad = [p["name"] + "=" + p["version"] for p in json.load(sys.stdin)["packages"] if p["version"] != v]
print(" ".join(bad))' "$version")
[ -z "$bad" ] || { echo "Cargo package versions differ from $version: $bad" >&2; exit 1; }

security find-identity -p codesigning | grep -q "$IDENTITY" \
  || { echo "signing identity $IDENTITY is not in the keychain" >&2; exit 1; }

# 2. Build.
if [ "$skip_build" = 1 ]; then
  app="${BOTTLE_APP:?set BOTTLE_APP to the stub app with --skip-build}"
else
  pnpm install --frozen-lockfile
  uv run --no-project python scripts/vendor_sherpa_onnx.py
  pnpm tauri build --features local-stt --bundles app -- --locked
  app="target/release/bundle/macos/${APP_NAME}.app"
fi
[ -d "$app" ] || { echo "no app at $app" >&2; exit 1; }

# Everything below writes under dist/release. Do it after the build: the frontend build clears dist/.
out="dist/release"
stage="$out/stage"
dmg="$out/${APP_NAME}-${version}-macos-arm64${dry}.dmg"
rm -rf "$stage" "$dmg" "$dmg.sha256"
mkdir -p "$stage"
ditto "$app" "$stage/${APP_NAME}.app"

# 3. Sign. Info.plist usage strings stay in the bundle; entitlements (if any) are kept.
codesign --force --deep --preserve-metadata=entitlements --sign "$IDENTITY" \
  --identifier "$BUNDLE_ID" "$stage/${APP_NAME}.app"
codesign --verify --deep --strict "$stage/${APP_NAME}.app"
dr=$(codesign -d -r- "$stage/${APP_NAME}.app" 2>&1 | grep designated)
echo "$dr"
shopt -s nocasematch
[[ "$dr" == *"identifier \"$BUNDLE_ID\""* && "$dr" == *"certificate leaf = H\"$IDENTITY\""* ]] \
  || { echo "designated requirement is not the expected one" >&2; exit 1; }
shopt -u nocasematch

# 4. DMG: the app plus an /Applications link. Sign the DMG too.
ln -s /Applications "$stage/Applications"
hdiutil create -volname "$APP_NAME" -srcfolder "$stage" -ov -format UDZO "$dmg"
codesign --force --sign "$IDENTITY" --identifier "${BUNDLE_ID}.dmg" "$dmg"
codesign --verify "$dmg"
rm -rf "$stage"

# 5. SHA-256.
(cd "$out" && shasum -a 256 "$(basename "$dmg")" > "$(basename "$dmg").sha256")
cat "$dmg.sha256"

# 6. The command for the owner. This script does not run it.
if [ -n "$dry" ]; then
  echo "DRY RUN: no gh command printed."
  exit 0
fi
pre=""
case "$version" in *-rc*) pre=" --prerelease" ;; esac
echo
if gh release view "v${version}" >/dev/null 2>&1; then
  echo "The release v${version} exists (the Windows workflow made the draft). Run:"
  echo "gh release upload v${version} $dmg $dmg.sha256"
  echo "gh release edit v${version} --notes-file $notes --draft=false"
else
  echo "Run after the v${version} tag is pushed:"
  echo "gh release create v${version} $dmg $dmg.sha256 --notes-file $notes --verify-tag${pre}"
fi
