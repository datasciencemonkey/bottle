# Bottle

New here? Read the [vision and intro](docs/VISION.md).

Bottle turns your team's unwritten know-how into governed context that every Genie answer can use. It is captured where the work happens, approved by you, and shared with everyone. It is a small desktop app for Databricks users. Floating agents, called **MOGs**, let you dictate into any app, ask Genie One about your governed data, and record calls to your Unity Catalog volume. Bottle is a companion to Genie One, not a replacement.

## What ships now (macOS)

- **Dictation.** Hold **Right Option** and speak. Local Parakeet speech-to-text pastes the text at your cursor. A small pop-up offers Copy and Ask Genie.
- **Ask Genie.** Hold Right Option and tap **G** (⌥G). Bottle sends your words to Genie One through MCP. The answer card shows Markdown, tables, clickable workspace links and an Open in Databricks link.
- **The hub.** Conversations (with thread replies), Recordings and Settings.
- **Call recording.** Hold Right Option and tap **R** (⌥R). Bottle records the mic and system audio to your Unity Catalog volume. A Gemini model writes the transcript and a Claude model writes the notes, through the Databricks AI Gateway. Recording is off until you turn it on in Settings.
- **Several MOGs.** Each MOG has a name, an accent colour and a right-click menu. The lamp shows moods, an orbiting spark and eyes that follow the cursor.

See [`docs/VISION.md`](docs/VISION.md) for what is planned and not yet built, such as ontology hydration (recorded transcripts becoming UC Pages in your domain, after your review) and memory-first answers.

## Requirements

- macOS 14.4 or later. Windows 11 is second: its hotkeys are partly done (no Windows ⌥G yet).
- The Databricks CLI. Bottle can install the official CLI after you consent, then sign you in to a workspace.
- Tokens come from the CLI (`databricks auth token --profile <p>`) and live only in memory. One CLI profile is one workspace.

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
