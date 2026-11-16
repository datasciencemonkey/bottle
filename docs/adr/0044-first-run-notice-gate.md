# ADR 0044: The first-run notice gate on capture

- Status: accepted
- Issue: #27 (MB-08a, MB-08b, AT-24)
- Date: 2026-11-16

## Context

MB-08a says the hub must show the versioned legal notice before the first capture. MB-08b says a
new notice version must show again before the next capture. Section 9.3 gives the notice flow and
says ConsentService stores the record (version, time, user, device). Section 4.3 puts
ConsentService (T12) in the brain.

The issue did not say where the record lives, how the hub and the brain exchange the notice, or
where the gate sits. On main, the R2 CaptureService core (#4) is in `mog-core`, but
`capture_start` is not routed to it and returns `not_implemented`, and no command reaches the local store. The contract has no brain-to-hub event
and no hub-level view input. The final legal text is not in the requirements.

## Decisions

1. **The gate is in the brain.** Only the brain window may call `capture_start` (4.5). Only
   `src/brain/capture-gate.ts` calls it, and a test checks this. `startCapture` asks T12 for the
   notice state. Before acceptance it sends no command, sends the notice to the hub and rejects
   with the contract error `{ error: 'consent_required' }`.
   The capture lifecycle (#14, #4) must start capture through `startCapture`.
2. **T12 holds the record.** `acceptNotice(version)` stores
   `{ kind: 'notice', version, at, user, device }`. It refuses a version that is not the current
   one. `user` is null until a profile is signed in. `device` is a random ID, made once.
   A corrupt store value reads as "not accepted": the gate fails closed.
3. **Storage.** The record goes through the `ConsentStore` port. The default adapter is the Web
   Storage of the app origin (key `bottle.consent.notice`). It is local to the device and the OS
   user, and it holds no content. It is not private to the brain: the hub, MOG and card views use
   the same origin and share this Web Storage. Code in a view could write the record without a
   Continue click. The views run only bundled code (CSP `script-src 'self'`, Genie content is
   plain text), which limits this risk. A key in the same origin cannot fix it, so #63 must move
   the record behind the Rust local store before MVP-D. #63 moves the records to the local store, with the SEC-32a
   fields and the 13-month purge. #25 and #26 (MOG config) are not used: they are per MOG, and the
   notice is per app.
4. **Hub and brain.** Until contract issue #393 lands, the brain sends `hub:notice`
   `{ version, accepted }` to the `hub` window only (ARC-5). The hub sends `view_emit` kind
   `click` with mogId `hub` and payload `notice.ready` (on open) or `notice.accept` with the
   version (on Continue). The brain takes these actions only from the `hub` window. The names live
   in `src/shared/hub-notice.ts`, marked `TODO(#393)`.
5. **The hub view.** While the notice is not accepted, the hub shows only the title, the Labs
   label and the notice. The notice is a labelled dialog that takes focus (ADR 0022 decision 3).
   It shows "Before you start", the version, the copy, the 9.3 checkbox and Continue. Continue
   stays off until the box is checked. The notice closes only when the brain reports the record.
   A new version shows a fresh, unchecked notice.
6. **Notice text.** The copy in `LEGAL_NOTICE` follows the 9.3 step 2 items. It is a draft and
   is not approved legal text. While `LEGAL_NOTICE.draft` is true, the notice shows
   "Draft notice. This text is not yet approved." #394 (`needs:human`, `needs:admin`) gets the final text. A text
   change increases `NOTICE_VERSION` by 1.

## Consequences

- Each user sees the notice once per version. Capture stays blocked until the user accepts it.
- When #393 lands, the event and the actions move into `src/shared/ipc.ts` with no change in
  behaviour.
- The window commands (#285) cannot open the hub. #393 asks for a way to open and focus the hub
  when the gate blocks.
- The Web Storage record is lost if the user deletes the app data. Then the notice shows again,
  which is the safe result.
