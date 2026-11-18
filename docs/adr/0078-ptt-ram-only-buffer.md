# ADR 0078: The RAM-only PTT and hands-free buffer

- Status: Accepted
- Date: 2026-11-17
- Issue: #21 (CAP-12c, CAP-13a, CAP-13b, CAP-34, CAP-21)
- Owner: impl-21 (capture-lead-2)
- Builds on: ADR 0038 (the cpal mic backend), ADR 0055 (the capture host), ADR 0057 (the PTT lifecycle), ADR 0067 (the level stream and the -3 dBFS peak)

## Context

Before this ADR, the PTT half of the mic `TeeSink` was `DiscardSink`. The issue and section 9.2 set these rules:
- PTT and hands-free audio stay in RAM only.
- No pre-roll buffer exists before key-down.
- Pause closes the mic stream.
- The app zeroes the audio after STT and keeps no copy of audio that it sends for transcription.

The issue leaves these points open:
- Where the buffer lives.
- How it knows the start and the end of a capture.
- Its size cap.
- How it zeroes memory.
- How it hands the audio to STT, which (#46) does not exist yet.

## Decisions

1. **A new module, no contract change.** `mog_core::audio_device::ptt_buffer` has three parts:
   - `PttBuffer` is a shared handle.
   - `PttBufferSink` is the `FrameSink` half on the audio thread.
   - `PttCaptureEvents<E>` wraps the R2 `CaptureEvents`, in the same way as `LevelCaptureEvents`.
   No IPC type, schema or contract file changes. No audio crosses IPC (CAP-01b).
2. **One take per lease, driven by `capture.state`.** A take starts only when a `capture.state` shows a PTT or hands-free lease that records and is not paused. Before that, the sink drops each frame. The sink keeps no ring buffer, so no pre-roll exists (CAP-13a). A paused state, an idle state and a session state take no frames. R2 already opens the mic only for a lease and closes it on pause and stop (MOG-09, CAP-13b, ADR 0038).
3. **Session audio is not here.** The session journal owns session audio (#65, #116). A directed PTT in a session uses the session stream, so the segment pipeline cuts it (TODO(#116)).
4. **PTT over hands-free (ARC-F9).** The buffer holds at most two takes: a suspended hands-free take and the PTT take over it. Frames go to the lease that `capture.state` names. So the two utterances stay separate, and the hands-free take continues after the PTT.
5. **The size cap is the capture cap.** A PTT take holds at most 120 s (`HOLD_MAX_MS`, CAP-04a). A hands-free take holds at most 5 min (`HANDS_FREE_MAX_MS`, CAP-04b). The cap is in samples at the rate of the first frame. R2 stops the lease at the same time, so the cap only stops a late frame. The worst case is 7 min of audio at the device rate: 80 MB at 48 kHz.
6. **Fixed chunks, wiped before free.** The audio is in heap chunks of 4,096 samples. A chunk never moves, so a growing take never leaves an unwiped copy in freed memory, unlike a `Vec` that reallocates. Each chunk overwrites its samples with 0 (`write_volatile` and a compiler fence) in its `Drop`. This is the same method as `local_store/key_vault.rs`, so no new crate is needed.
7. **Stop.** The reasons that drop audio (`StopReason::discards_audio`: a tap, Esc or zero buffers) wipe the take. `SinkEvent::Discard` (CAP-17b) wipes every take at once. Any other stop does these steps:
   - It moves the take out under the lock.
   - After it releases the lock, it copies the take into one contiguous `Utterance` and wipes the chunks.
   - It calls `normalize::normalize_peak` on the utterance (CAP-20, ADR 0067 hand-off).
   - It gives the utterance to the `UtteranceSink`.
   A late `capture.state` of a stopped lease opens no new take.
8. **The `Utterance` is the only copy (CAP-34, section 9.2).** It has no `Clone`. Its `Debug` shows only the lease, the mode, the reason, the rate and the length: no sample and no MOG ID. Its `Drop` wipes every sample. So the app keeps no copy after the consumer drops it.
9. **The consumer is an interface.** R4 SttEngine (TODO(#46)) implements `UtteranceSink`. The sink runs on the thread that stopped the lease, so it must hand the utterance on fast. Until #46 lands, the host uses `NoUtteranceSink`, which drops, and so wipes, each utterance.
10. **A rate change mid-take.** After a device change (CAP-18), a frame at another rate is resampled to the take's rate by linear interpolation. TODO(#46): SttEngine resamples to 16 kHz.
11. **The host.** `CaptureHost::with_audio(device, clock, relay, levels, ptt)` builds the event chain `LevelCaptureEvents<PttCaptureEvents<BrainCaptureEvents>>`. `with_levels` keeps its signature and uses `NoUtteranceSink`. `install_capture` gives the mic `ptt.sink()` in place of `DiscardSink`.
12. **CAP-21 stays on the cold path.** SP-08 (#15) has not run on a device. So `MicDevice` builds the stream at the open, not before key-down (ADR 0038). A test pins this. TODO(#15): pre-build the stream if SP-08 passes. AT-21 (key-down p95 of 60 ms or less) needs that result and a device.
13. **No Handy code.** Handy keeps its recording in a plain `Vec<f32>` with a VAD pre-fill and no zeroing (`src-tauri/src/managers/audio.rs` @ aca8985). Bottle needs no pre-roll and a wipe, so this is new code.

## Consequences

- PTT and hands-free audio go from the mic to one in-RAM utterance per lease. The utterance has a -3 dBFS peak and waits for SttEngine (#46).
- The audio thread takes the buffer lock for each frame. The other side holds the lock only for O(1) steps.
- The OS can still page any process memory to swap. The app does not lock pages, because a lock of up to 80 MB can fail under the OS memory-lock limit, and a lock on the audio thread is a system call per chunk. macOS encrypts swap by default. On Windows, the page file is not encrypted by default. Follow-up: decide on best-effort page locks or document the Windows page file for admins.

## Open work

- `TODO(#46)`: SttEngine implements `UtteranceSink`, resamples to 16 kHz and drops each utterance after the decode.
- `TODO(#116)`: the session segment pipeline, with directed PTT segments and the gap marker.
- `TODO(#15)`: the pre-built stream if SP-08 passes (CAP-21, AT-21).
