# Bottle v0.1.10

**Your Genie, ready.** Bottle is an independent, experimental open-source project. It is not an
official Databricks product. Bottle lets you dictate into any app, ask Genie about your data and
record calls from a small assistant on your desktop.

This is the first public release since v0.1.2. It is the Bottle MVP.

## Install on macOS

Requirements: macOS 14.4 or later on Apple Silicon, and the Databricks CLI. Bottle can install
the CLI for you on first launch.

The build is signed with a self-signed certificate. It is not notarized, so macOS shows a warning
once.

1. Download `Bottle-0.1.10-macos-arm64.dmg` and open it.
2. Drag Bottle to Applications.
3. Open Bottle from Applications. macOS shows a warning. Click **Done**.
4. Open **System Settings → Privacy & Security**. Scroll down and click **Open Anyway** next to
   Bottle. Or run `xattr -dr com.apple.quarantine /Applications/Bottle.app` in Terminal, then open
   Bottle again.
5. Grant the permissions when Bottle asks: **Microphone**, **Accessibility**, **Input Monitoring**,
   and **System Audio Recording** (for call recording).
6. Download the local speech model (487 MB) when Bottle asks, or set **Dictation engine** to
   Databricks in **Settings → Workspace**.

Windows and Linux are work in progress. Ignore any Windows installer on this page.

## What Bottle does

- **Dictate (hold Right Option).** Speak and release. Bottle types the text into the app you use.
  Speech is transcribed on your Mac with Parakeet, or on your workspace with
  `system.ai.gemini-3-8-flash`. A model on your AI Gateway fixes punctuation and capitals.
- **Ask Genie (⌥G).** Bottle asks your **home** workspace's Genie first. It asks your other
  workspaces only when home has no answer, or when your question names a workspace. Answers cite
  each source as [1] [2], with links to open the answer in Databricks.
- **Record calls (⌥R).** Bottle records your mic and the other side of the call. It uploads the
  audio to your workspace, writes a transcript with `system.ai.gemini-3-8-flash` and notes with
  `system.ai.claude-sonnet-5-5`.
- **Conversations.** Every question and answer is kept on your Mac, encrypted. Reply in a thread to
  ask a follow-up. Open each answer in Genie One in its workspace.

## What is new since v0.1.2

- One orchestrator model (`system.ai.claude-haiku-5-5` by default) cleans your dictation and plans
  each Genie question. Pick another model in **Settings → Workspace**.
- **Home first.** ⌥G asks your home workspace first, and asks other workspaces only when needed.
- **Sign-in on demand.** When a workspace's sign-in is missing or has expired, Bottle opens the
  browser sign-in and then asks again once.
- **Copy answers.** Bottle copies each finished Genie answer to your clipboard and shows "Copied".
  The copy stays on this Mac. Turn it off in **Settings → Workspace**.
- **Databricks dictation** as an alternative to local speech.
- **Call recording fixes.** System audio now records reliably. A take shows in the list at once,
  with its status, until it is done. Stop responds at once. A calm hint shows when other apps are
  silent, and **Open System Settings** opens the right page.
- **Recordings.** Delete a recording that failed or stayed on this Mac.
- **Workspaces.** Refresh the profile list, name a profile when you connect, and connect to more
  Databricks hosts.
- Removed: the "Combine across workspaces" switch. Answers from several workspaces are now always
  combined.
- Saved recording settings that name old model ids switch to the `system.ai` models.

## Privacy

- Tokens come only from `databricks auth token`. They stay in memory. Bottle never writes them.
- Every model call goes through your workspace's AI Gateway, with `store: false`. Your admin's
  gateway logging settings still apply.
- Bottle has no server of its own.

## Known issues

- The app is not notarized. macOS shows a warning on first launch.
- Windows and Linux builds are not ready.
