# Bottle

New here? Read the [vision and intro](docs/VISION.md).

Bottle turns your team's unwritten know-how into governed context that every Genie answer can use. It is captured where the work happens, approved by you, and shared with everyone. It is a small desktop app for Databricks users. Floating agents, called **MOGs**, let you dictate into any app, ask Genie One about your governed data, and record calls to your Unity Catalog volume. Bottle is a companion to Genie One, not a replacement.

## What ships now (macOS)

- **Dictation with cleanup.** Hold **Right Option** and speak. Local Parakeet speech-to-text transcribes the speech. Bottle removes filler words on the device. Then one orchestrator model cleans up the text, in 1.5 seconds or less, on your home workspace's AI Gateway. Bottle pastes the model text only when its words match yours. Otherwise it pastes the local text. The setting "Clean up dictation" switches this step off. A small pop-up offers Copy and Ask Genie.
- **Ask.** Hold Right Option and tap **G** (⌥G). The orchestrator model asks the Genie One of each workspace that you switched on, and it can combine the answers. A combined answer cites each workspace as [1] [2]. Each marker links to that workspace's answer and its Open in Databricks link. If the model is not reachable, Bottle asks Genie One on your home workspace and shows a short notice. The answer card shows Markdown, tables and safe links.
- **The hub.** Conversations (with thread replies), Recordings and Settings. In Settings → Workspace, you pick the orchestrator model from the `system.ai` chat models. You also switch "Clean up dictation" and "Combine across workspaces" there.
- **Call recording.** Hold Right Option and tap **R** (⌥R). Bottle records the mic and system audio to your Unity Catalog volume. A Gemini model writes the transcript and a Claude model writes the notes, through the Databricks AI Gateway. Recording is off until you turn it on in Settings. A recording that did not upload can be deleted from Settings → Recordings with Delete; that removes only the copy on this Mac.
- **One MOG.** One floating MOG serves all your workspaces. Each connected Databricks CLI profile is a workspace of that MOG. The lamp shows moods, an orbiting spark and eyes that follow the cursor.

See [`docs/VISION.md`](docs/VISION.md) for what is planned and not yet built, such as ontology hydration (recorded transcripts becoming UC Pages in your domain, after your review) and memory-first answers.

## Requirements

- macOS 14.4 or later. Windows 11 is second: its hotkeys are partly done (no Windows ⌥G yet).
- The Databricks CLI. Bottle can install the official CLI after you consent, then sign you in to a workspace.
- Tokens come from the CLI (`databricks auth token --profile <p>`) and live only in memory. One CLI profile is one workspace. Model calls send `store: false`.

## Run locally

You need Node with pnpm, Rust stable, and `uv` for every Python command.

```sh
pnpm install
pnpm tauri dev                      # add --features local-stt for local Parakeet
scripts/package-macos.sh            # build a macOS DMG
scripts/pre-push.sh                 # the full local gate, before you push
```

Local speech-to-text needs the vendored sherpa-onnx build. See [`AGENTS.md`](AGENTS.md) section 4 for the setup steps.

## Start here

| File | Purpose |
|---|---|
| [`AGENTS.md`](AGENTS.md) | Context index and binding decisions for agents working in this repo |
| [`requirements.html`](requirements.html) | Final end-to-end requirements (v1.3, ASD-STE100, 9 SVG diagrams). Single source of truth. |
| [`research/final-requirements.md`](research/final-requirements.md) | Markdown source for `requirements.html` |
| [`research/director-decisions.md`](research/director-decisions.md) | Director decisions log |
| [`research/connector-spec-2026-11-14.md`](research/connector-spec-2026-11-14.md) | Verified Genie, MCP, OAuth and Lakebase API facts |
| [`research/message-board.md`](research/message-board.md) | Append-only agent message board |
| [`research/diagrams/`](research/diagrams/) | Architecture diagrams (SVG) |
| [`research/kimi-snapshot/`](research/kimi-snapshot/) | Snapshot of the alternative Kimi requirements run (reference only, never run) |
| [`mog-rive/`](mog-rive/) | Rive RML source for the MOG mascot (`tools/gen_mog_rml.py` regenerates `scene.rml`) |
| [`REQUIREMENTS.md`](REQUIREMENTS.md) | Early draft v0.2, superseded by `requirements.html` |

## Build

Rebuild the requirements HTML:

```sh
uv run python tools/build_requirements_html.py
```

Regenerate the mascot scene:

```sh
uv run python tools/gen_mog_rml.py
```

Build the mascot `.riv` before `pnpm build` (needs `rive login`; never commit the `.riv`). Without it, the MOG view shows the PNG fallback (ADR 0027):

```sh
rive mog-rive --publish
```

Regenerate the transparent PNG fallback after an art change (no login needed):

```sh
uv run --no-project --with pillow python tools/render_mog_fallback.py
```

## License

MIT. See [`LICENSE`](LICENSE).
