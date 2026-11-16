#!/usr/bin/env bash
# SP-01 signed rerun (needs:human, needs:mac). Builds the spike, signs it with a
# Developer ID, notarizes, staples, installs from the DMG, and reruns the
# measurements against the INSTALLED app.
#
# Needs:
#   APPLE_SIGNING_IDENTITY  e.g. "Developer ID Application: Example Corp (TEAMID1234)"
#   NOTARY_PROFILE          keychain profile made once with:
#       xcrun notarytool store-credentials <your-profile> --apple-id <id> --team-id <team>
#   SP01_RIV                path to the built MOG mascot .riv (rive mog-rive --publish)
# Usage:
#   APPLE_SIGNING_IDENTITY=... NOTARY_PROFILE=<your-profile> SP01_RIV=/path/mog-rive.riv \
#     spikes/sp-01/sign_and_rerun.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
: "${APPLE_SIGNING_IDENTITY:?set APPLE_SIGNING_IDENTITY}"
: "${NOTARY_PROFILE:?set NOTARY_PROFILE (xcrun notarytool keychain profile)}"
: "${SP01_RIV:?set SP01_RIV to the mascot .riv}"
export PATH="$HOME/.cargo/bin:$PATH"
OUT="$HERE/out/signed-$(sw_vers -productVersion)-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$OUT"
{
  echo "macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion)) $(uname -m)"
  sysctl -n machdep.cpu.brand_string
  rustc --version; node --version; pnpm --version
} | tee "$OUT/env.txt"

cd "$HERE"
pnpm install --frozen-lockfile
node scripts/copy-vendor.mjs
swiftc -O -o out/probe probe-swift/Probe.swift
swiftc -O -o out/post_moves probe-swift/PostMoves.swift

# 1. Build and sign (Tauri signs the .app and the DMG with this identity).
APPLE_SIGNING_IDENTITY="$APPLE_SIGNING_IDENTITY" pnpm tauri build --bundles app,dmg 2>&1 | tee "$OUT/build.log"
DMG="$(ls -t src-tauri/target/release/bundle/dmg/*.dmg | head -1)"
codesign --verify --deep --strict --verbose=2 "src-tauri/target/release/bundle/macos/SP01 Rive Transparent.app" 2>&1 | tee "$OUT/codesign.txt"

# 2. Notarize and staple the DMG.
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait 2>&1 | tee "$OUT/notary.txt"
grep -q "status: Accepted" "$OUT/notary.txt"
xcrun stapler staple "$DMG" | tee "$OUT/staple.txt"
spctl -a -t open --context context:primary-signature -vv "$DMG" 2>&1 | tee "$OUT/spctl-dmg.txt"

# 3. Install from the DMG the way a user does (quarantine flag kept).
xattr -w com.apple.quarantine "0081;$(printf %x "$(date +%s)");Safari;" "$DMG"
MNT="$(mktemp -d)"
hdiutil attach -nobrowse -mountpoint "$MNT" "$DMG"
rm -rf "/Applications/SP01 Rive Transparent.app"
ditto "$MNT/SP01 Rive Transparent.app" "/Applications/SP01 Rive Transparent.app"
hdiutil detach "$MNT"
spctl -a -vv "/Applications/SP01 Rive Transparent.app" 2>&1 | tee "$OUT/spctl-app.txt"

# 4. Rerun the full matrix on the installed, signed app.
OUT="$OUT/run" "$HERE/run_measurements.sh" "/Applications/SP01 Rive Transparent.app" "$SP01_RIV" | tee "$OUT/summary.txt"
echo "Signed rerun output: $OUT"
