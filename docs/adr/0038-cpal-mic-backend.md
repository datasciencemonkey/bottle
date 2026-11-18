# ADR 0038: The cpal mic backend of CaptureService

- Status: Accepted
- Date: 2026-11-16
- Issue: #388 (part of #4)
- Owner: impl-388 (capture-lead)

## Context

#4 builds `CaptureCore` and the `AudioDevice` trait in `crates/mog-core/src/capture.rs`. The trait has two calls, `open` and `close`. `close` must stop the stream and the OS mic indicator before it returns (MOG-09, CAP-13b). R2 runs one device call at a time, under its device mutex. A stream error after open must call `CaptureCore::device_failed`.

A cpal stream is not `Send`. The panic stop must finish in 200 ms (MB-05), but a CoreAudio or WASAPI open can take longer. SP-08 (#15) is not run by a person yet. It cannot say yet if a pre-built stream keeps the indicator off.

The issue left these points open: the open path, the open/close race, the failure path, the frame seam, the error codes and the cpal pin.

## Decision

1. **Module.** `mog_core::audio_device`. `MicDevice` implements `AudioDevice` over a `MicHost` seam. `CpalMicHost` is the cpal host for macOS (CoreAudio) and Windows (WASAPI). Tests use a fake host.
2. **One audio thread.** `MicDevice::spawn` starts one audio thread. The host is made on that thread. The thread owns every stream. `open` and `close` are messages to the thread.
3. **Cold open (the SP-08 fallback).** `open` builds the stream on the default input device, then calls `play()`. No stream exists before `open` (MOG-09). A device change takes effect at the next open. If SP-08 passes, a later issue can pre-build the stream for CAP-21 (`TODO(#15)`).
4. **Close waits for the drop.** `close` sends a message and waits until the audio thread drops the stream. The OS indicator is then off. There is no idle timeout, no lazy close and no always-on mode.
5. **Close during an open cancels it.** R2 never does this, because it runs one device call at a time. The backend still guards against it, because `MicDevice` is a public type. Such a `close` does not wait for the open in progress, so a panic stop stays fast. It marks the open as cancelled. The open drops its stream before `play()`, or right after `play()` returns. Then the open returns `open_cancelled`.
6. **Generations.** Each open has a new generation number. A late close or a late error of an old stream never touches a newer stream.
7. **Failure.** cpal `DeviceChanged`, `Xrun` and `RealtimeDenied` are transient: cpal reroutes or the audio goes on, so capture goes on. Every other stream error is fatal. On a fatal error the audio thread drops the stream. A separate notifier thread then calls `CaptureCore::device_failed(Mic)` through the `FailureLink`. A separate thread is necessary: `device_failed` can call `close`, and `close` waits on the audio thread. The link holds a weak reference to the core.
8. **Frames.** Frames go to the in-RAM `FrameSink` trait: mono `f32` at the device rate. The data callback mixes the channels down. No frame reaches the sink after `close` returns, and no frame of a cancelled open reaches it. The consumers are later issues: the RAM-only PTT buffer (#21, ADR 0078), the level stream (#23, wired to the host by #476), `TODO(#46)` speech recognition with the 16 kHz resample. Nothing goes to disk or to a log, and there is no pre-roll.
9. **Codes.** Device errors are content-free codes: `permission_denied`, `no_device`, `device_busy`, `unsupported_format`, `device_failed`, `open_cancelled`, `audio_thread_gone` and `system_audio_unsupported`. They cross IPC only as `host_failed` (ADR 0007).
10. **System audio** is not in this backend. `open(SystemAudio)` returns `system_audio_unsupported` (#108, #109).
11. **cpal pin.** `cpal = "0.18.2"` (the SP-08 version), with no default features, is a workspace dependency. `mog-core` uses it only on macOS and Windows. `Cargo.lock` pins the exact version. Supply-chain review: #160.

## Consequences

- A host builds the mic with `MicDevice::spawn(CpalMicHost::new, sink)`, puts the device into `CaptureCore::new`, then calls `link.connect(&core)`. The Tauri wiring is a later issue.
- The audio between `play()` and the end of `open` is dropped (a few ms). This keeps cancelled audio out of the sink.
- A data callback takes the sink lock. A sink must be fast and must not block.
- The real-device checks (open, indicator off after close, unplug) need a person on macOS and Windows. They are in a `needs:human` issue.
- The "Device lost (no replacement in 10 s)" row of section 7.2: cpal follows the default device and reports `DeviceChanged` (transient). A `DeviceNotAvailable` error stops capture at once.
