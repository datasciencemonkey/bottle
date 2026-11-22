# Bottle v0.1.2

Bottle is an experimental Databricks Labs app. It lets you dictate, ask Genie and record calls from a small lamp on your desktop.

## Install on macOS

Requirements: macOS 14.4 or later (Apple Silicon) and the Databricks CLI. Bottle can install the CLI for you on first launch.

The build is signed with a self-signed certificate. It is not notarized. macOS shows a warning once.

1. Download `Bottle-0.1.2-macos-arm64.dmg` and open it.
2. Drag Bottle to Applications.
3. Open Bottle from Applications. macOS shows a warning. Click **Done**.
4. Open System Settings, then Privacy & Security. Scroll down and click **Open Anyway** next to Bottle. Confirm. Or, instead of this step, run this command in Terminal and open Bottle again: `xattr -dr com.apple.quarantine /Applications/Bottle.app`
5. Grant the permissions when Bottle asks: **Microphone**, **Accessibility** and **Input Monitoring**. If you deny one, open System Settings, then Privacy & Security, and turn it on. Reopen Bottle after you grant Input Monitoring.

Later versions keep the same signing identity, so you only see the warning once and your permissions stay.

## Install on Windows

Requirements: Windows 11 and the Databricks CLI.

1. Download the `.exe` installer and run it.
2. SmartScreen shows "Windows protected your PC". Click **More info**, then **Run anyway**. The installer is not signed.
3. Dictation works on Windows: hold Right Alt (AltGr) to talk. Bottle on Windows has no Option-G or Option-R yet. Ask Genie by hotkey and call recording are macOS only for now.

## What is new in v0.1.2

- **Faster first screen (#622).** The hub no longer sits on a loading skeleton after launch. If the brain is late, Bottle asks again for a few seconds and then shows a short message.
- **New Home screen (#624).** The hub opens on Home, with the lamp, "Bottle" and "Ship in a bottle." The sidebar order is now Home, MOGs, Conversations, Recordings and Settings.
- **Wipe works (#625).** Settings, then Wipe, now deletes local history older than 30 days, older than 90 days, or everything. If nothing is old enough, it says "Nothing to wipe." If a wipe fails, it shows a short reason.

Planned, not in this release: turning transcripts into Unity Catalog Pages, memory-first ask, and Option-G on Windows.

## Known issues

- A dependency advisory on the `glib` crate is open until the next Tauri upgrade (#612).
- The macOS build is not notarized. The Windows installer is not signed. Both show a first-launch warning.
- Windows has no Option-G or Option-R hotkeys yet.
