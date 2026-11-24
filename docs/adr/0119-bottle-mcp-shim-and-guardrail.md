# ADR 0119: The ask_genie guardrail, the stdio shim and their channel

- Status: accepted (slice 2a, lane R2)
- Date: 2026-11-23
- Owner: slice 2a, guard lane
- Changes: ADR 0117 (the pair gate and the fail-closed proxy gate)

## Context

Slice 2a runs one `genie app-server` process for each user. The Genie Code session in it may call one
MCP tool, `ask_genie`. The tool starts a Genie question in another workspace. Genie answers can hold text
that tries to steer the model, and a workspace can be switched off while a turn runs. Rust must hold every
rule, because the model and the shim are not trusted.

## Decisions

### Where the code is

`crates/mog-core/src/agent_run/guard/`:
`ledger.rs` (sessions, turns, call log, stop rules, `SessionGuard`), `ask.rs` (the ten checks),
`genie_poll.rs` (the poll loop), `receipt.rs` (receipts and text blocks), `channel.rs` and `peer.rs` (the
app socket), `shim.rs` (the stdio shim). The channel, peer and shim are `cfg(unix)`.

### One shim for each server process, bound to the session

- Genie Code starts `bottle --agent-mcp-shim` once for each server process. The shim reads
  `BOTTLE_AGENT_SOCK`, `BOTTLE_AGENT_SESSION` and `BOTTLE_AGENT_SECRET` (64 hex digits).
- The socket check order is: peer uid, parent PID of the peer, then `hello{sessionId, secret}`. The parent
  PID must be the app-server PID that the host gave with `bind_pid`. The secret is compared with
  `RunSecret::matches` (constant time). One connection for each session. The secret dies with the session.
  Each failure closes the connection with no reply.
- The socket parent PID is read with `LOCAL_PEERPID` and `proc_pidinfo` on macOS, and `SO_PEERCRED` and
  `/proc/<pid>/stat` on Linux. Another Unix system fails closed.
- The tool schema comes from the guard in the `hello` reply. It has no enum of aliases. The catalog is in the
  prompt, and Rust checks the alias.
- A `cancel` frame (MCP `notifications/cancelled`) removes the response only. The Genie calls end through
  `stop_turn`, because a timeout keeps them polling.
- When the socket ends, the shim sends SIGTERM to its parent and exits, so a Bottle crash leaves no orphan.

### The ten checks, in order

`run_stopped` or `no_active_turn`, `invalid_args`, `workspace_not_available` (not in the catalog),
`cross_workspace_off` (D1 off and a tainted session), `budget_reached` (D2, for each turn, reserved at the
check), the live `WorkspaceGate` (`workspace_not_available`, the reservation goes back), one conversation
for each workspace in a turn (a second call waits as Queued), the send through the one R8 and the poll loop,
the result, and the content-free `AgentActionLog` entry.

- The action log entry is written before the send. No entry, no call (`audit_unavailable`).
  The tool name is `genie_one` (ADR 0085 closed set) and the MOG id is the session id.
- A refused ask does not use the budget. A failed call does.
- The third refusal of a turn stops it. The guard sends the turn id to the hook set with
  `set_refusal_hook`, and refuses every later ask with `run_stopped`. The session host interrupts the turn
  and ends it with `refusals`.
- `AgentAsks` caps (2 for each session, 4 for each profile) make a call wait. They do not refuse it.

### Taint and receipts

- `D1_SUPPORTED = false`. With D1 off a workspace that is not the agent workspace gets a receipt (counts,
  state, no answer text). The agent workspace gets a text block in `<genie_answer>` tags, after the
  quarantine check. A hit is withheld from the model and from the user view.
- The first text that the model reads from the agent workspace taints the session. The taint lasts the
  session, not the turn. A withheld text does not taint.

### The poll loop

A Rust copy of `poll-actor.ts`: Table G-POLL delays, jitter of 10 %, the G-08 floor, a 30 s cap for each poll,
a give-up at 10 min. `timeout`, `poll_inflight` and `rate_limited` poll again. Another error ends the call.
`login_required`, `auth_expired` and `token_unusable` give `signed_out`.

### Stop

`stop_turn(User | Lockdown | Failed | SessionEnded)` cancels each open Genie call with
`genie_cancel_response`. The ready parts stay. `Timeout` and `Offline` keep the open calls polling. A call
that waits for its place never goes out after any stop.

### The pair gate and the proxy gate (ADR 0117 follow-up)

- `ConversationPairs` keeps `(profile, conversation)` of each successful `genie_ask` in memory. It is in
  `McpCallComponent` (brain path) and `GenieProxyDeps` (agent path).
- With a gate, `genie_poll_response`, `genie_get_query_result` and `genie_cancel_response` need a recorded
  pair or a profile that the gate allows. The second case lets PollJournal resume calls after a restart.
- `GenieProxyDeps` fails closed. With no gate every `genie_ask` is refused with `workspace_not_available`.
  The app host passes the `WorkspaceStore` gate (`agent_host::genie_proxy_deps`).

## Consequences

- No question, answer, secret or token is in a `Debug` output, an event or an error of the guard. A canary
  test scans them.
- The conversation for each workspace is for one turn. A follow-up turn asks a new Genie conversation.
- A `Timeout` stop can leave a poll thread that ends after its turn. It writes to the call log only.
- Windows has no shim and no socket (AGENTS.md section 1a). `genie_code_status` reports `unsupported_os`.
