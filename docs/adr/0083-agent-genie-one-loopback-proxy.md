# ADR 0083: The main agent reaches Genie One through a per-session R8 loopback proxy

- Status: accepted
- Issue: #139 (RTR-27, RTR-28, ARC-6, ARC-7, R8, 4.6 rules 5, 7 and 9, ARC-F1, AT-42)
- Date: 2026-11-18
- Owner: impl-139 (agent-lead-2)

## Context

The main agent decides when to call Genie One MCP (RTR-21b). The harness process must never
hold a Databricks token (RTR-28). The Rust core must add the token and apply the bridge rules to
each agent call (RTR-27, ARC-7). R8 (`ReqwestMcpTransport`, ADR 0069) already applies the
transport rules to the brain's `mcp_call`. The rate bucket is ADR 0075 and the poll guard is
ADR 0073. Each agent session already has one `LoopbackSession` with one secret and three
listeners (ADR 0013). `bottle-mcp` serves one listener. The `genie-one-mcp` listener answered
501. The issue left these points open:

1. Where the proxy lives, and which R8 instance it uses.
2. Who picks the profile and the warehouse of an agent call.
3. Which of G-04a..G-20b the proxy applies, and which stay in the brain.
4. What the harness gets back when a call fails.
5. How the proxy stops when its session stops.
6. How a test can show that the harness holds no token (AT-42).

## Decisions

1. **One proxy per session, on the session's `genie-one-mcp` listener.** The handler is
   `mcp_transport::proxy::GenieProxy`. The `bottle-mcp` server of the session serves it when
   `BottleMcpDeps::with_genie_proxy` is set. The proxy uses the same per-session secret as
   `bottle-mcp` (one secret per session, ADR 0013). `SharedAgentHost` sends
   `genieOneMcp {url, authorization}` in the `open_session` frame on stdin, never in the
   environment. The proxy sends each call to the ONE R8 of the process. That is the instance that
   serves the brain's `mcp_call`, so agent calls and brain calls share the R8 sessions, the rate
   bucket and the poll guard. `McpTransport::start_loopback_proxy` (a #139 stub) is removed:
   the proxy belongs to an agent session, not to R8.
2. **The session decides; the agent never does.** A `GenieTargets` seam gives the profile and
   the warehouse of the session's MOG, for each call. So a user-approved workspace change
   (RTR-29d) applies to the next call. The proxy drops every `profile` key (4.6 rule 5). It never
   reads the agent's `_meta` or `tags`. `_meta.warehouse_id` comes from the target only (G-06b,
   G-07a). The tags are the session's MOG with `op: genie` (SEC-06a). The real resolver is the
   MOG talk target (ADR 0058), wired by the app host: `TODO(#563)`. Until then,
   `UnavailableGenieTargets` sends nothing.
3. **Rules.** The proxy applies the R8 rules through R8: G-04a..G-05b (session, protocol, a fresh
   token for each HTTP request, only to the profile host, the 401 and 404 rules), G-06a (the
   allowlist and the argument contracts), G-07b and ARC-F1 (the poll guard, #31), G-09a..G-10b
   (the bucket) and rule 7 (the scrub). It checks G-06a and G-20b (`item_id` must match
   `^pub_.+_output$`) itself before the target lookup, so a refused call lends no token. It
   also keeps a ledger of the agent's in-flight asks (`mcp_transport::agent_asks`): 2 or less
   for each MOG (G-11a), 4 or less for each profile (G-11b) and 1 for each conversation
   (G-11c). An ask is in flight until a poll gives `completed`, `incomplete` or `failed`, until a
   cancel of its conversation (G-25c), or until it gives up after 10 min (G-12b). After the
   give-up, no poll of that conversation is sent, and no new ask goes to it. An ask whose send
   timed out stays in flight until its give-up (G-16a), so the agent cannot ask again at once. A
   capped call is refused before any send, with a content-free code. The agent sees the code and
   can wait: the card queue of G-11 is a brain rule. The other card and PollActor rules (G-12a,
   G-13a, G-15b, G-16b, G-17, G-18, G-19, G-20a) need the brain's state of the ask. Issue #518
   counts brain asks and agent asks in one cap, and adds the late re-polls and the no-re-ask
   rules for the agent (`TODO(#518)` in the code).
4. **Content-free failures.** A call off the allowlist, or with bad arguments, gets a JSON-RPC
   `-32602` error, and nothing is sent. Any other failure is an MCP tool result with
   `isError: true` and the fixed R8 code as its only text (for example `genie_one_off`,
   `rate_limited`, `poll_inflight`, `ask_limit_mog`, `ask_given_up`, `genie_target_unavailable`). A Genie JSON-RPC error is the
   code `genie_error` with the scrubbed `rpcError`. The proxy answers `initialize`, `ping` and
   notifications itself, so the agent's protocol handshake never reaches Genie One. This
   follows `dispatch`, `tool_result` and `describe` of `genie_bridge.py`. Unlike the plugin,
   the proxy offers no `profile` argument to the agent.
5. **A stop never waits for Genie.** The R8 call runs on its own thread. The request waits for
   it, and checks every 50 ms whether the session stopped. A stop ends the request with
   `session_closed`, so the AgentHost lock is never held for a Genie call (ADR 0014). The R8 call
   ends on its own time cap, and its result is dropped. A poll slot stays held until then,
   which is correct: the poll is still in flight upstream.
6. **AT-42 scan with a real child.** `tests/at_42_harness_scan.rs` starts `SharedAgentHost` with
   the real `StdRuntimeLauncher`. The runtime is the test binary in child mode. The child asks
   and polls through the proxy. Then it scans its environment, its arguments, every byte it got
   (stdin and every reply), and every writable page of its own memory. The scan uses Linux
   `/proc/self/mem`, Windows `VirtualQuery` and `ReadProcessMemory`, and macOS `mach_vm_region`
   and `mach_vm_read_overwrite`. The fake broker lends random `test-token-not-real-<n>` tokens.
   The child gets only their SHA-256 values, and a canary of the same shape that it must find. In a
   manual mutation run, one token put into the child's memory made the scan fail, as it must.

## Consequences

- The harness has one way to Genie One. It carries only the session secret, which opens only
  that session's listeners. R8 adds the token inside the core.
- Agent and brain calls compete for the same per-profile bucket. A 429 on a brain call also slows
  the agent, and the reverse is also true (G-09a).
- An agent poll that overlaps a brain poll of the same conversation fails at once with
  `poll_inflight` (ARC-F1). The agent polls again later.
- Residual risk: a harness with a shell tool can still run the Databricks CLI from `PATH`.
  SP-19 and #141 limit those tools (ADR 0014, `TODO(#111)`). The live harness run of AT-42 is
  `TODO(#111)`, and the live Genie One shapes are `TODO(#29)`.
