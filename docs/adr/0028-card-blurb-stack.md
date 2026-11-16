# ADR 0028: The card blurb stack: slots, splits, pins and auto-collapse

- Status: Accepted
- Date: 2026-11-16
- Issue: #85 (depends on #78, ADR 0022; ADR 0004 thin views)
- Owner: shell-lead (implemented by impl-85)

## Context

UX-11a, UX-11b, UX-11c and UX-12 set the rules for the blurbs on the card. The card shows 3 blurbs or less, newest on top, and older blurbs go to History with "N more in History". A split answer takes one slot. A pinned blurb keeps its slot. A completed blurb with no hover for 60 s collapses (`mog.card.autoHideSec`), and it stays one click away in History. The settings are `mog.card.maxBlurbs` (3) and `mog.card.autoHideSec` (60 s), from Appendix F. The issue left these points open: where the rules live, how many pins are allowed, if a pinned blurb collapses, what "hover" means for a keyboard user, and what happens to a late split part.

## Decisions

1. **The brain owns the stack (ARC-2).** `src/brain/blurb-stack.ts` is a pure class with no timers. The caller gives it the monotonic time (`src/brain/clock.ts`). `src/brain/card-stack.ts` is the controller for each MOG. It sends `card.update` after each change and arms one timer at `nextDeadline()`. The scheduler is injected, so the tests use the sim clock. The card view renders the brain order and sends input only.
2. **An entry is one blurb or one split.** All parts with one `splitGroupId` are one entry with the ID of the group, and they take one slot (UX-11b). A later part joins its entry and pushes nothing out. A part that comes after its group went to History starts a new entry. A split is complete only when all its parts are complete.
3. **`maxBlurbs` is clamped to 1..3.** UX-11a is a MUST of "3 or less", so a setting above 3 shows 3. A bad value falls back to the default.
4. **Pins are capped at `maxBlurbs - 1`.** The card can never show more than `maxBlurbs` entries, and the newest answer always has a slot. With the default of 3, two blurbs can be pinned. A refused pin sends no update, so the pin button stays not pressed. When the stack is full, the oldest entry that is not pinned goes to History.
5. **A pinned entry does not collapse.** A pin means "keep this". After an unpin, the 60 s start again.
6. **Hover includes keyboard focus.** A blurb is "hovered" while the pointer is on it or the keyboard focus is in it. The view sends `view_emit hover {blurbId, on, via: 'pointer' | 'focus'}`. The brain tracks the two sources apart, so a mouse leave does not end a focus. The 60 s start again when the last source ends. This keeps keyboard users equal to pointer users (#96, #97).
7. **History.** A collapsed or pushed-out entry adds one to `historyCount`. The stack keeps the IDs of the newest 100 History entries. The full blurbs are in the local store (G-38a). The "N more in History" button sends `view_emit click {target: 'history'}`. The controller calls `openHistory(mogId)`, which opens the hub History once #103 builds it (`TODO(#103)`). The pin button sends `view_emit click {target: 'pin', blurbId}`, and the brain toggles the pin.
8. **Payload.** The view-local `CardUpdate` in `src/views/view-ipc.ts` gets the optional fields `pinned`, `completed`, `parts` and `historyCount`. Old payloads stay valid. A payload with bad `parts` or a bad `historyCount` is dropped. The type moves to the contract with #285 (`TODO(#285)`). The brain `view_emit` handler that calls `CardStack.input` also comes with #285.

## Consequences

- The settings come from the constructor until the settings store exists (`TODO(#77)`).
- Blurb text is plain text until safe markdown lands (`TODO(#210)`). The line templates are #86, the progress line is #88, and the expanded sections are #89.
- GenieBridge does not produce blurbs yet. The tests use typed fixtures.
