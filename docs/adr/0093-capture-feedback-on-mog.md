# ADR 0093: The MOG mood follows capture, and a refused start shows its reason

- Status: Accepted
- Date: 2026-11-18
- Issue: #551 (UX-01a, UX-05a, UX-05b, UX-07, UX-06a, UX-06b, CAP-16a, MB-08a, CAP-17b, MOG-06a, MOG-06d). Covers task 2 of #457. Refs #4, #60, #552.
- Owner: capture-lead-3 (implemented by impl-demo-ptt)

## Context

On `main`, nothing sent `mog.view_state`. `startBrain` built T11 but did not build the mood publisher (`src/brain/mood-publisher.ts`). So the MOG never showed `listening`, and the level meter looked dead. A refused capture start (`onRefused`) was silent. Every dictation went to Copy (`insertionOn: () => false`).

The issue does not say which MOGs get a mood, how a refusal looks on the MOG, or how long it shows.

## Decisions

1. **One module.** `src/brain/capture-mood.ts` feeds each `capture:state` to one mood publisher. It updates every MOG that a capture state (`owner`, `directedTo`) or a refusal has named. Until #98 and #39, every MOG is `ready` and has no request phases, so a stop goes back to `idle` after the 400 ms hold.
2. **A refusal is a failed request.** The MOG shows `error` with the attention cue for 3 s (4.8), and one badge `capture_refused` with a fixed text and label. The badge goes with the error window. A granted capture of the same MOG ends it at once.
3. **Reasons.** `consent_required`, `consent_storage_failed`, `mic_busy` and `host_failed` come from the `IpcError` code. Any other rejection is `host_failed`. A `host_failed` becomes `permission_denied` or `no_device` when `capture:state.mic` reports that fault, before or after the refusal (CAP-17b). The texts hold no content: no transcript, device name or error message.
4. **Consent.** The consent gate still shows the hub notice on `consent_required` (MB-08a). A failed consent records read now rejects with `consent_storage_failed` and starts nothing.
5. **Badges in the publisher.** `MoodPublisher.update` takes optional badges and sends them in `mog.view_state`. A change of badges alone sends a new state. Badges never change the mood (ADR 0029).
6. **Level source.** The MOG view's default level source is `viewMicLevels()` (the `mog.level` frames of `LevelRouter`). `NO_MIC_LEVELS` is removed.
7. **Insertion.** `insertionOn` is `true`. `insert_text` (#418) checks Accessibility before it touches the clipboard and answers `no_permission`. The route turns that, or a throw, into `insert_failed` with the text, so the dictation goes to Copy and is not lost. A prompt-free brain read of the permission (#552) is parked for the Settings UI.

## Consequences

- The user sees `listening` on key-down, the level meter moves, and a refused press shows why.
- TODO(#39): `processing` after the key-up needs the request phases.
- TODO(#60): the Appendix F `app.dictation.insert` setting.
