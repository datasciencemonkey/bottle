# ADR 0082: R4 SttEngine: model residency, preload on key-down and VAD decode during speech

- Status: Accepted
- Date: 2026-11-18
- Issue: #46 (CAP-51a, CAP-51b, CAP-51c, CAP-52, CAP-72; the CAP-20 hand-off of #23)
- Owner: impl-46 (capture-lead-2)
- Builds on: ADR 0049 (#45 packaging), ADR 0067 (#23 peak normalisation), ADR 0078 (#21 PTT buffer, PR #496)

## Context

The requirements set these rules:
- The model stays resident on AC power (CAP-51a).
- The model unloads after 30 min idle on battery (CAP-51b).
- The talk-key down preloads the model (CAP-51c).
- R4 decodes closed VAD segments during speech, so that only the tail decodes after key-up (CAP-52, tail p95 400 ms or less).
- Each transcript records the provider, the model and the runtime (CAP-72).

The issue leaves these points open:
- Where R4 gets the audio during speech.
- The VAD.
- The segment cut rule. #283 is open, and SP-04 shows that a 10 s tail misses the 400 ms limit.
- How the result reaches the brain.

## Decisions

1. **A new module, no contract change.** `mog_core::stt_engine` holds these parts:
   - `LocalSttEngine` is the handle of one worker thread that owns the recognizer (CAP-48).
   - `power` holds the power-source probe.
   - `residency` holds the pure policy.
   - `vad` holds the VAD trait, the energy VAD and the segment cutter.
   - `Resampler` converts the device rate to 16 kHz.
   - `LocalStt` implements the `stt::SttEngine` interface, and `TranscriptStore` keeps the results until `stt_transcribe` asks for them.
   The R4 stub goes away. Every call sends a message to the worker, so no caller waits for a model load or a decode.
2. **Live frames, not the stopped utterance.** CAP-52 needs the audio while the key is down. The PTT buffer (#21) gives its audio only at the stop. So R4 reads the mic frames through its own tee half, `SttFrameSink`. It learns the lease start and stop from `SttCaptureEvents`, which wraps the R2 events in the same way as `PttCaptureEvents`. A granted PTT or hands-free state begins a take. Any other state stops the frames. A stop commits the take or drops it. With no live lease, the sink copies no frame, so no pre-roll exists (CAP-13a). The host tees `SttFrameSink` next to the PTT buffer (`TeeSink`), and `CaptureHost::with_stt` puts `SttCaptureEvents` in the R2 event chain. R4 is also the PTT buffer's `UtteranceSink`. R4 has decoded the same audio, so it drops (and so zeroes) the utterance at once.
3. **The audio that R4 reads goes through `normalize_peak` (CAP-20).** Each closed segment and the tail get the -3 dBFS peak just before the decode. This is the #23 hand-off. The peak is per segment, so a loud word late in the hold does not make an early segment quieter.
4. **The segment cut rule (CAP-52).** A segment closes at a VAD pause of 300 ms or more after 3 s of audio. A segment never grows past 30 s, which is the R2 segment limit and the CAP-53a request limit. This is the #283 proposal, and TODO(#283) tracks the final rule.
   - A pause cut needs speech in the segment. So a quiet input that the VAD misses is not cut into pieces and dropped. It decodes at the 30 s cut or at key-up.
   - A segment with no speech and a peak under -60 dBFS is not decoded: a decode of silence can only make up words.
5. **The VAD.** `EnergyVad` (RMS over -50 dBFS on 32 ms windows) is the default. It needs no model file. `stt_local::SileroVad` wraps the sherpa-onnx Silero VAD under the `local-stt` feature. The Silero model file has no pinned download yet. TODO(#513): pin it and install it with the #45 installer under the same consent.
6. **Residency (CAP-51a, CAP-51b).**
   - The worker reads the OS power source at start and every 60 s. On macOS it calls IOKit `IOPSGetProvidingPowerSourceType`, and on Windows `GetSystemPowerStatus`. The calls are raw FFI with no new crate.
   - An unknown source, for example a desktop, counts as AC power.
   - On AC power the model loads at start and stays loaded. On AC power, a model that loaded once loads again after a battery unload.
   - On battery the model unloads after 30 min with no use. A use is a preload, a lease start or a decode. The model never unloads while a take is open.
7. **Preload (CAP-51c).** `preload()` and a granted lease load the model if it is not loaded, so the key-up path does not load it. The hotkey host calls `preload()` from `PttForwarder::with_key_down` at each `hotkey.ptt` down, before the brain gets the event.
8. **Commit and drop.** A stop that keeps the audio decodes the open tail. Then it gives one `SttDone` with one `Transcript` for each lease (`segmentId` `ptt-<lease>`). These stops make no transcript and zero the audio:
   - A stop for which `StopReason::discards_audio` is true (tap, cancel, Esc, zero buffers).
   - The panic stop (MB-05). ADR 0078 leaves this choice to the consumer: after a panic key, no text is inserted or sent.
   - A zero-buffer `SinkEvent::Discard`.
   A PTT over hands-free keeps two takes apart (ARC-F9, at most 2).
9. **CAP-72.** The loader reports an `SttIdentity`: `local_parakeet`, `stt::LOCAL_STT_MODEL` and `stt::LOCAL_STT_RUNTIME`. `stt_local::MODEL` and `RUNTIME` are now aliases of these constants. `latencyMs` is the time from the commit to the text, so it measures only the tail decode (section 8.1, step 3).
10. **Supervision.** The worker catches a panic in the loader, the recognizer, the VAD and the transcript sink, so the worker lives on.
    - A loader or recognizer panic drops the model and gives the lease the code `stt_engine_failed`. The next use loads a new model.
    - A VAD panic replaces the VAD of that take with the energy VAD. The window counts as speech, so no audio is dropped as silence.
    - A sink panic loses that one result. Handy restarts a worker process. Here the recognizer runs in the process, so the caught panic is the restart point.
11. **Privacy (CAP-01b, CAP-34, section 9.2).**
    - Each segment buffer has a fixed capacity, so it never moves in memory. The engine zeroes it after each decode and before it frees it, and it zeroes each frame message after use.
    - No audio and no text go to a log, a thread name or a `Debug` output. `SttDone` has a hand-written `Debug` with no text.
    - `ptt_buffer::live_utterances()` counts the live utterances (content-free), so a test proves that R4 keeps no second copy (CAP-34).
    - Each frame message names its lease, and `begin()` enqueues `Begin` before the audio thread can see the new lease. So the first frames of a PTT over hands-free never reach the hands-free take.
    - The sherpa-onnx Silero detector keeps its own copy of a speech segment. R4 drops that copy after each window.
12. **No Handy code is copied.** Handy's patterns are used: model residency with an idle unload, a supervised worker, preload, and a VAD with an onset and a hangover. Handy's engine (`transcribe-cpp`, GGUF Silero), its history and its logging are not used. So no MIT header and no new THIRD_PARTY_NOTICES entry are needed.

## Consequences

- The host runs R4 in the capture chain, tested with fakes (`src-tauri/tests/stt_host.rs`). It does not make text in the app yet, for these reasons:
  - The `local-stt` feature of `src-tauri` is off by default; CI and the release build turn it on in #534 (ADR 0049 amendment 1, #413).
  - `stt_transcribe` has no host route, and no core code emits `capture.segment` yet (TODO(#514)). The route is in `src-tauri/src/ipc.rs`, a contract file.
- #498 (the Copy card for `copy` and `insert_failed`) must land before #514 or with it, because #514 first produces routed dictation text. This PR produces no routed text: T4 SttProvider (#52) still rejects.
- The tail latency (AT-21, SP-04) and the 8 h residency on a device stay device steps. The real-model test in `tests/stt_local.rs` runs in the #414 job with `BOTTLE_STT_MODEL_DIR` (TODO(#414)).
- Partial text for `onPartial` (the texts of closed segments) is possible, but no contract field carries it. That is a later contract change.
