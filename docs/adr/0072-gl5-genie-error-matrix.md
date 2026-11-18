# ADR 0072: The GL-5 Genie error matrix is an ordered rule table in the brain

- Status: accepted
- Issue: #32 (G-14, G-15a, G-15b; also the G-05b card text)
- Date: 2026-11-17
- Owner: impl-32 (agent-lead-2)

## Context

G-14 tells the bridge to classify a Genie One MCP failure from three sources with the GL-5
matrix: the HTTP status, the JSON-RPC error and the `isError` text. The requirements name the
matrix but do not list its rows. These points were open:

1. The rows, and their order when two sources disagree.
2. Where the user text comes from.
3. Which failures retry without a user action.
4. How an R8 failure code (ADR 0069 decision 4) reaches the mapper.

Inputs: ARC-F2, ARC-F3, ARC-F4 and ARC-F6, Table W-ERR, the Appendix E texts and failure action
map, the D56 live busy text, and the status rules of `genie_bridge.py` in
antigravity-bricks-plugin @ a31fd2c (MIT).

## Decisions

1. `src/brain/components/genie-errors.ts` holds `GL5_MATRIX`, an ordered table of rows. Each row
   has an ID, a source, a `BackendErrorCode` and a rule. The first row that matches wins:
   - `busy-text`: "stream closed before response.created" in any source, any case →
     `CONVERSATION_BUSY` (G-15a). It is first because the live busy reply comes with HTTP 200.
   - HTTP status: 401 → `AUTH_EXPIRED`; 403 with IP access list text → `NETWORK_BLOCKED`;
     other 403 → `NO_GRANT`; 404 → `GENIE_ONE_OFF` (G-05b, R8 has already initialised again
     once for a 404 with a session); 409 → `CONVERSATION_BUSY`; 429 → `RATE_LIMITED`;
     504 → `ASK_UNCERTAIN` (ARC-F4, a timeout after the send); other 5xx → `GENIE_FAILED`;
     other non-2xx → `CLIENT_BUG`.
   - JSON-RPC: the protocol codes -32700, -32600, -32601 and -32602 → `CLIENT_BUG`. The
     Databricks `error_code`: `UNAUTHENTICATED` → `AUTH_EXPIRED`; `PERMISSION_DENIED` →
     `NO_GRANT`; `REQUEST_LIMIT_EXCEEDED`, `RESOURCE_EXHAUSTED` and `RATE_LIMIT_EXCEEDED*` →
     `RATE_LIMITED`; `FEATURE_DISABLED` → `GENIE_ONE_OFF`.
   - Text: IP access list → `NETWORK_BLOCKED`; guardrail or policy block → `POLICY_DENIED`;
     rate limit or "too many requests" → `RATE_LIMITED`; any other error text or code →
     `GENIE_FAILED`.
   - `no-signal`: no error at all → `CLIENT_BUG`.
   The HTTP status wins over the JSON-RPC error and the text, except for the busy text.
2. The user text is a fixed Bottle text per code (`userTextFor`). Upstream text is read only to
   classify, then dropped: it never goes into a text, an event or a log (W-14, SEC-05a). The
   texts come from Appendix E where it has one ("Genie is still answering your last question
   here.", "Genie busy, retrying in N s", "I do not know if Genie got your question.", "A
   workspace policy blocked this question."). `GENIE_ONE_OFF` is "Genie One isn't available in
   <host>." (G-05b), the same text as `genie_one_off_message` in R8. The host goes into a text
   only if the caller passes it and it is a plain host name; else the text says "this
   workspace". `GenieBridge.mapError` has no host input in 4.7, so it uses "this workspace".
3. Actions follow the Appendix E failure action map. Only `RATE_LIMITED` has `autoRetry`, and
   it keeps a numeric `retry_after` (ARC-F3); the rate bucket (#138) runs the countdown. Every
   other code waits for a user action. `retry` (the failed `AskEvent` field) is `relogin` for
   `AUTH_EXPIRED`, `without_preamble` for `POLICY_DENIED`, `user` for codes with a retry
   action, and `none` for the rest.
4. `sendGenieAsk` sends one `genie_ask` and never sends it again for any failure (G-15b). R8
   already resends once after a 401 and after a 404 with a session. A thrown `IpcError` code
   goes through `classifyTransportCode`, which maps every ADR 0069 code (for example
   `genie_one_off`, `non_json_reply` → `PROXY_PAGE`, `timeout` → `ASK_UNCERTAIN`).
5. Gap: the Tauri host turns a code outside `ipc_gateway::ERROR_CODES` into `host_failed`, so
   the R8 codes do not reach the brain yet. Contract-change #479 adds them. Until it lands,
   `host_failed` maps to `GENIE_FAILED`. No contract file changes in #32.

## Consequences

- A new live text is a new row and a new test double. The live busy envelope and the policy
  text wait for the SP-09 live run: `TODO(#29)`.
- `submit` (#33) and the ARC-F2 conversation queue (#148) build on `sendGenieAsk` and
  `toFailedEvent`. Polling (#31) can use `classifyMcpReply` for a failed poll.
- W-14 (#42) can reuse `userTextFor` for the Table W-ERR texts.
