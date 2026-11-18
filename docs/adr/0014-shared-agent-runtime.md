# ADR 0014: One shared agent runtime per harness kind, a session per MOG, and a loopback model proxy

- Status: Accepted (impl-112, 2026-11-16). Live parts pending SP-19 (#111).
- Issue: #112. Requirements: RTR-21a, RTR-22, RTR-23, ARC-7, RTR-28, ARC-F14, U11. Acceptance: AT-42.

## Context

U11 and RTR-21a require one shared agent runtime (one sidecar process) with one main-agent session for each active MOG. RTR-22 requires two harness kinds: Claude and Codex. The requirements do not say how one sidecar runs both kinds. SP-19 (#111) has not run, so the runtime program and its protocol are not known.

## Decision

1. `SharedAgentHost` (R16, `crates/mog-core/src/agent_host.rs`) starts at most one runtime process for each harness kind in use. All MOGs that use Claude share one process. All MOGs that use Codex share one process. If every MOG uses one kind, there is exactly one runtime process. Three MOGs on Claude give one process and three sessions (AT-42).
2. Each MOG gets its own session: its own `agentSessionId`, its own per-session secret and its own turns. Sessions share no conversation state.
3. The host talks to the runtime with JSON lines: `open_session`, `turn` and `close_session` on stdin; `session_failed` on stdout. End of stdout means the process exited. SP-19 fixes the real protocol (`TODO(#111)`).
4. ARC-F14: a failed session restarts once with a new secret and the same `agentSessionId`. The other sessions get no frame. A second failure marks the session `Failed`, revokes its secret and makes `send` return `agent_unavailable`, so the brain routes with the fallback classifier (RTR-15b). If the runtime process exits, the host starts it again once and reopens its sessions. A second exit fails all sessions of that kind. Events from an older process are ignored.
5. The runtime gets a clean environment: `env_clear`, then an allowlist of non-secret names (`PATH`, temp and locale names, the Windows start names), then the loopback base URL. A value that looks like a token is dropped. The user's `HOME` and `USERPROFILE` are not passed. The runtime gets a Bottle-owned scratch home (`RuntimeConfig::home`), so it cannot find `~/.databrickscfg` or the CLI token cache through its home. The session secret goes on stdin, not in the environment or the arguments (ARC-7, RTR-28).
6. Locking: the host sends turns and close frames, stops processes and sends status reports after it releases its state lock. Open frames are queued under the lock. `RuntimeProcess::send_frame` must not block. `StdRuntimeLauncher` writes frames from a writer thread through a queue of 64 frames. If one write takes longer than the write deadline (10 s by default), the runtime does not answer (ARC-F14). A watchdog then stops it, and the host gets `Exited` and applies item 4.
   - On unix the runtime leads its own process group. A stop kills the whole group, so a wrapper (`npx`, `uv run`, a shell shim) and its children stop too. The group signal goes out only once, before the child is reaped; a later stop does nothing, so no signal can reach a reused PID. On Windows only the direct child stops (`TODO(#111)`: a job object).
   - `Exited` goes out exactly once: from the reader at end of output, or from the watchdog after its kill, even if an escaped process still holds the pipes.
   - A full frame queue gives `agent_unavailable` at once.
   - The model target lookup runs outside the host lock, and only when the MOG has no live session. `launch` runs under the lock and must return quickly.
   - `TODO(#111)`: a runtime that takes frames but never answers needs an answer or heartbeat deadline. SP-19 defines it with the sidecar protocol.
7. The loopback model proxy binds `127.0.0.1:0`:
   - Claude: `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>/anthropic`. The proxy forwards `POST /v1/messages` and `/v1/messages/count_tokens` to `https://<host>/ai-gateway/anthropic/...`.
   - Codex: `OPENAI_BASE_URL=http://127.0.0.1:<port>/openai/v1`. The proxy forwards `POST /chat/completions` and `/responses` to `https://<host>/ai-gateway/mlflow/v1/...` (the Unity Gateway route of NFR-21a and D57).
   - A local model target must be on loopback and gets no token.
   - The proxy refuses an unknown secret (401) before it reads a token. It forwards no other path (404), so no Page, workspace or other Databricks endpoint is reachable through it. It drops the inbound credential and all headers except `content-type`, `accept`, `anthropic-version` and `anthropic-beta`. It adds the token, lent by R6 `TokenBroker::with_token`, on the outbound hop only.
   - The proxy refuses a request with a `Host` header that does not name its own listener (403, DNS rebinding). It refuses control characters in the request line and headers, folded headers, and a repeated `Host`, `Content-Length`, `Transfer-Encoding`, `x-api-key` or `Authorization` header (400). It refuses `Transfer-Encoding` (411) and bodies over 8 MiB (413).
   - It serves 32 connections or fewer at the same time (503 for more, without a wait). The whole request head must arrive in 10 s. An error reply waits 200 ms in total for the rest of the request. After the secret check, the read timeout is 120 s.
   - A gateway host must be a DNS name with two or more labels. IP literals and single-label names such as `localhost` are refused. The R4 host allowlist (`TODO(#207)`) also checks that the host is a Databricks workspace host.
   - The proxy writes no log. Its error bodies are fixed strings.
8. Per-session secrets and session IDs come from the OS random source (`getrandom` 0.3, already in `Cargo.lock` through Tauri).

## Consequences

- Overlap with #224 (PR #308): #308 adds a shared `loopback` listener with a per-session secret and a Host check for `bottle-mcp`, the Genie One MCP proxy and the model proxy. This PR keeps its own `ModelProxy` and `SessionSecret`. #308 merged while this PR was in review. Follow-up #325 moves `ModelProxy` onto the `loopback` listener and its secret, with the same tests. The agent lead and the security lead must agree on this.
- Residual risk (`TODO(#111)`): the scratch home stops the harness from reading the user's CLI config through `HOME`. A harness with a shell or file tool can still run the `databricks` CLI from `PATH` with an absolute config path, or read files by absolute path. SP-19 and #141 must turn off or limit the shell and file tools of the harness, and the AT-42 process scan must check for this.

- If SP-19 picks one SDK sidecar that hosts both kinds, `RuntimeConfigs` can point both kinds at the same program. The host then needs one map key instead of two. No caller changes.
- The real outbound hop (`Upstream`) with the R4 host policy, and streamed (SSE) answers, are `TODO(#111)`. Until then `UnavailableUpstream` answers 502.
- The model target resolver (`ModelTargets`: home profile and host of a MOG) is a narrow interface with a test double. The real one needs R5 CliRunner (#16) and R6 TokenBroker (#18).
- IPC wiring of `agent_start`, `agent_stop` and `agent_send` in `mog_core::ipc::handle` is a contract change: #307.
- The live first-tool-call latency (NFR-03d, 2.0 s or less at p95) and "usage rows show the user" (RTR-23) need SP-19 and a live workspace. They are pending on #111. The doubles do not measure them.

## Amendment (#113): the `bottle-mcp` server of each session

- Each session gets its own R17 `bottle-mcp` server (`mog_core::bottle_mcp::BottleMcpServer`). It is the `bottle-mcp` handler on a `loopback::LoopbackSession` (#224), so it reuses that listener: `127.0.0.1:0` only, the Host check, a constant-time check of `Authorization: Bearer <secret>`, 32 connections or fewer, a body of 1 MiB or less, and the rule-7 scrubber on every reply.
- The URL (`http://127.0.0.1:<port>/mcp`) and the `Authorization` value go to the runtime child only, in the `open_session` frame on stdin (`bottleMcp: {url, authorization}`). They do not go into the environment: the environment is shared by all the sessions of a runtime, and the secret is per session. No status report, Debug print or log holds them (RTR-25).
- The server speaks minimal MCP JSON-RPC 2.0 on `POST /mcp`: `initialize`, `notifications/initialized` (202), `ping`, `tools/list` and `tools/call`. `tools/list` returns exactly the 7 tools of RTR-26. A call of any other name is error `-32602`, and nothing runs (RTR-29e). Bad arguments are `-32602` too.
- `transcript.search`, `memory.recall` and `memory.write` go through the `LocalTools` seam. `StoreLocalTools` runs the fixed R10 entries `mem.recall.fts` and the atomic group `mem.item.write` (source `agent`; #114, ADR 0089: the item and its embed job in one transaction, with the R10 quarantine check) for the session's MOG only. The local catalog has no transcript statement yet: `TODO(#61)`.
- `show_blurb`, `set_mood`, `propose_page` and `ask_user` go through the `ToolRelay` seam. `BrainRelay` emits `agent.tool_call {mogId, callId, tool, args}` and waits for `agent_tool_result {mogId, callId, result}` for 300 s or less. `propose_page` only proposes; the user decides. `set_mood` refuses `listening`, because the agent cannot open the mic. The app host wiring of the relay and the store is `TODO(#307)`.
- A restart (ARC-F14) closes the old server and starts a new one with a new secret. A stop or a failure closes the server. A waiting UI call sees the stop within 50 ms.
