# Bottle

**Your Genie, ready.**

Bottle is an independent, experimental, open-source project. It is **not** an official Databricks
product and **not** Databricks Labs.

Bottle turns your team's unwritten know-how into context that every [Genie](https://www.databricks.com/product/ai-bi/genie)
answer can use. You dictate, ask questions and record calls where the work happens. You approve
what gets kept. Everyone's Genie gets smarter from it. See [`docs/VISION.md`](docs/VISION.md) for
the full picture, including what is planned and not yet built.

## What Bottle does

Bottle puts a small floating assistant, called a **MOG**, on your desktop. It is tied to your
Databricks workspace through your Databricks CLI profile.

- **Dictate.** Hold a key and speak. Bottle transcribes your voice on your Mac and pastes cleaned-up
  text wherever your cursor is.
- **Ask.** Hold a key, tap a letter, and ask a question out loud. Bottle asks your workspace's Genie
  and shows the answer next to the MOG, with citations and a deep link.
- **Record.** Hold a key, tap a letter, and record a call. Bottle uploads the audio to your Unity
  Catalog volume and writes a transcript and notes.

Today, capture (dictate, ask, record) and the Ask workflow ship. Turning recorded transcripts into
governed Unity Catalog Pages that Genie can search — the "knowledge flywheel" — is **planned**, not
shipped. Read [`docs/VISION.md`](docs/VISION.md) for the full plan.

## Install

Bottle runs on **macOS 14.4 or later, on Apple Silicon**. Windows and Linux: work in progress.

