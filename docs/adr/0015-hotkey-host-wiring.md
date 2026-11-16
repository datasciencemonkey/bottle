# ADR 0015: Host wiring of the talk key: queue sink, latched target, no prompt at start

- Status: Accepted
- Date: 2026-11-16
- Issue: #305 (follows #6 and ADR 0011)
- Owner: impl-305 (capture-lead)

## Context

R1 HotkeyService on macOS (`MacHotkeyService`, #6) calls its sink from the tap thread with the engine lock held. The Tauri host must create the service with the D66 talk key and forward `HotkeyOutput::Ptt(phase)` to the brain as `hotkey.ptt {mogId, phase}` (section 4.5). The talk key must target the last-interacted MOG (MB-06a). The host must not prompt for a permission on its own (CAP-36, CAP-37). The issue leaves these choices open.

## Decision

1. **One host module.** `src-tauri/src/hotkey_host.rs` holds the wiring. `lib.rs` only calls `install` in setup, `on_page_load`, `on_window_event`, and `shutdown` on `RunEvent::Exit`. If `install` fails, the app still starts without the talk key.
2. **Queue sink.** The R1 sink only pushes the output on an `mpsc` queue. A forwarder thread drains the queue and sends to the brain Channel. So the tap callback returns fast and never waits for a WebView.
3. **Latched target.** The forwarder reads the target at `down` and keeps it until `up` or `cancel`. One press goes to one MOG, even if the target moves during the hold. With no target, the press sends nothing, and no lone `up` follows.
4. **Interim target inputs.** MOG windows cannot take focus (MOG-01b), so MOG focus is not the main input. When a `mog-<id>` view finishes loading, that MOG becomes the target. So the last-opened MOG is the target. When a `card-<id>` window opens or gets focus, its MOG becomes the target. When the target MOG window closes, no MOG is the target. The brain's `mog_set_active` (#73) replaces these inputs.
5. **Status seam.** `TalkKeyPaused`, `TalkKeyRestored` and `FallbackChordUnavailable` go to a `StatusSink`. The app sink does nothing until the `hotkey.status` event of #297 exists.
6. **No prompt at start.** `HotkeyHost::start` calls the service `start`, which checks Input Monitoring with no prompt. On `input_monitoring_required` the state is `NeedsPermission`. Only `request_ptt_permission(UserAction)` prompts, and it starts the service again when granted. `request_insertion_permission(UserAction)` asks for Accessibility only. The IPC command for these calls is contract change #312.
7. **Platform seam.** `platform_service` picks the service by `cfg`. macOS: `MacHotkeyService` and `MacPermissions`. Windows: `WindowsHotkeyService` and `NoOsPermission` through the `HookSignal` adapter (amendment below, #314). Other OS: the R1 stub, so the state is `Unavailable("not_implemented")`.
8. **Stop on quit.** `RunEvent::Exit` calls `stop`, which removes the tap and the watchdog.

## Consequences

- `hotkey.ptt` reaches the brain only after the user grants Input Monitoring and the wizard or Settings calls `request_ptt_permission` (#312, #43), or after a restart with the permission granted.
- The talk key comes from the D66 default until Settings stores it (#77).
- Real-device checks of the wiring are steps H1-H5 in `docs/tests/hotkey-macos.md` (#303).

## Amendment 1: the Windows `HookSignal` adapter (#314)

- Date: 2026-11-16. Owner: impl-314 (capture-lead).

`WindowsHotkeyService::new(talk)` (#7) returns the service and a `Receiver<HookSignal>`. It does not take a `HotkeySink`. So the host adds an adapter.

1. **One adapter thread.** `spawn_hook_adapter` starts the thread `bottle-hotkey-signals`. It drains the receiver and owns one `HotkeyEngine` (#6). The engine emits into the host `QueueSink`. The forwarder then sends `hotkey.ptt {mogId, phase}` for the latched target (MB-06a), as on macOS.
2. **Shared PTT machine.** `TalkDown` and `TalkUp` go through `forward_to_engine` (#7). So Windows gives the same phases as macOS. A repeated `down` during a hold gives no second `down`.
3. **Deferred signals.** The adapter does not drop a signal in silence. `Escape`, `OtherKeyDown`, `TalkKeyBlocked`, `HookReinstalled` and `HookFailed` go to a `HookSignalSeam`. The app seam marks each consumer: `TODO(#14)` (capture lifecycle), `TODO(#24)` (CAP-44c cancel), `TODO(#294)` (CAP-42 tooltip), `TODO(#297)` (hook health status).
4. **No prompt.** Windows has no OS prompt for a `WH_KEYBOARD_LL` hook. `NoOsPermission` reports granted and never opens a prompt.
5. **Spawn failure.** If the adapter thread does not start, the host gets a service whose `start` returns `hook_adapter_spawn_failed`. The real hook is not installed, and the host state is `Unavailable("hook_adapter_spawn_failed")`.
6. **Thread end.** The thread ends when the service drops its sender.
7. **Platform-neutral code.** The adapter is not behind `cfg`, so its tests run on every OS with a fake receiver. The `#[cfg(windows)]` part of `platform_service` is four lines.

Consequence: the full `src-tauri` crate does not build for `x86_64-pc-windows-msvc` on a macOS host (the `tauri-winres` build script needs `llvm-rc`). The Windows build of the host is checked by the Windows CI job (#227) and by the device run (#296).
