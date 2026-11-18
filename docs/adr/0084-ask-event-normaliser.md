# ADR 0084: The GL-1 normaliser ends each request in one terminal AskEvent

- Status: accepted
- Issue: #33 (G-21a, G-21b, G-22, G-23a, G-23b; AT-09)
- Date: 2026-11-18
- Owner: impl-33 (agent-lead-2)
- Amends: ADR 0073 decision 5 (a `rate_limited` poll), ADR 0072 decision 3 (the countdown text)

## Context

G-21a and G-21b tell each backend to emit only the normalised GL-1 events, with one terminal
event for each request. G-22 adds `needs_input(clarify)`, and G-23a and G-23b set the card for
status `incomplete`. The PollActor (#31, ADR 0073) and the GL-5 matrix (#32, ADR 0072) are on
main. These points were open:

1. Which AskEvents are terminal.
2. Where the normaliser runs, and how `ask`, `resume` and `submit` share it.
3. What the bridge sends for a request with a context preamble before the ContextGate (#152).
4. What the brain does after R8 stops an ask on RATE_LIMITED (ADR 0075).
5. What a poll that R8 refuses with `rate_limited` does.
6. Where the clarify follow-up state lives, and when it ends.
7. How the card shows each terminal event.

Inputs: section 4.7, Appendix E, ADR 0072, ADR 0073, ADR 0075, and the `conversation_in` rule
of `genie_bridge.py` in antigravity-bricks-plugin @ a31fd2c (MIT).

## Decisions

1. **Terminal events.** These events end a request: `answer`, `failed`,
   `needs_input(clarify)`, `uncertain`, `abandoned` and `cancel_requested`. An `approval` or a
   `choose_profile` input comes before the ask, so it does not end the request.
2. **One normaliser.** `src/brain/components/ask-events.ts` holds the normaliser.
   - `pollEvents` turns the PollActor updates into `progress` events and one terminal event.
     `ask` and `resume` both use it.
   - `oneTerminal` passes events up to the first terminal event and drops the rest. A stream
     that ends with no terminal event ends in `failed STREAM_LOST`.
   - `ask` and `submit` both use `oneTerminal` and `faultsAsFailed`. A thrown fault becomes one
     `failed` event. The event has no upstream text.
3. **The outcome rules.**
   - `completed` with an answer of less than 400 chars that ends with "?" and has no query item
     from any poll gives `needs_input(clarify)`. The prompt is the answer and the options are
     empty. TODO(#29): SP-09 records the live clarify signal.
   - `completed` with an answer or a query item gives `answer`.
   - `completed` with no answer and no query item gives `failed GENIE_FAILED`.
   - `incomplete` with an answer gives `answer{incomplete:true}`.
   - `incomplete` with no answer, or with only whitespace, gives `failed GENIE_INCOMPLETE`.
   - `failed` gives `failed GENIE_FAILED`. An error reply or an R8 refusal goes through GL-5.
   - The give-up gives `abandoned`. An aborted signal gives `cancel_requested`.
     TODO(#151): the bridge sends `genie_cancel_response`.
   - Key numbers are the bold spans of the answer that hold a digit. The normaliser copies them
     verbatim, 3 or less (UX-14a). The query items of all polls are merged by item ID.
4. **No unapproved preamble.** `submit` does not send a request that has a preamble. It gives
   `failed TOOL_BLOCKED` until the ContextGate approval binds the payload. TODO(#152).
5. **No second countdown.** R8 runs the RATE_LIMITED countdown and the 5 tries of an ask
   (ADR 0075). Thus a `rate_limited` ask that reaches the brain is final:
   - The event is `failed RATE_LIMITED` with `retry: 'user'`.
   - The text is "Genie is busy. Try again in a minute." The text has no countdown.
   - The card offers [Retry]. The brain never asks again.
   - The countdown text of `userTextFor` shows 300 s or less. This is the review follow-up of
     #32. R8 holds a profile for 300 s or less.
6. **A rate-limited poll polls again.** R8 does not send a poll again after a 429. The PollActor
   thus treats `rate_limited` like `timeout` and `poll_inflight`. It waits one more delay and
   polls again with the same IDs. The next call waits in the halved bucket. An `isError` poll
   reply that GL-5 maps to RATE_LIMITED also polls again. The give-up still stops the loop. Thus
   only an ask ends in RATE_LIMITED. Genie did not take that question, so [Retry] makes no
   duplicate question.
7. **Clarify follow-up.** `ClarifyFollowUps` keeps one open clarify for each MOG, with its
   conversation and profile.
   - The next `submit` of the MOG uses up the clarify.
   - If that submit is on the same profile within 5 min and has no conversation ID, it goes to
     the clarify conversation.
   - A submit on another profile does not use the clarify conversation: the conversation stays in
     its workspace (G-27b, W-12a).
   - The bridge never sends a follow-up on its own. Only the user's next utterance uses it (B1).
8. **Card render.** `src/brain/outcome-blurb.ts` maps each terminal event to the Appendix E
   lines and controls. It reads only the event, never the backend kind (G-21a).
   - Completed shows the answer and [Open in Databricks].
   - Partial answer shows "Partial answer: <answer>", "Genie did not finish all steps." and
     [Ask again].
   - Failed shows the GL-5 user text, "Code <code>" and the failure action map.
   - Clarify shows "Genie asks: <question>", [Answer by voice] and [Type].
   - Gave up, Ask uncertain and Cancelled use their own rows.
   - A `transport:poll_inflight` busy shows only [Wait], because [New conversation] would ask the
     question again (ARC-F1).
   - TODO(#86): the line cuts, the bold key numbers and the footer.
9. **Audit.** An accepted ask writes one content-free `ask` entry through the
   `AuditLogClient` of ADR 0059 (the `audit_append` IPC client by default). The entry has the question size and the SHA-256 of the sent
   text. A failed audit write does not stop the ask.
10. **G-21a inspection.** A test reads the views and the mascot, card and memory modules. These
    files must not name a backend kind. The Forget code of #123 (`memory-service.ts`) keeps the
    backend of a conversation as data (`ExchangeRef['backend']`). It checks the value with the
    bridge's `isBackendKind`, and it never names a kind. The per-backend delete stays in
    `forget-flow.ts`, beside the Genie One delete utility.

## Consequences

- `GenieBridge.submit` sends an ask only when its caller reads the events. The caller calls it
  only after a user action. The 1.5 s preview gate is #142.
- The OperationTracker states (#39), the conversation pins (#145) and the exchange row (#40)
  build on these events.
- The host of a profile comes from an injected `hostOf` seam until ProfileService lists
  profiles (TODO(#17)). Without it, the texts say "this workspace".
- The `audit_append` call does not carry the hashes yet: TODO(#442).
