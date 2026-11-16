# ADR 0012: One shared agent runtime per harness kind, a session per MOG, and a loopback model proxy

- Status: Accepted (impl-112, 2026-11-16). Live parts pending SP-19 (#111).
- Issue: #112. Requirements: RTR-21a, RTR-22, RTR-23, ARC-7, RTR-28, ARC-F14, U11. Acceptance: AT-42.

## Context

U11 and RTR-21a require one shared agent runtime (one sidecar process) with one main-agent session for each active MOG. RTR-22 requires two harness kinds: Claude and Codex. The requirements do not say how one sidecar runs both kinds. SP-19 (#111) has not run, so the runtime program and its protocol are not known.

## Decision

1. `SharedAgentHost` (R16, `crates/mog-core/src/agent_host.rs`) starts at most one runtime process for each harness kind in use. All MOGs that use Claude share one process. All MOGs that use Codex share one process. If every MOG uses one kind, there is exactly one runtime process. Three MOGs on Claude give one process and three sessions (AT-42).
2. Each MOG gets its own session: its own `agentSessionId`, its own per-session secret and its own turns. Sessions share no conversation state.
3. The host talks to the runtime with JSON lines: `open_session`, `turn` and `close_session` on stdin; `session_failed` on stdout. End of stdout means the process exited. SP-19 fixes the real protocol (`TODO(#111)`).
4. ARC-F14: a failed session restarts once with a new secret and the same `agentSessionId`. The other sessions get no frame. A second failure marks the session `Failed`, revokes its secret and makes `send` return `agent_unavailable`, so the brain routes with the fallback classifier (RTR-15b). If the runtime process exits, the host starts it again once and reopens its sessions. A second exit fails all sessions of that kind. Events from an older process are ignored.
5. The runtime gets a clean environment: `env_clear`, then an allowlist of non-secret names (`PATH`, `HOME`, temp and locale names, the Windows start names), then the loopback base URL. A value that looks like a token is dropped. The session secret goes on stdin, not in the environment or the arguments (ARC-7, RTR-28).
6. The loopback model proxy binds `127.0.0.1:0`:
   - Claude: `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>/anthropic`. The proxy forwards `POST /v1/messages` and `/v1/messages/count_tokens` to `https://<host>/ai-gateway/anthropic/...`.
   - Codex: `OPENAI_BASE_URL=http://127.0.0.1:<port>/openai/v1`. The proxy forwards `POST /chat/completions` and `/responses` to `https://<host>/ai-gateway/mlflow/v1/...` (the Unity Gateway route of NFR-21a and D57).
   - A local model target must be on loopback and gets no token.
   - The proxy refuses an unknown secret (401) before it reads a token. It forwards no other path (404), so no Page, workspace or other Databricks endpoint is reachable through it. It drops the inbound credential and all headers except `content-type`, `accept`, `anthropic-version` and `anthropic-beta`. It adds the token, lent by R6 `TokenBroker::with_token`, on the outbound hop only.
   - The proxy writes no log. Its error bodies are fixed strings.
7. Per-session secrets and session IDs come from the OS random source (`getrandom` 0.3, already in `Cargo.lock` through Tauri).

## Consequences

- If SP-19 picks one SDK sidecar that hosts both kinds, `RuntimeConfigs` can point both kinds at the same program. The host then needs one map key instead of two. No caller changes.
- The real outbound hop (`Upstream`) with the R4 host policy, and streamed (SSE) answers, are `TODO(#111)`. Until then `UnavailableUpstream` answers 502.
- The model target resolver (`ModelTargets`: home profile and host of a MOG) is a narrow interface with a test double. The real one needs R5 CliRunner (#16) and R6 TokenBroker (#18).
- IPC wiring of `agent_start`, `agent_stop` and `agent_send` in `mog_core::ipc::handle` is a contract change: #307.
- The live first-tool-call latency (NFR-03d, 2.0 s or less at p95) and "usage rows show the user" (RTR-23) need SP-19 and a live workspace. They are pending on #111. The doubles do not measure them.
