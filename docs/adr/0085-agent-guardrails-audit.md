# ADR 0085: Main-agent guardrails, tool decisions and the agent action log

- Status: accepted
- Issue: #141 (RTR-03c, RTR-21b, RTR-29a, RTR-29b, RTR-29c, RTR-29d, RTR-30, AT-42, F5 step 4, B4, 9.2)
- Date: 2026-11-18
- Owner: impl-141 (agent-lead-2)

## Context

The main agent decides when to call Genie One, the memory tools, the Page tool and the UI tools
(RTR-21b). It must treat transcript and memory text as data (RTR-03c). It must never start
capture, approve outbound context, publish a Page or switch the workspace without a user action
(RTR-29a..d). Each agent action must go to the content-free audit log (RTR-30, SEC-23).
`bottle-mcp` (#113) already exposes no forbidden tool (RTR-29e). The audit log is #140 (ADR 0059).
The Genie One proxy is #139 (ADR 0083). The issue left these points open:

1. Which component writes the agent action entry, and when.
2. What an entry holds.
3. How the core stops the agent from sending outbound context to Genie with no approval.
4. Where the system prompt and the tool policy live.
5. How an agent hop proposal reaches the user.
6. What the brain does with a UI tool call.

## Decisions

1. **The core writes the entry, before the action runs.** `bottle-mcp` writes one entry for
   each call of one of its 7 tools. The Genie One proxy writes `agent_action:genie_one` for each
   allowlisted Genie call. A `GenieProxy` with no guard uses `NoActionLog`, which refuses every
   entry, so an unguarded proxy sends nothing. `BottleMcpServer::start` gives each proxy the
   session's log and approvals. Both write before any argument check or send, so a refused call is in
   the log too (it is a decision of the agent). A name that is not a tool runs nothing and has no
   entry: the closed kind set has no place for it. If the entry cannot be written, nothing runs,
   and the agent gets `audit_unavailable` (fail closed). The brain writes no agent entry.
2. **An entry holds the `mogId`, `agent_action:<tool>`, a new UUID `opId`, the wall-clock time
   and `args_bytes`.** No text and no hash (the #140 rule for agent actions). The time comes from
   a `WallClock` seam. Until the app host wires the one audit log of the process (`TODO(#307)`),
   `BottleMcpDeps::new` uses an in-memory `AuditLog` (the #140 store until #442).
3. **Outbound approval binds to the turn question.** The brain sends an `AgentTurn` only after
   the user lets the preview go (B1, #142). `SharedAgentHost::send` stores the SHA-256 of the
   turn `question` as the one approved outbound text of that MOG (`OutboundApprovals`). The
   agent's `genie_ask` may send only that exact text, once. Any other text is refused with
   `context_not_approved`, before the ask ledger and the target lookup. So the agent cannot add
   memory or transcript text (a preamble) to a question, and injected text cannot start a new
   ask. The proxy uses the approval when it checks the ask. If the target lookup or the ask
   ledger (G-11a..c) refuses the ask before any send, the proxy gives the approval back
   (`restore`), and a restore never replaces a newer approval. A new turn replaces the approval. A session stop or a failed send clears it. A context
   send with a preamble goes only through the brain ContextGate approval (#152).
4. **The prompt and the policy are core constants.** `agent_guard::MAIN_AGENT_SYSTEM_PROMPT`
   and `tool_policy()` go to the harness in the `open_session` frame (`systemPrompt`,
   `toolPolicy`). No user text joins the prompt: the turn goes as JSON data in its own frame.
   The policy lists the 2 MCP servers, the 7 tools and `builtinTools: none`. SP-19 maps it to
   each harness (`TODO(#111)`).
5. **A hop proposal becomes the `switch_profile` chip.** The router step is the agent's first
   step (B4). Its `hop` outcome (#144) goes to `HopChips.offer`, which shows
   "Switch to <profile>?" (Appendix E, 30 s). Only `HopChips.click` calls `profile_select`. An
   offer, an expiry, a dismiss or an agent tool call never does. The agent gets no hop tool:
   RTR-29e forbids one, and text that asks for a switch is data.
6. **The brain checks each UI tool call again.** `CoreAgentBridge.onTool` takes only the 4 UI
   tools, checks the arguments with the `bottle-mcp` bounds and passes the text to the handler
   unchanged. The bridge holds no capture, context, Page create or publish, Genie or profile
   handle. `propose_page` returns only the review ID. `handleToolCall` answers with
   `agent_tool_result`. The handlers are a seam that the app host wires (`TODO(#307)`).

## Consequences

- The agent cannot rephrase a question for Genie. A follow-up is a new user turn. This is
  strict, and it matches B1: nothing goes to Genie without a user action.
- The AT-42 adversarial set (`crates/mog-core/tests/fixtures/at_42_adversarial.json`) drives the
  Rust and TS tests. With doubles, the tests show that the core and the brain do not act on the
  text. Whether a live model obeys the text needs the live harness run (`TODO(#111)`, SP-19).
- An audit store failure stops the agent's tools. This is a clear failure, not a silent gap in
  the security log.
