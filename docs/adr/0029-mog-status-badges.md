# ADR 0029: MOG status badges

- Status: Accepted
- Date: 2026-11-16
- Issue: #50
- Owner: shell-lead (implemented by impl-50)

## Context

Issue #50 adds the MOG badges: the hover badge "<profile> · <backend label>" (MOG-07), the STT badge (UX-38), and the hop, offline and sync badges (UX-39a). A badge must not change the mood (UX-39b). The brain owns the logic and the views only render (ARC-2, ADR 0004). The inputs come from areas that do not exist yet. The 4.7 `MoodMapper` overlays have `offline` and `hop`, but no field for the identity, the STT provider, the hop scope or the sync state. The contract files do not change in this issue (#285 owns the move of the view payload types).

## Decisions

1. **A sibling pure function, not a mood overlay.** `mapBadges(sources)` in `src/brain/components/mog-badges.ts` returns a list of badges. Each badge has `kind`, `text`, an accessible `label` and `hoverOnly`. It takes no mood source and returns no mood. `mapMood` takes none of its inputs except the lifecycle and the hop that it already had. So a badge input cannot change the mood by construction. Tests vary the badge inputs over every mood and check that the mood stays the same.
2. **The brain writes the text.** The view renders `text` and uses `label` as the accessible name. So the badge copy has one owner, and no state is colour only (UX-33). A badge holds profile names, hosts and model names only. It never holds a token.
3. **Hover badges and state badges.** The identity and STT badges are `hoverOnly` (MOG-07, UX-38 "on hover"). The hop, offline and sync badges show while their state applies (UX-39a, Appendix E chip 20 "while the condition lasts"). The view keeps a local hover flag from the pointer events of the whole MOG view, so moving onto a badge keeps the hover badges open. Real hover on the non-activating macOS panel is an SP-01 follow-up (`TODO(#10)`, ADR 0009). The hover badges show for the pointer only. Keyboard access to the MOG is #97's work; the STT badge is also in the card footer.
4. **Texts.**
   - Identity: "<profile> · <backend label>". The label is "Genie One", "Genie: <agent>" or "<app name>" (UX-15). With no display name, it is "Genie agent" or "Custom agent", never a space ID or an app URL. During a hop, the profile is the hop target, and the backend is Genie One unless the hop names its own backend (SC-5 step 3). MOG-07 names the host, so the host is a visible second line (`detail`) and is also in the accessible label.
   - STT: "STT: local" or "STT: Databricks <model>" ("STT: Databricks" with no model name yet), from the provider of the last `Transcript`. Before the first transcript, the selected provider is used. With none, local is used, because local is the default (CAP-57c). The accessible label of local says "on this device" (CAP-63).
   - Hop: "Hop: <target> (this question)" or "(this session)" (W-11a: the scope shows on the badge).
   - Offline: "Offline". Sync: "Sync paused", only when sync is paused (MEM-24b, v1).
5. **Order and layout:** identity, STT, hop, offline, sync. The badges sit in one row at the foot of the MOG window, over the mascot (`.badges` in `mog.css`). They have no fill, because the MOG page paints no background (MOG-01a). A border and a text shadow keep them readable on any desktop.
6. **Card footer.** `createCardStack` gets `setSttBadge(text)`. The view-local `CardUpdate` gets an optional `sttBadge`, and the card view shows it in a `<footer>`. The view-local `MogViewState` gets an optional `badges` list. Both are optional view-local fields, the same as the #85 precedent. #285 moves them to the contract.
7. **The 4.7 offline overlay stays for old payloads.** When a `mog:view_state` has a `badges` list, the list is complete and the view renders only it. A payload with no list (the 4.7 form) still shows the `offline` overlay text. So the text never shows twice.
8. **Badge actions (Appendix E).** The STT badge (chip 21) has `action: 'open_data_disclosures'` and the Sync paused badge (chip 20) has `action: 'open_setup'`. The view renders a badge with an action as a button. A click sends `view_emit` `click` with `{ target: 'badge', badge: <kind> }`; the brain picks what to open. The routing of that input waits for the brain's `view_emit` handler (`TODO(#285)`).
9. **Offline does not end the agent mood (UX-39b).** ADR 0018 ends an agent `set_mood` when a lifecycle state starts after it. A move between `ready` and `offline` is a badge change, not a mood row (4.8), so it no longer ends the agent mood. Other lifecycle states (paused, dozing, login required, starting, stopped) still end it.

## Inputs not built yet

`BadgeSources` is typed now and filled with fixtures in tests: profile and host `TODO(#17)`, backend and its name `TODO(#25)`, STT provider `TODO(#52)`, hop target and scope `TODO(#164)`, lifecycle `TODO(#98)`, sync `TODO(#195)`. The brain does not send `mog:view_state` with badges yet; that wiring comes with these producers and #285.

## Notes

- CAP-63 says the STT badge shows "On this device". UX-38 says "STT: local". This ADR shows "STT: local" as the text and "on this device" in the accessible label.
