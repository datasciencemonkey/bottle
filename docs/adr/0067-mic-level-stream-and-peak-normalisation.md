# ADR 0067: The mic level stream and the -3 dBFS peak normalisation

- Status: Accepted
- Date: 2026-11-17
- Issue: #23 (CAP-16a, CAP-16b, CAP-20)
- Owner: impl-23 (capture-lead)
- Builds on: ADR 0027 (the level meter), ADR 0038 (the cpal mic backend), ADR 0040 (`mog.level`)

## Context

The issue leaves these points open:
- Where the RMS level is computed, and how the rate stays at 30 Hz or less.
- Which MOG is the target when a session MOG has a directed PTT to another MOG.
- How "no level event after the stop" holds across the audio thread and the stop.
- The level of silence, because JSON has no `-Infinity`.
- How the -3 dBFS normalisation treats silence, loud input and bad samples.

The contract already has the `capture_subscribe` `onLevel: Channel<number>` and the `mog.level` event with `MicLevelFrame` (ADR 0040). PR #440 (#425) changes `capture.rs`, `types.rs`, the IPC files and the host. So this work is in new modules. The host hookup came later in #476 (decision 7).

## Decisions

1. **Rust computes the level.** `audio_device::level::LevelSink` is a `FrameSink`. It sends one RMS level in dBFS (`20 * log10(rms)`) per window of `ceil(rate / 30)` samples. It sends at most one level per data callback, so a large block never makes a burst. The rate is 30 Hz or less in audio time (CAP-16a). Only a number crosses IPC, never a sample (CAP-01b).
2. **Silence is -100 dBFS.** `LEVEL_FLOOR_DBFS` is finite, so it survives JSON. The meter shows 0 for any value under -60 dBFS (Table UX-LEVEL).
3. **A gate stops the stream.** `LevelGate` is open only while a capture records and is not paused. `LevelCaptureEvents` wraps the R2 `CaptureEvents` and drives the gate from each `capture.state`. The gate closes before the inner sink sees the stop state. `close` takes the send lock, so a send in flight ends before `close` returns (CAP-16b). The audio thread uses `try_lock` and drops a level when the lock is busy, so it never waits. Each `open` starts a new epoch, so a new capture never holds samples of the old one. A gap or a discard marker also starts a new window.
4. **The brain routes.** The `onLevel` Channel has one receiver, the brain. `src/brain/level-stream.ts` (`LevelRouter`) sends each level as `mog.level` only to the target MOG. The target is `directedTo` when set (a directed PTT in a session, AT-14), else `owner`. A paused or idle capture has no target. Other MOGs get no frame, so their meter stays at 0 (CAP-16a, UX-06b). When the target changes or the capture stops, the old target gets one `owner: null` frame with `dbfs: -100`. With no target, the brain drops a late level (CAP-16b).
5. **Peak normalisation is a whole-segment gain.** `audio_device::normalize::normalize_peak` scales a segment so that its peak is -3 dBFS (CAP-20). Quiet input goes up, and loud input comes down. One gain keeps the shape. Digital silence stays silence (gain 1). There is no gain limit: CAP-20 sets the peak, and the VAD trim (#116, CAP-54b) keeps non-speech out of STT. A non-finite sample becomes 0 first, and the output is clamped to -1..1.
6. **No Handy code.** Handy draws an FFT spectrum for its overlay (`audio_toolkit/audio/visualizer.rs` @ aca8985) and has no peak normaliser. Bottle needs one RMS number, so this is new code.

7. **The host hookup (#476, amendment).** `install_capture` builds the level stream in three parts (`src-tauri/src/capture_host.rs`):
   - `spawn_level_mic` gives `MicDevice` a `TeeSink`. The `LevelSink` gets each frame first, then the PTT buffer (`DiscardSink` until #21).
   - `CaptureHost::with_levels` builds `CaptureCore` with `LevelCaptureEvents` over the brain event sink, so the gate follows `capture.state`.
   - `capture_subscribe` resolves the `onLevel` reference to a `ChannelLevelOut` and puts it in the gate with `LevelGate::set_out`. The gate starts with `NoLevelOut`, so a level before the first subscribe is dropped. A later subscribe (a brain reload) replaces the receiver. `set_out` takes the send lock, so no level goes to the old Channel after it returns.
   - The app resolver (`brain_level_resolver`) reads the reference on the `brain` webview. The gateway lets only the brain call `capture_subscribe`, so `src-tauri/src/ipc.rs` (a contract file) does not change.
   - A reference that does not resolve (no brain webview) answers `host_failed`, and the earlier receiver stays.

## Consequences

- The level stream and the normaliser work and are tested with synthetic tones. No real mic opens in the tests.
- `startBrain` registers the `onLevel` Channel and the router. Since #476 a level from the mic reaches the target MOG end to end (`src-tauri/tests/level_host.rs`, `src/brain/level-wiring.test.ts`).
- The brain now creates three Channels at start-up: the brain Channel and the two `capture_subscribe` Channels.

## Open work

- `TODO(#475)`: a level carries no capture ID. A level that the brain reads after the next capture's `capture:state` can reach the next target MOG. The contract change binds each level to its capture.
- `TODO(#21)`, `TODO(#46)`: the PTT buffer and the segment pipeline call `normalize_peak` just before R4 reads a segment. The AT-40 test of that hand-off stays pending (`at_40_cap_20_segment_handed_to_r4_peaks_at_minus_3_dbfs`).
- `TODO(#46)`: `onPartial` has no reader yet.
