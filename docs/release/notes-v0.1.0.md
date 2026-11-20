# Bottle v0.1.0

Bottle is an experimental Databricks Labs app. It lets you dictate, ask Genie and record calls from a small lamp on your desktop.

## Install on macOS

Requirements: macOS 14.4 or later (Apple Silicon) and the Databricks CLI. Bottle can install the CLI for you on first launch.

The build is signed with a self-signed certificate. It is not notarized. macOS shows a warning once.

1. Download `Bottle-0.1.0-macos-arm64.dmg` and open it.
2. Drag Bottle to Applications.
3. Open Bottle from Applications. macOS shows a warning. Click **Done**.
4. Open System Settings, then Privacy & Security. Scroll down and click **Open Anyway** next to Bottle. Confirm. Or, instead of this step, run this command in Terminal and open Bottle again: `xattr -dr com.apple.quarantine /Applications/Bottle.app`
5. Grant the permissions when Bottle asks: **Microphone**, **Accessibility** and **Input Monitoring**. If you deny one, open System Settings, then Privacy & Security, and turn it on. Reopen Bottle after you grant Input Monitoring.

Later versions keep the same signing identity, so you only see the warning once and your permissions stay.

## Install on Windows

Requirements: Windows 11 and the Databricks CLI.

1. Download the `.exe` installer and run it.
2. SmartScreen shows "Windows protected your PC". Click **More info**, then **Run anyway**. The installer is not signed.
3. Bottle on Windows has no Option-G or Option-R yet. Dictation works. Ask Genie by hotkey and call recording are macOS only for now.

## What is in v0.1.0

- **Dictation.** Hold Right Option, speak, and release. Local Parakeet speech-to-text turns the audio into text. Bottle pastes it at the cursor. A pop-up shows the text with Copy and Ask Genie.
- **Ask Genie with Option-G.** Hold Right Option, tap G, and release. Bottle sends your words to Genie One and shows a short answer card with Open in Databricks. You can ask a follow-up in the same thread.
- **Call recording with Option-R.** Records your mic and the system audio, uploads to a Unity Catalog volume, and returns a transcript and notes through AI Gateway. The default notes preset lists action items for each speaker. The setting is off by default.
- **Several MOGs.** Each lamp has its own name, colour and moods.
- **Local history.** Each Genie exchange is saved in an encrypted local store. Settings has a Wipe button.
- **Databricks setup.** Bottle installs the Databricks CLI after you agree, and the Connect a workspace form signs you in.

Planned, not in this release: turning transcripts into Unity Catalog Pages, memory-first ask, dictation cleanup, and Option-G on Windows.

## Known issues

- Option-R on macOS: a second R press in the same Right Option hold can type a ® character into the focused app (#604).
- A dependency advisory on the `glib` crate is open until the next Tauri upgrade (#612).
- The macOS build is not notarized. The Windows installer is not signed. Both show a first-launch warning.
- Windows has no Option-G or Option-R hotkeys yet.
