# ADR 0069: R8 McpTransport is a blocking thin JSON-RPC client with a strict tool allowlist

- Status: accepted
- Issue: #30 (G-04a, G-04b, G-04c, G-04d, G-05a, G-05b, G-06a, G-06b, G-07a)
- Date: 2026-11-17
- Owner: impl-30 (agent-lead)

## Context

R8 calls Genie One MCP for the brain (`mcp_call`). The requirements fix the protocol rules
(streamable HTTP, `Mcp-Session-Id`, 2025-06-18, a fresh token per request, the 404 rules and the
tool allowlist). The SP-09 spike (#29) proved the design offline on async `reqwest`, but its live
part needs a person (`needs:human`). The issue left these points open:

1. Sync or async, and how R8 gets the token and the host.
2. What a session is, and who starts it when callers run at the same time.
3. How strict the allowlist is, and what R8 does with `meta`.
4. Which upstream failures are errors and which reach the brain as a reply.
5. The size and time caps.

## Decisions

1. R8 is `ReqwestMcpTransport` on blocking `reqwest` with `rustls`, the same client stack as R7
   (ADR 0046). There is no `rmcp` (G-04a). The seams are `ProfileHosts` (the token host),
   `McpEndpoints` (the Genie One URL; production: `CliTokenBroker::genie_one_endpoint`, so `?o=`
   is always there), `TokenBroker`, `TokenEgressPolicy`, `RequestDecorator` and `McpLog`.
   R8 checks that the egress policy lets the token of the profile host go to the endpoint URL. Then
   it asks R6 for a token for that host on each HTTP request (G-04d, 4.6 rule 2). R6 checks the
   host again (ADR 0066).
2. A session is one (profile, MOG ID from `tags.mog`) pair. The first call runs `initialize`
   and `notifications/initialized` under the session lock, so concurrent first calls share one
   `initialize`. Later calls send `Mcp-Session-Id` and `MCP-Protocol-Version: 2025-06-18`. A 404
   with a session clears it (only if no other caller made a new one) and the call runs again one
   time (G-05a). A 404 without a session, or a second 404, is `genie_one_off` (G-05b).
   `genie_one_off_message(host)` gives the card text.
3. The allowlist is a table of the four Genie tools with their argument names, from
   `genie_tools.json` (a contract test checks the table against `src/test-support/genie-tools.json`).
   R8 refuses any other tool name (exact match) and any argument set outside the contract before
   it looks up a host or lends a token. `profile` keys are removed first (rule 5). `tools/list`
   keeps only the listed allowlisted tools, in allowlist order. `_meta` is built, never forwarded:
   only `warehouse_id` (1 to 64 letters or digits), only on `genie_ask` (G-06b). `progressToken`
   and every other key are dropped (G-07a).
4. Errors (content-free codes): `tool_not_allowed`, `invalid_tool_args`, `invalid_meta`,
   `profile_invalid`, `host_refused`, `redirect_refused` (no redirect is followed), `auth_expired`
   (a second 401; the first drops that lease and resends once on the same session),
   `genie_one_off`, `rate_limited` (429, not retried), `non_json_reply` (a page is never echoed or
   retried), `reply_too_large`, `timeout`, `offline`, `bad_reply`, `protocol_unsupported`.
   A JSON-RPC `error`, a result with `isError`, and a non-2xx reply with a JSON body reach the
   brain as a reply with `isError: true`, `httpStatus` and the scrubbed `rpcError`. The brain
   classifies them with the GL-5 matrix (G-14, G-15a). R8 never sends a `tools/call` again after a
   busy or other tool error (G-15b).
5. Caps (`McpLimits`): 8 MiB per reply, 10 s to connect, 60 s per call by default, and a
   per-call `timeoutMs` that is capped at 120 s. An SSE reply is read only until the response with
   the request ID arrives.
6. Logs: one line per HTTP exchange with fixed words and the status
   (`mcp tools/call attempt=1 status=200`) and one line per refusal (`mcp refused code=..`). No
   token, profile, host, session ID, tool name from the caller, or request or reply content.
   `Debug` of the transport, `McpCall` and `McpReply` shows only counts and status.

## Consequences

- `mcp_call` runs through `McpCallComponent` (a `GatewayComponents`). The Tauri host routes it
  there in #467 (a contract-change issue for `src-tauri/src/ipc.rs`).
- The live checks (the real `tools/list`, the busy envelope, a token rotation mid-session) wait for
  the SP-09 live run: `TODO(#29)`.
- MCP calls do not use the R7 rate bucket yet (G-09a). Amended by ADR 0075 (#138): with
  `with_rate_limit`, R8 waits in the shared bucket and sends an ask again after a 429. The in-flight poll guard (G-07b) is the
  brain `PollActor` plus #31. The agent loopback proxy is #139.
