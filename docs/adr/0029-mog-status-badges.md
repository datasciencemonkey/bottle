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
3. **Hover badges and state badges.** The identity and STT badges are `hoverOnly` (MOG-07, UX-38 "on hover"). The hop, offline and sync badges show while their state applies (UX-39a, Appendix E chip 20 "while the condition lasts"). The view keeps a local hover flag from its pointer events. Real hover on the non-activating macOS panel is an SP-01 follow-up (`TODO(#10)`, ADR 0009).
4. **Texts.**
   - Identity: "<profile> · <backend label>". The label is "Genie One", "Genie: <agent>" or "<app name>" (UX-15). With no display name, it is "Genie agent" or "Custom agent", never a space ID or an app URL. During a hop, the profile is the hop target (SC-5 step 3). The host is in the accessible label, because MOG-07 names the host.
   - STT: "STT: local" or "STT: Databricks <model>", from the provider of the last `Transcript`. Before the first transcript, the selected provider is used. With none, local is used, because local is the default (CAP-57c). The accessible label of local says "on this device" (CAP-63).
   - Hop: "Hop: <target> (this question)" or "(this session)" (W-11a: the scope shows on the badge).
   - Offline: "Offline". Sync: "Sync paused", only when sync is paused (MEM-24b, v1).
5. **Order:** identity, STT, hop, offline, sync.
6. **Card footer.** `createCardStack` gets `setSttBadge(text)`. The view-local `CardUpdate` gets an optional `sttBadge`, and the card view shows it in a `<footer>`. The view-local `MogViewState` gets an optional `badges` list. Both are optional view-local fields, the same as the #85 precedent. #285 moves them to the contract.
7. **The 4.7 offline overlay stays.** The MOG view shows the old `offline` overlay text only when the brain sends no offline badge, so the text never shows twice.

## Inputs not built yet

`BadgeSources` is typed now and filled with fixtures in tests: profile and host `TODO(#17)`, backend and its name `TODO(#25)`, STT provider `TODO(#52)`, hop target and scope `TODO(#164)`, lifecycle `TODO(#98)`, sync `TODO(#195)`. The brain does not send `mog:view_state` with badges yet; that wiring comes with these producers and #285.

## Notes

- Offline is a lifecycle state in region A. In `mapMood`, a lifecycle change after an agent `set_mood` ends the agent mood (ADR 0018). The orthogonality test keeps the same `since` for offline. When #98 adds the lifecycle, an offline change must not restart the agent mood, so that offline stays a badge only.
- CAP-63 says the STT badge shows "On this device". UX-38 says "STT: local". This ADR shows "STT: local" as the text and "on this device" in the accessible label.
