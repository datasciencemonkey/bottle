#!/usr/bin/env bash
# Build the macOS debug DMG on a Mac, sign and notarize it when the secrets exist, and write a
# SHA-256 file for it (NFR-15a, NFR-15d). App CI has no macOS runner (ADR 0045), so this script
# replaces the macOS leg of the App CI package job.
#
# Signing runs only when APPLE_CERTIFICATE and APPLE_SIGNING_IDENTITY are set (APPLE_CERTIFICATE
# is the base64 .p12 that Tauri imports; APPLE_CERTIFICATE_PASSWORD is its password).
# Notarization runs only when the DMG is signed and APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID are
# set. Without the secrets the DMG is unsigned and not notarized.
#
# Usage: scripts/package-macos.sh          build, sign and notarize as the secrets permit
#        scripts/package-macos.sh --plan   print the sign and notarize decision only (any OS)
set -euo pipefail

sign=skip
notarize=skip
if [ -n "${APPLE_CERTIFICATE:-}" ] && [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  sign=yes
  if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    notarize=yes
  fi
fi

if [ "${1:-}" = "--plan" ]; then
  echo "sign: $sign"
  echo "notarize: $notarize"
  exit 0
fi

if [ "$(uname -s)" != "Darwin" ]; then
  echo "package-macos.sh runs on macOS only" >&2
  exit 1
fi

cd "$(git rev-parse --show-toplevel)"
export PATH="$HOME/.cargo/bin:$PATH"
BUNDLE_DIR=target/debug/bundle/dmg

pnpm install --frozen-lockfile
rm -f "$BUNDLE_DIR"/*.dmg "$BUNDLE_DIR"/*.sha256

# The build gets the signing variables only when signing runs, and never the notary variables:
# the DMG is notarized and stapled below, as the App CI package job did.
unset_vars=(-u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID)
if [ "$sign" = skip ]; then
  unset_vars+=(-u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD -u APPLE_SIGNING_IDENTITY)
  echo "APPLE_* signing secrets are absent; the DMG is unsigned and not notarized."
fi
env "${unset_vars[@]}" pnpm tauri build --debug --bundles dmg -- --locked

if [ "$notarize" = yes ]; then
  for f in "$BUNDLE_DIR"/*.dmg; do
    xcrun notarytool submit "$f" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" --wait
    xcrun stapler staple "$f"
  done
elif [ "$sign" = yes ]; then
  echo "APPLE_ID, APPLE_PASSWORD or APPLE_TEAM_ID is absent; the DMG is signed but not notarized."
fi

shopt -s nullglob
files=("$BUNDLE_DIR"/*.dmg)
[ ${#files[@]} -gt 0 ] || { echo "no DMG in $BUNDLE_DIR" >&2; exit 1; }
for f in "${files[@]}"; do
  (cd "$(dirname "$f")" && shasum -a 256 "$(basename "$f")" > "$(basename "$f").sha256")
  cat "$f.sha256"
done
