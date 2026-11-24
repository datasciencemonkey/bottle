# ADR 0118: The talk key during a call recording, and the paste target lock

- Status: accepted (owner decision, 2026-11-23). Contract part only (C-UX1); the behaviour lands in UX slices 2 and 6.
- Date: 2026-11-23
- Owner: ux-multitask (the owner approved the plan on 2026-11-23)
- Amends: ADR 0103 decision 6. Refers to ADR 0109, ADR 0098, ADR 0057 and ADR 0048.

## Context

ADR 0103 decision 6 refuses the talk key during a recording. So the user cannot dictate or ask Genie during a
call. A pause would lose the other side of the call. If the mic stays in the recording, private requests go into
the call notes.

Today the paste goes to the app that has the focus at paste time. If the user changes apps during the hold, the
text goes to the wrong app.

## Decisions

### The talk key during a recording

1. The talk key works during a recording, as push-to-talk only.
2. Hands-free still answers `recording_in_progress` during a recording.
3. A press while the recording starts or stops still answers `recording_in_progress`.
4. The recording keeps the mic. The talk key reads the same stream. Bottle opens no second device.
5. Rust closes a mic gate in the recording tap at the talk-key down, before the brain hears the press.
6. The gate stays closed while the key is down or a talk-key lease is live, and for 250 ms after.
7. While the gate is closed, the recording gets silence for the mic. The call audio continues.
8. Only Rust opens or closes the gate. The brain cannot open it.
9. The recording never pauses or stops because of a hold.
10. Rust records each gated range. It writes the ranges to `job.json` with each part (`heldRanges`).
11. Each gated range of 1 s or more gets the transcript line "You spoke to Bottle (m:ss)" at its start time.
    `transcript.json` gets `bottleGaps: [{startS, durS}]`.
12. No marker text and no gated mic audio go to the transcribe or notes model.
13. The MOG shows "Call only" while the key is held during a recording.
14. The first hold in a recording shows the note "Your call can still hear you." once.
15. `RecordingSettings.heldNoteShown` keeps the note from showing again. A settings file from before this ADR
    reads as `false`.

### The paste target lock

16. `insert_text` takes the `leaseId` of the capture that made the text.
17. At the talk-key down, or the hands-free start, Rust records the app that has the focus. R2 binds that
    record to the lease at grant. With no record, the paste fails closed.
18. If the focused app at paste time is not that app, `insert_text` answers `target_changed`.
19. A `target_changed` answer sends no key event. Before the clipboard write, it writes nothing.
    During the settle wait, Rust restores the clipboard by the ADR 0048 rules.
20. An unknown or expired lease, or a lease of another MOG, also answers `target_changed`.
21. The app record never crosses IPC and never goes into a log.

## Contract (C-UX1)

- `insert_text` args: `{ mogId, text, leaseId }`. The reply `result` adds `target_changed`.
- `RecordingSettings.heldNoteShown: boolean`. `RecordingSettingsPatch` accepts it.
- No new command, event, error code or migration.
- File formats (not IPC): `job.json` `heldRanges`, `transcript.json` `bottleGaps`.

Until UX slice 2 lands, Rust accepts the `leaseId` and does not check it yet.

## Consequences

- Bottle requests stay out of the recording file, the transcript and the notes.
- The call app still hears the user. Bottle cannot mute the call.
- The recording knob, the consent notice and the Rust knob check do not change.
- The mic closes when the recording and the hold both end.
- A user who changes apps during a hold gets Copy instead of a paste. This is the safe direction.
- A downgrade to a build from before this ADR cannot read a settings file with `heldNoteShown`. It reads the
  recording settings as the defaults (knob off, consent asked again). This fails safe. The release notes say so.
