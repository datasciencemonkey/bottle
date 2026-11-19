# ADR 0097: UC2 Ask Genie is a direct Genie One MCP call

- Status: accepted (user decision, 2026-11-19)
- Issues: #573 (this build), #557 (Ask Genie press), #142 (ask gate), #16 (R5 CliRunner)
- Date: 2026-11-19
- Owner: agent-lead-4 (implemented by impl-genie-direct)
- Amends: ADR 0086 (the hand-off seam of the ask gate)

## Context

The user decided this on 2026-11-19: keep the Genie path simple. A press on Ask Genie must send
the exact text straight to Genie One MCP. Bottle must use only the parts that are already merged.
The path must not use the main agent or the Agent SDK.

Before this ADR, the ask gate gave the approved question to the main agent through T15
`agent_send` (ADR 0086). The main agent then called `genie_ask`.

## Decision

1. **Direct call.** The ask gate's `send` seam is `createGenieDirectSend` (`src/brain/genie-direct.ts`).
   It calls T8 GenieBridge `submit` with the binding `genie_one_mcp` and the MOG's target profile
   (its home profile). T8 sends one `genie_ask` through the `mcp_call` IPC command. Then T8 polls
   with the PollActor (#483). The main agent takes no part. `AskGateDeps.send(AgentTurn)` does
   not change.
2. **Token.** The host gets the token from R6 TokenBroker (`databricks auth token --profile <p>`,
   #464) over R5 CliRunner (#16). The token stays in the host. The webview never sees it.
3. **Tools.** Only the McpTransport allowlist (#470) is used: `genie_ask`, `genie_poll_response`,
   `genie_get_query_result`, `genie_cancel_response`. Bottle adds no new typed Genie IPC route.
4. **Sanitizer and cap.** The press is the confirmation (B1, ADR 0096). In the gate, the G-47
   sanitizer (`src/brain/question-sanitizer.ts`) runs first. It removes control, zero-width and
   bidi characters (every Unicode Cf and default-ignorable code point, so also the tag
   characters and the variation selectors), code fences, HTML tags and comments, and markdown
   links (the link text stays).
   Then the G-30a cap runs. The card shows the text that goes out. A text that is empty after the
   sanitizer is not sent, and the card shows "Not sent."
5. **Card.** One blurb per request shows on the card of the owner MOG. At the send, it shows the
   Appendix E Acknowledged line "Asking Genie in this workspace (profile <p>)" (G-44). On
   `progress`, it shows "Working… m:ss" (G-17a). On the terminal event, it shows the
   `outcome-blurb.ts` text: the answer, or the fixed GL-5 failure text ("<user_text>" and
   "Code <code>"). An answer also sets the expanded sections of `card-sections.ts` (the answer and
   its key numbers).
6. **No retry.** Bottle never sends a question again by itself (G-15b). The outcome controls
   ([Retry], [Open in Databricks]) do not show yet.
7. **The main agent** (#16, R16, ADR 0014) is only for later open-ended requests. It is not part
   of UC2 Ask Genie.

## Not in this build

- The workspace host in the Acknowledged line (#40). Until then the line says "this workspace".
- The newest progress step with the banned words filtered (G-17b, UX-16a, #87).
- The outcome controls, the footer and the deep link (#86). The table preview (#150).
- The local SQLite history of each exchange (`mem_local_exec` `mem.exchange.upsert`, #455,
  host route #426).

## Consequences

- UC2 needs #16 (a real R5) before an answer comes back. Until then `mcp_call` answers
  `not_implemented`, and the card shows the fixed failure text.
- ADR 0086 stays valid for the preview and the chip. Only its hand-off seam changes.
- Tests: `src/brain/genie-direct.test.ts`, `src/brain/question-sanitizer.test.ts`,
  `src/brain/components/start-brain-genie-direct.test.ts`, and the gate tests in
  `src/brain/ask-gate.test.ts`.
