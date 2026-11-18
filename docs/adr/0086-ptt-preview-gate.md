# ADR 0086: The T3 ask gate: the 1.5 s preview and the Ask Genie chip

- Status: Accepted
- Date: 2026-11-18
- Issue: #142 (RTR-04a, RTR-04b, RTR-05; RTR-14, RTR-17b, UX-17, UX-19b, UX-20c; AT-31, AT-06). Depends on #59 (ADR 0053), #57, #117 (ADR 0056), #141.
- Owner: agent-lead-2 (implemented by impl-142)

## Context

Binding rule B1: nothing goes to Genie without a user action. A PTT or hands-free question with confidence 0.7 or more shows the 1.5 s cancelable preview (RTR-04a). When the preview ends with no cancel, the main agent gets the question (RTR-04b). Below 0.7 the card shows an "Ask Genie" chip, and no network call occurs before the click (RTR-05). The #59 route (`src/brain/utterance-route.ts`) returns `preview` or `chip`; `startBrain` only released the owner (TODO(#142)).

The issue does not say how the card shows the preview controls, what [Edit] does without a text box, what cancels a preview other than Esc and [Cancel], what the card shows after a cancel, or where the gate hands the question on.

## Decisions

1. **One module.** `src/brain/ask-gate.ts` has `createAskGate(deps)` and `wireAskGate(bus, gate, stackFor)`. It is the runtime part of T3 CapturePolicy. `CapturePolicy.decide` holds the D30 thresholds as a pure function. `startBrain` wires the gate as the `onAsk` of the utterance hook.
2. **One way out.** The gate calls only `send(turn)`, the hand-off seam. In `startBrain`, `send` is T15 `AgentBridge.send`, so the question goes out as `agent_send`. With #141 the core makes that question the one approved outbound question for the agent's `genie_ask` (RTR-29b). The gate never calls `mcp_call` or `agent_*` itself.
3. **The turn.** `AgentTurn` holds the route question, the router decision, the MOG's target profile and an empty recall list (TODO(#200)). With no target profile the gate sends nothing and shows "Not sent.", because Bottle never picks a profile. In `startBrain` the target is `undefined` until T7 gives it (TODO(#163)); the router input is also `undefined` until then, so no utterance reaches the gate.
4. **Preview.** The blurb is the Appendix E Preview: `"<question>"` and "Sending in 1.5 s". The controls are [Send now] [Edit] [Cancel]. One timer runs on the monotonic clock at the route's `previewMs` (1500). A forced ask (Shift) previews too (RTR-17b).
5. **Controls on the card.** `CardUpdate` has no controls field. The card stack gets template controls (`setControls(owner, controls, onClick)`, `clearControls(owner)`). They go first in `CardUpdate.chips`, before the catalogue chips. A `chip_click` on a control ID runs its handler. A control is not a catalogue chip and does not count in the UX-20a limit of 4. A later contract change can move them to their own field.
6. **Cancels.** These cancel a preview and send nothing: [Cancel]; Esc in the card of the MOG (`view:input` `cancel` from `card-<id>`); any talk-key press (`hotkey:ptt` down), for every MOG, because the user starts a new utterance; `cardClosed(mogId)`; a MOG that is not open when the timer ends (or at a [Send now] or chip click); and a new ask for the same MOG (one preview per MOG). A cancel sets the blurb to `"<question>"` and "Not sent." Appendix E has no template for this state, so the text is a Bottle text with no banned word. The Esc of the card does one thing (UX-19b): it cancels the preview, and the card stays.
7. **Edit.** [Edit] stops the timer and holds the question with [Send now] [Cancel]. A held question never sends by itself. The card has no text box yet (TODO(#526)).
8. **Chip.** A `chip` route offers Appendix E chip 1 (`ask_genie`, "Ask Genie", 2 min or 3 newer) with the question blurb. The chip holds no request owner: the owner is released at once, and the click names the MOG. The gate keeps the offer and refuses a click from another MOG, an unknown ID, a second click, or a click after 2 min or 3 newer offers. The card stack's `onChip` gives the gate the click; the chip row already refuses a click on an expired chip (UX-20c).
9. **Warning.** An `injection_suspected` route shows the fixed warning of ADR 0056 as a control with no handler, for 2 min. A click sends nothing (RTR-14). A `preview` route with a flagged decision also gets the warning, never the preview.
10. **Failure.** A rejected `send` (ARC-F14) sets the blurb to "Not sent." The gate does not retry. TODO(#143): the fallback classifier route of RTR-15b.
11. **Card input.** Until #374 owns all card input, `wireAskGate` routes the card's `cancel` to the gate and its `chip_click` to the card stack of that MOG. The capture lifecycle still gets the same `cancel` for a hands-free stop (CAP-14b).

## Consequences

- The IPC test (`src/brain/ask-gate-wiring.test.ts`) shows no `agent_send` and no `mcp_call` before 1.5 s, after each cancel, or without a chip click.
- `startBrain` makes the card stack of each MOG on first use, in the same block as #498 (PR #504).
- TODO(#374): no card-hide signal reaches the brain yet, so `startBrain` does not call `cardClosed`. Until then a closed MOG is caught by the `isOpen` check when the timer ends.
- The Appendix E "(bar)" of the preview is not drawn: `CardUpdate` has no countdown field. The card view can add it with #526.
- #374: the card's Esc handler (`card-esc.ts`) must treat an open preview as a request in flight, so one Esc cancels it and does not hide the card.
- TODO(#526): the edit text box. TODO(#200): recall hits in the turn. TODO(#163): the target profile.
