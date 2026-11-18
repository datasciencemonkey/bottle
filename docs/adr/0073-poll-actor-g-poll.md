# ADR 0073: The PollActor runs in the brain, and R8 refuses an overlapping poll

- Status: accepted
- Issue: #31 (G-07a, G-07b, G-07c, G-08, G-16c, G-18a, G-18b; ARC-F1, ARC-F4, ARC-F10, ARC-F12)
- Date: 2026-11-18
- Owner: impl-31 (agent-lead-2)

## Context

T9 ConversationManager has one PollActor for each conversation. The actor polls Genie One MCP
on the Table G-POLL schedule, with the G-08 floor. ARC-F1 says that R8 refuses an overlapping
poll with `E_POLL_INFLIGHT`, also for the calls of the main agent. The issue left these points
open:

1. Where the loop runs, and where the overlap check runs.
2. How the `E_POLL_INFLIGHT` code crosses IPC.
3. Which "previous delay" the x1.5 step uses, and when the jitter and the floor apply.
4. What counts as an in-flight ask for G-08.
5. Which failures poll again, and which failures end the loop.
6. Where the IDs and the profile persist (ARC-F10), and what `MogBackend.resume` does.

## Decisions

1. The loop runs in the brain (`src/brain/components/poll-actor.ts`), because T9 is a brain
   component. Each poll goes through `ConversationManager.enqueuePoll`, so the brain sends one
   poll at a time for each conversation. R8 also checks for an overlap
   (`crates/mog-core/src/mcp_transport/poll.rs`). It is in `ReqwestMcpTransport::call_tool`,
   so every caller gets the check: `mcp_call` from the brain, and the loopback proxy of the
   main agent (#139). The check keys on `conversation_id` only, over all profiles and MOGs.
2. A second `genie_poll_response` on a conversation with an open poll fails at once with
   `CoreError::Unavailable { component: "R8", reason: "poll_inflight" }`. The Rust constant is
   `E_POLL_INFLIGHT`. The check runs after the allowlist and argument checks, and before any
   host lookup, token lend or send. The slot is an RAII guard, so a reply, an error or a panic
   frees it. This PR changes no contract file. Contract-change #479 (PR #495, merged) put the
   R8 codes, `poll_inflight` included, in `ERROR_CODES` and `IPC_ERROR_CODES`. So the Tauri host
   (`src-tauri/src/ipc.rs` `rejection`) sends `poll_inflight` and `timeout` to the brain as
   themselves, not as `host_failed`. The host routing of `mcp_call` to R8 is #467. Until it
   lands, `mcp_call` answers `not_implemented` in the app, and the tests run on the gateway and
   doubles.
3. The delay is computed at the end of the ask or of the previous poll, from the elapsed time
   of the ask at that moment. The base delay is 2 s, 3 s or 5 s by band. After 180 s it is the
   previous base delay x 1.5, at most 15 s. The x1.5 step uses the previous base delay, not the
   jittered delay, so the jitter does not compound. Then the jitter (+-10 %) applies, and then
   the G-08 floor. So the floor can make a delay longer, but never shorter.
4. An in-flight ask is an open PollActor loop on the profile, over the whole brain window. The
   count goes up when `ConversationManager.poll` is called and down when the loop ends, throws
   or is stopped. The actor checks the floor again each time it wakes. So an ask that starts
   during a sleep also delays the next poll.
5. A `timeout` (the 30 s poll cap of ARC-F4, G-16c) or a `poll_inflight` refusal makes the actor
   wait one more delay and poll again with the same IDs. The status `unknown`, and any status
   that the bridge does not know, is not terminal (G-18a, ARC-F12). `completed`, `incomplete`
   and `failed` end the loop. An `isError` reply ends the loop with the reply unchanged. Any other
   refusal ends the loop with that error. No poll starts after 10 min
   (G-12b). The loop then ends with `gave_up`, and `resume` yields the `abandoned` event. The
   soft notice, the `abandoned` exchange status (G-12c) and the late re-polls stay with #149.
   The actor never calls `genie_ask`.
   The GL-5 matrix (#32, `genie-errors.ts`) maps `poll_inflight` to `CONVERSATION_BUSY` with a
   user retry. Only a caller other than the PollActor (for example a manual "Check again") sees
   it, and that caller waits and never asks again.
6. The actor writes `{opId, mogId, profile, host, conversationId, responseId, askedAt}` to a
   `PollJournal` when the loop starts. Only a terminal status removes the entry. A timeout, an
   error, an abort or the give-up keeps it, so a later poll can resume with the same IDs. The
   brain journal is in memory now. The durable journal is the `genie_exchange` row of
   ADR 0061, which already keeps the profile, the host and both IDs. RecoveryManager (#39) reads
   that row after a restart (`TODO(#39)`). The host routing of `mem_local_exec` is #426.
   `GenieOneMcpBackend.resume(ref)` looks up the journal entry for the conversation and response
   IDs. It refuses a ref with no response ID, a ref that is not in the journal, or a ref for
   another profile or host. It never asks again. Then it runs the PollActor and maps each poll
   to `progress`. The terminal answer is mapped in its simplest form; #33 and #87 normalise it.
   `resume` ends the other outcomes in a `failed` event from the GL-5 matrix (#32, ADR 0072,
   `genie-errors.ts`): an `isError` reply goes through `classifyMcpReply`, an R8 refusal code
   goes through `classifyTransportCode`, and a Genie `failed` status is `GENIE_FAILED`. The
   retry of each event is a user action or none, so nothing is asked again automatically.

## Consequences

- Two polls on one conversation never overlap, whoever sends them. A caller that gets
  `poll_inflight` polls again later with the same IDs.
- With 4 in-flight asks on a profile, each poll waits 6 s or more, so the polls stay at
  40 calls/min (AT-34). The shared bucket and AIMD are R7 (#138).
- The `#30` test `g_04b_concurrent_first_calls_share_one_initialize` now uses one conversation
  for each thread, because four open polls on one conversation are an ARC-F1 overlap.
- The "poll again with the same IDs after a timeout" rule comes from the reference plugin
  (`genie_bridge.py` `late_text`, MIT). See `THIRD_PARTY_NOTICES.md`.
- `TODO(#29)`: the SP-09 live run confirms the poll reply shape on a real workspace.

- Amended by ADR 0084 (#33): a poll refused with `rate_limited`, or an `isError` poll reply
  that GL-5 maps to RATE_LIMITED, polls again with the same IDs after the next delay.
