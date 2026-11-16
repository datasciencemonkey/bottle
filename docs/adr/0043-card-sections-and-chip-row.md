# ADR 0032: The expanded card sections and the chip row rules

- Status: Accepted
- Date: 2026-11-16
- Issue: #89 (depends on #85, ADR 0028; ADR 0004 thin views)
- Owner: shell-lead (implemented by mog-b2)

## Context

UX-18 sets the order of the expanded card sections (Table UX-EXP). UX-20a, UX-20b and UX-20c set the chip rules: 4 chips or less on a blurb, a text label on each chip, and an action only on a click, Enter or Space. Appendix E lists 24 chips with a trigger, an action and an expiry.

## Decisions

1. **The brain builds the sections.** `src/brain/card-sections.ts` has `buildSections(input)`. It returns the sections in the fixed order: answer, key numbers, SQL (collapsed), table preview, "Sent to Genie", "Resend without context", Copy as Markdown or CSV, footer, "Where your data goes". A section with no content is left out. The footer and "Where your data goes" always show. The table preview shows 10 rows and the count of the rest. The copy keeps all rows (`tableToMarkdown`, `tableToCsv`). A CSV cell that starts with `=`, `+`, `@` or a `-` that is not a number gets a `'` first, so a sheet never runs it. The view renders the list in the order it gets.
2. **The brain owns the chip row.** `src/brain/card-chips.ts` has `ChipRow`, pure like `BlurbStack`: the caller gives the time. The row shows the first 4 chips in the order offered. The 5th and later chips are `overflowChips`. The view shows them in the footer of the expanded card, so the UX-EXP order stays fixed.
3. **The catalogue is code.** `CHIP_CATALOGUE` has the 24 rows of Appendix E with the expiry as data: a time limit (`ttlMs`), a count of newer chips (`newer`, Ask Genie: 2 min or 3 newer chips), expiry on open (`onOpen`, Genie finished). A chip with "with the blurb", "with the request" or "while the condition lasts" has no time limit. The caller removes it by `remove` or `removeOwner`.
4. **Which chips show.** The two badges (STT, Sync paused) are not chips: ADR 0029 owns them, and the row refuses them. The v1 chips (Synced to host, Meeting may have ended, Sync paused) show only when the row is made with `v1: true`. A label with no letter or digit (an icon only) is refused (UX-20b). The row never sends a label-less chip to the view.
5. **A chip acts only on a click (UX-20c, B1).** `CardStack.input('chip_click', {chipId})` is the only path to `onChip(action)`. An offer, an expiry, a hover and a focus never call it. An unknown or expired ID does nothing. The view renders each chip as a native `button`, so Enter and Space click it, and it has no hover handler. Chips that send data are marked `sends: true` in the catalogue.
6. **View payload.** The view-local `CardUpdate` gets the optional `overflowChips` and `sections`. A bad `sections` list drops the whole update. With `sections`, the view shows them and not the old `answerMd` and footer. `answerMd` stays for old payloads. Contract move: `TODO(#285)`.

## Consequences

- The answer and the deep link show as plain text. Safe markdown and trusted links come with #210 (`TODO(#210)`): `data-section="answer"` is the integration point.
- Nothing calls `setExpanded`, `offerChip` or `onChip` yet. GenieBridge, the router and the `view_emit` routing come with #86, #88, #285 and the agent runtime. The tests use typed fixtures.
- The click targets `resend` and `copy {format}` reach the brain as `view_emit click`. The handler is #285.
