# ADR 0096: Every utterance is dictation first; a question also gets an Ask Genie chip

- Status: accepted
- Issue: #557 (user decision, 2026-11-18)
- Date: 2026-11-18
- Owner: agent-lead-4
- Amends: ADR 0092 (decision 4, the no-profile pick card), ADR 0086 (when the preview shows)

## Context

The user decided this (relayed by the orchestrator on 2026-11-18): every PTT utterance must be
dictation first. Nothing goes to Genie by itself. A question gets an "Ask Genie" option on the
#559 pop-up. A press sends the exact text. With no profile, the press opens the profile picker.

Before this change, a confident question (0.7 or more) got the 1.5 s preview, which sends by
itself (RTR-04a). A question with no home profile got the pick card of ADR 0092, with
[Pick a profile], [Insert as text] and [Copy]. The text was not pasted.

## Decisions

1. **Dictation first** (`utterance-route.ts`). An unforced PTT or hands-free question is
   inserted at the cursor (or goes to Copy when insertion is off or refused), the same as a
   dictation. Then the route is a `chip` that carries the dictation result in `dictated`.
2. **No auto-routing.** An unforced question never gets the preview, at any confidence. This
   amends RTR-04a and RTR-05: the preview is only for a Shift-forced ask (RTR-17a, RTR-17b),
   which is an explicit user action.
3. **Exact text.** The chip question is the dictated text (`cleaned_text`, else the STT text),
   not the router's rewritten `genie_question`. The pop-up shows it, and a press sends it.
4. **The press is the confirmation** (B1). With no home profile, a press opens the profile
   picker (W-07d) and sends nothing. When the user saves a pick, the gate sends the same text
   once. Bottle never picks. With no home profile, a Shift-forced ask is a chip, so only a
   press can start the picker.
5. **The pick card goes.** The no-profile card of ADR 0092 (decision 4) and [Insert as text]
   are removed: the text is already pasted, or on the Copy pop-up.
6. **Order on the pop-up.** The chip is offered first, then the dictation result goes to the
   Copy sink. A refused paste therefore shows Copy (the newest blurb), so the text is not lost.
   A pasted question shows Ask Genie.
7. RTR-14 still holds: an injection-suspected question is pasted and gets the warning chip only.
   A session utterance (RTR-07) is not dictated into the focused app, as before.

## Consequences

- AT-06 and AT-31 checks of the preview at 0.7 now apply to Shift-forced asks only.
- The Ask Genie chip keeps its 2 min expiry; an expired press sends nothing.
- A pick can come long after the press. The text still sends once, because the press confirmed it.
