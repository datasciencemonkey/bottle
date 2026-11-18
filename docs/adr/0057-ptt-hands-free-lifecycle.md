# ADR 0057: The PTT and hands-free capture lifecycle

- Status: Accepted
- Date: 2026-11-16
- Issue: #14
- Owner: impl-14 for capture-lead

Drafted to STE100 rules.

## Context

Issue #14 connects the talk key to R2 CaptureService. R2 `CaptureCore` (#4) has leases, the 120 s hold cap and the 5 min hands-free cap. Section 4.5 permits only the brain to call `capture_start`, and ADR 0044 puts the consent gate in the brain. The capture lead asked us to put the timing rules in `CaptureCore`. The issue left these choices open: the double-tap measure, the start of the VAD silence, the place of each rule, and what Esc does with the audio.

## Decisions

1. **The split.** R1 decides the key rules: a tap (CAP-44a) and a double-tap (CAP-14a). R2 `CaptureCore` owns the timing rules: the post-roll, the tap discard, the hands-free stops and the VAD end. The brain module `src/brain/capture-lifecycle.ts` maps the brain events to the R2 calls. It starts every capture through the consent gate (`startCapture`).
2. **Double-tap.** R1 sends `hotkey.ptt {phase: 'double_tap'}` after the second of two taps. The two key-downs must be 400 ms or less apart (`DOUBLE_TAP_MAX_GAP`). This is the usual double-click measure. A PTT hold or a cancelled press between the taps resets the rule. A third tap starts a new pair.
3. **Post-roll.** On key-up the brain calls `release`. The lease stays live for 120 ms (`POST_ROLL_MS`), then stops with the reason `released`. The stop time is a `tick()` deadline, the same as the caps. A key-down in the post-roll gets `mic_busy` from R2. The brain records nothing for that press and calls `onRefused` with the MOG and the mode. The refused press stays until its key-up or cancel. Thus its key-up sends nothing, and a tap still stops hands-free (decision 5).
4. **Tap discard.** On `cancel` (a tap, or another key under CAP-44c) the brain calls `discard`. R2 stops the lease at once with the reason `discarded`. `StopReason::discards_audio` is true for `discarded` and `escape`. The segment pipeline (#116) drops the audio of those leases. The brain also marks the lease, so its segments never go to STT.
5. **Hands-free stops.** A tap on a press that began during hands-free stops hands-free with the reason `tap`. The audio of the hands-free lease is kept. Esc stops hands-free with the reason `escape`, and the audio is dropped, because Esc is a cancel action. A click on the listening MOG stops hands-free when `mog.capture.clickTogglesListening` is on. A `double_tap` that comes from the tap that stopped hands-free does not start it again. Esc comes from the card of the hands-free MOG as `view:input` `cancel`, because the card is the only key window (UX-19a). Today the brain times the tap from its own `down` and `cancel` times, because the host drops R1's `HotkeyOutput::Tap`. Thus a CAP-44c cancel under 250 ms during hands-free stops hands-free. Contract change #444 carries R1's decision to the brain.
6. **VAD end.** `CaptureCore::voice_activity(speech)` takes the VAD state. The silence counts only after the first speech in the lease. Thus a user who thinks before speaking keeps the mic, and the 5 min cap still applies. After `vadEndMs` of silence (default 1,500 ms, 0 = off, per MOG through `set_vad_end_ms`) the lease stops with the reason `vad_end`. While a PTT holds the mic over hands-free, the VAD end waits. After the PTT, the silence counts again from its end.
7. **One lease at a time.** R2 refuses a start while another start is pending, or while a lease other than hands-free is live (ARC-F9). The discard of a tap waits for the grant of its lease. Thus the brain starts hands-free on `double_tap` only after the discard of the second tap has settled. A key-down in that wait cancels the hands-free start, because R1 then starts a new pair.
8. **The utterance hook.** The brain sends each kept final segment to an STT step, then to `onUtterance({ mogId, text, forceAsk, segment, mode })`. `forceAsk` comes from `shift` on the key-down (RTR-17a, #424). An empty STT text sends nothing. A segment ref that is not final, or a repeat of a segment ID, goes to no STT call. Thus the hook fires once for each final segment.

## Consequences

- `TODO(#444)`: the `hotkey.ptt` `cancel` carries R1's tap decision.
- Done in #425 (ADR 0055): the host routes `capture_*` to `CaptureCore` and calls `tick()` at each deadline. The post-roll arm after `release` waits for #433.
- `TODO(#433)`: `capture_stop` needs an `end` argument for `release`, `discard`, `tap` and `escape`, and `MogViewState` needs a timer field. Until then the IPC adapter sends a plain stop.
- `TODO(#434)`: the macOS tap does not report Esc, and the host does not stop hands-free on the Windows Esc yet.
- `TODO(#116)`: the Silero VAD calls `voice_activity`. `TODO(#46)`: the STT step.
- `routeUtterance` plugs in at `onUtterance` in `startBrain` through `createUtteranceHandler` (`src/brain/utterance-wiring.ts`, #59, ADR 0053).
- Handy (`managers/audio.rs`, `stop_recording`, extra recording buffer) is the pattern for the post-roll. No Handy code is copied.
