# ADR 0062: Mic permission, zero buffers, default device change and the built-in mic preference

- Status: Accepted
- Date: 2026-11-17
- Issue: #22 (CAP-17a, CAP-17b, CAP-18, CAP-22)
- Owner: impl-22 (capture-lead)
- Builds on: ADR 0038 (the cpal mic backend)

## Context

The issue leaves these points open:
- How the app finds a denied mic.
- How the 2 s zero-buffer check runs.
- How the stream opens again when the default device changes.
- Where the Bluetooth preference applies.

cpal 0.18 behaves in a different way on each OS. On macOS, CoreAudio moves an input stream to the new default device by itself, and cpal reports `DeviceChanged`. WASAPI on Windows does not rebind a stream. cpal reports `StreamInvalidated` when a new default exists, or `DeviceNotAvailable` when no default exists.

There is no wire field for a mic fault, and there is no app-wide setting IPC. #453 asks contract-lead for both.

## Decisions

1. **No permission call of our own (CAP-17a).** Bottle does not call `AVCaptureDevice` or ask for the mic at launch. The first stream build shows the OS prompt. Only `MicDevice::open` does a build, and only `capture_start`, a user action, calls `open`. A denied mic on macOS gives digital silence, so decision 2 catches it. Windows refuses the build, and the backend returns `permission_denied`. So we need no new native dependency.
2. **The zero watch runs on the audio thread (CAP-17b).** `ZeroWatch` has a 2 s window (`MicConfig::zero_window`). The window starts at open, and each block with sound restarts it. A block of digital silence counts as no sound, and so does no block at all. When a stream is open, the audio thread waits with `recv_timeout` until the deadline. At expiry, the backend does these steps:
   - It stops the stream.
   - It sends `SinkEvent::Discard`, so the sink drops the audio of the capture and no transcript is made.
   - It reports `MicFault::ZeroBuffers`.
   Then `CaptureCore::mic_silent` stops the leases with `StopReason::ZeroBuffers` (`zero_buffers`). The watch also applies during the capture: 2 s of digital silence (a hardware mute) stops it too. A quiet room is not digital silence.
3. **Reopen on the audio thread, same lease (CAP-18).**
   - `DeviceChanged` maps to `StreamFault::Rerouted`. The stream still runs. The sink gets `SinkEvent::Gap { gap_ms: 0 }`.
   - `StreamInvalidated` maps to `StreamFault::Invalidated`. The backend drops the old stream at once. Then it builds and plays a new stream on the new default, with no debounce, under the same generation (lease). Then the sink gets `SinkEvent::Gap { gap_ms }`.
   - Each built stream has its own epoch, so a late frame or fault of the old stream does nothing.
   - The sink keeps what it holds, so no buffered audio goes.
   - A failed reopen closes the slot and reports `MicFault::Lost` (`device_lost`).
   - The 10 s grace for "device lost, no replacement" (section 7.2) is not in this ADR.
4. **The Bluetooth preference applies at build (CAP-22).** `MicDevice::set_prefer_built_in` stores a flag. The audio thread calls `MicHost::set_preference` just before each build. It never does this at launch. `choose_input` is the rule: with the preference on, a Bluetooth default (or no default) gives way to the built-in mic, if there is one. `CpalMicHost` reads the cpal `InterfaceType` of each device. It does not log or keep device names. When the build does not use the headset mic, the headset stays in A2DP.
5. **Contract gaps behind doubles (#453).** The fix-it card needs the mic fault in `capture.state`. The hub needs an app-wide setting IPC. Until #453 lands:
   - `StopReason::ZeroBuffers` stays inside the core.
   - The hub `MicPreference` uses an in-memory store.
   The code marks both points `TODO(#453)`.

## Consequences

- A sink that keeps audio must handle `SinkEvent::Discard` (#21) and carry `SinkEvent::Gap` into the transcript (#116).
- The device checks need a person: the prompt timing, AirPods in A2DP and a real default-device switch. They are in the `needs:human` issue #391.
