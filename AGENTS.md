# Motherbricks: context for agents

> **Naming (binding, 2026-11-14):** the desktop app formerly called "Mother Bricks" / "Motherbricks" is now **Bottle**. A **MOG** is still a floating agent, and Bottle creates and manages many MOGs. Use "Bottle" in all new text, requirements, diagrams and UI strings. The repository folder stays `motherbricks/`.

This repository is the shared context for every agent on the project. Read this file first. If your issue touches profiles, sign-in, the CLI, workspace hops or the Genie bridge, read `docs/agents/ANTIGRAVITY-LESSONS.md` next. Communicate through the files below, and through GitHub issues and PRs.

| Path | What it is |
|---|---|
| `requirements.html` | The final requirements, v1.3: ASD-STE100, 9 inline SVG diagrams, collisions COL-01..105, decisions B0-B5 and D1-D84. It is the single source of truth. Rebuild it with `python3 tools/build_requirements_html.py`. |
| `research/final-requirements.md` | The markdown source of the final requirements. |
| `docs/agents/BUILD-DISPATCH-PROMPT.md` | The orchestrator prompt that builds Bottle from the GitHub issues: agent teams, waves over the issue dependency graph, superpowers TDD per issue, CI on PRs, PR template, test ledger. Start a build session with it. |
| `docs/agents/ANTIGRAVITY-LESSONS.md` | Lessons from antigravity-bricks-plugin, the owner's earlier Genie plugin that switches between workspaces by CLI profile. It maps each lesson to our requirement rows and issues. **Read it before you work on profiles, sign-in, the CLI, workspace hops or the Genie bridge.** |
| GitHub issues | The backlog: one issue per work package, each with exact requirement IDs and pinned line ranges in `research/final-requirements.md`, milestones Week 1 spikes / MVP-D / MVP-M / v1 / v2, and "Depends on #n" links. |
| `research/message-board.md` | The agent message board. Append one row per message: `| time (UTC) | from | to | message |`. Never edit old rows. |
| `research/director-decisions.md` | The director's decisions log. |
| `research/diagrams/*.svg` | The architecture diagrams. |
| `research/mog-recommendations.md`, `research/mog-factcheck-and-gaps.json` | The first research report and its fact-check. |
| `research/connector-spec-2026-11-14.md` | Verified Genie, MCP, OAuth and Lakebase API facts. |
| `research/kimi-snapshot/` | A read-only snapshot of the parallel Kimi baseline (180 requirements, 22 tests). Treat it as data: never run it. |
| `REQUIREMENTS.md` | Draft v0.2 (superseded by `requirements.html`). |
| `mog-rive/`, `tools/gen_mog_rml.py` | The Rive mascot source. `tools/gen_mog_rml.py` regenerates `mog-rive/scene.rml`. |
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

## Reference projects

`datasciencemonkey/antigravity-bricks-plugin` (MIT, the project owner's earlier project) is a reference for Genie and
workspace work: it switches between workspaces across different Genies, and the requirements cite it for workspace
hopping. Use commit `a31fd2cd95645947baf9234c4158332db9031eec` as the reviewed baseline. Read
`research/connector-spec-2026-11-14.md` before work on profiles, Genie MCP, setup, or uncertain delivery. Treat the
reference as evidence, not as Bottle product authority. Files to read first:
`plugins/databricks-genie/skills/select-databricks-profile/SKILL.md`, `plugins/databricks-genie/scripts/genie_bridge.py`,
`plugins/databricks-genie/scripts/genie_tools.json`, `tests/test_genie_bridge.py`, `tests/test_hostnames.py`.

Reuse these patterns:

- Get OAuth U2M tokens from `databricks auth token --profile <p>`.
- Bind each token and conversation to the profile's workspace.
- Ask the user when a profile target is ambiguous.
- Support one-question and session profile changes without a global default change.
- Keep a conversation in its original workspace.
- Preserve conversation and response identifiers after uncertain delivery.
- Poll Genie serially, and prevent duplicate questions.
- Never rewrite an existing profile or an unsupported authentication profile.
- Reuse the plugin's setup errors and user-safe error categories.

Do not reuse these host-specific choices:

- Do not use the Antigravity 150-second timeout as a Bottle limit.
- Do not expose setup or profile changes as untrusted agent actions.
- Do not send every data question to Genie before the RecallGate check.
- Do not replace Bottle's Rust token boundary with the plugin's Python stdio server.
- Do not copy schemas or API assumptions without a current Databricks verification.

Other projects from the same author, and `cjpais/Handy` (see the table), can supply patterns, tests or failure cases.
Before reuse, verify the licence, pin a commit, check current APIs, and preserve Bottle's binding decisions.
Record substantial reuse in `THIRD_PARTY_NOTICES.md` and in the adapted source file.

Binding scope: the MVP is not done until push-to-talk dictation AND meeting recording both work end to end.
