# ADR 0031: The view latency path, its tests on the sim clock, and the key-up hold exception

- Status: Accepted
- Date: 2026-11-16
- Issue: #13 (depends on #9 and #12; the capture lifecycle #14 is open)
- Owner: shell-lead (implemented by impl-13)

## Context

UX-05b: the MOG shows `listening` less than 100 ms after key-down, when the mic is open (p95).
UX-36: the view renders the UI frame of each pipeline step 100 ms or less after its event reaches
the brain window (p95). Both rows name AT-21, which runs on reference devices. The path crosses
the core (hotkey, capture), the brain window and the MOG window, and the brain did not send the
mapped mood to `mog:view_state` yet (ADR 0018). The capture lifecycle (#14) is not built. The
issue left open how to test the path without a device, and how the two windows share a clock.

## Decisions

1. **One brain step from mood to view.** `src/brain/mood-publisher.ts` runs `mapMood`, then
   `holdMood`, and calls `send` in the same task: no await, no timer and no hold stand between
   the event and the send for `listening`. A change that the UX-05a hold defers is sent when the
   hold ends, on an injected scheduler. The same mood and overlays are not sent twice (UX-09a:
   an idle MOG gets no writes). `startBrain` does not build it yet: `TODO(#14)`.
2. **Key-up skips the hold too (amends ADR 0018 decision 3).** CAP-44a makes a hold shorter than
   250 ms a tap. A press of 250-400 ms ends inside the 400 ms `listening` hold, so `processing`
   showed up to about 150 ms after key-up. That fails NFR-01c (every sample within 100 ms) and
   UX-36. `listening` -> `processing` is the user ending the capture, a direct action like
   key-down, so it shows at once. `processing` holds 400 ms like any mood, and `listening` still
   holds 400 ms before any other change (a tap's `idle`, an `error`). Cost: on a 250-400 ms press,
   `listening` shows for less than 400 ms, so UX-05a does not hold for that case. The rows with a
   measured limit win, as in ADR 0018.
3. **`listening` keys on the lease, not on the first audio frame.** NFR-02 lets the mic go live up
   to 150 ms (p95) after key-down. If `listening` waited for audio, UX-05b could not hold. So
   "the mic is open" means the lease opened: CaptureService sends `capture:state` with
   `recording: true` when the lease starts, before `mic_live`. This is a rule for #14.
4. **p95 allocations of the hops.** Key-down to the brain 10 ms (8.1 step 0), capture lease start
   to `capture:state` in the brain 30 ms, brain `emit_to` to the view 30 ms, then at most one
   60 Hz frame (16.7 ms). Worst case 86.7 ms < 100 ms. The step path (UX-36) is the last two:
   46.7 ms. The device run (#355) checks the real numbers against these.
5. **Tests on the sim clock.** `src/test-support/sim-scheduler.ts` runs timers and 60 Hz frames on
   the #229 `SimClock`. `src/acceptance/at-21-view-latency.test.tsx` runs the real `BrainBus`,
   `onPtt`, the publisher, `sendMogViewState`, `MogView` and the frame probe, with a capture double
   (`TODO(#14)`) and each hop at its allocation. No test reads wall time, so CI cannot flake on it.
6. **The trace and the frame probe.** With the build flag `VITE_BOTTLE_LATENCY_TRACE=1`, each
   `mog.view_state` has `trace { seq, eventAt, sentAt, keyDownAt? }` (numbers only, SEC-16b). The
   times use `performance.timeOrigin + performance.now()` (`epochNow`), because each window has
   its own `performance.now()` origin. The MOG view's `FrameProbe` notes when it applied the state
   and when the next `requestAnimationFrame` ran, and reports both rows. A person reads
   `__bottleLatency.toJSON()` in Web Inspector. The flag is off in a normal build: no trace, no
   probe. The `trace` field must be in the contract payload: noted on #285.
7. **The row rules match ADR 0016.** Nearest-rank percentiles, 20 samples or more, and a state
   whose frame never ran counts as over the limit (`src/shared/latency.ts`).
8. **A step inside a UX-05a hold is late on purpose.** For example an `error` 100 ms after a
   `success`. The trace keeps the time the event reached the brain, so the probe shows the delay
   instead of hiding it. With the 8.1 timings (local p50 0.63 s to Inserted) such steps are rare.

## Consequences

- AT-21 for UX-05b and UX-36 needs the device run #355, after #14 wires the publisher.
- The trace starts when key-down reaches the brain. The key-down to brain part is NFR-08b
  (`mog_core::latency_bench`). #355 also films key to pixels at 240 fps.
- The card view (chip, blurb) and the pill frames are not traced yet. They follow the same
  pattern when the pill (#297) and the card pipeline land.
