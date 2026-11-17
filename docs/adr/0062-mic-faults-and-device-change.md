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

A cpal 0.18 input stream stays on the device it was built on, on both CoreAudio and WASAPI. cpal follows the default device only for output streams. So a new default input gives no stream error. If the stream's device goes, the error is `DeviceNotAvailable` (`Fatal` here). A stream that cannot go on gives `StreamInvalidated`. Bottle must find a new default input by itself.

There is no wire field for a mic fault, and there is no app-wide setting IPC. #453 asks contract-lead for both.

## Decisions

1. **No permission call of our own (CAP-17a).** Bottle does not call `AVCaptureDevice` or ask for the mic at launch. The first stream build shows the OS prompt. Only `MicDevice::open` does a build, and only `capture_start`, a user action, calls `open`. A denied mic on macOS gives digital silence, so decision 2 catches it. Windows refuses the build, and the backend returns `permission_denied`. So we need no new native dependency. `src-tauri/Info.plist` holds `NSMicrophoneUsageDescription`, because macOS stops an app that opens the mic without that key.
2. **The zero watch runs on the audio thread (CAP-17b).** `ZeroWatch` has a 2 s window (`MicConfig::zero_window`). The window starts at open, and each block with sound restarts it. A block of digital silence counts as no sound, and so does no block at all. When a stream is open, the audio thread waits with `recv_timeout` until the deadline. At expiry, the backend does these steps:
   - It stops the stream.
   - It sends `SinkEvent::Discard`, so the sink drops the audio of the capture and no transcript is made.
   - It reports `MicFault::ZeroBuffers`.
   Then `CaptureCore::mic_silent` stops the leases with `StopReason::ZeroBuffers` (`zero_buffers`). The watch also applies during the capture: 2 s of digital silence (a hardware mute) stops it too. A quiet room is not digital silence.
3. **Poll the default input, and reopen on the audio thread under the same lease (CAP-18, section 7.2).**
   - While a stream is open, the audio thread reads `MicHost::default_input_id` every 200 ms (`MicConfig::device_poll`). The value is an opaque ID, which is compared only and never logged. A change starts a reopen. So the reopen starts within 200 ms of the change, and the 500 ms limit holds when a build takes less than 300 ms.
   - `StreamInvalidated` (`StreamFault::Invalidated`) starts a reopen at once. `DeviceNotAvailable` (`StreamFault::Fatal`) also starts a reopen, if the host can name its default device.
   - A reopen drops the old stream. Then it builds and plays a new stream on the default device, with no debounce, under the same generation (lease). Then the sink gets `SinkEvent::Gap { gap_ms }`. The sink keeps what it holds, so no buffered audio goes.
   - Each built stream has its own epoch, so a late frame or fault of the old stream does nothing.
   - If no stream opens, the backend tries again every 250 ms for 10 s (`MicConfig::lost_grace`, the "device lost, no replacement in 10 s" row of section 7.2). After that it closes the slot and reports `MicFault::Lost` (`device_lost`). A `close` during the wait ends the wait with no fault.
   - `DeviceChanged` (`StreamFault::Rerouted`, a backend that moves the stream by itself) gives only a gap marker.
   - Like the cancelled open of ADR 0038 decision 5, a `close` that comes during a reopen build waits for that one build.
4. **The Bluetooth preference applies at build (CAP-22).** `MicDevice::set_prefer_built_in` stores a flag. The audio thread calls `MicHost::set_preference` just before each build, also each reopen. It never does this at launch. So when AirPods connect during a capture and become the default, the poll reopens the stream, and the rule picks the built-in mic again. `choose_input` is the rule: with the preference on, a Bluetooth default (or no default) gives way to the built-in mic, if there is one. `CpalMicHost` reads the cpal `InterfaceType` of each device. It does not log or keep device names. When the build does not use the headset mic, the headset stays in A2DP.
5. **Contract gaps behind doubles (#453).** The fix-it card needs the mic fault in `capture.state`. The hub needs an app-wide setting IPC. Until #453 lands:
   - `StopReason::ZeroBuffers` stays inside the core.
   - The hub `MicPreference` uses an in-memory store.
   The code marks both points `TODO(#453)`.

## Consequences

- A sink that keeps audio must handle `SinkEvent::Discard` (#21) and carry `SinkEvent::Gap` into the transcript (#116).
- The device checks need a person: the prompt timing, AirPods in A2DP and a real default-device switch. They are in the `needs:human` issue #391.
