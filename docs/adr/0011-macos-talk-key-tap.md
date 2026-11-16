# ADR 0011: The macOS talk key: listen-only FlagsChanged tap, polled Secure Input, cancel on SEI

- Status: Accepted
- Date: 2026-11-16
- Issue: #6 (soft dependency: spike SP-03 #5)
- Owner: impl-6 (capture-lead)

## Context

R1 HotkeyService on macOS must use a listen-only CGEventTap on its own run-loop thread (CAP-35a). It must report key-down and key-up of modifier-only keys (CAP-35b). It needs Input Monitoring only, and Accessibility only for text insertion (CAP-36, CAP-37, D70). A watchdog must start the tap again after a timeout and create it again on wake, session activation and permission grant (CAP-38a, CAP-38b). When Secure Event Input (SEI) starts, R1 must end an active PTT in 250 ms or less and switch to the Carbon chord Ctrl+Shift+Space (CAP-39a, CAP-39b, D66). After a gap, R1 must read the physical key state again (CAP-45). The issue leaves these choices open.

## Decision

1. **Events of interest: `FlagsChanged` only.** The talk keys of D66 are modifier-only keys. A `FlagsChanged` event carries only the key code of a modifier and the modifier flags. The tap never asks for `KeyDown` or `KeyUp`, so typed text never reaches Bottle. Nothing logs a key code.
2. **Tap: `kCGSessionEventTap`, `kCGEventTapOptionListenOnly`.** This needs Input Monitoring only. One constant (`hotkey_macos::TAP_SPEC`) holds the spec, and a test checks it.
3. **Split: platform-neutral logic in `hotkey_core.rs`.** The PTT machine, the tap watchdog decisions, the SEI edge monitor, the permission rules and the engine have no OS calls. Tests drive them with fakes and `SimClock`. The Windows hook (#7) uses the same module. `hotkey_macos.rs` holds the FFI.
4. **SEI detection: a poll every 100 ms.** macOS has no SEI notification. `IsSecureEventInputEnabled()` is cheap. A 100 ms tick gives a worst case of 100 ms, inside the 250 ms limit of CAP-39a. The same tick checks the tap state, the session and the permission.
5. **SEI start ends PTT with `cancel`, not `up`.** SEI means that a protected field (for example a password field) has focus. Dictation (UC1) inserts into the focused app, so an `up` could put dictated text into that field. `cancel` drops the audio. When SEI ends, a chord PTT ends with `up`, because the chord is unregistered and its release would be lost.
6. **Gap detection.** Wake: the difference of `mach_continuous_time` and `mach_absolute_time` grows by more than 500 ms (the absolute clock stops in sleep). Unlock and session activation: `CGSessionCopyCurrentDictionary` (`kCGSSessionOnConsoleKey`, `CGSSessionScreenIsLocked`). Permission grant: `IOHIDCheckAccess` goes to granted. Physical state: `CGEventSourceFlagsState` and `CGEventSourceKeyState` of the HID system state. A re-read can end a PTT, but it never starts one.
7. **Watchdog.** A tap disable (timeout, user input, or a failed health check) turns the tap on again. Three disables in 10 s create the tap again. Wake, session activation and permission grant create the tap again. Each action is followed by a key-state re-read, because a key-up can be lost while the tap is off.
8. **Carbon chord on the main thread.** `RegisterEventHotKey` and its handler run on the main dispatch queue, which the Tauri app runs. The chord is registered only while SEI is on.
9. **No new crates.** The FFI is raw `extern "C"` to system frameworks (CoreFoundation, CoreGraphics, ApplicationServices, IOKit, Carbon, libSystem). No `tauri-plugin-global-shortcut` (ARC-4), no `rdev`, no `handy-keys`. The `IsSecureEventInputEnabled` binding and the poll pattern are adapted from Handy `secure_input.rs` (MIT, see `THIRD_PARTY_NOTICES.md`).
10. **Start never prompts.** `start()` checks Input Monitoring with `IOHIDCheckAccess` and returns `input_monitoring_required` (`CoreError::Unavailable`) if it is missing. Only `request_ptt_permission` and `request_insertion_permission` can prompt, and they take a `UserAction` that only a UI click handler makes.

## Consequences

- The pill of CAP-39c needs a core-to-brain event. Section 4.5 has none, so contract-change #297 asks for `hotkey.status`. Until it lands, R1 emits `HotkeyOutput::TalkKeyPaused` to its sink only.
- The host must still wire `MacHotkeyService` into the Tauri app and forward the sink to `hotkey.ptt` (follow-up).
- If a permission is revoked while R1 runs, the watchdog creates the tap again when the permission comes back. If `start()` failed for lack of permission, the host calls `start()` again after the request.
- Real-device results (1,000 HID cycles, prompts, tap timeout, sleep and wake, SEI timing, Carbon release) wait for SP-03 (#5) and a person: steps in `docs/tests/hotkey-macos.md`.
- If SP-03 fails (for example key-ups are lost in SEI), the fallback of section 16 applies: an active tap with Accessibility, or chord-only.
