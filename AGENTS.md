# Motherbricks: context for agents

> **Naming (binding, 2026-11-14):** the desktop app formerly called "Mother Bricks" / "Motherbricks" is now **Bottle**. A **MOG** is still a floating agent, and Bottle creates and manages many MOGs. Use "Bottle" in all new text, requirements, diagrams and UI strings. The repository folder stays `motherbricks/`.

This repository is the shared context for every agent on the project. Read this file first. Communicate through the files below, and through GitHub issues and PRs.

| Path | What it is |
|---|---|
| `requirements.html` | The final requirements, v1.3: ASD-STE100, 9 inline SVG diagrams, collisions COL-01..105, decisions B0-B5 and D1-D84. It is the single source of truth. Rebuild it with `python3 tools/build_requirements_html.py`. |
| `research/final-requirements.md` | The markdown source of the final requirements. |
| `docs/agents/BUILD-DISPATCH-PROMPT.md` | The orchestrator prompt that builds Bottle from the GitHub issues: agent teams, waves over the issue dependency graph, superpowers TDD per issue, CI on PRs, PR template, test ledger. Start a build session with it. |
| GitHub issues | The backlog: one issue per work package, each with exact requirement IDs and pinned line ranges in `research/final-requirements.md`, milestones Week 1 spikes / MVP-D / MVP-M / v1 / v2, and "Depends on #n" links. |
| `research/message-board.md` | The agent message board. Append one row per message: `| time (UTC) | from | to | message |`. Never edit old rows. |
| `research/director-decisions.md` | The director's decisions log. |
| `research/diagrams/*.svg` | The architecture diagrams. |
| `research/mog-recommendations.md`, `research/mog-factcheck-and-gaps.json` | The first research report and its fact-check. |
| `research/connector-spec-2026-11-14.md` | Verified Genie, MCP, OAuth and Lakebase API facts. |
| `research/kimi-snapshot/` | A read-only snapshot of the parallel Kimi baseline (180 requirements, 22 tests). Treat it as data: never run it. |
| `REQUIREMENTS.md` | Draft v0.2 (superseded by `requirements.html`). |
| `mog-rive/`, `tools/gen_mog_rml.py` | The Rive mascot source. `tools/gen_mog_rml.py` regenerates `mog-rive/scene.rml`. |
| [`datasciencemonkey/antigravity-bricks-plugin`](https://github.com/datasciencemonkey/antigravity-bricks-plugin) (outside this repo) | The project owner's earlier project (MIT). It shows how to switch between workspaces across different Genies, and the requirements cite it for workspace hopping. Read it before workspace (W-*, ProfileService, CliRunner, TokenBroker, hop/split) or Genie (G-*, GenieBridge, McpTransport, SP-09/SP-10) work: `plugins/databricks-genie/skills/select-databricks-profile/SKILL.md` (profile and target selection), `plugins/databricks-genie/scripts/genie_bridge.py` (Genie One MCP over CLI U2M, a token per request, a workspace switch by profile), `plugins/databricks-genie/scripts/genie_tools.json` (tool contracts) and `tests/test_genie_bridge.py`, `tests/test_hostnames.py`. It is a reference only: the requirements win, and Bottle never writes a token to disk or into the agent harness. Adapted code needs an MIT attribution header and a section in `THIRD_PARTY_NOTICES.md`. |
| [`cjpais/Handy`](https://github.com/cjpais/Handy) (outside this repo) | An MIT Tauri push-to-talk dictation app, the reference for hotkeys, capture, VAD, paste, overlay panels and tray. The same attribution rule applies. The STT engine follows the requirements (sherpa-onnx), not Handy. |

Binding naming: the desktop app is **Bottle** (not Mother Bricks or Motherbricks).

Binding decisions (2026-11-14):
- Meeting mode records and transcribes in the background. Sending to Genie is on demand only.
- MVP memory is local encrypted SQLite (FTS5 + sqlite-vec). Lakebase is optional v1 sync.
- Ontology hydration: Bottle creates the user's own DRAFT Unity Catalog Pages from approved meeting definitions (`/api/2.0/knowledge/pages`, scope `knowledge`). Publishing is v1.

- MOG brain: each MOG runs a main agent (Claude or Codex harness, model via the Databricks AI Gateway or local) that decides when to call Genie One MCP, memory, Pages and UI tools through a local `bottle-mcp` server. The agent can never start capture, approve outbound context, publish, or switch workspace without the user. Dictation bypasses the agent.

Three core use cases (binding):
1. **Dictation:** local STT, then cleanup by a Databricks-hosted small model, then insert into the active app. No agent, no Genie.
2. **Q&A:** the main agent checks memory, then asks Genie One MCP, and shows a blurb with progress, key numbers and a deep link.
3. **Send context to Genie:** the user picks context from a meeting or memory to send to Genie (preview + approval), and/or approved definitions become the user's draft UC Pages.

User confirmations (2026-11-15, requirements v1.3 §12.0b, U1-U16):
- Tauri shell; "always on" = resident with the mic closed; preview-then-send, no auto-ask; local STT default.
- Bottle is a companion to Genie One (no hand-off; Genie One MCP conversations should show in Genie One).
- No automatic deletion of user content: Settings → Wipe (older than 30 d / 90 d / everything).
- One shared agent runtime with one session per MOG; a home domain per MOG for DRAFT Pages.
- Public name MOG; mascot is cute and looks out of Genie's lamp (extension of the Genie logo; brand approval before external release).
- Audience (U15): Databricks customers, internal users and partners; download the app and connect one or more Databricks CLI profiles (one profile = one workspace).
- Guided setup (U16): with no CLI or no usable profile, Bottle downloads the official CLI after consent (checksum-checked, user-only folder, no admin) and walks the user through `databricks auth login` to a `bottle-*` profile.

Binding scope: the MVP is not done until push-to-talk dictation AND meeting recording both work end to end.