1. Go to [the latest release](https://github.com/datasciencemonkey/bottle/releases/latest) and
   download the `.dmg` file. (Or browse the [project site](https://datasciencemonkey.github.io/bottle/).)
2. Open the `.dmg` and drag Bottle to Applications.
3. Open Bottle from Applications.

The build is self-signed, not through the Apple Developer program, so macOS blocks it the first
time:

4. macOS shows a warning that it can't verify the app. Click **Done**.
5. Open **System Settings → Privacy & Security**. Scroll down and click **Open Anyway** next to
   Bottle. Confirm.
6. Open Bottle again from Applications.

You only see this warning once. Later versions use the same signing identity.

## First run

1. **Grant permissions.** Bottle asks for:
   - **Microphone** — for dictation and call recording.
   - **Accessibility** and **Input Monitoring** — for the hold-to-talk hotkeys and pasting text
     at your cursor.
   - **System Audio Recording** — only if you turn on call recording (see below). It lets Bottle
     hear the other side of a call.

   If you deny one, open **System Settings → Privacy & Security**, turn it on, and reopen Bottle.

2. **Connect a workspace.** If the Databricks CLI isn't installed, Bottle offers to install the
   official CLI, with your consent. Then "Connect a workspace" signs you in
   (`databricks auth login`) and saves a CLI profile — for example `my-workspace` — in
   `~/.databrickscfg`. One CLI profile is one workspace.
3. **Pick a home workspace.** If you connect more than one workspace, Bottle asks you to choose a
   home workspace. Bottle never picks one for you. The home workspace is always on; you switch
   other workspaces on or off in **Settings → Workspace**.

Your Databricks token comes only from the CLI (`databricks auth token --profile my-workspace`) and
lives only in memory. Bottle never writes it to disk.

## Using Bottle

### Dictate: hold Right Option

Hold **Right Option** and speak, then release.

> "Remind the team the churn definition excludes trial accounts."

Bottle transcribes your speech on your Mac with a local model (Parakeet). A chat model on your home
workspace's AI Gateway then cleans up the text — removes filler words, fixes punctuation — in about
1.5 seconds. Bottle pastes the cleaned text at your cursor only when it still matches what you said;
otherwise it pastes the local, uncleaned text. A small pop-up offers **Copy** and **Ask Genie**.

Turn off cleanup with the **Clean up dictation** switch in Settings → Workspace. You can also switch
**Dictation engine** to Databricks, which sends the audio to a Databricks model instead of
transcribing locally, and falls back to the local model on any error.

### Ask: ⌥G (hold Right Option, tap G)

Hold **Right Option**, tap **G**, ask your question out loud, then release.

> "What was our churn rate last quarter?"

Bottle asks your home workspace's Genie first. It asks your other switched-on workspaces only if
the home workspace has no answer, or if your question names a workspace by name. The answer
appears next to the MOG as Markdown, with tables where Genie returns them. Each source is cited as
`[1]`, `[2]`, and so on; each citation links to that workspace's Genie answer. The card also offers
**Open in Databricks** and, when more than one workspace answered, **Open Genie One in
`<workspace>`** for each one. Ask a follow-up the same way — it continues the same conversation.

If the model itself isn't reachable, Bottle falls back to asking Genie on your home workspace
directly and shows a short notice.

### Record: ⌥R (hold Right Option, tap R)

Call recording is **off until you turn it on** in Settings. Once it's on:

Hold **Right Option**, tap **R**, to start recording. Tap **Right Option + R** again to stop.

Bottle records your microphone and the other side of the call, uploads the audio to your Unity
Catalog volume, and writes a transcript and notes through the Databricks AI Gateway. Recording a
call may need everyone's consent — let people know before you start.

A recording that fails to upload, or one your Mac could not hear the other side of, stays only on
your Mac. You can delete that local-only copy from **Settings → Recordings**; deleting it removes
only the copy on this Mac.

## Privacy

- **Tokens stay in memory.** Bottle reads your Databricks token from the CLI at the moment it needs
  it and never writes it to disk.
- **Model calls don't get logged upstream.** Every call to a Databricks model sets `store: false`.
- **Only `system.ai.*` models, through your own AI Gateway.** Dictation cleanup and Ask use one
  orchestrator model (default `system.ai.claude-haiku-5-5`, pick another in Settings → Workspace).
  Call recording uses a Gemini model for the transcript and a Claude model for notes, both through
  your workspace's AI Gateway.
- **The model can't start a capture.** The orchestrator model has exactly two tools: ask Genie in a
  workspace you switched on, and submit its final answer. It cannot turn on your mic, start a
  recording, or reach the network on its own.
- **Your data stays in your workspace.** Recordings, transcripts and notes go to your own Unity
  Catalog volume. Questions and answers go to your own Genie. Bottle has no server of its own.
- **Local history is encrypted.** Each Genie exchange is saved on your Mac in an encrypted SQLite
  database (SQLCipher), with the key in the macOS keychain. Clear it any time from Settings → Wipe.

## Troubleshooting

**The hotkey does nothing.**
Check **System Settings → Privacy & Security → Accessibility** and **Input Monitoring**. Both need
to be on for Bottle. If you just granted Input Monitoring, reopen Bottle.

**Recording shows "Bottle can't hear the other side of the call."**
Open **System Settings → Privacy & Security → System Audio Recording** or click **Open System
Settings** on the notice, and turn Bottle on there.

**A workspace doesn't show up.**
Bottle reads workspaces from your Databricks CLI profiles. Check that the profile exists
(`databricks auth profiles list`), then click the refresh button (the circular arrow) in
**Settings → Workspace**.

**Sign-in expired.**
Settings → Workspace shows the CLI command to run for that workspace. Run it in Terminal, then
refresh.

## Developers

Prerequisites: Node 22+ with pnpm, Rust (stable), and [`uv`](https://docs.astral.sh/uv/) for every
Python command.

```sh
pnpm install
pnpm tauri dev        # run the app locally; add --features local-stt for local dictation
scripts/pre-push.sh   # the full local gate: lint, typecheck, tests, clippy — run before you push
```

See [`AGENTS.md`](AGENTS.md) for the architecture and where things live, and
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the branch, commit and PR workflow.

## License

MIT. See [`LICENSE`](LICENSE).
