# Bottle

New here? Read the [vision and intro](docs/VISION.md).

Bottle is a small desktop app (macOS 14.4+ first, Windows 11 second) for creating and managing **MOGs**: floating, always-resident agents that dictate, answer questions through Genie One MCP, and send approved context to Genie on Databricks. Bottle is a companion to Genie One, not a replacement.

**Status:** requirements v1.3 are final (all user confirmations U1-U16 settled). No application code yet; the next step is the week-1 spikes (SP-01..SP-21), then MVP-D and MVP-M.

## Core use cases

1. **Dictation (UC1):** local Parakeet STT, cleanup by a small Databricks-hosted model, insert into the active app. No agent, no Genie.
2. **Q&A (UC2):** the MOG's main agent (Claude or Codex harness) checks local memory, asks Genie One MCP, and shows a blurb with progress, key numbers and a deep link.
3. **Send context to Genie (UC3):** pick context from a meeting or memory, preview and approve it; approved definitions become the user's DRAFT Unity Catalog Pages.

Meeting sessions record and transcribe in the background (You + Others tracks, consent, fail-closed recording indicator). Nothing goes to Genie without a user click.

The MVP is done only when **MVP-D** (dictation and Q&A) and **MVP-M** (meeting recording) both pass end to end on a physical device against a live workspace.

## Architecture at a glance

- **Tauri v2** shell with a Rust core (`mog-core`) and a TS/React UI; one hidden `brain` window holds the domain logic.
- **No token crosses IPC.** Only the Rust core holds Databricks tokens; agent harnesses reach the model, Genie One MCP and the local `bottle-mcp` server through loopback proxies.
- The agent can never start capture, approve outbound context, publish a Page or switch workspace without the user.
- **Memory:** local encrypted SQLite (SQLCipher, FTS5, `sqlite-vec`). Lakebase is an optional v1 sync target.
- **Auth:** Databricks CLI U2M profiles (one profile = one workspace). Guided setup installs the official CLI after consent if it is missing.

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
