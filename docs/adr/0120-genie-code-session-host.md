# ADR 0120: The Genie Code session host (slice 2a, R1)

Superseded by ADR 0122. The session host is removed.

Status: accepted (drafted to STE100 rules; the official dictionary check was not done).

Superseded in part by ADR 0121: the turn text (the question and the combined answer) now also
lives on disk, in the local store, until the user deletes the thread or uses Wipe. The session
host itself is unchanged: GENIE_HOME stays disposable and holds no turn content.

## Context

The MOG agent (ADR 0117 follow-up, plan Amendment 2) runs on one live `genie app-server` per
user. The spike (`bottle-tools/genie-code-spike/REPORT2.md`) shows that app-server, with an
ephemeral thread and `approvalPolicy:"never"`, keeps the context in memory only, and that the
lockdown holds. `exec-server` ignores the sandbox. `exec resume` writes plaintext transcripts.

## Decision

- Rust owns the process. The code is `crates/mog-core/src/agent_run/session/`. One actor thread
  owns the session, the turn queue (1 live, 3 waiting), the deadlines and all state. IPC,
  server lines, the network watch and the timer reach it as messages.
- The session starts on the first turn. GENIE_HOME (`<app local data>/genie-home`, 0700) is
  deleted and written again before each start, and deleted at each end, at quit, at app start and
  on every Wipe scope (`wipe.rs` `AgentWipeHook`, wired in `consent_host.rs`).
- `config.toml` is the golden file `session/golden_config.toml`: approval policy `never`,
  read-only sandbox, 23 feature flags off, memories off, one MCP server (`bottle`, tool
  `ask_genie`, `tool_timeout_sec = 600`, auto-approve on that tool only). The shim secret never
  reaches disk: `env_vars` names the variables, and the values go in the server's environment.
- Lockdown tripwires end the session with `agent_lockdown_violation`: a `thread/start` reply that
  is not ephemeral, not the pinned version, not the `databricks` provider, not `never` or not
  read-only; any server-to-client request (it gets a refusal first); a
  `remoteControl/status/changed` other than `disabled`; an item whose type is not on the allow-list of genie 0.5.0 (`userMessage`,
  `agentMessage`, `reasoning`, `mcpToolCall`); a tool call to a server other than `bottle` (the three `codex` resource
  tools are allowed).
- Deadlines per turn: "Still working…" at 4 min, `turn/interrupt` at 10 min, a kill of the
  process group (the app-server and its children in that group; NOT the MCP shim, which runs in its own group and ends on EOF or when its parent changes, ADR 0119) 5 s after any interrupt that gets no `turn/completed`. An offline turn, or a
  "waiting for network" error, interrupts at once. The session ends after 30 min with no turn.
- A crash fails the live turn. The next waiting turn gets one new session. A second crash in a
  row ends the waiting turns with `failed` and 0 calls, so the brain uses the direct path.
- D1 off taint is per session. A tainted session restarts for the next new (not follow-up)
  question. A change of the agent profile, D1 or the catalog ends the session after the live turn.
- Tokens of a turn are the sum of `last.inputTokens + last.outputTokens` over the
  `thread/tokenUsage/updated` events between `turn/started` and `turn/completed`.
- The binary is found in Bottle's bin folder, on `PATH` or at `~/.local/bin/genie`. Bottle runs it
  only with the pinned version and the pinned SHA-256 of `genie` and `genie-code-mode-host`
  (`pin.rs`). The install source is an open owner question: `genie_code_install` answers
  `install_failed` until `INSTALL_SOURCE` is set.

## Verified locally with the real binary (genie 0.5.0, fake Responses server)

`tools/genie-lockdown/lockdown.py` (local only). Results of this run:

- V2: `env_vars` forwards the named variables to the `bottle` MCP server. A variable that is not
  named does not reach it (the MCP child gets a cleaned environment).
- `apply_patch` is offered, and the sandbox rejects it: "writing is blocked by read-only
  sandbox". No file is created.
- The offered tools: `list_mcp_resources`, `list_mcp_resource_templates`, `read_mcp_resource`,
  `request_user_input`, `apply_patch` and the `mcp__bottle` namespace (`ask_genie`). No shell,
  web search or image tool.
- `request_user_input` is offered. A call to it raised no server request in this run. If a later
  version sends one, the tripwire ends the session.
- During a session the only network contact is the model host. `genie doctor` contacts
  `github.com:443`; Bottle never runs `doctor`.
- `unbounded_connection_retries` is enabled by default. An offline turn would retry for ever, so
  the offline interrupt (not a config key) ends it.
- Not checked: V4 (offline behaviour with a real network cut), V5 (`[otel]` and `[analytics]` keys
  under `--strict-config`, which the app-server does not use), V6 and V8.

## Consequences

- Another genie version needs new fixtures in `crates/mog-core/tests/fixtures/genie-<v>/`, a new
  run of the canary tool and new snapshots before `pin.rs` changes.
- The guard (R2) names the shim variables `BOTTLE_AGENT_SOCK`, `BOTTLE_AGENT_SESSION` and
  `BOTTLE_AGENT_SECRET` (`guard::shim::env_pairs`). The host takes `Arc<dyn ShimProvider>` and
  `Arc<dyn SessionGuard>` from managed state, and `SessionHost::refusal_hook()` goes into
  `AgentGuard::set_refusal_hook`: the third refusal of a turn ends it with outcome `refusals`.
