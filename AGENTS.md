# Bottle: context for coding agents

Read this file first. It tells you what we are building, where the code lives, how to work in this repo, and which
work is in or out of scope. If this file and `requirements.html` disagree on **scope**, this file wins (it records the
user's later decisions). For detailed behaviour, `requirements.html` stays the source of truth.

Naming: the desktop app is **Bottle** (never "Mother Bricks" or "Motherbricks"). A **MOG** is a small floating
assistant window that Bottle creates and manages. Use "Bottle" in all new text, code, diagrams and UI strings.

## 1. What we are building (MVP, as of 2026-11-19)

Bottle is a Tauri desktop app (macOS first, Windows second) for Databricks users. This section describes the
**target** MVP. Section 1a says which parts are on main and which are still in open PRs. The MVP has two jobs:

| Key action | What happens |
|---|---|
| Hold **Right Option**, speak, release | **Dictation.** Local Parakeet STT transcribes the audio on the device. Bottle pastes the text at the cursor in the frontmost app. A small pop-up on the MOG shows the text with Copy and **Ask Genie** (PRs #572, #571). |
| Hold **Right Option**, tap **G** (⌥G) during the hold, release | **Ask Genie.** Nothing is pasted. Bottle sends the exact text to **Genie One** through a direct MCP call, then shows the answer (markdown and key numbers) in the MOG card. |

The user always knows whether they dictate or ask, so the key decides. Bottle does not guess the intent. The pop-up's
Ask Genie button is only a fallback for text you dictated and then want to ask about.

Binding decisions for the MVP:

- **Direct Genie path.** Ask Genie is one direct Genie One MCP call (JSON-RPC over HTTPS). No agent, no Claude Agent
  SDK, no Python bridge and no generic fetch layer sits in the middle. The parts on main: TokenBroker (R6),
  McpTransport (R8, tool allowlist `genie_ask`, `genie_poll_response`, `genie_get_query_result`,
  `genie_cancel_response`), PollActor and GenieBridge (T8). See ADR 0097.
- **Auth.** Tokens come only from `databricks auth token --profile <p>` (OAuth U2M through the user's Databricks CLI)
  and live only in memory. One CLI profile = one workspace. The user picks the profile; code never guesses one.
- **Local history.** Each Genie exchange is saved in the local encrypted SQLite store (SQLCipher) through the
  `mem_local_exec` route. There is no cloud sync in the MVP.
- **Onboarding.** If the Databricks CLI is missing, Bottle offers to download the official CLI (after consent,
  SHA-256 checked, user-only folder, no admin) (#161). Then a "Connect a workspace" form runs `databricks auth login`
  in the background (#162).
- **Design.** Minimal, calm UI on the shared tokens in `src/views/tokens.css`. The pop-up is small and theme-aligned.

### Out of scope for now ("we'll consider later")

Meeting recording, UC Pages, Lakebase sync, memory recall and embeddings, the main agent and `bottle-mcp` tools,
workspace hop and split questions, mascot artwork work, admin/MDM, telemetry export, audit log, cloud ASR, Linux,
signed public installers, and most spikes. Code for some of these is already on main. Leave it alone unless an MVP
change needs it. Do not open new issues for this work. The non-core issues are being closed with "We'll consider this
later".

### 1a. On main vs in flight (2026-11-19)

| Part | State |
|---|---|
| Hotkey, capture, local Parakeet STT, paste (`insert_text`) | On main. |
| TokenBroker, McpTransport, PollActor, GenieBridge (T8) | On main. |
| Ask route trigger | On main it is **Shift** with the talk key (`hotkey:ptt` sends `shift: true`, ADR 0053), and without Shift a router model guesses the intent. Target: ⌥G, no guess (#577, ADR 0098, not yet written). |
| Ask send path | On main the ask gate still calls `agent_send` (`src/brain/components/index.ts`). Target: straight to GenieBridge T8 with `genie_one_mcp` (PR #575, ADR 0097, stacked on #571). |
| Pop-up with Copy and Ask Genie | PR #572 (pop-up) and PR #571 (dictation first, ADR 0096). |
| `mem_local_exec` to the local SQLite store | PR #576. On main the route is still `TODO(#426)`. |
| CliRunner (R5) | PR #255. On main mcp_host uses a stub, so every `mcp_call` fails `not_implemented`. |
| CLI install, Connect a workspace | #161, #162 (in progress, stacked on #255). |

Re-check with `gh pr list` before you rely on this table.

## 2. Architecture map

```
 talk key (Right Option, ⌥G)
        │
 ┌──────▼─────────────────────────── Rust core: crates/mog-core + src-tauri ───────────────────────────┐
 │ hotkey_macos / hotkey_windows ─► hotkey_host ─► hotkey.ptt event (Down / Up, ask flag)               │
 │ capture (single mic owner, cpal) ─► stt_engine / stt_local (sherpa-onnx Parakeet, feature local-stt) │
 │ insertion (insert_text: clipboard paste, no Enter, clipboard restore)                                │
 │ cli_runner (R5: find, vet, run `databricks`) ─► token_broker (R6) ─► mcp_transport (R8) ─► Genie One │
 │ profile_service (CLI profiles)   local_store (SQLCipher, mem_local_exec)   ipc + ipc_gateway (gate)  │
 └──────▲───────────────────────────────────────────────────────────────────────────────────────────────┘
        │ Tauri IPC: commands and events, typed in src/shared/ipc.ts ⇄ crates/mog-core/src/ipc.rs
 ┌──────┴──────────── brain: src/brain (hidden window, TypeScript) ──────────────────────────────────────┐
 │ ptt / capture-lifecycle ─► stt-wiring ─► utterance-route: dictation → insert_text | ask → genie-bridge │
 │ components/genie-bridge (T8) + poll-actor + ask-events ─► card     memory-service ─► mem_local_exec    │
 └──────┬─────────────────────────────────────────────────────────────────────────────────────────────────┘
        │ events to views
 ┌──────▼──────── views: src/views (React, one WebView each) ────────┐
 │ mog/  floating MOG and pop-up    card/  answer card   hub/  main window, profiles, settings │
 └──────────────────────────────────────────────────────────────────────┘
```

Rules of the boundary:

- Rust owns the mic, the hotkey, the paste, the CLI, tokens, network and storage. The brain never sees a token.
- The brain owns flow and UI state. Views only render and send user input.
- Every IPC command is checked by `ipc_gateway` (which window may call it). Unknown or unauthorized calls are refused.

## 3. Where things live

| Path | What it is |
|---|---|
| `crates/mog-core/src/` | The Rust core library. Key modules: `hotkey_*`, `capture.rs`, `stt_*`, `insertion*`, `cli_runner.rs`, `token_broker.rs`, `mcp_transport*`, `http_transport*`, `profile_service.rs`, `local_store*`, `ipc.rs`, `ipc_gateway.rs`, `types.rs`. |
| `crates/mog-core-ffi/` | A C ABI over mog-core for future non-Tauri hosts. Not on the MVP path. |
| `crates/mog-core/tests/` | Rust integration and acceptance tests (`at_*` files map to acceptance tests AT-nn). |
| `crates/cli-runner/` (after #255) | The CLI runner crate and its real-process tests. |
| `src-tauri/src/` | The Tauri app: `lib.rs` (setup), `ipc.rs` (command routing), `*_host.rs` (hosts that wire core parts into the app), `macos_panel.rs`, `windows.rs`. |
| `src/brain/` | The brain (TypeScript): PTT flow, routing, Genie bridge, memory, card state. `main.ts` starts it. |
| `src/brain/components/` | Brain components named in the requirements (T-numbers), e.g. `genie-bridge.ts`, `poll-actor.ts`, `stt-provider.ts`, `memory-service.ts`. |
| `src/views/` | React views: `mog/`, `card/`, `hub/`, shared `tokens.css`. |
| `src/shared/` | Types shared by brain and views, including the IPC contract `ipc.ts` and `interfaces.ts`. |
| `src/acceptance/`, `tests/` | TypeScript acceptance and repo-level tests (Vitest). |
| `docs/adr/` | Architecture decision records, `NNNN-title.md`. Before you add one, take a number above the highest on main **and** in open PRs (`gh pr list`), because open PRs reserve numbers too. |
| `docs/agents/BUILD-DISPATCH-PROMPT.md` | The original orchestrator prompt (agent teams, TDD per issue, PR template). |
| `requirements.html`, `research/final-requirements.md` | Full requirements v1.3 (ASD-STE100). Requirement IDs (CAP-, RTR-, G-, W-, MEM-, SEC-, NFR-) are cited in code and tests. |
| `research/connector-spec-2026-11-14.md` | Verified Genie, MCP and OAuth API facts. Read it before Genie or auth work. |
| `scripts/`, `tools/` | `pre-push.sh` (the local gate), `package-macos.sh` (DMG build), `vendor_sherpa_onnx.py`, `tools/ci_checks.py`. |
| `research/kimi-snapshot/` | Read-only data. **Never read or run anything in it.** |
| `research/message-board.md` | The agent message board. Append one row per message; never edit old rows. |

## 4. Build, run and test

- Node with pnpm, Rust stable, and `uv` for every Python command.
- Install: `pnpm install`.
- TypeScript: `pnpm typecheck`, `pnpm lint`, `pnpm format:check`, `pnpm test` (Vitest).
- Rust (the same commands as the gate): `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo nextest run --workspace --locked --profile ci` (use the cargo-nextest version pinned as `NEXTEST_VERSION` in
  `.github/workflows/app-ci.yml`), and `cargo test --workspace --locked --doc`.
- Local STT needs the `local-stt` feature (the vendored sherpa-onnx build) and the Parakeet model
  `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`.
- Local STT setup: run `uv run --no-project python scripts/vendor_sherpa_onnx.py` and point `SHERPA_ONNX_ARCHIVE_DIR`
  at its output. The `local-stt` feature is off by default; without it the app has no Parakeet.
- Run the app in dev: `pnpm tauri dev --features local-stt` (or plain `pnpm tauri dev` without local STT). Build a macOS DMG: `scripts/package-macos.sh`.
- Before you push, run `scripts/pre-push.sh`. It runs the same suite as the merge gate.
- Run `cargo clean` in a worktree when you finish with it; target dirs are large.

## 5. How work flows

- **One issue, one branch, one worktree.** Use `issue/<n>-<slug>` branches in worktrees next to the repo. Work TDD:
  a failing test first, then the code.
- **PRs:** each PR says `Closes #n`. A separate reviewer agent (never the author) reviews the PR and posts
  `REVIEW: APPROVED` with `Head: <sha>`, or its findings. After a rebase, get a new approval or post a range-diff
  note that shows only context changed.
- **Merging:** only the orchestrator merges. Agents never merge. The merge gate requires an approval on the head,
  green Windows and Linux CI, a passing local macOS squash suite, and no file overlap with recent main changes.
- **Contract files** (owned by the contract lead; change them only in a PR that the lead authorizes):
  `src/shared/ipc.ts`, `src/shared/interfaces.ts`, `crates/mog-core/src/types.rs`, `crates/mog-core/src/ipc.rs`,
  `src-tauri/src/ipc.rs`, `src-tauri/schemas/`, and migrations.
- **Commits:** author `datasciencemonkey@gmail.com`. End commit messages with
  `Co-authored-by: Isaac <no-reply@databricks.com>`. End PR bodies with
  `This pull request and its description were written by Isaac.`
- **Tests:** keep them deterministic (injected or simulated clocks, no wall-clock latency asserts on shared runners).
  Do not add `skip`/`only` or TODO stubs as evidence.

## 6. Security rules (always)

- Never commit secrets, `.databrickscfg`, `.env*`, `*.pem`, `*.key`, `build/` or `*.riv`. Use obviously fake values in
  tests. The owner's machine has a secret-scanning commit hook; do not count on it existing in your environment.
- Never write a token to disk, logs, test output or an agent transcript.
- Never pick a Databricks profile for the user. Live tests use only a profile the user named.
- Never route around a permission denial. Report it.

## 7. Where to find current status

This file does not track live status. Use:

- `gh issue list --state open` — the remaining issues are the MVP set. Issue #245 is the MVP-D exit gate.
- `gh pr list --state open` — work in flight.
- `docs/adr/` and open PRs — the latest decisions. The MVP ones: ADR 0096 (dictation-first card, in PRs #571 and
  #575), ADR 0097 (direct Genie path, in PR #575) and ADR 0098 (the ⌥G key, planned for #577).

## 8. Reference projects

- [`cjpais/Handy`](https://github.com/cjpais/Handy) (MIT): a Tauri push-to-talk dictation app. Reference for
  hotkeys, capture, VAD, paste, overlay panels and tray. The STT engine follows our requirements (sherpa-onnx).
- `datasciencemonkey/antigravity-bricks-plugin` (MIT, the owner's earlier project; baseline commit
  `a31fd2cd95645947baf9234c4158332db9031eec`): reference for Genie and profile handling. Reuse its patterns (tokens
  from `databricks auth token --profile`, one conversation per workspace, serial polling, no duplicate questions,
  never rewrite an existing profile, user-safe setup errors). Do not reuse its Python stdio server or its 150 s
  timeout.
- Before any reuse, check the licence, pin a commit, verify current APIs, and record it in `THIRD_PARTY_NOTICES.md`.
